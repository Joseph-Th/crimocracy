//! Optional, record-backed campaign endings. Sandbox play never ends for merely meeting a threshold:
//! the player must explicitly claim a qualified victory. A typed history event freezes the result
//! without changing the serialized AppState layout, so existing campaign saves remain loadable.

use crate::contacts::{ContactKind, ContactStatus};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, HistoryEventId, IdExhaustionError, IdKind, LedgerTransactionId, OrganizationId,
    ReportId,
};
use crate::core::state::AppState;
use crate::core::time::{SimTime, ensure_time_current};
use crate::economy::BusinessOperatingStatus;
use crate::enterprises::EnterpriseStatus;
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::history_system::{
    HistoryError, ValidatedHistoryEvent, validate_campaign_victory_event,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_report_draft};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::OrganizationKind;
use crate::world::territory_influence::resolve_neighborhood_influence;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// These are ending choices, not automatic achievements. Variant order is persistent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VictoryPath {
    PoliticalPower,
    FundedRetirement,
    CityUnderboss,
}

impl VictoryPath {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PoliticalPower => "legitimate political power",
            Self::FundedRetirement => "funded retirement",
            Self::CityUnderboss => "city underboss",
        }
    }
}

pub const RETIREMENT_NEST_EGG: Money = Money::from_cents(5_000_000);
pub const RETIREMENT_SEVERANCE_DAYS: i64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VictoryProgress {
    pub total_districts: usize,
    pub required_political_districts: usize,
    pub owned_operating_business_districts: usize,
    pub active_political_contacts: usize,
    pub required_underboss_districts: usize,
    pub own_active_racket_districts: usize,
    pub accounted_net: Money,
    pub retirement_total: Money,
    /// Only valid in a full simulation context; do not expose hidden rival standing in the player client.
    underboss_ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum VictoryError {
    #[error("no player organization is designated")]
    NoPlayer,
    #[error("Campaign over: your organization dissolved")]
    Defeated,
    #[error("Campaign over: victory has already been claimed")]
    AlreadyWon,
    #[error(
        "legitimate political power requires a live City Council contact and operating businesses in at least half of the city's districts"
    )]
    PoliticalNotReady,
    #[error(
        "funded retirement needs one leader, no pending decisions or active operations, and enough accounted money for the leader's nest egg and 180 days of wages for each subordinate"
    )]
    RetirementNotReady,
    #[error(
        "city underboss requires uncontested influence in three quarters of districts and more active rackets than any rival"
    )]
    UnderbossNotReady,
    #[error("retirement payment arithmetic overflow")]
    Arithmetic,
    #[error("campaign outcome changed since validation")]
    Stale,
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

fn retirement_members(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
) -> Result<(Vec<(CharacterId, Money)>, Money), VictoryError> {
    let members: Vec<_> = state
        .world()
        .characters_in_organization(organization)
        .collect();
    let leaders: Vec<_> = members
        .iter()
        .filter(|member| member.supervisor().is_none())
        .map(|member| member.id())
        .collect();
    if leaders.len() != 1 || members.is_empty() {
        return Err(VictoryError::RetirementNotReady);
    }
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .ok_or(VictoryError::Arithmetic)?;
    let mut payees = Vec::with_capacity(members.len());
    for member in members {
        let amount = if member.id() == leaders[0] {
            RETIREMENT_NEST_EGG
        } else {
            severance
        };
        payees.push((member.id(), amount));
    }
    let required = RETIREMENT_NEST_EGG
        .checked_add(
            severance
                .checked_mul(i64::try_from(payees.len() - 1).map_err(|_| VictoryError::Arithmetic)?)
                .ok_or(VictoryError::Arithmetic)?,
        )
        .ok_or(VictoryError::Arithmetic)?;
    Ok((payees, required))
}

pub fn progress(registry: &Registry, state: &AppState) -> Result<VictoryProgress, VictoryError> {
    let organization = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let total_districts = state.world().neighborhoods().count();
    let required_political_districts = total_districts.div_ceil(2);
    let required_underboss_districts = (total_districts * 3).div_ceil(4);
    let owned_operating_business_districts = state
        .world()
        .businesses_owned_by_organization(organization)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
        })
        .map(|business| business.neighborhood())
        .collect::<BTreeSet<_>>()
        .len();
    let active_political_contacts = state
        .contacts()
        .contacts_for_sponsor(organization)
        .filter(|contact| {
            contact.kind() == ContactKind::Political
                && contact.status() == ContactStatus::Active
                && state
                    .world()
                    .get_organization(contact.institution())
                    .is_some_and(|institution| institution.kind() == OrganizationKind::Political)
                && state
                    .world()
                    .get_character(contact.handler())
                    .is_some_and(|handler| handler.organization() == Some(organization))
        })
        .count();
    let own_active_racket_districts = state
        .world()
        .neighborhoods()
        .filter(|district| {
            state
                .enterprises()
                .enterprises_for_organization(organization)
                .any(|enterprise| {
                    if enterprise.status() != EnterpriseStatus::Active {
                        return false;
                    }
                    match enterprise.location() {
                        crate::enterprises::EnterpriseLocation::Neighborhood(id) => {
                            id == district.id()
                        }
                        crate::enterprises::EnterpriseLocation::Business(id) => state
                            .world()
                            .get_business(id)
                            .is_some_and(|business| business.neighborhood() == district.id()),
                    }
                })
        })
        .count();
    let uncontested = state
        .world()
        .neighborhoods()
        .filter(|district| {
            resolve_neighborhood_influence(state, district.id())
                .is_ok_and(|summary| summary.economic_leader() == Some(organization))
        })
        .count();
    let own_active = state
        .enterprises()
        .enterprises_for_organization(organization)
        .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
        .count();
    let rival_max = state
        .world()
        .organizations()
        .filter(|owner| owner.kind() == OrganizationKind::Criminal && owner.id() != organization)
        .map(|owner| {
            state
                .enterprises()
                .enterprises_for_organization(owner.id())
                .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
                .count()
        })
        .max()
        .unwrap_or(0);
    // Include negative accounted balances; a debt cannot be ignored by spending another pocket.
    let net = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| i128::from(account.balance().cents()))
        .sum::<i128>()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    let retirement_total = retirement_members(registry, state, organization)
        .map(|(_, total)| total)
        .unwrap_or(RETIREMENT_NEST_EGG);
    Ok(VictoryProgress {
        total_districts,
        required_political_districts,
        owned_operating_business_districts,
        active_political_contacts,
        required_underboss_districts,
        own_active_racket_districts,
        accounted_net: Money::from_cents(net as i64),
        retirement_total,
        underboss_ready: total_districts > 0
            && uncontested >= required_underboss_districts
            && own_active >= 2
            && own_active > rival_max,
    })
}

fn ensure_eligible(
    registry: &Registry,
    state: &AppState,
    path: VictoryPath,
) -> Result<(), VictoryError> {
    if state.player_organization_defeated() {
        return Err(VictoryError::Defeated);
    }
    if state.campaign_victory().is_some() {
        return Err(VictoryError::AlreadyWon);
    }
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let standing = progress(registry, state)?;
    match path {
        VictoryPath::PoliticalPower
            if standing.active_political_contacts == 0
                || standing.total_districts == 0
                || standing.owned_operating_business_districts
                    < standing.required_political_districts =>
        {
            Err(VictoryError::PoliticalNotReady)
        }
        VictoryPath::FundedRetirement => {
            let (_, required) = retirement_members(registry, state, owner)?;
            let spendable: i128 = state
                .finance()
                .accounts_for(FinancialOwner::Organization(owner))
                .filter(|account| account.kind() == AccountKind::AccountedFunds)
                .map(|account| i128::from(account.available_payment_balance().cents()))
                .sum();
            if standing.accounted_net < required
                || spendable < i128::from(required.cents())
                || state
                    .decisions()
                    .pending_for_recipient(owner)
                    .next()
                    .is_some()
                || state
                    .operations()
                    .operations_for_organization(owner)
                    .any(|operation| {
                        !matches!(
                            operation.status(),
                            crate::operations::OperationStatus::Completed
                                | crate::operations::OperationStatus::Aborted
                        )
                    })
            {
                Err(VictoryError::RetirementNotReady)
            } else {
                Ok(())
            }
        }
        VictoryPath::CityUnderboss if !standing.underboss_ready => {
            Err(VictoryError::UnderbossNotReady)
        }
        VictoryPath::PoliticalPower | VictoryPath::CityUnderboss => Ok(()),
    }
}

fn prepare_retirement_payment(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
) -> Result<ValidatedLedgerTransaction, VictoryError> {
    let (payees, required) = retirement_members(registry, state, owner)?;
    let mut remaining = required;
    let mut postings = Vec::new();
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
    {
        if remaining == Money::ZERO {
            break;
        }
        let debit = account.available_payment_balance().min(remaining);
        if debit <= Money::ZERO {
            continue;
        }
        postings.push(LedgerPosting {
            account: account.id(),
            amount: debit.checked_neg().ok_or(VictoryError::Arithmetic)?,
        });
        remaining = remaining
            .checked_sub(debit)
            .ok_or(VictoryError::Arithmetic)?;
    }
    if remaining != Money::ZERO {
        return Err(VictoryError::RetirementNotReady);
    }
    let mut missing = Vec::new();
    let mut destinations = Vec::new();
    for (person, amount) in payees {
        let destination = state
            .finance()
            .accounts_for(FinancialOwner::Character(person))
            .find(|account| {
                account.kind() == AccountKind::AccountedFunds
                    && account.can_receive_liquid_payment(amount)
            })
            .map(|account| account.id());
        if destination.is_none() {
            missing.push(FinancialAccountDraft {
                owner: FinancialOwner::Character(person),
                kind: AccountKind::AccountedFunds,
            });
        }
        destinations.push((destination, amount));
    }
    let openings = if missing.is_empty() {
        None
    } else {
        Some(validate_open_accounts(state, missing)?)
    };
    let mut opened = 0;
    for (destination, amount) in destinations {
        let id = match destination {
            Some(id) => id,
            None => {
                let id = openings
                    .as_ref()
                    .and_then(|plan| plan.account_id(opened))
                    .ok_or(VictoryError::Arithmetic)?;
                opened += 1;
                id
            }
        };
        postings.push(LedgerPosting {
            account: id,
            amount,
        });
    }
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: "Final campaign retirement settlement: leader nest egg and crew severance".to_owned(),
        postings,
        authorization: None,
    };
    Ok(match openings {
        Some(plan) => validate_record_transaction_with_openings(state, plan, draft)?,
        None => validate_record_transaction(state, draft)?,
    })
}

pub struct ValidatedCampaignVictory {
    owner: OrganizationId,
    path: VictoryPath,
    validated_at: SimTime,
    history: ValidatedHistoryEvent,
    report: ValidatedReport,
    settlement: Option<ValidatedLedgerTransaction>,
}

pub struct CampaignVictoryOutcome {
    pub history: HistoryEventId,
    pub report: ReportId,
    pub settlement: Option<LedgerTransactionId>,
    pub path: VictoryPath,
}

pub fn validate_claim_victory(
    registry: &Registry,
    state: &AppState,
    path: VictoryPath,
) -> Result<ValidatedCampaignVictory, VictoryError> {
    ensure_eligible(registry, state, path)?;
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let settlement = if path == VictoryPath::FundedRetirement {
        Some(prepare_retirement_payment(registry, state, owner)?)
    } else {
        None
    };
    let settlement_id = settlement
        .as_ref()
        .map(|_| LedgerTransactionId::from_raw(state.ids.next_raw(IdKind::LedgerTransaction)));
    let history = validate_campaign_victory_event(state, owner, path, settlement_id)?;
    let report = validate_report_draft(state, ReportDraft {
        recipient: owner,
        kind: ReportKind::CampaignVictory,
        title: format!("Campaign victory: {}", path.label()),
        entries: vec![ReportEntry {
            attention: AttentionClass::Crisis,
            summary: match path {
                VictoryPath::PoliticalPower =>
                    "Legitimate businesses across the city and a live political sponsor secured a governing coalition.".to_owned(),
                VictoryPath::FundedRetirement =>
                    "The leader retired with a funded nest egg; each subordinate received 180 days of wages in an accounted personal account.".to_owned(),
                VictoryPath::CityUnderboss =>
                    "Uncontested economic influence across the city's districts and superior underworld strength secured the underboss position.".to_owned(),
            },
            sources: Vec::new(),
            entities: BTreeSet::from([EntityRef::Organization(owner)]),
            decision: None,
        }],
    })?;
    let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
    if let Some(payment) = &settlement {
        budget.extend(payment.id_budget());
    }
    state.ids.reserve_many(&budget)?;
    Ok(ValidatedCampaignVictory {
        owner,
        path,
        validated_at: state.now(),
        history,
        report,
        settlement,
    })
}

impl ValidatedCampaignVictory {
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<CampaignVictoryOutcome, VictoryError> {
        ensure_time_current(state.now(), self.validated_at).map_err(|_| VictoryError::Stale)?;
        if state.player_organization() != Some(self.owner) {
            return Err(VictoryError::Stale);
        }
        ensure_eligible(registry, state, self.path)?;
        if let Some(settlement) = &self.settlement {
            settlement.ensure_current(state)?;
        }
        let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
        if let Some(settlement) = &self.settlement {
            budget.extend(settlement.id_budget());
        }
        state.ids.reserve_many(&budget)?;
        let settlement = self.settlement.map(|plan| plan.commit_preflighted(state));
        let history = self.history.commit(state)?;
        let report = self.report.commit(state)?;
        Ok(CampaignVictoryOutcome {
            history,
            report,
            settlement,
            path: self.path,
        })
    }
}

#[cfg(test)]
mod tests;
