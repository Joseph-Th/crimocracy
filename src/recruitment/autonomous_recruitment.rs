//! Daily delegated recruitment decisions; `recruitment_system` remains the canonical transaction owner.

use crate::core::attention::AttentionClass;
use crate::core::id::{CharacterId, DecisionRequestId, RecruitmentAttemptId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::decision_system::{DecisionError, validate_request_recruitment_approval};
use crate::decisions::{DecisionResponse, RecruitmentApprovalRequestDraft};
use crate::delegation::delegation_system::{DelegationError, resolve_policy_for_manager};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::recruitment::recruitment_system::{
    RecruitmentError, find_recruitment_candidates, validate_delegated_recruitment_attempt,
};
use crate::recruitment::scoring::recruitment_relationship_support;
use crate::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crate::registry::{RecruitmentDefinition, Registry};
use crate::world::{ApprovalPolicy, AutonomyLevel, PolicyKind, PolicySetting, TraitKind};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use thiserror::Error;

/// One autonomous pass's surfaced pitches and durable approval requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutonomousRecruitmentOutcome {
    pub(crate) attempts: Vec<RecruitmentAttemptId>,
    pub(crate) approval_requests: Vec<DecisionRequestId>,
}

#[derive(Debug, Error)]
pub(crate) enum AutonomousRecruitmentError {
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Recruitment(#[from] RecruitmentError),
    #[error(transparent)]
    Decision(#[from] DecisionError),
}

/// Applies the authored recruitment cadence for delegated personnel managers. Candidate choice
/// is deterministic managerial judgment: strongest relationship support wins, with CharacterId
/// used only as an exact-score tie-breaker. This avoids both creation-order strategy and a random
/// manager choosing a visibly weaker relationship while preserving causal information boundaries.
pub(crate) fn apply_due_autonomous_recruitment(
    registry: &Registry,
    state: &mut AppState,
) -> Result<AutonomousRecruitmentOutcome, AutonomousRecruitmentError> {
    let cadence = u64::from(
        registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes(),
    );
    if state.now() == SimTime::ZERO || !state.now().as_minutes().is_multiple_of(cadence) {
        return Ok(AutonomousRecruitmentOutcome::default());
    }

    let personnel_scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let authorities: Vec<_> = state
        .delegation()
        .active_for_scope(personnel_scope)
        .map(|mandate| (mandate.id(), mandate.organization(), mandate.manager()))
        .collect();
    let mut outcome = AutonomousRecruitmentOutcome::default();
    let mut recruited_this_pass = BTreeSet::new();

    for (mandate, organization, manager) in authorities {
        let manager_record = state
            .world()
            .get_character(manager)
            .ok_or(RecruitmentError::MissingRecruiter(manager))?;
        if state.legal().active_arrest_for_character(manager).is_some()
            || !matches!(
                manager_record.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
        {
            continue;
        }

        let policy =
            resolve_policy_for_manager(state, manager, PolicyKind::IndependentRecruitment)?;
        let mut candidates = find_recruitment_candidates(registry, state, organization, manager)?;
        candidates.retain(|candidate| !recruited_this_pass.contains(candidate));
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
        };
        let approach = resolve_autonomous_recruitment_approach(manager_record);

        match policy.setting {
            PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated) => {
                sort_candidates_by_relationship(
                    registry.recruitment(),
                    state,
                    manager,
                    &mut candidates,
                );
                let Some(&candidate) = candidates.first() else {
                    continue;
                };
                let attempt = validate_delegated_recruitment_attempt(
                    registry,
                    state,
                    authority,
                    RecruitmentDraft {
                        target_organization: organization,
                        recruiter: manager,
                        candidate,
                        approach,
                    },
                )?
                .commit(state)?;
                recruited_this_pass.insert(candidate);
                outcome.attempts.push(attempt);
            }
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval) => {
                candidates.retain(|candidate| {
                    state
                        .decisions()
                        .pending_for_recruitment_approval(organization, *candidate)
                        .is_none()
                });
                sort_candidates_by_relationship(
                    registry.recruitment(),
                    state,
                    manager,
                    &mut candidates,
                );
                let Some(&candidate) = candidates.first() else {
                    continue;
                };
                let request = validate_request_recruitment_approval(
                    registry,
                    state,
                    RecruitmentApprovalRequestDraft {
                        authority,
                        target_organization: organization,
                        recruiter: manager,
                        candidate,
                        approach,
                        attention: AttentionClass::Exception,
                        summary: approval_request_summary(state, manager, candidate),
                    },
                )?;
                if state.player_organization() == Some(organization) {
                    let committed = request.commit(state)?;
                    outcome.approval_requests.push(committed.decision);
                } else {
                    let (committed, resolution) = request.commit_autonomous_resolution(
                        registry,
                        state,
                        DecisionResponse::Approve,
                    )?;
                    outcome.approval_requests.push(committed.decision);
                    if let Some(attempt) = resolution.recruitment_attempt {
                        recruited_this_pass.insert(candidate);
                        outcome.attempts.push(attempt);
                    }
                }
            }
            PolicySetting::AssociateLegalSupport(_) => {}
        }
    }
    Ok(outcome)
}

fn sort_candidates_by_relationship(
    definition: &RecruitmentDefinition,
    state: &AppState,
    recruiter: CharacterId,
    candidates: &mut [CharacterId],
) {
    // Resolve each relationship score exactly once. Sorting directly with a comparison closure
    // would repeatedly walk the social index for the same candidates as the sort compared them.
    let mut ranked: Vec<_> = candidates
        .iter()
        .copied()
        .map(|candidate| {
            let relationship = state
                .social()
                .get_relationship(candidate, recruiter)
                .expect("autonomous recruitment candidates must retain their relationship edge");
            (
                Reverse(recruitment_relationship_support(
                    definition,
                    relationship.dimensions(),
                )),
                candidate,
            )
        })
        .collect();
    ranked.sort_unstable();
    for (candidate, (_, ranked_candidate)) in candidates.iter_mut().zip(ranked) {
        *candidate = ranked_candidate;
    }
}

fn approval_request_summary(
    state: &AppState,
    recruiter: CharacterId,
    candidate: CharacterId,
) -> String {
    let recruiter_name = state
        .world()
        .get_character(recruiter)
        .expect("recruitment approval requester must reference a persisted manager")
        .name();
    let candidate_name = state
        .world()
        .get_character(candidate)
        .expect("recruitment approval must reference a persisted candidate")
        .name();
    format!("{recruiter_name} seeks approval to bring {candidate_name} into the organization.")
}

fn resolve_autonomous_recruitment_approach(
    manager: &crate::world::CharacterRecord,
) -> RecruitmentApproach {
    if manager.has_trait(TraitKind::Charismatic) {
        RecruitmentApproach::PersonalAppeal
    } else if manager.has_trait(TraitKind::Ambitious) || manager.has_trait(TraitKind::Proud) {
        RecruitmentApproach::Advancement
    } else if manager.has_trait(TraitKind::Cautious) {
        RecruitmentApproach::Protection
    } else if manager.has_trait(TraitKind::Greedy) {
        RecruitmentApproach::FinancialOpportunity
    } else {
        RecruitmentApproach::PersonalAppeal
    }
}
