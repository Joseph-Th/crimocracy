//! Daily delegated recruitment decisions; `recruitment_system` remains the canonical transaction owner.

use crate::core::attention::AttentionClass;
use crate::core::id::{
    CharacterId, DecisionRequestId, MandateId, OrganizationId, RecruitmentAttemptId,
};
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

#[derive(Clone, Debug)]
struct PreparedRecruitmentAuthority {
    mandate: MandateId,
    organization: OrganizationId,
    manager: CharacterId,
    policy: ApprovalPolicy,
    approach: RecruitmentApproach,
    candidates: Vec<CharacterId>,
    strongest_relationship_support: u8,
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
    let authorities = prepare_recruitment_authorities(registry, state, personnel_scope)?;
    let mut outcome = AutonomousRecruitmentOutcome::default();
    let mut recruited_this_pass = BTreeSet::new();

    for prepared in authorities {
        let PreparedRecruitmentAuthority {
            mandate,
            organization,
            manager,
            policy,
            approach,
            candidates,
            strongest_relationship_support: _,
        } = prepared;
        // The list was ranked from one read-only snapshot, but earlier authorities in this same
        // pass may already have pitched a prospect or raised this organization's approval request.
        // Recheck only those pass-local exclusions and fall through to the next ranked prospect;
        // canonical validation below still owns every consequential precondition.
        let candidate = candidates.into_iter().find(|candidate| {
            !recruited_this_pass.contains(candidate)
                && state
                    .decisions()
                    .pending_for_recruitment_approval(organization, *candidate)
                    .is_none()
        });
        let Some(candidate) = candidate else {
            continue;
        };
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
        };

        match policy {
            ApprovalPolicy::Delegated => {
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
            ApprovalPolicy::RequireApproval => {
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
        }
    }
    Ok(outcome)
}

/// Builds the day's actionable manager queue without mutation. Managers with no currently usable
/// prospect are absent entirely. Cross-manager contention is ordered by the strongest visible
/// relationship each manager can act on, so a lower mandate ID cannot steal first access to a
/// shared prospect from a materially stronger relationship. Stable IDs break only exact score
/// ties. Pending approvals are unavailable to every autonomous channel because the canonical
/// recruitment validators treat that pair as exclusively owned by the decision route.
fn prepare_recruitment_authorities(
    registry: &Registry,
    state: &AppState,
    personnel_scope: ResponsibilityScope,
) -> Result<Vec<PreparedRecruitmentAuthority>, AutonomousRecruitmentError> {
    let mut prepared = Vec::new();
    for mandate in state.delegation().active_for_scope(personnel_scope) {
        let organization = mandate.organization();
        let manager = mandate.manager();
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
        let resolved_policy =
            resolve_policy_for_manager(state, manager, PolicyKind::IndependentRecruitment)?;
        let PolicySetting::IndependentRecruitment(policy) = resolved_policy.setting else {
            unreachable!("independent-recruitment policy lookup returned another policy kind");
        };
        let mut candidates = find_recruitment_candidates(registry, state, organization, manager)?;
        candidates.retain(|candidate| {
            state
                .decisions()
                .pending_for_recruitment_approval(organization, *candidate)
                .is_none()
        });
        let Some(strongest_relationship_support) = sort_candidates_by_relationship(
            registry.recruitment(),
            state,
            manager,
            &mut candidates,
        ) else {
            continue;
        };
        prepared.push(PreparedRecruitmentAuthority {
            mandate: mandate.id(),
            organization,
            manager,
            policy,
            approach: resolve_autonomous_recruitment_approach(manager_record),
            candidates,
            strongest_relationship_support,
        });
    }
    prepared.sort_unstable_by_key(|authority| {
        (
            Reverse(authority.strongest_relationship_support),
            authority.manager,
            authority.organization,
            authority.mandate,
        )
    });
    Ok(prepared)
}

fn sort_candidates_by_relationship(
    definition: &RecruitmentDefinition,
    state: &AppState,
    recruiter: CharacterId,
    candidates: &mut [CharacterId],
) -> Option<u8> {
    // Resolve each relationship score exactly once. Sorting directly with a comparison closure
    // would repeatedly walk the social index for the same candidates as the sort compared them.
    let mut ranked: Vec<_> = candidates
        .iter()
        .copied()
        .map(|candidate| {
            (
                Reverse(candidate_relationship_support(
                    definition, state, recruiter, candidate,
                )),
                candidate,
            )
        })
        .collect();
    ranked.sort_unstable();
    let strongest = ranked.first().map(|(Reverse(score), _)| *score);
    for (candidate, (_, ranked_candidate)) in candidates.iter_mut().zip(ranked) {
        *candidate = ranked_candidate;
    }
    strongest
}

fn candidate_relationship_support(
    definition: &RecruitmentDefinition,
    state: &AppState,
    recruiter: CharacterId,
    candidate: CharacterId,
) -> u8 {
    let relationship = state
        .social()
        .get_relationship(candidate, recruiter)
        .expect("autonomous recruitment candidates must retain their relationship edge");
    recruitment_relationship_support(definition, relationship.dimensions())
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
