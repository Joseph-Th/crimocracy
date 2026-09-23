//! Daily delegated recruitment decisions; `recruitment_system` remains the canonical transaction owner.

use crate::core::attention::AttentionClass;
use crate::core::id::{
    CharacterId, DecisionRequestId, IdKind, MandateId, OrganizationId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::is_recurring_boundary;
use crate::decisions::decision_system::{
    DecisionError, DecisionRequestOutcome, ValidatedAutonomousRecruitmentApproval,
    ValidatedRecruitmentApprovalRequest, validate_request_recruitment_approval,
};
use crate::decisions::{DecisionResponse, RecruitmentApprovalRequestDraft};
use crate::delegation::delegation_system::{DelegationError, resolve_policy_for_manager};
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::recruitment::recruitment_system::{
    RecruitmentError, ValidatedRecruitmentAttempt, find_recruitment_candidates,
    validate_delegated_recruitment_attempt,
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
    candidates: Vec<RankedRecruitmentCandidate>,
}

enum PlannedAutonomousRecruitmentAction {
    Delegated(ValidatedRecruitmentAttempt),
    PlayerApproval(ValidatedRecruitmentApprovalRequest),
    AutonomousApproval(ValidatedAutonomousRecruitmentApproval),
}

struct AutonomousRecruitmentPlan {
    actions: Vec<PlannedAutonomousRecruitmentAction>,
    id_budget: Vec<(IdKind, u32)>,
}

/// Applies the authored recruitment cadence for delegated personnel managers. Candidate choice
/// is deterministic managerial judgment: strongest relationship support wins, with CharacterId
/// used only as an exact-score tie-breaker. This avoids both creation-order strategy and a random
/// manager choosing a visibly weaker relationship while preserving causal information boundaries.
pub(crate) fn apply_due_autonomous_recruitment(
    registry: &Registry,
    state: &mut AppState,
) -> Result<AutonomousRecruitmentOutcome, AutonomousRecruitmentError> {
    if !is_recurring_boundary(
        state.now(),
        registry.recruitment().autonomous_attempt_cadence(),
    ) {
        return Ok(AutonomousRecruitmentOutcome::default());
    }

    let plan = match plan_due_autonomous_recruitment(registry, state) {
        Ok(plan) => plan,
        Err(error) if autonomous_recruitment_is_terminally_blocked(&error) => {
            return Ok(AutonomousRecruitmentOutcome::default());
        }
        Err(error) => return Err(error),
    };
    // The daily personnel pass is one fallible cohort. Exact action selection and every
    // action-specific artifact budget were resolved above without mutation, so allocator
    // exhaustion cannot leave only the strongest prefix of managers recorded.
    if state.ids.reserve_many(&plan.id_budget).is_err() {
        return Ok(AutonomousRecruitmentOutcome::default());
    }
    Ok(plan.commit(state))
}

fn autonomous_recruitment_is_terminally_blocked(error: &AutonomousRecruitmentError) -> bool {
    use crate::decisions::decision_system::DecisionError;

    matches!(
        error,
        AutonomousRecruitmentError::Recruitment(RecruitmentError::IdExhaustion(_))
            | AutonomousRecruitmentError::Decision(DecisionError::IdExhaustion(_))
    )
}

fn plan_due_autonomous_recruitment(
    registry: &Registry,
    state: &AppState,
) -> Result<AutonomousRecruitmentPlan, AutonomousRecruitmentError> {
    let personnel_scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let mut authorities = prepare_recruitment_authorities(registry, state, personnel_scope)?;
    let mut claimed_this_pass = BTreeSet::new();
    let mut actions = Vec::new();
    let mut id_budget = Vec::new();
    let mut decision_count = 0_u32;

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
            candidates: _,
        } = prepared;
        let manager_record = state
            .world()
            .get_character(manager)
            .ok_or(RecruitmentError::MissingRecruiter(manager))?;
        let approach = resolve_autonomous_recruitment_approach(manager_record);
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
        };

        match policy {
            ApprovalPolicy::Delegated => {
                let validated = validate_delegated_recruitment_attempt(
                    registry,
                    state,
                    authority,
                    RecruitmentDraft {
                        target_organization: organization,
                        recruiter: manager,
                        candidate,
                        approach,
                    },
                )?;
                id_budget.extend(validated.id_budget());
                actions.push(PlannedAutonomousRecruitmentAction::Delegated(validated));
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
                        summary: approval_request_summary(state, manager, candidate, approach),
                    },
                )?;
                decision_count = decision_count
                    .checked_add(1)
                    .expect("persisted mandate count must fit the u32 decision ID space");
                // Besides proving this request count is representable, the check makes the
                // predicted future decision ID below safe to derive without wrapping.
                state
                    .ids
                    .reserve(IdKind::DecisionRequest, decision_count)
                    .map_err(DecisionError::from)?;
                if state.player_organization() == Some(organization) {
                    id_budget.extend(request.id_budget());
                    actions.push(PlannedAutonomousRecruitmentAction::PlayerApproval(request));
                } else {
                    let predicted_decision = DecisionRequestId::from_raw(
                        state
                            .ids
                            .next_raw(IdKind::DecisionRequest)
                            .checked_add(decision_count - 1)
                            .expect("decision-count reserve proved the predicted ID representable"),
                    );
                    let prepared = request.prepare_autonomous_resolution(
                        registry,
                        state,
                        DecisionResponse::Approve,
                        predicted_decision,
                    )?;
                    id_budget.extend(prepared.id_budget());
                    actions.push(PlannedAutonomousRecruitmentAction::AutonomousApproval(
                        prepared,
                    ));
                }
            }
        }
        // Every selected route either records an attempt immediately or owns the candidate in a
        // pending player decision. The read-only planner therefore models the existing same-pass
        // contention rule exactly without needing a speculative AppState clone.
        claimed_this_pass.insert(candidate);
    }
    Ok(AutonomousRecruitmentPlan { actions, id_budget })
}

impl AutonomousRecruitmentPlan {
    fn commit(self, state: &mut AppState) -> AutonomousRecruitmentOutcome {
        let mut outcome = AutonomousRecruitmentOutcome::default();
        for action in self.actions {
            match action {
                PlannedAutonomousRecruitmentAction::Delegated(attempt) => {
                    outcome.attempts.push(attempt.commit_preflighted(state));
                }
                PlannedAutonomousRecruitmentAction::PlayerApproval(request) => {
                    outcome
                        .approval_requests
                        .push(request.commit_preflighted(state));
                }
                PlannedAutonomousRecruitmentAction::AutonomousApproval(prepared) => {
                    let (request, resolution) = prepared.commit_preflighted(state);
                    outcome.approval_requests.push(request);
                    if let Some(attempt) = resolution.recruitment_attempt {
                        outcome.attempts.push(attempt);
                    }
                }
            }
        }
        outcome
    }
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
            candidates,
        });
    }
    Ok(prepared)
}

/// Selects the strongest relationship that can still act in the current pass. Each authority's
/// candidate vector is already ordered by relationship support and CharacterId, so finding its
/// first live candidate is cheap and deterministic. The global comparison is repeated after every
/// action because a consumed prospect can expose a materially weaker fallback for one manager.
/// Highest relationship support acts first; manager, organization, and mandate IDs break only
/// exact ties, so cross-organization contention is deterministic without favoring any side.
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
                    candidate.relationship_support,
                    Reverse(authority.manager),
                    Reverse(authority.organization),
                    Reverse(authority.mandate),
                ),
                index,
                candidate.character,
            ))
        })
        .max_by_key(|(priority, _, _)| *priority)
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
    approach: RecruitmentApproach,
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
    format!(
        "{recruiter_name} seeks approval to approach {candidate_name} with a {approach:?} recruitment pitch."
    )
}

fn resolve_autonomous_recruitment_approach(
    manager: &crate::world::CharacterRecord,
) -> RecruitmentApproach {
    // Autonomous choice may use only facts available to the acting manager. Candidate drives and
    // traits are latent character state, not recruiter knowledge merely because a relationship
    // edge exists. They still affect the candidate's canonical willingness calculation after the
    // manager chooses a pitch, while the manager's own disposition determines what they try.
    manager_recruitment_preference(manager)
}

fn manager_recruitment_preference(manager: &crate::world::CharacterRecord) -> RecruitmentApproach {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::CharacterDraft;
    use crate::world::world_system::insert_character;
    use std::collections::BTreeMap;

    #[test]
    fn autonomous_proud_manager_prefers_advancement_without_ambition() {
        let mut state = AppState::new(17);
        let manager = insert_character(
            &mut state,
            CharacterDraft {
                name: "Proud Manager".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::from([TraitKind::Proud]),
                drives: BTreeMap::new(),
            },
        )
        .expect("standalone manager fixture should insert");
        let manager = state
            .world()
            .get_character(manager)
            .expect("inserted manager should persist");

        assert_eq!(
            manager_recruitment_preference(manager),
            RecruitmentApproach::Advancement
        );
    }
}
