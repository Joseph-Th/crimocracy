//! Focused tests for deterministic investigation-work scheduling and resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{
    validate_add_evidence, validate_assign_investigator, validate_open_investigation,
};
use crate::legal::{ArrestDraft, EvidenceDraft, InvestigationDraft, InvestigationWorkFocus};
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
    witness: CharacterId,
    first_evidence: EvidenceId,
    /// Kept in the case graph so review support has multi-evidence context; not focused directly.
    _second_evidence: EvidenceId,
}

fn run_until_work_resolved(registry: &Registry, state: &mut AppState, work: InvestigationWorkId) {
    let due_at = state
        .legal()
        .get_investigation_work(work)
        .expect("scheduled fixture work should persist")
        .due_at();
    let remaining_ticks = due_at
        .as_minutes()
        .checked_sub(state.now().as_minutes())
        .expect("scheduled fixture work cannot already be overdue");
    assert!(
        remaining_ticks > 0,
        "fixture work wait must begin before its due time"
    );
    for _ in 0..remaining_ticks {
        let outcome = run_tick(registry, state);
        if outcome.resolved_investigation_work.contains(&work) {
            return;
        }
        assert_eq!(
            state
                .legal()
                .get_investigation_work(work)
                .expect("fixture work should persist while awaiting resolution")
                .status(),
            InvestigationWorkStatus::Scheduled,
            "fixture work {work} left Scheduled without appearing in the resolution outcome"
        );
    }
    panic!("fixture work {work} did not resolve by its due time {due_at:?}");
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
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Independent Case Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("case witness fixture should validate");
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
    validate_assign_investigator(&state, investigation, investigator)
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
        witness,
        first_evidence,
        _second_evidence: second_evidence,
    }
}

#[test]
fn held_work_schedule_rejects_clock_overflow_before_work_id_is_consumed() {
    use crate::core::id::IdKind;

    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let validated = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("work should validate before the clock moves");
    let duration = registry
        .get_investigation_work(InvestigationWorkKind::EvidenceReview)
        .duration();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(duration.as_minutes()) + 1,
    ));
    let next_work = fixture.state.ids.next_raw(IdKind::InvestigationWork);

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("held work token must reject an unrepresentable due time");
    assert_eq!(error, InvestigationWorkError::SimulationTimeOverflow);
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::InvestigationWork),
        next_work
    );
    assert_eq!(fixture.state.legal().investigation_work().count(), 0);
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
fn autonomous_evidence_review_advances_to_each_reviewable_source_once() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let later_source = add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: EntityRef::Character(fixture.middle),
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );

    let first = apply_evidence_review_scheduling(&registry, &mut fixture.state)
        .expect("first autonomous review should schedule");
    assert_eq!(first.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(first[0])
            .expect("first review should persist")
            .focus(),
        InvestigationWorkFocus::evidence(fixture.first_evidence)
    );

    let duration = registry
        .get_investigation_work(InvestigationWorkKind::EvidenceReview)
        .duration();
    fixture.state.advance_clock(duration);
    let first_plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        first[0],
        InvestigationWorkRandomness::new(0),
    )
    .expect("first review should resolve");
    validate_investigation_work_resolution_plan(&registry, &fixture.state, first_plan)
        .expect("first resolution should validate")
        .commit(&mut fixture.state)
        .expect("first resolution should commit");

    let second = apply_evidence_review_scheduling(&registry, &mut fixture.state)
        .expect("later reviewable evidence should remain actionable");
    assert_eq!(second.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(second[0])
            .expect("second review should persist")
            .focus(),
        InvestigationWorkFocus::evidence(later_source),
        "a completed review of one source must not make later reviewable evidence inert"
    );

    fixture.state.advance_clock(duration);
    let second_plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        second[0],
        InvestigationWorkRandomness::new(0),
    )
    .expect("second review should resolve");
    validate_investigation_work_resolution_plan(&registry, &fixture.state, second_plan)
        .expect("second resolution should validate")
        .commit(&mut fixture.state)
        .expect("second resolution should commit");
    assert!(
        apply_evidence_review_scheduling(&registry, &mut fixture.state)
            .expect("exhausted review scheduling should resolve")
            .is_empty(),
        "each reviewable source receives one autonomous attempt"
    );
    validate_state(&fixture.state).expect("multi-source review state should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn detention_cancellation_token_stales_when_case_changes_before_commit() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("review work should validate")
    .commit(&mut fixture.state)
    .expect("review work should schedule");
    let cancellation =
        validate_cancel_investigation_work_for_detention(&fixture.state, fixture.investigator)
            .expect("detention cancellation should validate")
            .expect("scheduled investigator should have work to cancel");
    let expected_version = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .version();

    add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: EntityRef::Character(fixture.middle),
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    let found_version = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .version();
    assert_eq!(found_version, expected_version + 1);
    assert_eq!(
        cancellation
            .ensure_current(&fixture.state)
            .expect_err("case mutation must stale the cancellation token"),
        InvestigationWorkError::StaleInvestigation {
            investigation: fixture.investigation,
            expected: expected_version,
            found: found_version,
        }
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("stale cancellation must preserve work")
            .status(),
        InvestigationWorkStatus::Scheduled
    );
    validate_state(&fixture.state).expect("stale cancellation must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_evidence_review_does_not_repeat_an_inconclusive_attempt() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        0,
        EvidenceStrength::Weak,
        EvidenceReliability::Questionable,
        Admissibility::Inadmissible,
    );
    let scheduled = apply_evidence_review_scheduling(&registry, &mut fixture.state)
        .expect("weak reviewable evidence should still receive one review attempt");
    assert_eq!(scheduled.len(), 1);
    let duration = registry
        .get_investigation_work(InvestigationWorkKind::EvidenceReview)
        .duration();
    fixture.state.advance_clock(duration);
    let plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        scheduled[0],
        InvestigationWorkRandomness::new(0),
    )
    .expect("weak review should resolve");
    assert_eq!(plan.outcome(), InvestigationWorkOutcome::Inconclusive);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
        .expect("inconclusive resolution should validate")
        .commit(&mut fixture.state)
        .expect("inconclusive resolution should commit");

    assert!(
        apply_evidence_review_scheduling(&registry, &mut fixture.state)
            .expect("completed inconclusive source should be exhausted")
            .is_empty(),
        "autonomous casework must not retry one inconclusive source forever and keep the case artificially active"
    );
    validate_state(&fixture.state).expect("inconclusive review state should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn custody_cancelled_evidence_review_is_retryable_after_restaffing() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let first_review = apply_evidence_review_scheduling(&registry, &mut fixture.state)
        .expect("initial review should schedule")[0];

    let misconduct_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Detective misconduct inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.investigator)]),
        },
    )
    .expect("misconduct case should validate")
    .commit(&mut fixture.state)
    .expect("misconduct case should commit");
    let misconduct_evidence = add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: misconduct_case,
            police: fixture.police,
            subject: EntityRef::Character(fixture.investigator),
            origin: EntityRef::Character(fixture.first),
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    let misconduct_corroboration = add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: misconduct_case,
            police: fixture.police,
            subject: EntityRef::Character(fixture.investigator),
            origin: EntityRef::Character(fixture.second_investigator),
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.investigator,
            investigation: misconduct_case,
            evidence: BTreeSet::from([misconduct_evidence, misconduct_corroboration]),
        },
    )
    .expect("detective arrest should validate")
    .commit(&mut fixture.state)
    .expect("detective arrest should cancel scheduled work");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(first_review)
            .expect("cancelled review should remain history")
            .status(),
        InvestigationWorkStatus::Cancelled
    );
    validate_assign_investigator(
        &fixture.state,
        fixture.investigation,
        fixture.second_investigator,
    )
    .expect("replacement detective should validate")
    .commit(&mut fixture.state)
    .expect("replacement detective should commit");

    let retried = apply_evidence_review_scheduling(&registry, &mut fixture.state)
        .expect("cancelled source should be retryable after restaffing");
    assert_eq!(retried.len(), 1);
    let retry = fixture
        .state
        .legal()
        .get_investigation_work(retried[0])
        .expect("retried review should persist");
    assert_eq!(retry.investigator(), fixture.second_investigator);
    assert_eq!(
        retry.focus(),
        InvestigationWorkFocus::evidence(fixture.first_evidence)
    );
    validate_state(&fixture.state).expect("retried review state should remain valid");
    validate_invariants(&fixture.state);
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
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
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
        let interview = scheduled[0];
        run_until_work_resolved(&registry, &mut fixture.state, interview);
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
    let direct_error = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect_err("direct canonical scheduling must enforce the same authored attempt limit");
    assert_eq!(
        direct_error,
        InvestigationWorkError::WitnessInterviewLimitReached {
            witness: case_witness,
            attempts: u8::try_from(limit).expect("authored attempt limit must fit u8"),
            limit: registry.legal().witness_interview_attempt_limit(),
        }
    );
    validate_state(&fixture.state).expect("capped interview state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("capped interview state should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_interview_scheduling_prioritizes_unattempted_witness_before_retry() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        0,
        EvidenceStrength::Weak,
        EvidenceReliability::Mixed,
        Admissibility::Unknown,
    );
    let first_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Hostile,
        },
    )
    .expect("first witness registration should validate")
    .commit(&mut fixture.state)
    .expect("first witness registration should commit");
    let first_interview = apply_witness_interview_scheduling(&registry, &mut fixture.state)
        .expect("first interview scheduling should resolve")[0];
    let duration = registry
        .get_investigation_work(InvestigationWorkKind::WitnessInterview)
        .duration();
    fixture.state.advance_clock(duration);
    let first_plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        first_interview,
        InvestigationWorkRandomness::new(0),
    )
    .expect("hostile low-skill interview should resolve");
    assert_eq!(first_plan.outcome(), InvestigationWorkOutcome::Inconclusive);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, first_plan)
        .expect("first interview resolution should validate")
        .commit(&mut fixture.state)
        .expect("first interview resolution should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_case_witness(first_witness)
            .expect("first witness should persist")
            .interview_attempts(),
        1
    );

    let untouched_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.middle,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Hostile,
        },
    )
    .expect("second witness registration should validate")
    .commit(&mut fixture.state)
    .expect("second witness registration should commit");

    let scheduled = apply_witness_interview_scheduling(&registry, &mut fixture.state)
        .expect("fresh-witness scheduling should resolve");
    assert_eq!(scheduled.len(), 1);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(scheduled[0])
            .expect("scheduled interview should persist")
            .focus(),
        InvestigationWorkFocus::witness(untouched_witness),
        "an untouched witness should be interviewed before retrying an earlier failed witness"
    );
    validate_state(&fixture.state).expect("fresh-witness priority state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn direct_interview_scheduling_rejects_witness_who_already_gave_statement() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Weak,
        EvidenceReliability::Mixed,
        Admissibility::Unknown,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.target),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness registration should validate")
    .commit(&mut fixture.state)
    .expect("case witness registration should commit");
    crate::legal::witness_system::validate_record_witness_statement(
        &registry,
        &fixture.state,
        crate::legal::WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(80),
            summary: "The witness already gave a usable account.".to_owned(),
        },
    )
    .expect("statement should validate")
    .commit(&mut fixture.state)
    .expect("statement should commit");

    let error = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect_err("a statemented witness must not consume another detective interview");
    assert_eq!(
        error,
        InvestigationWorkError::WitnessAlreadyStatemented {
            witness: case_witness,
        }
    );
    validate_state(&fixture.state).expect("rejected redundant interview must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn interview_statement_uses_registered_subject_despite_other_case_evidence() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.first),
            origin: EntityRef::Character(fixture.middle),
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Direct,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Admissible,
        },
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("case witness should validate")
    .commit(&mut fixture.state)
    .expect("case witness should commit");
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect("witness interview should validate")
    .commit(&mut fixture.state)
    .expect("witness interview should commit");
    let duration = registry
        .get_investigation_work(InvestigationWorkKind::WitnessInterview)
        .duration();
    fixture.state.advance_clock(duration);
    let plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        work,
        InvestigationWorkRandomness::new(0),
    )
    .expect("interview should resolve through the canonical decision path");
    assert_eq!(plan.outcome(), InvestigationWorkOutcome::Connected);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
        .expect("interview resolution should validate")
        .commit(&mut fixture.state)
        .expect("interview resolution should commit");
    let statement_id = *fixture
        .state
        .legal()
        .get_case_witness(case_witness)
        .expect("case witness should persist")
        .statements()
        .iter()
        .next()
        .expect("connected interview must persist its statement");
    let statement = fixture
        .state
        .legal()
        .get_witness_statement(statement_id)
        .expect("persisted witness statement should resolve");

    assert_eq!(
        statement.subject(),
        EntityRef::Character(fixture.first),
        "later or stronger case evidence must not make a witness testify about a different subject"
    );
    validate_state(&fixture.state).expect("statement-selection fixture should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn actionable_evidence_against_witness_cancels_pending_interview_and_blocks_future_interviews() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Weak,
        EvidenceReliability::Mixed,
        Admissibility::Unknown,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("independent witness should register")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect("witness interview should initially schedule")
    .commit(&mut fixture.state)
    .expect("witness interview should commit");

    let promotion = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.witness),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("new evidence against the witness should validate")
    .commit(&mut fixture.state)
    .expect("new evidence should promote the witness into the case subject set");

    let record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("invalidated interview should remain historical");
    assert_eq!(record.status(), InvestigationWorkStatus::Cancelled);
    assert_eq!(
        record
            .cancellation()
            .expect("invalidated interview should record cancellation")
            .reason(),
        InvestigationWorkCancellationReason::WitnessBecameCaseSubject(promotion)
    );
    assert!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist")
            .subjects()
            .contains(&EntityRef::Character(fixture.witness))
    );
    assert_eq!(
        validate_schedule_investigation_work(
            &registry,
            &fixture.state,
            InvestigationWorkDraft {
                investigation: fixture.investigation,
                investigator: fixture.investigator,
                kind: InvestigationWorkKind::WitnessInterview,
                focus: InvestigationWorkFocus::witness(case_witness),
            },
        )
        .expect_err("a promoted case subject cannot be scheduled as a witness again"),
        InvestigationWorkError::WitnessIsCaseSubject {
            witness: case_witness,
            character: fixture.witness,
        }
    );
    assert_eq!(
        crate::legal::witness_system::validate_set_witness_cooperation(
            &fixture.state,
            case_witness,
            crate::legal::WitnessCooperation::Hostile,
        )
        .expect_err("a promoted case subject cannot keep acting as a witness"),
        crate::legal::witness_system::WitnessError::WitnessIsCaseSubject {
            investigation: fixture.investigation,
            witness: fixture.witness,
        }
    );
    validate_state(&fixture.state).expect("witness-role invalidation should leave valid state");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("witness-role invalidation should remain registry-valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("subject-conflict cancellation should remain save-valid"),
    )
    .expect("subject-conflict cancellation should restore");
    assert_eq!(
        restored
            .legal()
            .get_investigation_work(work)
            .and_then(|record| record.cancellation())
            .map(|cancellation| cancellation.reason()),
        Some(InvestigationWorkCancellationReason::WitnessBecameCaseSubject(promotion))
    );
    validate_invariants(&restored);
}

#[test]
fn direct_statement_cancels_redundant_pending_interview_without_cancelling_its_case() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Weak,
        EvidenceReliability::Mixed,
        Admissibility::Unknown,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.target),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("witness should register")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect("interview should schedule")
    .commit(&mut fixture.state)
    .expect("interview should commit");

    let statement = crate::legal::witness_system::validate_record_witness_statement(
        &registry,
        &fixture.state,
        crate::legal::WitnessStatementDraft {
            case_witness,
            origin: None,
            confidence: rating(80),
            summary: "The witness voluntarily gave the account before the appointment.".to_owned(),
        },
    )
    .expect("direct statement should validate while an interview is pending")
    .commit(&mut fixture.state)
    .expect("direct statement should cancel the now-redundant interview");

    let work_record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("cancelled interview should remain historical");
    assert_eq!(work_record.status(), InvestigationWorkStatus::Cancelled);
    assert_eq!(
        work_record
            .cancellation()
            .expect("cancelled interview should keep provenance")
            .reason(),
        InvestigationWorkCancellationReason::WitnessStatementRecorded(statement.statement)
    );
    assert!(
        fixture
            .state
            .legal()
            .find_investigation_work_due_at_or_before(SimTime::from_minutes(u64::MAX))
            .iter()
            .all(|candidate| *candidate != work),
        "cancelled interview must leave the due-work index immediately"
    );
    validate_state(&fixture.state).expect("redundant interview cancellation should be valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("redundant interview cancellation should remain registry-valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("statement cancellation provenance should remain save-valid"),
    )
    .expect("statement cancellation provenance should restore");
    assert_eq!(
        restored
            .legal()
            .get_investigation_work(work)
            .and_then(|record| record.cancellation())
            .map(|cancellation| cancellation.reason()),
        Some(InvestigationWorkCancellationReason::WitnessStatementRecorded(statement.statement))
    );
    validate_invariants(&restored);
}

#[test]
fn late_reviewable_evidence_is_scheduled_after_case_was_already_staffed() {
    let registry = build_registry();
    let mut state = AppState::new(0x1A7E_1A7E);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Late Evidence Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Late Evidence Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let investigator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detective Later".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("investigator fixture should validate");
    let suspect = insert_character(
        &mut state,
        CharacterDraft {
            name: "Late Evidence Suspect".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("suspect fixture should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Early Witness".to_owned(),
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
            title: "Evidence arrives later".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(suspect)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    crate::legal::witness_system::validate_register_case_witness(
        &state,
        crate::legal::CaseWitnessDraft {
            investigation,
            witness,
            subject: EntityRef::Character(suspect),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("witness registration should validate")
    .commit(&mut state)
    .expect("witness registration should commit");

    // The first authoritative minute staffs the case and schedules its witness interview, but
    // there is no reviewable evidence yet. This is exactly the state that used to strand a later
    // fingerprint because initial reviews only inspected the `staffed_investigations` output.
    let first_tick = run_tick(&registry, &mut state);
    assert_eq!(
        first_tick.staffed_investigations,
        vec![(investigation, investigator)]
    );
    assert!(first_tick.scheduled_investigation_work.is_empty());
    assert_eq!(first_tick.scheduled_witness_interviews.len(), 1);
    let interview = first_tick.scheduled_witness_interviews[0];

    let fingerprint = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Fingerprint,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("late fingerprint should validate")
    .commit(&mut state)
    .expect("late fingerprint should commit");

    let second_tick = run_tick(&registry, &mut state);
    assert!(
        second_tick.staffed_investigations.is_empty(),
        "the case was already staffed before the evidence arrived"
    );
    assert!(
        second_tick.scheduled_investigation_work.is_empty(),
        "late evidence must wait while the case's single detective is interviewing a witness"
    );
    assert!(second_tick.scheduled_witness_interviews.is_empty());

    if !second_tick.resolved_investigation_work.contains(&interview) {
        run_until_work_resolved(&registry, &mut state, interview);
    }

    // Scheduling phases run before due work resolves, so the newly idle detective picks up the
    // deferred evidence review on the following authoritative minute. Evidence review has phase
    // priority over witness scheduling and therefore owns the detective for its authored duration.
    let review_tick = run_tick(&registry, &mut state);
    assert_eq!(review_tick.scheduled_investigation_work.len(), 1);
    assert!(review_tick.scheduled_witness_interviews.is_empty());
    let review = state
        .legal()
        .get_investigation_work(review_tick.scheduled_investigation_work[0])
        .expect("late-evidence review should persist");
    assert_eq!(review.kind(), InvestigationWorkKind::EvidenceReview);
    assert_eq!(
        review.focus(),
        InvestigationWorkFocus::evidence(fingerprint)
    );
    assert_eq!(
        state
            .legal()
            .work_for_investigation(investigation)
            .filter(|work| work.kind() == InvestigationWorkKind::WitnessInterview)
            .count(),
        1,
        "the completed interview remains history while the deferred review starts"
    );
    validate_state(&state).expect("late evidence scheduling state should validate");
    validate_state_against_registry(&registry, &state)
        .expect("late evidence scheduling should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn later_witness_pressure_does_not_rewrite_completed_interview_support() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("cooperative witness should register")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let work = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect("witness interview should schedule")
    .commit(&mut fixture.state)
    .expect("witness interview should commit");
    let duration = registry
        .get_investigation_work(InvestigationWorkKind::WitnessInterview)
        .duration();
    fixture.state.advance_clock(duration);
    let plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        work,
        InvestigationWorkRandomness::new(0),
    )
    .expect("due cooperative interview should resolve");
    assert_eq!(plan.outcome(), InvestigationWorkOutcome::Connected);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
        .expect("fresh interview resolution should validate")
        .commit(&mut fixture.state)
        .expect("interview resolution should commit");
    let historical_support = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .and_then(|record| record.resolution())
        .expect("completed interview should retain its resolution")
        .factors()
        .source_support();
    assert_eq!(historical_support.value(), 85);

    crate::legal::witness_system::validate_set_witness_cooperation(
        &fixture.state,
        case_witness,
        crate::legal::WitnessCooperation::Hostile,
    )
    .expect("later witness pressure should be able to change cooperation")
    .commit(&mut fixture.state)
    .expect("later cooperation change should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_case_witness(case_witness)
            .expect("witness should persist")
            .cooperation(),
        crate::legal::WitnessCooperation::Hostile
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(work)
            .and_then(|record| record.resolution())
            .expect("historical interview should persist")
            .factors()
            .source_support(),
        historical_support,
        "later intimidation must affect future interviews, not the completed interview"
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("later cooperation changes must not retroactively invalidate completed work");
    build_save(&registry, &fixture.state)
        .expect("completed interview followed by later pressure should remain save-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn witness_scheduler_surfaces_work_id_exhaustion_without_partial_schedule() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        80,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness: fixture.witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("witness should register")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let work_before = fixture
        .state
        .legal()
        .work_for_investigation(fixture.investigation)
        .count();
    let investigation_before = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .clone();
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::InvestigationWork, u32::MAX);

    let error = apply_witness_interview_scheduling(&registry, &mut fixture.state)
        .expect_err("autonomous witness scheduling must surface allocator exhaustion");
    assert!(matches!(error, InvestigationWorkError::IdExhaustion(_)));
    assert_eq!(
        fixture
            .state
            .legal()
            .work_for_investigation(fixture.investigation)
            .count(),
        work_before,
        "failed scheduling must not insert a partial work record"
    );
    assert!(
        fixture
            .state
            .legal()
            .scheduled_work_for_focus(
                fixture.investigation,
                InvestigationWorkKind::WitnessInterview,
                InvestigationWorkFocus::witness(case_witness),
            )
            .is_none()
    );
    let investigation_after = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist");
    assert_eq!(
        investigation_after.version(),
        investigation_before.version()
    );
    assert_eq!(
        investigation_after.last_activity_at(),
        investigation_before.last_activity_at()
    );
    validate_state(&fixture.state).expect("failed witness scheduling must leave valid state");
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
        assert!(
            run_tick(&registry, &mut fixture.state)
                .resolved_investigation_work
                .is_empty()
        );
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
fn scheduling_is_versioned_and_deduplicated_per_focus() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
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
fn investigator_cannot_hold_two_scheduled_casework_tasks() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let witness = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Capacity Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness fixture should validate");
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &fixture.state,
        crate::legal::CaseWitnessDraft {
            investigation: fixture.investigation,
            witness,
            subject: EntityRef::Character(fixture.first),
            cooperation: crate::legal::WitnessCooperation::Cooperative,
        },
    )
    .expect("witness registration should validate")
    .commit(&mut fixture.state)
    .expect("witness registration should commit");
    let review = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        review_draft(&fixture, fixture.first_evidence),
    )
    .expect("first detective task should validate")
    .commit(&mut fixture.state)
    .expect("first detective task should schedule");

    let error = validate_schedule_investigation_work(
        &registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::WitnessInterview,
            focus: InvestigationWorkFocus::witness(case_witness),
        },
    )
    .expect_err("one detective cannot start overlapping authored-duration work");
    assert_eq!(
        error,
        InvestigationWorkError::InvestigatorBusy {
            investigator: fixture.investigator,
            work: review,
        }
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .work_for_investigator(fixture.investigator)
            .filter(|work| work.status() == InvestigationWorkStatus::Scheduled)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("busy-investigator rejection should preserve state");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_witness_scheduler_starts_only_one_interview_per_detective() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        85,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    for name in ["First Capacity Witness", "Second Capacity Witness"] {
        let witness = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: name.to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("witness fixture should validate");
        crate::legal::witness_system::validate_register_case_witness(
            &fixture.state,
            crate::legal::CaseWitnessDraft {
                investigation: fixture.investigation,
                witness,
                subject: EntityRef::Character(fixture.first),
                cooperation: crate::legal::WitnessCooperation::Cooperative,
            },
        )
        .expect("witness registration should validate")
        .commit(&mut fixture.state)
        .expect("witness registration should commit");
    }

    let scheduled = apply_witness_interview_scheduling(&registry, &mut fixture.state)
        .expect("autonomous witness scheduling should resolve");
    assert_eq!(scheduled.len(), 1);
    assert!(
        apply_witness_interview_scheduling(&registry, &mut fixture.state)
            .expect("busy detective should be a normal scheduling deferral")
            .is_empty()
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .work_for_investigator(fixture.investigator)
            .filter(|work| work.status() == InvestigationWorkStatus::Scheduled)
            .count(),
        1,
        "a single detective must not interview multiple witnesses in parallel"
    );
    validate_state(&fixture.state).expect("serialized witness scheduling should validate");
    validate_invariants(&fixture.state);
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
    validate_assign_investigator(&restored, second_investigation, fixture.second_investigator)
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
