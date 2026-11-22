//! Named civilian and institutional people make their own deterministic work
//! decisions. An occupation is derived from current membership and actual
//! property titles, not assigned in a parallel roster that can drift from world
//! ownership. This subsystem owns only the personal work/condition snapshot.

use super::{CharacterLife, CivicActivity, CivicProfession, OrganizationKind, WorldState};
use crate::core::id::CharacterId;
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, is_day_boundary};
use crate::economy::BusinessOperatingStatus;
use crate::legal::{InvestigationStatus, PatrolDeploymentStatus};

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
        Some(profession.unwrap_or(CivicProfession::Resident))
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
    if detained || day.is_multiple_of(7) {
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
                    let position =
                        (day + u64::from(character.raw())) % districts.len().max(1) as u64;
                    districts
                        .get(position as usize)
                        .copied()
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
                    | CivicActivity::Community => 2,
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
                    }
            })
    })
}
