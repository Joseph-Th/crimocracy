//! Focused tests for investigator-held case-activity knowledge: emission on staffing and
//! cold-case shelving, sightline parsing, and canonical contact-channel disclosure.

use super::*;
use crate::build_registry;
use crate::contacts::contact_system::{
    find_pending_disclosure_sources, validate_contact_disclosure, validate_establish_contact,
    InstitutionalContactDraft,
};
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::invariants::validate_invariants;
use crate::core::time::SimDuration;
use crate::intelligence::InformationTopic;
use crate::legal::investigation_system::{
    apply_autonomous_investigator_staffing, apply_cold_case_decay, validate_incident_intake,
};
use crate::legal::{
    Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft,
};
use crate::social::relationship_system::validate_set_relationship;
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{
    AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

struct KnowledgeFixture {
    state: AppState,
    criminal: OrganizationId,
    detective: CharacterId,
}

fn make_test_knowledge_fixture() -> KnowledgeFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xCADE_5EED);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Knowledge Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Knowledge Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Knowledge Boss".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("boss fixture should validate");
    let detective = insert_character(
        &mut state,
        CharacterDraft {
            name: "Knowledge Detective".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective fixture should validate");
    let level = |value: u8| {
        crate::social::RelationshipLevel::try_new(value).expect("test level must be valid")
    };
    validate_set_relationship(
        &state,
        boss,
        detective,
        crate::social::RelationshipDimensions {
            trust: level(45),
            respect: level(0),
            fear: level(0),
            affection: level(0),
            dependence: level(0),
            resentment: level(0),
            debt: level(0),
        },
    )
    .expect("handler-contact relationship should validate")
    .commit(&mut state);
    KnowledgeFixture {
        state,
        criminal,
        detective,
    }
}

fn open_operation_case(state: &mut AppState) -> InvestigationId {
    let registry = build_registry();
    let police = police_of(state);
    let criminal = criminal_of(state);
    // Cold-case decay only shelves operation-originated cases, so the fixture opens the case
    // through the same operation-originated intake path production exposure uses.
    let leader = crate::world::world_system::insert_character(
        state,
        CharacterDraft {
            name: "Origin Crew Leader".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(60))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let origin = crate::operations::operation_system::validate_authorize_operation(
        &registry,
        state,
        crate::operations::OperationDraft {
            title: "Origin surveillance".to_owned(),
            kind: crate::operations::OperationKind::Surveillance,
            responsible_organization: criminal,
            leader,
            objective: crate::operations::OperationObjective::GatherInformation {
                target: EntityRef::Organization(criminal),
            },
            approach: crate::operations::OperationApproach::Covert,
            roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("origin operation should validate")
    .commit(state)
    .expect("origin operation should commit");
    let outcome = validate_incident_intake(
        state,
        IncidentIntakeDraft {
            owner: police,
            title: "Incident linked to a test burglary".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(origin)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Operation(origin),
                origin: Some(EntityRef::Operation(origin)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            }],
            origin_operation: Some(origin),
            notified_organizations: BTreeSet::from([criminal]),
            witness: None,
        },
    )
    .expect("incident intake should validate")
    .commit(state)
    .expect("incident intake should commit");
    outcome.investigation
}

// Small accessor shims so the draft builders above stay readable.
fn police_of(state: &AppState) -> OrganizationId {
    state
        .world
        .organizations()
        .find(|org| org.kind() == OrganizationKind::LawEnforcement)
        .map(|org| org.id())
        .expect("police fixture must exist")
}

fn criminal_of(state: &AppState) -> OrganizationId {
    state
        .world
        .organizations()
        .find(|org| org.kind() == OrganizationKind::Criminal)
        .map(|org| org.id())
        .expect("criminal fixture must exist")
}

#[test]
fn staffing_records_lead_held_active_case_knowledge() {
    let _registry = build_registry();
    let mut fixture = make_test_knowledge_fixture();
    let investigation = open_operation_case(&mut fixture.state);

    // Before staffing there is no institutional knower and no knowledge record.
    assert!(record_lead_case_activity_knowledge(&mut fixture.state, investigation).is_none());

    let staffed = apply_autonomous_investigator_staffing(&mut fixture.state)
        .expect("staffing pass must succeed");
    assert_eq!(staffed, vec![(investigation, fixture.detective)]);

    let held: Vec<_> = fixture
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Character(fixture.detective),
            InformationTopic::LegalActivity,
        )
        .collect();
    assert_eq!(
        held.len(),
        1,
        "staffing records exactly one activity knowledge record"
    );
    let summary = held[0].summary();
    let status = CaseActivityStatus::parse_summary_marker(summary)
        .expect("knowledge summary carries a parseable marker");
    assert_eq!(status, CaseActivityStatus::Active);
    assert_eq!(status.is_hot(), Some(true));
    assert!(summary.contains("Knowledge Precinct"));
    validate_invariants(&fixture.state);
}

#[test]
fn cold_shelving_refreshes_the_leads_knowledge_to_shelved() {
    let registry = build_registry();
    let mut fixture = make_test_knowledge_fixture();
    let investigation = open_operation_case(&mut fixture.state);
    apply_autonomous_investigator_staffing(&mut fixture.state).expect("staffing pass must succeed");

    // Force the authored inactivity window to elapse with no further case work, then decay.
    let window = registry.legal().cold_case_window();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(window.as_minutes() + 60));
    let decay = apply_cold_case_decay(&mut fixture.state, window).expect("decay must succeed");
    assert_eq!(decay.suspended, vec![investigation]);

    let held: Vec<_> = fixture
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Character(fixture.detective),
            InformationTopic::LegalActivity,
        )
        .collect();
    assert_eq!(held.len(), 2, "active plus refreshed shelved knowledge");
    let shelved = held
        .iter()
        .find(|record| {
            CaseActivityStatus::parse_summary_marker(record.summary())
                == Some(CaseActivityStatus::Shelved)
        })
        .expect("shelving refreshes the lead's knowledge");
    assert_eq!(
        CaseActivityStatus::parse_summary_marker(shelved.summary())
            .expect("parsed shelved marker")
            .is_hot(),
        Some(false)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn contact_channel_discloses_each_new_development_exactly_once() {
    let registry = build_registry();
    let mut fixture = make_test_knowledge_fixture();
    crate::world::world_system::designate_player_organization(&mut fixture.state, fixture.criminal)
        .expect("criminal organization should be eligible as the player organization");
    let _investigation = open_operation_case(&mut fixture.state);
    apply_autonomous_investigator_staffing(&mut fixture.state).expect("staffing pass must succeed");

    let sponsor = fixture.criminal;
    let handler = fixture
        .state
        .world
        .characters_in_organization(sponsor)
        .next()
        .map(|character| character.id())
        .expect("criminal fixture holds a handler");
    let contact = validate_establish_contact(
        &fixture.state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: fixture.detective,
        },
    )
    .expect("the standing relationship supports a police channel")
    .commit(&mut fixture.state)
    .expect("validated contact establishment commits");

    // First ask: the channel offers the active-development read.
    let first = find_pending_disclosure_sources(&fixture.state, contact);
    assert_eq!(first.len(), 1);
    let disclosure = validate_contact_disclosure(&fixture.state, contact, first[0])
        .expect("pending source must disclose")
        .commit(&mut fixture.state)
        .expect("disclosure must commit");
    let disclosed = fixture
        .state
        .contacts()
        .get_disclosure(disclosure)
        .expect("disclosure persists")
        .disclosed_information();
    let record = fixture
        .state
        .intelligence()
        .get_information(disclosed)
        .expect("disclosed information persists");
    assert_eq!(record.holder(), KnowledgeHolder::Organization(sponsor));
    assert_eq!(
        CaseActivityStatus::parse_summary_marker(record.summary()),
        Some(CaseActivityStatus::Active)
    );

    // The same development cannot be sold twice; nothing new is pending yet.
    assert!(find_pending_disclosure_sources(&fixture.state, contact).is_empty());

    // Shelving the case produces a fresh disclosable development.
    let window = registry.legal().cold_case_window();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(window.as_minutes() + 120));
    apply_cold_case_decay(&mut fixture.state, window).expect("decay must succeed");
    let second = find_pending_disclosure_sources(&fixture.state, contact);
    assert_eq!(second.len(), 1, "the shelf is a fresh, disclosable fact");
    let disclosure = validate_contact_disclosure(&fixture.state, contact, second[0])
        .expect("refreshed knowledge must disclose")
        .commit(&mut fixture.state)
        .expect("second disclosure must commit");
    let disclosed = fixture
        .state
        .contacts()
        .get_disclosure(disclosure)
        .expect("disclosure persists")
        .disclosed_information();
    let record = fixture
        .state
        .intelligence()
        .get_information(disclosed)
        .expect("disclosed information persists");
    assert_eq!(
        CaseActivityStatus::parse_summary_marker(record.summary()),
        Some(CaseActivityStatus::Shelved)
    );
    assert!(find_pending_disclosure_sources(&fixture.state, contact).is_empty());
    validate_invariants(&fixture.state);
}
