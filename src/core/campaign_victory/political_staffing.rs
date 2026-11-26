//! Allocate distinct civic and financial directors together. Maximizing posts
//! separately can strand a district even when a complete coalition exists.

use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::politics::political_system::{
    campaign_office_available, eligible_manager, held_electoral_majority,
};
use crate::world::AutonomyLevel;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct DistrictCandidates {
    pub political: BTreeSet<CharacterId>,
    pub finance: BTreeSet<CharacterId>,
}

use super::district_matching::{self, Assignments, Choice, Priority};

pub(super) struct PoliticalStaffing {
    pub candidates: BTreeMap<NeighborhoodId, DistrictCandidates>,
    pub assigned: Assignments,
}

pub(super) fn allocate(
    state: &AppState,
    owner: OrganizationId,
    economically_ready: &BTreeSet<NeighborhoodId>,
    ballot_winners: &BTreeSet<NeighborhoodId>,
) -> PoliticalStaffing {
    let mut candidates = BTreeMap::new();
    // Personnel eligibility is independent of the age or profitability of
    // local properties. Show the real planning pool even in an unowned ward.
    for district in state.world().neighborhoods().map(|ward| ward.id()) {
        let mut local = DistrictCandidates::default();
        for mandate in state
            .delegation()
            .active_for_scope(ResponsibilityScope::Neighborhood(district))
        {
            if mandate.organization() != owner {
                continue;
            }
            let manager = mandate.manager();
            if !campaign_office_available(state, owner, district, manager) {
                continue;
            }
            if eligible_manager(state, owner, district, manager) {
                local.political.insert(manager);
            }
            if mandate.scopes().contains(&ResponsibilityScope::Function(
                ResponsibilityFunction::Finance,
            )) && state.world().get_character(manager).is_some_and(|person| {
                person.organization() == Some(owner)
                    && person.supervisor().is_some()
                    && matches!(
                        person.autonomy(),
                        AutonomyLevel::Delegated | AutonomyLevel::Broad
                    )
                    && state.legal().active_arrest_for_character(manager).is_none()
            }) {
                local.finance.insert(manager);
            }
        }
        candidates.insert(district, local);
    }
    let electoral = economically_ready
        .iter()
        .filter(|&&district| held_electoral_majority(state, owner, district))
        .copied()
        .collect();
    // Only economically qualified districts can consume a counted team.
    let assigned = solve(
        candidates
            .iter()
            .filter(|(district, _)| economically_ready.contains(district)),
        &electoral,
        ballot_winners,
    );
    PoliticalStaffing {
        candidates,
        assigned,
    }
}

fn solve<'a>(
    candidates: impl IntoIterator<Item = (&'a NeighborhoodId, &'a DistrictCandidates)>,
    electoral: &BTreeSet<NeighborhoodId>,
    ballot_winners: &BTreeSet<NeighborhoodId>,
) -> Assignments {
    district_matching::solve(candidates.into_iter().map(|(&district, local)| Choice {
        district,
        priority: if electoral.contains(&district) {
            if ballot_winners.contains(&district) {
                Priority::Primary
            } else {
                Priority::Secondary
            }
        } else {
            Priority::Ordinary
        },
        roles: [&local.political, &local.finance],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(political: &[u32], finance: &[u32]) -> DistrictCandidates {
        DistrictCandidates {
            political: political
                .iter()
                .map(|&id| CharacterId::from_raw(id))
                .collect(),
            finance: finance
                .iter()
                .map(|&id| CharacterId::from_raw(id))
                .collect(),
        }
    }

    #[test]
    fn joint_assignment_reserves_dual_qualified_people_for_the_role_that_needs_them() {
        let a = NeighborhoodId::from_raw(1);
        let b = NeighborhoodId::from_raw(2);
        let candidates = BTreeMap::from([(a, local(&[1, 2], &[1])), (b, local(&[3], &[4]))]);
        let assigned = solve(&candidates, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(assigned.len(), 2);
        assert_eq!(
            assigned[&a],
            (CharacterId::from_raw(2), CharacterId::from_raw(1))
        );
    }

    #[test]
    fn electoral_priority_is_not_sacrificed_to_incomplete_or_unready_wards() {
        let a = NeighborhoodId::from_raw(1);
        let b = NeighborhoodId::from_raw(2);
        let c = NeighborhoodId::from_raw(3);
        let candidates = BTreeMap::from([
            (a, local(&[1], &[2])),
            (b, local(&[1], &[3])),
            (c, local(&[2], &[4])),
        ]);
        let assigned = solve(&candidates, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(assigned.len(), 2);
        let assigned = solve(&candidates, &BTreeSet::from([a]), &BTreeSet::new());
        assert_eq!(assigned.len(), 1);
        assert!(assigned.contains_key(&a));
    }

    #[test]
    fn exact_match_agrees_with_exhaustive_small_role_graphs() {
        // Two districts, three people, all possible eligibility edges and
        // electoral priority combinations. Brute force every feasible team.
        let districts = [NeighborhoodId::from_raw(1), NeighborhoodId::from_raw(2)];
        let people = [
            CharacterId::from_raw(1),
            CharacterId::from_raw(2),
            CharacterId::from_raw(3),
        ];
        for graph in 0_u16..(1 << 12) {
            let candidates = districts
                .into_iter()
                .enumerate()
                .map(|(d, district)| {
                    let set = |role: usize| {
                        people
                            .into_iter()
                            .enumerate()
                            .filter(|(p, _)| graph & (1 << (6 * d + 3 * role + p)) != 0)
                            .map(|(_, person)| person)
                            .collect()
                    };
                    (
                        district,
                        DistrictCandidates {
                            political: set(0),
                            finance: set(1),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();
            for priority_mask in 0..4 {
                let electoral = districts
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| priority_mask & (1 << i) != 0)
                    .map(|(_, id)| id)
                    .collect::<BTreeSet<_>>();
                for winning_mask in 0..4 {
                    let winners = districts
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| winning_mask & (1 << i) != 0)
                        .map(|(_, id)| id)
                        .collect::<BTreeSet<_>>();
                    // Three people cannot fill two complete teams.
                    let expected = candidates
                        .iter()
                        .filter(|(_, local)| {
                            local
                                .political
                                .iter()
                                .any(|p| local.finance.iter().any(|f| p != f))
                        })
                        .map(|(district, _)| {
                            [
                                usize::from(
                                    electoral.contains(district) && winners.contains(district),
                                ),
                                usize::from(electoral.contains(district)),
                                1,
                            ]
                        })
                        .max()
                        .unwrap_or_default();
                    let result = solve(&candidates, &electoral, &winners);
                    assert_eq!(
                        [
                            result
                                .keys()
                                .filter(|district| electoral.contains(district)
                                    && winners.contains(district))
                                .count(),
                            result
                                .keys()
                                .filter(|district| electoral.contains(district))
                                .count(),
                            result.len(),
                        ],
                        expected
                    );
                    let mut used = BTreeSet::new();
                    for (district, (p, f)) in result {
                        assert!(used.insert(p) && used.insert(f));
                        assert!(candidates[&district].political.contains(&p));
                        assert!(candidates[&district].finance.contains(&f));
                    }
                }
            }
        }
    }
}
