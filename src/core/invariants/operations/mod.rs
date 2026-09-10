//! Release-safe structural validation for the operations subsystem.

mod aborts;
mod dispositions;

use super::opportunities::validate_operation_exposure_links;
use crate::core::attention::AttentionClass;
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::InformationId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_economics::{resolve_cash_proceeds, resolve_property_proceeds};
use crate::operations::operation_execution::write_legal_activity_summary;
use crate::operations::operation_execution::{
    has_police_response_arrived_by, resolve_execution_margin, resolve_exposure_level,
    resolve_exposure_score, resolve_intelligence_factors, resolve_objective_outcome,
};
use crate::operations::operation_objective::{
    blocker_matches_objective, effective_objective_outcome,
};
use crate::operations::operation_system::{
    is_information_subject_relevant, is_valid_operation_objective,
    resolve_deadline_without_execution_window, resolve_earliest_operation_deadline,
    resolve_operation_earliest_start, try_resolve_operation_earliest_start,
};
use crate::operations::police_response_integration::resolve_police_arrival_delay;
use crate::operations::property_disposition::resolve_property_liquidation_value;
use crate::operations::surveillance_integration::{
    is_supported_surveillance_target, is_valid_persisted_surveillance_information,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationBusinessTargetOwnership,
    OperationConstraint, OperationContingency, OperationKind, OperationObjective,
    OperationObjectiveBlocker, OperationObjectiveOutcome, OperationRecord, OperationStatus,
};
use crate::registry::{OperationDefinition, OperationExecutionDefinition, Registry};
use crate::reports::ReportKind;
use std::collections::BTreeSet;

/// The started/due instant pair every in-progress abort arm re-derives: an operation that
/// never truly began cannot carry an in-progress abort record.
fn resolve_abort_started_due(
    operation: &OperationRecord,
) -> Result<(SimTime, SimTime), StateValidationError> {
    let (Some(started_at), Some(due_at)) = (operation.started_at(), operation.resolution_due_at())
    else {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    };
    Ok((started_at, due_at))
}

pub(super) fn validate_operations_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for operation in state.operations.operations() {
        validate_operation_against_registry(registry, state, operation)?;
    }
    Ok(())
}

fn validate_operation_against_registry(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
) -> Result<(), StateValidationError> {
    let definition = registry.get_operation(operation.kind());
    let execution = definition.execution();
    validate_authored_operation_plan(state, operation, definition, execution)?;
    let Some(resolution) = operation.resolution() else {
        return Ok(());
    };
    let police_response_arrived =
        validate_authored_operation_resolution(registry, state, operation, execution, resolution)?;
    validate_authored_property_disposition(registry, state, operation, resolution)?;
    validate_authored_operation_exposure(
        state,
        operation,
        execution,
        resolution,
        police_response_arrived,
    )
}

fn validate_authored_operation_plan(
    state: &AppState,
    operation: &OperationRecord,
    definition: &OperationDefinition,
    execution: &OperationExecutionDefinition,
) -> Result<(), StateValidationError> {
    let has_police_entry_contingency = operation
        .contingencies()
        .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry);
    let police_response_matches_authorship = operation.police_response().is_none_or(|response| {
        state
            .legal
            .get_police_response(response)
            .is_some_and(|response| {
                let delay =
                    resolve_police_arrival_delay(execution, response.response_presence().value());
                response.alert_score() >= execution.police_dispatch_threshold()
                    && response
                        .dispatched_at()
                        .checked_add(crate::core::time::SimDuration::from_minutes(delay))
                        == Some(response.arrival_due_at())
            })
    });
    let deadline_window_is_valid = resolve_deadline_without_execution_window(
        execution,
        operation
            .started_at()
            .unwrap_or_else(|| resolve_operation_earliest_start(operation)),
        operation.constraints(),
    )
    .is_none();
    let authored_window_is_representable = operation
        .started_at()
        .unwrap_or_else(|| resolve_operation_earliest_start(operation))
        .as_minutes()
        .checked_add(u64::from(execution.duration().as_minutes()))
        .is_some();
    let before_start_deadline_abort_is_valid = operation.abort_record().is_none_or(|abort| {
        if abort.phase() != OperationAbortPhase::BeforeStart
            || abort.cause() != OperationAbortCause::DeadlineMissed
        {
            return true;
        }
        resolve_earliest_operation_deadline(operation).is_some_and(|deadline| {
            abort.aborted_at() >= deadline
                || resolve_deadline_without_execution_window(
                    execution,
                    abort.aborted_at(),
                    operation.constraints(),
                )
                .is_some()
        })
    });
    let business_target_is_valid = authored_business_target_is_valid(state, operation, execution);
    if !definition
        .supported_approaches()
        .contains(&operation.approach())
        || definition
            .required_roles()
            .iter()
            .any(|role| !operation.roles().contains_key(role))
        || operation
            .roles()
            .keys()
            .any(|role| execution.capability_for_role(*role).is_none())
        || operation.intelligence().iter().any(|information| {
            state
                .intelligence
                .get_information(*information)
                .is_none_or(|record| {
                    !execution
                        .relevant_intelligence_topics()
                        .contains(&record.topic())
                })
        })
        || (has_police_entry_contingency && execution.operation_entry_offset().is_none())
        || (execution.operation_entry_offset().is_none() && operation.entry_at().is_some())
        || (operation.started_at().is_some()
            && execution.operation_entry_offset().is_some()
            && operation.entry_at().is_none())
        || !authored_window_is_representable
        || !deadline_window_is_valid
        || !before_start_deadline_abort_is_valid
        || !police_response_matches_authorship
        || !business_target_is_valid
    {
        return Err(invalid_operation_definition(operation));
    }
    Ok(())
}

fn authored_business_target_is_valid(
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
) -> bool {
    let business = match operation.objective() {
        OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        }
        | OperationObjective::ObtainCash {
            target: EntityRef::Business(business),
        }
        | OperationObjective::DisruptBusiness {
            target: EntityRef::Business(business),
        } => *business,
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::FreeDetainee { .. }
        | OperationObjective::DisruptBusiness { .. } => {
            return execution.business_target().is_none();
        }
    };
    let Some(requirement) = execution.business_target() else {
        return false;
    };
    let Some(record) = state.world.get_business(business) else {
        return false;
    };
    if !requirement
        .required_functions()
        .iter()
        .all(|function| record.has_function(*function))
    {
        return false;
    }
    let (could_be_owned, definitely_owned) = state.world.business_owner_evidence_at(
        business,
        crate::world::BusinessOwner::Organization(operation.responsible_organization()),
        operation.authorized_at(),
    );
    match operation
        .kind()
        .business_target_ownership()
        .expect("business-target operation kind must define ownership semantics")
    {
        // Same-minute transfer ordering is not persisted. For a foreign target it is enough that
        // sponsor ownership was not certain for the whole timestamp; for a sponsor-hosted venue,
        // sponsor ownership must have been possible at some point in that timestamp.
        OperationBusinessTargetOwnership::Foreign => !definitely_owned,
        OperationBusinessTargetOwnership::SponsorOwned => could_be_owned,
    }
}

fn validate_authored_operation_resolution(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<bool, StateValidationError> {
    let factors = resolution.factors();
    let expected_margin = resolve_execution_margin(execution, factors);
    let base_expected_outcome = resolve_objective_outcome(execution, expected_margin);
    validate_resolution_objective_context(state, operation, resolution, base_expected_outcome)?;
    let expected_outcome =
        effective_objective_outcome(base_expected_outcome, resolution.objective_blocker());
    if matches!(
        operation.objective(),
        OperationObjective::FreeDetainee { .. }
    ) && expected_outcome != OperationObjectiveOutcome::Failed
    {
        let released_at = resolution
            .extraction_arrest()
            .and_then(|arrest| state.legal.get_arrest(arrest))
            .and_then(|arrest| arrest.released_at());
        if released_at != Some(resolution.resolved_at()) {
            return Err(invalid_operation_definition(operation));
        }
    }
    let (
        expected_intelligence_quality,
        expected_intelligence_adjustment,
        expected_intelligence_topics_covered,
        expected_intelligence_topics_relevant,
    ) = resolve_intelligence_factors(registry, state, operation.id());
    let expected_police_response_arrived =
        has_police_response_arrived_by(state, operation, resolution.resolved_at());
    let expected_property_proceeds =
        resolve_property_proceeds(registry, state, operation, resolution.objective_outcome())
            .map_err(|_| invalid_operation_definition(operation))?;
    let expected_cash_proceeds =
        resolve_cash_proceeds(registry, state, operation, resolution.objective_outcome()).map_err(
            |_| StateValidationError::InvalidOperationCashProceeds {
                operation: operation.id(),
            },
        )?;
    if factors.variance().unsigned_abs() > execution.variance_limit()
        || factors.time_pressure() > execution.max_time_pressure()
        || factors.approach_adjustment()
            != execution
                .approach_difficulty_adjustment(operation.approach())
                .expect("validated operation approach must have an execution adjustment")
        || factors.intelligence_quality() != expected_intelligence_quality
        || factors.intelligence_adjustment() != expected_intelligence_adjustment
        || factors.intelligence_topics_covered() != expected_intelligence_topics_covered
        || factors.intelligence_topics_relevant() != expected_intelligence_topics_relevant
        || factors.intelligence_topics_covered() > factors.intelligence_topics_relevant()
        || factors.police_response_arrived() != expected_police_response_arrived
        || resolution.execution_margin() != expected_margin
        || resolution.objective_outcome() != expected_outcome
        || resolution.property_proceeds() != expected_property_proceeds.proceeds
    {
        return Err(invalid_operation_definition(operation));
    }
    if resolution.cash_proceeds() != expected_cash_proceeds.proceeds {
        return Err(StateValidationError::InvalidOperationCashProceeds {
            operation: operation.id(),
        });
    }
    Ok(expected_police_response_arrived)
}

fn validate_resolution_objective_context(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    base_expected_outcome: OperationObjectiveOutcome,
) -> Result<(), StateValidationError> {
    let invalid = || invalid_operation_definition(operation);
    // Business ownership has append-only history, but different domains share minute-level
    // timestamps. An ownership transfer and an operation resolution with the same `SimTime` have
    // no persisted cross-domain ordering, so distinguish ownership that was possible at some
    // point during that timestamp from ownership that was true for every possible placement of
    // the resolution among the same-minute transfers.
    let sponsor_ownership_evidence = match operation.objective() {
        OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        }
        | OperationObjective::ObtainCash {
            target: EntityRef::Business(business),
        }
        | OperationObjective::DisruptBusiness {
            target: EntityRef::Business(business),
        } => Some(state.world.business_owner_evidence_at(
            *business,
            crate::world::BusinessOwner::Organization(operation.responsible_organization()),
            resolution.resolved_at(),
        )),
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::FreeDetainee { .. }
        | OperationObjective::DisruptBusiness { .. } => None,
    };
    if base_expected_outcome != OperationObjectiveOutcome::Failed
        && sponsor_ownership_evidence.is_some_and(|(could_be_owned, definitely_owned)| {
            match operation
                .kind()
                .business_target_ownership()
                .expect("persisted business-target operation kind must define ownership semantics")
            {
                OperationBusinessTargetOwnership::Foreign => definitely_owned,
                OperationBusinessTargetOwnership::SponsorOwned => !could_be_owned,
            }
        })
        && resolution.objective_blocker()
            != Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    {
        return Err(invalid());
    }
    if let Some(blocker) = resolution.objective_blocker() {
        if base_expected_outcome == OperationObjectiveOutcome::Failed
            || !blocker_matches_objective(operation, blocker)
        {
            return Err(invalid());
        }
        match blocker {
            OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
                let possible_mismatch = sponsor_ownership_evidence.is_some_and(
                    |(could_be_owned, definitely_owned)| {
                        match operation.kind().business_target_ownership().expect(
                            "persisted business-target operation kind must define ownership semantics",
                        ) {
                            OperationBusinessTargetOwnership::Foreign => could_be_owned,
                            OperationBusinessTargetOwnership::SponsorOwned => !definitely_owned,
                        }
                    },
                );
                if !possible_mismatch {
                    return Err(invalid());
                }
            }
            OperationObjectiveBlocker::ExtractionCustodyEnded => {
                let OperationObjective::FreeDetainee { target } = operation.objective() else {
                    return Err(invalid());
                };
                let arrest = operation
                    .extraction_arrest()
                    .and_then(|id| state.legal.get_arrest(id))
                    .ok_or_else(invalid)?;
                if arrest.character() != *target
                    || resolution.extraction_arrest().is_some()
                    || arrest
                        .released_at()
                        .is_none_or(|released_at| released_at > resolution.resolved_at())
                {
                    return Err(invalid());
                }
            }
            // Business operating status and witness cooperation/case activity are mutable domains
            // without complete historical timelines. Their blocker is the persisted validated
            // resolution snapshot; objective compatibility above is the release-safe proof.
            OperationObjectiveBlocker::TargetEconomyInactive
            | OperationObjectiveBlocker::NoPressureableWitnessCase => {}
        }
    }

    match operation.objective() {
        OperationObjective::FreeDetainee { target } => {
            if let Some(arrest_id) = resolution.extraction_arrest() {
                if operation.extraction_arrest() != Some(arrest_id) {
                    return Err(invalid());
                }
                let arrest = state.legal.get_arrest(arrest_id).ok_or_else(invalid)?;
                if arrest.character() != *target
                    || arrest.arrested_at() > resolution.resolved_at()
                    || arrest
                        .released_at()
                        .is_some_and(|released_at| released_at < resolution.resolved_at())
                {
                    return Err(invalid());
                }
            }
        }
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => {
            if resolution.extraction_arrest().is_some() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn validate_authored_property_disposition(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.property_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationPropertyDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.property_proceeds().ok_or_else(invalid)?;
    let expected_realized = resolve_property_liquidation_value(
        registry,
        state,
        operation.kind(),
        proceeds.estimated_value(),
        operation.id(),
        disposition.venue(),
    )
    .map_err(|_| invalid())?;
    if disposition.realized_value() != expected_realized {
        return Err(invalid());
    }
    Ok(())
}

fn validate_authored_operation_exposure(
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
    resolution: &crate::operations::OperationResolutionRecord,
    expected_police_response_arrived: bool,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    let exposure_factors = exposure.factors();
    let expected_intelligence_mitigation =
        u16::from(resolution.factors().intelligence_quality().value())
            .saturating_mul(u16::from(execution.intelligence_mitigation_weight()))
            / 100;
    let expected_exposure_score = resolve_exposure_score(execution, exposure_factors);
    let expected_exposure_level = resolve_exposure_level(execution, expected_exposure_score);
    if exposure_factors.variance().unsigned_abs() > execution.exposure_variance_limit()
        || exposure_factors.approach_adjustment()
            != execution
                .exposure_approach_adjustment(operation.approach())
                .expect("validated operation approach must have an exposure adjustment")
        || exposure_factors.intelligence_mitigation()
            != u8::try_from(expected_intelligence_mitigation)
                .expect("bounded exposure intelligence mitigation must fit u8")
        || exposure_factors.police_response_arrived() != expected_police_response_arrived
        || exposure.score() != expected_exposure_score
        || exposure.level() != expected_exposure_level
    {
        return Err(invalid_operation_exposure(operation));
    }
    if let Some(evidence_id) = exposure.evidence().iter().next() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or_else(|| invalid_operation_exposure(operation))?;
        if evidence.kind() != execution.exposure_evidence_kind() {
            return Err(invalid_operation_exposure(operation));
        }
    }
    Ok(())
}

fn invalid_operation_definition(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationDefinition {
        operation: operation.id(),
    }
}

fn invalid_operation_exposure(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationExposure {
        operation: operation.id(),
    }
}

fn detention_abort_matches_arrest(
    state: &AppState,
    operation: &OperationRecord,
    aborted_at: SimTime,
    character: crate::core::id::CharacterId,
) -> bool {
    operation.participants().contains(&character)
        && state
            .legal
            .arrests()
            .any(|arrest| arrest.character() == character && arrest.arrested_at() == aborted_at)
}

/// The earliest authored completion deadline among the persisted constraints, if any.
fn resolve_completion_deadline(operation: &OperationRecord) -> Option<SimTime> {
    operation
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            OperationConstraint::CompleteBy(deadline) => Some(*deadline),
            OperationConstraint::RequireIntelligenceTopic(_) => None,
        })
        .min()
}

#[derive(Default)]
struct OperationInvariantContext {
    after_action_information: BTreeSet<InformationId>,
    legal_activity_information: BTreeSet<InformationId>,
    discovered_information: BTreeSet<InformationId>,
    after_action_reports: BTreeSet<crate::core::id::ReportId>,
    history_events: BTreeSet<crate::core::id::HistoryEventId>,
    disposition_transactions: BTreeSet<crate::core::id::LedgerTransactionId>,
    disposition_information: BTreeSet<InformationId>,
    disposition_reports: BTreeSet<crate::core::id::ReportId>,
    text: String,
    surveillance_signatures: BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)>,
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
    state
        .world
        .get_character(operation.leader())
        .ok_or(StateValidationError::MissingEntity {
            context: "operation leader",
            entity: EntityRef::Character(operation.leader()),
        })?;
    let requires_active_membership = matches!(
        operation.status(),
        OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision
    );
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
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => {
            if operation.extraction_arrest().is_some() {
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
        &mut context.disposition_transactions,
        &mut context.disposition_information,
        &mut context.disposition_reports,
        &mut context.text,
    )?;
    dispositions::validate_operation_property_disposition(
        state,
        operation,
        resolution,
        &mut context.disposition_transactions,
        &mut context.disposition_information,
        &mut context.disposition_reports,
        &mut context.text,
    )?;
    validate_completion_after_action(state, operation, resolution, context)?;
    validate_completion_legal_activity(state, operation, resolution, context)?;
    validate_completion_history(state, operation, resolution, context)?;
    validate_operation_discoveries(
        state,
        operation,
        resolution,
        &mut context.discovered_information,
        &mut context.surveillance_signatures,
    )?;
    validate_operation_exposure_links(state, operation, resolution)
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
                && entry
                    .entities
                    .contains(&EntityRef::Operation(operation.id()))
                && entry.entities.contains(&EntityRef::Organization(
                    operation.responsible_organization(),
                ))
                && entry
                    .entities
                    .contains(&EntityRef::Character(operation.leader()))
        })
    {
        return Err(StateValidationError::InvalidOperationAfterActionReport {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_completion_legal_activity(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    context: &mut OperationInvariantContext,
) -> Result<(), StateValidationError> {
    let Some(information_id) = resolution.legal_activity_information() else {
        if resolution.exposure().investigation().is_some() {
            return Err(StateValidationError::InvalidOperationLegalActivity {
                operation: operation.id(),
            });
        }
        return Ok(());
    };
    let investigation_id = resolution.exposure().investigation().ok_or(
        StateValidationError::InvalidOperationLegalActivity {
            operation: operation.id(),
        },
    )?;
    let investigation = state.legal.get_investigation(investigation_id).ok_or(
        StateValidationError::InvalidOperationLegalActivity {
            operation: operation.id(),
        },
    )?;
    let information = state.intelligence.get_information(information_id).ok_or(
        StateValidationError::InvalidOperationLegalActivity {
            operation: operation.id(),
        },
    )?;
    if !context.legal_activity_information.insert(information_id)
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::LegalActivity
        || information.source_entity() != Some(EntityRef::Character(operation.leader()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != resolution.resolved_at()
        || information.recorded_at() != resolution.resolved_at()
        || information.reliability() != Reliability::GenerallyReliable
        || information.specificity() != Specificity::Specific
    {
        return Err(StateValidationError::InvalidOperationLegalActivity {
            operation: operation.id(),
        });
    }
    let authority_name = state
        .world
        .get_organization(investigation.owner())
        .ok_or(StateValidationError::InvalidOperationLegalActivity {
            operation: operation.id(),
        })?
        .name();
    context.text.clear();
    write_legal_activity_summary(&mut context.text, operation.title(), authority_name)
        .expect("String buffer writes are infallible");
    if information.summary() != context.text.as_str() {
        return Err(StateValidationError::InvalidOperationLegalActivity {
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
    let valid = state
        .history
        .get_event(resolution.history_event())
        .is_some_and(|event| {
            context.history_events.insert(event.id())
                && event.kind() == HistoryEventKind::Operation
                && event.occurred_at() == resolution.resolved_at()
                && event
                    .entities()
                    .contains(&EntityRef::Operation(operation.id()))
                && event.entities().contains(&EntityRef::Organization(
                    operation.responsible_organization(),
                ))
                && event
                    .entities()
                    .contains(&EntityRef::Character(operation.leader()))
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

/// The canonical after-action report title for an operation, compared against the authored
/// suffix so per-record validation never rebuilds the string.
fn is_after_action_title(title: &str, operation_title: &str) -> bool {
    title.strip_suffix(" after-action report") == Some(operation_title)
}

fn validate_operation_discoveries(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    discovered_information: &mut BTreeSet<InformationId>,
    actual_signatures: &mut BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)>,
) -> Result<(), StateValidationError> {
    actual_signatures.clear();
    match operation.kind() {
        OperationKind::Surveillance => {
            let OperationObjective::GatherInformation { target } = operation.objective() else {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            };
            if !is_supported_surveillance_target(*target) {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
            match resolution.objective_outcome() {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    if resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Failed
                    if !resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Achieved
                | OperationObjectiveOutcome::Partial
                | OperationObjectiveOutcome::Failed => {}
            }
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::WitnessPressure
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Extraction
        | OperationKind::Sabotage
        | OperationKind::Arson => {
            if !resolution.discovered_information().is_empty() {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
        }
    }

    for information_id in resolution.discovered_information() {
        let information = state.intelligence.get_information(*information_id).ok_or(
            StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            },
        )?;
        if !discovered_information.insert(*information_id)
            || !actual_signatures.insert((
                information.topic(),
                information.subject(),
                information.signal().cloned(),
            ))
            || state
                .operations
                .operation_for_discovered_information(*information_id)
                .is_none_or(|source| source.id() != operation.id())
            || information.recorded_at() != resolution.resolved_at()
            || !is_valid_persisted_surveillance_information(operation, information)
        {
            return Err(StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            });
        }
    }
    // The resolution record froze the signatures this surveillance actually produced; the
    // discovered intelligence records must match that set exactly.
    if operation.kind() == OperationKind::Surveillance
        && resolution.surveillance_signatures() != actual_signatures
    {
        return Err(StateValidationError::InvalidOperationDiscovery {
            operation: operation.id(),
        });
    }
    Ok(())
}
