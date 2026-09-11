//! Daily delegated recruitment decisions; `recruitment_system` remains the canonical transaction owner.

use crate::core::attention::AttentionClass;
use crate::core::id::{CharacterId, MandateId, OrganizationId, RecruitmentAttemptId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::decision_system::{
    DecisionError, DecisionRequestOutcome, validate_request_recruitment_approval,
};
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
    pub(crate) approval_requests: Vec<DecisionRequestOutcome>,
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
struct RankedRecruitmentCandidate {
    character: CharacterId,
    relationship_support: u8,
}

#[derive(Clone, Debug)]
struct PreparedRecruitmentAuthority {
    mandate: MandateId,
    organization: OrganizationId,
    manager: CharacterId,
    policy: ApprovalPolicy,
    approach: RecruitmentApproach,
    candidates: Vec<RankedRecruitmentCandidate>,
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
    let mut authorities = prepare_recruitment_authorities(registry, state, personnel_scope)?;
    let mut outcome = AutonomousRecruitmentOutcome::default();
    let mut claimed_this_pass = BTreeSet::new();

    while !authorities.is_empty() {
        let Some((authority_index, candidate)) =
            select_next_recruitment_action(state, &authorities, &claimed_this_pass)
        else {
            break;
        };
        let prepared = authorities.remove(authority_index);
        let PreparedRecruitmentAuthority {
            mandate,
            organization,
            manager,
            policy,
            approach,
            candidates: _,
        } = prepared;
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
                claimed_this_pass.insert(candidate);
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
                    // The strongest live relationship won this pass's contention. Keep the
                    // candidate unavailable to weaker same-minute autonomous pitches while
                    // leadership owns the surfaced approval decision.
                    claimed_this_pass.insert(candidate);
                    outcome.approval_requests.push(committed);
                } else {
                    let (committed, resolution) = request.commit_autonomous_resolution(
                        registry,
                        state,
                        DecisionResponse::Approve,
                    )?;
                    outcome.approval_requests.push(committed);
                    if let Some(attempt) = resolution.recruitment_attempt {
                        claimed_this_pass.insert(candidate);
                        outcome.attempts.push(attempt);
                    }
                }
            }
        }
    }
    Ok(outcome)
}

/// Builds the day's actionable manager queue without mutation. Managers with no currently usable
/// prospect are absent entirely. Candidate lists are relationship-ranked here, while the live
/// cross-manager priority is selected after each same-pass action. This matters when an earlier
/// manager consumes another manager's first choice: the losing manager's weaker fallback must
/// not retain the stronger first choice's stale priority over another manager's still-actionable
/// relationship.
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
        let candidates =
            rank_candidates_by_relationship(registry.recruitment(), state, manager, candidates);
        if candidates.is_empty() {
            continue;
        }
        prepared.push(PreparedRecruitmentAuthority {
            mandate: mandate.id(),
            organization,
            manager,
            policy,
            approach: resolve_autonomous_recruitment_approach(manager_record),
            candidates,
        });
    }
    Ok(prepared)
}

/// Selects the strongest relationship that can still act in the current pass. Each authority's
/// candidate vector is already ordered by relationship support and CharacterId, so finding its
/// first live candidate is cheap and deterministic. The global comparison is repeated after every
/// action because a consumed prospect can expose a materially weaker fallback for one manager.
fn select_next_recruitment_action(
    state: &AppState,
    authorities: &[PreparedRecruitmentAuthority],
    claimed_this_pass: &BTreeSet<CharacterId>,
) -> Option<(usize, CharacterId)> {
    authorities
        .iter()
        .enumerate()
        .filter_map(|(index, authority)| {
            let candidate = authority.candidates.iter().find(|candidate| {
                !claimed_this_pass.contains(&candidate.character)
                    && state
                        .decisions()
                        .pending_for_recruitment_approval(
                            authority.organization,
                            candidate.character,
                        )
                        .is_none()
            })?;
            Some((
                (
                    Reverse(candidate.relationship_support),
                    authority.manager,
                    authority.organization,
                    authority.mandate,
                ),
                index,
                candidate.character,
            ))
        })
        .min_by_key(|(priority, _, _)| *priority)
        .map(|(_, index, candidate)| (index, candidate))
}

fn rank_candidates_by_relationship(
    definition: &RecruitmentDefinition,
    state: &AppState,
    recruiter: CharacterId,
    candidates: Vec<CharacterId>,
) -> Vec<RankedRecruitmentCandidate> {
    // Snapshot each relationship score exactly once for this read-only daily preparation pass.
    // Later same-pass contention changes candidate availability, not the relationship facts used
    // to rank that day's pitches.
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .map(|candidate| RankedRecruitmentCandidate {
            character: candidate,
            relationship_support: candidate_relationship_support(
                definition, state, recruiter, candidate,
            ),
        })
        .collect();
    ranked.sort_unstable_by_key(|candidate| {
        (Reverse(candidate.relationship_support), candidate.character)
    });
    ranked
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
