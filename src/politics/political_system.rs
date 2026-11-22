//! Authoritative political expenditures and daily electoral competition.
//! Campaign actions settle through the real balanced ledger before influence or
//! specialist contracts change; all future-affecting standing is in AppState.

use super::{
    DistrictCampaign, INFLUENCE_CAP, INFLUENCE_COST_PER_POINT, MAYORAL_INFLUENCE_REQUIRED,
    MAYORAL_SUSTAINED_DAYS, OrganizationCampaign, SpecialistKind,
};
use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, ensure_time_current, is_day_boundary};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::world::{AutonomyLevel, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PoliticalError {
    #[error("political campaign requires a valid criminal organization and city district")]
    InvalidScope,
    #[error("only the designated player may issue this political order")]
    NotPlayer,
    #[error(
        "investment must be positive, in 5000-cent increments, and fit within local influence capacity"
    )]
    InvalidInvestment,
    #[error("this political specialist already serves this district")]
    SpecialistExists,
    #[error("this political specialist is not employed in this district")]
    MissingSpecialist,
    #[error(
        "specialist requires an active, available, independently delegated political manager in that district"
    )]
    InvalidManager,
    #[error("campaign spending would overflow its recorded lifetime budget")]
    BudgetOverflow,
    #[error("campaign order is stale")]
    Stale,
    #[error("insufficient accessible accounted campaign funds")]
    InsufficientFunds,
    #[error(transparent)]
    Finance(#[from] FinanceError),
}

fn valid_scope(state: &AppState, owner: OrganizationId, district: NeighborhoodId) -> bool {
    state
        .world()
        .get_organization(owner)
        .is_some_and(|o| o.kind() == OrganizationKind::Criminal)
        && state.world().get_neighborhood(district).is_some()
}

pub fn eligible_manager(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    manager: CharacterId,
) -> bool {
    state.world().get_character(manager).is_some_and(|person| {
        person.organization() == Some(owner)
            && person.supervisor().is_some()
            && matches!(
                person.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
            && state.legal().active_arrest_for_character(manager).is_none()
    }) && state
        .delegation()
        .active_for_scope(ResponsibilityScope::Neighborhood(district))
        .any(|mandate| {
            mandate.organization() == owner
                && mandate.manager() == manager
                && mandate.scopes().contains(&ResponsibilityScope::Function(
                    ResponsibilityFunction::Political,
                ))
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampaignAction {
    Invest(u16),
    Hire(SpecialistKind, CharacterId),
    Salary,
}

pub struct ValidatedPoliticalOrder {
    owner: OrganizationId,
    district: NeighborhoodId,
    action: CampaignAction,
    cents: i64,
    at: SimTime,
    transaction: ValidatedLedgerTransaction,
    clearing: crate::core::id::FinancialAccountId,
}

fn validate_spend(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    cents: i64,
    action: CampaignAction,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    if !valid_scope(state, owner, district) || cents <= 0 {
        return Err(PoliticalError::InvalidScope);
    }
    let campaign = state.politics().organization(owner);
    let spent = campaign.map_or(0, OrganizationCampaign::spent_cents);
    spent
        .checked_add(cents)
        .ok_or(PoliticalError::BudgetOverflow)?;
    let account = state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .find(|account| account.available_payment_balance().cents() >= cents)
        .ok_or(PoliticalError::InsufficientFunds)?;
    let (clearing, openings) = if let Some(id) = campaign.and_then(|c| c.clearing_account) {
        (id, None)
    } else {
        let opening = validate_open_accounts(
            state,
            vec![FinancialAccountDraft {
                owner: FinancialOwner::Organization(owner),
                kind: AccountKind::Settlement,
            }],
        )?;
        (
            opening
                .account_id(0)
                .expect("one campaign clearing account"),
            Some(opening),
        )
    };
    let transaction_draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!(
            "Municipal political campaign in district-{}",
            district.raw()
        ),
        postings: vec![
            LedgerPosting {
                account: account.id(),
                amount: Money::from_cents(-cents),
            },
            LedgerPosting {
                account: clearing,
                amount: Money::from_cents(cents),
            },
        ],
        authorization: None,
    };
    let transaction = if let Some(openings) = openings {
        validate_record_transaction_with_openings(state, openings, transaction_draft)?
    } else {
        validate_record_transaction(state, transaction_draft)?
    };
    Ok(ValidatedPoliticalOrder {
        owner,
        district,
        action,
        cents,
        at: state.now(),
        transaction,
        clearing,
    })
}

pub(crate) fn validate_invest_for(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    cents: i64,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    if cents <= 0 || cents % INFLUENCE_COST_PER_POINT != 0 {
        return Err(PoliticalError::InvalidInvestment);
    }
    let points = u16::try_from(cents / INFLUENCE_COST_PER_POINT)
        .map_err(|_| PoliticalError::InvalidInvestment)?;
    let current = state
        .politics()
        .organization(owner)
        .and_then(|campaign| campaign.district(district))
        .map_or(0, DistrictCampaign::influence);
    if points == 0
        || current
            .checked_add(points)
            .is_none_or(|v| v > INFLUENCE_CAP)
    {
        return Err(PoliticalError::InvalidInvestment);
    }
    validate_spend(
        state,
        owner,
        district,
        cents,
        CampaignAction::Invest(points),
    )
}

/// Purchase influence directly. It is not a permanent asset: the daily phase
/// erodes it even if all organizers are dismissed.
pub fn validate_invest_influence(
    state: &AppState,
    district: NeighborhoodId,
    cents: i64,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    let owner = state
        .player_organization()
        .ok_or(PoliticalError::NotPlayer)?;
    validate_invest_for(state, owner, district, cents)
}

pub fn validate_hire_specialist(
    state: &AppState,
    district: NeighborhoodId,
    manager: CharacterId,
    kind: SpecialistKind,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    let owner = state
        .player_organization()
        .ok_or(PoliticalError::NotPlayer)?;
    if !eligible_manager(state, owner, district, manager) {
        return Err(PoliticalError::InvalidManager);
    }
    if state
        .politics()
        .organization(owner)
        .and_then(|campaign| campaign.district(district))
        .is_some_and(|campaign| campaign.specialists.contains_key(&kind))
    {
        return Err(PoliticalError::SpecialistExists);
    }
    validate_spend(
        state,
        owner,
        district,
        kind.signing_cost(),
        CampaignAction::Hire(kind, manager),
    )
}

impl ValidatedPoliticalOrder {
    pub fn commit(self, state: &mut AppState) -> Result<(), PoliticalError> {
        ensure_time_current(state.now(), self.at).map_err(|_| PoliticalError::Stale)?;
        if !valid_scope(state, self.owner, self.district) {
            return Err(PoliticalError::Stale);
        }
        let campaign = state.politics().organization(self.owner);
        let current = campaign
            .and_then(|campaign| campaign.district(self.district))
            .map_or(0, DistrictCampaign::influence);
        match self.action {
            CampaignAction::Invest(points)
                if current
                    .checked_add(points)
                    .is_none_or(|v| v > INFLUENCE_CAP) =>
            {
                return Err(PoliticalError::Stale);
            }
            CampaignAction::Hire(kind, manager)
                if !eligible_manager(state, self.owner, self.district, manager)
                    || campaign
                        .and_then(|c| c.district(self.district))
                        .is_some_and(|d| d.specialists.contains_key(&kind)) =>
            {
                return Err(PoliticalError::Stale);
            }
            CampaignAction::Invest(_) | CampaignAction::Hire(..) | CampaignAction::Salary => {}
        }
        if campaign
            .map_or(0, OrganizationCampaign::spent_cents)
            .checked_add(self.cents)
            .is_none()
            || campaign
                .and_then(|c| c.clearing_account)
                .is_some_and(|account| account != self.clearing)
        {
            return Err(PoliticalError::Stale);
        }
        self.transaction.commit(state)?;
        let campaign = state.politics.organizations.entry(self.owner).or_default();
        campaign.clearing_account = Some(self.clearing);
        campaign.spent_cents += self.cents;
        let district = campaign.districts.entry(self.district).or_default();
        match self.action {
            CampaignAction::Invest(points) => district.influence += points,
            CampaignAction::Hire(kind, manager) => {
                district.specialists.insert(kind, manager);
            }
            CampaignAction::Salary => {}
        }
        Ok(())
    }
}

pub fn dismiss_specialist(
    state: &mut AppState,
    district: NeighborhoodId,
    kind: SpecialistKind,
) -> Result<(), PoliticalError> {
    let owner = state
        .player_organization()
        .ok_or(PoliticalError::NotPlayer)?;
    let slot = state
        .politics
        .organizations
        .get_mut(&owner)
        .and_then(|campaign| campaign.districts.get_mut(&district))
        .ok_or(PoliticalError::MissingSpecialist)?;
    slot.specialists
        .remove(&kind)
        .ok_or(PoliticalError::MissingSpecialist)?;
    Ok(())
}

/// Rival pressure is real, persistent and uses the same paid influence action.
/// The read-only player surface returns only a coarse category, never a rival's books.
pub fn rival_pressure(state: &AppState, owner: OrganizationId, district: NeighborhoodId) -> u16 {
    state
        .politics()
        .organizations
        .iter()
        .filter(|(organization, _)| **organization != owner)
        .filter_map(|(_, campaign)| campaign.district(district))
        .map(DistrictCampaign::influence)
        .max()
        .unwrap_or(0)
}

pub fn controls_district(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
) -> bool {
    let own = state
        .politics()
        .organization(owner)
        .and_then(|campaign| campaign.district(district))
        .map_or(0, DistrictCampaign::influence);
    own >= MAYORAL_INFLUENCE_REQUIRED
        && u32::from(own) > u32::from(rival_pressure(state, owner, district)) * 3 / 2
}

pub fn held_electoral_majority(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
) -> bool {
    controls_district(state, owner, district)
        && state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.district(district))
            .and_then(DistrictCampaign::leading_since)
            .is_some_and(|start| {
                state.now().as_minutes().saturating_sub(start.as_minutes())
                    >= MAYORAL_SUSTAINED_DAYS * DAY_MINUTES
            })
}

fn apply_daily_decay(state: &mut AppState) {
    for campaign in state.politics.organizations.values_mut() {
        for district in campaign.districts.values_mut() {
            let decay = (district.influence / 40).max(5);
            district.influence = district.influence.saturating_sub(decay);
        }
    }
}

fn pay_campaign_staff(state: &mut AppState) {
    let offices = state
        .politics()
        .organizations
        .iter()
        .flat_map(|(&owner, campaign)| {
            campaign
                .districts
                .iter()
                .map(move |(&district, office)| (owner, district, office.specialists.clone()))
        })
        .collect::<Vec<_>>();
    for (owner, district, specialists) in offices {
        let active = specialists
            .into_iter()
            .filter(|(kind, manager)| {
                let _ = kind;
                eligible_manager(state, owner, district, *manager)
            })
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>();
        let cents: i64 = active.iter().map(|kind| kind.daily_salary()).sum();
        if cents == 0 {
            continue;
        }
        // Wages are earned only if funded in full. A depleted campaign cannot
        // accrue free influence, and does not incur an unrecorded loan.
        if let Ok(order) = validate_spend(state, owner, district, cents, CampaignAction::Salary)
            && order.commit(state).is_ok()
            && let Some(office) = state
                .politics
                .organizations
                .get_mut(&owner)
                .and_then(|campaign| campaign.districts.get_mut(&district))
        {
            let growth: u16 = active.iter().map(|kind| kind.daily_growth()).sum();
            office.influence = office.influence.saturating_add(growth).min(INFLUENCE_CAP);
        }
    }
}

fn autonomous_rival_campaigns(state: &mut AppState) {
    // Weekly initiative: the existing commercial stake determines which ward
    // a rival contests, with a deterministic preference for the player's own
    // organized wards. Rivals must actually pay for influence from clean books.
    if !(state.now().as_minutes() / DAY_MINUTES).is_multiple_of(7) {
        return;
    }
    let Some(player) = state.player_organization() else {
        return;
    };
    let player_districts: BTreeSet<_> = state
        .politics()
        .organization(player)
        .map(|campaign| campaign.districts.keys().copied().collect())
        .unwrap_or_default();
    let rivals = state
        .world()
        .organizations()
        .filter(|organization| {
            organization.kind() == OrganizationKind::Criminal && organization.id() != player
        })
        .map(|organization| organization.id())
        .collect::<Vec<_>>();
    for owner in rivals {
        let districts = state
            .world()
            .businesses_owned_by_organization(owner)
            .map(|business| business.neighborhood())
            .collect::<BTreeSet<_>>();
        let choice = districts
            .iter()
            .copied()
            .find(|district| player_districts.contains(district))
            .or_else(|| districts.iter().copied().next());
        let Some(district) = choice else {
            continue;
        };
        let current = state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.district(district))
            .map_or(0, DistrictCampaign::influence);
        let points = (INFLUENCE_CAP - current).min(100);
        if points == 0 {
            continue;
        }
        if let Ok(order) = validate_invest_for(
            state,
            owner,
            district,
            i64::from(points) * INFLUENCE_COST_PER_POINT,
        ) {
            let _ = order.commit(state);
        }
    }
}

/// After same-day payroll and commerce, campaigns pay workers, suffer ordinary
/// attention decay, and face rival political spending. The last step records
/// continuously defended majority tenure; one losing day resets the clock.
pub fn apply_due_political_campaigns(state: &mut AppState) {
    if !is_day_boundary(state.now()) {
        return;
    }
    apply_daily_decay(state);
    pay_campaign_staff(state);
    autonomous_rival_campaigns(state);
    let owners = state
        .politics()
        .organizations
        .iter()
        .flat_map(|(&owner, campaign)| {
            campaign
                .districts
                .keys()
                .copied()
                .map(move |district| (owner, district))
        })
        .collect::<Vec<_>>();
    for (owner, district) in owners {
        let leading = controls_district(state, owner, district);
        let now = state.now();
        if let Some(office) = state
            .politics
            .organizations
            .get_mut(&owner)
            .and_then(|campaign| campaign.districts.get_mut(&district))
        {
            if leading {
                office.leading_since.get_or_insert(now);
            } else {
                office.leading_since = None;
            }
        }
    }
}

/// Release-safe save/invariant checks. Historical contracts may refer to
/// former or detained employees, but the daily phase never pays them.
pub fn valid_political_state(state: &AppState) -> bool {
    for (&owner, campaign) in &state.politics().organizations {
        if state
            .world()
            .get_organization(owner)
            .is_none_or(|record| record.kind() != OrganizationKind::Criminal)
            || campaign.spent_cents < 0
            || (campaign.spent_cents > 0 && campaign.clearing_account.is_none())
        {
            return false;
        }
        if let Some(id) = campaign.clearing_account
            && state.finance().get_account(id).is_none_or(|account| {
                account.owner() != FinancialOwner::Organization(owner)
                    || account.kind() != AccountKind::Settlement
            })
        {
            return false;
        }
        for (&district, office) in &campaign.districts {
            if state.world().get_neighborhood(district).is_none()
                || office.influence > INFLUENCE_CAP
                || office
                    .leading_since
                    .is_some_and(|start| start > state.now())
                || office
                    .specialists
                    .values()
                    .any(|manager| state.world().get_character(*manager).is_none())
            {
                return false;
            }
        }
    }
    true
}
