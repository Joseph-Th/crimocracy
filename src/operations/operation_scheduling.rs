//! Read-only operation timing, booking, deadline, and due-work projections.

use crate::core::id::{CharacterId, OperationId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::operations::{OperationConstraint, OperationKind, OperationRecord, OperationStatus};
use crate::registry::{OperationExecutionDefinition, Registry};
use std::collections::BTreeSet;

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

/// The authored runtime schedule, before or after a leadership pause. This is a
/// read-only projection; only the operation owner persists these two instants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationTiming {
    pub(crate) due_at: SimTime,
    pub(crate) entry_at: Option<SimTime>,
}

impl OperationTiming {
    /// Preserve remaining execution time during a decision pause. Entry moves only
    /// if it had not yet occurred. A hard completion deadline never moves.
    pub(crate) fn after_pause(self, paused_at: SimTime, resumed_at: SimTime) -> Option<Self> {
        let minutes = resumed_at
            .as_minutes()
            .checked_sub(paused_at.as_minutes())?;
        let due_at = checked_shift_past_pause(self.due_at, minutes)?;
        let entry_at = match self.entry_at {
            Some(entry) if entry > paused_at => Some(checked_shift_past_pause(entry, minutes)?),
            other => other,
        };
        Some(Self { due_at, entry_at })
    }
}

fn checked_shift_past_pause(time: SimTime, paused_minutes: u64) -> Option<SimTime> {
    time.as_minutes()
        .checked_add(paused_minutes)
        .map(SimTime::from_minutes)
}

fn bound_due_at(end: SimTime, constraints: &[OperationConstraint]) -> SimTime {
    constraints
        .iter()
        .filter_map(|constraint| match constraint {
            OperationConstraint::CompleteBy(deadline) => Some(*deadline),
            OperationConstraint::RequireIntelligenceTopic(_) => None,
        })
        .fold(end, SimTime::min)
}

/// Resolve the exact initial schedule for an actual begin, including an authored
/// entry milestone and any compression by the earliest hard deadline.
pub(crate) fn initial_operation_timing(
    execution: &OperationExecutionDefinition,
    start: SimTime,
    constraints: &[OperationConstraint],
) -> Option<OperationTiming> {
    Some(OperationTiming {
        due_at: bound_due_at(start.checked_add(execution.duration())?, constraints),
        entry_at: match execution.operation_entry_offset() {
            Some(offset) => Some(start.checked_add(offset)?),
            None => None,
        },
    })
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
    let end = start
        .checked_add(registry.get_operation(kind).execution().duration())
        .unwrap_or(SimTime::MAX);
    (start, bound_due_at(end, constraints))
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
    projected_started_window(existing, now)
}

/// Use the same projection for admission and historical overlap validation.
/// An unresolved pause cannot reserve personnel past the first minute on which
/// deadline cleanup would abort the operation. The deadline minute itself is
/// still reserved when continuing would exceed it. Begin-time checks continue
/// to require actual release, even when cleanup cannot commit.
fn projected_started_window(
    operation: &OperationRecord,
    at: SimTime,
) -> Option<(SimTime, SimTime)> {
    let start = operation.started_at()?;
    let mut end = operation.resolution_due_at()?;
    if operation.status() == OperationStatus::AwaitingDecision
        && let Some(paused_at) = operation.awaiting_decision_since()
        && paused_at <= at
    {
        end = checked_shift_past_pause(end, at.as_minutes() - paused_at.as_minutes())
            .unwrap_or(SimTime::MAX);
        if let Some(deadline) = operation.completion_deadline() {
            let cleanup = deadline
                .checked_add(SimDuration::ONE_MINUTE)
                .unwrap_or(SimTime::MAX);
            end = end.min(cleanup);
        }
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
    projected_started_window(operation, at)
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

/// Finds a participant already committed to an overlapping non-terminal operation at
/// authorization/commit time.
pub(crate) fn find_busy_participant(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    requested_kind: OperationKind,
    requested_start: SimTime,
    constraints: &[OperationConstraint],
) -> Option<(CharacterId, OperationId)> {
    let (requested_start, requested_end) = projected_authorized_operation_window(
        registry,
        state.now(),
        requested_kind,
        requested_start,
        constraints,
    );
    find_busy_participant_in_window(
        registry,
        state,
        participants,
        None,
        requested_start,
        requested_end,
    )
}

/// Begin-time availability is stricter than authorization-time interval projection. A future
/// follow-up may be authorized exactly at another operation's projected end, but the crew is
/// released only when that earlier operation actually completes or aborts. An unresolved decision
/// or a due operation whose resolution is blocked still owns its participants after that boundary.
pub(crate) fn find_busy_participant_for_begin(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    operation: OperationId,
    requested_start: SimTime,
    requested_end: SimTime,
) -> Option<(CharacterId, OperationId)> {
    let requested = state
        .operations
        .get_operation(operation)
        .expect("begin-time operation must exist");
    participants
        .iter()
        .find_map(|participant| {
            state
                .operations
                .active_operations_for_participant(*participant)
                .find(|other| {
                    other.id() != operation
                        && has_overlapping_operation_window(
                            registry,
                            other,
                            state.now(),
                            requested_start,
                            requested_end,
                        )
                        && (other.status() != OperationStatus::Authorized
                            || authorized_booking_priority(other)
                                < authorized_booking_priority(requested))
                })
                .map(|other| (*participant, other.id()))
        })
        .or_else(|| {
            participants.iter().find_map(|participant| {
                state
                    .operations
                    .active_operations_for_participant(*participant)
                    .find(|other| {
                        other.id() != operation
                            && matches!(
                                other.status(),
                                OperationStatus::InProgress | OperationStatus::AwaitingDecision
                            )
                    })
                    .map(|other| (*participant, other.id()))
            })
        })
}

fn find_busy_participant_in_window(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    excluded_operation: Option<OperationId>,
    requested_start: SimTime,
    requested_end: SimTime,
) -> Option<(CharacterId, OperationId)> {
    participants.iter().find_map(|participant| {
        state
            .operations
            .active_operations_for_participant(*participant)
            .find(|operation| {
                Some(operation.id()) != excluded_operation
                    && has_overlapping_operation_window(
                        registry,
                        operation,
                        state.now(),
                        requested_start,
                        requested_end,
                    )
            })
            .map(|operation| (*participant, operation.id()))
    })
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

/// True once an operation can no longer satisfy its earliest completion deadline.
/// An authorized operation misses its deadline on the deadline minute itself because beginning
/// then could never resolve before it. Live work keeps the deadline minute: an in-progress or
/// decision-paused operation may still resolve on that minute, and only a deadline that passes
/// without resolution is a miss. This matches the automatic overdue cleanup, so a manual
/// deadline abort and the tick pass can never disagree about the deadline minute.
pub(crate) fn has_missed_operation_deadline(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> bool {
    let Some(record) = state.operations.get_operation(operation) else {
        return false;
    };
    let Some(deadline) = record.completion_deadline() else {
        return false;
    };
    if record.status() == OperationStatus::Authorized {
        if state.now() >= deadline {
            return true;
        }
        return resolve_deadline_without_execution_window(
            registry.get_operation(record.kind()).execution(),
            state.now(),
            record.constraints(),
        )
        .is_some();
    }
    state.now() > deadline
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
        .and_then(OperationRecord::completion_deadline)
        .is_some_and(|deadline| state.now() > deadline)
}

/// Finds in-progress or decision-paused work whose completion deadline has fully passed. The
/// operation owner maintains this as a derived chronological index, so quiet ticks do not scan
/// every running and paused operation merely to rediscover immutable authored deadlines.
pub(crate) fn find_due_operations_with_missed_deadlines(state: &AppState) -> Vec<OperationId> {
    state
        .operations
        .find_running_past_completion_deadline(state.now())
}
