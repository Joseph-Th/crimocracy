//! Decision validation and atomic cross-subsystem commits; sibling decision state owns pending indexes.

use crate::core::attention::AttentionClass;
use crate::core::id::{
    CharacterId, DecisionRequestId, IdExhaustionError, OperationId, OrganizationId,
    PoliceResponseId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::{
    build_recruitment_approval_authority_snapshot, build_recruitment_approval_context,
    build_resolution, DecisionContext, DecisionRecordParts, DecisionRequestDraft,
    DecisionRequestRecord, DecisionResponse, DecisionStatus, RecruitmentApprovalContext,
    RecruitmentApprovalRequestDraft,
};
use crate::delegation::delegation_system::{
    resolve_mandate_authority, resolve_policy_for_manager, DelegationError,
};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::legal::PoliceResponseStatus;
use crate::operations::operation_system::{
    has_missed_operation_deadline, validate_deadline_missed_operation,
    validate_decision_abort_operation, validate_police_arrival_abort_if_applicable, OperationError,
    ValidatedOperationAbort,
};
use crate::operations::{OperationContingency, OperationStatus};
use crate::recruitment::recruitment_system::{
    recruitment_policy_source, validate_approved_recruitment_attempt,
    validate_recruitment_proposal, RecruitmentError, ValidatedRecruitmentAttempt,
    ValidatedRecruitmentProposal,
};
use crate::recruitment::RecruitmentDraft;
use crate::registry::Registry;
use crate::world::{ApprovalPolicy, PolicyKind, PolicySetting};
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
    #[error("police response {response} changed after validation; expected version {expected}, found {found}")]
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
    #[error("recruitment approval authority belongs to organization {authority_organization}, not target {target_organization}")]
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
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Recruitment(#[from] RecruitmentError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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

#[derive(Debug)]
pub struct ValidatedDecisionRequest {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    expected_operation_version: u32,
    police_response: PoliceResponseDecisionDependency,
    options: BTreeSet<DecisionResponse>,
}

impl ValidatedDecisionRequest {
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
        if let Some(decision) = state.decisions.pending_for_operation(operation_id) {
            return Err(DecisionError::ExistingPendingDecision {
                operation: operation_id,
                decision,
            });
        }
        self.revalidate_police_response(state)?;
        // Auto-pause only applies to decisions the player is responsible for resolving: a decision
        // addressed to some other organization should not pause the simulation based on the
        // player's own attention preferences.
        let requests_pause = state.player_organization() == Some(self.recipient)
            && state
                .attention_settings()
                .is_auto_pause_enabled(self.draft.attention);
        let id = state.ids.next_decision_request()?;
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
            .set_awaiting_decision(operation_id, state.now());
        Ok(DecisionRequestOutcome {
            decision: id,
            requests_pause,
        })
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
    let expected_version = response
        .version()
        .checked_add(1)
        .expect("police response version counter exhausted");
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

#[derive(Debug)]
pub struct ValidatedRecruitmentApprovalRequest {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    proposal: ValidatedRecruitmentProposal,
    options: BTreeSet<DecisionResponse>,
}

impl ValidatedRecruitmentApprovalRequest {
    pub fn commit(self, state: &mut AppState) -> Result<DecisionRequestOutcome, DecisionError> {
        let context = match self.draft.context {
            DecisionContext::RecruitmentApproval(context) => context,
            DecisionContext::OperationPoliceArrival { .. } => {
                unreachable!("validated recruitment approval must retain recruitment context")
            }
        };
        if let Some(decision) = state.decisions.pending_for_recruitment_approval(
            context.target_organization(),
            context.recruiter(),
            context.candidate(),
        ) {
            return Err(DecisionError::ExistingPendingRecruitmentApproval { decision });
        }
        validate_recruitment_approval_authority_snapshot(state, context)?;
        // Live authority/policy agreement was just re-proven against the context snapshot;
        // the proposal revalidates its own personnel state at commit.
        self.proposal.revalidate_state(state)?;

        let requests_pause = state.player_organization() == Some(self.recipient)
            && state
                .attention_settings()
                .is_auto_pause_enabled(self.draft.attention);
        let id = state.ids.next_decision_request()?;
        state
            .decisions
            .insert(DecisionRequestRecord::from(DecisionRecordParts {
                id,
                recipient: self.recipient,
                requested_at: state.now(),
                options: self.options,
                draft: self.draft,
            }));
        Ok(DecisionRequestOutcome {
            decision: id,
            requests_pause,
        })
    }
}

pub fn validate_request_recruitment_approval(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentApprovalRequestDraft,
) -> Result<ValidatedRecruitmentApprovalRequest, DecisionError> {
    validate_request_metadata(state, draft.recruiter, draft.attention, &draft.summary)?;
    if draft.authority.manager != draft.recruiter {
        return Err(DecisionError::RecruitmentApprovalManagerMismatch {
            recruiter: draft.recruiter,
            manager: draft.authority.manager,
        });
    }
    if draft.authority.scope != ResponsibilityScope::Function(ResponsibilityFunction::Personnel) {
        return Err(DecisionError::RecruitmentApprovalRequiresPersonnelScope {
            scope: draft.authority.scope,
        });
    }
    let authority = resolve_mandate_authority(state, draft.authority)?;
    if authority.organization() != draft.target_organization {
        return Err(DecisionError::RecruitmentApprovalOrganizationMismatch {
            authority_organization: authority.organization(),
            target_organization: draft.target_organization,
        });
    }
    let policy =
        resolve_policy_for_manager(state, draft.recruiter, PolicyKind::IndependentRecruitment)?;
    let approval = policy.independent_recruitment_approval();
    if approval != ApprovalPolicy::RequireApproval {
        return Err(DecisionError::RecruitmentApprovalPolicyMismatch { policy: approval });
    }
    if let Some(decision) = state.decisions.pending_for_recruitment_approval(
        draft.target_organization,
        draft.recruiter,
        draft.candidate,
    ) {
        return Err(DecisionError::ExistingPendingRecruitmentApproval { decision });
    }
    let proposal = validate_recruitment_proposal(
        registry,
        state,
        RecruitmentDraft {
            target_organization: draft.target_organization,
            recruiter: draft.recruiter,
            candidate: draft.candidate,
            approach: draft.approach,
        },
    )?;
    let context = build_recruitment_approval_context(
        draft.target_organization,
        draft.recruiter,
        draft.candidate,
        draft.approach,
        build_recruitment_approval_authority_snapshot(
            draft.authority,
            authority.mandate_version(),
            authority.manager_version(),
            recruitment_policy_source(policy.source),
        ),
    );
    Ok(ValidatedRecruitmentApprovalRequest {
        draft: DecisionRequestDraft {
            requester: draft.recruiter,
            context,
            attention: draft.attention,
            summary: draft.summary,
        },
        recipient: draft.target_organization,
        proposal,
        options: BTreeSet::from([DecisionResponse::Approve, DecisionResponse::Reject]),
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
        if decision.status() != DecisionStatus::Pending {
            return Err(DecisionError::DecisionNotPending(self.decision));
        }
        // The abort-vs-deadline classification is fixed at validation; reject clock drift so a
        // resolution cannot commit under conditions that changed after validation.
        if state.now() != self.validated_at {
            return Err(DecisionError::StaleResolutionTime {
                expected: self.validated_at,
                found: state.now(),
            });
        }

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
                        state.operations.resume(operation, state.now());
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
            let operation_record = state
                .operations
                .get_operation(operation)
                .ok_or(DecisionError::MissingOperation(operation))?;
            if operation_record.status() != OperationStatus::AwaitingDecision {
                return Err(DecisionError::OperationNotAwaitingDecision { operation });
            }
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
                DecisionResponse::Abort => Some(Box::new(
                    if has_missed_operation_deadline(state, operation) {
                        validate_deadline_missed_operation(state, operation)?
                    } else {
                        validate_decision_abort_operation(state, operation, decision)?
                    },
                )),
                DecisionResponse::Continue => None,
                DecisionResponse::Approve | DecisionResponse::Reject => {
                    unreachable!("operation responses were validated above")
                }
            };
            DecisionResolutionAction::Operation {
                operation,
                expected_operation_version: operation_record.version(),
                next_status,
                abort,
            }
        }
        DecisionContext::RecruitmentApproval(context) => {
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
            DecisionResolutionAction::RecruitmentApproval { context, attempt }
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

fn validate_recruitment_approval_authority_snapshot(
    state: &AppState,
    context: RecruitmentApprovalContext,
) -> Result<(), DecisionError> {
    let snapshot = context.authority();
    let authority = resolve_mandate_authority(state, snapshot.authority())?;
    if authority.organization() != context.target_organization()
        || authority.mandate_version() != snapshot.mandate_version()
        || authority.manager_version() != snapshot.manager_version()
    {
        return Err(DecisionError::StaleRecruitmentApprovalAuthority);
    }
    let policy = resolve_policy_for_manager(
        state,
        context.recruiter(),
        PolicyKind::IndependentRecruitment,
    )?;
    if policy.setting != PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval)
        || recruitment_policy_source(policy.source) != snapshot.policy_source()
    {
        return Err(DecisionError::StaleRecruitmentApprovalAuthority);
    }
    Ok(())
}
