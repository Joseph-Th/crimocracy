//! Named civilian and institutional people make their own deterministic work
//! decisions. An occupation is derived from current membership and actual
//! property titles, not assigned in a parallel roster that can drift from world
//! ownership. This subsystem owns only the personal work/condition snapshot.

use super::{
    CharacterLife, CivicActivity, CivicProfession, IndependentVocation, OrganizationKind,
    WorldState,
};
use crate::core::id::{BusinessId, CharacterId, NeighborhoodId, OrganizationId};
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
        self.get_character(character)
            .map(|person| self.profession_for_person(person))
    }

    fn profession_for_person(&self, person: &super::CharacterRecord) -> CivicProfession {
        if self
            .businesses_owned_by_character(person.id())
            .next()
            .is_some()
        {
            return CivicProfession::BusinessOwner;
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
        profession.unwrap_or(match person.life().vocation() {
            IndependentVocation::Resident => CivicProfession::Resident,
            IndependentVocation::FreelanceCriminal => CivicProfession::FreelanceCriminal,
            IndependentVocation::BusinessEmployee(_) => CivicProfession::BusinessEmployee,
            IndependentVocation::Journalist => CivicProfession::Journalist,
            IndependentVocation::IndependentLawyer => CivicProfession::IndependentLawyer,
        })
    }

    /// Public appointments and registered sole proprietors; this is not an
    /// intelligence feed for rival organizations' private personnel.
    pub fn public_civic_people(&self) -> impl Iterator<Item = &super::CharacterRecord> {
        self.characters()
            .filter(|person| match self.profession_for_person(person) {
                CivicProfession::BusinessOwner
                | CivicProfession::PoliceOfficer
                | CivicProfession::Politician
                | CivicProfession::Lawyer
                | CivicProfession::Prosecutor
                | CivicProfession::LaborRepresentative
                | CivicProfession::MerchantRepresentative
                | CivicProfession::BusinessEmployee
                | CivicProfession::Journalist
                | CivicProfession::IndependentLawyer => true,
                CivicProfession::Resident
                | CivicProfession::FreelanceCriminal
                | CivicProfession::Criminal => false,
            })
    }
}

#[derive(Clone, Copy)]
struct DayWork {
    action: Option<CivicActivity>,
    business_profit: i8,
    detained: bool,
}

fn is_rest_day(character: CharacterId, day: u64) -> bool {
    (day + u64::from(character.raw()) % 7).is_multiple_of(7)
}

/// Working days in the calendar from the first day boundary through this
/// activity, before accounting for detention or periods without a job.
fn maximum_workdays(character: CharacterId, day: u64) -> u64 {
    day - (day + u64::from(character.raw()) % 7) / 7
}

/// Fixed institutional duties share their immutable organization-kind mapping
/// with historical activity validation. Police work depends on live assignments.
fn institutional_work(
    organization: OrganizationId,
    kind: OrganizationKind,
) -> Option<CivicActivity> {
    match kind {
        OrganizationKind::Political => Some(CivicActivity::Politics(organization)),
        OrganizationKind::LegalServices | OrganizationKind::LegalAuthority => {
            Some(CivicActivity::LegalService(organization))
        }
        OrganizationKind::Prosecutor => Some(CivicActivity::Prosecution(organization)),
        OrganizationKind::Labor => Some(CivicActivity::LaborAdvocacy(organization)),
        OrganizationKind::Commercial | OrganizationKind::Civic => {
            Some(CivicActivity::CommercialAdvocacy(organization))
        }
        OrganizationKind::LawEnforcement | OrganizationKind::Criminal => None,
    }
}

/// A storefront either remains open or completed a settlement during the
/// preceding workday. The final settled day counts for both proprietors and
/// employees even if the settlement closed the shop before this phase.
/// Only a fresh result in the current title and operating period can affect
/// standing; an active shop that missed a settlement has neutral earnings.
fn business_work_profit(state: &AppState, business: &super::BusinessRecord) -> Option<i64> {
    let economy = state.economy().get_business_economy(business.id())?;
    let cycle = state
        .economy()
        .cycles_for(business.id())
        .next_back()
        .filter(|cycle| {
            cycle.business_version() == business.version()
                && state.now().as_minutes() - cycle.occurred_at().as_minutes() < DAY_MINUTES
                && economy
                    .loss_streak_anchor()
                    .is_none_or(|anchor| cycle.occurred_at() > anchor)
        });
    if economy.status() != BusinessOperatingStatus::Active && cycle.is_none() {
        return None;
    }
    Some(cycle.map_or(0, |cycle| cycle.net_cash().cents()))
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
    if detained || is_rest_day(character, day) {
        return DayWork {
            action: None,
            business_profit: 0,
            detained,
        };
    }
    let mut properties = state
        .world()
        .businesses_owned_by_character(character)
        .peekable();
    let proprietor = properties.peek().is_some();
    let owned = properties
        .filter_map(|business| {
            business_work_profit(state, business).map(|profit| (business.id(), profit))
        })
        // The owner visits the most troubled business that worked this day
        // or remains active, not necessarily the earliest property.
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
    // A title defines the current profession even when every property is closed.
    // The old independent vocation is a fallback only after ownership ends.
    if proprietor {
        return DayWork {
            action: None,
            business_profit: 0,
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
            kind @ (OrganizationKind::Criminal
            | OrganizationKind::LegalAuthority
            | OrganizationKind::LegalServices
            | OrganizationKind::Prosecutor
            | OrganizationKind::Political
            | OrganizationKind::Labor
            | OrganizationKind::Civic
            | OrganizationKind::Commercial) => institutional_work(organization, kind),
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
                .world()
                .get_business(workplace)
                .and_then(|business| business_work_profit(state, business))
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
            // Six demanding detective or freelance shifts add up to 24 fatigue
            // per week. A full rest day must clear that ordinary weekly load;
            // otherwise every continuously assigned investigator eventually
            // becomes permanently exhausted despite taking scheduled days off.
            // Chronic losses still make proprietors progressively exhausted.
            life.fatigue = life
                .fatigue
                .saturating_sub(if work.detained { 2 } else { 24 });
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
pub(crate) fn apply_due_character_lives(state: &mut AppState) {
    if !is_day_boundary(state.now()) {
        return;
    }
    let day = state.now().as_minutes() / DAY_MINUTES;
    let changes: Vec<_> = state
        .world()
        .characters()
        .filter_map(|person| {
            let work = choose_work(state, person.id(), day);
            let life = finish_day(person.life(), state.now(), work);
            (person.life() != life).then_some((person.id(), life))
        })
        .collect();
    for (id, life) in changes {
        state.world.update_character_life(id, life);
    }
}

pub(crate) fn valid_character_lives(state: &AppState) -> bool {
    state.world().characters().all(|person| {
        let life = person.life();
        life.professional_standing() <= 100
            && life.fatigue() <= 100
            && life
                .last_activity()
                .map_or(life.career_days() == 0, |(at, action)| {
                    let day = at.as_minutes() / DAY_MINUTES;
                    at <= state.now()
                        && at.as_minutes() > 0
                        && at.as_minutes().is_multiple_of(DAY_MINUTES)
                        && !is_rest_day(person.id(), day)
                        && life.career_days() > 0
                        && u64::from(life.career_days()) <= maximum_workdays(person.id(), day)
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
                            | CivicActivity::CommercialAdvocacy(id) => state
                                .world()
                                .get_organization(id)
                                .is_some_and(|organization| {
                                    institutional_work(id, organization.kind()) == Some(action)
                                }),
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

#[cfg(test)]
mod business_work_tests;
#[cfg(test)]
mod validation_tests;
