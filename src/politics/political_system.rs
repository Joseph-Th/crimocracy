//! Authoritative political expenditures and daily electoral competition.
//! Campaign actions settle through the real balanced ledger before influence or
//! specialist contracts change; all future-affecting standing is in AppState.

use super::{
    CivicInitiative, DistrictCampaign, GRASSROOTS_REQUIRED, INFLUENCE_CAP,
    INFLUENCE_COST_PER_POINT, INSTITUTIONAL_REQUIRED, MAYORAL_FILING_FEE_CENTS,
    MAYORAL_FILING_LEAD_DAYS, MAYORAL_INFLUENCE_REQUIRED, MAYORAL_SUSTAINED_DAYS,
    MUNICIPAL_ELECTION_INTERVAL_DAYS, MayoralCandidacy, OrganizationCampaign, SpecialistKind,
};
use crate::contacts::ContactKind;
use crate::contacts::contact_system::usable_contacts_of_kind;
use crate::core::campaign_victory::POLITICAL_CONTACT_DAYS;
use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::Money;
use crate::finance::finance_system::FinanceError;
use crate::world::{
    AutonomyLevel, BusinessFunction, BusinessKind, CapabilityKind, OrganizationKind,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod daily;
mod election;
mod expenditure;
#[cfg(test)]
pub(super) use daily::affordable_rival_campaign_points;
pub(crate) use daily::apply_due_political_campaigns;
pub use daily::daily_campaign_erosion;
pub(crate) use election::apply_due_mayoral_elections;
pub use expenditure::ValidatedPoliticalOrder;
use expenditure::{CampaignAction, valid_campaign_expenditure, validate_spend};

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
/// Next ballot's elapsed-day ordinal, not the one-based display date.
pub fn next_municipal_election_day(now: SimTime) -> Option<u64> {
    let day = (now.as_minutes() / DAY_MINUTES / MUNICIPAL_ELECTION_INTERVAL_DAYS)
        .checked_add(1)?
        .checked_mul(MUNICIPAL_ELECTION_INTERVAL_DAYS)?;
    SimTime::from_elapsed_days(day).map(|_| day)
}

/// Earliest scheduled ballot for which a new filing still satisfies the
/// minimum lead time. Unlike the next calendar election, this can skip a ballot
/// after its filing cutoff and can be unavailable even before the final ballot.
pub fn next_mayoral_nomination_day(now: SimTime) -> Option<u64> {
    let next = next_municipal_election_day(now)?;
    let deadline = now
        .as_minutes()
        .checked_add(MAYORAL_FILING_LEAD_DAYS * DAY_MINUTES)?;
    let election_day = if SimTime::from_elapsed_days(next)?.as_minutes() >= deadline {
        next
    } else {
        next.checked_add(MUNICIPAL_ELECTION_INTERVAL_DAYS)?
    };
    SimTime::from_elapsed_days(election_day).map(|_| election_day)
}

fn established_contact(state: &AppState, owner: OrganizationId, kind: ContactKind) -> bool {
    let required_days = if kind == ContactKind::Political {
        POLITICAL_CONTACT_DAYS
    } else {
        30 // Existing shorter standing requirement for a labor contact.
    };
    usable_contacts_of_kind(state, owner, kind).any(|contact| {
        state
            .now()
            .as_minutes()
            .saturating_sub(contact.established_at().as_minutes())
            >= required_days * DAY_MINUTES
    })
}

fn eligible_candidate(state: &AppState, owner: OrganizationId, leader: CharacterId) -> bool {
    crate::world::world_system::sole_available_executive(state, owner) == Some(leader)
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
    let election_day =
        next_mayoral_nomination_day(state.now()).ok_or(PoliticalError::ElectionUnavailable)?;
    if state
        .politics()
        .organization(owner)
        .and_then(OrganizationCampaign::candidacy)
        .is_some()
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

/// Reconcile the uninterrupted majority clock for every campaign in the affected ward.
/// A paid initiative or investment can change both the investor's standing and a rival's.
/// Daily maintenance calls this only after decay, salaries and rival spending have settled,
/// so temporary intermediate scores inside that one minute are not observed as lost days.
fn reconcile_district_majorities(state: &mut AppState, district: NeighborhoodId) {
    let owners: Vec<_> = state
        .politics()
        .organizations
        .iter()
        .filter(|(_, campaign)| campaign.district(district).is_some())
        .map(|(&owner, _)| owner)
        .collect();
    let now = state.now();
    for owner in owners {
        let leading = controls_district(state, owner, district);
        let office = state
            .politics
            .organizations
            .get_mut(&owner)
            .and_then(|campaign| campaign.districts.get_mut(&district))
            .expect("district campaign exists");
        if leading {
            office.leading_since.get_or_insert(now);
        } else {
            office.leading_since = None;
        }
    }
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

/// Release-safe save/invariant checks. Historical contracts may refer to
/// former or detained employees, but the daily phase never pays them.
pub fn valid_political_state(state: &AppState) -> bool {
    let mut clearing_accounts = BTreeSet::new();
    for (&owner, campaign) in &state.politics().organizations {
        if state
            .world()
            .get_organization(owner)
            .is_none_or(|record| record.kind() != OrganizationKind::Criminal)
        {
            return false;
        }
        let Some(clearing) = campaign.clearing_account else {
            return false;
        };
        if !clearing_accounts.insert(clearing)
            || !valid_campaign_expenditure(state, owner, campaign.spent_cents, clearing)
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
                || office.leading_since.is_some() != controls_district(state, owner, district)
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
        if let Some(candidacy) = campaign.candidacy
            && (candidacy.filed_at > state.now()
                || next_mayoral_nomination_day(candidacy.filed_at) != Some(candidacy.election_day)
                || SimTime::from_elapsed_days(candidacy.election_day)
                    .is_none_or(|vote_at| vote_at <= state.now())
                || state.world().get_character(candidacy.candidate).is_none())
        {
            return false;
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
