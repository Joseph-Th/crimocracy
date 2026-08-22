//! Relationship-gated recruitment decisions with causal factors, cooldowns, and atomic accepted membership changes.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, DecisionRequestId, IdExhaustionError, IdKind, InformationId,
    OrganizationId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::delegation::delegation_system::{
    ensure_mandate_authority_current, resolve_mandate_authority, resolve_policy_for_manager,
    DelegationError, PolicySource, ResolvedPolicy,
};
use crate::delegation::{
    MandateAuthority, ResolvedMandateAuthority, ResponsibilityFunction, ResponsibilityScope,
};
use crate::history::history_system::{validate_record_event, HistoryError, ValidatedHistoryEvent};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSourceKind, InformationTopic, KnowledgeHolder,
    Reliability, Specificity,
};
use crate::recruitment::{
    build_recruitment_factors, build_recruitment_record, build_recruitment_relationship_snapshot,
    RecruitmentApproach, RecruitmentAuthority, RecruitmentDraft, RecruitmentFactorComponents,
    RecruitmentFactors, RecruitmentOutcome, RecruitmentPolicySource, RecruitmentRecordContextParts,
    RecruitmentRecordParts, RecruitmentRecordResolutionParts, RecruitmentRelationshipSnapshot,
};
use crate::registry::{RecruitmentDefinition, Registry};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::social::RelationshipDimensions;
use crate::world::world_system::{
    validate_reassign_character, ValidatedCharacterReassignment, WorldError,
};
use crate::world::{
    ApprovalPolicy, AutonomyLevel, DriveKind, Lifecycle, OrganizationKind, PolicyKind,
    PolicySetting, TraitKind,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RecruitmentError {
    #[error("target organization {0} does not exist")]
    MissingTargetOrganization(OrganizationId),
    #[error("target organization {0} is not active")]
    InactiveTargetOrganization(OrganizationId),
    #[error("target organization {0} is not a criminal organization")]
    InvalidTargetOrganizationKind(OrganizationId),
    #[error("recruiter {0} does not exist")]
    MissingRecruiter(CharacterId),
    #[error("recruiter {0} is not active")]
    InactiveRecruiter(CharacterId),
    #[error("recruiter {recruiter} is detained under arrest {arrest}")]
    DetainedRecruiter {
        recruiter: CharacterId,
        arrest: ArrestId,
    },
    #[error("recruiter {recruiter} is not a member of target organization {organization}")]
    RecruiterOrganizationMismatch {
        recruiter: CharacterId,
        organization: OrganizationId,
    },
    #[error("executive recruitment is reserved for organization heads; recruiter {recruiter} reports to {supervisor}")]
    ExecutiveRecruiterSupervised {
        recruiter: CharacterId,
        supervisor: CharacterId,
    },
    #[error("recruitment approval decision {decision} is already pending for this candidate")]
    PendingRecruitmentApproval { decision: DecisionRequestId },
    #[error("candidate {0} does not exist")]
    MissingCandidate(CharacterId),
    #[error("candidate {0} is not active")]
    InactiveCandidate(CharacterId),
    #[error("a character cannot recruit themselves")]
    SelfRecruitment,
    #[error("candidate {candidate} is already a member of target organization {organization}")]
    CandidateAlreadyMember {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error("candidate {candidate} belongs to organization {organization}, which requires a different personnel system")]
    CandidateOrganizationNotRecruitable {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error("candidate {candidate} has no relationship edge to recruiter {recruiter}")]
    NoRecruitmentRelationship {
        candidate: CharacterId,
        recruiter: CharacterId,
    },
    #[error("candidate {candidate} cannot be approached again by organization {organization} before {next_eligible_at:?}")]
    Cooldown {
        candidate: CharacterId,
        organization: OrganizationId,
        next_eligible_at: SimTime,
    },
    #[error("recruitment plan was decided at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("candidate {candidate} changed after recruitment was decided; expected version {expected}, found {found}")]
    StaleCandidate {
        candidate: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("recruiter {recruiter} changed after recruitment was decided; expected version {expected}, found {found}")]
    StaleRecruiter {
        recruiter: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("relationship {from}->{to} changed after recruitment was decided; expected version {expected:?}, found {found:?}")]
    StaleRelationship {
        from: CharacterId,
        to: CharacterId,
        expected: Option<u32>,
        found: Option<u32>,
    },
    #[error("recruitment history for candidate {candidate} and organization {organization} changed after the plan was decided")]
    StaleRecruitmentHistory {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error(
        "candidate {candidate} legal-pressure knowledge changed after recruitment was decided"
    )]
    StalePressureKnowledge { candidate: CharacterId },
    #[error("delegated recruitment requires recruiter {recruiter} to be the authority manager {manager}")]
    DelegatedRecruiterMismatch {
        recruiter: CharacterId,
        manager: CharacterId,
    },
    #[error("delegated recruitment requires Personnel scope, not {scope:?}")]
    DelegatedRecruitmentRequiresPersonnelScope { scope: ResponsibilityScope },
    #[error("delegated recruitment authority belongs to organization {authority_organization}, not target {target_organization}")]
    DelegatedOrganizationMismatch {
        authority_organization: OrganizationId,
        target_organization: OrganizationId,
    },
    #[error("manager {manager} cannot recruit independently under policy {policy:?}")]
    IndependentRecruitmentNotDelegated {
        manager: CharacterId,
        policy: ApprovalPolicy,
    },
    #[error("manager {manager} cannot use an approval decision under policy {policy:?}")]
    IndependentRecruitmentApprovalNotRequired {
        manager: CharacterId,
        policy: ApprovalPolicy,
    },
    #[error("independent recruitment policy changed after validation")]
    StaleRecruitmentPolicy,
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    World(#[from] WorldError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Copy, Debug)]
struct MandateRecruitmentGuard {
    authority: ResolvedMandateAuthority,
    policy: ResolvedPolicy,
    required_policy: ApprovalPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecruitmentPlan {
    draft: RecruitmentDraft,
    context: RecruitmentPlanContext,
    dependencies: RecruitmentPlanDependencies,
}

#[derive(Debug)]
pub(crate) struct ValidatedRecruitmentProposal {
    plan: RecruitmentPlan,
}

impl ValidatedRecruitmentProposal {
    pub(crate) fn revalidate_state(&self, state: &AppState) -> Result<(), RecruitmentError> {
        validate_plan_state_snapshot(state, &self.plan)?;
        if self.plan.context.outcome == RecruitmentOutcome::Accepted {
            validate_reassign_character(
                state,
                self.plan.draft.candidate,
                Some(self.plan.draft.target_organization),
                Some(self.plan.draft.recruiter),
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecruitmentPlanContext {
    previous_organization: Option<OrganizationId>,
    previous_supervisor: Option<CharacterId>,
    occurred_at: SimTime,
    factors: RecruitmentFactors,
    margin: i16,
    outcome: RecruitmentOutcome,
    pressure_information: Option<InformationId>,
}

pub(crate) fn validate_recruitment_proposal(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentDraft,
) -> Result<ValidatedRecruitmentProposal, RecruitmentError> {
    let plan = decide_recruitment_attempt(registry, state, draft)?;
    validate_plan_state_snapshot(state, &plan)?;
    validate_plan_definition(registry.recruitment(), state, &plan)?;
    if plan.context.outcome == RecruitmentOutcome::Accepted {
        validate_reassign_character(
            state,
            plan.draft.candidate,
            Some(plan.draft.target_organization),
            Some(plan.draft.recruiter),
        )?;
    }
    Ok(ValidatedRecruitmentProposal { plan })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecruitmentPlanDependencies {
    expected_candidate_version: u32,
    expected_recruiter_version: u32,
    recruiter_relationship: RecruitmentRelationshipSnapshot,
    incumbent_relationship: Option<RecruitmentRelationshipSnapshot>,
    pressure_information_snapshot: BTreeSet<InformationId>,
    expected_latest_attempt: Option<RecruitmentAttemptId>,
}

pub fn find_recruitment_candidates(
    registry: &Registry,
    state: &AppState,
    target_organization: OrganizationId,
    recruiter: CharacterId,
) -> Result<Vec<CharacterId>, RecruitmentError> {
    validate_target_and_recruiter(state, target_organization, recruiter)?;
    let mut candidates = Vec::new();
    for relationship in state.social.relationships_to(recruiter) {
        let candidate = relationship.from();
        let Some(record) = state.world.get_character(candidate) else {
            continue;
        };
        if record.lifecycle() != Lifecycle::Active
            || record.organization() == Some(target_organization)
            || !candidate_organization_is_recruitable(state, record.organization())
            || recruitment_is_on_cooldown(
                registry.recruitment(),
                state,
                candidate,
                target_organization,
            )
            || validate_reassign_character(
                state,
                candidate,
                Some(target_organization),
                Some(recruiter),
            )
            .is_err()
        {
            continue;
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

pub(crate) fn resolve_due_autonomous_recruitment(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<RecruitmentAttemptId>, RecruitmentError> {
    let cadence = u64::from(
        registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes(),
    );
    if state.now() == SimTime::ZERO || !state.now().as_minutes().is_multiple_of(cadence) {
        return Ok(Vec::new());
    }

    let personnel_scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let authorities: Vec<_> = state
        .delegation()
        .active_for_scope(personnel_scope)
        .map(|mandate| (mandate.id(), mandate.organization(), mandate.manager()))
        .collect();
    let mut attempts = Vec::new();
    for (mandate, organization, manager) in authorities {
        let Some(manager_record) = state.world().get_character(manager) else {
            continue;
        };
        if state.legal().active_arrest_for_character(manager).is_some() {
            continue;
        }
        if !matches!(
            manager_record.autonomy(),
            AutonomyLevel::Delegated | AutonomyLevel::Broad
        ) {
            continue;
        }
        // A single mandate that currently cannot resolve (policy, candidate availability, or a
        // transient commit rejection) must not abort every other mandate's due work in the same
        // minute: each eligible authority is evaluated independently.
        let policy =
            match resolve_policy_for_manager(state, manager, PolicyKind::IndependentRecruitment) {
                Ok(policy) => policy,
                Err(_) => continue,
            };
        if policy.setting != PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated) {
            continue;
        }
        let approach = autonomous_recruitment_approach(manager_record);
        let mut candidates =
            find_recruitment_candidates(registry, state, organization, manager).unwrap_or_default();
        if candidates.is_empty() {
            continue;
        }
        let candidate = {
            // Stabilize order before the RNG draw so determinism does not depend on BTree
            // iteration quirks; the drawn index must address the sorted candidate list.
            candidates.sort_unstable();
            let index =
                draw_candidate_index(state.recruitment_rng_mut(), candidates.len()).unwrap_or(0);
            candidates[index]
        };
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
        };
        let attempt = match validate_delegated_recruitment_attempt(
            registry,
            state,
            authority,
            RecruitmentDraft {
                target_organization: organization,
                recruiter: manager,
                candidate,
                approach,
            },
        ) {
            Ok(validated) => match validated.commit(state) {
                Ok(attempt) => attempt,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        attempts.push(attempt);
    }
    Ok(attempts)
}

fn autonomous_recruitment_approach(manager: &crate::world::CharacterRecord) -> RecruitmentApproach {
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

pub(crate) fn decide_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentDraft,
) -> Result<RecruitmentPlan, RecruitmentError> {
    let (candidate, recruiter) = validate_recruitment_request(registry, state, draft)?;
    let recruiter_relationship = state
        .social
        .get_relationship(draft.candidate, draft.recruiter)
        .expect("validated recruitment relationship must exist");
    let incumbent_relationship = candidate.supervisor().map(|supervisor| {
        let relationship = state.social.get_relationship(draft.candidate, supervisor);
        build_recruitment_relationship_snapshot(
            draft.candidate,
            supervisor,
            relationship.map(|record| record.dimensions()),
            relationship.map(|record| record.version()),
        )
    });
    let recruiter_relationship = build_recruitment_relationship_snapshot(
        draft.candidate,
        draft.recruiter,
        Some(recruiter_relationship.dimensions()),
        Some(recruiter_relationship.version()),
    );
    let pressure_information_snapshot =
        candidate_pressure_information_ids(state, draft.candidate, state.now());
    let (pressure_information, perceived_legal_pressure) = select_perceived_legal_pressure_at(
        registry.recruitment(),
        state,
        draft.candidate,
        state.now(),
    );
    let factors = resolve_recruitment_factors_from_context(RecruitmentFactorContext {
        definition: registry.recruitment(),
        candidate,
        recruiter,
        approach: draft.approach,
        recruiter_relationship,
        incumbent_relationship,
        perceived_legal_pressure,
        had_previous_organization: candidate.organization().is_some(),
    })
    .expect("validated recruitment must retain a candidate-to-recruiter relationship snapshot");
    let margin = resolve_recruitment_margin(registry.recruitment(), factors, draft.approach);
    let outcome = classify_recruitment_outcome(margin);
    Ok(RecruitmentPlan {
        draft,
        context: RecruitmentPlanContext {
            previous_organization: candidate.organization(),
            previous_supervisor: candidate.supervisor(),
            occurred_at: state.now(),
            factors,
            margin,
            outcome,
            pressure_information,
        },
        dependencies: RecruitmentPlanDependencies {
            expected_candidate_version: candidate.version(),
            expected_recruiter_version: recruiter.version(),
            recruiter_relationship,
            incumbent_relationship,
            pressure_information_snapshot,
            expected_latest_attempt: state
                .recruitment
                .latest_attempt_for(draft.candidate, draft.target_organization)
                .map(|attempt| attempt.id()),
        },
    })
}

pub fn validate_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentDraft,
) -> Result<ValidatedRecruitmentAttempt, RecruitmentError> {
    // The executive channel bypasses manager recruitment policies, so it is reserved for
    // organization heads; supervised members recruit through delegated or approved channels.
    let recruiter_record = state
        .world
        .get_character(draft.recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(draft.recruiter))?;
    if let Some(supervisor) = recruiter_record.supervisor() {
        return Err(RecruitmentError::ExecutiveRecruiterSupervised {
            recruiter: draft.recruiter,
            supervisor,
        });
    }
    if let Some(pending) = state.decisions().pending_for_recruitment_approval(
        draft.target_organization,
        draft.recruiter,
        draft.candidate,
    ) {
        // An approval request already covers this candidate; a direct attempt must not race it.
        return Err(RecruitmentError::PendingRecruitmentApproval { decision: pending });
    }
    validate_recruitment_plan_with_authority(
        registry,
        state,
        decide_recruitment_attempt(registry, state, draft)?,
        RecruitmentAuthority::ExecutiveApproval,
        None,
    )
}

/// Shared authority prelude for mandate-backed recruitment channels: manager identity, personnel
/// scope, organization match, and the manager's independent-recruitment policy.
fn validate_personnel_authority(
    state: &AppState,
    authority: MandateAuthority,
    draft: &RecruitmentDraft,
) -> Result<
    (
        ResolvedMandateAuthority,
        crate::delegation::delegation_system::ResolvedPolicy,
    ),
    RecruitmentError,
> {
    if authority.manager != draft.recruiter {
        return Err(RecruitmentError::DelegatedRecruiterMismatch {
            recruiter: draft.recruiter,
            manager: authority.manager,
        });
    }
    if authority.scope != ResponsibilityScope::Function(ResponsibilityFunction::Personnel) {
        return Err(
            RecruitmentError::DelegatedRecruitmentRequiresPersonnelScope {
                scope: authority.scope,
            },
        );
    }
    let resolved_authority = resolve_mandate_authority(state, authority)?;
    if resolved_authority.organization() != draft.target_organization {
        return Err(RecruitmentError::DelegatedOrganizationMismatch {
            authority_organization: resolved_authority.organization(),
            target_organization: draft.target_organization,
        });
    }
    let policy =
        resolve_policy_for_manager(state, authority.manager, PolicyKind::IndependentRecruitment)?;
    Ok((resolved_authority, policy))
}

pub fn validate_delegated_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    authority: MandateAuthority,
    draft: RecruitmentDraft,
) -> Result<ValidatedRecruitmentAttempt, RecruitmentError> {
    let (resolved_authority, policy) = validate_personnel_authority(state, authority, &draft)?;
    let approval = policy.independent_recruitment_approval();
    if approval != ApprovalPolicy::Delegated {
        return Err(RecruitmentError::IndependentRecruitmentNotDelegated {
            manager: authority.manager,
            policy: approval,
        });
    }
    let persisted_authority = RecruitmentAuthority::Delegated {
        mandate: authority.mandate,
        manager: authority.manager,
        scope: authority.scope,
        mandate_version: resolved_authority.mandate_version(),
        manager_version: resolved_authority.manager_version(),
        policy: approval,
        policy_source: recruitment_policy_source(policy.source),
    };
    validate_recruitment_plan_with_authority(
        registry,
        state,
        decide_recruitment_attempt(registry, state, draft)?,
        persisted_authority,
        Some(MandateRecruitmentGuard {
            authority: resolved_authority,
            policy,
            required_policy: ApprovalPolicy::Delegated,
        }),
    )
}

pub(crate) fn validate_approved_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    decision: DecisionRequestId,
    authority: MandateAuthority,
    draft: RecruitmentDraft,
) -> Result<ValidatedRecruitmentAttempt, RecruitmentError> {
    let (resolved_authority, policy) = validate_personnel_authority(state, authority, &draft)?;
    let approval = policy.independent_recruitment_approval();
    if approval != ApprovalPolicy::RequireApproval {
        return Err(
            RecruitmentError::IndependentRecruitmentApprovalNotRequired {
                manager: authority.manager,
                policy: approval,
            },
        );
    }
    let persisted_authority = RecruitmentAuthority::ApprovedDecision {
        decision,
        mandate: authority.mandate,
        manager: authority.manager,
        scope: authority.scope,
        mandate_version: resolved_authority.mandate_version(),
        manager_version: resolved_authority.manager_version(),
        policy: approval,
        policy_source: recruitment_policy_source(policy.source),
    };
    validate_recruitment_plan_with_authority(
        registry,
        state,
        decide_recruitment_attempt(registry, state, draft)?,
        persisted_authority,
        Some(MandateRecruitmentGuard {
            authority: resolved_authority,
            policy,
            required_policy: ApprovalPolicy::RequireApproval,
        }),
    )
}

fn validate_recruitment_plan_with_authority(
    registry: &Registry,
    state: &AppState,
    plan: RecruitmentPlan,
    authority: RecruitmentAuthority,
    delegated_guard: Option<MandateRecruitmentGuard>,
) -> Result<ValidatedRecruitmentAttempt, RecruitmentError> {
    validate_plan_state_snapshot(state, &plan)?;
    validate_plan_definition(registry.recruitment(), state, &plan)?;
    let reassignment = if plan.context.outcome == RecruitmentOutcome::Accepted {
        Some(validate_reassign_character(
            state,
            plan.draft.candidate,
            Some(plan.draft.target_organization),
            Some(plan.draft.recruiter),
        )?)
    } else {
        None
    };
    let history = if plan.context.outcome == RecruitmentOutcome::Accepted {
        let candidate = state
            .world
            .get_character(plan.draft.candidate)
            .expect("validated recruitment candidate must exist");
        let recruiter = state
            .world
            .get_character(plan.draft.recruiter)
            .expect("validated recruiter must exist");
        let organization = state
            .world
            .get_organization(plan.draft.target_organization)
            .expect("validated target organization must exist");
        // When the candidate is poached from another organization, campaign history must not leak
        // the hidden recruiting organization: the defector's former organization is told only that
        // the member left, and the player discovers the destination through surveillance, not a
        // global history read. So a defection event omits the organization entity and its name.
        if plan.context.previous_organization.is_some() {
            Some(validate_record_event(
                state,
                HistoryEventDraft {
                    occurred_at: plan.context.occurred_at,
                    kind: HistoryEventKind::Recruitment,
                    summary: format!(
                        "{} joined after recruitment by {}.",
                        candidate.name(),
                        recruiter.name()
                    ),
                    entities: BTreeSet::from([
                        EntityRef::Character(plan.draft.candidate),
                        EntityRef::Character(plan.draft.recruiter),
                    ]),
                },
            )?)
        } else {
            Some(validate_record_event(
                state,
                HistoryEventDraft {
                    occurred_at: plan.context.occurred_at,
                    kind: HistoryEventKind::Recruitment,
                    summary: format!(
                        "{} joined {} after recruitment by {}.",
                        candidate.name(),
                        organization.name(),
                        recruiter.name()
                    ),
                    entities: BTreeSet::from([
                        EntityRef::Character(plan.draft.candidate),
                        EntityRef::Character(plan.draft.recruiter),
                        EntityRef::Organization(plan.draft.target_organization),
                    ]),
                },
            )?)
        }
    } else {
        None
    };
    let candidate = state
        .world
        .get_character(plan.draft.candidate)
        .expect("validated recruitment candidate must exist");
    let recruiter = state
        .world
        .get_character(plan.draft.recruiter)
        .expect("validated recruiter must exist");
    let organization = state
        .world
        .get_organization(plan.draft.target_organization)
        .expect("validated target organization must exist");
    let outcome_information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(plan.draft.target_organization),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::Personnel,
            source_entity: Some(EntityRef::Character(plan.draft.recruiter)),
            subject: EntityRef::Character(plan.draft.candidate),
            observed_at: plan.context.occurred_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: match plan.context.outcome {
                RecruitmentOutcome::Accepted => format!(
                    "{} accepted {}'s recruitment approach and joined {}.",
                    candidate.name(),
                    recruiter.name(),
                    organization.name()
                ),
                RecruitmentOutcome::Refused => format!(
                    "{} refused {}'s recruitment approach on behalf of {}.",
                    candidate.name(),
                    recruiter.name(),
                    organization.name()
                ),
            },
        },
    )?;
    let departure_report = match (plan.context.outcome, plan.context.previous_organization) {
        (RecruitmentOutcome::Accepted, Some(previous_organization)) => {
            let previous = state
                .world
                .get_organization(previous_organization)
                .expect("valid previous membership must reference an organization");
            Some(validate_record_report(
                state,
                ReportDraft {
                    recipient: previous_organization,
                    kind: ReportKind::AfterAction,
                    title: "Personnel change".to_owned(),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Notable,
                        summary: format!(
                            "{} left {} and is no longer available for assignments.",
                            candidate.name(),
                            previous.name()
                        ),
                        sources: Vec::new(),
                        entities: BTreeSet::from([
                            EntityRef::Character(plan.draft.candidate),
                            EntityRef::Organization(previous_organization),
                        ]),
                        decision: None,
                    }],
                },
            )?)
        }
        (RecruitmentOutcome::Accepted, None) | (RecruitmentOutcome::Refused, _) => None,
    };
    Ok(ValidatedRecruitmentAttempt {
        plan,
        authority,
        delegated_guard,
        reassignment,
        history,
        outcome_information,
        departure_report,
    })
}

pub struct ValidatedRecruitmentAttempt {
    plan: RecruitmentPlan,
    authority: RecruitmentAuthority,
    delegated_guard: Option<MandateRecruitmentGuard>,
    reassignment: Option<ValidatedCharacterReassignment>,
    history: Option<ValidatedHistoryEvent>,
    outcome_information: ValidatedInformation,
    departure_report: Option<ValidatedReport>,
}

impl ValidatedRecruitmentAttempt {
    pub fn commit(self, state: &mut AppState) -> Result<RecruitmentAttemptId, RecruitmentError> {
        let mut budget = Vec::new();
        if self.history.is_some() {
            budget.push((IdKind::HistoryEvent, 1));
        }
        budget.push((IdKind::Information, 1));
        if self.departure_report.is_some() {
            budget.push((IdKind::Report, 1));
        }
        budget.push((IdKind::RecruitmentAttempt, 1));
        state.ids.reserve_many(&budget)?;
        if let Some(guard) = self.delegated_guard {
            ensure_mandate_authority_current(state, guard.authority)?;
            let current_policy = resolve_policy_for_manager(
                state,
                guard.authority.authority().manager,
                PolicyKind::IndependentRecruitment,
            )?;
            if current_policy != guard.policy {
                return Err(RecruitmentError::StaleRecruitmentPolicy);
            }
            if current_policy.setting
                != PolicySetting::IndependentRecruitment(guard.required_policy)
            {
                return Err(RecruitmentError::StaleRecruitmentPolicy);
            }
        }
        validate_plan_state_snapshot(state, &self.plan)?;
        let history_event = match self.plan.context.outcome {
            RecruitmentOutcome::Accepted => {
                self.reassignment
                    .expect("accepted recruitment must carry a reassignment token")
                    .commit(state)?;
                Some(
                    self.history
                        .expect("accepted recruitment must carry a history token")
                        .commit(state)?,
                )
            }
            RecruitmentOutcome::Refused => {
                debug_assert!(self.reassignment.is_none());
                debug_assert!(self.history.is_none());
                None
            }
        };
        let outcome_information = self.outcome_information.commit(state)?;
        if let Some(report) = self.departure_report {
            report.commit(state)?;
        }
        let id = state.ids.next_recruitment_attempt()?;
        state
            .recruitment
            .insert(build_recruitment_record(RecruitmentRecordParts {
                id,
                draft: self.plan.draft,
                context: RecruitmentRecordContextParts {
                    authority: self.authority,
                    recruiter_relationship: self.plan.dependencies.recruiter_relationship,
                    incumbent_relationship: self.plan.dependencies.incumbent_relationship,
                    previous_organization: self.plan.context.previous_organization,
                    previous_supervisor: self.plan.context.previous_supervisor,
                    pressure_information: self.plan.context.pressure_information,
                    occurred_at: self.plan.context.occurred_at,
                },
                resolution: RecruitmentRecordResolutionParts {
                    factors: self.plan.context.factors,
                    margin: self.plan.context.margin,
                    outcome: self.plan.context.outcome,
                    outcome_information,
                    history_event,
                },
            }));
        Ok(id)
    }
}

pub(crate) fn recruitment_policy_source(source: PolicySource) -> RecruitmentPolicySource {
    match source {
        PolicySource::Organization(organization) => {
            RecruitmentPolicySource::Organization(organization)
        }
        PolicySource::Mandate(mandate) => RecruitmentPolicySource::Mandate(mandate),
    }
}

fn validate_target_and_recruiter(
    state: &AppState,
    target_organization: OrganizationId,
    recruiter: CharacterId,
) -> Result<(), RecruitmentError> {
    let organization = state.world.get_organization(target_organization).ok_or(
        RecruitmentError::MissingTargetOrganization(target_organization),
    )?;
    if organization.lifecycle() != Lifecycle::Active {
        return Err(RecruitmentError::InactiveTargetOrganization(
            target_organization,
        ));
    }
    if organization.kind() != OrganizationKind::Criminal {
        return Err(RecruitmentError::InvalidTargetOrganizationKind(
            target_organization,
        ));
    }
    let recruiter_record = state
        .world
        .get_character(recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(recruiter))?;
    if recruiter_record.lifecycle() != Lifecycle::Active {
        return Err(RecruitmentError::InactiveRecruiter(recruiter));
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(recruiter) {
        return Err(RecruitmentError::DetainedRecruiter {
            recruiter,
            arrest: arrest.id(),
        });
    }
    if recruiter_record.organization() != Some(target_organization) {
        return Err(RecruitmentError::RecruiterOrganizationMismatch {
            recruiter,
            organization: target_organization,
        });
    }
    Ok(())
}

fn validate_recruitment_request<'a>(
    registry: &Registry,
    state: &'a AppState,
    draft: RecruitmentDraft,
) -> Result<
    (
        &'a crate::world::CharacterRecord,
        &'a crate::world::CharacterRecord,
    ),
    RecruitmentError,
> {
    let (candidate, recruiter) = validate_recruitment_request_base(state, draft)?;
    validate_cooldown(
        registry.recruitment(),
        state,
        draft.candidate,
        draft.target_organization,
    )?;
    validate_reassign_character(
        state,
        draft.candidate,
        Some(draft.target_organization),
        Some(draft.recruiter),
    )?;
    Ok((candidate, recruiter))
}

fn validate_recruitment_request_base(
    state: &AppState,
    draft: RecruitmentDraft,
) -> Result<
    (
        &crate::world::CharacterRecord,
        &crate::world::CharacterRecord,
    ),
    RecruitmentError,
> {
    validate_target_and_recruiter(state, draft.target_organization, draft.recruiter)?;
    if draft.candidate == draft.recruiter {
        return Err(RecruitmentError::SelfRecruitment);
    }
    let candidate = state
        .world
        .get_character(draft.candidate)
        .ok_or(RecruitmentError::MissingCandidate(draft.candidate))?;
    if candidate.lifecycle() != Lifecycle::Active {
        return Err(RecruitmentError::InactiveCandidate(draft.candidate));
    }
    if candidate.organization() == Some(draft.target_organization) {
        return Err(RecruitmentError::CandidateAlreadyMember {
            candidate: draft.candidate,
            organization: draft.target_organization,
        });
    }
    if let Some(organization) = candidate.organization() {
        let current = state
            .world
            .get_organization(organization)
            .expect("valid character membership must reference an organization");
        if current.kind() != OrganizationKind::Criminal {
            return Err(RecruitmentError::CandidateOrganizationNotRecruitable {
                candidate: draft.candidate,
                organization,
            });
        }
    }
    if state
        .social
        .get_relationship(draft.candidate, draft.recruiter)
        .is_none()
    {
        return Err(RecruitmentError::NoRecruitmentRelationship {
            candidate: draft.candidate,
            recruiter: draft.recruiter,
        });
    }
    let recruiter = state
        .world
        .get_character(draft.recruiter)
        .expect("validated recruiter must exist");
    Ok((candidate, recruiter))
}

fn candidate_organization_is_recruitable(
    state: &AppState,
    organization: Option<OrganizationId>,
) -> bool {
    organization.is_none_or(|organization| {
        state
            .world
            .get_organization(organization)
            .is_some_and(|record| record.kind() == OrganizationKind::Criminal)
    })
}

/// One cooldown rule, two shapes: the discovery filter asks the predicate form, the
/// transaction path needs the typed error with the exact next-eligible instant.
fn recruitment_is_on_cooldown(
    definition: &RecruitmentDefinition,
    state: &AppState,
    candidate: CharacterId,
    organization: OrganizationId,
) -> bool {
    validate_cooldown(definition, state, candidate, organization).is_err()
}

fn validate_cooldown(
    definition: &RecruitmentDefinition,
    state: &AppState,
    candidate: CharacterId,
    organization: OrganizationId,
) -> Result<(), RecruitmentError> {
    if let Some(attempt) = state
        .recruitment
        .latest_attempt_for(candidate, organization)
    {
        let next_eligible_at = attempt.occurred_at() + definition.cooldown();
        if state.now() < next_eligible_at {
            return Err(RecruitmentError::Cooldown {
                candidate,
                organization,
                next_eligible_at,
            });
        }
    }
    Ok(())
}

pub(crate) struct RecruitmentFactorContext<'a> {
    pub definition: &'a RecruitmentDefinition,
    pub candidate: &'a crate::world::CharacterRecord,
    pub recruiter: &'a crate::world::CharacterRecord,
    pub approach: RecruitmentApproach,
    pub recruiter_relationship: RecruitmentRelationshipSnapshot,
    pub incumbent_relationship: Option<RecruitmentRelationshipSnapshot>,
    pub perceived_legal_pressure: u8,
    pub had_previous_organization: bool,
}

pub(crate) fn resolve_recruitment_factors_from_context(
    context: RecruitmentFactorContext<'_>,
) -> Option<RecruitmentFactors> {
    let RecruitmentFactorContext {
        definition,
        candidate,
        recruiter,
        approach,
        recruiter_relationship,
        incumbent_relationship,
        perceived_legal_pressure,
        had_previous_organization,
    } = context;
    let relationship = recruiter_relationship.dimensions()?;

    let base_influence = definition
        .recruiter_capabilities()
        .iter()
        .filter_map(|kind| recruiter.capability(*kind))
        .map(|rating| rating.value())
        .max()
        .unwrap_or(0);
    let recruiter_influence = base_influence
        .saturating_add(
            u8::from(recruiter.has_trait(TraitKind::Charismatic))
                .saturating_mul(definition.charismatic_recruiter_bonus()),
        )
        .min(100);

    let drive_alignment = definition
        .drives_for_approach(approach)
        .iter()
        .map(|kind| drive_value(candidate, *kind))
        .max()
        .unwrap_or(0);

    let relationship_support = recruitment_relationship_support(definition, relationship);
    let (incumbent_attachment, incumbent_resentment) = incumbent_relationship
        .and_then(|snapshot| snapshot.dimensions())
        .map(|dimensions| recruitment_incumbent_factors(definition, dimensions))
        .unwrap_or((0, 0));

    let membership_resistance = if had_previous_organization {
        definition.existing_membership_resistance()
    } else {
        0
    };
    let trait_adjustment =
        recruitment_trait_adjustment(definition, candidate, approach, incumbent_resentment);

    Some(build_recruitment_factors(RecruitmentFactorComponents {
        recruiter_influence,
        drive_alignment,
        relationship_support,
        incumbent_attachment,
        incumbent_resentment,
        perceived_legal_pressure,
        membership_resistance,
        trait_adjustment,
    }))
}

pub(crate) fn recruitment_relationship_support(
    definition: &RecruitmentDefinition,
    dimensions: RelationshipDimensions,
) -> u8 {
    let weights = definition.relationships().recruiter_support;
    let positive_relationship = u16::from(dimensions.trust.value())
        .saturating_mul(u16::from(weights.trust_weight))
        + u16::from(dimensions.respect.value()).saturating_mul(u16::from(weights.respect_weight))
        + u16::from(dimensions.affection.value())
            .saturating_mul(u16::from(weights.affection_weight))
        + u16::from(dimensions.debt.value()).saturating_mul(u16::from(weights.debt_weight));
    let positive = u8::try_from(positive_relationship / u16::from(weights.divisor))
        .expect("bounded relationship support must fit u8")
        .min(100);
    let fear_penalty = u8::try_from(
        u16::from(dimensions.fear.value()).saturating_mul(u16::from(weights.fear_penalty_weight))
            / u16::from(weights.fear_penalty_divisor),
    )
    .expect("bounded relationship fear penalty must fit u8");
    positive.saturating_sub(fear_penalty)
}

pub(crate) fn recruitment_incumbent_factors(
    definition: &RecruitmentDefinition,
    dimensions: RelationshipDimensions,
) -> (u8, u8) {
    let weights = definition.relationships().incumbent_attachment;
    let attachment = (u16::from(dimensions.trust.value())
        .saturating_mul(u16::from(weights.trust_weight))
        + u16::from(dimensions.respect.value()).saturating_mul(u16::from(weights.respect_weight))
        + u16::from(dimensions.affection.value())
            .saturating_mul(u16::from(weights.affection_weight))
        + u16::from(dimensions.dependence.value())
            .saturating_mul(u16::from(weights.dependence_weight)))
        / u16::from(weights.divisor);
    (
        u8::try_from(attachment).expect("bounded incumbent attachment must fit u8"),
        dimensions.resentment.value(),
    )
}

fn drive_value(character: &crate::world::CharacterRecord, kind: DriveKind) -> u8 {
    character.drive(kind).map_or(0, |rating| rating.value())
}

fn recruitment_trait_adjustment(
    definition: &RecruitmentDefinition,
    candidate: &crate::world::CharacterRecord,
    approach: RecruitmentApproach,
    incumbent_resentment: u8,
) -> i16 {
    definition
        .trait_rules()
        .iter()
        .filter(|rule| candidate.has_trait(rule.trait_kind))
        .filter(|rule| {
            rule.approach
                .is_none_or(|rule_approach| rule_approach == approach)
        })
        .filter(|rule| {
            rule.minimum_incumbent_resentment
                .is_none_or(|minimum| incumbent_resentment >= minimum)
        })
        .try_fold(0_i16, |total, rule| total.checked_add(rule.adjustment))
        .expect("validated authored recruitment trait adjustments must fit i16")
}

pub(crate) fn resolve_recruitment_margin(
    definition: &RecruitmentDefinition,
    factors: RecruitmentFactors,
    approach: RecruitmentApproach,
) -> i16 {
    let weights = definition.weights();
    // Legal pressure is approach-sensitive: only Protection offers genuinely
    // leverage fear of prosecution. Financial/Advancement pitches do not benefit
    // from a target being "wanted", which would otherwise reward poaching the
    // most-investigated characters with money alone.
    let legal_weight = if approach == RecruitmentApproach::Protection {
        i16::from(weights.perceived_legal_pressure)
    } else {
        0
    };
    let score = definition.base_willingness()
        + weighted(
            factors.recruiter_influence(),
            i16::from(weights.recruiter_influence),
        )
        + weighted(
            factors.drive_alignment(),
            i16::from(weights.drive_alignment),
        )
        + weighted(
            factors.relationship_support(),
            i16::from(weights.relationship_support),
        )
        + weighted(
            factors.incumbent_resentment(),
            i16::from(weights.incumbent_resentment),
        )
        + weighted(factors.perceived_legal_pressure(), legal_weight)
        - weighted(
            factors.incumbent_attachment(),
            i16::from(weights.incumbent_attachment),
        )
        - i16::from(factors.membership_resistance())
        + factors.trait_adjustment();
    score - definition.acceptance_score()
}

fn weighted(value: u8, weight: i16) -> i16 {
    i16::from(value) * weight / 100
}

pub(crate) fn classify_recruitment_outcome(margin: i16) -> RecruitmentOutcome {
    if margin >= 0 {
        RecruitmentOutcome::Accepted
    } else {
        RecruitmentOutcome::Refused
    }
}

fn validate_plan_state_snapshot(
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<(), RecruitmentError> {
    if state.now() != plan.context.occurred_at {
        return Err(RecruitmentError::StaleTime {
            expected: plan.context.occurred_at,
            found: state.now(),
        });
    }
    let candidate = state
        .world
        .get_character(plan.draft.candidate)
        .ok_or(RecruitmentError::MissingCandidate(plan.draft.candidate))?;
    if candidate.version() != plan.dependencies.expected_candidate_version {
        return Err(RecruitmentError::StaleCandidate {
            candidate: plan.draft.candidate,
            expected: plan.dependencies.expected_candidate_version,
            found: candidate.version(),
        });
    }
    let recruiter = state
        .world
        .get_character(plan.draft.recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(plan.draft.recruiter))?;
    if recruiter.version() != plan.dependencies.expected_recruiter_version {
        return Err(RecruitmentError::StaleRecruiter {
            recruiter: plan.draft.recruiter,
            expected: plan.dependencies.expected_recruiter_version,
            found: recruiter.version(),
        });
    }
    validate_relationship_snapshot(state, plan.dependencies.recruiter_relationship)?;
    if let Some(snapshot) = plan.dependencies.incumbent_relationship {
        validate_relationship_snapshot(state, snapshot)?;
    }
    if candidate_pressure_information_ids(state, plan.draft.candidate, state.now())
        != plan.dependencies.pressure_information_snapshot
    {
        return Err(RecruitmentError::StalePressureKnowledge {
            candidate: plan.draft.candidate,
        });
    }
    let latest = state
        .recruitment
        .latest_attempt_for(plan.draft.candidate, plan.draft.target_organization)
        .map(|attempt| attempt.id());
    if latest != plan.dependencies.expected_latest_attempt {
        return Err(RecruitmentError::StaleRecruitmentHistory {
            candidate: plan.draft.candidate,
            organization: plan.draft.target_organization,
        });
    }
    validate_recruitment_request_base(state, plan.draft)?;
    Ok(())
}

fn validate_plan_definition(
    definition: &RecruitmentDefinition,
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<(), RecruitmentError> {
    let candidate = state
        .world
        .get_character(plan.draft.candidate)
        .ok_or(RecruitmentError::MissingCandidate(plan.draft.candidate))?;
    let recruiter = state
        .world
        .get_character(plan.draft.recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(plan.draft.recruiter))?;
    let (pressure_information, perceived_legal_pressure) = select_perceived_legal_pressure_at(
        definition,
        state,
        plan.draft.candidate,
        plan.context.occurred_at,
    );
    if pressure_information != plan.context.pressure_information {
        return Err(RecruitmentError::StalePressureKnowledge {
            candidate: plan.draft.candidate,
        });
    }
    let factors = resolve_recruitment_factors_from_context(RecruitmentFactorContext {
        definition,
        candidate,
        recruiter,
        approach: plan.draft.approach,
        recruiter_relationship: plan.dependencies.recruiter_relationship,
        incumbent_relationship: plan.dependencies.incumbent_relationship,
        perceived_legal_pressure,
        had_previous_organization: plan.context.previous_organization.is_some(),
    })
    .expect("validated recruitment plan must preserve its recruiter relationship");
    debug_assert_eq!(factors, plan.context.factors);
    debug_assert_eq!(
        resolve_recruitment_margin(definition, factors, plan.draft.approach),
        plan.context.margin
    );
    debug_assert_eq!(
        classify_recruitment_outcome(plan.context.margin),
        plan.context.outcome
    );
    Ok(())
}

pub(crate) fn select_perceived_legal_pressure_at(
    definition: &RecruitmentDefinition,
    state: &AppState,
    candidate: CharacterId,
    at: SimTime,
) -> (Option<InformationId>, u8) {
    // Selection runs over exactly the ID set the staleness token captures, so a fresh plan
    // can never spuriously fail with `StalePressureKnowledge`.
    let selected = candidate_pressure_information_ids(state, candidate, at)
        .into_iter()
        .filter_map(|id| state.intelligence.get_information(id))
        .map(|information| {
            (
                information.id(),
                perceived_legal_pressure_score(definition, information, at),
                information.observed_at(),
            )
        })
        .filter(|(_, score, _)| *score > 0)
        .max_by_key(|(id, score, observed_at)| (*score, *observed_at, *id))
        .map_or((None, 0), |(id, score, _)| (Some(id), score));
    selected
}
fn perceived_legal_pressure_score(
    definition: &RecruitmentDefinition,
    information: &InformationRecord,
    at: SimTime,
) -> u8 {
    let quality = definition.information_quality();
    let reliability = u16::from(quality.reliability_score(information.reliability()));
    let specificity = u16::from(quality.specificity_score(information.specificity()));
    let base = (reliability + specificity) / 2;
    let age = at
        .as_minutes()
        .saturating_sub(information.observed_at().as_minutes());
    let max_age = u64::from(definition.perceived_legal_pressure_max_age().as_minutes());
    let remaining = max_age.saturating_sub(age);
    u8::try_from(u64::from(base) * remaining / max_age)
        .expect("bounded perceived legal pressure must fit u8")
}

/// The single staleness predicate for candidate pressure knowledge: both the plan's snapshot
/// and the commit-time revalidation must agree through this one derivation.
fn candidate_pressure_information_ids(
    state: &AppState,
    candidate: CharacterId,
    at: SimTime,
) -> BTreeSet<InformationId> {
    state
        .intelligence
        .information_for_holder_by_topic(
            KnowledgeHolder::Character(candidate),
            InformationTopic::PoliceActivity,
        )
        .filter(|information| {
            information.subject() == EntityRef::Character(candidate)
                && information.recorded_at() <= at
                && information.observed_at() <= at
        })
        .map(InformationRecord::id)
        .collect()
}

fn draw_candidate_index(rng: &mut impl rand_core::RngCore, choice_count: usize) -> Option<usize> {
    if choice_count == 0 {
        return None;
    }
    let bound = u64::try_from(choice_count).expect("candidate count must fit u64");
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = rng.next_u64();
        if draw < rejection_zone {
            return Some((draw % bound) as usize);
        }
    }
}

fn validate_relationship_snapshot(
    state: &AppState,
    snapshot: RecruitmentRelationshipSnapshot,
) -> Result<(), RecruitmentError> {
    let relationship = state
        .social
        .get_relationship(snapshot.from(), snapshot.to());
    let found = relationship.map(|relationship| relationship.version());
    let found_dimensions = relationship.map(|relationship| relationship.dimensions());
    if found != snapshot.version() || found_dimensions != snapshot.dimensions() {
        return Err(RecruitmentError::StaleRelationship {
            from: snapshot.from(),
            to: snapshot.to(),
            expected: snapshot.version(),
            found,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
