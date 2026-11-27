//! Focused tests for investigator-held case-activity knowledge: emission on staffing and
//! cold-case shelving, typed semantics, and canonical contact-channel disclosure.

use super::*;
use crate::build_registry;
use crate::contacts::contact_system::{
    InstitutionalContactDraft, find_pending_disclosure_sources, validate_contact_disclosure,
    validate_establish_contact,
};
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::time::SimDuration;
use crate::intelligence::{CaseActivitySignal, InformationSignal, InformationTopic};
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
    .commit(&mut state)
    .expect("relationship should commit");
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
    // Cold-case decay only shelves origin-linked cases, so this fixture deliberately opens one
    // through the operation-originated intake path production exposure uses.
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
            origin: Some(EntityRef::Operation(origin)),
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
    assert_eq!(
        fixture
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Character(fixture.detective),
                InformationTopic::LegalActivity,
            )
            .count(),
        0
    );

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
    assert_eq!(
        held[0].signal(),
        Some(&InformationSignal::CaseActivity(CaseActivitySignal::Active))
    );
    assert_eq!(
        held[0].subject(),
        fixture
            .state
            .legal()
            .get_investigation(investigation)
            .expect("staffed case must persist")
            .origin()
            .expect("fixture case must retain its operation origin"),
        "case-activity knowledge must identify the incident it describes rather than the authority's whole caseload"
    );
    assert!(summary.starts_with(case_activity_summary_prefix(CaseActivitySignal::Active)));
    assert!(summary.contains("Knowledge Precinct"));
    validate_invariants(&fixture.state);
}

#[test]
fn continued_staffed_incident_delivers_witness_knowledge_through_contact() {
    use crate::core::id::{IdExhaustionError, IdKind};
    use crate::legal::investigation_system::InvestigationError;
    use crate::legal::{IncidentWitnessDraft, WitnessCooperation};
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{
        OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
    };

    let registry = build_registry();
    let mut fixture = make_test_knowledge_fixture();
    let police = police_of(&fixture.state);
    let sponsor = fixture.criminal;
    let handler = fixture
        .state
        .world()
        .characters_in_organization(sponsor)
        .next()
        .unwrap()
        .id();
    let suspect = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Repeated Incident Participant".to_owned(),
            organization: Some(sponsor),
            supervisor: Some(handler),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(60))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let authorize = |state: &mut AppState, title: &str, delay: u32| {
        validate_authorize_operation(
            &registry,
            state,
            OperationDraft {
                title: title.to_owned(),
                kind: OperationKind::Surveillance,
                responsible_organization: sponsor,
                leader: suspect,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Organization(police),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, suspect)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: state.now() + SimDuration::from_minutes(delay),
            },
        )
        .unwrap()
        .commit(state)
        .unwrap()
    };
    let first = authorize(&mut fixture.state, "First incident", 1);
    let second = authorize(&mut fixture.state, "Second incident", 360);
    let draft = |state: &AppState, origin, witness| IncidentIntakeDraft {
        owner: police,
        title: "Related identified incident".to_owned(),
        subjects: BTreeSet::from([EntityRef::Operation(origin), EntityRef::Character(suspect)]),
        evidence: vec![IncidentEvidenceDraft {
            subject: EntityRef::Character(suspect),
            origin: Some(EntityRef::Operation(origin)),
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        }],
        origin: Some(EntityRef::Operation(origin)),
        witness,
    };
    let investigation =
        validate_incident_intake(&fixture.state, draft(&fixture.state, first, None))
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap()
            .investigation;
    assert_eq!(
        apply_autonomous_investigator_staffing(&mut fixture.state).unwrap(),
        vec![(investigation, fixture.detective)]
    );
    let contact = validate_establish_contact(
        &fixture.state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: fixture.detective,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let pending = find_pending_disclosure_sources(&fixture.state, contact);
    assert_eq!(pending.len(), 1);
    validate_contact_disclosure(&fixture.state, contact, pending[0])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let witness = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Second Incident Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let second_draft = |state: &AppState| {
        draft(
            state,
            second,
            Some(IncidentWitnessDraft {
                character: witness,
                subject: EntityRef::Character(suspect),
                cooperation: WitnessCooperation::Cooperative,
            }),
        )
    };
    // Exhaustion must reject the complete intake rather than register an undisclosable
    // witness, append evidence or consume any other artifact IDs.
    let mut exhausted = fixture.state.clone();
    let held = validate_incident_intake(&exhausted, second_draft(&exhausted)).unwrap();
    exhausted
        .ids
        .set_next_raw_for_test(IdKind::Information, u32::MAX);
    let before = bincode::serialize(&exhausted).unwrap();
    let result = held.commit(&mut exhausted);
    assert!(matches!(
        result,
        Err(InvestigationError::IdExhaustion(
            IdExhaustionError::Exhausted {
                kind: "information",
                ..
            }
        ))
    ));
    assert_eq!(bincode::serialize(&exhausted).unwrap(), before);

    let outcome = validate_incident_intake(&fixture.state, second_draft(&fixture.state))
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(outcome.investigation, investigation);
    assert!(outcome.case_witness.is_some());
    assert_eq!(fixture.state.legal().investigations().count(), 1);
    let pending = find_pending_disclosure_sources(&fixture.state, contact);
    assert_eq!(
        pending.len(),
        1,
        "the staffed case's newly registered witness must reach the lead's contact channel"
    );
    let signal = InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
        investigation,
    });
    let known = fixture
        .state
        .intelligence()
        .get_information(pending[0])
        .unwrap();
    assert_eq!(
        known.holder(),
        KnowledgeHolder::Character(fixture.detective)
    );
    assert_eq!(known.subject(), EntityRef::Character(witness));
    assert_eq!(known.signal(), Some(&signal));
    let disclosure = validate_contact_disclosure(&fixture.state, contact, pending[0])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let disclosed = fixture
        .state
        .contacts()
        .get_disclosure(disclosure)
        .unwrap()
        .disclosed_information();
    assert!(find_pending_disclosure_sources(&fixture.state, contact).is_empty());
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    let record = restored.intelligence().get_information(disclosed).unwrap();
    assert_eq!(record.holder(), KnowledgeHolder::Organization(sponsor));
    assert_eq!(record.signal(), Some(&signal));
    assert_eq!(record.subject(), EntityRef::Character(witness));
    assert!(find_pending_disclosure_sources(&restored, contact).is_empty());
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
    assert_eq!(decay, vec![investigation]);

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
            record.signal()
                == Some(&InformationSignal::CaseActivity(
                    CaseActivitySignal::Shelved,
                ))
        })
        .expect("shelving refreshes the lead's knowledge");
    assert_eq!(
        shelved.subject(),
        fixture
            .state
            .legal()
            .get_investigation(investigation)
            .expect("shelved case must persist")
            .origin()
            .expect("fixture case must retain its operation origin")
    );
    assert!(shelved.summary().contains("has already shelved the case"));
    validate_invariants(&fixture.state);
}

#[test]
fn contact_channel_discloses_each_new_development_exactly_once() {
    let registry = build_registry();
    let mut fixture = make_test_knowledge_fixture();
    crate::world::world_system::designate_player_organization(&mut fixture.state, fixture.criminal)
        .expect("criminal organization should be eligible as the player organization");
    let investigation = open_operation_case(&mut fixture.state);
    let case_subject = fixture
        .state
        .legal()
        .get_investigation(investigation)
        .expect("fixture case must persist")
        .origin()
        .expect("fixture case must retain its operation origin");
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
    assert_eq!(record.subject(), case_subject);
    assert_eq!(
        record.signal(),
        Some(&InformationSignal::CaseActivity(CaseActivitySignal::Active))
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
        record.signal(),
        Some(&InformationSignal::CaseActivity(
            CaseActivitySignal::Shelved
        ))
    );
    assert_eq!(record.subject(), case_subject);
    assert!(find_pending_disclosure_sources(&fixture.state, contact).is_empty());
    validate_invariants(&fixture.state);

    let envelope =
        build_save(&registry, &fixture.state).expect("typed case-activity disclosure should save");
    let bytes = bincode::serialize(&envelope).expect("typed disclosure save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("typed disclosure save should deserialize");
    let restored = restore_save(&registry, decoded).expect("typed disclosure save should restore");
    assert_eq!(
        restored
            .intelligence()
            .get_information(disclosed)
            .expect("restored disclosure information should persist")
            .signal(),
        Some(&InformationSignal::CaseActivity(
            CaseActivitySignal::Shelved
        ))
    );
    validate_invariants(&restored);
}
