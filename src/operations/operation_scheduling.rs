//! Read-only operation timing, booking, deadline, and due-work projections.

use crate::core::id::OperationId;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::operations::operation_state::{checked_shift_past_pause, pause_duration_minutes};
use crate::operations::{OperationConstraint, OperationKind, OperationRecord, OperationStatus};
use crate::registry::{OperationExecutionDefinition, Registry};

pub(crate) fn authorized_booking_priority(operation: &OperationRecord) -> (SimTime, OperationId) {
    debug_assert_eq!(operation.status(), OperationStatus::Authorized);
    (resolve_operation_earliest_start(operation), operation.id())
}

pub(crate) fn earliest_operation_start_from_authorization(
    authorized_at: SimTime,
    scheduled_for: SimTime,
) -> SimTime {
    checked_earliest_operation_start_from_authorization(authorized_at, scheduled_for)
        .expect("validated operation must retain a representable earliest start")
}

pub(crate) fn checked_earliest_operation_start_from_authorization(
    authorized_at: SimTime,
    scheduled_for: SimTime,
) -> Option<SimTime> {
    if scheduled_for > authorized_at {
        Some(scheduled_for)
    } else {
        authorized_at
            .as_minutes()
            .checked_add(1)
            .map(SimTime::from_minutes)
    }
}

/// First legal execution minute for a persisted authorized operation. A future plan begins on its
/// authored schedule, while a plan authorized for the current minute cannot retroactively execute
/// inside the minute in which authorization was committed.
pub(crate) fn resolve_operation_earliest_start(record: &OperationRecord) -> SimTime {
    earliest_operation_start_from_authorization(record.authorized_at(), record.scheduled_for())
}

pub(crate) fn try_resolve_operation_earliest_start(record: &OperationRecord) -> Option<SimTime> {
    checked_earliest_operation_start_from_authorization(
        record.authorized_at(),
        record.scheduled_for(),
    )
}

/// Earliest completion deadline that leaves no executable minute after the operation's authored
/// entry milestone when beginning at begin_at. None means every deadline still leaves a usable
/// execution window.
pub(crate) fn resolve_deadline_without_execution_window(
    execution: &OperationExecutionDefinition,
    begin_at: SimTime,
    constraints: &[OperationConstraint],
) -> Option<SimTime> {
    let earliest_deadline = constraints
        .iter()
        .filter_map(|constraint| match constraint {
            OperationConstraint::CompleteBy(deadline) => Some(*deadline),
            OperationConstraint::RequireIntelligenceTopic(_) => None,
        });
    let entry_offset = u64::from(
        execution
            .operation_entry_offset()
            .unwrap_or(SimDuration::from_minutes(0))
            .as_minutes(),
    );
    let first_executable_minute = begin_at.as_minutes().checked_add(entry_offset);
    earliest_deadline
        .filter(|deadline| {
            first_executable_minute
                .is_none_or(|first_executable| deadline.as_minutes() <= first_executable)
        })
        .min()
}

/// Occupancy window for an operation that is still awaiting its begin transition.
pub(crate) fn projected_authorized_operation_window(
    registry: &Registry,
    authorized_at: SimTime,
    kind: OperationKind,
    scheduled_for: SimTime,
    constraints: &[OperationConstraint],
) -> (SimTime, SimTime) {
    let start = earliest_operation_start_from_authorization(authorized_at, scheduled_for);
    let mut end = start
        .checked_add(registry.get_operation(kind).execution().duration())
        .unwrap_or(SimTime::from_minutes(u64::MAX));
    for constraint in constraints {
        let OperationConstraint::CompleteBy(deadline) = constraint else {
            continue;
        };
        if *deadline < end {
            end = *deadline;
        }
    }
    (start, end)
}

/// Effective occupancy window of a started non-terminal operation, projecting any unresolved
/// decision pause through now. Authorized operations return None because they have no persisted
/// runtime end yet.
pub(crate) fn projected_operation_window(
    existing: &OperationRecord,
    now: SimTime,
) -> Option<(SimTime, SimTime)> {
    if matches!(
        existing.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return None;
    }
    let start = existing.started_at().unwrap_or(existing.scheduled_for());
    let mut end = existing.resolution_due_at()?;
    if existing.status() == OperationStatus::AwaitingDecision
        && let Some(paused_at) = existing.awaiting_decision_since()
    {
        end = checked_shift_past_pause(end, pause_duration_minutes(paused_at, now))
            .unwrap_or(SimTime::from_minutes(u64::MAX));
    }
    Some((start, end))
}

/// Effective participant-booking window for any non-terminal operation.
pub(crate) fn resolve_operation_booking_window(
    registry: &Registry,
    operation: &OperationRecord,
    now: SimTime,
) -> Option<(SimTime, SimTime)> {
    if let Some(window) = projected_operation_window(operation, now) {
        return Some(window);
    }
    if operation.status() != OperationStatus::Authorized {
        return None;
    }
    Some(projected_authorized_operation_window(
        registry,
        operation.authorized_at(),
        operation.kind(),
        operation.scheduled_for(),
        operation.constraints(),
    ))
}

/// Reconstructs the booking window visible at a historical instant for persistence validation.
pub(crate) fn resolve_operation_booking_window_at(
    registry: &Registry,
    operation: &OperationRecord,
    at: SimTime,
) -> Option<(SimTime, SimTime)> {
    if operation.status() == OperationStatus::Authorized
        || operation
            .started_at()
            .is_none_or(|started_at| at < started_at)
    {
        return Some(projected_authorized_operation_window(
            registry,
            operation.authorized_at(),
            operation.kind(),
            operation.scheduled_for(),
            operation.constraints(),
        ));
    }
    if matches!(
        operation.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return None;
    }
    let start = operation.started_at()?;
    let mut end = operation.resolution_due_at()?;
    if operation.status() == OperationStatus::AwaitingDecision
        && let Some(paused_at) = operation.awaiting_decision_since()
        && paused_at <= at
    {
        end = checked_shift_past_pause(end, pause_duration_minutes(paused_at, at))
            .unwrap_or(SimTime::from_minutes(u64::MAX));
    }
    Some((start, end))
}

pub(crate) fn has_overlapping_operation_window(
    registry: &Registry,
    existing: &OperationRecord,
    now: SimTime,
    requested_start: SimTime,
    requested_end: SimTime,
) -> bool {
    let Some((existing_start, existing_end)) =
        resolve_operation_booking_window(registry, existing, now)
    else {
        return false;
    };
    requested_start < existing_end && existing_start < requested_end
}

pub(crate) fn find_due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state
        .operations
        .find_due_authorized(state.now())
        .into_iter()
        .filter(|operation| {
            let record = state
                .operations
                .get_operation(*operation)
                .expect("authorized start-time index must reference an operation");
            resolve_operation_earliest_start(record) <= state.now()
        })
        .collect()
}

pub(crate) fn resolve_earliest_operation_deadline(record: &OperationRecord) -> Option<SimTime> {
    record
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            OperationConstraint::CompleteBy(deadline) => Some(*deadline),
            OperationConstraint::RequireIntelligenceTopic(_) => None,
        })
        .min()
}

/// True once an operation can no longer satisfy its earliest completion deadline.
pub(crate) fn has_missed_operation_deadline(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> bool {
    let Some(record) = state.operations.get_operation(operation) else {
        return false;
    };
    let Some(deadline) = resolve_earliest_operation_deadline(record) else {
        return false;
    };
    if state.now() >= deadline {
        return true;
    }
    if record.status() != OperationStatus::Authorized {
        return false;
    }
    resolve_deadline_without_execution_window(
        registry.get_operation(record.kind()).execution(),
        state.now(),
        record.constraints(),
    )
    .is_some()
}

/// True only after the completion deadline has fully passed. The deadline minute itself remains
/// available to an explicit leadership decision or same-minute resolution.
pub(crate) fn has_operation_deadline_fully_passed(
    state: &AppState,
    operation: OperationId,
) -> bool {
    state
        .operations
        .get_operation(operation)
        .and_then(resolve_earliest_operation_deadline)
        .is_some_and(|deadline| state.now() > deadline)
}

/// Finds in-progress or decision-paused work whose completion deadline has fully passed, restoring
/// one global chronological order across the separate status indexes.
pub(crate) fn find_due_operations_with_missed_deadlines(state: &AppState) -> Vec<OperationId> {
    let mut due = state
        .operations
        .operations_with_status(OperationStatus::InProgress)
        .chain(
            state
                .operations
                .operations_with_status(OperationStatus::AwaitingDecision),
        )
        .filter(|operation| {
            resolve_earliest_operation_deadline(operation)
                .is_some_and(|deadline| state.now() > deadline)
        })
        .map(|operation| {
            (
                resolve_earliest_operation_deadline(operation)
                    .expect("filtered overdue operation must retain a completion deadline"),
                operation.id(),
            )
        })
        .collect::<Vec<_>>();
    due.sort_unstable();
    due.into_iter().map(|(_, operation)| operation).collect()
}
