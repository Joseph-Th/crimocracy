//! Focused tests for witness registration, interviews, testimony, and pressure effects.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::time::{SimDuration, SimTime};
use crate::legal::investigation_system::{
    InvestigationTransition, validate_add_evidence, validate_open_investigation,
    validate_transition_investigation,
};
use crate::legal::{CaseWitnessRecord, EvidenceDraft, InvestigationDraft, WitnessStatementDraft};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct WitnessFixture {
    state: AppState,
    police: crate::core::id::OrganizationId,
    criminal: crate::core::id::OrganizationId,
    investigation: InvestigationId,
    witness: CharacterId,
    subject: CharacterId,
}

#[derive(Clone, Serialize)]
struct CaseWitnessRecordWire {
    id: CaseWitnessId,
    investigation: InvestigationId,
    witness: CharacterId,
    cooperation: WitnessCooperation,
    registered_at: SimTime,
    statements: BTreeSet<crate::core::id::WitnessStatementId>,
    interview_attempts: u8,
    version: u32,
}

fn case_witness_wire(record: &CaseWitnessRecord) -> CaseWitnessRecordWire {
    CaseWitnessRecordWire {
        id: record.id(),
        investigation: record.investigation(),
        witness: record.witness(),
        cooperation: record.cooperation(),
        registered_at: record.registered_at(),
        statements: record.statements().clone(),
        interview_attempts: record.interview_attempts(),
        version: record.version(),
    }
}

fn replace_serialized_case_witness(
    envelope: SaveEnvelope,
    original: &CaseWitnessRecord,
    replacement: &CaseWitnessRecordWire,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("case witness should serialize");
    let mirror = case_witness_wire(original);
    assert_eq!(
        bincode::serialize(&mirror).expect("case witness mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement case witness should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized case witness must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout case witness corruption must remain decodable")
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn make_fixture() -> WitnessFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x7117_E551);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Daniel Mercer".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness fixture should validate");
    let subject = insert_character(
        &mut state,
        CharacterDraft {
            name: "Frank Dello".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("subject fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Witness identification inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(subject)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("investigation fixture should commit");
    WitnessFixture {
        state,
        police,
        criminal,
        investigation,
        witness,
        subject,
    }
}

#[test]
fn external_witness_cooperation_change_does_not_refresh_police_case_activity() {
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let activity_before = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .last_activity_at();

    fixture.state.advance_clock(SimDuration::from_minutes(60));
    validate_set_witness_cooperation(&fixture.state, case_witness, WitnessCooperation::Reluctant)
        .expect("external cooperation degradation should validate")
        .commit(&mut fixture.state)
        .expect("external cooperation degradation should commit");

    let investigation = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist after witness change");
    assert_eq!(
        investigation.last_activity_at(),
        activity_before,
        "a witness-side change must not reset the police institution's inactivity clock"
    );
    validate_state(&fixture.state).expect("witness-side cooperation change should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn restore_rejects_witness_attempt_counter_without_completed_interview_history() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let record = fixture
        .state
        .legal()
        .get_case_witness(case_witness)
        .expect("registered witness should persist")
        .clone();
    assert_eq!(record.interview_attempts(), 0);
    assert_eq!(record.version(), 1);
    let mut corrupted = case_witness_wire(&record);
    corrupted.interview_attempts = 1;
    // Make the version superficially plausible as though one mutation occurred. Restore must
    // still reject because there is no completed WitnessInterview work record backing the
    // future-affecting attempt counter.
    corrupted.version = 2;

    let error = restore_save(
        &registry,
        replace_serialized_case_witness(
            build_save(&registry, &fixture.state)
                .expect("valid case witness should save before attempt corruption"),
            &record,
            &corrupted,
        ),
    )
    .expect_err("interview attempts must be derived from completed interview work");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidCaseWitness {
                witness: invalid
            }
        ) if invalid == case_witness
    ));
}

#[test]
fn named_witness_statement_creates_source_bearing_testimony_and_survives_save() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    let outcome = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            subject: EntityRef::Character(fixture.subject),
            origin: Some(EntityRef::Organization(fixture.criminal)),
            confidence: rating(88),
            summary: "Mercer identifies Frank Dello as the man he saw leaving the crew's garage."
                .to_owned(),
        },
    )
    .expect("named witness statement should validate")
    .commit(&mut fixture.state)
    .expect("named witness statement should commit");

    let statement = fixture
        .state
        .legal()
        .get_witness_statement(outcome.statement)
        .expect("statement should exist");
    assert_eq!(statement.case_witness(), case_witness);
    assert_eq!(statement.evidence(), outcome.evidence);
    assert_eq!(statement.confidence(), rating(88));
    let evidence = fixture
        .state
        .legal()
        .get_evidence(outcome.evidence)
        .expect("statement evidence should exist");
    assert_eq!(evidence.kind(), EvidenceKind::WitnessTestimony);
    assert_eq!(evidence.strength(), EvidenceStrength::Direct);
    assert_eq!(evidence.reliability(), EvidenceReliability::HighlyReliable);
    assert_eq!(evidence.admissibility(), Admissibility::Unknown);
    assert_eq!(evidence.subject(), EntityRef::Character(fixture.subject));
    assert_eq!(
        evidence.origin(),
        Some(EntityRef::Organization(fixture.criminal))
    );
    assert_eq!(
        evidence.source(),
        Some(EntityRef::Character(fixture.witness))
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_evidence(outcome.evidence)
            .map(|record| record.source()),
        Some(Some(EntityRef::Character(fixture.witness)))
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .witness_statement_for_evidence(outcome.evidence)
            .map(|record| record.id()),
        Some(outcome.statement)
    );
    assert_eq!(
        validate_record_witness_statement(
            &registry,
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                confidence: rating(95),
                summary: "Mercer repeats the same identification.".to_owned(),
            },
        )
        .expect_err("one case witness cannot manufacture corroboration by repeating testimony"),
        WitnessError::WitnessAlreadyStatemented(case_witness)
    );

    let mut restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("named testimony state should save"),
    )
    .expect("named testimony state should restore");
    let restored_evidence = restored
        .legal()
        .get_evidence(outcome.evidence)
        .expect("restored witness evidence should exist");
    assert_eq!(
        restored_evidence.source(),
        Some(EntityRef::Character(fixture.witness))
    );
    assert_eq!(
        restored
            .legal()
            .witness_statement_for_evidence(outcome.evidence)
            .map(|record| record.id()),
        Some(outcome.statement)
    );

    let second_witness = insert_character(
        &mut restored,
        CharacterDraft {
            name: "Nora Bell".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("post-restore witness fixture should validate");
    let second_case_witness = validate_register_case_witness(
        &restored,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: second_witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("post-restore witness registration should validate")
    .commit(&mut restored)
    .expect("post-restore witness registration should allocate a fresh ID");
    let second_statement = validate_record_witness_statement(
        &registry,
        &restored,
        WitnessStatementDraft {
            case_witness: second_case_witness,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            confidence: rating(61),
            summary: "Bell separately places Dello near the garage that evening.".to_owned(),
        },
    )
    .expect("post-restore testimony should validate")
    .commit(&mut restored)
    .expect("post-restore testimony should allocate fresh statement and evidence IDs");
    assert!(second_case_witness.raw() > case_witness.raw());
    assert!(second_statement.statement.raw() > outcome.statement.raw());
    assert!(second_statement.evidence.raw() > outcome.evidence.raw());
    validate_state(&restored).expect("restored testimony state should be structurally valid");
    validate_state_against_registry(&registry, &restored)
        .expect("restored testimony state should remain registry-valid");
    validate_invariants(&restored);
}

#[test]
fn witness_registration_and_cooperation_tokens_reject_case_and_statement_changes() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let stale_registration = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("registration should initially validate");
    validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("case mutation should validate")
    .commit(&mut fixture.state)
    .expect("case mutation should commit");
    assert!(matches!(
        stale_registration.commit(&mut fixture.state),
        Err(WitnessError::StaleInvestigation { .. })
    ));

    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("fresh registration should validate")
    .commit(&mut fixture.state)
    .expect("fresh registration should commit");
    assert_eq!(
        validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect_err("same character cannot be registered twice on one case"),
        WitnessError::DuplicateCaseWitness {
            investigation: fixture.investigation,
            witness: fixture.witness,
            existing: case_witness,
        }
    );

    let stale_cooperation = validate_set_witness_cooperation(
        &fixture.state,
        case_witness,
        WitnessCooperation::Cooperative,
    )
    .expect("cooperation change should initially validate");
    validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            confidence: rating(55),
            summary: "Mercer says he is fairly sure Dello was present.".to_owned(),
        },
    )
    .expect("statement should validate")
    .commit(&mut fixture.state)
    .expect("statement should commit");
    assert!(matches!(
        stale_cooperation.commit(&mut fixture.state),
        Err(WitnessError::StaleCaseWitness { .. })
    ));
    validate_set_witness_cooperation(
        &fixture.state,
        case_witness,
        WitnessCooperation::Cooperative,
    )
    .expect("fresh cooperation token should validate")
    .commit(&mut fixture.state)
    .expect("fresh cooperation change should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_case_witness(case_witness)
            .expect("case witness should exist")
            .cooperation(),
        WitnessCooperation::Cooperative
    );
    validate_state(&fixture.state).expect("versioned witness state should remain valid");
}

#[test]
fn suspended_case_preserves_testimony_but_rejects_new_witness_activity() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let case_witness = validate_register_case_witness(
        &fixture.state,
        CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("registration should validate")
    .commit(&mut fixture.state)
    .expect("registration should commit");
    let historical = validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            confidence: rating(72),
            summary: "Mercer identifies Dello from the alley encounter.".to_owned(),
        },
    )
    .expect("historical statement should validate")
    .commit(&mut fixture.state)
    .expect("historical statement should commit");
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .expect("case suspension should validate")
    .commit(&mut fixture.state)
    .expect("case suspension should commit");

    let statement_error = match validate_record_witness_statement(
        &registry,
        &fixture.state,
        WitnessStatementDraft {
            case_witness,
            subject: EntityRef::Character(fixture.subject),
            origin: None,
            confidence: rating(90),
            summary: "Mercer offers a second identification.".to_owned(),
        },
    ) {
        Ok(_) => panic!("suspended case must reject new witness statements"),
        Err(error) => error,
    };
    assert_eq!(
        statement_error,
        WitnessError::InactiveInvestigation(fixture.investigation)
    );
    assert_eq!(
        validate_set_witness_cooperation(
            &fixture.state,
            case_witness,
            WitnessCooperation::Hostile,
        )
        .expect_err("suspended case must reject cooperation mutation"),
        WitnessError::InactiveInvestigation(fixture.investigation)
    );
    assert!(
        fixture
            .state
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    assert!(
        fixture
            .state
            .legal()
            .get_evidence(historical.evidence)
            .is_some()
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("suspended case with testimony should save"),
    )
    .expect("suspended case with testimony should restore");
    assert!(
        restored
            .legal()
            .get_witness_statement(historical.statement)
            .is_some()
    );
    validate_state(&restored).expect("historical testimony should survive suspension");
    validate_invariants(&restored);
}

#[test]
fn anonymous_witness_testimony_remains_valid_without_named_source() {
    let registry = build_registry();
    let mut fixture = make_fixture();
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.subject),
            origin: Some(EntityRef::Organization(fixture.criminal)),
            kind: EvidenceKind::WitnessTestimony,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("anonymous testimony should remain valid evidence")
    .commit(&mut fixture.state)
    .expect("anonymous testimony should commit");
    let record = fixture
        .state
        .legal()
        .get_evidence(evidence)
        .expect("anonymous testimony should exist");
    assert_eq!(record.kind(), EvidenceKind::WitnessTestimony);
    assert_eq!(record.source(), None);
    assert!(
        fixture
            .state
            .legal()
            .witness_statement_for_evidence(evidence)
            .is_none()
    );
    validate_state(&fixture.state).expect("anonymous testimony should remain structurally valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("anonymous testimony should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_confidence_maps_to_deterministic_evidence_bands() {
    let testimony = build_registry().legal().witness_testimony();
    for (confidence, strength) in [
        (0, EvidenceStrength::Weak),
        (34, EvidenceStrength::Weak),
        (35, EvidenceStrength::Corroborating),
        (59, EvidenceStrength::Corroborating),
        (60, EvidenceStrength::Strong),
        (84, EvidenceStrength::Strong),
        (85, EvidenceStrength::Direct),
        (100, EvidenceStrength::Direct),
    ] {
        assert_eq!(
            resolve_witness_strength(
                testimony,
                rating(confidence),
                WitnessCooperation::Cooperative,
            ),
            strength
        );
    }
    for (confidence, reliability) in [
        (0, EvidenceReliability::Questionable),
        (24, EvidenceReliability::Questionable),
        (25, EvidenceReliability::Mixed),
        (49, EvidenceReliability::Mixed),
        (50, EvidenceReliability::Credible),
        (79, EvidenceReliability::Credible),
        (80, EvidenceReliability::HighlyReliable),
        (100, EvidenceReliability::HighlyReliable),
    ] {
        assert_eq!(
            resolve_witness_reliability(
                testimony,
                rating(confidence),
                WitnessCooperation::Cooperative,
            ),
            reliability
        );
    }
}

#[test]
fn uncooperative_witnesses_cannot_produce_top_band_testimony() {
    let testimony = build_registry().legal().witness_testimony();
    for confidence in 0..=100 {
        let confidence = rating(confidence);
        for (cooperation, strength_cap, reliability_cap) in [
            (
                WitnessCooperation::Cooperative,
                EvidenceStrength::Direct,
                EvidenceReliability::HighlyReliable,
            ),
            (
                WitnessCooperation::Reluctant,
                EvidenceStrength::Strong,
                EvidenceReliability::Credible,
            ),
            (
                WitnessCooperation::Hostile,
                EvidenceStrength::Corroborating,
                EvidenceReliability::Mixed,
            ),
        ] {
            let strength = resolve_witness_strength(testimony, confidence, cooperation);
            assert!(strength <= strength_cap);
            let reliability = resolve_witness_reliability(testimony, confidence, cooperation);
            assert!(reliability <= reliability_cap);
        }
    }
}
