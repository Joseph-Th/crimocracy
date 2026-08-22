//! Focused tests for institutional contact establishment, termination, and disclosure channels.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character, WorldError,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct ContactFixture {
    registry: crate::registry::Registry,
    state: AppState,
    sponsor: OrganizationId,
    handler: CharacterId,
    institution: OrganizationId,
    source: CharacterId,
}

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("fixture relationship level should validate")
}

fn relationship(trust: u8, debt: u8) -> RelationshipDimensions {
    RelationshipDimensions {
        trust: level(trust),
        respect: level(35),
        fear: level(0),
        affection: level(15),
        dependence: level(20),
        resentment: level(0),
        debt: level(debt),
    }
}

fn make_fixture(institution_kind: OrganizationKind) -> ContactFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x0C01_7AC7);
    let sponsor = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Contact Test Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("sponsor should validate");
    let institution = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Contact Test Institution".to_owned(),
            kind: institution_kind,
        },
    )
    .expect("institution should validate");
    let handler = insert_character(
        &mut state,
        CharacterDraft {
            name: "Contact Handler".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("handler should validate");
    let source = insert_character(
        &mut state,
        CharacterDraft {
            name: "Institutional Source".to_owned(),
            organization: Some(institution),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("source should validate");
    validate_set_relationship(&state, handler, source, relationship(70, 45))
        .expect("contact relationship should validate")
        .commit(&mut state);
    ContactFixture {
        registry,
        state,
        sponsor,
        handler,
        institution,
        source,
    }
}

fn establish(fixture: &mut ContactFixture) -> ContactId {
    validate_establish_contact(
        &fixture.state,
        InstitutionalContactDraft {
            sponsor: fixture.sponsor,
            handler: fixture.handler,
            contact: fixture.source,
        },
    )
    .expect("institutional contact should validate")
    .commit(&mut fixture.state)
    .expect("institutional contact should commit")
}

fn record_source_information(
    fixture: &mut ContactFixture,
    holder: KnowledgeHolder,
) -> InformationId {
    record_source_information_with_topic(fixture, holder, InformationTopic::PoliceActivity)
}

fn record_source_information_with_topic(
    fixture: &mut ContactFixture,
    holder: KnowledgeHolder,
    topic: InformationTopic,
) -> InformationId {
    validate_record_information(
        &fixture.state,
        InformationDraft {
            holder,
            source_kind: InformationSourceKind::DirectObservation,
            topic,
            source_entity: None,
            subject: EntityRef::Character(fixture.handler),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Detectives have been asking questions about the contact handler.".to_owned(),
        },
    )
    .expect("source information should validate")
    .commit(&mut fixture.state)
    .expect("source information should commit")
}

#[test]
fn police_contact_disclosure_preserves_personal_source_provenance_and_save_round_trip() {
    let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
    let contact = establish(&mut fixture);
    assert_eq!(
        fixture
            .state
            .contacts()
            .get_contact(contact)
            .expect("contact should persist")
            .kind(),
        ContactKind::Police
    );
    let source_character = fixture.source;
    let source =
        record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
    let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
        .expect("personally held police information should be disclosable")
        .commit(&mut fixture.state)
        .expect("contact disclosure should commit");
    let disclosure_record = fixture
        .state
        .contacts()
        .get_disclosure(disclosure)
        .expect("disclosure should persist");
    let disclosed = fixture
        .state
        .intelligence()
        .get_information(disclosure_record.disclosed_information())
        .expect("disclosed information should persist");
    assert_eq!(
        disclosed.holder(),
        KnowledgeHolder::Organization(fixture.sponsor)
    );
    assert_eq!(
        disclosed.source_kind(),
        InformationSourceKind::PoliceContact
    );
    assert_eq!(
        disclosed.source_entity(),
        Some(EntityRef::Character(fixture.source))
    );
    assert_eq!(disclosed.derived_from(), &BTreeSet::from([source]));
    assert_eq!(disclosed.reliability(), Reliability::GenerallyReliable);
    assert_eq!(disclosed.specificity(), Specificity::Specific);
    assert_eq!(
        fixture
            .state
            .contacts()
            .disclosure_for_information(disclosed.id())
            .map(ContactDisclosureRecord::id),
        Some(disclosure)
    );
    validate_state(&fixture.state).expect("contact disclosure state should validate");
    validate_invariants(&fixture.state);

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("contact disclosure state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored =
        restore_save(&fixture.registry, decoded).expect("contact disclosure state should restore");
    assert_eq!(
        restored
            .contacts()
            .get_disclosure(disclosure)
            .map(ContactDisclosureRecord::disclosed_information),
        Some(disclosed.id())
    );
    validate_invariants(&restored);
}

#[test]
fn contact_disclosure_cannot_read_institution_owned_hidden_information() {
    let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
    let contact = establish(&mut fixture);
    let institution = fixture.institution;
    let hidden =
        record_source_information(&mut fixture, KnowledgeHolder::Organization(institution));
    let error = validate_contact_disclosure(&fixture.state, contact, hidden)
        .err()
        .expect("institution-owned truth must not pass through a personal contact implicitly");
    assert_eq!(
        error,
        ContactError::InformationUnavailable {
            information: hidden,
            contact: fixture.source,
        }
    );
    assert_eq!(fixture.state.contacts().disclosures().count(), 0);
    validate_invariants(&fixture.state);
}

#[test]
fn active_contact_locks_memberships_until_termination_then_history_survives_moves() {
    let mut fixture = make_fixture(OrganizationKind::LawEnforcement);
    let contact = establish(&mut fixture);
    let second_sponsor = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Second Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second sponsor should validate");
    let second_institution = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Ward Office".to_owned(),
            kind: OrganizationKind::Political,
        },
    )
    .expect("second institution should validate");

    let handler_error =
        validate_reassign_character(&fixture.state, fixture.handler, Some(second_sponsor), None)
            .expect_err("active contact handler must not leave sponsor");
    assert_eq!(
        handler_error,
        WorldError::ActiveInstitutionalContactHandler {
            character: fixture.handler,
            contact,
        }
    );
    let source_error = validate_reassign_character(
        &fixture.state,
        fixture.source,
        Some(second_institution),
        None,
    )
    .expect_err("active external contact must not leave institution");
    assert_eq!(
        source_error,
        WorldError::ActiveInstitutionalContactAssignment {
            character: fixture.source,
            contact,
        }
    );

    let source_character = fixture.source;
    let source =
        record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
    let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
        .expect("active contact disclosure should validate")
        .commit(&mut fixture.state)
        .expect("active contact disclosure should commit");
    validate_terminate_contact(&fixture.state, contact)
        .expect("active contact should terminate")
        .commit(&mut fixture.state)
        .expect("contact termination should commit");
    validate_reassign_character(&fixture.state, fixture.handler, Some(second_sponsor), None)
        .expect("terminated contact should release handler membership dependency")
        .commit(&mut fixture.state)
        .expect("handler move should commit");
    validate_reassign_character(
        &fixture.state,
        fixture.source,
        Some(second_institution),
        None,
    )
    .expect("terminated contact should release external membership dependency")
    .commit(&mut fixture.state)
    .expect("external contact move should commit");

    let historical = fixture
        .state
        .contacts()
        .get_contact(contact)
        .expect("terminated contact should remain historical");
    assert_eq!(historical.status(), ContactStatus::Terminated);
    assert_eq!(historical.institution(), fixture.institution);
    assert!(fixture
        .state
        .contacts()
        .get_disclosure(disclosure)
        .is_some());
    validate_state(&fixture.state)
        .expect("terminated contact history should survive personnel moves");
    validate_invariants(&fixture.state);
}

#[test]
fn establishment_token_rejects_relationship_change_without_partial_contact() {
    let mut fixture = make_fixture(OrganizationKind::Political);
    let stale = validate_establish_contact(
        &fixture.state,
        InstitutionalContactDraft {
            sponsor: fixture.sponsor,
            handler: fixture.handler,
            contact: fixture.source,
        },
    )
    .expect("contact establishment should initially validate");
    validate_set_relationship(
        &fixture.state,
        fixture.handler,
        fixture.source,
        relationship(40, 80),
    )
    .expect("relationship revision should validate")
    .commit(&mut fixture.state);
    let error = stale
        .commit(&mut fixture.state)
        .expect_err("relationship revision must stale establishment token");
    assert_eq!(
        error,
        ContactError::StaleRelationship {
            from: fixture.handler,
            to: fixture.source,
        }
    );
    assert_eq!(
        fixture
            .state
            .contacts()
            .contacts_for_sponsor(fixture.sponsor)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn disclosure_token_rejects_contact_termination_and_duplicate_source() {
    let mut fixture = make_fixture(OrganizationKind::Press);
    let contact = establish(&mut fixture);
    let source_character = fixture.source;
    let source = record_source_information_with_topic(
        &mut fixture,
        KnowledgeHolder::Character(source_character),
        InformationTopic::General,
    );
    let stale = validate_contact_disclosure(&fixture.state, contact, source)
        .expect("contact disclosure should initially validate");
    validate_terminate_contact(&fixture.state, contact)
        .expect("contact termination should validate")
        .commit(&mut fixture.state)
        .expect("contact termination should commit");
    let error = stale
        .commit(&mut fixture.state)
        .expect_err("terminated contact must stale pending disclosure");
    assert_eq!(
        error,
        ContactError::StaleContact {
            contact,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(fixture.state.contacts().disclosures().count(), 0);

    let mut duplicate_fixture = make_fixture(OrganizationKind::Press);
    let duplicate_contact = establish(&mut duplicate_fixture);
    let duplicate_source_character = duplicate_fixture.source;
    let duplicate_source = record_source_information_with_topic(
        &mut duplicate_fixture,
        KnowledgeHolder::Character(duplicate_source_character),
        InformationTopic::General,
    );
    validate_contact_disclosure(
        &duplicate_fixture.state,
        duplicate_contact,
        duplicate_source,
    )
    .expect("first disclosure should validate")
    .commit(&mut duplicate_fixture.state)
    .expect("first disclosure should commit");
    let duplicate_error = validate_contact_disclosure(
        &duplicate_fixture.state,
        duplicate_contact,
        duplicate_source,
    )
    .err()
    .expect("same source information must not be disclosed twice through one contact");
    assert_eq!(
        duplicate_error,
        ContactError::DuplicateDisclosure {
            contact: duplicate_contact,
            information: duplicate_source,
        }
    );
    validate_invariants(&fixture.state);
    validate_invariants(&duplicate_fixture.state);
}

#[test]
fn press_contact_cannot_launder_operational_knowledge_outside_its_domain() {
    let mut fixture = make_fixture(OrganizationKind::Press);
    let contact = establish(&mut fixture);
    let source_character = fixture.source;
    let source =
        record_source_information(&mut fixture, KnowledgeHolder::Character(source_character));
    let error = validate_contact_disclosure(&fixture.state, contact, source)
        .err()
        .expect("press contacts must not disclose police-activity knowledge");
    assert_eq!(
        error,
        ContactError::InformationOutsideContactDomain {
            information: source,
            topic: InformationTopic::PoliceActivity,
            kind: ContactKind::Press,
        }
    );
    assert_eq!(fixture.state.contacts().disclosures().count(), 0);
    validate_invariants(&fixture.state);
}

#[test]
fn institution_kind_controls_disclosure_channel_without_generic_influence_score() {
    for (organization_kind, contact_kind, source_kind) in [
        (
            OrganizationKind::LegalAuthority,
            ContactKind::Legal,
            InformationSourceKind::Lawyer,
        ),
        (
            OrganizationKind::Political,
            ContactKind::Political,
            InformationSourceKind::PoliticalContact,
        ),
        (
            OrganizationKind::Labor,
            ContactKind::Labor,
            InformationSourceKind::ProfessionalContact,
        ),
        (
            OrganizationKind::Commercial,
            ContactKind::Professional,
            InformationSourceKind::ProfessionalContact,
        ),
    ] {
        let mut fixture = make_fixture(organization_kind);
        let contact = establish(&mut fixture);
        assert_eq!(
            fixture
                .state
                .contacts()
                .get_contact(contact)
                .expect("contact should persist")
                .kind(),
            contact_kind
        );
        let source_character = fixture.source;
        let topic = match contact_kind {
            ContactKind::Legal => InformationTopic::LegalActivity,
            ContactKind::Political => InformationTopic::MarketAccess,
            ContactKind::Labor | ContactKind::Professional => {
                InformationTopic::FinancialPerformance
            }
            ContactKind::Police => InformationTopic::PoliceActivity,
            ContactKind::Press => InformationTopic::General,
        };
        let source = record_source_information_with_topic(
            &mut fixture,
            KnowledgeHolder::Character(source_character),
            topic,
        );
        let disclosure = validate_contact_disclosure(&fixture.state, contact, source)
            .expect("institutional disclosure should validate")
            .commit(&mut fixture.state)
            .expect("institutional disclosure should commit");
        let information = fixture
            .state
            .contacts()
            .get_disclosure(disclosure)
            .and_then(|record| {
                fixture
                    .state
                    .intelligence()
                    .get_information(record.disclosed_information())
            })
            .expect("disclosed information should persist");
        assert_eq!(information.source_kind(), source_kind);
        validate_state(&fixture.state).expect("typed institutional contact state should validate");
        validate_invariants(&fixture.state);
    }
}
