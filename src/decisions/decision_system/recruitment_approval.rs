//! Recruitment-approval decision lifecycle and authority freshness.
//!
//! The parent decision facade owns generic request/resolution orchestration and operation decisions.
//! This child owns recruitment-specific request creation, obsolete-approval cancellation, and the
//! authority/policy snapshot contract used again when leadership resolves an approval.

use super::{
    DecisionError, DecisionRequestOutcome, DecisionResolutionOutcome,
    insert_pending_decision_request, validate_request_metadata,
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

pub(crate) struct ValidatedAutonomousRecruitmentApproval {
    draft: DecisionRequestDraft,
    recipient: OrganizationId,
    options: BTreeSet<DecisionResponse>,
    response: DecisionResponse,
    predicted_decision: DecisionRequestId,
    attempt: Option<crate::recruitment::recruitment_system::ValidatedRecruitmentAttempt>,
}

impl ValidatedRecruitmentApprovalRequest {
    fn context(&self) -> RecruitmentApprovalContext {
        match self.draft.context {
            DecisionContext::RecruitmentApproval(context) => context,
            DecisionContext::OperationPoliceArrival { .. } => {
                unreachable!("validated recruitment approval must retain recruitment context")
            }
        }
    }

    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        vec![(IdKind::DecisionRequest, 1)]
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), DecisionError> {
        let context = self.context();
        if let Some(decision) = state
            .decisions
            .pending_for_recruitment_approval(context.target_organization(), context.candidate())
        {
            return Err(DecisionError::ExistingPendingRecruitmentApproval { decision });
        }
        validate_recruitment_approval_authority_snapshot(state, context)?;
        self.proposal.revalidate_state(state)?;
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<DecisionRequestOutcome, DecisionError> {
        state.ids.reserve_many(&self.id_budget())?;
        self.ensure_current(state)?;
        Ok(self.commit_preflighted(state))
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) -> DecisionRequestOutcome {
        insert_pending_decision_request(state, self.recipient, self.draft, self.options)
            .expect("recruitment approval decision ID was preflighted before mutation")
    }

    pub(crate) fn prepare_autonomous_resolution(
        self,
        registry: &Registry,
        state: &AppState,
        response: DecisionResponse,
        predicted_decision: DecisionRequestId,
    ) -> Result<ValidatedAutonomousRecruitmentApproval, DecisionError> {
        let context = self.context();
        debug_assert_ne!(state.player_organization(), Some(self.recipient));
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
        Ok(ValidatedAutonomousRecruitmentApproval {
            draft: self.draft,
            recipient: self.recipient,
            options: self.options,
            response,
            predicted_decision,
            attempt,
        })
    }
}

impl ValidatedAutonomousRecruitmentApproval {
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let mut budget = vec![(IdKind::DecisionRequest, 1)];
        if let Some(attempt) = &self.attempt {
            budget.extend(attempt.id_budget());
        }
        budget
    }

    pub(crate) fn commit_preflighted(
        self,
        state: &mut AppState,
    ) -> (DecisionRequestOutcome, DecisionResolutionOutcome) {
        let recruitment_attempt = self
            .attempt
            .map(|attempt| attempt.commit_preflighted(state));
        let decision = state
            .ids
            .next_decision_request()
            .expect("autonomous decision ID was preflighted before mutation");
        debug_assert_eq!(decision, self.predicted_decision);
        let record = DecisionRequestRecord::from_resolved(
            DecisionRecordParts {
                id: decision,
                recipient: self.recipient,
                requested_at: state.now(),
                options: self.options,
                draft: self.draft,
            },
            build_resolution(self.response, state.now(), self.recipient),
        );
        state.decisions.insert_resolved(record);
        (
            DecisionRequestOutcome {
                decision,
                requests_pause: false,
            },
            DecisionResolutionOutcome {
                recruitment_attempt,
            },
        )
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
