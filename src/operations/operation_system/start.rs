//! Begin-time operation admission and commit.
//!
//! Authorization reserves a future plan. This module owns the later admission check that turns
//! that reservation into live work after current custody, objective, deadline, booking, and
//! police-response dependencies have been re-evaluated.

use super::{OperationError, OperationTransition};
use crate::core::id::OperationId;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::ensure_version_can_advance;
use crate::operations::OperationStatus;
use crate::operations::operation_scheduling::{
    find_busy_participant_for_begin, resolve_deadline_without_execution_window,
    resolve_operation_earliest_start,
};
use crate::operations::police_response_integration::{
    OperationPoliceResponseStartPlan, PoliceResponseStartError,
    decide_operation_police_response_start,
};
use crate::registry::Registry;

pub(crate) struct ValidatedOperationStart {
    operation: OperationId,
    expected_version: u32,
    started_at: SimTime,
    resolution_due_at: SimTime,
    police_response: OperationPoliceResponseStartPlan,
}

impl ValidatedOperationStart {
    pub(crate) fn commit(self, state: &mut AppState) -> Result<(), OperationError> {
        let record = state
            .operations
            .get_operation(self.operation)
            .ok_or(OperationError::MissingOperation(self.operation))?;
        if record.version() != self.expected_version {
            return Err(OperationError::StaleBeginOperation {
                operation: self.operation,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        ensure_version_can_advance(record.version(), "operation")?;
        if record.status() != OperationStatus::Authorized {
            return Err(OperationError::InvalidTransition {
                status: record.status(),
                transition: OperationTransition::Begin,
            });
        }
        crate::core::time::ensure_time_current(state.now(), self.started_at)
            .map_err(|(expected, found)| OperationError::StaleBeginTime { expected, found })?;
        let entry_at = self.police_response.entry_at();
        let response = self.police_response.commit_dispatch(state)?;
        state.operations.begin(
            self.operation,
            self.started_at,
            self.resolution_due_at,
            entry_at,
            response,
        );
        Ok(())
    }
}

/// Start planning resolves only the operation record and dispatch validation. Decision,
/// intelligence, and durable-allocation failures belong exclusively to response arrival.
fn map_police_start_planning_error(error: PoliceResponseStartError) -> OperationError {
    match error {
        PoliceResponseStartError::MissingOperation(operation) => {
            OperationError::MissingOperation(operation)
        }
        PoliceResponseStartError::SimulationTimeOverflow => OperationError::SimulationTimeOverflow,
        PoliceResponseStartError::PoliceResponse(dispatch) => dispatch.into(),
    }
}

pub(crate) fn validate_begin_operation(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationStart, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    if record.status() != OperationStatus::Authorized {
        return Err(OperationError::InvalidTransition {
            status: record.status(),
            transition: OperationTransition::Begin,
        });
    }
    ensure_version_can_advance(record.version(), "operation")?;
    let earliest_start = resolve_operation_earliest_start(record);
    if state.now() < earliest_start {
        return Err(OperationError::StartBeforeEarliestStart {
            operation,
            earliest_start,
        });
    }
    if let Some(deadline) = record.completion_deadline()
        && state.now() >= deadline
    {
        return Err(OperationError::DeadlineMissed {
            operation,
            deadline,
            now: state.now(),
        });
    }
    if let Some((opportunity, valid_until)) = state
        .opportunities()
        .expired_window_for_operation(operation, state.now())
    {
        return Err(OperationError::OpportunityWindowExpired {
            operation,
            opportunity: opportunity.id(),
            valid_until,
            now: state.now(),
        });
    }
    if let Some(blocker) =
        crate::operations::operation_objective::resolve_objective_blocker(state, record)
    {
        return Err(OperationError::ObjectiveUnavailable { operation, blocker });
    }
    let execution = registry.get_operation(record.kind()).execution();
    if let Some(deadline) =
        resolve_deadline_without_execution_window(execution, state.now(), record.constraints())
    {
        return Err(OperationError::DeadlineMissed {
            operation,
            deadline,
            now: state.now(),
        });
    }
    let mut resolution_due_at = state
        .now()
        .checked_add(execution.duration())
        .ok_or(OperationError::SimulationTimeOverflow)?;
    for constraint in record.constraints() {
        let crate::operations::OperationConstraint::CompleteBy(deadline) = constraint else {
            continue;
        };
        if *deadline < resolution_due_at {
            resolution_due_at = *deadline;
        }
    }
    let participants = record.participants();
    for character in &participants {
        if let Some(arrest) = state.legal.active_arrest_for_character(*character) {
            return Err(OperationError::DetainedParticipant {
                character: *character,
                arrest: arrest.id(),
            });
        }
    }
    if let Some((character, conflicting_operation)) = find_busy_participant_for_begin(
        registry,
        state,
        &participants,
        operation,
        state.now(),
        resolution_due_at,
    ) {
        return Err(OperationError::ParticipantBusy {
            character,
            operation: conflicting_operation,
        });
    }
    let police_response = decide_operation_police_response_start(registry, state, operation)
        .map_err(map_police_start_planning_error)?;
    Ok(ValidatedOperationStart {
        operation,
        expected_version: record.version(),
        started_at: state.now(),
        resolution_due_at,
        police_response,
    })
}
