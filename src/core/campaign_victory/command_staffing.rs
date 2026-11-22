//! Exact, read-only matching for the underboss ending's independent district captains.
//!
//! A maximum *individual post* matching is not enough: one captain in each of two
//! districts satisfies neither district's two-post command requirement. Search for
//! the maximum number of *complete districts* instead, with a cheap relaxed-post
//! matching bound that makes ordinary large, well-staffed cities finish immediately.

use super::UNDERBOSS_CAPTAINS_PER_DISTRICT;
use crate::core::id::{CharacterId, NeighborhoodId};
use std::collections::{BTreeMap, BTreeSet};

/// The greatest number of eligible districts that can each be assigned two distinct
/// proven managers, with no manager serving two districts simultaneously.
pub(super) fn staffed_racket_districts(
    districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
) -> usize {
    // Each post requires one independent captain, not an arbitrary number of fractional
    // posts. The two-seat rule is the authored underboss contract.
    debug_assert_eq!(UNDERBOSS_CAPTAINS_PER_DISTRICT, 2);
    let mut choices = districts
        .iter()
        .filter_map(|district| captains.get(district))
        .filter(|members| members.len() >= UNDERBOSS_CAPTAINS_PER_DISTRICT)
        .map(|members| members.iter().copied().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    // A constrained district is considered first, improving branch-and-bound pruning.
    // Ties keep the stable NeighborhoodId order from the input set.
    choices.sort_by_key(Vec::len);
    let mut best = 0;
    search_complete_districts(&choices, 0, &mut BTreeSet::new(), 0, &mut best);
    best
}

/// A relaxed bound: each remaining district can take zero, one, or two posts.
/// Every fully commanded district consumes two posts, so floor(maximum posts / 2)
/// is an upper bound on complete districts even when posts are scattered.
fn maximum_remaining_pairs(
    choices: &[Vec<CharacterId>],
    start: usize,
    committed: &BTreeSet<CharacterId>,
) -> usize {
    fn augment(
        district: usize,
        choices: &[Vec<CharacterId>],
        committed: &BTreeSet<CharacterId>,
        assigned: &mut BTreeMap<CharacterId, usize>,
        seen: &mut BTreeSet<CharacterId>,
    ) -> bool {
        for &captain in &choices[district] {
            if committed.contains(&captain) || !seen.insert(captain) {
                continue;
            }
            if assigned
                .get(&captain)
                .copied()
                .is_none_or(|previous| augment(previous, choices, committed, assigned, seen))
            {
                assigned.insert(captain, district);
                return true;
            }
        }
        false
    }

    let mut assigned = BTreeMap::new();
    let mut posts = 0;
    for district in start..choices.len() {
        for _ in 0..UNDERBOSS_CAPTAINS_PER_DISTRICT {
            posts += usize::from(augment(
                district,
                choices,
                committed,
                &mut assigned,
                &mut BTreeSet::new(),
            ));
        }
    }
    posts / UNDERBOSS_CAPTAINS_PER_DISTRICT
}

fn search_complete_districts(
    choices: &[Vec<CharacterId>],
    index: usize,
    committed: &mut BTreeSet<CharacterId>,
    complete: usize,
    best: &mut usize,
) {
    let remaining = choices.len() - index;
    let upper = maximum_remaining_pairs(choices, index, committed);
    if complete + upper <= *best {
        return;
    }
    if upper == remaining {
        // The relaxed match filled every post; consequently every district is complete.
        *best = complete + remaining;
        return;
    }
    let available: Vec<_> = choices[index]
        .iter()
        .copied()
        .filter(|captain| !committed.contains(captain))
        .collect();
    for (offset, &first) in available.iter().enumerate() {
        committed.insert(first);
        for &second in &available[offset + 1..] {
            committed.insert(second);
            search_complete_districts(choices, index + 1, committed, complete + 1, best);
            committed.remove(&second);
            if *best == complete + upper {
                committed.remove(&first);
                return;
            }
        }
        committed.remove(&first);
    }
    // Skipping an eligible district may release a manager needed to fully staff
    // two others; exploring only locally complete assignments would miss that outcome.
    search_complete_districts(choices, index + 1, committed, complete, best);
}
