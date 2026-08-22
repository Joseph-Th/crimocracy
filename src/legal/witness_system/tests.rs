//! Focused tests for witness registration, interviews, testimony, and pressure effects.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save};
use crate::legal::investigation_system::{
    validate_add_evidence, validate_open_investigation, validate_transition_investigation,
    InvestigationTransition,
};
use crate::legal::{EvidenceDraft, InvestigationDraft, WitnessStatementDraft};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating};
use std::collections::{BTreeMap, BTreeSet};

struct WitnessFixture {
    state: AppState,
    police: crate::core::id::OrganizationId,
    criminal: crate::core::id::OrganizationId,
    investigation: InvestigationId,
    witness: CharacterId,
    subject: CharacterId,
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
    assert!(fixture
        .state
        .legal()
        .get_witness_statement(historical.statement)
        .is_some());
    assert!(fixture
        .state
        .legal()
        .get_evidence(historical.evidence)
        .is_some());

    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("suspended case with testimony should save"),
    )
    .expect("suspended case with testimony should restore");
    assert!(restored
        .legal()
        .get_witness_statement(historical.statement)
        .is_some());
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
    assert!(fixture
        .state
        .legal()
        .witness_statement_for_evidence(evidence)
        .is_none());
    validate_state(&fixture.state).expect("anonymous testimony should remain structurally valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("anonymous testimony should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_confidence_maps_to_deterministic_evidence_bands() {
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
            resolve_witness_strength(rating(confidence), WitnessCooperation::Cooperative),
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
            resolve_witness_reliability(rating(confidence), WitnessCooperation::Cooperative),
            reliability
        );
    }
}

#[test]
fn uncooperative_witnesses_cannot_produce_top_band_testimony() {
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
            let strength = resolve_witness_strength(confidence, cooperation);
            assert!(strength <= strength_cap);
            let reliability = resolve_witness_reliability(confidence, cooperation);
            assert!(reliability <= reliability_cap);
        }
    }
}
