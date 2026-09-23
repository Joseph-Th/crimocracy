//! Release-safe structural validation for operation plans and runtime lifecycles.

use super::discoveries::validate_operation_discoveries;
use super::*;

#[derive(Default)]
struct OperationInvariantContext {
    after_action_information: BTreeSet<InformationId>,
    discovered_information: BTreeSet<InformationId>,
    after_action_reports: BTreeSet<crate::core::id::ReportId>,
    history_events: BTreeSet<crate::core::id::HistoryEventId>,
    dispositions: dispositions::DispositionInvariantContext,
    discovery_signatures: BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)>,
}

pub(super) fn validate_operations(state: &AppState) -> Result<(), StateValidationError> {
    let mut context = OperationInvariantContext::default();
    for operation in state.operations.operations() {
        validate_operation(state, operation, &mut context)?;
    }
    Ok(())
}

fn validate_operation(
    state: &AppState,
    operation: &OperationRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    validate_operation_definition(state, operation)?;
    validate_operation_actors(state, operation)?;
    validate_operation_plan(state, operation)?;
    validate_common_runtime_links(state, operation)?;
    match operation.status() {
        OperationStatus::Authorized => validate_authorized_operation(operation),
        OperationStatus::InProgress => validate_in_progress_operation(state, operation),
        OperationStatus::AwaitingDecision => validate_awaiting_operation(state, operation),
        OperationStatus::Completed => validate_completed_operation(state, operation, context),
        OperationStatus::Aborted => validate_aborted_operation(state, operation, context),
    }
}

fn validate_operation_definition(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let Some(earliest_start) = try_resolve_operation_earliest_start(operation) else {
        return Err(StateValidationError::InvalidOperationDefinition {
            operation: operation.id(),
        });
    };
    if operation.title().trim().is_empty()
        || operation.version() == 0
        || operation.authorized_at() > operation.scheduled_for()
        || operation.authorized_at() > state.now()
        || operation
            .started_at()
            .is_some_and(|started_at| started_at < earliest_start)
    {
        return Err(StateValidationError::InvalidOperationDefinition {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_operation_actors(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let leader = state.world.get_character(operation.leader()).ok_or(
        StateValidationError::MissingEntity {
            context: "operation leader",
            entity: EntityRef::Character(operation.leader()),
        },
    )?;
    let requires_active_membership = matches!(
        operation.status(),
        OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision
    );
    if requires_active_membership
        && let Some(arrest) = state.legal.active_arrest_for_character(leader.id())
    {
        return Err(StateValidationError::ActiveOperationDetainedParticipant {
            operation: operation.id(),
            participant: leader.id(),
            arrest: arrest.id(),
        });
    }
    let mut role_participants = BTreeSet::new();
    for participant in operation.roles().values() {
        if !role_participants.insert(*participant) {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
        let participant_record =
            state
                .world
                .get_character(*participant)
                .ok_or(StateValidationError::MissingEntity {
                    context: "operation participant",
                    entity: EntityRef::Character(*participant),
                })?;
        if requires_active_membership
            && participant_record.organization() != Some(operation.responsible_organization())
        {
            return Err(StateValidationError::ActiveOperationForeignParticipant {
                operation: operation.id(),
                participant: *participant,
            });
        }
        if requires_active_membership
            && let Some(arrest) = state.legal.active_arrest_for_character(*participant)
        {
            return Err(StateValidationError::ActiveOperationDetainedParticipant {
                operation: operation.id(),
                participant: *participant,
                arrest: arrest.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_plan(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    validate_operation_intelligence(state, operation)?;
    validate_operation_objective(state, operation)?;
    validate_operation_constraints(state, operation)?;
    validate_operation_contingencies(operation);
    Ok(())
}

fn validate_operation_intelligence(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    for information in operation.intelligence() {
        let record = state.intelligence.get_information(*information).ok_or(
            StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            },
        )?;
        if record.holder() != KnowledgeHolder::Organization(operation.responsible_organization())
            || record.recorded_at() > operation.authorized_at()
            || !is_information_subject_relevant(state, operation.objective(), record.subject())
        {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_objective(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    for entity in operation.objective().referenced_entities() {
        if !is_entity_present(state, entity) {
            return Err(StateValidationError::MissingEntity {
                context: "operation objective",
                entity,
            });
        }
    }
    if !is_valid_operation_objective(operation.kind(), operation.objective()) {
        return Err(StateValidationError::InvalidOperationDefinition {
            operation: operation.id(),
        });
    }
    match operation.objective() {
        OperationObjective::FreeDetainee { target } => {
            if !operation.witness_pressure_cases().is_empty() {
                return Err(invalid_operation_definition(operation));
            }
            let arrest_id = operation
                .extraction_arrest()
                .ok_or_else(|| invalid_operation_definition(operation))?;
            let arrest = state
                .legal
                .get_arrest(arrest_id)
                .ok_or_else(|| invalid_operation_definition(operation))?;
            if arrest.character() != *target
                || arrest.arrested_at() > operation.authorized_at()
                || arrest
                    .released_at()
                    .is_some_and(|released_at| released_at < operation.authorized_at())
            {
                return Err(invalid_operation_definition(operation));
            }
        }
        OperationObjective::Frighten {
            target: EntityRef::Character(character),
        } if operation.kind() == OperationKind::WitnessPressure => {
            if operation.extraction_arrest().is_some()
                || operation.witness_pressure_cases().is_empty()
                || operation
                    .witness_pressure_cases()
                    .iter()
                    .any(|case_witness| {
                        state
                            .legal
                            .get_case_witness(*case_witness)
                            .is_none_or(|witness| witness.witness() != *character)
                    })
            {
                return Err(invalid_operation_definition(operation));
            }
        }
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => {
            if operation.extraction_arrest().is_some()
                || !operation.witness_pressure_cases().is_empty()
            {
                return Err(invalid_operation_definition(operation));
            }
        }
    }
    Ok(())
}

fn validate_operation_constraints(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    for constraint in operation.constraints() {
        match constraint {
            OperationConstraint::CompleteBy(deadline) => {
                if operation.scheduled_for() >= *deadline {
                    return Err(invalid_runtime(operation));
                }
            }
            OperationConstraint::RequireIntelligenceTopic(topic) => {
                let covered = operation.intelligence().iter().any(|information| {
                    state
                        .intelligence
                        .get_information(*information)
                        .is_some_and(|record| record.topic() == *topic)
                });
                if !covered {
                    return Err(invalid_runtime(operation));
                }
            }
        }
    }
    Ok(())
}

fn validate_operation_contingencies(operation: &OperationRecord) {
    // Exhaustiveness canary: a new contingency must be classified before persisted operations
    // can pass restore validation.
    for contingency in operation.contingencies() {
        match contingency {
            OperationContingency::AbortOnPoliceArrivalBeforeEntry
            | OperationContingency::RequestDecisionOnPoliceArrival => {}
        }
    }
}

fn validate_common_runtime_links(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    // The current operation vocabulary has one bounded active lifecycle:
    // authorize(v1) -> begin(v2) -> optional police-decision pause(v3) -> continue(v4), followed
    // by one terminal status advance. Completed work may then advance once more for its single
    // property/cash disposition. Rejecting versions outside this envelope at restore keeps an
    // impossible high-version active record from entering a due queue it can never leave.
    let continued_police_decisions = state
        .decisions
        .decisions_for_operation(operation.id())
        .filter(|decision| {
            matches!(
                decision.context(),
                DecisionContext::OperationPoliceArrival { .. }
            ) && decision.status() == DecisionStatus::Resolved
                && decision
                    .resolution()
                    .is_some_and(|resolution| resolution.response() == DecisionResponse::Continue)
        })
        .count();
    let version_matches_status = match operation.status() {
        OperationStatus::Authorized => operation.version() == 1,
        OperationStatus::InProgress => match continued_police_decisions {
            0 => operation.version() == 2,
            1 => operation.version() == 4,
            _ => false,
        },
        OperationStatus::AwaitingDecision => {
            operation.version() == 3
                && continued_police_decisions == 0
                && state
                    .decisions
                    .pending_for_operation(operation.id())
                    .is_some()
        }
        OperationStatus::Completed => {
            let disposition_count = u32::from(operation.property_disposition().is_some())
                + u32::from(operation.cash_disposition().is_some());
            disposition_count <= 1
                && match continued_police_decisions {
                    0 => operation.version() == 3 + disposition_count,
                    1 => operation.version() == 5 + disposition_count,
                    _ => false,
                }
        }
        OperationStatus::Aborted => (2..=5).contains(&operation.version()),
    };
    if !version_matches_status {
        return Err(invalid_runtime(operation));
    }
    if operation.entry_at().is_some_and(|entry_at| {
        operation
            .started_at()
            .is_none_or(|started_at| entry_at <= started_at)
    }) {
        return Err(invalid_runtime(operation));
    }
    if let Some(response_id) = operation.police_response()
        && state
            .legal
            .get_police_response(response_id)
            .is_none_or(|response| response.source_operation() != operation.id())
    {
        return Err(invalid_runtime(operation));
    }
    if matches!(
        operation.status(),
        OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision
    ) && state
        .world
        .get_character(operation.leader())
        .is_none_or(|leader| leader.organization() != Some(operation.responsible_organization()))
    {
        return Err(StateValidationError::ActiveOperationInvalidLeader {
            operation: operation.id(),
        });
    }
    if operation.status() != OperationStatus::Completed
        && (operation.property_disposition().is_some() || operation.cash_disposition().is_some())
    {
        return Err(StateValidationError::InvalidOperationPropertyDisposition {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_authorized_operation(operation: &OperationRecord) -> Result<(), StateValidationError> {
    if operation.started_at().is_some()
        || operation.resolution_due_at().is_some()
        || operation.entry_at().is_some()
        || operation.police_response().is_some()
        || operation.awaiting_decision_since().is_some()
        || operation.resolution().is_some()
        || operation.abort_record().is_some()
    {
        return Err(invalid_runtime(operation));
    }
    Ok(())
}

fn validate_in_progress_operation(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let (Some(started_at), Some(due_at)) = (operation.started_at(), operation.resolution_due_at())
    else {
        return Err(invalid_runtime(operation));
    };
    if started_at > due_at
        || started_at > state.now()
        || operation.awaiting_decision_since().is_some()
        || operation.resolution().is_some()
        || operation.abort_record().is_some()
    {
        return Err(invalid_runtime(operation));
    }
    Ok(())
}

fn validate_awaiting_operation(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let (Some(started_at), Some(due_at), Some(paused_at)) = (
        operation.started_at(),
        operation.resolution_due_at(),
        operation.awaiting_decision_since(),
    ) else {
        return Err(invalid_runtime(operation));
    };
    if started_at > due_at
        || started_at > paused_at
        || paused_at > state.now()
        || operation.resolution().is_some()
        || operation.abort_record().is_some()
    {
        return Err(invalid_runtime(operation));
    }
    Ok(())
}

fn validate_completed_operation(
    state: &AppState,
    operation: &OperationRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    let (Some(started_at), Some(due_at), Some(resolution)) = (
        operation.started_at(),
        operation.resolution_due_at(),
        operation.resolution(),
    ) else {
        return Err(invalid_runtime(operation));
    };
    if started_at > due_at
        || resolution.resolved_at() < due_at
        || resolution.resolved_at() > state.now()
        || operation.awaiting_decision_since().is_some()
        || operation.abort_record().is_some()
    {
        return Err(invalid_runtime(operation));
    }
    validate_operation_proceeds(operation, resolution)?;
    dispositions::validate_operation_cash_disposition(
        state,
        operation,
        resolution,
        &mut context.dispositions,
    )?;
    dispositions::validate_operation_property_disposition(
        state,
        operation,
        resolution,
        &mut context.dispositions,
    )?;
    validate_completion_after_action(state, operation, resolution, context)?;
    validate_participant_after_action_information(
        state,
        operation,
        resolution,
        &mut context.after_action_information,
    )?;
    validate_completion_history(state, operation, resolution, context)?;
    validate_operation_discoveries(
        state,
        operation,
        resolution,
        &mut context.discovered_information,
        &mut context.discovery_signatures,
    )?;
    exposure::validate_operation_exposure_links(state, operation, resolution)
}

fn validate_participant_after_action_information(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    seen: &mut BTreeSet<InformationId>,
) -> Result<(), StateValidationError> {
    let participants = operation.participants();
    if resolution.participant_information().len() != participants.len()
        || !participants.iter().all(|participant| {
            resolution
                .participant_information()
                .contains_key(participant)
        })
    {
        return Err(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        });
    }
    let expected_summary =
        participant_after_action_summary(operation, resolution.objective_outcome());
    for participant in participants {
        let information_id = *resolution
            .participant_information()
            .get(&participant)
            .expect("participant map shape was validated above");
        let Some(information) = state.intelligence.get_information(information_id) else {
            return Err(StateValidationError::InvalidOperationAfterAction {
                operation: operation.id(),
            });
        };
        if !seen.insert(information_id)
            || information.holder() != KnowledgeHolder::Character(participant)
            || information.source_kind() != InformationSourceKind::AfterAction
            || information.topic() != InformationTopic::OperationalOutcome
            || information.source_entity() != Some(EntityRef::Character(operation.leader()))
            || information.subject() != EntityRef::Operation(operation.id())
            || information.observed_at() != resolution.resolved_at()
            || information.recorded_at() != resolution.resolved_at()
            || information.reliability() != Reliability::DirectAccess
            || information.specificity() != Specificity::Precise
            || information.signal().is_some()
            || !information.derived_from().is_empty()
            || information.summary() != expected_summary
        {
            return Err(StateValidationError::InvalidOperationAfterAction {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_proceeds(
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    if let Some(proceeds) = resolution.property_proceeds() {
        let valid_target = matches!(
            operation.objective(),
            OperationObjective::AcquireProperty { target } if *target == proceeds.target()
        );
        if !valid_target
            || resolution.objective_outcome() == OperationObjectiveOutcome::Failed
            || proceeds.estimated_value().cents() <= 0
        {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
    }
    if let Some(proceeds) = resolution.cash_proceeds() {
        let valid_target = matches!(
            operation.objective(),
            OperationObjective::ObtainCash { target } if *target == proceeds.target()
        );
        if !valid_target
            || resolution.objective_outcome() == OperationObjectiveOutcome::Failed
            || proceeds.amount().cents() <= 0
        {
            return Err(StateValidationError::InvalidOperationCashProceeds {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_completion_after_action(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    let expected_entities = resolve_completion_history_entities(
        state,
        operation,
        resolution.factors().police_response_arrived(),
    );
    let information = state
        .intelligence
        .get_information(resolution.after_action_information())
        .ok_or(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        })?;
    if !context
        .after_action_information
        .insert(resolution.after_action_information())
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::OperationalOutcome
        || information.source_entity() != Some(EntityRef::Character(operation.leader()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != resolution.resolved_at()
        || information.recorded_at() != resolution.resolved_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || information.signal().is_some()
        || !information.derived_from().is_empty()
    {
        return Err(StateValidationError::InvalidOperationAfterAction {
            operation: operation.id(),
        });
    }
    let report = state
        .reports
        .get_report(resolution.after_action_report())
        .ok_or(StateValidationError::InvalidOperationAfterActionReport {
            operation: operation.id(),
        })?;
    let report_entry = report.entries().first();
    if !context.after_action_reports.insert(report.id())
        || report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::AfterAction
        || !is_after_action_title(report.title(), operation.title())
        || report.generated_at() != resolution.resolved_at()
        || report.entries().len() != 1
        || !report_entry.is_some_and(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == information.summary()
                && entry.sources.is_empty()
                && entry.decision.is_none()
                && entry.entities == expected_entities
        })
    {
        return Err(StateValidationError::InvalidOperationAfterActionReport {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_completion_history(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    let expected_entities = resolve_completion_history_entities(
        state,
        operation,
        resolution.factors().police_response_arrived(),
    );
    let expected_summary = completion_history_summary(operation, resolution.objective_outcome());
    let valid = state
        .history
        .get_event(resolution.history_event())
        .is_some_and(|event| {
            context.history_events.insert(event.id())
                && event.kind() == HistoryEventKind::Operation
                && event.occurred_at() == resolution.resolved_at()
                && event.summary() == expected_summary
                && event.entities() == &expected_entities
        });
    if !valid {
        return Err(StateValidationError::InvalidOperationHistory {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_aborted_operation(
    state: &AppState,
    operation: &OperationRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    let abort = operation
        .abort_record()
        .ok_or(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        })?;
    let pause_shape_valid = match abort.phase() {
        OperationAbortPhase::AwaitingDecision => operation
            .awaiting_decision_since()
            .is_some_and(|paused_at| paused_at <= abort.aborted_at()),
        OperationAbortPhase::BeforeStart | OperationAbortPhase::InProgress => {
            operation.awaiting_decision_since().is_none()
        }
    };
    if !pause_shape_valid || operation.resolution().is_some() {
        return Err(invalid_runtime(operation));
    }
    aborts::validate_operation_abort_links(
        state,
        operation,
        abort,
        &mut context.after_action_information,
        &mut context.after_action_reports,
        &mut context.history_events,
    )
}

fn invalid_runtime(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationRuntime {
        operation: operation.id(),
    }
}
