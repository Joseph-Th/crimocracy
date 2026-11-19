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
    recruitment_approval_is_deferred, validate_delegated_recruitment_attempt,
};
use crate::recruitment::scoring::recruitment_relationship_support;
use crate::recruitment::{
    ALL_RECRUITMENT_APPROACHES, RecruitmentApproach, RecruitmentDraft, RecruitmentOutcome,
};
use crate::registry::{RecruitmentDefinition, Registry};
use crate::world::{ApprovalPolicy, AutonomyLevel, PolicyKind, PolicySetting, TraitKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
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
    approach: RecruitmentApproach,
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

enum RecruitmentPlanningResult {
    Ready(AutonomousRecruitmentPlan),
    Contested(IdKind),
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

    let plan = match plan_viable_autonomous_recruitment(registry, state) {
        Ok(plan) => plan,
        Err(error) if autonomous_recruitment_is_terminally_blocked(&error) => {
            return Ok(AutonomousRecruitmentOutcome::default());
        }
        Err(error) => return Err(error),
    };
    // Individual terminally unallocatable routes were excluded during planning. The final
    // cohort has already passed its complete aggregate ID reservation without mutation.
    Ok(plan.commit(state))
}

/// Shared ID scarcity cannot award a contested final artifact to an arbitrary manager, or
/// suppress managers whose route needs no such artifact. Retry against the same immutable state
/// with only the contested ID kinds excluded. Every retry releases provisional candidate claims,
/// and each exclusion makes progress through the finite set of ID kinds.
fn plan_viable_autonomous_recruitment(
    registry: &Registry,
    state: &AppState,
) -> Result<AutonomousRecruitmentPlan, AutonomousRecruitmentError> {
    let mut contested = BTreeSet::new();
    loop {
        match plan_due_autonomous_recruitment(registry, state, &contested)? {
            RecruitmentPlanningResult::Ready(plan) => {
                if let Some(kind) = first_unavailable_cohort_id_kind(state, &plan.id_budget) {
                    contested.insert(kind);
                } else {
                    return Ok(plan);
                }
            }
            RecruitmentPlanningResult::Contested(kind) => {
                contested.insert(kind);
            }
        }
    }
}

/// The finance-style cohort reservation uses typed ID kinds, never error display text, to
/// isolate a contested resource. Preserve the allocator's aggregate checked arithmetic.
fn first_unavailable_cohort_id_kind(state: &AppState, budget: &[(IdKind, u32)]) -> Option<IdKind> {
    let mut totals = BTreeMap::<IdKind, u32>::new();
    for (kind, count) in budget {
        let total = totals.entry(*kind).or_insert(0);
        let Some(next) = total.checked_add(*count) else {
            return Some(*kind);
        };
        *total = next;
    }
    totals
        .into_iter()
        .find_map(|(kind, total)| state.ids.reserve(kind, total).is_err().then_some(kind))
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
    contested: &BTreeSet<IdKind>,
) -> Result<RecruitmentPlanningResult, AutonomousRecruitmentError> {
    let personnel_scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let mut authorities = prepare_recruitment_authorities(registry, state, personnel_scope)?;
    let id_available = |kind| !contested.contains(&kind) && state.ids.reserve(kind, 1).is_ok();
    let approval_available = id_available(IdKind::DecisionRequest);
    let attempt_available =
        id_available(IdKind::RecruitmentAttempt) && id_available(IdKind::Information);
    authorities.retain(|authority| {
        let needs_approval = authority.policy == ApprovalPolicy::RequireApproval;
        let needs_attempt =
            !needs_approval || state.player_organization() != Some(authority.organization);
        (!needs_approval || approval_available) && (!needs_attempt || attempt_available)
    });
    let mut claimed_this_pass = BTreeSet::new();
    let mut actions = Vec::new();
    let mut id_budget = Vec::new();
    let mut decision_count = 0_u32;

    while let Some((authority_index, candidate, approach)) =
        select_next_recruitment_action(&authorities, &claimed_this_pass)
    {
        let prepared = &authorities[authority_index];
        let (mandate, organization, manager, policy) = (
            prepared.mandate,
            prepared.organization,
            prepared.manager,
            prepared.policy,
        );
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
        };
        let next_decision_count = decision_count
            .checked_add(u32::from(policy == ApprovalPolicy::RequireApproval))
            .expect("persisted mandate count must fit the u32 decision ID space");
        let decisions_fit = state
            .ids
            .reserve(IdKind::DecisionRequest, next_decision_count)
            .is_ok();

        let action = match policy {
            ApprovalPolicy::Delegated => PlannedAutonomousRecruitmentAction::Delegated(
                validate_delegated_recruitment_attempt(
                    registry,
                    state,
                    authority,
                    RecruitmentDraft {
                        target_organization: organization,
                        recruiter: manager,
                        candidate,
                        approach,
                    },
                )?,
            ),
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
                if state.player_organization() == Some(organization) {
                    PlannedAutonomousRecruitmentAction::PlayerApproval(request)
                } else {
                    // Even if the cohort cannot fit another decision, check whether this
                    // pitch is viable in isolation before declaring shared contention.
                    // The first free ID is valid for that read-only probe; its token is
                    // discarded unless the complete decision cohort fits.
                    let offset = if decisions_fit { decision_count } else { 0 };
                    let predicted_decision = DecisionRequestId::from_raw(
                        state
                            .ids
                            .next_raw(IdKind::DecisionRequest)
                            .checked_add(offset)
                            .expect("decision reservation proved the predicted ID representable"),
                    );
                    PlannedAutonomousRecruitmentAction::AutonomousApproval(
                        request.prepare_autonomous_resolution(
                            registry,
                            state,
                            DecisionResponse::Approve,
                            predicted_decision,
                        )?,
                    )
                }
            }
        };
        let action_budget = action.id_budget();
        // Only viable pitches can claim prospects or contend for shared capacity. A blocked
        // prospect does not spend the manager's action: reconsider alternatives in global order.
        if state.ids.reserve_many(&action_budget).is_err()
            || action_budget
                .iter()
                .any(|(kind, _)| contested.contains(kind))
        {
            authorities[authority_index]
                .candidates
                .retain(|prospect| prospect.character != candidate);
            continue;
        }
        if !decisions_fit {
            return Ok(RecruitmentPlanningResult::Contested(
                IdKind::DecisionRequest,
            ));
        }
        decision_count = next_decision_count;
        id_budget.extend(action_budget);
        actions.push(action);
        claimed_this_pass.insert(candidate);
        authorities.remove(authority_index);
    }
    Ok(RecruitmentPlanningResult::Ready(
        AutonomousRecruitmentPlan { actions, id_budget },
    ))
}

impl PlannedAutonomousRecruitmentAction {
    fn id_budget(&self) -> Vec<(IdKind, u32)> {
        match self {
            Self::Delegated(attempt) => attempt.id_budget(),
            Self::PlayerApproval(request) => request.id_budget(),
            Self::AutonomousApproval(prepared) => prepared.id_budget(),
        }
    }
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
        // The shared recruitment query already filters pending organization-candidate approvals
        // through the world's canonical reassignment guard. Do not maintain a second, subtly
        // different availability policy here.
        let mut candidates = find_recruitment_candidates(registry, state, organization, manager)?;
        if policy == ApprovalPolicy::RequireApproval {
            candidates.retain(|candidate| {
                !recruitment_approval_is_deferred(registry, state, organization, *candidate)
            });
        }
        let candidates = rank_candidates_by_relationship(
            registry.recruitment(),
            state,
            organization,
            manager_record,
            candidates,
        );
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
    authorities: &[PreparedRecruitmentAuthority],
    claimed_this_pass: &BTreeSet<CharacterId>,
) -> Option<(usize, CharacterId, RecruitmentApproach)> {
    authorities
        .iter()
        .enumerate()
        .filter_map(|(index, authority)| {
            // Candidate discovery already excluded every pending approval in this immutable
            // planning snapshot. Only same-pass claims can change eligibility as one manager
            // consumes a stronger manager's first-choice prospect.
            let candidate = authority
                .candidates
                .iter()
                .find(|candidate| !claimed_this_pass.contains(&candidate.character))?;
            Some((
                (
                    candidate.relationship_support,
                    Reverse(authority.manager),
                    Reverse(authority.organization),
                    Reverse(authority.mandate),
                ),
                index,
                candidate.character,
                candidate.approach,
            ))
        })
        .max_by_key(|(priority, _, _, _)| *priority)
        .map(|(_, index, candidate, approach)| (index, candidate, approach))
}

fn rank_candidates_by_relationship(
    definition: &RecruitmentDefinition,
    state: &AppState,
    organization: OrganizationId,
    recruiter: &crate::world::CharacterRecord,
    candidates: Vec<CharacterId>,
) -> Vec<RankedRecruitmentCandidate> {
    // Snapshot each relationship score exactly once for this read-only daily preparation pass.
    // Later same-pass contention changes candidate availability, not the relationship facts used
    // to rank that day's pitches.
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let approach =
                resolve_prospect_pitch(definition, state, organization, recruiter, candidate)?;
            Some(RankedRecruitmentCandidate {
                character: candidate,
                relationship_support: candidate_relationship_support(
                    definition,
                    state,
                    recruiter.id(),
                    candidate,
                ),
                approach,
            })
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

/// A refusal is actionable feedback, not an excuse to ask the executive to authorize
/// the same deterministic pitch every week. Reconsider unused approaches within a bounded
/// retry window. If all four were refused under the same relationship, let the prospect
/// rest until an old refusal ages out or the relationship changes. Only own attempt history
/// and the existing relationship are consulted, never the candidate's latent willingness.
fn resolve_prospect_pitch(
    definition: &RecruitmentDefinition,
    state: &AppState,
    organization: OrganizationId,
    manager: &crate::world::CharacterRecord,
    candidate: CharacterId,
) -> Option<RecruitmentApproach> {
    let preferred = resolve_autonomous_recruitment_approach(manager);
    let relationship = state
        .social()
        .get_relationship(candidate, manager.id())
        .expect("ranked prospect must have a relationship");
    let retry_window = u64::from(definition.cooldown().as_minutes())
        * (ALL_RECRUITMENT_APPROACHES.len() as u64 * 2);
    let mut refused = BTreeSet::new();
    for attempt in state
        .recruitment()
        .attempts_for_prospect(candidate, organization)
    {
        if state
            .now()
            .as_minutes()
            .saturating_sub(attempt.occurred_at().as_minutes())
            >= retry_window
        {
            break;
        }
        if attempt.recruiter() != manager.id() {
            continue;
        }
        if attempt.outcome() == RecruitmentOutcome::Accepted
            || attempt.recruiter_relationship().version() != Some(relationship.version())
        {
            break;
        }
        refused.insert(attempt.approach());
        if refused.len() == ALL_RECRUITMENT_APPROACHES.len() {
            return None;
        }
    }
    std::iter::once(preferred)
        .chain(ALL_RECRUITMENT_APPROACHES)
        .find(|approach| !refused.contains(approach))
}

fn resolve_autonomous_recruitment_approach(
    manager: &crate::world::CharacterRecord,
) -> RecruitmentApproach {
    // Autonomous choice may use only facts available to the acting manager. Candidate drives and
    // traits are latent character state, not recruiter knowledge merely because a relationship
    // edge exists. They still affect the candidate's canonical willingness calculation after the
    // manager chooses a pitch, while the manager's own disposition determines what they try.
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
            resolve_autonomous_recruitment_approach(manager),
            RecruitmentApproach::Advancement
        );
    }
}
