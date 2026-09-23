//! Focused tests for information recording, transfer, lineage, and holder indexes.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::id::{BusinessId, EnterpriseId, InvestigationId, NeighborhoodId};
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::intelligence::{
    CaseActivitySignal, EnterpriseLocationSignal, LegalPersonStatusSignal, PatrolIntervalSignal,
    Reliability, Specificity,
};
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::witness_system::validate_register_case_witness;
use crate::legal::{
    Admissibility, ArrestDraft, CaseWitnessDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, InvestigationDraft, WitnessCooperation,
};
use crate::reports::report_system::{ReportError, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

fn make_transfer_fixture() -> (
    crate::registry::Registry,
    AppState,
    OrganizationId,
    CharacterId,
) {
    let registry = build_registry();
    let mut state = AppState::new(0x1F0A_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Information Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let character = insert_character(
        &mut state,
        CharacterDraft {
            name: "Information Courier".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("character fixture should validate");
    (registry, state, organization, character)
}

#[test]
fn typed_signal_compatibility_requires_every_semantic_axis() {
    let organization = OrganizationId::from_raw(1);
    let character = CharacterId::from_raw(2);
    let enterprise = EnterpriseId::from_raw(3);
    let business = BusinessId::from_raw(4);
    let neighborhood = NeighborhoodId::from_raw(5);

    let case_activity = InformationSignal::CaseActivity(CaseActivitySignal::Active);
    assert!(case_activity.is_compatible(
        InformationTopic::LegalActivity,
        EntityRef::Organization(organization)
    ));
    assert!(!case_activity.is_compatible(
        InformationTopic::Personnel,
        EntityRef::Organization(organization)
    ));
    assert!(!case_activity.is_compatible(
        InformationTopic::LegalActivity,
        EntityRef::Character(character)
    ));

    let legal_person = InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
        arrest: ArrestId::from_raw(6),
    });
    assert!(legal_person.is_compatible(
        InformationTopic::LegalActivity,
        EntityRef::Character(character)
    ));
    assert!(
        !legal_person.is_compatible(InformationTopic::Personnel, EntityRef::Character(character))
    );
    assert!(!legal_person.is_compatible(
        InformationTopic::LegalActivity,
        EntityRef::Organization(organization)
    ));

    let enterprise_location =
        InformationSignal::EnterpriseLocation(EnterpriseLocationSignal::Business(business));
    assert!(enterprise_location.is_compatible(
        InformationTopic::EnterpriseActivity,
        EntityRef::Enterprise(enterprise)
    ));
    assert!(!enterprise_location.is_compatible(
        InformationTopic::Personnel,
        EntityRef::Enterprise(enterprise)
    ));
    assert!(!enterprise_location.is_compatible(
        InformationTopic::TargetSecurity,
        EntityRef::Enterprise(enterprise)
    ));
    assert!(!enterprise_location.is_compatible(
        InformationTopic::EnterpriseActivity,
        EntityRef::Business(business)
    ));

    let personnel = InformationSignal::PersonnelPresence {
        characters: BTreeSet::from([character]),
    };
    assert!(personnel.is_compatible(
        InformationTopic::Personnel,
        EntityRef::Organization(organization)
    ));
    assert!(
        !InformationSignal::PersonnelPresence {
            characters: BTreeSet::new(),
        }
        .is_compatible(
            InformationTopic::Personnel,
            EntityRef::Organization(organization)
        )
    );
    assert!(!personnel.is_compatible(
        InformationTopic::PoliceActivity,
        EntityRef::Organization(organization)
    ));
    assert!(!personnel.is_compatible(
        InformationTopic::Personnel,
        EntityRef::Enterprise(enterprise)
    ));

    let patrol = InformationSignal::PatrolPattern {
        intervals: BTreeSet::from([
            PatrolIntervalSignal::try_new(120, 180).expect("test patrol interval must be valid")
        ]),
    };
    assert!(patrol.is_compatible(
        InformationTopic::PoliceActivity,
        EntityRef::Neighborhood(neighborhood)
    ));
    assert!(
        !InformationSignal::PatrolPattern {
            intervals: BTreeSet::new(),
        }
        .is_compatible(
            InformationTopic::PoliceActivity,
            EntityRef::Neighborhood(neighborhood)
        )
    );
    assert!(!patrol.is_compatible(
        InformationTopic::Schedule,
        EntityRef::Neighborhood(neighborhood)
    ));
    assert!(!patrol.is_compatible(
        InformationTopic::PoliceActivity,
        EntityRef::Organization(organization)
    ));
}

#[test]
fn typed_signal_referenced_entities_are_complete_and_exact() {
    let character_a = CharacterId::from_raw(11);
    let character_b = CharacterId::from_raw(12);
    let investigation = InvestigationId::from_raw(13);
    let business = BusinessId::from_raw(14);
    let neighborhood = NeighborhoodId::from_raw(15);

    assert_eq!(
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
            investigation,
        })
        .referenced_entities(),
        vec![EntityRef::Investigation(investigation)]
    );
    assert_eq!(
        InformationSignal::EnterpriseLocation(EnterpriseLocationSignal::Business(business))
            .referenced_entities(),
        vec![EntityRef::Business(business)]
    );
    assert_eq!(
        InformationSignal::EnterpriseLocation(EnterpriseLocationSignal::Neighborhood(neighborhood))
            .referenced_entities(),
        vec![EntityRef::Neighborhood(neighborhood)]
    );
    assert_eq!(
        InformationSignal::PersonnelPresence {
            characters: BTreeSet::from([character_b, character_a]),
        }
        .referenced_entities(),
        vec![
            EntityRef::Character(character_a),
            EntityRef::Character(character_b)
        ]
    );
    assert!(
        InformationSignal::CaseActivity(CaseActivitySignal::Active)
            .referenced_entities()
            .is_empty()
    );
    assert!(
        InformationSignal::PatrolPattern {
            intervals: BTreeSet::from([
                PatrolIntervalSignal::try_new(60, 90).expect("test patrol interval must be valid")
            ]),
        }
        .referenced_entities()
        .is_empty()
    );
}

#[test]
fn typed_signal_rejects_incompatible_topic_without_mutation() {
    let (_registry, state, organization, character) = make_transfer_fixture();
    let error = match validate_record_information_with_signal(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(character),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "This personnel observation must not masquerade as case activity.".to_owned(),
        },
        InformationSignal::CaseActivity(CaseActivitySignal::Active),
    ) {
        Ok(_) => panic!("case-activity semantics require legal-activity information"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        IntelligenceError::InvalidSignal {
            signal: InformationSignal::CaseActivity(CaseActivitySignal::Active),
            topic: InformationTopic::Personnel,
            subject: EntityRef::Organization(organization),
        }
    );
    assert_eq!(state.intelligence().information().count(), 0);
    validate_invariants(&state);
}

#[test]
fn legal_person_signals_must_match_their_subject_and_episode() {
    let (registry, mut state, organization, suspect) = make_transfer_fixture();
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Typed Signal Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Typed Signal Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Typed signal case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(suspect)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation,
            witness,
            subject: EntityRef::Character(suspect),
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("witness registration should validate")
    .commit(&mut state)
    .expect("witness registration should commit");
    let strong = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("strong evidence should validate")
    .commit(&mut state)
    .expect("strong evidence should commit");
    let corroborating = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Fingerprint,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating evidence should validate")
    .commit(&mut state)
    .expect("corroborating evidence should commit");
    let arrest = validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: suspect,
            investigation,
            evidence: BTreeSet::from([strong, corroborating]),
        },
    )
    .expect("arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");
    let information_count = state.intelligence().information().count();

    let wrong_witness_subject = validate_record_information_with_signal(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: None,
            subject: EntityRef::Character(suspect),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "This subject was not the registered witness.".to_owned(),
        },
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
            investigation,
        }),
    )
    .err()
    .expect("witness signal must identify the actual registered witness");
    assert!(matches!(
        wrong_witness_subject,
        IntelligenceError::InvalidSignal { .. }
    ));

    let wrong_detention_subject = validate_record_information_with_signal(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: None,
            subject: EntityRef::Character(witness),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "This subject was not the arrested person.".to_owned(),
        },
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained { arrest }),
    )
    .err()
    .expect("detention signal must identify the arrest's actual character");
    assert!(matches!(
        wrong_detention_subject,
        IntelligenceError::InvalidSignal { .. }
    ));
    assert_eq!(
        state.intelligence().information().count(),
        information_count,
        "rejected semantic mismatches must not create knowledge"
    );
    validate_state(&state).expect("semantic-signal rejection should preserve valid state");
    validate_invariants(&state);
}

#[test]
fn detention_signal_rejects_missing_arrest_without_mutation() {
    let (_registry, state, organization, character) = make_transfer_fixture();
    let missing_arrest = ArrestId::from_raw(999);
    let error = match validate_record_information_with_signal(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: None,
            subject: EntityRef::Character(character),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "A detention claim must identify a persisted arrest episode.".to_owned(),
        },
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
            arrest: missing_arrest,
        }),
    ) {
        Ok(_) => panic!("typed detention knowledge cannot reference a missing arrest"),
        Err(error) => error,
    };
    assert_eq!(error, IntelligenceError::MissingArrest(missing_arrest));
    assert_eq!(state.intelligence().information().count(), 0);
    validate_invariants(&state);
}

#[test]
fn concurrent_duplicate_internal_transfer_is_rejected_without_mutation() {
    let (registry, mut state, organization, character) = make_transfer_fixture();
    let source = record_character_information(&mut state, character, organization);
    let first = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("first internal transfer should validate");
    let concurrent = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("concurrent token may validate against the same pre-transfer snapshot");

    let transferred = first
        .commit(&mut state)
        .expect("first internal transfer should commit");
    let before = bincode::serialize(&state).expect("post-transfer state should serialize");
    assert_eq!(
        concurrent
            .commit(&mut state)
            .expect_err("the same source must not be transferred twice to one recipient"),
        IntelligenceError::DuplicateTransfer {
            source_information: source,
            recipient: KnowledgeHolder::Organization(organization),
            existing: transferred,
        }
    );
    assert_eq!(
        bincode::serialize(&state).expect("rejected duplicate state should serialize"),
        before,
        "duplicate transfer rejection must not allocate an ID or mutate knowledge"
    );
    let duplicate = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .err()
    .expect("later duplicate validation should fail immediately");
    assert_eq!(
        duplicate,
        IntelligenceError::DuplicateTransfer {
            source_information: source,
            recipient: KnowledgeHolder::Organization(organization),
            existing: transferred,
        }
    );
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .count(),
        1
    );
    validate_state(&state).expect("one-shot transfer state should remain valid");
    validate_invariants(&state);

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("one-shot transfer state should save"),
    )
    .expect("one-shot transfer state should restore with derived indexes rebuilt");
    assert_eq!(
        validate_information_transfer(
            &restored,
            InformationTransferDraft {
                source,
                recipient: KnowledgeHolder::Organization(organization),
            },
        )
        .err()
        .expect("duplicate transfer must remain blocked after restore"),
        IntelligenceError::DuplicateTransfer {
            source_information: source,
            recipient: KnowledgeHolder::Organization(organization),
            existing: transferred,
        }
    );
    validate_state(&restored).expect("restored transfer index should validate");
    validate_invariants(&restored);
}

#[test]
fn typed_signal_survives_internal_transfer_with_lineage() {
    let (_registry, mut state, organization, character) = make_transfer_fixture();
    let signal = InformationSignal::CaseActivity(CaseActivitySignal::Active);
    let source = validate_record_information_with_signal(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(character),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Organization(organization)),
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "The member directly observed that the known case remains active.".to_owned(),
        },
        signal.clone(),
    )
    .expect("compatible typed information should validate")
    .commit(&mut state)
    .expect("typed source information should commit");

    let transferred = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("typed internal transfer should validate")
    .commit(&mut state)
    .expect("typed internal transfer should commit");
    let record = state
        .intelligence()
        .get_information(transferred)
        .expect("transferred typed information should persist");
    assert_eq!(record.signal(), Some(&signal));
    assert_eq!(record.derived_from(), &BTreeSet::from([source]));
    validate_state(&state).expect("typed transfer state should validate");
    validate_invariants(&state);
}

fn record_character_information(
    state: &mut AppState,
    character: CharacterId,
    organization: OrganizationId,
) -> InformationId {
    validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Character(character),
            source_kind: InformationSourceKind::DirectObservation,
            topic: crate::intelligence::InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: crate::intelligence::Reliability::DirectAccess,
            specificity: crate::intelligence::Specificity::Precise,
            summary: "A member directly observed information relevant to leadership.".to_owned(),
        },
    )
    .expect("character information fixture should validate")
    .commit(state)
    .expect("character information fixture should commit")
}

#[test]
fn explicit_transfer_creates_stable_organization_knowledge_and_provenance() {
    let (registry, mut state, organization, character) = make_transfer_fixture();
    let source = record_character_information(&mut state, character, organization);

    let direct_report_error = match validate_record_report(
        &state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Legal,
            title: "Unreported member knowledge".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: "Leadership cannot cite knowledge that has not been reported upward."
                    .to_owned(),
                sources: vec![source],
                entities: BTreeSet::from([EntityRef::Character(character)]),
                decision: None,
            }],
        },
    ) {
        Ok(_) => panic!("organization report must reject character-only knowledge"),
        Err(error) => error,
    };
    assert_eq!(
        direct_report_error,
        ReportError::InformationUnavailable {
            information: source,
            recipient: organization,
        }
    );

    let transferred = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("member-to-organization transfer should validate")
    .commit(&mut state)
    .expect("validated information transfer should commit");
    let transferred_record = state
        .intelligence()
        .get_information(transferred)
        .expect("transferred information should persist");
    assert_eq!(
        transferred_record.holder(),
        KnowledgeHolder::Organization(organization)
    );
    assert_eq!(
        transferred_record.source_kind(),
        InformationSourceKind::InternalReport
    );
    assert_eq!(
        transferred_record.topic(),
        crate::intelligence::InformationTopic::TargetSecurity
    );
    assert_eq!(
        transferred_record.source_entity(),
        Some(EntityRef::Character(character))
    );
    assert_eq!(transferred_record.derived_from(), &BTreeSet::from([source]));
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );
    assert_eq!(
        state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(organization),
                crate::intelligence::InformationTopic::TargetSecurity,
            )
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );

    let report = validate_record_report(
        &state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Legal,
            title: "Reported member knowledge".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: "Leadership now possesses a provenance-bearing internal report."
                    .to_owned(),
                sources: vec![transferred],
                entities: BTreeSet::from([EntityRef::Character(character)]),
                decision: None,
            }],
        },
    )
    .expect("organization-held transfer should be reportable")
    .commit(&mut state)
    .expect("organization-held transfer report should commit");

    validate_reassign_character(&state, character, None, None)
        .expect("character should be able to leave after reporting information")
        .commit(&mut state)
        .expect("character reassignment should commit");
    validate_state(&state).expect("historical organization report must survive membership change");
    validate_invariants(&state);

    let envelope =
        build_save(&registry, &state).expect("provenance-bearing organization report should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("provenance save should restore");
    assert!(restored.reports().get_report(report).is_some());
    assert_eq!(
        restored
            .intelligence()
            .information_derived_from(source)
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );
    validate_invariants(&restored);
}

#[test]
fn transfer_token_becomes_stale_after_character_membership_change() {
    let (_registry, mut state, organization, character) = make_transfer_fixture();
    let source = record_character_information(&mut state, character, organization);
    let transfer = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("transfer should validate against current membership");

    validate_reassign_character(&state, character, None, None)
        .expect("membership change should validate")
        .commit(&mut state)
        .expect("membership change should commit");
    let error = transfer
        .commit(&mut state)
        .expect_err("transfer must reject a stale character membership snapshot");
    assert_eq!(
        error,
        IntelligenceError::StaleTransferCharacter {
            character,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn internal_transfer_rejects_unrelated_organization() {
    let (registry, mut state, organization, character) = make_transfer_fixture();
    let other = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Unrelated Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second organization fixture should validate");
    let source = record_character_information(&mut state, character, organization);

    let error = match validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(other),
        },
    ) {
        Ok(_) => panic!("internal transfer must not cross unrelated organizations"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        IntelligenceError::TransferNotPermitted {
            source_holder: KnowledgeHolder::Character(character),
            recipient: KnowledgeHolder::Organization(other),
        }
    );
    validate_invariants(&state);
}

#[test]
fn peer_transfer_allows_members_of_the_same_organization() {
    let (_registry, mut state, organization, source_character) = make_transfer_fixture();
    let recipient = insert_character(
        &mut state,
        CharacterDraft {
            name: "Information Peer".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("same-organization peer should validate");
    let source = record_character_information(&mut state, source_character, organization);

    let transferred = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Character(recipient),
        },
    )
    .expect("same-organization peers should be allowed to transfer information")
    .commit(&mut state)
    .expect("validated peer transfer should commit");

    let record = state
        .intelligence()
        .get_information(transferred)
        .expect("peer transfer should persist");
    assert_eq!(record.holder(), KnowledgeHolder::Character(recipient));
    assert_eq!(record.derived_from(), &BTreeSet::from([source]));
    validate_state(&state).expect("same-organization peer transfer should remain valid");
    validate_invariants(&state);
}

#[test]
fn peer_transfer_rejects_characters_in_different_organizations() {
    let (registry, mut state, organization, source_character) = make_transfer_fixture();
    let other = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Outside Information Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("outside organization should validate");
    let outsider = insert_character(
        &mut state,
        CharacterDraft {
            name: "Outside Information Recipient".to_owned(),
            organization: Some(other),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("outside character should validate");
    let source = record_character_information(&mut state, source_character, organization);

    let error = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Character(outsider),
        },
    )
    .err()
    .expect("cross-organization peer transfer must be rejected");
    assert_eq!(
        error,
        IntelligenceError::TransferNotPermitted {
            source_holder: KnowledgeHolder::Character(source_character),
            recipient: KnowledgeHolder::Character(outsider),
        }
    );
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .count(),
        0,
        "rejected peer transfer must not create derived information"
    );
    validate_state(&state).expect("rejected cross-organization transfer must preserve state");
    validate_invariants(&state);
}

#[test]
fn generic_information_recording_cannot_forge_derived_provenance() {
    let (_registry, mut state, organization, character) = make_transfer_fixture();
    record_character_information(&mut state, character, organization);

    for source_kind in [
        InformationSourceKind::PoliceContact,
        InformationSourceKind::LegalContact,
        InformationSourceKind::PoliticalContact,
        InformationSourceKind::ProfessionalContact,
    ] {
        let error = match validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind,
                topic: crate::intelligence::InformationTopic::Personnel,
                source_entity: Some(EntityRef::Character(character)),
                subject: EntityRef::Organization(organization),
                observed_at: state.now(),
                reliability: crate::intelligence::Reliability::GenerallyReliable,
                specificity: crate::intelligence::Specificity::Specific,
                summary: "Contact-derived provenance must come from a disclosure.".to_owned(),
            },
        ) {
            Ok(_) => panic!("generic recording must not create contact-derived information"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            IntelligenceError::ContactSourceRequiresDisclosure(source_kind)
        );
    }
    for source_kind in [
        InformationSourceKind::Accounting,
        InformationSourceKind::Surveillance,
        InformationSourceKind::AfterAction,
    ] {
        let error = match validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind,
                topic: crate::intelligence::InformationTopic::Personnel,
                source_entity: Some(EntityRef::Character(character)),
                subject: EntityRef::Organization(organization),
                observed_at: state.now(),
                reliability: crate::intelligence::Reliability::GenerallyReliable,
                specificity: crate::intelligence::Specificity::Specific,
                summary: "System-authored provenance must come from its owning system.".to_owned(),
            },
        ) {
            Ok(_) => panic!("generic recording must not create system-authored information"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            IntelligenceError::SystemSourceRequiresOwner(source_kind)
        );
    }

    let internal_report_error = match validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::InternalReport,
            topic: crate::intelligence::InformationTopic::Personnel,
            source_entity: Some(EntityRef::Character(character)),
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: crate::intelligence::Reliability::DirectAccess,
            specificity: crate::intelligence::Specificity::Precise,
            summary: "This must use the canonical transfer path.".to_owned(),
        },
    ) {
        Ok(_) => panic!("generic recording must not create internal reports"),
        Err(error) => error,
    };
    assert_eq!(
        internal_report_error,
        IntelligenceError::InternalReportRequiresTransfer
    );
    validate_invariants(&state);
}
