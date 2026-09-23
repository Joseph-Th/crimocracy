//! Decision validation and atomic cross-subsystem commits; sibling decision state owns pending indexes.

mod recruitment_approval;

use recruitment_approval::validate_recruitment_approval_authority_snapshot;
pub(crate) use recruitment_approval::{
    ValidatedAutonomousRecruitmentApproval, ValidatedRecruitmentApprovalCancellations,
    validate_cancel_recruitment_approvals_for_mandate_change,
    validate_cancel_recruitment_approvals_for_organization_policy_change,
};
pub use recruitment_approval::{
    ValidatedRecruitmentApprovalRequest, validate_request_recruitment_approval,
};

use crate::core::attention::AttentionClass;
use crate::core::id::{
    CharacterId, DecisionRequestId, IdExhaustionError, IdKind, OperationId, OrganizationId,
    PoliceResponseId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{
    VersionCapacityError, advance_version_preflighted, ensure_version_can_advance,
};
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionRecordParts, DecisionRequestDraft,
    DecisionRequestRecord, DecisionResponse, DecisionStatus, RecruitmentApprovalContext,
    build_cancellation, build_resolution,
};
use crate::delegation::ResponsibilityScope;
use crate::delegation::delegation_system::DelegationError;
use crate::legal::PoliceResponseStatus;
use crate::operations::operation_abort::{
    ValidatedOperationAbort, validate_deadline_missed_operation, validate_decision_abort_operation,
    validate_police_arrival_abort_if_applicable,
};
use crate::operations::operation_scheduling::has_operation_deadline_fully_passed;
use crate::operations::operation_system::{
    OperationError, apply_decision_pause_preflighted, apply_decision_resume_preflighted,
};
use crate::operations::{OperationContingency, OperationStatus};
use crate::recruitment::RecruitmentDraft;
use crate::recruitment::recruitment_system::{
    RecruitmentError, ValidatedRecruitmentAttempt, validate_approved_recruitment_attempt,
};
use crate::registry::Registry;
use crate::world::ApprovalPolicy;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DecisionError {
    #[error("decision summary must not be empty")]
    EmptySummary,
    #[error("decision resolution was validated at {expected:?} but committed at {found:?}")]
    StaleResolutionTime { expected: SimTime, found: SimTime },
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
    #[error("operation decision request has a non-operation context")]
    InvalidOperationDecisionContext,
    #[error("operation {operation} has no standing contingency for this exception")]
    MissingContingency { operation: OperationId },
    #[error("operation {operation} already has pending decision {decision}")]
    ExistingPendingDecision {
        operation: OperationId,
        decision: DecisionRequestId,
    },
    #[error("police response {0} does not exist")]
    MissingPoliceResponse(PoliceResponseId),
    #[error(
        "police response {response} cannot support an exception decision for operation {operation}"
    )]
    InvalidPoliceResponseDecision {
        operation: OperationId,
        response: PoliceResponseId,
    },
    #[error(
        "police response {response} changed after validation; expected version {expected}, found {found}"
    )]
    StalePoliceResponse {
        response: PoliceResponseId,
        expected: u32,
        found: u32,
    },
    #[error("recruitment proposal already has pending decision {decision}")]
    ExistingPendingRecruitmentApproval { decision: DecisionRequestId },
    #[error(
        "recruitment approval requires recruiter {recruiter} to be authority manager {manager}"
    )]
    RecruitmentApprovalManagerMismatch {
        recruiter: CharacterId,
        manager: CharacterId,
    },
    #[error("recruitment approval requires Personnel scope, not {scope:?}")]
    RecruitmentApprovalRequiresPersonnelScope { scope: ResponsibilityScope },
    #[error(
        "recruitment approval authority belongs to organization {authority_organization}, not target {target_organization}"
    )]
    RecruitmentApprovalOrganizationMismatch {
        authority_organization: OrganizationId,
        target_organization: OrganizationId,
    },
    #[error("recruitment approval request requires RequireApproval policy, found {policy:?}")]
    RecruitmentApprovalPolicyMismatch { policy: ApprovalPolicy },
    #[error(
        "recruitment approval authority or effective policy changed after the request was created"
    )]
    StaleRecruitmentApprovalAuthority,
    #[error("decision {0} does not exist")]
    MissingDecision(DecisionRequestId),
    #[error("decision {0} is no longer pending")]
    DecisionNotPending(DecisionRequestId),
    #[error(
        "decision {decision} cannot be cancelled because character {character} is not a participant in operation {operation}"
    )]
    InvalidDetentionCancellation {
        decision: DecisionRequestId,
        operation: OperationId,
        character: CharacterId,
    },
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
    #[error(
        "operation {operation} changed after validation; expected version {expected}, found {found}"
    )]
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
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Recruitment(#[from] RecruitmentError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub(crate) struct ValidatedOperationDecisionCancellation {
    decision: DecisionRequestId,
    operation: OperationId,
    character: CharacterId,
    expected_decision_version: u32,
    cancelled_at: SimTime,
}

impl ValidatedOperationDecisionCancellation {
    pub(crate) fn decision(&self) -> DecisionRequestId {
        self.decision
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), DecisionError> {
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
        ensure_version_can_advance(decision.version(), "decision request")?;
        if decision.status() != DecisionStatus::Pending {
            return Err(DecisionError::DecisionNotPending(self.decision));
        }
        crate::core::time::ensure_time_current(state.now(), self.cancelled_at)
            .map_err(|(expected, found)| DecisionError::StaleResolutionTime { expected, found })?;
        let operation = state
            .operations
            .get_operation(self.operation)
            .ok_or(DecisionError::MissingOperation(self.operation))?;
        if operation.status() != OperationStatus::AwaitingDecision
            || state.decisions.pending_for_operation(self.operation) != Some(self.decision)
            || decision.context().operation() != Some(self.operation)
            || !operation.has_participant(self.character)
        {
            return Err(DecisionError::InvalidDetentionCancellation {
                decision: self.decision,
                operation: self.operation,
                character: self.character,
            });
        }
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        state.decisions.cancel(
            self.decision,
            build_cancellation(
                self.cancelled_at,
                DecisionCancellationReason::OperationParticipantDetained(self.character),
            ),
        );
    }
}

pub(crate) fn validate_cancel_operation_decision_for_detention(
    state: &AppState,
    operation: OperationId,
    character: CharacterId,
) -> Result<Option<ValidatedOperationDecisionCancellation>, DecisionError> {
    let operation_record = state
        .operations
        .get_operation(operation)
        .ok_or(DecisionError::MissingOperation(operation))?;
    if operation_record.status() != OperationStatus::AwaitingDecision {
        return Ok(None);
    }
    let decision_id = state
        .decisions
        .pending_for_operation(operation)
        .ok_or(DecisionError::OperationNotAwaitingDecision { operation })?;
    let decision = state
        .decisions
        .get_decision(decision_id)
        .ok_or(DecisionError::MissingDecision(decision_id))?;
    if !operation_record.has_participant(character)
        || decision.context().operation() != Some(operation)
    {
        return Err(DecisionError::InvalidDetentionCancellation {
            decision: decision_id,
            operation,
            character,
        });
    }
    ensure_version_can_advance(decision.version(), "decision request")?;
    Ok(Some(ValidatedOperationDecisionCancellation {
        decision: decision_id,
        operation,
        character,
        expected_decision_version: decision.version(),
        cancelled_at: state.now(),
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionRequestOutcome {
    pub decision: DecisionRequestId,
    pub requests_pause: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoliceResponseDecisionDependency {
    response: PoliceResponseId,
    expected_version: u32,
}

/// Auto-pause only applies to decisions the player is responsible for resolving: a decision
/// addressed to some other organization must not pause the simulation based on the player's
/// own attention preferences. Shared by every decision-request commit path.
fn is_player_pause_requested(
    state: &AppState,
    recipient: OrganizationId,
    attention: AttentionClass,
) -> bool {
    state.player_organization() == Some(recipient)
        && state.attention_settings().is_auto_pause_enabled(attention)
}

/// Single pending-request insertion path shared by every decision kind. Domain-specific
/// validators prove their dependencies first; the decision owner alone allocates the request
/// identity, derives the player pause signal, and updates pending indexes.
pub(super) fn insert_pending_decision_request(
    state: &mut AppState,
    recipient: OrganizationId,
    draft: DecisionRequestDraft,
    options: BTreeSet<DecisionResponse>,
) -> Result<DecisionRequestOutcome, DecisionError> {
    let requests_pause = is_player_pause_requested(state, recipient, draft.attention);
    let id = state.ids.next_decision_request()?;
    state
        .decisions
        .insert(DecisionRequestRecord::from(DecisionRecordParts {
            id,
            recipient,
            requested_at: state.now(),
            options,
            draft,
        }));
    Ok(DecisionRequestOutcome {
        decision: id,
        requests_pause,
    })
}

#[derive(Debug)]
pub struct ValidatedDecisionRequest {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    expected_operation_version: u32,
    police_response: PoliceResponseDecisionDependency,
    options: BTreeSet<DecisionResponse>,
}

impl ValidatedDecisionRequest {
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        vec![(IdKind::DecisionRequest, 1)]
    }

    pub fn commit(self, state: &mut AppState) -> Result<DecisionRequestOutcome, DecisionError> {
        let operation_id = self.operation();
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
        ensure_version_can_advance(operation.version(), "operation")?;
        if let Some(decision) = state.decisions.pending_for_operation(operation_id) {
            return Err(DecisionError::ExistingPendingDecision {
                operation: operation_id,
                decision,
            });
        }
        self.revalidate_police_response(state)?;
        let outcome =
            insert_pending_decision_request(state, self.recipient, self.draft, self.options)?;
        apply_decision_pause_preflighted(state, operation_id, state.now());
        Ok(outcome)
    }

    fn operation(&self) -> OperationId {
        self.draft
            .context
            .operation()
            .expect("validated operation decision must retain operation context")
    }

    fn revalidate_police_response(&self, state: &AppState) -> Result<(), DecisionError> {
        let dependency = &self.police_response;
        let operation = self.operation();
        let response = state
            .legal
            .get_police_response(dependency.response)
            .ok_or(DecisionError::MissingPoliceResponse(dependency.response))?;
        if response.version() != dependency.expected_version {
            return Err(DecisionError::StalePoliceResponse {
                response: dependency.response,
                expected: dependency.expected_version,
                found: response.version(),
            });
        }
        if response.status() != PoliceResponseStatus::Arrived
            || response.source_operation() != operation
            || state
                .operations
                .get_operation(operation)
                .is_none_or(|record| record.police_response() != Some(dependency.response))
        {
            return Err(DecisionError::InvalidPoliceResponseDecision {
                operation,
                response: dependency.response,
            });
        }
        Ok(())
    }
}

pub(crate) fn validate_request_police_arrival_decision_on_arrival(
    state: &AppState,
    response_id: PoliceResponseId,
) -> Result<ValidatedDecisionRequest, DecisionError> {
    let response = state
        .legal
        .get_police_response(response_id)
        .ok_or(DecisionError::MissingPoliceResponse(response_id))?;
    let operation_id = response.source_operation();
    let operation = state
        .operations
        .get_operation(operation_id)
        .ok_or(DecisionError::MissingOperation(operation_id))?;
    if operation.status() != OperationStatus::InProgress {
        return Err(DecisionError::OperationNotInProgress {
            operation: operation_id,
        });
    }
    ensure_version_can_advance(operation.version(), "operation")?;
    if let Some(decision) = state.decisions.pending_for_operation(operation_id) {
        return Err(DecisionError::ExistingPendingDecision {
            operation: operation_id,
            decision,
        });
    }
    if !has_matching_contingency(state, operation_id)? {
        return Err(DecisionError::MissingContingency {
            operation: operation_id,
        });
    }
    if operation.police_response() != Some(response_id)
        || response.status() != PoliceResponseStatus::Dispatched
        || response.arrival_due_at() > state.now()
    {
        return Err(DecisionError::InvalidPoliceResponseDecision {
            operation: operation_id,
            response: response_id,
        });
    }
    if validate_police_arrival_abort_if_applicable(state, operation_id)?.is_some() {
        return Err(DecisionError::InvalidPoliceResponseDecision {
            operation: operation_id,
            response: response_id,
        });
    }
    ensure_version_can_advance(response.version(), "police response")?;
    let expected_version = advance_version_preflighted(response.version());
    let draft = DecisionRequestDraft {
        requester: operation.leader(),
        context: DecisionContext::OperationPoliceArrival {
            operation: operation_id,
            response: response_id,
        },
        attention: AttentionClass::Exception,
        summary: police_arrival_decision_summary(operation.title()),
    };
    validate_request_metadata(state, draft.requester, draft.attention, &draft.summary)?;
    Ok(ValidatedDecisionRequest {
        draft,
        recipient: operation.responsible_organization(),
        expected_operation_version: operation.version(),
        police_response: PoliceResponseDecisionDependency {
            response: response_id,
            expected_version,
        },
        options: BTreeSet::from([DecisionResponse::Continue, DecisionResponse::Abort]),
    })
}

fn police_arrival_decision_summary(operation_title: &str) -> String {
    format!(
        "Police response reached the target during {operation_title}. Leadership direction is required."
    )
}

fn validate_request_metadata(
    state: &AppState,
    requester: CharacterId,
    attention: AttentionClass,
    summary: &str,
) -> Result<(), DecisionError> {
    if summary.trim().is_empty() {
        return Err(DecisionError::EmptySummary);
    }
    match attention {
        AttentionClass::Exception | AttentionClass::Crisis => {}
        AttentionClass::Routine | AttentionClass::Notable => {
            return Err(DecisionError::InvalidAttention);
        }
    }
    let _ = state
        .world
        .get_character(requester)
        .ok_or(DecisionError::MissingCharacter(requester))?;
    Ok(())
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
            OperationContingency::RequestDecisionOnPoliceArrival
        )
    }))
}

enum DecisionResolutionAction {
    Operation {
        operation: OperationId,
        expected_operation_version: u32,
        next_status: OperationStatus,
        abort: Option<Box<ValidatedOperationAbort>>,
    },
    RecruitmentApproval {
        context: RecruitmentApprovalContext,
        attempt: Option<Box<ValidatedRecruitmentAttempt>>,
    },
}

pub struct ValidatedDecisionResolution {
    decision: DecisionRequestId,
    response: DecisionResponse,
    resolver: OrganizationId,
    expected_decision_version: u32,
    validated_at: SimTime,
    action: DecisionResolutionAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionResolutionOutcome {
    pub recruitment_attempt: Option<RecruitmentAttemptId>,
}

impl ValidatedDecisionResolution {
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        match &self.action {
            DecisionResolutionAction::Operation { abort, .. } => abort
                .as_ref()
                .map_or_else(Vec::new, |abort| abort.id_budget()),
            DecisionResolutionAction::RecruitmentApproval { attempt, .. } => attempt
                .as_ref()
                .map_or_else(Vec::new, |attempt| attempt.id_budget()),
        }
    }

    pub fn commit(self, state: &mut AppState) -> Result<DecisionResolutionOutcome, DecisionError> {
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
        ensure_version_can_advance(decision.version(), "decision request")?;
        if decision.status() != DecisionStatus::Pending {
            return Err(DecisionError::DecisionNotPending(self.decision));
        }
        // The abort-vs-deadline classification is fixed at validation; reject clock drift so a
        // resolution cannot commit under conditions that changed after validation.
        crate::core::time::ensure_time_current(state.now(), self.validated_at)
            .map_err(|(expected, found)| DecisionError::StaleResolutionTime { expected, found })?;

        let recruitment_attempt = match self.action {
            DecisionResolutionAction::Operation {
                operation,
                expected_operation_version,
                next_status,
                abort,
            } => {
                let record = state
                    .operations
                    .get_operation(operation)
                    .ok_or(DecisionError::MissingOperation(operation))?;
                if record.version() != expected_operation_version {
                    return Err(DecisionError::StaleOperation {
                        operation,
                        expected: expected_operation_version,
                        found: record.version(),
                    });
                }
                ensure_version_can_advance(record.version(), "operation")?;
                if record.status() != OperationStatus::AwaitingDecision {
                    return Err(DecisionError::OperationNotAwaitingDecision { operation });
                }

                match next_status {
                    OperationStatus::InProgress => {
                        debug_assert!(abort.is_none());
                        // Re-check at commit: the pause may have lengthened since validation,
                        // extending the post-resume window past a conflicting authorization.
                        crate::operations::operation_system::validate_operation_resume_participants(
              state,
              operation,
              state.now(),
            )?;
                        state.decisions.resolve(
                            self.decision,
                            build_resolution(self.response, state.now(), self.resolver),
                        );
                        apply_decision_resume_preflighted(state, operation, state.now());
                    }
                    OperationStatus::Aborted => {
                        (*abort.expect("abort decision must carry an operation abort token"))
                            .commit(state)?;
                        state.decisions.resolve(
                            self.decision,
                            build_resolution(self.response, state.now(), self.resolver),
                        );
                    }
                    OperationStatus::Authorized
                    | OperationStatus::AwaitingDecision
                    | OperationStatus::Completed => {
                        unreachable!("operation decision only resumes or aborts operations")
                    }
                }
                None
            }
            DecisionResolutionAction::RecruitmentApproval { context, attempt } => {
                if self.response == DecisionResponse::Approve {
                    validate_recruitment_approval_authority_snapshot(state, context)?;
                    let attempt = (*attempt
                        .expect("approved recruitment decision must carry an attempt token"))
                    .commit(state)?;
                    state.decisions.resolve(
                        self.decision,
                        build_resolution(self.response, state.now(), self.resolver),
                    );
                    Some(attempt)
                } else {
                    debug_assert_eq!(self.response, DecisionResponse::Reject);
                    debug_assert!(attempt.is_none());
                    state.decisions.resolve(
                        self.decision,
                        build_resolution(self.response, state.now(), self.resolver),
                    );
                    None
                }
            }
        };
        Ok(DecisionResolutionOutcome {
            recruitment_attempt,
        })
    }
}

fn validate_operation_resolution_action(
    registry: &Registry,
    state: &AppState,
    decision: DecisionRequestId,
    response: DecisionResponse,
    operation: OperationId,
) -> Result<DecisionResolutionAction, DecisionError> {
    let operation_record = state
        .operations
        .get_operation(operation)
        .ok_or(DecisionError::MissingOperation(operation))?;
    if operation_record.status() != OperationStatus::AwaitingDecision {
        return Err(DecisionError::OperationNotAwaitingDecision { operation });
    }
    ensure_version_can_advance(operation_record.version(), "operation")?;

    // Arrival processing already applied any standing pre-entry abort before raising
    // the decision, so a Continue here always resumes and an Abort stands down.
    let next_status = match response {
        DecisionResponse::Continue => OperationStatus::InProgress,
        DecisionResponse::Abort => OperationStatus::Aborted,
        DecisionResponse::Approve | DecisionResponse::Reject => {
            return Err(DecisionError::InvalidResponse { decision, response });
        }
    };
    if next_status == OperationStatus::InProgress {
        // Resuming shifts the operation's window; a participant may have been booked
        // into the gap while the operation was paused.
        crate::operations::operation_system::validate_operation_resume_participants(
            state,
            operation,
            state.now(),
        )?;
    }
    let abort = match response {
        // A leadership abort on the deadline minute itself is a choice, not a missed
        // deadline: only an abort strictly after the deadline records `DeadlineMissed`.
        DecisionResponse::Abort => Some(Box::new(
            if has_operation_deadline_fully_passed(state, operation) {
                validate_deadline_missed_operation(registry, state, operation)?
            } else {
                validate_decision_abort_operation(state, operation, decision)?
            },
        )),
        DecisionResponse::Continue => None,
        DecisionResponse::Approve | DecisionResponse::Reject => {
            unreachable!("operation responses were validated above")
        }
    };
    Ok(DecisionResolutionAction::Operation {
        operation,
        expected_operation_version: operation_record.version(),
        next_status,
        abort,
    })
}

fn validate_recruitment_resolution_action(
    registry: &Registry,
    state: &AppState,
    decision: DecisionRequestId,
    response: DecisionResponse,
    context: RecruitmentApprovalContext,
) -> Result<DecisionResolutionAction, DecisionError> {
    let attempt = match response {
        DecisionResponse::Approve => {
            validate_recruitment_approval_authority_snapshot(state, context)?;
            Some(Box::new(validate_approved_recruitment_attempt(
                registry,
                state,
                decision,
                context.authority().authority(),
                RecruitmentDraft {
                    target_organization: context.target_organization(),
                    recruiter: context.recruiter(),
                    candidate: context.candidate(),
                    approach: context.approach(),
                },
            )?))
        }
        DecisionResponse::Reject => None,
        DecisionResponse::Continue | DecisionResponse::Abort => {
            return Err(DecisionError::InvalidResponse { decision, response });
        }
    };
    Ok(DecisionResolutionAction::RecruitmentApproval { context, attempt })
}

pub fn validate_resolve_decision(
    registry: &Registry,
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
    ensure_version_can_advance(record.version(), "decision request")?;
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

    let action = match record.context() {
        DecisionContext::OperationPoliceArrival { operation, .. } => {
            validate_operation_resolution_action(registry, state, decision, response, operation)?
        }
        DecisionContext::RecruitmentApproval(context) => {
            validate_recruitment_resolution_action(registry, state, decision, response, context)?
        }
    };
    Ok(ValidatedDecisionResolution {
        decision,
        response,
        resolver,
        expected_decision_version: record.version(),
        validated_at: state.now(),
        action,
    })
}
