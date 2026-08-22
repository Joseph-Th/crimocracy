//! Focused tests for operation authorization, transitions, aborts, and indexes.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::MandateId;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::time::SimTime;
use crate::decisions::decision_system::validate_request_decision;
use crate::decisions::{DecisionContext, DecisionRequestDraft, OperationExceptionReason};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::{
    OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
};
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
    validate_reassign_character,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn make_test_operation_state() -> (Registry, AppState, OrganizationId, CharacterId, EntityRef) {
    let registry = build_registry();
    let mut state = AppState::new(19);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let leader = insert_character(
        &registry,
        &mut state,
        CharacterDraft {
            name: "Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Test Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                    political_influence: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    social_cohesion: Rating::try_new(50).expect("fixture rating should validate"),
                    visible_violence_tolerance: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Test Business".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business fixture should validate");
    (
        registry,
        state,
        organization,
        leader,
        EntityRef::Business(business),
    )
}

fn make_test_draft(
    organization: OrganizationId,
    leader: CharacterId,
    target: EntityRef,
) -> OperationDraft {
    OperationDraft {
        title: "Test intimidation racket".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash { target },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    }
}

#[test]
fn invalid_terminal_transition_leaves_operation_unchanged() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("validated operation should remain current");

    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("authorized operation should begin");
    apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
        .expect("in-progress operation should abort");
    let before = state
        .operations
        .get_operation(operation)
        .expect("operation should exist")
        .version();

    let error = apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
        .expect_err("terminal operation must reject further transitions");
    assert_eq!(
        error,
        OperationError::InvalidTransition {
            status: OperationStatus::Aborted,
            transition: OperationTransition::Abort,
        }
    );
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation should still exist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(record.version(), before);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_foreign_crew_members_before_authorization() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut state = state;
    let foreign_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Foreign Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("foreign organization fixture should validate");
    let foreign_member = insert_character(
        &registry,
        &mut state,
        CharacterDraft {
            name: "Foreign Member".to_owned(),
            organization: Some(foreign_organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("foreign member fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    draft.roles.insert(RoleKind::Coordinator, foreign_member);

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("foreign crew members must not be authorized");
    assert_eq!(
        error,
        OperationError::ForeignParticipant {
            character: foreign_member,
            expected: organization,
            actual: Some(foreign_organization),
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_one_character_filling_multiple_roles() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Impossible double assignment".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash { target },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Lookout, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("one character must not fill multiple simultaneous roles");
    assert_eq!(
        error,
        OperationError::DuplicateRoleParticipant {
            character: leader,
            first_role: RoleKind::Lookout,
            second_role: RoleKind::Coordinator,
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_administrative_records_as_field_objective_targets() {
    let (registry, state, organization, leader, _) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Invalid mandate coercion".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash {
            target: EntityRef::Mandate(MandateId::from_raw(1)),
        },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("field operations must not target internal mandate records");
    assert_eq!(
        error,
        OperationError::InvalidObjectiveTarget {
            objective: OperationObjectiveKind::ObtainCash,
            target: EntityRef::Mandate(MandateId::from_raw(1)),
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_overlapping_participant_assignment_until_prior_operation_is_terminal() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");

    let error = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect_err("one person must not be scheduled for overlapping operations");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first,
        }
    );
    assert_eq!(state.operations().operations().count(), 1);

    validate_authority_abort_operation(&state, first)
        .expect("the first operation should be cancellable before start")
        .commit(&mut state)
        .expect("the cancellation should commit");
    validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("terminal operations must release their participants");
    validate_invariants(&state);
}

#[test]
fn operation_allows_non_overlapping_future_assignment() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");
    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = state
        .operations()
        .get_operation(first)
        .expect("first operation should persist")
        .scheduled_for()
        + registry
            .get_operation(OperationKind::Intimidation)
            .execution()
            .duration();

    validate_authorize_operation(&registry, &state, later)
        .expect("a future operation after the prior window should validate");
    validate_invariants(&state);
}

#[test]
fn operation_commit_rechecks_participant_availability_after_validation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate");
    let second = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("the second plan can validate against the same initial snapshot");
    let first_id = first
        .commit(&mut state)
        .expect("the first plan should commit");

    let error = second
        .commit(&mut state)
        .expect_err("commit must recheck a participant reservation created after validation");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first_id,
        }
    );
    assert_eq!(state.operations().operations().count(), 1);
    validate_invariants(&state);
}

#[test]
fn property_acquisition_requires_a_business_target() {
    let (registry, state, organization, leader, _) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Invalid property seizure".to_owned(),
        kind: OperationKind::Burglary,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::AcquireProperty {
            target: EntityRef::Character(leader),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::EntrySpecialist, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("property acquisition must identify a business target");
    assert_eq!(
        error,
        OperationError::InvalidPropertyTarget(EntityRef::Character(leader))
    );
}

#[test]
fn expired_planning_information_is_not_reported_as_covered() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: target,
            observed_at: SimTime::ZERO,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "Expired personnel observation".to_owned(),
        },
    )
    .expect("information fixture should validate")
    .commit(&mut state)
    .expect("information fixture should commit");
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(10_081);
    draft.intelligence.insert(information);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("operation with stale but structurally valid information should authorize")
        .commit(&mut state)
        .expect("operation should commit");

    let (quality, adjustment, covered, relevant) =
        crate::operations::operation_execution::resolve_intelligence_factors(
            &registry, &state, operation,
        );
    assert_eq!(quality.value(), 0);
    assert_eq!(adjustment, 0);
    assert_eq!(covered, 0);
    assert_eq!(relevant, 3);
}

#[test]
fn pre_start_cancellation_records_cause_without_fabricating_execution_artifacts() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate")
        .commit(&mut state)
        .expect("future operation should commit");

    // A pre-start cancellation creates no information, report, or history record. Its
    // transaction must therefore remain usable even when an unrelated optional ID stream is
    // exhausted.
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Information, u32::MAX);

    validate_authority_abort_operation(&state, operation)
        .expect("authorized operation should accept a leadership cancellation")
        .commit(&mut state)
        .expect("validated pre-start cancellation should commit");

    let record = state
        .operations()
        .get_operation(operation)
        .expect("cancelled operation should persist");
    let abort = record
        .abort_record()
        .expect("cancelled operation should persist its abort record");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(abort.aborted_at(), SimTime::ZERO);
    assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
    assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
    assert!(abort.artifacts().is_none());
    assert!(record.started_at().is_none());
    assert!(record.resolution_due_at().is_none());
    assert_eq!(state.reports().reports_for(organization).count(), 0);
    validate_state(&state).expect("pre-start cancellation should be structurally valid");
    validate_invariants(&state);
}

#[test]
fn in_progress_authority_abort_records_causal_artifacts_and_survives_save_round_trip() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("operation fixture should commit");
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(5));

    validate_authority_abort_operation(&state, operation)
        .expect("in-progress operation should accept a leadership abort")
        .commit(&mut state)
        .expect("validated in-progress abort should commit");

    let record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should persist");
    let abort = record
        .abort_record()
        .expect("started abort should persist its provenance");
    assert_eq!(abort.aborted_at(), SimTime::from_minutes(5));
    assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
    assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
    assert!(record.resolution().is_none());
    let artifacts = abort
        .artifacts()
        .expect("started abort should produce after-action artifacts");
    let information = state
        .intelligence()
        .get_information(artifacts.information())
        .expect("abort information should persist");
    assert_eq!(
        information.holder(),
        KnowledgeHolder::Organization(organization)
    );
    assert_eq!(information.topic(), InformationTopic::OperationalOutcome);
    assert_eq!(information.subject(), EntityRef::Operation(operation));
    assert!(information.summary().contains("aborted by leadership"));
    let report = state
        .reports()
        .get_report(artifacts.report())
        .expect("abort report should persist");
    assert_eq!(report.kind(), ReportKind::AfterAction);
    assert_eq!(report.recipient(), organization);
    assert_eq!(report.entries().len(), 1);
    assert_eq!(report.entries()[0].summary, information.summary());
    let history = state
        .history()
        .get_event(artifacts.history_event())
        .expect("abort history should persist");
    assert_eq!(history.summary(), information.summary());

    let envelope = build_save(&registry, &state).expect("aborted operation should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("aborted operation should restore");
    let restored_abort = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.abort_record())
        .expect("restored abort provenance should persist");
    assert_eq!(restored_abort, abort);
    validate_state(&restored).expect("restored abort state should validate");
    validate_invariants(&restored);
}

#[test]
fn abort_token_rejects_time_staleness_without_partial_mutation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("operation fixture should commit");
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin");
    let abort =
        validate_authority_abort_operation(&state, operation).expect("fresh abort should validate");
    state.advance_clock(crate::core::time::SimDuration::ONE_MINUTE);

    let error = abort
        .commit(&mut state)
        .expect_err("abort token must expire when simulation time advances");
    assert_eq!(
        error,
        OperationError::StaleAbortTime {
            operation,
            expected: SimTime::ZERO,
            found: SimTime::from_minutes(1),
        }
    );
    let record = state
        .operations()
        .get_operation(operation)
        .expect("stale abort must leave operation present");
    assert_eq!(record.status(), OperationStatus::InProgress);
    assert!(record.abort_record().is_none());
    assert_eq!(state.reports().reports_for(organization).count(), 0);
    assert_eq!(
        state
            .history()
            .events_for(EntityRef::Operation(operation))
            .count(),
        0
    );
    validate_state(&state).expect("stale abort rejection must leave valid state");
    validate_invariants(&state);
}

#[test]
fn missing_required_role_is_rejected_before_id_allocation() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.roles.clear();

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("missing required role must fail validation");
    assert_eq!(
        error,
        OperationError::MissingRequiredRole(RoleKind::Coordinator)
    );
    assert_eq!(
        state
            .operations
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn operation_cannot_begin_before_scheduled_time() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate")
        .commit(&mut state)
        .expect("validated operation should remain current");
    let version = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .version();

    let error = apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect_err("operation must not begin early");
    assert_eq!(error, OperationError::StartBeforeScheduled(operation));
    let record = state
        .operations()
        .get_operation(operation)
        .expect("operation should still exist");
    assert_eq!(record.status(), OperationStatus::Authorized);
    assert_eq!(record.version(), version);
    validate_invariants(&state);
}

#[test]
fn missed_completion_deadline_aborts_before_start_with_visible_provenance() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBefore(
            SimTime::from_minutes(40),
        ));
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");

    state.advance_clock(crate::core::time::SimDuration::from_minutes(41));
    let outcome = crate::core::simulation::run_tick(&registry, &mut state);

    assert!(outcome.started_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-missed operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    let abort = record
        .abort_record()
        .expect("deadline miss should persist an abort record");
    assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
    assert_eq!(abort.cause(), OperationAbortCause::DeadlineMissed);
    let artifacts = abort
        .artifacts()
        .expect("deadline miss should produce visible provenance");
    let information = state
        .intelligence()
        .get_information(artifacts.information())
        .expect("deadline information should persist");
    assert!(information
        .summary()
        .contains("missed its completion deadline"));
    assert_eq!(state.reports().reports_for(organization).count(), 1);
    validate_state(&state).expect("deadline-missed operation should remain valid");
    validate_invariants(&state);

    let envelope = build_save(&registry, &state).expect("deadline miss should be saveable");
    let bytes = bincode::serialize(&envelope).expect("deadline save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("deadline save should deserialize");
    let restored = restore_save(&registry, decoded).expect("deadline save should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.abort_record())
            .map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
}

#[test]
fn in_progress_operation_cannot_resolve_at_or_after_completion_deadline() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBefore(
            SimTime::from_minutes(10),
        ));
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin before its deadline");

    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));
    let outcome = crate::core::simulation::run_tick(&registry, &mut state);

    assert!(outcome.resolved_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-aborted operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(
        record.abort_record().map(|abort| abort.phase()),
        Some(OperationAbortPhase::InProgress)
    );
    assert_eq!(
        record.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    let artifacts = record
        .abort_record()
        .and_then(|abort| abort.artifacts())
        .expect("an in-progress deadline miss should be visible");
    assert!(state
        .intelligence()
        .get_information(artifacts.information())
        .expect("deadline information should persist")
        .summary()
        .contains("before execution could complete"));
    validate_state(&state).expect("deadline abort should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn decision_paused_operation_auto_aborts_when_deadline_expires() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBefore(
            SimTime::from_minutes(10),
        ));
    draft
        .contingencies
        .push(crate::operations::OperationContingency::RequestDecisionOnUnexpectedCondition);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("decision-capable operation should validate")
        .commit(&mut state)
        .expect("decision-capable operation should commit");
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin before its deadline");
    let decision = validate_request_decision(
        &state,
        DecisionRequestDraft {
            requester: leader,
            context: DecisionContext::OperationException {
                operation,
                reason: OperationExceptionReason::UnexpectedCondition,
            },
            attention: AttentionClass::Exception,
            summary: "The crew encountered an unexpected condition.".to_owned(),
        },
    )
    .expect("operation exception should request a decision")
    .commit(&mut state)
    .expect("decision request should commit")
    .decision;
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("operation should persist")
            .status(),
        OperationStatus::AwaitingDecision
    );

    state.advance_clock(crate::core::time::SimDuration::from_minutes(10));
    crate::core::simulation::run_tick(&registry, &mut state);

    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-aborted operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(
        record.abort_record().map(|abort| abort.phase()),
        Some(OperationAbortPhase::AwaitingDecision)
    );
    assert_eq!(
        record.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    let request = state
        .decisions()
        .get_decision(decision)
        .expect("deadline decision should remain historical");
    assert_eq!(request.status(), crate::decisions::DecisionStatus::Resolved);
    assert_eq!(
        request.resolution().map(|resolution| resolution.response()),
        Some(crate::decisions::DecisionResponse::Abort)
    );
    assert!(state.decisions().pending_for_operation(operation).is_none());
    validate_state(&state).expect("deadline decision abort should remain valid");
    validate_invariants(&state);
}

#[test]
fn stale_authorization_cannot_commit_after_leader_reassignment() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let validated = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation should validate against the original hierarchy");

    validate_reassign_character(&state, leader, None, None)
        .expect("leader should be reassignable before the operation is committed")
        .commit(&mut state)
        .expect("reassignment should commit");

    let error = validated
        .commit(&mut state)
        .expect_err("stale authorization must not create an invalid operation");
    assert_eq!(
        error,
        OperationError::StaleParticipant {
            character: leader,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn authorization_expires_if_scheduled_time_passes_before_commit() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(2);
    let validated = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate");

    for _ in 0..3 {
        crate::core::simulation::run_tick(&registry, &mut state);
    }

    let error = validated
        .commit(&mut state)
        .expect_err("authorization must expire once its scheduled time is in the past");
    assert_eq!(
        error,
        OperationError::AuthorizationExpired {
            scheduled_for: 2,
            now: 3,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn operation_intelligence_must_be_owned_and_relevant() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let irrelevant = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary:
                "Detailed security information that does not answer an intimidation planning need."
                    .to_owned(),
        },
    )
    .expect("irrelevant information fixture should still be valid information")
    .commit(&mut state)
    .expect("irrelevant information fixture should commit");
    let unavailable = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(leader),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "The leader knows the target's personnel pattern personally.".to_owned(),
        },
    )
    .expect("character information fixture should validate")
    .commit(&mut state)
    .expect("character information fixture should commit");

    let mut draft = make_test_draft(organization, leader, target);
    draft.intelligence = BTreeSet::from([irrelevant]);
    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("authored operation should reject an irrelevant intelligence topic");
    assert_eq!(error, OperationError::IrrelevantInformation(irrelevant));

    let mut draft = make_test_draft(organization, leader, target);
    draft.intelligence = BTreeSet::from([unavailable]);
    let error = validate_authorize_operation(&registry, &state, draft).expect_err(
        "operation plan should reject intelligence not yet reported to the organization",
    );
    assert_eq!(
        error,
        OperationError::InformationUnavailable {
            information: unavailable,
            organization,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}
