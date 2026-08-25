//! Focused tests for deterministic investigation-work scheduling and resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::legal::investigation_system::{
    validate_add_evidence, validate_assign_investigator, validate_open_investigation,
    validate_remove_investigator, InvestigationError,
};
use crate::legal::{EvidenceDraft, InvestigationDraft, InvestigationWorkFocus, InvestigatorRole};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating};
use std::collections::{BTreeMap, BTreeSet};

struct WorkFixture {
    state: AppState,
    police: crate::core::id::OrganizationId,
    investigation: InvestigationId,
    investigator: CharacterId,
    second_investigator: CharacterId,
    first: CharacterId,
    middle: CharacterId,
    target: CharacterId,
    first_evidence: EvidenceId,
    /// Kept in the case graph so review support has multi-evidence context; not focused directly.
    _second_evidence: EvidenceId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn make_fixture(
    investigator_skill: u8,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
) -> WorkFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x1A7E_5731);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Pattern Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Pattern Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let investigator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detective Harlan".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                rating(investigator_skill),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("investigator fixture should validate");
    let second_investigator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detective Vera".to_owned(),
            organization: Some(police),
            supervisor: Some(investigator),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                rating(investigator_skill),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second investigator fixture should validate");
    let mut insert_subject = |name: &str| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("case subject fixture should validate")
    };
    let first = insert_subject("Frank Dello");
    let middle = insert_subject("Maria Vale");
    let target = insert_subject("Fulton Garage Manager");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Vehicle association inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(first)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("investigation fixture should commit");
    validate_assign_investigator(&state, investigation, investigator, InvestigatorRole::Lead)
        .expect("investigator assignment should validate")
        .commit(&mut state)
        .expect("investigator assignment should commit");

    let first_evidence = add_evidence(
        &mut state,
        TestEvidenceDraft {
            investigation,
            police,
            subject: EntityRef::Character(middle),
            origin: EntityRef::Character(first),
            // Reviewable so evidence-review work drafts can focus it.
            kind: EvidenceKind::Fingerprint,
            strength,
            reliability,
            admissibility,
        },
    );
    let second_evidence = add_evidence(
        &mut state,
        TestEvidenceDraft {
            investigation,
            police,
            subject: EntityRef::Character(target),
            origin: EntityRef::Character(middle),
            kind: EvidenceKind::KnownAssociation,
            strength,
            reliability,
            admissibility,
        },
    );
    WorkFixture {
        state,
        police,
        investigation,
        investigator,
        second_investigator,
        first,
        middle,
        target,
        first_evidence,
        _second_evidence: second_evidence,
    }
}

struct TestEvidenceDraft {
    investigation: InvestigationId,
    police: crate::core::id::OrganizationId,
    subject: EntityRef,
    origin: EntityRef,
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
}

fn add_evidence(state: &mut AppState, draft: TestEvidenceDraft) -> EvidenceId {
    let TestEvidenceDraft {
        investigation,
        police,
        subject,
        origin,
        kind,
        strength,
        reliability,
        admissibility,
    } = draft;
    validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject,
            origin: Some(origin),
            kind,
            strength,
            reliability,
            admissibility,
            discovered_at: state.now(),
        },
    )
    .expect("evidence fixture should validate")
    .commit(state)
    .expect("evidence fixture should commit")
}

fn review_draft(fixture: &WorkFixture, evidence: EvidenceId) -> InvestigationWorkDraft {
    InvestigationWorkDraft {
        investigation: fixture.investigation,
        investigator: fixture.investigator,
        kind: InvestigationWorkKind::EvidenceReview,
        focus: InvestigationWorkFocus::evidence(evidence),
    }
}

#[test]
fn witness_interview_scheduling_stops_after_the_authored_attempt_limit() {
    // Regression: a completed interview produces a statement only when it connects, so a
    // hostile witness facing an incapable investigator used to be re-scheduled forever,
    // consuming institutional work and refreshing the case's cold-case clock each cycle.
    let registry = build_registry();
    let mut fixture = make_fixture(
        0,
        EvidenceStrength::Weak,
        EvidenceReliability::Mixed,
        Admissibility::Unknown,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.first,
            cooperation: crate::legal::WitnessCooperation::Hostile,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");

    let limit = u32::from(registry.legal().witness_interview_attempt_limit());
    for attempt in 1..=limit {
        let scheduled = apply_witness_interview_scheduling(&registry, &mut fixture.state)
            .expect("interview scheduling pass should resolve");
        assert_eq!(
            scheduled.len(),
            1,
            "attempt {attempt} should schedule exactly one interview"
        );
        loop {
            let outcome = run_tick(&registry, &mut fixture.state);
            if !outcome.resolved_investigation_work.is_empty() {
                break;
            }
        }
    }

    // The authored attempt budget is spent: the scheduling pass must propose nothing further,
    // even with the witness still statementless and the case active.
    let witness = fixture
        .state
        .legal()
        .get_case_witness(case_witness)
        .expect("case witness should persist");
    assert_eq!(
        u32::from(witness.interview_attempts()),
        limit,
        "every completed interview counts against the budget"
    );
    assert!(
        apply_witness_interview_scheduling(&registry, &mut fixture.state)
            .expect("exhausted scheduling pass should resolve")
            .is_empty()
    );
    validate_state(&fixture.state).expect("capped interview state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("capped interview state should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn evidence_review_develops_case_owned_evidence_without_inventing_subjects() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let fingerprint = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.first),
            origin: None,
            kind: EvidenceKind::Fingerprint,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("fingerprint evidence should validate")
    .commit(&mut fixture.state)
    .expect("fingerprint evidence should commit");
    let subjects_before = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .subjects()
        .clone();
    let draft = InvestigationWorkDraft {
        investigation: fixture.investigation,
        investigator: fixture.investigator,
        kind: InvestigationWorkKind::EvidenceReview,
        focus: InvestigationWorkFocus::evidence(fingerprint),
    };
    let work = validate_schedule_investigation_work(&registry, &fixture.state, draft)
        .expect("case-owned fingerprint should support evidence review")
        .commit(&mut fixture.state)
        .expect("evidence review should schedule");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("scheduled evidence review should persist")
            .due_at(),
        SimTime::from_minutes(180)
    );

    for _ in 0..179 {
        assert!(run_tick(&registry, &mut fixture.state)
            .resolved_investigation_work
            .is_empty());
    }
    let outcome = run_tick(&registry, &mut fixture.state);
    assert_eq!(outcome.resolved_investigation_work, vec![work]);
    let record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("completed evidence review should persist");
    let resolution = record
        .resolution()
        .expect("review should have a resolution");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Developed);
    let derived_id = resolution
        .derived_evidence()
        .expect("successful evidence review should derive forensic analysis");
    let source = fixture
        .state
        .legal()
        .get_evidence(fingerprint)
        .expect("source fingerprint should persist");
    let derived = fixture
        .state
        .legal()
        .get_evidence(derived_id)
        .expect("forensic analysis should persist");
    assert_eq!(derived.kind(), EvidenceKind::ForensicAnalysis);
    assert_eq!(derived.subject(), source.subject());
    assert_eq!(derived.origin(), source.origin());
    assert_eq!(derived.strength(), source.strength());
    assert_eq!(derived.reliability(), EvidenceReliability::Credible);
    assert_eq!(derived.admissibility(), source.admissibility());
    assert_eq!(derived.derived_from(), &BTreeSet::from([fingerprint]));
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist after review")
            .subjects(),
        &subjects_before
    );
    assert!(matches!(
        validate_schedule_investigation_work(&registry, &fixture.state, draft),
        Err(InvestigationWorkError::EvidenceAlreadyReviewed {
            evidence,
            derived
        }) if evidence == fingerprint && derived == derived_id
    ));
    validate_state(&fixture.state).expect("evidence review state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("evidence review should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn scheduling_is_versioned_deduplicated_and_blocks_investigator_release() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let stale_removal =
        validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator)
            .expect("investigator should initially be releasable");
    let stale_schedule = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("initial schedule token should validate");
    add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.middle),
            origin: EntityRef::Character(fixture.target),
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
        },
    );
    assert!(matches!(
        stale_schedule.commit(&mut fixture.state),
        Err(InvestigationWorkError::StaleInvestigation { .. })
    ));

    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("fresh schedule should validate after case change")
    .commit(&mut fixture.state)
    .expect("fresh schedule should commit");
    assert!(matches!(
        stale_removal.commit(&mut fixture.state),
        Err(InvestigationError::StaleInvestigation { .. })
    ));
    assert_eq!(
        validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator,)
            .expect_err("scheduled work must block investigator release"),
        InvestigationError::ScheduledInvestigationWork {
            investigator: fixture.investigator,
            work,
        }
    );
    assert_eq!(
        validate_schedule_investigation_work(
            &registry,
            &fixture.state,
            review_draft(&fixture, fixture.first_evidence)
        )
        .expect_err("same focus must not schedule duplicate work"),
        InvestigationWorkError::DuplicateScheduledWork { work }
    );
    validate_state(&fixture.state).expect("scheduled work dependencies should remain valid");
}

#[test]
fn save_round_trip_preserves_due_work_and_deterministic_resolution() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("evidence review should validate")
    .commit(&mut fixture.state)
    .expect("evidence review should schedule");
    for _ in 0..179 {
        run_tick(&registry, &mut fixture.state);
    }
    let mut restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("pending work should save"),
    )
    .expect("pending work should restore");
    let original_outcome = run_tick(&registry, &mut fixture.state);
    let restored_outcome = run_tick(&registry, &mut restored);
    assert_eq!(original_outcome, restored_outcome);
    assert_eq!(original_outcome.resolved_investigation_work, vec![work]);

    let original_resolution = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("original work should exist")
        .resolution()
        .expect("original work should resolve")
        .clone();
    let restored_resolution = restored
        .legal()
        .get_investigation_work(work)
        .expect("restored work should exist")
        .resolution()
        .expect("restored work should resolve")
        .clone();
    assert_eq!(original_resolution, restored_resolution);
    let original_derived = original_resolution
        .derived_evidence()
        .expect("strong work should derive evidence");
    let restored_derived = restored_resolution
        .derived_evidence()
        .expect("restored strong work should derive evidence");
    assert_eq!(original_derived, restored_derived);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_evidence(original_derived)
            .expect("original derived evidence should exist")
            .derived_from(),
        restored
            .legal()
            .get_evidence(restored_derived)
            .expect("restored derived evidence should exist")
            .derived_from()
    );

    let second_investigation = validate_open_investigation(
        &restored,
        InvestigationDraft {
            owner: fixture.police,
            title: "Post-restore association inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.first)]),
        },
    )
    .expect("post-restore investigation should validate")
    .commit(&mut restored)
    .expect("post-restore investigation should commit");
    validate_assign_investigator(
        &restored,
        second_investigation,
        fixture.second_investigator,
        InvestigatorRole::Lead,
    )
    .expect("post-restore investigator assignment should validate")
    .commit(&mut restored)
    .expect("post-restore investigator assignment should commit");
    add_evidence(
        &mut restored,
        TestEvidenceDraft {
            investigation: second_investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.middle),
            origin: EntityRef::Character(fixture.first),
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    let restored_evidence = add_evidence(
        &mut restored,
        TestEvidenceDraft {
            investigation: second_investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: EntityRef::Character(fixture.middle),
            kind: EvidenceKind::Fingerprint,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    let second_work = validate_schedule_investigation_work(
        &registry,
        &restored,
        InvestigationWorkDraft {
            investigation: second_investigation,
            investigator: fixture.second_investigator,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(restored_evidence),
        },
    )
    .expect("post-restore evidence review should validate")
    .commit(&mut restored)
    .expect("post-restore evidence review should allocate a fresh work ID");
    assert!(second_work.raw() > work.raw());
    validate_state_against_registry(&registry, &restored)
        .expect("restored work should retain authored causal validity");
}
