//! Recruitment-approval decision lifecycle and authority freshness.
//!
//! The parent decision facade owns generic request/resolution orchestration and operation decisions.
//! This child owns recruitment-specific request creation, obsolete-approval cancellation, and the
//! authority/policy snapshot contract used again when leadership resolves an approval.

use super::{
    DecisionError, DecisionRequestOutcome, DecisionResolutionOutcome, is_player_pause_requested,
    validate_request_metadata,
};
use crate::core::id::{DecisionRequestId, IdKind, MandateId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionRecordParts, DecisionRequestDraft,
    DecisionRequestRecord, DecisionResponse, DecisionStatus, RecruitmentApprovalContext,
    RecruitmentApprovalRequestDraft, build_cancellation,
    build_recruitment_approval_authority_snapshot, build_recruitment_approval_context,
    build_resolution,
};
use crate::delegation::delegation_system::{resolve_mandate_authority, resolve_policy_for_manager};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::recruitment::recruitment_system::{
    ValidatedRecruitmentProposal, recruitment_policy_source, validate_approved_recruitment_attempt,
    validate_recruitment_proposal,
};
use crate::recruitment::{RecruitmentDraft, RecruitmentPolicySource};
use crate::registry::Registry;
use crate::world::{ApprovalPolicy, PolicyKind, PolicySetting};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecruitmentApprovalCancellationScope {
    Mandate(MandateId),
    OrganizationPolicy(OrganizationId),
}

/// Frozen set of pending recruitment approvals whose effective authority is about to be
/// superseded. Mandate and organization-policy mutations compose this decision-owned lifecycle
/// change so an obsolete approval never remains pending with an Approve option that cannot work.
#[derive(Debug)]
pub(crate) struct ValidatedRecruitmentApprovalCancellations {
    scope: RecruitmentApprovalCancellationScope,
    decisions: Vec<(DecisionRequestId, u32)>,
    cancelled_at: SimTime,
}

impl ValidatedRecruitmentApprovalCancellations {
    pub(crate) fn is_current(&self, state: &AppState) -> bool {
        state.now() == self.cancelled_at
            && pending_recruitment_approvals_for_scope(state, self.scope) == self.decisions
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        let reason = match self.scope {
            RecruitmentApprovalCancellationScope::Mandate(mandate) => {
                DecisionCancellationReason::RecruitmentAuthorityChanged(mandate)
            }
            RecruitmentApprovalCancellationScope::OrganizationPolicy(organization) => {
                DecisionCancellationReason::RecruitmentOrganizationPolicyChanged(organization)
            }
        };
        for (decision, _) in self.decisions {
            state
                .decisions
                .cancel(decision, build_cancellation(self.cancelled_at, reason));
        }
    }
}

pub(crate) fn validate_cancel_recruitment_approvals_for_mandate_change(
    state: &AppState,
    mandate: MandateId,
) -> Result<ValidatedRecruitmentApprovalCancellations, VersionCapacityError> {
    validate_cancel_recruitment_approvals_for_scope(
        state,
        RecruitmentApprovalCancellationScope::Mandate(mandate),
    )
}

pub(crate) fn validate_cancel_recruitment_approvals_for_organization_policy_change(
    state: &AppState,
    organization: OrganizationId,
) -> Result<ValidatedRecruitmentApprovalCancellations, VersionCapacityError> {
    validate_cancel_recruitment_approvals_for_scope(
        state,
        RecruitmentApprovalCancellationScope::OrganizationPolicy(organization),
    )
}

fn validate_cancel_recruitment_approvals_for_scope(
    state: &AppState,
    scope: RecruitmentApprovalCancellationScope,
) -> Result<ValidatedRecruitmentApprovalCancellations, VersionCapacityError> {
    let decisions = pending_recruitment_approvals_for_scope(state, scope);
    for (decision, _) in &decisions {
        let record = state
            .decisions
            .get_decision(*decision)
            .expect("pending recruitment approval scan must reference a persisted decision");
        ensure_version_can_advance(record.version(), "decision request")?;
    }
    Ok(ValidatedRecruitmentApprovalCancellations {
        scope,
        decisions,
        cancelled_at: state.now(),
    })
}

fn pending_recruitment_approvals_for_scope(
    state: &AppState,
    scope: RecruitmentApprovalCancellationScope,
) -> Vec<(DecisionRequestId, u32)> {
    state
        .decisions
        .decisions()
        .filter(|decision| decision.status() == DecisionStatus::Pending)
        .filter(|decision| {
            let DecisionContext::RecruitmentApproval(context) = decision.context() else {
                return false;
            };
            match scope {
                RecruitmentApprovalCancellationScope::Mandate(mandate) => {
                    context.authority().authority().mandate == mandate
                }
                RecruitmentApprovalCancellationScope::OrganizationPolicy(organization) => {
                    context.target_organization() == organization
                        && matches!(
                            context.authority().policy_source(),
                            RecruitmentPolicySource::Organization {
                                organization: source,
                                ..
                            } if source == organization
                        )
                }
            }
        })
        .map(|decision| (decision.id(), decision.version()))
        .collect()
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
        if let Some(decision) = state
            .decisions
            .pending_for_recruitment_approval(context.target_organization(), context.candidate())
        {
            return Err(DecisionError::ExistingPendingRecruitmentApproval { decision });
        }
        validate_recruitment_approval_authority_snapshot(state, context)?;
        // Live authority/policy agreement was just re-proven against the context snapshot;
        // the proposal revalidates its own personnel state at commit.
        self.proposal.revalidate_state(state)?;

        let requests_pause = is_player_pause_requested(state, self.recipient, self.draft.attention);
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

    /// Commits a non-player approval request and its deterministic leadership response as one
    /// transaction. The resolved decision remains durable history, but never exists in pending
    /// indexes where a later failure could strand it indefinitely.
    pub(crate) fn commit_autonomous_resolution(
        self,
        registry: &Registry,
        state: &mut AppState,
        response: DecisionResponse,
    ) -> Result<(DecisionRequestOutcome, DecisionResolutionOutcome), DecisionError> {
        let context = match self.draft.context {
            DecisionContext::RecruitmentApproval(context) => context,
            DecisionContext::OperationPoliceArrival { .. } => {
                unreachable!("validated recruitment approval must retain recruitment context")
            }
        };
        debug_assert_ne!(state.player_organization(), Some(self.recipient));
        let predicted_decision =
            DecisionRequestId::from_raw(state.ids.next_raw(IdKind::DecisionRequest));
        if !self.options.contains(&response) {
            return Err(DecisionError::InvalidResponse {
                decision: predicted_decision,
                response,
            });
        }
        if let Some(decision) = state
            .decisions
            .pending_for_recruitment_approval(context.target_organization(), context.candidate())
        {
            return Err(DecisionError::ExistingPendingRecruitmentApproval { decision });
        }
        validate_recruitment_approval_authority_snapshot(state, context)?;
        self.proposal.revalidate_state(state)?;

        let attempt = match response {
            DecisionResponse::Approve => Some(validate_approved_recruitment_attempt(
                registry,
                state,
                predicted_decision,
                context.authority().authority(),
                RecruitmentDraft {
                    target_organization: context.target_organization(),
                    recruiter: context.recruiter(),
                    candidate: context.candidate(),
                    approach: context.approach(),
                },
            )?),
            DecisionResponse::Reject => None,
            DecisionResponse::Continue | DecisionResponse::Abort => {
                return Err(DecisionError::InvalidResponse {
                    decision: predicted_decision,
                    response,
                });
            }
        };

        // Reserve every fallible allocation before the first mutation. The attempt's own
        // commit repeats its budget check defensively, but cannot exhaust after this preflight.
        state.ids.reserve(IdKind::DecisionRequest, 1)?;
        if let Some(attempt) = attempt.as_ref() {
            attempt.preflight_ids(state)?;
        }

        let recruitment_attempt = attempt.map(|attempt| {
            attempt
                .commit(state)
                .expect("autonomous recruitment attempt was revalidated and fully preflighted")
        });
        let decision = state
            .ids
            .next_decision_request()
            .expect("autonomous decision ID was preflighted before mutation");
        debug_assert_eq!(decision, predicted_decision);
        let record = DecisionRequestRecord::from_resolved(
            DecisionRecordParts {
                id: decision,
                recipient: self.recipient,
                requested_at: state.now(),
                options: self.options,
                draft: self.draft,
            },
            build_resolution(response, state.now(), self.recipient),
        );
        state.decisions.insert_resolved(record);
        Ok((
            DecisionRequestOutcome {
                decision,
                requests_pause: false,
            },
            DecisionResolutionOutcome {
                recruitment_attempt,
            },
        ))
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
    if let Some(decision) = state
        .decisions
        .pending_for_recruitment_approval(draft.target_organization, draft.candidate)
    {
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
            recruitment_policy_source(policy),
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

pub(super) fn validate_recruitment_approval_authority_snapshot(
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
        || recruitment_policy_source(policy) != snapshot.policy_source()
    {
        return Err(DecisionError::StaleRecruitmentApprovalAuthority);
    }
    Ok(())
}
