//! Decision validation and atomic cross-subsystem commits; sibling decision state owns pending indexes.

use crate::core::attention::AttentionClass;
use crate::core::id::{
    CharacterId, DecisionRequestId, OperationId, OrganizationId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::decisions::{
    build_recruitment_approval_authority_snapshot, build_recruitment_approval_context,
    build_resolution, DecisionContext, DecisionRecordParts, DecisionRequestDraft,
    DecisionRequestRecord, DecisionResponse, DecisionStatus, OperationExceptionReason,
    RecruitmentApprovalContext, RecruitmentApprovalRequestDraft,
};
use crate::delegation::delegation_system::{
    resolve_mandate_authority, resolve_policy_for_manager, DelegationError, PolicySource,
    ResolvedPolicy,
};
use crate::delegation::{ResolvedMandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::operations::operation_system::{
    validate_decision_abort_operation, OperationError, ValidatedOperationAbort,
};
use crate::operations::{OperationContingency, OperationStatus};
use crate::recruitment::recruitment_system::{
    validate_approved_recruitment_attempt, validate_recruitment_proposal, RecruitmentError,
    ValidatedRecruitmentAttempt, ValidatedRecruitmentProposal,
};
use crate::recruitment::{RecruitmentDraft, RecruitmentPolicySource};
use crate::registry::Registry;
use crate::world::{ApprovalPolicy, PolicyKind, PolicySetting};
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
    #[error("operation decision request has a non-operation context")]
    InvalidOperationDecisionContext,
    #[error("operation {operation} has no standing contingency for this exception")]
    MissingContingency { operation: OperationId },
    #[error("operation {operation} already has pending decision {decision}")]
    ExistingPendingDecision {
        operation: OperationId,
        decision: DecisionRequestId,
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
        let operation_id = self
            .draft
            .context
            .operation()
            .expect("validated operation decision must retain operation context");
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
            .set_awaiting_decision(operation_id, state.now());
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
    validate_request_metadata(state, draft.requester, draft.attention, &draft.summary)?;

    let operation_id = draft
        .context
        .operation()
        .ok_or(DecisionError::InvalidOperationDecisionContext)?;
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
        DecisionContext::RecruitmentApproval(_) => {
            return Err(DecisionError::InvalidOperationDecisionContext);
        }
    };

    Ok(ValidatedDecisionRequest {
        recipient: operation.responsible_organization(),
        expected_operation_version: operation.version(),
        draft,
        options,
    })
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
    if state.world.get_character(requester).is_none() {
        return Err(DecisionError::MissingCharacter(requester));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedRecruitmentApprovalRequest {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    proposal: ValidatedRecruitmentProposal,
    authority: ResolvedMandateAuthority,
    policy: ResolvedPolicy,
    options: BTreeSet<DecisionResponse>,
}

impl ValidatedRecruitmentApprovalRequest {
    pub fn commit(self, state: &mut AppState) -> Result<DecisionRequestOutcome, DecisionError> {
        let context = match self.draft.context {
            DecisionContext::RecruitmentApproval(context) => context,
            DecisionContext::OperationException { .. } => {
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
        if self.authority.mandate_version() != context.authority().mandate_version()
            || self.authority.manager_version() != context.authority().manager_version()
            || recruitment_policy_source(self.policy.source) != context.authority().policy_source()
        {
            return Err(DecisionError::StaleRecruitmentApprovalAuthority);
        }
        self.proposal.revalidate_state(state)?;

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
    let approval = match policy.setting {
        PolicySetting::IndependentRecruitment(approval) => approval,
        _ => unreachable!("policy kind resolution returned the wrong policy variant"),
    };
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
        authority,
        policy,
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
            OperationContingency::RequestDecisionOnUnexpectedCondition
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
        DecisionContext::OperationException {
            operation,
            reason: _,
        } => {
            let operation_record = state
                .operations
                .get_operation(operation)
                .ok_or(DecisionError::MissingOperation(operation))?;
            if operation_record.status() != OperationStatus::AwaitingDecision {
                return Err(DecisionError::OperationNotAwaitingDecision { operation });
            }
            let next_status = match response {
                DecisionResponse::Continue => OperationStatus::InProgress,
                DecisionResponse::Abort => OperationStatus::Aborted,
                DecisionResponse::Approve | DecisionResponse::Reject => {
                    return Err(DecisionError::InvalidResponse { decision, response });
                }
            };
            let abort = match response {
                DecisionResponse::Abort => Some(Box::new(validate_decision_abort_operation(
                    state, operation, decision,
                )?)),
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
                    if state
                        .recruitment
                        .attempt_for_approval_decision(decision)
                        .is_some()
                    {
                        return Err(DecisionError::DecisionNotPending(decision));
                    }
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

fn recruitment_policy_source(source: PolicySource) -> RecruitmentPolicySource {
    match source {
        PolicySource::Organization(organization) => {
            RecruitmentPolicySource::Organization(organization)
        }
        PolicySource::Mandate(mandate) => RecruitmentPolicySource::Mandate(mandate),
    }
}
