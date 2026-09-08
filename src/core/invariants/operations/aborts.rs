//! Persisted operation-abort lifecycle and artifact validation.

use super::{
    detention_abort_matches_arrest, is_after_action_title, resolve_abort_started_due,
    resolve_completion_deadline,
};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{InformationId, ReportId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionResponse, DecisionStatus,
};
use crate::history::HistoryEventKind;
use crate::intelligence::{InformationSourceKind, InformationTopic, KnowledgeHolder};
use crate::operations::{
    OperationAbortArtifacts, OperationAbortCause, OperationAbortPhase, OperationAbortRecord,
    OperationContingency, OperationRecord,
};
use crate::reports::ReportKind;
use std::collections::BTreeSet;

struct AbortArtifactSets<'a> {
    information: &'a mut BTreeSet<InformationId>,
    reports: &'a mut BTreeSet<ReportId>,
    history: &'a mut BTreeSet<crate::core::id::HistoryEventId>,
}

pub(super) fn validate_operation_abort_links(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    operation_after_action_information: &mut BTreeSet<InformationId>,
    operation_after_action_reports: &mut BTreeSet<ReportId>,
    operation_history_events: &mut BTreeSet<crate::core::id::HistoryEventId>,
) -> Result<(), StateValidationError> {
    if abort.aborted_at() > state.now() {
        return Err(invalid_abort(operation));
    }

    let mut seen = AbortArtifactSets {
        information: operation_after_action_information,
        reports: operation_after_action_reports,
        history: operation_history_events,
    };
    match abort.phase() {
        OperationAbortPhase::BeforeStart => {
            validate_before_start_abort(state, operation, abort, &mut seen)
        }
        OperationAbortPhase::InProgress => {
            validate_in_progress_abort(state, operation, abort, &mut seen)
        }
        OperationAbortPhase::AwaitingDecision => {
            validate_awaiting_decision_abort(state, operation, abort, &mut seen)
        }
    }
}

fn validate_before_start_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    seen: &mut AbortArtifactSets<'_>,
) -> Result<(), StateValidationError> {
    match (abort.cause(), abort.artifacts()) {
        (OperationAbortCause::AuthorityOrder, None) => {
            if operation.started_at().is_some() || operation.resolution_due_at().is_some() {
                return Err(invalid_abort(operation));
            }
            Ok(())
        }
        (OperationAbortCause::DeadlineMissed, Some(artifacts)) => {
            if operation.started_at().is_some()
                || operation.resolution_due_at().is_some()
                || resolve_completion_deadline(operation)
                    .is_none_or(|deadline| deadline > abort.aborted_at())
            {
                return Err(invalid_abort(operation));
            }
            validate_operation_abort_artifacts(state, operation, abort, artifacts, seen)
        }
        (OperationAbortCause::ParticipantDetained(character), Some(artifacts)) => {
            if operation.started_at().is_some()
                || operation.resolution_due_at().is_some()
                || !detention_abort_matches_arrest(state, operation, abort.aborted_at(), character)
            {
                return Err(invalid_abort(operation));
            }
            validate_operation_abort_artifacts(state, operation, abort, artifacts, seen)
        }
        (OperationAbortCause::AuthorityOrder, Some(_))
        | (OperationAbortCause::DeadlineMissed, None)
        | (OperationAbortCause::Decision(_), _)
        | (OperationAbortCause::PoliceArrival(_), _)
        | (OperationAbortCause::ParticipantDetained(_), None) => Err(invalid_abort(operation)),
    }
}

fn validate_in_progress_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    seen: &mut AbortArtifactSets<'_>,
) -> Result<(), StateValidationError> {
    let Some(artifacts) = abort.artifacts() else {
        return Err(invalid_abort(operation));
    };
    match abort.cause() {
        OperationAbortCause::DeadlineMissed => {
            let (started_at, due_at) = resolve_abort_started_due(operation)?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || resolve_completion_deadline(operation)
                    .is_none_or(|deadline| deadline > abort.aborted_at())
            {
                return Err(invalid_abort(operation));
            }
        }
        OperationAbortCause::ParticipantDetained(character) => {
            let (started_at, due_at) = resolve_abort_started_due(operation)?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || !detention_abort_matches_arrest(state, operation, abort.aborted_at(), character)
            {
                return Err(invalid_abort(operation));
            }
        }
        OperationAbortCause::AuthorityOrder => {
            let (started_at, due_at) = resolve_abort_started_due(operation)?;
            if started_at > due_at || abort.aborted_at() < started_at {
                return Err(invalid_abort(operation));
            }
        }
        OperationAbortCause::PoliceArrival(response_id) => {
            validate_in_progress_police_abort(state, operation, abort, response_id)?;
        }
        OperationAbortCause::Decision(_) => return Err(invalid_abort(operation)),
    }
    validate_operation_abort_artifacts(state, operation, abort, artifacts, seen)
}

fn validate_in_progress_police_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    response_id: crate::core::id::PoliceResponseId,
) -> Result<(), StateValidationError> {
    let (Some(started_at), Some(due_at), Some(entry_at)) = (
        operation.started_at(),
        operation.resolution_due_at(),
        operation.entry_at(),
    ) else {
        return Err(invalid_abort(operation));
    };
    let response = state
        .legal
        .get_police_response(response_id)
        .ok_or_else(|| invalid_abort(operation))?;
    if started_at > due_at
        || abort.aborted_at() < started_at
        || operation.police_response() != Some(response_id)
        || response.source_operation() != operation.id()
        || response
            .arrived_at()
            .is_none_or(|arrived_at| arrived_at > abort.aborted_at() || arrived_at >= entry_at)
        || !operation
            .contingencies()
            .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
    {
        return Err(invalid_abort(operation));
    }
    Ok(())
}

fn validate_awaiting_decision_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    seen: &mut AbortArtifactSets<'_>,
) -> Result<(), StateValidationError> {
    let Some(artifacts) = abort.artifacts() else {
        return Err(invalid_abort(operation));
    };
    match abort.cause() {
        OperationAbortCause::ParticipantDetained(character) => {
            validate_awaiting_detention_abort(state, operation, abort, character)?;
        }
        OperationAbortCause::DeadlineMissed => {
            validate_awaiting_deadline_abort(operation, abort)?;
        }
        OperationAbortCause::PoliceArrival(response_id) => {
            validate_awaiting_police_abort(state, operation, abort, response_id)?;
        }
        OperationAbortCause::Decision(decision_id) => {
            validate_decision_abort(state, operation, abort, decision_id)?;
        }
        OperationAbortCause::AuthorityOrder => return Err(invalid_abort(operation)),
    }
    validate_operation_abort_artifacts(state, operation, abort, artifacts, seen)
}

fn validate_awaiting_detention_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    character: crate::core::id::CharacterId,
) -> Result<(), StateValidationError> {
    let (started_at, due_at) = resolve_abort_started_due(operation)?;
    let Some(paused_at) = operation.awaiting_decision_since() else {
        return Err(invalid_abort(operation));
    };
    let cancelled_decisions = state
        .decisions
        .decisions_for_operation(operation.id())
        .filter(|decision| {
            decision.status() == DecisionStatus::Cancelled
                && decision.cancellation().is_some_and(|cancellation| {
                    cancellation.cancelled_at() == abort.aborted_at()
                        && cancellation.reason()
                            == DecisionCancellationReason::OperationParticipantDetained(character)
                })
        })
        .count();
    if started_at > due_at
        || started_at > paused_at
        || paused_at > abort.aborted_at()
        || cancelled_decisions != 1
        || !detention_abort_matches_arrest(state, operation, abort.aborted_at(), character)
    {
        return Err(invalid_abort(operation));
    }
    Ok(())
}

fn validate_awaiting_deadline_abort(
    operation: &OperationRecord,
    abort: OperationAbortRecord,
) -> Result<(), StateValidationError> {
    let (started_at, due_at) = resolve_abort_started_due(operation)?;
    let Some(paused_at) = operation.awaiting_decision_since() else {
        return Err(invalid_abort(operation));
    };
    if started_at > due_at
        || started_at > paused_at
        || paused_at > abort.aborted_at()
        || resolve_completion_deadline(operation)
            .is_none_or(|deadline| deadline > abort.aborted_at())
    {
        return Err(invalid_abort(operation));
    }
    Ok(())
}

fn validate_awaiting_police_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    response_id: crate::core::id::PoliceResponseId,
) -> Result<(), StateValidationError> {
    let (Some(started_at), Some(due_at), Some(entry_at), Some(paused_at)) = (
        operation.started_at(),
        operation.resolution_due_at(),
        operation.entry_at(),
        operation.awaiting_decision_since(),
    ) else {
        return Err(invalid_abort(operation));
    };
    let response = state
        .legal
        .get_police_response(response_id)
        .ok_or_else(|| invalid_abort(operation))?;
    let paused_minutes = abort
        .aborted_at()
        .as_minutes()
        .checked_sub(paused_at.as_minutes())
        .ok_or_else(|| invalid_abort(operation))?;
    let projected_entry = if entry_at > paused_at {
        SimTime::from_minutes(
            entry_at
                .as_minutes()
                .checked_add(paused_minutes)
                .ok_or_else(|| invalid_abort(operation))?,
        )
    } else {
        entry_at
    };
    let matching_continue_decisions = state
        .decisions
        .decisions_for_operation(operation.id())
        .filter(|decision| {
            matches!(
                decision.context(),
                DecisionContext::OperationPoliceArrival { .. }
            ) && decision.resolution().is_some_and(|resolution| {
                resolution.response() == DecisionResponse::Continue
                    && resolution.resolved_at() == abort.aborted_at()
            })
        })
        .count();
    if started_at > due_at
        || started_at > paused_at
        || operation.police_response() != Some(response_id)
        || response.source_operation() != operation.id()
        || response.arrived_at().is_none_or(|arrived_at| {
            arrived_at > abort.aborted_at() || arrived_at >= projected_entry
        })
        || !operation
            .contingencies()
            .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
        || matching_continue_decisions != 1
    {
        return Err(invalid_abort(operation));
    }
    Ok(())
}

fn validate_decision_abort(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    decision_id: crate::core::id::DecisionRequestId,
) -> Result<(), StateValidationError> {
    let (started_at, due_at) = resolve_abort_started_due(operation)?;
    let decision = state
        .decisions
        .get_decision(decision_id)
        .ok_or_else(|| invalid_abort(operation))?;
    let decision_matches = matches!(
        decision.context(),
        DecisionContext::OperationPoliceArrival {
            operation: decision_operation,
            ..
        } if decision_operation == operation.id()
    );
    let resolution = decision
        .resolution()
        .ok_or_else(|| invalid_abort(operation))?;
    if started_at > due_at
        || abort.aborted_at() < started_at
        || !decision_matches
        || decision.status() != DecisionStatus::Resolved
        || decision.recipient() != operation.responsible_organization()
        || decision.requester() != operation.leader()
        || resolution.response() != DecisionResponse::Abort
        || resolution.resolved_at() != abort.aborted_at()
    {
        return Err(invalid_abort(operation));
    }
    Ok(())
}

fn invalid_abort(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationAbort {
        operation: operation.id(),
    }
}

fn validate_operation_abort_artifacts(
    state: &AppState,
    operation: &OperationRecord,
    abort: OperationAbortRecord,
    artifacts: OperationAbortArtifacts,
    seen: &mut AbortArtifactSets<'_>,
) -> Result<(), StateValidationError> {
    let information = state
        .intelligence
        .get_information(artifacts.information())
        .ok_or(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        })?;
    if !seen.information.insert(information.id())
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::OperationalOutcome
        || information.source_entity() != Some(EntityRef::Character(operation.leader()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != abort.aborted_at()
        || information.recorded_at() != abort.aborted_at()
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    // District-scoped enforcement knowledge exists exactly when the abort was caused by a
    // pre-entry police arrival: that is the only abort path where the debriefed crew gives
    // the organization first-hand knowledge of a response in the target's neighborhood.
    let expected_police_activity = match abort.cause() {
        OperationAbortCause::PoliceArrival(response) => state
            .legal
            .get_police_response(response)
            .map(|response| (response.authority(), response.neighborhood())),
        OperationAbortCause::AuthorityOrder
        | OperationAbortCause::Decision(_)
        | OperationAbortCause::DeadlineMissed
        | OperationAbortCause::ParticipantDetained(_) => None,
    };
    match (
        artifacts.police_activity_information(),
        expected_police_activity,
    ) {
        (Some(information_id), Some((authority, neighborhood))) => {
            let police = state.intelligence.get_information(information_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            if police.holder()
                != KnowledgeHolder::Organization(operation.responsible_organization())
                || police.source_kind() != InformationSourceKind::AfterAction
                || police.topic() != InformationTopic::PoliceActivity
                || police.source_entity() != Some(EntityRef::Organization(authority))
                || police.subject() != EntityRef::Neighborhood(neighborhood)
                || police.observed_at() != abort.aborted_at()
                || police.recorded_at() != abort.aborted_at()
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
        }
        (None, None) => {}
        _ => {
            return Err(StateValidationError::InvalidOperationAbort {
                operation: operation.id(),
            });
        }
    }

    let report = state.reports.get_report(artifacts.report()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    let report_entry = report.entries().first();
    if !seen.reports.insert(report.id())
        || report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::AfterAction
        || !is_after_action_title(report.title(), operation.title())
        || report.generated_at() != abort.aborted_at()
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
                && match abort.cause() {
                    OperationAbortCause::AuthorityOrder => true,
                    OperationAbortCause::Decision(decision) => entry
                        .entities
                        .contains(&EntityRef::DecisionRequest(decision)),
                    OperationAbortCause::PoliceArrival(response) => state
                        .legal
                        .get_police_response(response)
                        .is_some_and(|response| {
                            entry
                                .entities
                                .contains(&EntityRef::Organization(response.authority()))
                                && entry
                                    .entities
                                    .contains(&EntityRef::Neighborhood(response.neighborhood()))
                        }),
                    OperationAbortCause::DeadlineMissed => true,
                    OperationAbortCause::ParticipantDetained(character) => {
                        entry.entities.contains(&EntityRef::Character(character))
                    }
                }
        })
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    let history = state.history.get_event(artifacts.history_event()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    if !seen.history.insert(history.id())
        || history.kind() != HistoryEventKind::Operation
        || history.occurred_at() != abort.aborted_at()
        || history.summary() != information.summary()
        || !history
            .entities()
            .contains(&EntityRef::Operation(operation.id()))
        || !history.entities().contains(&EntityRef::Organization(
            operation.responsible_organization(),
        ))
        || !history
            .entities()
            .contains(&EntityRef::Character(operation.leader()))
        || match abort.cause() {
            OperationAbortCause::AuthorityOrder => false,
            OperationAbortCause::Decision(decision) => !history
                .entities()
                .contains(&EntityRef::DecisionRequest(decision)),
            OperationAbortCause::PoliceArrival(response) => state
                .legal
                .get_police_response(response)
                .is_none_or(|response| {
                    !history
                        .entities()
                        .contains(&EntityRef::Organization(response.authority()))
                        || !history
                            .entities()
                            .contains(&EntityRef::Neighborhood(response.neighborhood()))
                }),
            OperationAbortCause::DeadlineMissed => false,
            OperationAbortCause::ParticipantDetained(character) => !history
                .entities()
                .contains(&EntityRef::Character(character)),
        }
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }
    Ok(())
}
