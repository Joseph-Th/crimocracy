//! Focused tests for case-evidence graph construction and queries.

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
