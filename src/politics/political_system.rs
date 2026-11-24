//! Authoritative political expenditures and daily electoral competition.
//! Campaign actions settle through the real balanced ledger before influence or
//! specialist contracts change; all future-affecting standing is in AppState.

use super::{
    CivicInitiative, DistrictCampaign, GRASSROOTS_REQUIRED, INFLUENCE_CAP,
    INFLUENCE_COST_PER_POINT, INSTITUTIONAL_REQUIRED, MAYORAL_FILING_FEE_CENTS,
    MAYORAL_FILING_LEAD_DAYS, MAYORAL_INFLUENCE_REQUIRED, MAYORAL_SUSTAINED_DAYS,
    MUNICIPAL_ELECTION_INTERVAL_DAYS, MayoralCandidacy, MayoralElection, OrganizationCampaign,
    SpecialistKind, WardBallot,
};
use crate::contacts::{ContactKind, ContactStatus};
use crate::core::campaign_victory::POLITICAL_CONTACT_DAYS;
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
use crate::registry::Registry;
use crate::reputation::AudienceKind;
use crate::reputation::reputation_system::resolve_score;
use crate::world::{
    AutonomyLevel, BusinessFunction, BusinessKind, CapabilityKind, OrganizationKind,
};
use std::collections::{BTreeMap, BTreeSet};
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
    #[error("local initiative needs the appropriate active owned venue or institutional contact")]
    MissingCivicInfrastructure,
    #[error("this local initiative was already organized in the past seven days")]
    InitiativeCooldown,
    #[error(
        "a mayoral candidate must be the available executive with a 180-day active council contact"
    )]
    IneligibleCandidate,
    #[error("a candidacy is already registered for the upcoming municipal election")]
    AlreadyNominated,
    #[error("no valid future municipal election remains on the finite simulation clock")]
    ElectionUnavailable,
    #[error(
        "specialist requires an active, available, independently delegated political manager in that district"
    )]
    InvalidManager,
    #[error(
        "this manager already supervises specialists in another district; dismiss that office's contracts before assigning a new office"
    )]
    ManagerOfficeConflict,
    #[error("campaign spending would overflow its recorded lifetime budget")]
    BudgetOverflow,
    #[error("campaign order is stale")]
    Stale,
    #[error("insufficient accessible accounted campaign funds")]
    InsufficientFunds,
    #[error("the city has no political institution to receive declared campaign expenditures")]
    MissingMunicipalInstitution,
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
        .active_for_manager(manager)
        .is_some_and(|mandate| {
            mandate.organization() == owner
                && mandate
                    .scopes()
                    .contains(&ResponsibilityScope::Neighborhood(district))
                && mandate.scopes().contains(&ResponsibilityScope::Function(
                    ResponsibilityFunction::Political,
                ))
        })
}

/// A standing mandate may cover several wards, but an ongoing political office
/// requires a local supervisor. Contracts retain their assigned office until
/// explicitly dismissed, even if the supervisor temporarily loses authority.
pub fn campaign_office_available(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    manager: CharacterId,
) -> bool {
    state.politics().organization(owner).is_none_or(|campaign| {
        campaign.districts.iter().all(|(&other, office)| {
            other == district
                || !office
                    .specialists
                    .values()
                    .any(|&assigned| assigned == manager)
        })
    })
}

/// Full daily contractual salary and the currently payable subset, before
/// affordability is checked at the daily settlement. Both use authored wages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CampaignSalaryCommitment {
    pub contracted: Money,
    pub payable: Money,
}

pub fn campaign_salary_commitment(
    state: &AppState,
    owner: OrganizationId,
) -> CampaignSalaryCommitment {
    let mut contracted = 0_i64;
    let mut payable = 0_i64;
    if let Some(campaign) = state.politics().organization(owner) {
        for (&district, office) in &campaign.districts {
            for (&kind, &manager) in &office.specialists {
                contracted = contracted.saturating_add(kind.daily_salary());
                if eligible_manager(state, owner, district, manager) {
                    payable = payable.saturating_add(kind.daily_salary());
                }
            }
        }
    }
    CampaignSalaryCommitment {
        contracted: Money::from_cents(contracted),
        payable: Money::from_cents(payable),
    }
}

/// The next ballot must actually fit on the finite simulation clock. Returning
/// `None` also keeps read-only campaign views from advertising an impossible date.
pub fn next_municipal_election_day(now: SimTime) -> Option<u64> {
    let day = (now.as_minutes() / DAY_MINUTES / MUNICIPAL_ELECTION_INTERVAL_DAYS)
        .checked_add(1)?
        .checked_mul(MUNICIPAL_ELECTION_INTERVAL_DAYS)?;
    day.checked_mul(DAY_MINUTES).map(|_| day)
}

fn established_contact(state: &AppState, owner: OrganizationId, kind: ContactKind) -> bool {
    state.contacts().contacts_for_sponsor(owner).any(|contact| {
        contact.kind() == kind
            && contact.status() == ContactStatus::Active
            && state
                .world()
                .get_character(contact.handler())
                .is_some_and(|person| person.organization() == Some(owner))
            && state
                .now()
                .as_minutes()
                .saturating_sub(contact.established_at().as_minutes())
                >= (if kind == ContactKind::Political {
                    POLITICAL_CONTACT_DAYS
                } else {
                    30 // Existing shorter standing requirement for a labor contact.
                }) * DAY_MINUTES
            && (kind != ContactKind::Political
                || state
                    .world()
                    .get_organization(contact.institution())
                    .is_some_and(|organization| organization.kind() == OrganizationKind::Political))
    })
}

fn eligible_candidate(state: &AppState, owner: OrganizationId, leader: CharacterId) -> bool {
    state
        .world()
        .characters_in_organization(owner)
        .filter(|person| person.supervisor().is_none())
        .count()
        == 1
        && state.world().get_character(leader).is_some_and(|person| {
            person.organization() == Some(owner)
                && person.supervisor().is_none()
                && state.legal().active_arrest_for_character(leader).is_none()
        })
        && established_contact(state, owner, ContactKind::Political)
}

fn active_local_venues(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
) -> impl Iterator<Item = &crate::world::BusinessRecord> {
    state
        .world()
        .businesses_owned_by_organization(owner)
        .filter(move |business| business.neighborhood() == district)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some_and(|economy| {
                    economy.status() == crate::economy::BusinessOperatingStatus::Active
                })
        })
}

pub fn initiative_available(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    kind: CivicInitiative,
) -> bool {
    let infrastructure = match kind {
        CivicInitiative::PublicMeetings => {
            active_local_venues(state, owner, district).any(|business| {
                business.has_function(BusinessFunction::MeetingSpace)
                    || business.has_function(BusinessFunction::CustomerAccess)
            })
        }
        CivicInitiative::NeighborhoodServices => {
            active_local_venues(state, owner, district).any(|business| {
                matches!(
                    business.kind(),
                    BusinessKind::SoupKitchen | BusinessKind::Pharmacy | BusinessKind::Laundry
                )
            })
        }
        CivicInitiative::LaborCoalition => {
            active_local_venues(state, owner, district)
                .any(|business| business.has_function(BusinessFunction::UnionAccess))
                || established_contact(state, owner, ContactKind::Labor)
        }
        CivicInitiative::LocalPress => {
            active_local_venues(state, owner, district).any(|business| {
                business.has_function(BusinessFunction::PrintingPress)
                    || business.kind() == BusinessKind::NewsService
            })
        }
    };
    valid_scope(state, owner, district) && infrastructure
}

fn initiative_ready(
    state: &AppState,
    owner: OrganizationId,
    district: NeighborhoodId,
    manager: CharacterId,
    kind: CivicInitiative,
) -> Result<(), PoliticalError> {
    if !eligible_manager(state, owner, district, manager) {
        return Err(PoliticalError::InvalidManager);
    }
    if !initiative_available(state, owner, district, kind) {
        return Err(PoliticalError::MissingCivicInfrastructure);
    }
    if state
        .politics()
        .organization(owner)
        .and_then(|campaign| campaign.district(district))
        .and_then(|office| office.initiative_last_held(kind))
        .is_some_and(|held| {
            state.now().as_minutes().saturating_sub(held.as_minutes()) < 7 * DAY_MINUTES
        })
    {
        return Err(PoliticalError::InitiativeCooldown);
    }
    Ok(())
}

pub fn validate_civic_initiative(
    state: &AppState,
    district: NeighborhoodId,
    manager: CharacterId,
    kind: CivicInitiative,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    let owner = state
        .player_organization()
        .ok_or(PoliticalError::NotPlayer)?;
    initiative_ready(state, owner, district, manager, kind)?;
    validate_spend(
        state,
        owner,
        district,
        kind.cost_cents(),
        CampaignAction::Initiative(kind, manager),
    )
}

pub fn validate_nominate_mayor(
    state: &AppState,
    candidate: CharacterId,
) -> Result<ValidatedPoliticalOrder, PoliticalError> {
    let owner = state
        .player_organization()
        .ok_or(PoliticalError::NotPlayer)?;
    if !eligible_candidate(state, owner, candidate) {
        return Err(PoliticalError::IneligibleCandidate);
    }
    let proposed =
        next_municipal_election_day(state.now()).ok_or(PoliticalError::ElectionUnavailable)?;
    let deadline = state
        .now()
        .as_minutes()
        .checked_add(MAYORAL_FILING_LEAD_DAYS * DAY_MINUTES)
        .ok_or(PoliticalError::ElectionUnavailable)?;
    let proposed_at = proposed
        .checked_mul(DAY_MINUTES)
        .ok_or(PoliticalError::ElectionUnavailable)?;
    let election_day = if proposed_at >= deadline {
        proposed
    } else {
        proposed
            .checked_add(MUNICIPAL_ELECTION_INTERVAL_DAYS)
            .ok_or(PoliticalError::ElectionUnavailable)?
    };
    election_day
        .checked_mul(DAY_MINUTES)
        .ok_or(PoliticalError::ElectionUnavailable)?;
    if state
        .politics()
        .organization(owner)
        .and_then(OrganizationCampaign::candidacy)
        .is_some_and(|candidacy| candidacy.election_day >= proposed)
    {
        return Err(PoliticalError::AlreadyNominated);
    }
    let district = state
        .world()
        .neighborhoods()
        .next()
        .ok_or(PoliticalError::InvalidScope)?
        .id();
    validate_spend(
        state,
        owner,
        district,
        MAYORAL_FILING_FEE_CENTS,
        CampaignAction::Nominate(MayoralCandidacy {
            candidate,
            filed_at: state.now(),
            election_day,
        }),
    )
}

/// Actual personnel skill determines how much public work their contracted
/// specialists can complete. Idle appointments do not confer a fixed bonus.
pub fn specialist_output(
    state: &AppState,
    manager: CharacterId,
    kind: SpecialistKind,
) -> (u16, u16, u16) {
    let (machine, residents, institutions, capability) = match kind {
        SpecialistKind::Organizer => (8_u16, 22_u16, 2_u16, CapabilityKind::SocialAccess),
        SpecialistKind::Publicist => (18, 9, 12, CapabilityKind::Negotiation),
        SpecialistKind::Strategist => (34, 4, 28, CapabilityKind::Management),
    };
    let rating = state
        .world()
        .get_character(manager)
        .and_then(|person| person.capability(capability))
        .map_or(0, |rating| u16::from(rating.value()));
    let adjust = |base: u16| base * (60 + rating) / 100;
    (adjust(machine), adjust(residents), adjust(institutions))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampaignAction {
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
    if !campaign_office_available(state, owner, district, manager) {
        return Err(PoliticalError::ManagerOfficeConflict);
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
                    || next_municipal_election_day(state.now())
                        .is_none_or(|day| day > candidacy.election_day) =>
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
    let (grassroots, institutions) = state
        .politics()
        .organization(owner)
        .and_then(|campaign| campaign.district(district))
        .map_or((0, 0), |office| {
            (office.grassroots(), office.institutions())
        });
    own >= MAYORAL_INFLUENCE_REQUIRED
        && grassroots >= GRASSROOTS_REQUIRED
        && institutions >= INSTITUTIONAL_REQUIRED
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

/// Actual standing lost on the next daily decay pass, before paid specialist work.
/// The campaign reader uses the same arithmetic as the simulation, not a separate forecast.
pub fn daily_campaign_erosion(machine: u16, grassroots: u16, institutions: u16) -> (u16, u16, u16) {
    (
        machine.min((machine / 40).max(5)),
        grassroots.min((grassroots / 60).max(3)),
        institutions.min((institutions / 50).max(4)),
    )
}

fn apply_daily_decay(state: &mut AppState) {
    for campaign in state.politics.organizations.values_mut() {
        for district in campaign.districts.values_mut() {
            let (machine, grassroots, institutions) = daily_campaign_erosion(
                district.influence,
                district.grassroots,
                district.institutions,
            );
            district.influence -= machine;
            district.grassroots -= grassroots;
            district.institutions -= institutions;
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
            .filter(|(_, manager)| eligible_manager(state, owner, district, *manager))
            .map(|(kind, manager)| (kind, specialist_output(state, manager, kind)))
            .collect::<Vec<_>>();
        let cents: i64 = active.iter().map(|(kind, _)| kind.daily_salary()).sum();
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
            for (_, (machine, grassroots, institutions)) in active {
                office.influence = office.influence.saturating_add(machine).min(INFLUENCE_CAP);
                office.grassroots = office
                    .grassroots
                    .saturating_add(grassroots)
                    .min(INFLUENCE_CAP);
                office.institutions = office
                    .institutions
                    .saturating_add(institutions)
                    .min(INFLUENCE_CAP);
            }
        }
    }
}

/// Available clean investment after reserving one full week of current named
/// payroll across all liquid accounts. Rivals must not spend wages on elections,
/// and a cash-poor organization can buy a few real points rather than silently
/// forgoing political activity because it cannot afford the full weekly target.
pub(super) fn affordable_rival_campaign_points(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
) -> u16 {
    let members = state.world().characters_in_organization(owner).count() as i128;
    let reserve = i128::from(registry.upkeep().per_member_daily().cents()) * members * 7;
    let mut clean = 0_i128;
    let mut other = 0_i128;
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
    {
        let available = i128::from(account.available_payment_balance().cents());
        if account.kind() == AccountKind::AccountedFunds {
            clean += available;
        } else if account.kind().is_liquid() {
            // A legitimate operating pocket owned by the gang can fund wages too.
            // Only accounted funds can buy electoral influence, but every other
            // spendable kind reduces how much clean liquidity must be reserved.
            other += available;
        }
    }
    let spendable = (clean - (reserve - other).max(0)).max(0);
    u16::try_from((spendable / i128::from(INFLUENCE_COST_PER_POINT)).min(220))
        .expect("campaign points are bounded at 220")
}

fn autonomous_rival_campaigns(registry: &Registry, state: &mut AppState) {
    // Weekly initiative: defend an *operating* commercial stake against an
    // actual player challenge. Do not burn scarce rival capital on empty wards;
    // a large public player portfolio can also induce a modest preemptive defense
    // in wards where that organization owns several active businesses.
    if !(state.now().as_minutes() / DAY_MINUTES).is_multiple_of(7) {
        return;
    }
    let Some(player) = state.player_organization() else {
        return;
    };
    let player_prominence =
        crate::world::organization_pressure::public_prominence_tier(state, player);
    let player_properties_by_district = state
        .world()
        .businesses_owned_by_organization(player)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some_and(|economy| {
                    economy.status() == crate::economy::BusinessOperatingStatus::Active
                })
        })
        .fold(
            BTreeMap::<NeighborhoodId, usize>::new(),
            |mut counts, business| {
                *counts.entry(business.neighborhood()).or_default() += 1;
                counts
            },
        );
    // Take a small value snapshot before rival ledgers are mutated below.
    let player_pressure: BTreeMap<_, _> = state
        .politics()
        .organization(player)
        .map(|campaign| {
            campaign
                .districts
                .iter()
                .map(|(&district, office)| {
                    (
                        district,
                        (
                            office.influence(),
                            office.grassroots(),
                            office.institutions(),
                        ),
                    )
                })
                .collect()
        })
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
        let affordable = affordable_rival_campaign_points(registry, state, owner);
        if affordable == 0 {
            continue;
        }
        let districts = state
            .world()
            .businesses_owned_by_organization(owner)
            .filter(|business| {
                state
                    .economy()
                    .get_business_economy(business.id())
                    .is_some_and(|economy| {
                        economy.status() == crate::economy::BusinessOperatingStatus::Active
                    })
            })
            .map(|business| business.neighborhood())
            .collect::<BTreeSet<_>>();
        let choice = districts
            .into_iter()
            .filter_map(|district| {
                // A fully secured ward cannot accept more investment. Exclude it
                // before ranking, otherwise an intense but already capped player
                // challenge can indefinitely starve another operating ward.
                let own = state
                    .politics()
                    .organization(owner)
                    .and_then(|campaign| campaign.district(district))
                    .map_or(0, DistrictCampaign::influence);
                if own == INFLUENCE_CAP {
                    return None;
                }
                let (machine, grassroots, institutions) =
                    player_pressure.get(&district).copied().unwrap_or((0, 0, 0));
                let preempt = player_prominence >= 3
                    && player_properties_by_district
                        .get(&district)
                        .is_some_and(|&owned| owned >= 3);
                if machine == 0 && grassroots == 0 && institutions == 0 && !preempt {
                    return None;
                }
                // A rational adversary defends a lead instead of wasting
                // money indefinitely once its current position is sufficient.
                let desired = (u32::from(machine) * 3 / 4 + 150 + u32::from(player_prominence) * 30)
                    .min(u32::from(INFLUENCE_CAP)) as u16;
                if own >= desired {
                    return None;
                }
                Some((
                    district,
                    (
                        // A player presence that our machine already exceeds
                        // is not an urgent challenge merely because its
                        // historical office has nonzero support.
                        machine > own,
                        i32::from(desired) - i32::from(own),
                        u32::from(grassroots) + u32::from(institutions),
                        std::cmp::Reverse(district),
                    ),
                ))
            })
            .max_by_key(|(_, priority)| *priority)
            .map(|(district, _)| district);
        let Some(district) = choice else {
            continue;
        };
        let current = state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.district(district))
            .map_or(0, DistrictCampaign::influence);
        // A rising public business/political machine invites more determined
        // counter-campaigning. Rivals still need an actual local operating stake and
        // must pay the entire incremental cost from their own accounted funds.
        let weekly_budget_points = 100 + u16::from(player_prominence) * 20;
        let (machine, _, _) = player_pressure.get(&district).copied().unwrap_or((0, 0, 0));
        let desired = (u32::from(machine) * 3 / 4 + 150 + u32::from(player_prominence) * 30)
            .min(u32::from(INFLUENCE_CAP)) as u16;
        let points = (INFLUENCE_CAP - current)
            .min(desired.saturating_sub(current))
            .min(weekly_budget_points.min(220))
            .min(affordable);
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
pub fn apply_due_political_campaigns(registry: &Registry, state: &mut AppState) {
    if !is_day_boundary(state.now()) {
        return;
    }
    apply_daily_decay(state);
    pay_campaign_staff(state);
    autonomous_rival_campaigns(registry, state);
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
    apply_due_mayoral_elections(registry, state);
}

/// Proportional integer ballots using largest remainders. Truncated divisions cannot
/// silently give all unallocated votes to the final competitor (including one with
/// no support). Equal remainders favor stronger candidates, then the stable ballot
/// order. Every vote is assigned once and zero-score contenders receive none.
fn apportion_ward_votes(electorate: u32, scores: [u32; 3]) -> [u32; 3] {
    let total: u64 = scores.iter().map(|&score| u64::from(score)).sum();
    // The incumbent always has a positive base score in an actual election.
    assert!(
        total > 0,
        "a ward ballot requires at least one supported contender"
    );
    let weighted = scores.map(|score| u64::from(electorate) * u64::from(score));
    let mut votes = weighted.map(|count| (count / total) as u32);
    let mut remainders = weighted.map(|count| count % total);
    let remaining = electorate - votes.iter().sum::<u32>();
    for _ in 0..remaining {
        let index = (0..scores.len())
            .max_by_key(|&index| (remainders[index], scores[index], std::cmp::Reverse(index)))
            .expect("a ward always has three ballot contenders");
        votes[index] += 1;
        remainders[index] = 0;
    }
    votes
}

#[cfg(test)]
mod ballot_apportionment_tests {
    use super::apportion_ward_votes;

    #[test]
    fn unsupported_contenders_receive_no_rounding_votes() {
        assert_eq!(apportion_ward_votes(11, [2, 3, 0]), [4, 7, 0]);
        assert_eq!(apportion_ward_votes(9, [0, 4, 0]), [0, 9, 0]);
        assert_eq!(apportion_ward_votes(1, [0, 0, 8]), [0, 0, 1]);
    }

    #[test]
    fn proportional_rounding_allocates_every_vote_and_breaks_ties_stably() {
        assert_eq!(apportion_ward_votes(5, [1, 1, 1]), [2, 2, 1]);
        assert_eq!(apportion_ward_votes(1, [3, 3, 0]), [1, 0, 0]);
        assert_eq!(
            apportion_ward_votes(u32::MAX, [1, 1, 0]),
            [u32::MAX / 2 + 1, u32::MAX / 2, 0]
        );
        for electorate in 0..=100 {
            for scores in [[0, 1, 0], [1, 1, 0], [1, 2, 3], [97, 0, 4]] {
                let votes = apportion_ward_votes(electorate, scores);
                assert_eq!(votes.iter().sum::<u32>(), electorate);
                for index in 0..scores.len() {
                    if scores[index] == 0 {
                        assert_eq!(votes[index], 0);
                    }
                }
            }
        }
    }
}

fn apply_due_mayoral_elections(registry: &Registry, state: &mut AppState) {
    let day = state.now().as_minutes() / DAY_MINUTES;
    if day == 0 || !day.is_multiple_of(MUNICIPAL_ELECTION_INTERVAL_DAYS) {
        return;
    }
    let Some(owner) = state.player_organization() else {
        return;
    };
    let Some(candidacy) = state
        .politics()
        .organization(owner)
        .and_then(OrganizationCampaign::candidacy)
        .filter(|entry| entry.election_day == day)
    else {
        return;
    };

    // Ballots use only current world state, the real campaign ledger, local
    // support and crime reputation. There is no preselected winning candidate.
    let Ok(progress) = crate::core::campaign_victory::progress(registry, state) else {
        return;
    };
    let Ok(districts) = crate::core::campaign_victory::political_district_progress(registry, state)
    else {
        return;
    };
    let eligible = eligible_candidate(state, owner, candidacy.candidate)
        && crate::core::campaign_victory::mayoral_ballot_ready(&progress);
    let incumbent = state
        .world()
        .organizations()
        .filter(|organization| organization.kind() == OrganizationKind::Political)
        .flat_map(|organization| state.world().characters_in_organization(organization.id()))
        .filter(|person| {
            person.supervisor().is_none()
                && state
                    .legal()
                    .active_arrest_for_character(person.id())
                    .is_none()
        })
        .max_by_key(|person| {
            let negotiating = person
                .capability(CapabilityKind::Negotiation)
                .map_or(0, |rating| rating.value());
            let access = person
                .capability(CapabilityKind::SocialAccess)
                .map_or(0, |rating| rating.value());
            (
                u32::from(negotiating)
                    + u32::from(access)
                    + u32::from(person.life().professional_standing()),
                std::cmp::Reverse(person.id()),
            )
        });
    let incumbent_id = incumbent.map(|person| person.id());
    let incumbent_strength = incumbent.map_or(175_u32, |person| {
        u32::from(
            person
                .capability(CapabilityKind::Negotiation)
                .map_or(0, |rating| rating.value()),
        ) + u32::from(
            person
                .capability(CapabilityKind::SocialAccess)
                .map_or(0, |rating| rating.value()),
        ) + u32::from(person.life().professional_standing())
    });
    let nominee = state
        .world()
        .get_character(candidacy.candidate)
        .expect("a filed mayoral candidate still exists");
    let nominee_strength = u32::from(
        nominee
            .capability(CapabilityKind::Negotiation)
            .map_or(0, |rating| rating.value()),
    ) / 2
        + u32::from(nominee.life().professional_standing()) / 2;
    let fear = resolve_score(registry, state.reputation(), owner, AudienceKind::Residents);
    let excess_fear = u32::from(fear.saturating_sub(registry.reputation().baseline()));
    let mut ballots = Vec::new();
    for ward in state.world().neighborhoods() {
        let district = ward.id();
        let office = state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.district(district));
        let machine = u32::from(office.map_or(0, DistrictCampaign::influence));
        let grassroots = u32::from(office.map_or(0, DistrictCampaign::grassroots));
        let institutions = u32::from(office.map_or(0, DistrictCampaign::institutions));
        let local_rackets = state
            .enterprises()
            .enterprises_for_organization(owner)
            .filter(|racket| racket.status() == crate::enterprises::EnterpriseStatus::Active)
            .filter(|racket| match racket.location() {
                crate::enterprises::EnterpriseLocation::Neighborhood(id) => id == district,
                crate::enterprises::EnterpriseLocation::Business(id) => state
                    .world()
                    .get_business(id)
                    .is_some_and(|business| business.neighborhood() == district),
            })
            .count() as u32;
        let candidate_score =
            (machine * 50 + grassroots * 30 + institutions * 20) / 100 + nominee_strength;
        let candidate_score =
            candidate_score.saturating_sub(excess_fear * 5 + local_rackets.saturating_mul(25));
        let incumbent_score = 320_u32
            + incumbent_strength
            + u32::from(ward.profile().institutions.police_presence.value()) / 2;
        let rival_score = u32::from(rival_pressure(state, owner, district)) * 3 / 5;
        let electorate = 5_000_u32
            + u32::from(ward.profile().economy.wealth.value()) * 30
            + u32::from(ward.profile().economy.commercial_activity.value()) * 20;
        let [candidate_votes, incumbent_votes, rival_votes] =
            apportion_ward_votes(electorate, [candidate_score, incumbent_score, rival_score]);
        let local_ready = districts
            .iter()
            .any(|entry| entry.district == district && entry.mayoral_ready);
        ballots.push(WardBallot {
            district,
            candidate_votes,
            incumbent_votes,
            rival_votes,
            ward_won: local_ready
                && candidate_votes > incumbent_votes
                && candidate_votes > rival_votes,
        });
    }
    let wards_won = ballots.iter().filter(|ballot| ballot.ward_won).count();
    let elected = eligible && wards_won >= progress.required_political_districts;
    let result = MayoralElection {
        candidate: candidacy.candidate,
        incumbent: incumbent_id,
        election_day: day,
        held_at: state.now(),
        wards_needed: progress.required_political_districts,
        wards_won,
        eligible,
        elected,
        ballots,
    };
    let campaign = state
        .politics
        .organizations
        .get_mut(&owner)
        .expect("nominated candidate has a campaign");
    campaign.candidacy = None;
    campaign.elections.push(result);

    // The public election notice is an ordinary Standing report; the full
    // verified ballot ledger remains inspectable even if report IDs run out.
    let summary = if elected {
        format!(
            "Municipal election day {day}: the nominated leader won {wards_won} of {} wards and is mayor-elect. Certify with 'claim-victory political'.",
            progress.total_districts
        )
    } else {
        format!(
            "Municipal election day {day}: the nominated leader won {wards_won} of {} wards; {} needed. The mayoral campaign was defeated; another election can be contested.",
            progress.total_districts, progress.required_political_districts
        )
    };
    let report = crate::reports::report_system::validate_record_report(
        state,
        crate::reports::ReportDraft {
            recipient: owner,
            kind: crate::reports::ReportKind::Standing,
            title: format!("Municipal election day {day}"),
            entries: vec![crate::reports::ReportEntry {
                attention: if elected {
                    crate::core::attention::AttentionClass::Crisis
                } else {
                    crate::core::attention::AttentionClass::Notable
                },
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([crate::core::entity::EntityRef::Character(
                    candidacy.candidate,
                )]),
                decision: None,
            }],
        },
    );
    if let Ok(report) = report {
        let _ = report.commit(state);
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
                !matches!(account.owner(), FinancialOwner::Organization(institution)
                    if state.world().get_organization(institution).is_some_and(
                        |organization| organization.kind() == OrganizationKind::Political))
                    || account.kind() != AccountKind::Settlement
            })
        {
            return false;
        }
        // A manager can retain several specialist contracts in one ward, but
        // never multiple offices within the same employer's campaign. Current
        // saves must satisfy the same restriction as hiring and held-order commit.
        let mut manager_offices = BTreeMap::new();
        for (&district, office) in &campaign.districts {
            if state.world().get_neighborhood(district).is_none()
                || office.influence > INFLUENCE_CAP
                || office.grassroots > INFLUENCE_CAP
                || office.institutions > INFLUENCE_CAP
                || office
                    .leading_since
                    .is_some_and(|start| start > state.now())
                || office.specialists.values().any(|manager| {
                    state.world().get_character(*manager).is_none()
                        || manager_offices
                            .insert(*manager, district)
                            .is_some_and(|previous| previous != district)
                })
                || office.initiatives.values().any(|time| *time > state.now())
            {
                return false;
            }
        }
        if let Some(candidacy) = campaign.candidacy {
            let Some(vote_at) = candidacy.election_day.checked_mul(DAY_MINUTES) else {
                return false;
            };
            if candidacy.filed_at > state.now()
                || vote_at <= state.now().as_minutes()
                || !candidacy
                    .election_day
                    .is_multiple_of(MUNICIPAL_ELECTION_INTERVAL_DAYS)
                || vote_at.saturating_sub(candidacy.filed_at.as_minutes())
                    < MAYORAL_FILING_LEAD_DAYS * DAY_MINUTES
                || state.world().get_character(candidacy.candidate).is_none()
            {
                return false;
            }
        }
        let mut previous_day = 0;
        for election in &campaign.elections {
            let Some(vote_at) = election.election_day.checked_mul(DAY_MINUTES) else {
                return false;
            };
            if election.election_day <= previous_day
                || !election
                    .election_day
                    .is_multiple_of(MUNICIPAL_ELECTION_INTERVAL_DAYS)
                || election.held_at.as_minutes() != vote_at
                || election.held_at > state.now()
                || state.world().get_character(election.candidate).is_none()
                || election.incumbent.is_some_and(|incumbent| {
                    state.world().get_character(incumbent).is_none()
                        || incumbent == election.candidate
                })
                || election.wards_needed != (state.world().neighborhoods().count() * 3).div_ceil(4)
                || election.ballots.len() != state.world().neighborhoods().count()
                || election.wards_won != election.ballots.iter().filter(|b| b.ward_won).count()
                || election.elected
                    != (election.eligible && election.wards_won >= election.wards_needed)
            {
                return false;
            }
            let mut seen = BTreeSet::new();
            for ballot in &election.ballots {
                let Some(district) = state.world().get_neighborhood(ballot.district) else {
                    return false;
                };
                let votes = ballot.candidate_votes as u64
                    + ballot.incumbent_votes as u64
                    + ballot.rival_votes as u64;
                let expected_votes = 5_000_u64
                    + u64::from(district.profile().economy.wealth.value()) * 30
                    + u64::from(district.profile().economy.commercial_activity.value()) * 20;
                if !seen.insert(ballot.district)
                    || votes != expected_votes
                    || (ballot.ward_won
                        && (ballot.candidate_votes <= ballot.incumbent_votes
                            || ballot.candidate_votes <= ballot.rival_votes))
                {
                    return false;
                }
            }
            previous_day = election.election_day;
        }
    }
    true
}
