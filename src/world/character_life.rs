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
    let mut properties = state
        .world()
        .businesses_owned_by_character(character)
        .peekable();
    let proprietor = properties.peek().is_some();
    let owned = properties
        .filter_map(|business| {
            let economy = state.economy().get_business_economy(business.id())?;
            if economy.status() != BusinessOperatingStatus::Active {
                return None;
            }
            // Only settlements in the current ownership and operating period
            // can affect the proprietor. Reopening starts a new period even
            // when the title has not changed.
            let cycle = state
                .economy()
                .cycles_for(business.id())
                .next_back()
                .filter(|cycle| {
                    cycle.business_version() == business.version()
                        && economy
                            .loss_streak_anchor()
                            .is_none_or(|anchor| cycle.occurred_at() > anchor)
                });
            let profit = cycle.map_or(0, |cycle| cycle.net_cash().cents());
            Some((business.id(), profit))
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
pub fn apply_due_character_lives(state: &mut AppState) -> usize {
    if !is_day_boundary(state.now()) {
        return 0;
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
    let changed = changes.len();
    for (id, life) in changes {
        state.world.update_character_life(id, life);
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

#[cfg(test)]
mod business_work_tests {
    use super::*;
    use crate::build_registry;
    use crate::city_generation::{CityScale, generate_city};
    use crate::core::invariants::validate_invariants;
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::SimDuration;
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan, validate_suspend_business_economy,
    };
    use crate::finance::Money;
    use crate::world::BusinessOwner;
    use crate::world::world_system::validate_transfer_business_ownership;

    #[test]
    fn suspended_proprietor_does_not_resume_an_old_vocation() {
        let registry = build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
        let business = city
            .state
            .world()
            .businesses()
            .find(|shop| {
                matches!(shop.owner(), BusinessOwner::Character(_))
                    && city
                        .state
                        .economy()
                        .get_business_economy(shop.id())
                        .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
            })
            .unwrap()
            .id();
        let worker = city
            .state
            .world()
            .characters()
            .find(|person| {
                let IndependentVocation::BusinessEmployee(workplace) = person.life().vocation()
                else {
                    return false;
                };
                workplace != business
                    && city.state.world().civic_profession(person.id())
                        == Some(CivicProfession::BusinessEmployee)
                    && city
                        .state
                        .economy()
                        .get_business_economy(workplace)
                        .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
            })
            .unwrap()
            .id();
        validate_transfer_business_ownership(
            &city.state,
            business,
            BusinessOwner::Character(worker),
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        validate_suspend_business_economy(&city.state, business)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        assert_eq!(
            city.state.world().civic_profession(worker),
            Some(CivicProfession::BusinessOwner)
        );
        let day = (1..=7)
            .find(|day| !(*day + u64::from(worker.raw())).is_multiple_of(7))
            .unwrap();
        let life = city.state.world().get_character(worker).unwrap().life();
        city.state
            .advance_clock(SimDuration::from_minutes((day * DAY_MINUTES) as u32));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        apply_due_character_lives(&mut city.state);
        let after = city.state.world().get_character(worker).unwrap().life();
        assert_eq!(after.career_days(), life.career_days());
        assert_eq!(after.professional_standing(), life.professional_standing());
        assert_eq!(
            after.last_activity(),
            life.last_activity(),
            "a closed proprietor must not work at a previous employer"
        );
        validate_invariants(&city.state);
        let restored =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        assert_eq!(
            restored.world().get_character(worker).unwrap().life(),
            after
        );
    }

    #[test]
    fn reopened_proprietor_does_not_reuse_pre_suspension_earnings() {
        let registry = build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
        let (owner, business) = city
            .state
            .world()
            .businesses()
            .find_map(|shop| {
                let BusinessOwner::Character(owner) = shop.owner() else {
                    return None;
                };
                if city
                    .state
                    .world()
                    .businesses_owned_by_character(owner)
                    .count()
                    != 1
                {
                    return None;
                }
                city.state
                    .economy()
                    .get_business_economy(shop.id())
                    .filter(|economy| economy.status() == BusinessOperatingStatus::Active)
                    .map(|_| (owner, shop.id()))
            })
            .unwrap();
        let due = city
            .state
            .economy()
            .get_business_economy(business)
            .unwrap()
            .next_cycle_at()
            .unwrap();
        city.state.advance_clock(SimDuration::from_minutes(
            u32::try_from(due.as_minutes()).unwrap(),
        ));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        let plan = decide_business_cycle(&registry, &city.state, business, 0).unwrap();
        let cycle = validate_business_cycle_plan(&city.state, plan)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        assert_ne!(
            city.state.economy().get_cycle(cycle).unwrap().net_cash(),
            Money::ZERO
        );
        validate_suspend_business_economy(&city.state, business)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        crate::economy::business_economy_system::validate_resume_business_economy(
            &registry,
            &city.state,
            business,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();

        let day = city.state.now().as_minutes() / DAY_MINUTES;
        let working_day = (day..day + 7)
            .find(|&day| !(day + u64::from(owner.raw())).is_multiple_of(7))
            .unwrap();
        let work = choose_work(&city.state, owner, working_day);
        assert_eq!(
            work.action,
            Some(CivicActivity::BusinessManagement(business))
        );
        assert_eq!(work.business_profit, 0);
        let mut restored =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        assert_eq!(
            choose_work(&restored, owner, working_day).business_profit,
            0
        );

        for state in [&mut city.state, &mut restored] {
            let due = state
                .economy()
                .get_business_economy(business)
                .unwrap()
                .next_cycle_at()
                .unwrap();
            state.advance_clock(SimDuration::from_minutes(
                u32::try_from(due.as_minutes() - state.now().as_minutes()).unwrap(),
            ));
            crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, state)
                .unwrap();
            let plan = decide_business_cycle(&registry, state, business, 0).unwrap();
            let cycle = validate_business_cycle_plan(state, plan)
                .unwrap()
                .commit(state)
                .unwrap();
            let net = state.economy().get_cycle(cycle).unwrap().net_cash();
            assert_ne!(net, Money::ZERO);
            let work = choose_work(state, owner, working_day);
            assert_eq!(work.business_profit, if net > Money::ZERO { 1 } else { -1 });
            validate_invariants(state);
        }
    }

    #[test]
    fn new_proprietor_does_not_inherit_previous_owners_earnings_record() {
        let registry = build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
        let (previous_owner, business) = city
            .state
            .world()
            .businesses()
            .find_map(|shop| {
                let BusinessOwner::Character(owner) = shop.owner() else {
                    return None;
                };
                if city
                    .state
                    .world()
                    .businesses_owned_by_character(owner)
                    .count()
                    != 1
                {
                    return None;
                }
                city.state
                    .economy()
                    .get_business_economy(shop.id())
                    .filter(|economy| economy.status() == BusinessOperatingStatus::Active)
                    .map(|_| (owner, shop.id()))
            })
            .expect("generated city has an active private proprietor");
        let successor = city
            .state
            .world()
            .characters()
            .find(|person| {
                person.id() != previous_owner
                    && person.organization().is_none()
                    && city
                        .state
                        .world()
                        .businesses_owned_by_character(person.id())
                        .next()
                        .is_none()
            })
            .expect("generated city has an independent non-owner")
            .id();

        // Settle a real prior owner's business cycle; its signed net is the
        // historical signal that would otherwise be incorrectly inherited.
        for _ in 0..45 {
            city.state
                .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
            crate::opportunities::opportunity_system::apply_opportunity_expiry(
                &registry,
                &mut city.state,
            )
            .unwrap();
            if let Ok(plan) = decide_business_cycle(&registry, &city.state, business, 0) {
                validate_business_cycle_plan(&city.state, plan)
                    .unwrap()
                    .commit(&mut city.state)
                    .unwrap();
            }
            if city
                .state
                .economy()
                .cycles_for(business)
                .next_back()
                .is_some_and(|cycle| cycle.net_cash() != Money::ZERO)
            {
                break;
            }
        }
        let historical_cycle = city
            .state
            .economy()
            .cycles_for(business)
            .next_back()
            .expect("private business should settle during the review horizon");
        assert_ne!(historical_cycle.net_cash(), Money::ZERO);

        validate_transfer_business_ownership(
            &city.state,
            business,
            BusinessOwner::Character(successor),
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();

        let day = city.state.now().as_minutes() / DAY_MINUTES;
        let working_day = (day..day + 7)
            .find(|&day| !(day + u64::from(successor.raw())).is_multiple_of(7))
            .unwrap();
        let work = choose_work(&city.state, successor, working_day);
        assert_eq!(
            work.action,
            Some(CivicActivity::BusinessManagement(business))
        );
        assert_eq!(work.business_profit, 0);

        validate_invariants(&city.state);
        let restored =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        let restored_work = choose_work(&restored, successor, working_day);
        assert_eq!(restored_work.action, work.action);
        assert_eq!(restored_work.business_profit, work.business_profit);
    }
}
