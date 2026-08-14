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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
        InvestigationDraft,
    };
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn evidence_path_uses_stable_shortest_case_owned_links() {
        let registry = build_registry();
        let mut state = AppState::new(0xCA5E_6A4F);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Graph Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Graph Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let first = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "First Associate".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("first associate should validate");
        let target = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Target Associate".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("target associate should validate");
        let alternate = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Alternate Associate".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("alternate associate should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Linked associates".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");

        let mut add_link = |subject: EntityRef, origin: EntityRef| {
            validate_add_evidence(
                &state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject,
                    origin: Some(origin),
                    kind: EvidenceKind::KnownAssociation,
                    strength: EvidenceStrength::Corroborating,
                    reliability: EvidenceReliability::Credible,
                    admissibility: Admissibility::Unknown,
                    discovered_at: state.now(),
                },
            )
            .expect("graph evidence should validate")
            .commit(&mut state)
            .expect("graph evidence should commit")
        };
        let first_edge = add_link(
            EntityRef::Character(first),
            EntityRef::Organization(criminal),
        );
        let first_target_edge = add_link(EntityRef::Character(target), EntityRef::Character(first));
        add_link(
            EntityRef::Character(alternate),
            EntityRef::Organization(criminal),
        );
        add_link(
            EntityRef::Character(target),
            EntityRef::Character(alternate),
        );

        let unrelated_investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Separate direct link".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(target)]),
            },
        )
        .expect("separate investigation should validate")
        .commit(&mut state)
        .expect("separate investigation should commit");
        let unrelated_direct_edge = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation: unrelated_investigation,
                custodian: police,
                subject: EntityRef::Character(target),
                origin: Some(EntityRef::Organization(criminal)),
                kind: EvidenceKind::KnownAssociation,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("separate case evidence should validate")
        .commit(&mut state)
        .expect("separate case evidence should commit");

        let path = resolve_evidence_path(
            &state,
            investigation,
            EntityRef::Organization(criminal),
            EntityRef::Character(target),
        )
        .expect("case graph should resolve")
        .expect("target should be connected by case evidence");
        assert_eq!(path.links.len(), 2);
        assert_eq!(path.links[0].evidence, first_edge);
        assert_eq!(path.links[1].evidence, first_target_edge);
        assert_eq!(path.links[0].from, EntityRef::Organization(criminal));
        assert_eq!(path.links[1].to, EntityRef::Character(target));

        let unrelated_path = resolve_evidence_path(
            &state,
            unrelated_investigation,
            EntityRef::Organization(criminal),
            EntityRef::Character(target),
        )
        .expect("separate case graph should resolve")
        .expect("separate case should contain its direct link");
        assert_eq!(unrelated_path.links.len(), 1);
        assert_eq!(unrelated_path.links[0].evidence, unrelated_direct_edge);
    }
}
