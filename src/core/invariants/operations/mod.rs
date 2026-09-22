//! Release-safe structural validation for the operations subsystem.

mod aborts;
mod discoveries;
mod dispositions;
mod exposure;
mod registry;
mod structural;

use crate::core::attention::AttentionClass;
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::InformationId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::{DecisionContext, DecisionResponse, DecisionStatus};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_economics::{
    downgrade_empty_take_outcome, resolve_cash_proceeds, resolve_property_proceeds,
};
use crate::operations::operation_execution::{
    completion_history_summary, has_police_response_arrived_by, participant_after_action_summary,
    render_persisted_after_action_summary, resolve_completion_history_entities,
    resolve_execution_margin, resolve_intelligence_factors, resolve_objective_outcome,
};
use crate::operations::operation_intelligence::resolve_information_score;
use crate::operations::operation_objective::{
    blocker_matches_objective, character_objective_target, effective_objective_outcome,
};
use crate::operations::operation_scheduling::{
    resolve_deadline_without_execution_window, resolve_operation_booking_window,
    resolve_operation_booking_window_at, resolve_operation_earliest_start,
    try_resolve_operation_earliest_start,
};
use crate::operations::operation_system::{
    is_information_subject_relevant, is_valid_operation_objective,
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
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_operations_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    registry::validate_operations_against_registry(registry, state)
}

pub(super) fn validate_operations(state: &AppState) -> Result<(), StateValidationError> {
    structural::validate_operations(state)
}

fn invalid_operation_definition(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationDefinition {
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

/// The canonical after-action report title for an operation, compared against the authored
/// suffix so per-record validation never rebuilds the string.
fn is_after_action_title(title: &str, operation_title: &str) -> bool {
    title.strip_suffix(" after-action report") == Some(operation_title)
}
