//! Deterministic evidence-path inference over one investigation; it never reads hidden world relationships as legal knowledge.

use crate::core::entity::EntityRef;
use crate::core::id::{EvidenceId, InvestigationId};
use crate::core::state::AppState;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvidenceLink {
    pub from: EntityRef,
    pub to: EntityRef,
    pub evidence: EvidenceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidencePath {
    pub investigation: InvestigationId,
    pub from: EntityRef,
    pub to: EntityRef,
    pub links: Vec<EvidenceLink>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CaseGraphError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
}

pub fn resolve_evidence_path(
    state: &AppState,
    investigation_id: InvestigationId,
    from: EntityRef,
    to: EntityRef,
) -> Result<Option<EvidencePath>, CaseGraphError> {
    let investigation = state
        .legal()
        .get_investigation(investigation_id)
        .ok_or(CaseGraphError::MissingInvestigation(investigation_id))?;

    let mut adjacency: BTreeMap<EntityRef, Vec<(EntityRef, EvidenceId)>> = BTreeMap::new();
    for evidence_id in investigation.evidence() {
        let evidence = state
            .legal()
            .get_evidence(*evidence_id)
            .expect("investigation evidence index must reference existing evidence");
        let Some(origin) = evidence.origin() else {
            continue;
        };
        adjacency
            .entry(origin)
            .or_default()
            .push((evidence.subject(), evidence.id()));
        adjacency
            .entry(evidence.subject())
            .or_default()
            .push((origin, evidence.id()));
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
    }

    if from == to {
        return Ok(adjacency.contains_key(&from).then_some(EvidencePath {
            investigation: investigation_id,
            from,
            to,
            links: Vec::new(),
        }));
    }
    if !adjacency.contains_key(&from) || !adjacency.contains_key(&to) {
        return Ok(None);
    }

    let mut visited = BTreeSet::from([from]);
    let mut frontier = VecDeque::from([from]);
    let mut predecessor: BTreeMap<EntityRef, (EntityRef, EvidenceId)> = BTreeMap::new();
    while let Some(current) = frontier.pop_front() {
        let neighbors = adjacency
            .get(&current)
            .expect("queued evidence-graph entity must have adjacency");
        for (neighbor, evidence) in neighbors {
            if visited.insert(*neighbor) {
                predecessor.insert(*neighbor, (current, *evidence));
                if *neighbor == to {
                    return Ok(Some(build_path(investigation_id, from, to, &predecessor)));
                }
                frontier.push_back(*neighbor);
            }
        }
    }
    Ok(None)
}

fn build_path(
    investigation: InvestigationId,
    from: EntityRef,
    to: EntityRef,
    predecessor: &BTreeMap<EntityRef, (EntityRef, EvidenceId)>,
) -> EvidencePath {
    let mut reversed = Vec::new();
    let mut current = to;
    while current != from {
        let (previous, evidence) = predecessor
            .get(&current)
            .copied()
            .expect("reachable evidence-path node must have a predecessor");
        reversed.push(EvidenceLink {
            from: previous,
            to: current,
            evidence,
        });
        current = previous;
    }
    reversed.reverse();
    EvidencePath {
        investigation,
        from,
        to,
        links: reversed,
    }
}

#[cfg(test)]
mod tests;
