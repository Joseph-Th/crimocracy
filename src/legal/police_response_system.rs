//! Versioned law-enforcement dispatch and arrival transactions.

use crate::core::id::{
    IdExhaustionError, NeighborhoodId, OperationId, OrganizationId, PoliceResponseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::patrol_system::resolve_authority_patrol_presence_snapshot;
use crate::legal::{PoliceResponsePatrolSnapshot, PoliceResponseRecord, PoliceResponseStatus};
use crate::operations::OperationStatus;
use crate::world::{OrganizationKind, Rating};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PoliceResponseDispatchDraft {
    pub(crate) authority: OrganizationId,
    pub(crate) neighborhood: NeighborhoodId,
    pub(crate) source_operation: OperationId,
    pub(crate) arrival_due_at: SimTime,
    pub(crate) alert_score: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PoliceResponseError {
    #[error("law-enforcement authority {0} does not exist")]
    MissingAuthority(OrganizationId),
    #[error("organization {0} cannot provide patrol response")]
    InvalidAuthority(OrganizationId),
    #[error("neighborhood {0} does not exist or is inactive")]
    InvalidNeighborhood(NeighborhoodId),
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} is not awaiting its start transaction")]
    InvalidSourceOperation(OperationId),
    #[error(
        "law-enforcement authority {authority} has no jurisdiction over neighborhood {neighborhood}"
    )]
    OutsideJurisdiction {
        authority: OrganizationId,
        neighborhood: NeighborhoodId,
    },
    #[error("operation {operation} already has police response {response}")]
    DuplicateResponse {
        operation: OperationId,
        response: PoliceResponseId,
    },
    #[error("police response arrival cannot be scheduled at or before dispatch")]
    InvalidArrivalTime,
    #[error("police response {0} does not exist")]
    MissingResponse(PoliceResponseId),
    #[error("police response {0} is no longer dispatched")]
    ResponseNotDispatched(PoliceResponseId),
    #[error("police response {response} is not due until {due_at:?}")]
    ArrivalNotDue {
        response: PoliceResponseId,
        due_at: SimTime,
    },
    #[error(
        "police response {response} changed after validation; expected version {expected}, found {found}"
    )]
    StaleResponse {
        response: PoliceResponseId,
        expected: u32,
        found: u32,
    },
    #[error(
        "police response validation occurred at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("police response institutional context changed after validation")]
    StaleInstitutionalContext,
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug)]
pub(crate) struct ValidatedPoliceResponseDispatch {
    draft: PoliceResponseDispatchDraft,
    expected_operation_version: u32,
    response_presence: Rating,
    jurisdiction_version: u32,
    patrol: Option<PoliceResponsePatrolSnapshot>,
    validated_at: SimTime,
}

impl ValidatedPoliceResponseDispatch {
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<PoliceResponseId, PoliceResponseError> {
        validate_dispatch_snapshot(state, &self)?;
        let id = state.ids.next_police_response()?;
        state.legal.insert_police_response(PoliceResponseRecord {
            id,
            routing: super::PoliceResponseRouting {
                authority: self.draft.authority,
                neighborhood: self.draft.neighborhood,
                source_operation: self.draft.source_operation,
            },
            timing: super::PoliceResponseTiming {
                dispatched_at: self.validated_at,
                arrival_due_at: self.draft.arrival_due_at,
                arrived_at: None,
            },
            state: super::PoliceResponseState {
                alert_score: self.draft.alert_score,
                response_presence: self.response_presence,
                jurisdiction_version: self.jurisdiction_version,
                patrol: self.patrol,
                status: PoliceResponseStatus::Dispatched,
                version: 1,
            },
        });
        Ok(id)
    }
}

pub(crate) fn validate_dispatch_police_response(
    state: &AppState,
    draft: PoliceResponseDispatchDraft,
) -> Result<ValidatedPoliceResponseDispatch, PoliceResponseError> {
    validate_dispatch_dependencies(state, draft)?;
    let operation = state
        .operations
        .get_operation(draft.source_operation)
        .ok_or(PoliceResponseError::MissingOperation(
            draft.source_operation,
        ))?;
    let jurisdiction_version = state
        .legal
        .get_jurisdiction(draft.authority)
        .expect("validated response authority must have jurisdiction")
        .version();
    let patrol = resolve_authority_patrol_presence_snapshot(
        state,
        draft.authority,
        draft.neighborhood,
        state.now(),
    );
    Ok(ValidatedPoliceResponseDispatch {
        draft,
        expected_operation_version: operation.version(),
        response_presence: patrol.presence,
        jurisdiction_version,
        patrol: patrol
            .deployment
            .map(|(deployment, version)| PoliceResponsePatrolSnapshot::new(deployment, version)),
        validated_at: state.now(),
    })
}

fn validate_dispatch_dependencies(
    state: &AppState,
    draft: PoliceResponseDispatchDraft,
) -> Result<(), PoliceResponseError> {
    let authority = state
        .world
        .get_organization(draft.authority)
        .ok_or(PoliceResponseError::MissingAuthority(draft.authority))?;
    if authority.kind() != OrganizationKind::LawEnforcement {
        return Err(PoliceResponseError::InvalidAuthority(draft.authority));
    }
    if state.world.get_neighborhood(draft.neighborhood).is_none() {
        return Err(PoliceResponseError::InvalidNeighborhood(draft.neighborhood));
    }
    let operation = state
        .operations
        .get_operation(draft.source_operation)
        .ok_or(PoliceResponseError::MissingOperation(
            draft.source_operation,
        ))?;
    if operation.status() != OperationStatus::Authorized {
        return Err(PoliceResponseError::InvalidSourceOperation(
            draft.source_operation,
        ));
    }
    state
        .legal
        .get_jurisdiction(draft.authority)
        .filter(|record| record.neighborhoods().contains(&draft.neighborhood))
        .ok_or(PoliceResponseError::OutsideJurisdiction {
            authority: draft.authority,
            neighborhood: draft.neighborhood,
        })?;
    if let Some(response) = state
        .legal
        .police_response_for_operation(draft.source_operation)
    {
        return Err(PoliceResponseError::DuplicateResponse {
            operation: draft.source_operation,
            response: response.id(),
        });
    }
    if draft.arrival_due_at <= state.now() {
        return Err(PoliceResponseError::InvalidArrivalTime);
    }
    Ok(())
}

fn validate_dispatch_snapshot(
    state: &AppState,
    token: &ValidatedPoliceResponseDispatch,
) -> Result<(), PoliceResponseError> {
    if state.now() != token.validated_at {
        return Err(PoliceResponseError::StaleTime {
            expected: token.validated_at,
            found: state.now(),
        });
    }
    validate_dispatch_dependencies(state, token.draft)?;
    let operation = state
        .operations
        .get_operation(token.draft.source_operation)
        .expect("validated response source operation must exist");
    if operation.version() != token.expected_operation_version {
        return Err(PoliceResponseError::StaleInstitutionalContext);
    }
    let jurisdiction = state
        .legal
        .get_jurisdiction(token.draft.authority)
        .expect("validated response authority must retain jurisdiction");
    let patrol = resolve_authority_patrol_presence_snapshot(
        state,
        token.draft.authority,
        token.draft.neighborhood,
        token.validated_at,
    );
    let patrol_snapshot = patrol
        .deployment
        .map(|(deployment, version)| PoliceResponsePatrolSnapshot::new(deployment, version));
    if jurisdiction.version() != token.jurisdiction_version
        || patrol_snapshot != token.patrol
        || patrol.presence != token.response_presence
    {
        return Err(PoliceResponseError::StaleInstitutionalContext);
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ValidatedPoliceResponseArrival {
    response: PoliceResponseId,
    expected_version: u32,
    arrived_at: SimTime,
}

impl ValidatedPoliceResponseArrival {
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<PoliceResponseId, PoliceResponseError> {
        let record = state
            .legal
            .get_police_response(self.response)
            .ok_or(PoliceResponseError::MissingResponse(self.response))?;
        if record.version() != self.expected_version {
            return Err(PoliceResponseError::StaleResponse {
                response: self.response,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if state.now() != self.arrived_at {
            return Err(PoliceResponseError::StaleTime {
                expected: self.arrived_at,
                found: state.now(),
            });
        }
        validate_arrival_dependencies(state, record)?;
        state
            .legal
            .set_police_response_arrived(self.response, self.arrived_at);
        Ok(self.response)
    }
}

pub(crate) fn validate_police_response_arrival(
    state: &AppState,
    response: PoliceResponseId,
) -> Result<ValidatedPoliceResponseArrival, PoliceResponseError> {
    let record = state
        .legal
        .get_police_response(response)
        .ok_or(PoliceResponseError::MissingResponse(response))?;
    validate_arrival_dependencies(state, record)?;
    Ok(ValidatedPoliceResponseArrival {
        response,
        expected_version: record.version(),
        arrived_at: state.now(),
    })
}

fn validate_arrival_dependencies(
    state: &AppState,
    record: &PoliceResponseRecord,
) -> Result<(), PoliceResponseError> {
    if record.status() != PoliceResponseStatus::Dispatched {
        return Err(PoliceResponseError::ResponseNotDispatched(record.id()));
    }
    if state.now() < record.arrival_due_at() {
        return Err(PoliceResponseError::ArrivalNotDue {
            response: record.id(),
            due_at: record.arrival_due_at(),
        });
    }
    Ok(())
}

pub(crate) fn find_due_police_responses(state: &AppState) -> Vec<PoliceResponseId> {
    state
        .legal
        .find_police_responses_due_at_or_before(state.now())
}
