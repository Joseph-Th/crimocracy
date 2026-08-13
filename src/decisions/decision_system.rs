//! Decision validation and atomic cross-subsystem commits; sibling decision state owns pending indexes.

use crate::core::attention::AttentionClass;
use crate::core::id::{CharacterId, DecisionRequestId, OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::decisions::{
    build_resolution, DecisionContext, DecisionRecordParts, DecisionRequestDraft,
    DecisionRequestRecord, DecisionResponse, DecisionStatus, OperationExceptionReason,
};
use crate::operations::{OperationContingency, OperationStatus};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DecisionError {
    #[error("decision summary must not be empty")]
    EmptySummary,
    #[error("decision attention must be Exception or Crisis")]
    InvalidAttention,
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("operation {operation} is not in progress")]
    OperationNotInProgress { operation: OperationId },
    #[error("operation {operation} is not awaiting a decision")]
    OperationNotAwaitingDecision { operation: OperationId },
    #[error("character {requester} is not the responsible leader of operation {operation}")]
    InvalidRequester {
        requester: CharacterId,
        operation: OperationId,
    },
    #[error("operation {operation} has no standing contingency for this exception")]
    MissingContingency { operation: OperationId },
    #[error("operation {operation} already has pending decision {decision}")]
    ExistingPendingDecision {
        operation: OperationId,
        decision: DecisionRequestId,
    },
    #[error("decision {0} does not exist")]
    MissingDecision(DecisionRequestId),
    #[error("decision {0} is no longer pending")]
    DecisionNotPending(DecisionRequestId),
    #[error("organization {resolver} cannot resolve decision {decision} owned by {recipient}")]
    InvalidResolver {
        decision: DecisionRequestId,
        resolver: OrganizationId,
        recipient: OrganizationId,
    },
    #[error("response {response:?} is not available for decision {decision}")]
    InvalidResponse {
        decision: DecisionRequestId,
        response: DecisionResponse,
    },
    #[error("operation {operation} changed after validation; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "decision {decision} changed after validation; expected version {expected}, found {found}"
    )]
    StaleDecision {
        decision: DecisionRequestId,
        expected: u32,
        found: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionRequestOutcome {
    pub decision: DecisionRequestId,
    pub requests_pause: bool,
}

#[derive(Debug)]
pub struct ValidatedDecisionRequest {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    expected_operation_version: u32,
    options: BTreeSet<DecisionResponse>,
}

impl ValidatedDecisionRequest {
    pub fn commit(self, state: &mut AppState) -> Result<DecisionRequestOutcome, DecisionError> {
        let operation_id = self.draft.context.operation();
        let operation = state
            .operations
            .get_operation(operation_id)
            .ok_or(DecisionError::MissingOperation(operation_id))?;
        if operation.version() != self.expected_operation_version {
            return Err(DecisionError::StaleOperation {
                operation: operation_id,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        if operation.status() != OperationStatus::InProgress {
            return Err(DecisionError::OperationNotInProgress {
                operation: operation_id,
            });
        }
        if let Some(decision) = state.decisions.pending_for_operation(operation_id) {
            return Err(DecisionError::ExistingPendingDecision {
                operation: operation_id,
                decision,
            });
        }

        let requests_pause = state
            .attention_settings()
            .is_auto_pause_enabled(self.draft.attention);
        let id = state.ids.next_decision_request();
        state
            .decisions
            .insert(DecisionRequestRecord::from(DecisionRecordParts {
                id,
                recipient: self.recipient,
                requested_at: state.now(),
                options: self.options,
                draft: self.draft,
            }));
        state
            .operations
            .transition(operation_id, OperationStatus::AwaitingDecision);
        Ok(DecisionRequestOutcome {
            decision: id,
            requests_pause,
        })
    }
}

pub fn validate_request_decision(
    state: &AppState,
    draft: DecisionRequestDraft,
) -> Result<ValidatedDecisionRequest, DecisionError> {
    if draft.summary.trim().is_empty() {
        return Err(DecisionError::EmptySummary);
    }
    match draft.attention {
        AttentionClass::Exception | AttentionClass::Crisis => {}
        AttentionClass::Routine | AttentionClass::Notable => {
            return Err(DecisionError::InvalidAttention);
        }
    }
    if state.world.get_character(draft.requester).is_none() {
        return Err(DecisionError::MissingCharacter(draft.requester));
    }

    let operation_id = draft.context.operation();
    let operation = state
        .operations
        .get_operation(operation_id)
        .ok_or(DecisionError::MissingOperation(operation_id))?;
    if operation.status() != OperationStatus::InProgress {
        return Err(DecisionError::OperationNotInProgress {
            operation: operation_id,
        });
    }
    if operation.leader() != draft.requester {
        return Err(DecisionError::InvalidRequester {
            requester: draft.requester,
            operation: operation_id,
        });
    }
    if let Some(decision) = state.decisions.pending_for_operation(operation_id) {
        return Err(DecisionError::ExistingPendingDecision {
            operation: operation_id,
            decision,
        });
    }

    let options = match draft.context {
        DecisionContext::OperationException { operation, reason } => match reason {
            OperationExceptionReason::UnexpectedCondition => {
                if !has_matching_contingency(state, operation)? {
                    return Err(DecisionError::MissingContingency { operation });
                }
                BTreeSet::from([DecisionResponse::Continue, DecisionResponse::Abort])
            }
        },
    };

    Ok(ValidatedDecisionRequest {
        recipient: operation.responsible_organization(),
        expected_operation_version: operation.version(),
        draft,
        options,
    })
}

fn has_matching_contingency(
    state: &AppState,
    operation: OperationId,
) -> Result<bool, DecisionError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(DecisionError::MissingOperation(operation))?;
    Ok(record.contingencies().iter().any(|contingency| {
        matches!(
            contingency,
            OperationContingency::RequestDecisionOnUnexpectedCondition
        )
    }))
}

pub struct ValidatedDecisionResolution {
    decision: DecisionRequestId,
    response: DecisionResponse,
    resolver: OrganizationId,
    operation: OperationId,
    expected_decision_version: u32,
    expected_operation_version: u32,
    next_operation_status: OperationStatus,
}

impl ValidatedDecisionResolution {
    pub fn commit(self, state: &mut AppState) -> Result<(), DecisionError> {
        let decision = state
            .decisions
            .get_decision(self.decision)
            .ok_or(DecisionError::MissingDecision(self.decision))?;
        if decision.version() != self.expected_decision_version {
            return Err(DecisionError::StaleDecision {
                decision: self.decision,
                expected: self.expected_decision_version,
                found: decision.version(),
            });
        }
        if decision.status() != DecisionStatus::Pending {
            return Err(DecisionError::DecisionNotPending(self.decision));
        }

        let operation = state
            .operations
            .get_operation(self.operation)
            .ok_or(DecisionError::MissingOperation(self.operation))?;
        if operation.version() != self.expected_operation_version {
            return Err(DecisionError::StaleOperation {
                operation: self.operation,
                expected: self.expected_operation_version,
                found: operation.version(),
            });
        }
        if operation.status() != OperationStatus::AwaitingDecision {
            return Err(DecisionError::OperationNotAwaitingDecision {
                operation: self.operation,
            });
        }

        state.decisions.resolve(
            self.decision,
            build_resolution(self.response, state.now(), self.resolver),
        );
        state
            .operations
            .transition(self.operation, self.next_operation_status);
        Ok(())
    }
}

pub fn validate_resolve_decision(
    state: &AppState,
    decision: DecisionRequestId,
    resolver: OrganizationId,
    response: DecisionResponse,
) -> Result<ValidatedDecisionResolution, DecisionError> {
    if state.world.get_organization(resolver).is_none() {
        return Err(DecisionError::MissingOrganization(resolver));
    }
    let record = state
        .decisions
        .get_decision(decision)
        .ok_or(DecisionError::MissingDecision(decision))?;
    if record.status() != DecisionStatus::Pending {
        return Err(DecisionError::DecisionNotPending(decision));
    }
    if record.recipient() != resolver {
        return Err(DecisionError::InvalidResolver {
            decision,
            resolver,
            recipient: record.recipient(),
        });
    }
    if !record.options().contains(&response) {
        return Err(DecisionError::InvalidResponse { decision, response });
    }

    let operation_id = record.context().operation();
    let operation = state
        .operations
        .get_operation(operation_id)
        .ok_or(DecisionError::MissingOperation(operation_id))?;
    if operation.status() != OperationStatus::AwaitingDecision {
        return Err(DecisionError::OperationNotAwaitingDecision {
            operation: operation_id,
        });
    }

    let next_operation_status = match response {
        DecisionResponse::Continue => OperationStatus::InProgress,
        DecisionResponse::Abort => OperationStatus::Aborted,
    };
    Ok(ValidatedDecisionResolution {
        decision,
        response,
        resolver,
        operation: operation_id,
        expected_decision_version: record.version(),
        expected_operation_version: operation.version(),
        next_operation_status,
    })
}
