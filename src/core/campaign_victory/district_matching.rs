//! Exact read-only allocation of two distinct people per complete district team.
//! Shared by the political and underboss qualification owners. A relaxed
//! matching of individual posts bounds each priority prefix of the exact search.

use crate::core::id::{CharacterId, NeighborhoodId};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Assignments = BTreeMap<NeighborhoodId, (CharacterId, CharacterId)>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    Primary,
    Secondary,
    Ordinary,
}

pub(super) struct Choice<'a> {
    pub(super) district: NeighborhoodId,
    pub(super) priority: Priority,
    pub(super) roles: [&'a BTreeSet<CharacterId>; 2],
}

// Lexicographic cumulative counts: primary teams, primary plus secondary
// teams, then all complete teams.
type Score = [usize; 3];

fn add_match(score: &mut Score, priority: Priority) {
    for count in &mut score[priority as usize..] {
        *count += 1;
    }
}

pub(super) fn solve<'a>(choices: impl IntoIterator<Item = Choice<'a>>) -> Assignments {
    let mut choices = choices
        .into_iter()
        .filter(|choice| {
            !choice.roles[0].is_empty()
                && !choice.roles[1].is_empty()
                && choice.roles[0].union(choice.roles[1]).take(2).count() == 2
        })
        .collect::<Vec<_>>();
    // Within each priority tier consider scarce teams first, then stable IDs.
    choices.sort_by_key(|choice| {
        (
            choice.priority,
            choice.roles[0].len().saturating_mul(choice.roles[1].len()),
            choice.district,
        )
    });
    let mut best = Best::default();
    search(
        &choices,
        0,
        &mut BTreeSet::new(),
        &mut Assignments::new(),
        Score::default(),
        &mut best,
    );
    best.assigned
}

#[derive(Default)]
struct Best {
    score: Score,
    assigned: Assignments,
}

impl Best {
    fn consider(&mut self, score: Score, assigned: &Assignments) {
        if score > self.score {
            self.score = score;
            self.assigned = assigned.clone();
        }
    }
}

/// Relax complete-district constraints to ordinary bipartite post matching.
/// Processing posts in priority order preserves the maximum matching size of
/// every priority prefix as later augmenting paths rematch individuals. Half
/// of each prefix's matched post count bounds its complete-district count.
fn relaxed_matching(
    choices: &[Choice<'_>],
    start: usize,
    used: &BTreeSet<CharacterId>,
) -> (Score, Assignments) {
    fn augment(
        choices: &[Choice<'_>],
        slot: (usize, usize),
        used: &BTreeSet<CharacterId>,
        assigned: &mut BTreeMap<CharacterId, (usize, usize)>,
        seen: &mut BTreeSet<CharacterId>,
    ) -> bool {
        for &person in choices[slot.0].roles[slot.1] {
            if used.contains(&person) || !seen.insert(person) {
                continue;
            }
            if assigned
                .get(&person)
                .copied()
                .is_none_or(|previous| augment(choices, previous, used, assigned, seen))
            {
                assigned.insert(person, slot);
                return true;
            }
        }
        false
    }
    let mut people = BTreeMap::new();
    let mut posts = Score::default();
    for (index, choice) in choices.iter().enumerate().skip(start) {
        for role in 0..2 {
            if augment(
                choices,
                (index, role),
                used,
                &mut people,
                &mut BTreeSet::new(),
            ) {
                add_match(&mut posts, choice.priority);
            }
        }
    }
    let mut by_slot = BTreeMap::new();
    for (person, slot) in people {
        by_slot.insert(slot, person);
    }
    let pairs = choices
        .iter()
        .enumerate()
        .skip(start)
        .filter_map(|(index, choice)| {
            Some((
                choice.district,
                (*by_slot.get(&(index, 0))?, *by_slot.get(&(index, 1))?),
            ))
        })
        .collect();
    (posts.map(|count| count / 2), pairs)
}

fn search(
    choices: &[Choice<'_>],
    index: usize,
    used: &mut BTreeSet<CharacterId>,
    current: &mut Assignments,
    score: Score,
    best: &mut Best,
) {
    let (bound, pairs) = relaxed_matching(choices, index, used);
    let upper: Score = std::array::from_fn(|tier| score[tier] + bound[tier]);
    if upper <= best.score {
        return;
    }
    // Every complete pair in the relaxed match is already a valid assignment.
    // This often reaches the upper bound without enumerating any combinations.
    let mut feasible = current.clone();
    let mut lower = score;
    for choice in &choices[index..] {
        if pairs.contains_key(&choice.district) {
            add_match(&mut lower, choice.priority);
        }
    }
    feasible.extend(pairs);
    best.consider(lower, &feasible);
    if upper <= best.score || index == choices.len() {
        return;
    }
    let choice = &choices[index];
    for &first in choice.roles[0] {
        if !used.insert(first) {
            continue;
        }
        for &second in choice.roles[1] {
            if !used.insert(second) {
                continue;
            }
            current.insert(choice.district, (first, second));
            let mut next_score = score;
            add_match(&mut next_score, choice.priority);
            search(choices, index + 1, used, current, next_score, best);
            current.remove(&choice.district);
            used.remove(&second);
            if upper <= best.score {
                used.remove(&first);
                return;
            }
        }
        used.remove(&first);
    }
    search(choices, index + 1, used, current, score, best);
}
