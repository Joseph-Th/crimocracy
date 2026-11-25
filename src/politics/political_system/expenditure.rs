//! Canonical political spending: balanced municipal payments, held-order freshness,
//! campaign updates and lifetime expenditure reconciliation.

use super::{
    PoliticalError, campaign_office_available, eligible_candidate, eligible_manager,
    initiative_ready, next_mayoral_nomination_day, reconcile_district_majorities, valid_scope,
};
use crate::core::id::{CharacterId, FinancialAccountId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{SimTime, ensure_time_current};
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_open_accounts, validate_record_campaign_transaction,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::politics::{
    CivicInitiative, DistrictCampaign, INFLUENCE_CAP, MayoralCandidacy, OrganizationCampaign,
    SpecialistKind,
};
use crate::world::OrganizationKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CampaignAction {
    Invest(u16),
    Hire(SpecialistKind, CharacterId),
    Initiative(CivicInitiative, CharacterId),
    Nominate(MayoralCandidacy),
    Salary,
}

pub struct ValidatedPoliticalOrder {
    owner: OrganizationId,
    district: NeighborhoodId,
    action: CampaignAction,
    cents: i64,
    at: SimTime,
    transaction: ValidatedLedgerTransaction,
    clearing: FinancialAccountId,
}

pub(super) fn validate_spend(
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
    let mut remaining = cents;
    let mut postings = Vec::new();
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
    {
        let contribution = remaining.min(account.available_payment_balance().cents());
        if contribution > 0 {
            postings.push(LedgerPosting {
                account: account.id(),
                amount: Money::from_cents(-contribution),
            });
            remaining -= contribution;
        }
        if remaining == 0 {
            break;
        }
    }
    if remaining > 0 {
        return Err(PoliticalError::InsufficientFunds);
    }
    // Campaign expenditure must actually leave the organization's treasury.
    // A same-owner settlement credit would only park money in its own ledger.
    let municipal_institution = state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .ok_or(PoliticalError::MissingMunicipalInstitution)?
        .id();
    let (clearing, openings) = if let Some(id) = campaign.and_then(|c| c.clearing_account) {
        (id, None)
    } else {
        let opening = validate_open_accounts(
            state,
            vec![FinancialAccountDraft {
                owner: FinancialOwner::Organization(municipal_institution),
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
    postings.push(LedgerPosting {
        account: clearing,
        amount: Money::from_cents(cents),
    });
    let transaction_draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!(
            "Municipal political campaign in district-{}",
            district.raw()
        ),
        postings,
        authorization: None,
    };
    let transaction =
        validate_record_campaign_transaction(state, clearing, openings, transaction_draft)?;
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

impl ValidatedPoliticalOrder {
    pub fn commit(self, state: &mut AppState) -> Result<(), PoliticalError> {
        self.commit_order(state, true)
    }

    /// Daily rival spending is one part of the maintenance cohort; the daily owner reconciles
    /// every ward after all decay, staff output and investments have been applied.
    pub(super) fn commit_daily(self, state: &mut AppState) -> Result<(), PoliticalError> {
        self.commit_order(state, false)
    }

    fn commit_order(self, state: &mut AppState, reconcile: bool) -> Result<(), PoliticalError> {
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
                    || !campaign_office_available(state, self.owner, self.district, manager)
                    || campaign
                        .and_then(|c| c.district(self.district))
                        .is_some_and(|d| d.specialists.contains_key(&kind)) =>
            {
                return Err(PoliticalError::Stale);
            }
            CampaignAction::Initiative(kind, manager)
                if initiative_ready(state, self.owner, self.district, manager, kind).is_err() =>
            {
                return Err(PoliticalError::Stale);
            }
            CampaignAction::Nominate(candidacy)
                if !eligible_candidate(state, self.owner, candidacy.candidate)
                    || state
                        .politics()
                        .organization(self.owner)
                        .and_then(OrganizationCampaign::candidacy)
                        .is_some()
                    || next_mayoral_nomination_day(state.now()) != Some(candidacy.election_day) =>
            {
                return Err(PoliticalError::Stale);
            }
            CampaignAction::Invest(_)
            | CampaignAction::Hire(..)
            | CampaignAction::Initiative(..)
            | CampaignAction::Nominate(_)
            | CampaignAction::Salary => {}
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
        match self.action {
            CampaignAction::Invest(points) => {
                campaign
                    .districts
                    .entry(self.district)
                    .or_default()
                    .influence += points;
            }
            CampaignAction::Hire(kind, manager) => {
                campaign
                    .districts
                    .entry(self.district)
                    .or_default()
                    .specialists
                    .insert(kind, manager);
            }
            CampaignAction::Initiative(kind, _) => {
                let office = campaign.districts.entry(self.district).or_default();
                let (machine, grassroots, institutions) = kind.boosts();
                office.influence = office.influence.saturating_add(machine).min(INFLUENCE_CAP);
                office.grassroots = office
                    .grassroots
                    .saturating_add(grassroots)
                    .min(INFLUENCE_CAP);
                office.institutions = office
                    .institutions
                    .saturating_add(institutions)
                    .min(INFLUENCE_CAP);
                office.initiatives.insert(kind, self.at);
            }
            CampaignAction::Nominate(candidacy) => campaign.candidacy = Some(candidacy),
            CampaignAction::Salary => {}
        }
        if reconcile
            && matches!(
                self.action,
                CampaignAction::Invest(_) | CampaignAction::Initiative(..)
            )
        {
            reconcile_district_majorities(state, self.district);
        }
        Ok(())
    }
}

/// Reconcile the cumulative budget with the dedicated clearing pocket and its
/// indexed ledger history. No foreign money, withdrawals or delegated budget
/// postings count as a campaign's expenditure, regardless of the memo.
pub(super) fn valid_campaign_expenditure(
    state: &AppState,
    owner: OrganizationId,
    spent_cents: i64,
    clearing: FinancialAccountId,
) -> bool {
    let Some(account) = state.finance().get_account(clearing) else {
        return false;
    };
    if spent_cents <= 0
        || account.kind() != AccountKind::Settlement
        || account.balance().cents() != spent_cents
        || !matches!(account.owner(), FinancialOwner::Organization(institution)
            if state.world().get_organization(institution).is_some_and(
                |organization| organization.kind() == OrganizationKind::Political))
    {
        return false;
    }
    let mut total = 0_i128;
    for transaction in state
        .finance()
        .transactions_for_account_through(clearing, state.now())
    {
        if transaction.budget_usage().is_some() {
            return false;
        }
        let mut received = None;
        let mut debited = 0_i128;
        for posting in transaction.postings() {
            if posting.account == clearing {
                if posting.amount <= Money::ZERO || received.replace(posting.amount).is_some() {
                    return false;
                }
            } else {
                let Some(source) = state.finance().get_account(posting.account) else {
                    return false;
                };
                if posting.amount >= Money::ZERO
                    || source.owner() != FinancialOwner::Organization(owner)
                    || source.kind() != AccountKind::AccountedFunds
                {
                    return false;
                }
                debited -= i128::from(posting.amount.cents());
            }
        }
        if received.is_none_or(|amount| i128::from(amount.cents()) != debited) {
            return false;
        }
        total += debited;
    }
    total == i128::from(spent_cents)
}
