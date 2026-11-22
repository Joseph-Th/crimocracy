//! Named civilian and institutional people make their own deterministic work
//! decisions. An occupation is derived from current membership and actual
//! property titles, not assigned in a parallel roster that can drift from world
//! ownership. This subsystem owns only the personal work/condition snapshot.

use super::{
    CharacterLife, CivicActivity, CivicProfession, IndependentVocation, OrganizationKind,
    WorldState,
};
use crate::core::id::{BusinessId, CharacterId, NeighborhoodId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, is_day_boundary};
use crate::core::version::ensure_version_can_advance;
use crate::economy::BusinessOperatingStatus;
use crate::legal::{InvestigationStatus, PatrolDeploymentStatus};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CivicPersonError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("business {0} is not a workplace in this neighborhood")]
    InvalidWorkplace(BusinessId),
    #[error("character {0} is not an unaffiliated non-owner")]
    NotIndependent(CharacterId),
    #[error("character {0} cannot change vocation during a signed freelance contract")]
    ActiveFreelanceContract(CharacterId),
    #[error("character {0} already has this occupation and home")]
    Unchanged(CharacterId),
    #[error("character {0} changed after occupation validation")]
    StaleCharacter(CharacterId),
    #[error(transparent)]
    Version(#[from] crate::core::version::VersionCapacityError),
}

#[derive(Debug)]
pub struct ValidatedCivicOccupation {
    character: CharacterId,
    home: NeighborhoodId,
    vocation: IndependentVocation,
    expected_version: u32,
}

impl ValidatedCivicOccupation {
    pub fn commit(self, state: &mut AppState) -> Result<(), CivicPersonError> {
        let current = state
            .world()
            .get_character(self.character)
            .ok_or(CivicPersonError::MissingCharacter(self.character))?;
        if current.version() != self.expected_version {
            return Err(CivicPersonError::StaleCharacter(self.character));
        }
        validate_occupation(state, self.character, self.home, self.vocation, false)?;
        state
            .world
            .set_character_vocation(self.character, self.home, self.vocation);
        Ok(())
    }
}

pub fn validate_set_civic_occupation(
    state: &AppState,
    character: CharacterId,
    home: NeighborhoodId,
    vocation: IndependentVocation,
) -> Result<ValidatedCivicOccupation, CivicPersonError> {
    validate_occupation(state, character, home, vocation, true)?;
    Ok(ValidatedCivicOccupation {
        character,
        home,
        vocation,
        expected_version: state
            .world()
            .get_character(character)
            .expect("validated person exists")
            .version(),
    })
}

fn validate_occupation(
    state: &AppState,
    character: CharacterId,
    home: NeighborhoodId,
    vocation: IndependentVocation,
    reject_unchanged: bool,
) -> Result<(), CivicPersonError> {
    let person = state
        .world()
        .get_character(character)
        .ok_or(CivicPersonError::MissingCharacter(character))?;
    if state.world().active_freelance_contract(character).is_some()
        && person.life().vocation() != vocation
    {
        return Err(CivicPersonError::ActiveFreelanceContract(character));
    }
    if (person.organization().is_some() && vocation != IndependentVocation::Resident)
        || (person.supervisor().is_some() && vocation != IndependentVocation::Resident)
        || (state
            .world()
            .businesses_owned_by_character(character)
            .next()
            .is_some()
            && vocation != IndependentVocation::Resident)
    {
        return Err(CivicPersonError::NotIndependent(character));
    }
    if state.world().get_neighborhood(home).is_none() {
        return Err(CivicPersonError::MissingNeighborhood(home));
    }
    if let IndependentVocation::BusinessEmployee(workplace) = vocation
        && !state
            .world()
            .get_business(workplace)
            .is_some_and(|business| business.neighborhood() == home)
    {
        return Err(CivicPersonError::InvalidWorkplace(workplace));
    }
    if reject_unchanged
        && person.life().home() == Some(home)
        && person.life().vocation() == vocation
    {
        return Err(CivicPersonError::Unchanged(character));
    }
    ensure_version_can_advance(person.version(), "character")?;
    Ok(())
}

impl WorldState {
    /// A person who changes jobs or acquires a storefront changes profession
    /// immediately without an inert occupation flag to keep synchronized.
    pub fn civic_profession(&self, character: CharacterId) -> Option<CivicProfession> {
        let person = self.get_character(character)?;
        if self
            .businesses_owned_by_character(character)
            .next()
            .is_some()
        {
            return Some(CivicProfession::BusinessOwner);
        }
        let profession = person.organization().and_then(|organization| {
            self.get_organization(organization)
                .map(|record| match record.kind() {
                    OrganizationKind::LawEnforcement => CivicProfession::PoliceOfficer,
                    OrganizationKind::Political => CivicProfession::Politician,
                    OrganizationKind::LegalServices | OrganizationKind::LegalAuthority => {
                        CivicProfession::Lawyer
                    }
                    OrganizationKind::Prosecutor => CivicProfession::Prosecutor,
                    OrganizationKind::Labor => CivicProfession::LaborRepresentative,
                    OrganizationKind::Commercial | OrganizationKind::Civic => {
                        CivicProfession::MerchantRepresentative
                    }
                    OrganizationKind::Criminal => CivicProfession::Criminal,
                })
        });
        Some(profession.unwrap_or(match person.life().vocation() {
            IndependentVocation::Resident => CivicProfession::Resident,
            IndependentVocation::FreelanceCriminal => CivicProfession::FreelanceCriminal,
            IndependentVocation::BusinessEmployee(_) => CivicProfession::BusinessEmployee,
            IndependentVocation::Journalist => CivicProfession::Journalist,
            IndependentVocation::IndependentLawyer => CivicProfession::IndependentLawyer,
        }))
    }

    /// Public appointments and registered sole proprietors; this is not an
    /// intelligence feed for rival organizations' private personnel.
    pub fn public_civic_people(&self) -> impl Iterator<Item = &super::CharacterRecord> {
        self.characters().filter(|person| {
            self.businesses_owned_by_character(person.id())
                .next()
                .is_some()
                || person
                    .organization()
                    .and_then(|owner| self.get_organization(owner))
                    .is_some_and(|organization| {
                        !matches!(organization.kind(), OrganizationKind::Criminal)
                    })
                || (person.organization().is_none()
                    && matches!(
                        person.life().vocation(),
                        IndependentVocation::BusinessEmployee(_)
                            | IndependentVocation::Journalist
                            | IndependentVocation::IndependentLawyer
                    ))
        })
    }
}

#[derive(Clone, Copy)]
struct DayWork {
    action: Option<CivicActivity>,
    business_profit: i8,
    detained: bool,
}

fn choose_work(state: &AppState, character: CharacterId, day: u64) -> DayWork {
    let detained = state
        .legal()
        .active_arrest_for_character(character)
        .is_some();
    let person = state
        .world()
        .get_character(character)
        .expect("known character");
    // One rest day per person per week, staggered by persistent identity.
    // A universal day-seven holiday would leave every police beat and public
    // institution without any named on-duty staff at the same time.
    if detained || (day + u64::from(character.raw())).is_multiple_of(7) {
        return DayWork {
            action: None,
            business_profit: 0,
            detained,
        };
    }
    let owned = state
        .world()
        .businesses_owned_by_character(character)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some_and(|record| record.status() == BusinessOperatingStatus::Active)
        })
        .map(|business| {
            let cycle = state.economy().cycles_for(business.id()).next_back();
            let profit = cycle.map_or(0, |cycle| cycle.net_cash().cents());
            (business.id(), profit)
        })
        // The owner visits the most troubled active business, not necessarily
        // the earliest property in its personal portfolio.
        .min_by_key(|(id, profit)| (*profit, *id));
    if let Some((business, profit)) = owned {
        return DayWork {
            action: Some(CivicActivity::BusinessManagement(business)),
            business_profit: if profit > 0 {
                1
            } else if profit < 0 {
                -1
            } else {
                0
            },
            detained,
        };
    }
    let action = person
        .organization()
        .and_then(|organization| {
            state
                .world()
                .get_organization(organization)
                .map(|record| (organization, record.kind()))
        })
        .and_then(|(organization, kind)| match kind {
            OrganizationKind::LawEnforcement => {
                // Casework has priority over patrol, exactly as the legal owner
                // allocates named detectives. Unassigned officers cover their
                // authority's active deployment on a rotating daily route.
                if let Some(case) = state
                    .legal()
                    .active_investigation_for_investigator(character)
                    .filter(|case| case.status() == InvestigationStatus::Active)
                {
                    Some(CivicActivity::DetectiveWork(case.id()))
                } else {
                    let districts: Vec<_> = state
                        .world()
                        .neighborhoods()
                        .filter(|ward| {
                            state
                                .legal()
                                .active_patrol_deployments_for_neighborhood(ward.id())
                                .any(|deployment| {
                                    deployment.organization() == organization
                                        && deployment.status() == PatrolDeploymentStatus::Active
                                })
                        })
                        .map(|ward| ward.id())
                        .collect();
                    let assigned = person.life().home().filter(|home| districts.contains(home));
                    assigned
                        .or_else(|| {
                            let position =
                                (day + u64::from(character.raw())) % districts.len().max(1) as u64;
                            districts.get(position as usize).copied()
                        })
                        .map(CivicActivity::PolicePatrol)
                }
            }
            OrganizationKind::Political => Some(CivicActivity::Politics(organization)),
            OrganizationKind::LegalServices | OrganizationKind::LegalAuthority => {
                Some(CivicActivity::LegalService(organization))
            }
            OrganizationKind::Prosecutor => Some(CivicActivity::Prosecution(organization)),
            OrganizationKind::Labor => Some(CivicActivity::LaborAdvocacy(organization)),
            OrganizationKind::Commercial | OrganizationKind::Civic => {
                Some(CivicActivity::CommercialAdvocacy(organization))
            }
            OrganizationKind::Criminal => None, // Already acts through crews and enterprises.
        });
    let action = action.or_else(|| {
        if person.organization().is_some() {
            return None;
        }
        let home = person.life().home()?;
        match person.life().vocation() {
            IndependentVocation::Resident => Some(CivicActivity::Community),
            IndependentVocation::FreelanceCriminal => Some(CivicActivity::FreelanceWork(home)),
            IndependentVocation::BusinessEmployee(workplace) => state
                .economy()
                .get_business_economy(workplace)
                .filter(|economy| economy.status() == BusinessOperatingStatus::Active)
                .map(|_| CivicActivity::BusinessShift(workplace)),
            IndependentVocation::Journalist => Some(CivicActivity::Journalism(home)),
            IndependentVocation::IndependentLawyer => {
                Some(CivicActivity::IndependentLegalPractice(home))
            }
        }
    });
    DayWork {
        action,
        business_profit: 0,
        detained,
    }
}

fn finish_day(
    mut life: CharacterLife,
    today: crate::core::time::SimTime,
    work: DayWork,
) -> CharacterLife {
    match work.action {
        Some(action) => {
            life.career_days = life.career_days.saturating_add(1);
            life.last_activity = Some((today, action));
            let standing_change: i16 = match action {
                CivicActivity::BusinessManagement(_) => match work.business_profit {
                    1 => 1,
                    -1 => -2,
                    _ => 0,
                },
                CivicActivity::DetectiveWork(_) => 2,
                CivicActivity::PolicePatrol(_) => 1,
                CivicActivity::Politics(_)
                | CivicActivity::LegalService(_)
                | CivicActivity::Prosecution(_)
                | CivicActivity::LaborAdvocacy(_)
                | CivicActivity::CommercialAdvocacy(_) => 1,
                CivicActivity::Community => 0,
                CivicActivity::BusinessShift(_)
                | CivicActivity::Journalism(_)
                | CivicActivity::IndependentLegalPractice(_) => 1,
                CivicActivity::FreelanceWork(_) => 0,
            };
            life.professional_standing =
                (i16::from(life.professional_standing) + standing_change).clamp(0, 100) as u8;
            life.fatigue = life
                .fatigue
                .saturating_add(match action {
                    CivicActivity::DetectiveWork(_) => 4,
                    CivicActivity::BusinessManagement(_) if work.business_profit < 0 => 5,
                    CivicActivity::BusinessManagement(_)
                    | CivicActivity::PolicePatrol(_)
                    | CivicActivity::Politics(_)
                    | CivicActivity::LegalService(_)
                    | CivicActivity::Prosecution(_)
                    | CivicActivity::LaborAdvocacy(_)
                    | CivicActivity::CommercialAdvocacy(_)
                    | CivicActivity::Community
                    | CivicActivity::BusinessShift(_)
                    | CivicActivity::Journalism(_)
                    | CivicActivity::IndependentLegalPractice(_) => 2,
                    CivicActivity::FreelanceWork(_) => 4,
                })
                .min(100);
        }
        None => {
            life.fatigue = life
                .fatigue
                .saturating_sub(if work.detained { 2 } else { 14 });
            if work.detained {
                life.professional_standing = life.professional_standing.saturating_sub(1);
            }
        }
    }
    life
}

/// Daily civilian choices happen after business settlement and legal assignments.
/// They use only persisted identity, capability, property and case records and do
/// not consume RNG or mutate authority/character assignment versions.
pub fn apply_due_character_lives(state: &mut AppState) -> usize {
    if !is_day_boundary(state.now()) {
        return 0;
    }
    let day = state.now().as_minutes() / DAY_MINUTES;
    let changes: Vec<_> = state
        .world()
        .characters()
        .map(|person| {
            let work = choose_work(state, person.id(), day);
            (person.id(), finish_day(person.life(), state.now(), work))
        })
        .collect();
    let mut changed = 0;
    for (id, life) in changes {
        if state
            .world()
            .get_character(id)
            .is_some_and(|person| person.life() != life)
        {
            state.world.update_character_life(id, life);
            changed += 1;
        }
    }
    changed
}

pub(crate) fn valid_character_lives(state: &AppState) -> bool {
    state.world().characters().all(|person| {
        let life = person.life();
        life.professional_standing() <= 100
            && life.fatigue() <= 100
            && u64::from(life.career_days()) <= state.now().as_minutes() / DAY_MINUTES
            && life.last_activity().is_none_or(|(at, action)| {
                at <= state.now()
                    && at.as_minutes() > 0
                    && at.as_minutes().is_multiple_of(DAY_MINUTES)
                    && match action {
                        CivicActivity::BusinessManagement(id) => {
                            state.world().get_business(id).is_some()
                        }
                        CivicActivity::PolicePatrol(id) => {
                            state.world().get_neighborhood(id).is_some()
                        }
                        CivicActivity::DetectiveWork(id) => {
                            state.legal().get_investigation(id).is_some()
                        }
                        CivicActivity::Politics(id)
                        | CivicActivity::LegalService(id)
                        | CivicActivity::Prosecution(id)
                        | CivicActivity::LaborAdvocacy(id)
                        | CivicActivity::CommercialAdvocacy(id) => {
                            state.world().get_organization(id).is_some()
                        }
                        CivicActivity::Community => true,
                        CivicActivity::BusinessShift(id) => {
                            state.world().get_business(id).is_some()
                        }
                        CivicActivity::FreelanceWork(id)
                        | CivicActivity::Journalism(id)
                        | CivicActivity::IndependentLegalPractice(id) => {
                            state.world().get_neighborhood(id).is_some()
                        }
                    }
            })
            && life
                .home()
                .is_none_or(|home| state.world().get_neighborhood(home).is_some())
            && match life.vocation() {
                IndependentVocation::BusinessEmployee(business) => state
                    .world()
                    .get_business(business)
                    .is_some_and(|record| Some(record.neighborhood()) == life.home()),
                IndependentVocation::Resident
                | IndependentVocation::FreelanceCriminal
                | IndependentVocation::Journalist
                | IndependentVocation::IndependentLawyer => true,
            }
    })
}
