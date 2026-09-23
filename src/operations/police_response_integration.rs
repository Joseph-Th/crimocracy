//! Operation-facing police dispatch planning and deterministic response-arrival processing.

use crate::core::entity::EntityRef;
use crate::core::id::{IdExhaustionError, IdKind, OperationId, PoliceResponseId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::decision_system::{
    DecisionError, DecisionRequestOutcome, validate_request_police_arrival_decision_on_arrival,
};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::jurisdiction_system::resolve_police_response_authority;
use crate::legal::patrol_system::resolve_authority_patrol_presence_snapshot;
use crate::legal::police_response_system::{
    PoliceResponseDispatchDraft, PoliceResponseError, ValidatedPoliceResponseDispatch,
    find_due_police_responses, validate_dispatch_police_response, validate_police_response_arrival,
};
use crate::operations::operation_abort::{
    police_arrival_can_abort, validate_authority_abort_operation,
    validate_police_arrival_abort_operation,
};
use crate::operations::operation_execution::resolve_operation_police_alert_context;
use crate::operations::{OperationContingency, OperationStatus};
use crate::registry::OperationExecutionDefinition;
use crate::registry::Registry;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum PoliceResponseIntegrationError {
    #[error(transparent)]
    PoliceResponse(#[from] PoliceResponseError),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Operation(#[from] crate::operations::operation_system::OperationError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug, Error)]
pub(crate) enum PoliceResponseStartError {
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation response timing exceeds the representable simulation clock")]
    SimulationTimeOverflow,
    #[error(transparent)]
    PoliceResponse(#[from] PoliceResponseError),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PoliceResponseProcessingOutcome {
    pub(crate) arrived: Vec<PoliceResponseId>,
    pub(crate) decisions: Vec<DecisionRequestOutcome>,
    pub(crate) aborted_operations: Vec<OperationId>,
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
) -> Result<OperationPoliceResponseStartPlan, PoliceResponseStartError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(PoliceResponseStartError::MissingOperation(operation))?;
    let execution = registry.get_operation(record.kind()).execution();
    let entry_at = execution
        .operation_entry_offset()
        .map(|offset| {
            state
                .now()
                .checked_add(offset)
                .ok_or(PoliceResponseStartError::SimulationTimeOverflow)
        })
        .transpose()?;
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
    let patrol = resolve_authority_patrol_presence_snapshot(
        registry,
        state,
        authority,
        neighborhood,
        state.now(),
    );
    let delay = resolve_police_arrival_delay(execution, patrol.presence.value());
    let arrival_due_at = state
        .now()
        .checked_add(SimDuration::from_minutes(delay))
        .ok_or(PoliceResponseStartError::SimulationTimeOverflow)?;
    let dispatch = validate_dispatch_police_response(
        registry,
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
    match apply_due_police_response_arrivals_strict(state) {
        Ok(outcome) => Ok(outcome),
        Err(error) if police_response_arrival_is_terminally_blocked(&error) => {
            Ok(PoliceResponseProcessingOutcome::default())
        }
        Err(error) => Err(error),
    }
}

fn police_response_arrival_is_terminally_blocked(error: &PoliceResponseIntegrationError) -> bool {
    use crate::decisions::decision_system::DecisionError;
    use crate::legal::police_response_system::PoliceResponseError;
    use crate::operations::operation_system::OperationError;

    let operation_capacity = |error: &OperationError| {
        matches!(
            error,
            OperationError::IdExhaustion(_) | OperationError::VersionCapacity(_)
        )
    };
    match error {
        PoliceResponseIntegrationError::IdExhaustion(_)
        | PoliceResponseIntegrationError::Intelligence(IntelligenceError::IdExhaustion(_))
        | PoliceResponseIntegrationError::PoliceResponse(
            PoliceResponseError::IdExhaustion(_) | PoliceResponseError::VersionCapacity(_),
        ) => true,
        PoliceResponseIntegrationError::Operation(error) => operation_capacity(error),
        PoliceResponseIntegrationError::Decision(
            DecisionError::IdExhaustion(_) | DecisionError::VersionCapacity(_),
        ) => true,
        PoliceResponseIntegrationError::Decision(DecisionError::Operation(error)) => {
            operation_capacity(error)
        }
        PoliceResponseIntegrationError::PoliceResponse(_)
        | PoliceResponseIntegrationError::Decision(_)
        | PoliceResponseIntegrationError::Intelligence(_) => false,
    }
}

fn apply_due_police_response_arrivals_strict(
    state: &mut AppState,
) -> Result<PoliceResponseProcessingOutcome, PoliceResponseIntegrationError> {
    let due = find_due_police_responses(state);
    let mut planned = Vec::with_capacity(due.len());
    let mut batch_budget = Vec::new();
    for response_id in due {
        let (
            operation_id,
            should_abort_before_entry,
            autonomous_leadership_abort,
            decision,
            participant_pressure,
        ) = {
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
            let requests_leadership = !should_abort
                && operation.status() == OperationStatus::InProgress
                && operation
                    .contingencies()
                    .contains(&OperationContingency::RequestDecisionOnPoliceArrival);
            // Only the explicitly designated player organization has an external decision-maker.
            // Every other organization, including all organizations before player designation,
            // must resolve the exception autonomously or the operation can remain blocked forever.
            // Non-player leadership uses the conservative canonical exception rule: abort rather
            // than create a decision request that no external decision-maker can resolve.
            let autonomous_leadership_abort = requests_leadership
                && state.player_organization() != Some(operation.responsible_organization());
            let decision = if requests_leadership && !autonomous_leadership_abort {
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
            (
                operation.id(),
                should_abort,
                autonomous_leadership_abort,
                decision,
                participant_pressure,
            )
        };

        let arrival = validate_police_response_arrival(state, response_id)?;
        let abort = if should_abort_before_entry {
            Some(validate_police_arrival_abort_operation(
                state,
                operation_id,
                response_id,
            )?)
        } else if autonomous_leadership_abort {
            Some(validate_authority_abort_operation(state, operation_id)?)
        } else {
            None
        };

        // One arrival is a cross-domain transaction: legal response status, operation abort or
        // leadership decision, and every participant's first-hand police-pressure knowledge must
        // appear together. Collect every due response first because valid state gives each
        // response a distinct source operation, and operation booking prevents those running
        // operations from sharing participants. Earlier arrival effects therefore cannot stale
        // another due response's prepared transaction in this same minute.
        if let Some(abort) = abort.as_ref() {
            batch_budget.extend(abort.id_budget());
        }
        if let Some(decision) = decision.as_ref() {
            batch_budget.extend(decision.id_budget());
        }
        let participant_information = u32::try_from(participant_pressure.len())
            .expect("operation participant count must fit u32");
        batch_budget.push((IdKind::Information, participant_information));
        planned.push((
            response_id,
            operation_id,
            arrival,
            abort,
            decision,
            participant_pressure,
        ));
    }

    // All responses in this due set occur at one simulation instant. Reserve their complete
    // persistent-ID budget before marking the first response Arrived so global allocator pressure
    // cannot make stable PoliceResponseId order decide which operation receives same-minute
    // enforcement consequences.
    state.ids.reserve_many(&batch_budget)?;

    let mut arrived = Vec::with_capacity(planned.len());
    let mut decisions = Vec::new();
    let mut aborted_operations = Vec::new();
    for (response_id, operation_id, arrival, abort, decision, participant_pressure) in planned {
        arrival
            .commit(state)
            .expect("prevalidated distinct police-response arrival must remain current");
        if let Some(abort) = abort {
            abort
                .commit(state)
                .expect("prevalidated distinct police-arrival abort must remain current");
            aborted_operations.push(operation_id);
        } else if let Some(decision) = decision {
            decisions.push(
                decision
                    .commit(state)
                    .expect("prevalidated distinct police-response decision must remain current"),
            );
        }
        for information in participant_pressure {
            information
                .commit(state)
                .expect("police-pressure information IDs were preflighted for the full due set");
        }
        arrived.push(response_id);
    }
    Ok(PoliceResponseProcessingOutcome {
        arrived,
        decisions,
        aborted_operations,
    })
}

fn validate_participant_police_pressure_information(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    authority: crate::core::id::OrganizationId,
) -> Result<Vec<ValidatedInformation>, IntelligenceError> {
    let authority_name = state
        .world
        .get_organization(authority)
        .expect("validated police response authority must exist")
        .name();
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
