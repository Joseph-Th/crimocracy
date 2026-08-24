//! Operation-facing police dispatch planning and deterministic response-arrival processing.

use crate::core::entity::EntityRef;
use crate::core::id::{OperationId, PoliceResponseId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::decision_system::{
    validate_request_police_arrival_decision_on_arrival, DecisionError, DecisionRequestOutcome,
};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::jurisdiction_system::resolve_police_response_authority;
use crate::legal::patrol_system::resolve_authority_patrol_presence_snapshot;
use crate::legal::police_response_system::{
    due_dispatched_police_responses, validate_dispatch_police_response,
    validate_police_response_arrival, PoliceResponseDispatchDraft, PoliceResponseError,
    ValidatedPoliceResponseDispatch,
};
use crate::operations::operation_execution::resolve_operation_police_alert_context;
use crate::operations::operation_system::{
    police_arrival_can_abort, validate_police_arrival_abort_operation,
};
use crate::operations::{OperationContingency, OperationStatus};
use crate::registry::OperationExecutionDefinition;
use crate::registry::Registry;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum PoliceResponseIntegrationError {
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error(transparent)]
    PoliceResponse(#[from] PoliceResponseError),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PoliceResponseProcessingOutcome {
    pub(crate) arrived: Vec<PoliceResponseId>,
    pub(crate) decisions: Vec<DecisionRequestOutcome>,
}

#[derive(Debug)]
pub(crate) struct OperationPoliceResponseStartPlan {
    entry_at: Option<SimTime>,
    dispatch: Option<ValidatedPoliceResponseDispatch>,
}

impl OperationPoliceResponseStartPlan {
    pub(crate) fn entry_at(&self) -> Option<SimTime> {
        self.entry_at
    }

    pub(crate) fn commit_dispatch(
        self,
        state: &mut AppState,
    ) -> Result<Option<crate::core::id::PoliceResponseId>, PoliceResponseError> {
        self.dispatch
            .map(|dispatch| dispatch.commit(state))
            .transpose()
    }
}

/// Deterministic arrival delay for a dispatched response: authored base delay reduced by
/// patrol presence (percent of the authored reduction window) and clamped to the
/// authored minimum. Shared with the invariant validator so timing math cannot drift.
pub(crate) fn resolve_police_arrival_delay(
    execution: &OperationExecutionDefinition,
    response_presence: u8,
) -> u32 {
    let reduction = u32::from(response_presence)
        .saturating_mul(u32::from(execution.patrol_response_reduction_minutes()))
        / 100;
    let base = execution.base_police_response_delay().as_minutes();
    let minimum = execution.minimum_police_response_delay().as_minutes();
    base.saturating_sub(reduction).max(minimum)
}

pub(crate) fn decide_operation_police_response_start(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> Result<OperationPoliceResponseStartPlan, PoliceResponseIntegrationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(PoliceResponseIntegrationError::MissingOperation(operation))?;
    let execution = registry.get_operation(record.kind()).execution();
    let entry_at = execution
        .operation_entry_offset()
        .map(|offset| state.now() + offset);
    let alert = resolve_operation_police_alert_context(registry, state, operation, state.now());
    let Some(neighborhood) = alert.neighborhood() else {
        return Ok(OperationPoliceResponseStartPlan {
            entry_at,
            dispatch: None,
        });
    };
    if alert.score() < execution.police_dispatch_threshold() {
        return Ok(OperationPoliceResponseStartPlan {
            entry_at,
            dispatch: None,
        });
    }
    let Some(authority) = resolve_police_response_authority(state, neighborhood) else {
        return Ok(OperationPoliceResponseStartPlan {
            entry_at,
            dispatch: None,
        });
    };
    let patrol =
        resolve_authority_patrol_presence_snapshot(state, authority, neighborhood, state.now());
    let delay = resolve_police_arrival_delay(execution, patrol.presence.value());
    let arrival_due_at = state.now() + SimDuration::from_minutes(delay);
    let dispatch = validate_dispatch_police_response(
        state,
        PoliceResponseDispatchDraft {
            authority,
            neighborhood,
            source_operation: operation,
            arrival_due_at,
            alert_score: alert.score(),
        },
    )?;
    Ok(OperationPoliceResponseStartPlan {
        entry_at,
        dispatch: Some(dispatch),
    })
}

pub(crate) fn apply_due_police_response_arrivals(
    state: &mut AppState,
) -> Result<PoliceResponseProcessingOutcome, PoliceResponseIntegrationError> {
    let due = due_dispatched_police_responses(state);
    let mut arrived = Vec::with_capacity(due.len());
    let mut decisions = Vec::new();
    for response_id in due {
        let (operation_id, should_abort_before_entry, decision, participant_pressure) = {
            let response = state
                .legal
                .get_police_response(response_id)
                .expect("due police response must still exist");
            let operation = state
                .operations
                .get_operation(response.source_operation())
                .expect("police response source operation must exist");
            // The owning pre-entry abort predicate, shared with the canonical abort
            // validator so the tick pass and validation can never disagree.
            let should_abort = police_arrival_can_abort(state, operation, response_id);
            let decision = if !should_abort
                && operation.status() == OperationStatus::InProgress
                && operation
                    .contingencies()
                    .contains(&OperationContingency::RequestDecisionOnPoliceArrival)
            {
                Some(validate_request_police_arrival_decision_on_arrival(
                    state,
                    response_id,
                )?)
            } else {
                None
            };
            // Every participant present when the response arrives holds first-hand exposure
            // knowledge (`DirectAccess`): rivals later leverage exactly this to poach them,
            // and informant disclosures carry it onward. A pre-entry abort additionally
            // records debrief-derived organizational PoliceActivity knowledge through the
            // abort path — different holder, reliability, and consumer, so both records are
            // intentional and must stay.
            let participant_pressure = if operation.status() == OperationStatus::InProgress {
                validate_participant_police_pressure_information(
                    state,
                    operation,
                    response.authority(),
                )?
            } else {
                Vec::new()
            };
            (operation.id(), should_abort, decision, participant_pressure)
        };

        let arrival = validate_police_response_arrival(state, response_id)?;
        let abort = if should_abort_before_entry {
            Some(
                validate_police_arrival_abort_operation(state, operation_id, response_id)
                    .expect("due pre-entry response must satisfy the authored abort contingency"),
            )
        } else {
            None
        };
        arrival.commit(state)?;
        if let Some(abort) = abort {
            abort
                .commit(state)
                .expect("fresh police-arrival abort token must commit atomically");
        } else if let Some(decision) = decision {
            decisions.push(
                decision
                    .commit(state)
                    .expect("prevalidated police-response decision request must remain current"),
            );
        }
        for information in participant_pressure {
            information
                .commit(state)
                .expect("police pressure information should commit");
        }
        arrived.push(response_id);
    }
    Ok(PoliceResponseProcessingOutcome { arrived, decisions })
}

fn validate_participant_police_pressure_information(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    authority: crate::core::id::OrganizationId,
) -> Result<Vec<ValidatedInformation>, IntelligenceError> {
    let authority_name = state
        .world
        .get_organization(authority)
        .map_or("law enforcement", |record| record.name());
    let participants = operation.participants();
    participants
        .into_iter()
        .map(|participant| {
            let participant_name = state
                .world
                .get_character(participant)
                .expect("validated operation participant must exist")
                .name();
                validate_record_information(
                    state,
                    InformationDraft {
                        holder: KnowledgeHolder::Character(participant),
                        source_kind: InformationSourceKind::DirectObservation,
                        topic: InformationTopic::PoliceActivity,
                        source_entity: Some(EntityRef::Organization(authority)),
                        // The observation is personal exposure knowledge — exactly what a
                        // rival's recruitment pitch or an informant disclosure leverages.
                        subject: EntityRef::Character(participant),
                        observed_at: state.now(),
                        reliability: Reliability::DirectAccess,
                        specificity: Specificity::Precise,
                        summary: format!(
                            "{participant_name} directly experienced {authority_name} responding during {}.",
                            operation.title()
                        ),
                    },
                )
        })
        .collect()
}
