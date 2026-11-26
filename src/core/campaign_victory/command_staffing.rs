//! Underboss district captains and the joint senior-command witness.
//! Each complete district consumes two different proven captains, and one
//! person cannot serve twice. A senior commander must supervise selected
//! captains in two distinct districts of the same feasible coalition.

use super::UNDERBOSS_CAPTAINS_PER_DISTRICT;
use super::district_matching::{self, Assignments, Choice, Priority};
use crate::core::id::{CharacterId, NeighborhoodId};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn racket_district_assignment(
    districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
) -> Assignments {
    debug_assert_eq!(UNDERBOSS_CAPTAINS_PER_DISTRICT, 2);
    district_matching::solve(districts.iter().filter_map(|district| {
        let local = captains.get(district)?;
        Some(Choice {
            district: *district,
            roles: [local, local],
            priority: Priority::Ordinary,
        })
    }))
}

pub(super) fn staffed_racket_districts(
    districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
) -> usize {
    racket_district_assignment(districts, captains).len()
}

/// The unconstrained maximum team assignment may not contain the needed two
/// direct reports, even when an alternative coalition does. Try the ordinary
/// assignment first; otherwise require a report in each of two distinct wards
/// using the same exact two-role solver, with those wards as primary teams.
/// The solver must complete both and still reach the citywide district floor.
pub(super) fn supports_senior_command(
    districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
    direct_reports: &BTreeSet<CharacterId>,
    ordinary: &Assignments,
    required: usize,
) -> bool {
    let required = required.max(2);
    if ordinary.len() < required {
        return false;
    }
    if ordinary
        .values()
        .filter(|(first, second)| direct_reports.contains(first) || direct_reports.contains(second))
        .take(2)
        .count()
        == 2
    {
        return true;
    }
    let local_reports = districts
        .iter()
        .filter_map(|district| {
            let local = captains.get(district)?;
            if local.len() < UNDERBOSS_CAPTAINS_PER_DISTRICT {
                return None;
            }
            let reports = local
                .intersection(direct_reports)
                .copied()
                .collect::<BTreeSet<_>>();
            (!reports.is_empty()).then_some((*district, reports))
        })
        .collect::<Vec<_>>();
    for (index, (first, first_reports)) in local_reports.iter().enumerate() {
        for (second, second_reports) in &local_reports[index + 1..] {
            if first_reports.union(second_reports).take(2).count() < 2 {
                continue;
            }
            let assigned = district_matching::solve(districts.iter().filter_map(|district| {
                let local = captains.get(district)?;
                let reports = if district == first {
                    Some(first_reports)
                } else if district == second {
                    Some(second_reports)
                } else {
                    None
                };
                Some(Choice {
                    district: *district,
                    roles: [reports.unwrap_or(local), local],
                    priority: if reports.is_some() {
                        Priority::Primary
                    } else {
                        Priority::Ordinary
                    },
                })
            }));
            if assigned.len() >= required
                && assigned.contains_key(first)
                && assigned.contains_key(second)
            {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(rows: &[(u32, &[u32])]) -> BTreeMap<NeighborhoodId, BTreeSet<CharacterId>> {
        rows.iter()
            .map(|(district, people)| {
                (
                    NeighborhoodId::from_raw(*district),
                    people.iter().copied().map(CharacterId::from_raw).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn senior_reports_cannot_use_incompatible_complete_teams() {
        let captains = graph(&[(1, &[1, 3]), (2, &[2, 3]), (3, &[4, 5]), (4, &[6, 7])]);
        let districts = captains.keys().copied().collect();
        let ordinary = racket_district_assignment(&districts, &captains);
        let reports = BTreeSet::from([CharacterId::from_raw(1), CharacterId::from_raw(2)]);
        assert_eq!(ordinary.len(), 3);
        assert!(!supports_senior_command(
            &districts, &captains, &reports, &ordinary, 3
        ));
    }

    #[test]
    fn joint_command_agrees_with_exhaustive_small_captain_graphs() {
        // Three districts and four captains: every eligibility graph and every
        // subset of potential direct reports. Independently enumerate the
        // feasible pairs of complete districts using disjoint two-person masks.
        let districts = (1..=3)
            .map(NeighborhoodId::from_raw)
            .collect::<BTreeSet<_>>();
        let people = |mask: u16| {
            (0..4)
                .filter(|bit| mask & (1 << bit) != 0)
                .map(|bit| CharacterId::from_raw(bit + 1))
                .collect::<BTreeSet<_>>()
        };
        for graph in 0_u16..(1 << 12) {
            let masks: [u16; 3] = std::array::from_fn(|district| (graph >> (4 * district)) & 15);
            let captains = districts
                .iter()
                .copied()
                .zip(masks.map(people))
                .collect::<BTreeMap<_, _>>();
            let ordinary = racket_district_assignment(&districts, &captains);
            let teams = masks.map(|local| {
                (1_u16..16)
                    .filter(|team| team.count_ones() == 2 && team & local == *team)
                    .collect::<Vec<_>>()
            });
            let mut feasible = Vec::new();
            for (index, first) in teams.iter().enumerate() {
                for second in &teams[index + 1..] {
                    for &a in first {
                        for &b in second {
                            if a & b == 0 {
                                feasible.push((a, b));
                            }
                        }
                    }
                }
            }
            let expected = if !feasible.is_empty() {
                2
            } else if teams.iter().any(|team| !team.is_empty()) {
                1
            } else {
                0
            };
            assert_eq!(ordinary.len(), expected, "eligibility graph {graph:012b}");
            for reports in 0_u16..16 {
                let expected = feasible
                    .iter()
                    .any(|(a, b)| a & reports != 0 && b & reports != 0);
                assert_eq!(
                    supports_senior_command(&districts, &captains, &people(reports), &ordinary, 2),
                    expected,
                    "eligibility graph {graph:012b}, report mask {reports:04b}",
                );
            }
        }
    }

    #[test]
    fn senior_reports_can_select_an_alternative_feasible_coalition() {
        let captains = graph(&[(1, &[1, 3, 4]), (2, &[2, 5, 6]), (3, &[7, 8])]);
        let districts = captains.keys().copied().collect();
        let ordinary = BTreeMap::from([
            (
                NeighborhoodId::from_raw(1),
                (CharacterId::from_raw(3), CharacterId::from_raw(4)),
            ),
            (
                NeighborhoodId::from_raw(2),
                (CharacterId::from_raw(5), CharacterId::from_raw(6)),
            ),
            (
                NeighborhoodId::from_raw(3),
                (CharacterId::from_raw(7), CharacterId::from_raw(8)),
            ),
        ]);
        let reports = BTreeSet::from([CharacterId::from_raw(1), CharacterId::from_raw(2)]);
        assert!(supports_senior_command(
            &districts, &captains, &reports, &ordinary, 3
        ));
    }
}
