//! Relationship-gated recruitment decisions with causal factors, cooldowns, and atomic accepted membership changes.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, DecisionRequestId, IdExhaustionError, IdKind, InformationId,
    OrganizationId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::delegation::delegation_system::{
    DelegationError, PolicySource, ResolvedPolicy, ensure_mandate_authority_current,
    resolve_mandate_authority, resolve_policy_for_manager,
};
use crate::delegation::{
    MandateAuthority, ResolvedMandateAuthority, ResponsibilityFunction, ResponsibilityScope,
};
use crate::history::history_system::{HistoryError, ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::recruitment::scoring::{
    candidate_pressure_information_ids, resolve_perceived_legal_pressure_at,
    resolve_perceived_legal_pressure_from_ids, resolve_recruitment_factors_from_context,
    resolve_recruitment_margin, resolve_recruitment_outcome,
};
use crate::recruitment::{
    RecruitmentApproach, RecruitmentAuthority, RecruitmentDraft, RecruitmentFactors,
    RecruitmentOutcome, RecruitmentPolicySource, RecruitmentRecordContextParts,
    RecruitmentRecordParts, RecruitmentRecordResolutionParts, RecruitmentRelationshipSnapshot,
    build_recruitment_record, build_recruitment_relationship_snapshot,
};
use crate::registry::{RecruitmentDefinition, Registry};
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::{
    ValidatedCharacterReassignment, WorldError, validate_reassign_character,
};
use crate::world::{ApprovalPolicy, OrganizationKind, PolicyKind, PolicySetting};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RecruitmentError {
    #[error("target organization {0} does not exist")]
    MissingTargetOrganization(OrganizationId),
    #[error("target organization {0} is not a criminal organization")]
    InvalidTargetOrganizationKind(OrganizationId),
    #[error("recruiter {0} does not exist")]
    MissingRecruiter(CharacterId),
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
    #[error(
        "executive recruitment is reserved for organization heads; recruiter {recruiter} reports to {supervisor}"
    )]
    ExecutiveRecruiterSupervised {
        recruiter: CharacterId,
        supervisor: CharacterId,
    },
    #[error("recruitment approval decision {decision} is already pending for this candidate")]
    PendingRecruitmentApproval { decision: DecisionRequestId },
    #[error("candidate {0} does not exist")]
    MissingCandidate(CharacterId),
    #[error("candidate {candidate} references missing organization {organization}")]
    MissingCandidateOrganization {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error("a character cannot recruit themselves")]
    SelfRecruitment,
    #[error("candidate {candidate} is already a member of target organization {organization}")]
    CandidateAlreadyMember {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error(
        "candidate {candidate} belongs to organization {organization}, which requires a different personnel system"
    )]
    CandidateOrganizationNotRecruitable {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error("candidate {candidate} has no relationship edge to recruiter {recruiter}")]
    NoRecruitmentRelationship {
        candidate: CharacterId,
        recruiter: CharacterId,
    },
    #[error(
        "candidate {candidate} cannot be approached again by organization {organization} before {next_eligible_at:?}"
    )]
    Cooldown {
        candidate: CharacterId,
        organization: OrganizationId,
        next_eligible_at: SimTime,
    },
    #[error("recruitment plan was decided at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(
        "candidate {candidate} changed after recruitment was decided; expected version {expected}, found {found}"
    )]
    StaleCandidate {
        candidate: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "recruiter {recruiter} changed after recruitment was decided; expected version {expected}, found {found}"
    )]
    StaleRecruiter {
        recruiter: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "relationship {from}->{to} changed after recruitment was decided; expected version {expected:?}, found {found:?}"
    )]
    StaleRelationship {
        from: CharacterId,
        to: CharacterId,
        expected: Option<u32>,
        found: Option<u32>,
    },
    #[error(
        "recruitment history for candidate {candidate} and organization {organization} changed after the plan was decided"
    )]
    StaleRecruitmentHistory {
        candidate: CharacterId,
        organization: OrganizationId,
    },
    #[error("candidate {candidate} legal-pressure knowledge changed after recruitment was decided")]
    StalePressureKnowledge { candidate: CharacterId },
    #[error(
        "target organization {organization} underworld competence changed after recruitment was decided; expected {expected}, found {found}"
    )]
    StaleOrganizationCompetence {
        organization: OrganizationId,
        expected: u8,
        found: u8,
    },
    #[error(
        "delegated recruitment requires recruiter {recruiter} to be the authority manager {manager}"
    )]
    DelegatedRecruiterMismatch {
        recruiter: CharacterId,
        manager: CharacterId,
    },
    #[error("delegated recruitment requires Personnel scope, not {scope:?}")]
    DelegatedRecruitmentRequiresPersonnelScope { scope: ResponsibilityScope },
    #[error(
        "delegated recruitment authority belongs to organization {authority_organization}, not target {target_organization}"
    )]
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

pub(crate) const fn recruitment_member_report_title(outcome: RecruitmentOutcome) -> &'static str {
    match outcome {
        RecruitmentOutcome::Accepted => "Personnel change",
        RecruitmentOutcome::Refused => "Personnel approach",
    }
}

pub(crate) fn recruitment_member_report_summary(
    outcome: RecruitmentOutcome,
    candidate: &str,
    incumbent_organization: &str,
    outside_approach: Option<(&str, &str)>,
) -> String {
    match outcome {
        RecruitmentOutcome::Accepted => format!(
            "{candidate} left {incumbent_organization} and is no longer available for assignments."
        ),
        RecruitmentOutcome::Refused => {
            let (recruiter, target_organization) = outside_approach
                .expect("refused recruitment member reports always name the outside approach");
            format!(
                "{candidate} told {incumbent_organization} leadership that {recruiter} of {target_organization} tried to recruit them. They turned the approach down and remain with {incumbent_organization}."
            )
        }
    }
}

pub(crate) fn recruitment_member_report_entities(
    outcome: RecruitmentOutcome,
    candidate: CharacterId,
    recruiter: CharacterId,
    target_organization: OrganizationId,
    incumbent_organization: OrganizationId,
) -> BTreeSet<EntityRef> {
    match outcome {
        RecruitmentOutcome::Accepted => BTreeSet::from([
            EntityRef::Character(candidate),
            EntityRef::Organization(incumbent_organization),
        ]),
        RecruitmentOutcome::Refused => BTreeSet::from([
            EntityRef::Character(candidate),
            EntityRef::Character(recruiter),
            EntityRef::Organization(target_organization),
            EntityRef::Organization(incumbent_organization),
        ]),
    }
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
    pressure_information_max_age: SimDuration,
    expected_latest_attempt: Option<RecruitmentAttemptId>,
    /// Authored fallback used when the sparse underworld reputation record is absent. The
    /// validated token needs this value at commit so it can re-resolve the exact competence
    /// score that affected willingness without requiring the immutable Registry again.
    reputation_baseline: u8,
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
        let record = state
            .world
            .get_character(candidate)
            .ok_or(RecruitmentError::MissingCandidate(candidate))?;
        if record.organization() == Some(target_organization) {
            continue;
        }
        if let Some(organization) = record.organization() {
            let organization_record = state.world.get_organization(organization).ok_or(
                RecruitmentError::MissingCandidateOrganization {
                    candidate,
                    organization,
                },
            )?;
            if organization_record.kind() != OrganizationKind::Criminal {
                continue;
            }
        }
        if recruitment_is_on_cooldown(
            registry.recruitment(),
            state,
            candidate,
            target_organization,
        ) {
            continue;
        }
        if let Err(error) = validate_reassign_character(
            state,
            candidate,
            Some(target_organization),
            Some(recruiter),
        ) {
            if candidate_reassignment_is_temporarily_blocked(error) {
                continue;
            }
            return Err(error.into());
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

/// Reassignment failures that describe a currently unavailable prospect rather than broken
/// world state. Candidate discovery is a query, so routine workload/custody/leadership bindings
/// exclude a prospect without turning the whole discovery pass into an error. Every structural,
/// hierarchy, identity, or impossible business-only error propagates instead of being silently
/// erased by a blanket `is_err()` filter.
fn candidate_reassignment_is_temporarily_blocked(error: WorldError) -> bool {
    match error {
        WorldError::ActiveOperationAssignment { .. }
        | WorldError::ActiveMandateAssignment { .. }
        | WorldError::ActiveInvestigationAssignment { .. }
        | WorldError::ActiveArrestAssignment { .. }
        | WorldError::ActiveProsecutionAssignment { .. }
        | WorldError::InformantHandlerConflict { .. }
        | WorldError::ActiveInstitutionalContactHandler { .. }
        | WorldError::ActiveInstitutionalContactAssignment { .. }
        | WorldError::DirectReportAssignment { .. } => true,
        WorldError::EmptyName
        | WorldError::MissingOrganization(_)
        | WorldError::MissingCharacter(_)
        | WorldError::MissingNeighborhood(_)
        | WorldError::MissingBusiness(_)
        | WorldError::BusinessOwnershipUnchanged { .. }
        | WorldError::CharacterReassignmentUnchanged { .. }
        | WorldError::StaleBusiness { .. }
        | WorldError::BusinessOwnerChanged { .. }
        | WorldError::ActiveEnterpriseSupport { .. }
        | WorldError::ActiveEnterpriseHost { .. }
        | WorldError::SupervisorOrganizationMismatch { .. }
        | WorldError::SupervisorWithoutOrganization { .. }
        | WorldError::SelfSupervision { .. }
        | WorldError::SupervisionCycle { .. }
        | WorldError::DetainedSupervisor { .. }
        | WorldError::StaleCharacter { .. }
        | WorldError::InvalidPlayerOrganization(_)
        | WorldError::IdExhaustion(_) => false,
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
    let pressure_information_max_age = registry.recruitment().perceived_legal_pressure_max_age();
    let pressure_information_snapshot = candidate_pressure_information_ids(
        state,
        draft.candidate,
        state.now(),
        pressure_information_max_age,
    );
    let (pressure_information, perceived_legal_pressure) =
        resolve_perceived_legal_pressure_from_ids(
            registry.recruitment(),
            state,
            &pressure_information_snapshot,
            state.now(),
        );
    // The candidate weighs the outfit's demonstrated underworld competence, resolved through
    // the canonical reputation surface; the frozen value rides inside the plan's factors.
    let organization_competence = crate::reputation::reputation_system::resolve_score(
        registry,
        state.reputation(),
        draft.target_organization,
        crate::reputation::AudienceKind::Underworld,
        crate::reputation::ReputationDimension::Competence,
    );
    let factors = resolve_recruitment_factors_from_context(RecruitmentFactorContext {
        definition: registry.recruitment(),
        candidate,
        recruiter,
        approach: draft.approach,
        recruiter_relationship,
        incumbent_relationship,
        perceived_legal_pressure,
        organization_competence,
        had_previous_organization: candidate.organization().is_some(),
    })
    .expect("validated recruitment must retain a candidate-to-recruiter relationship snapshot");
    let margin = resolve_recruitment_margin(registry.recruitment(), factors, draft.approach);
    let outcome = resolve_recruitment_outcome(margin);
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
            pressure_information_max_age,
            expected_latest_attempt: state
                .recruitment
                .latest_attempt_for(draft.candidate, draft.target_organization)
                .map(|attempt| attempt.id()),
            reputation_baseline: registry.reputation().baseline(),
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
    if let Some(pending) = state
        .decisions()
        .pending_for_recruitment_approval(draft.target_organization, draft.candidate)
    {
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
    // One exclusive route per (organization, candidate): while an approval request sits
    // pending, the delegated channel must not race it â€” an attempt landing first would
    // strand the request against a candidate who is already a member, permanently blocking
    // the pair's executive channel too.
    if let Some(pending) = state
        .decisions()
        .pending_for_recruitment_approval(draft.target_organization, draft.candidate)
    {
        return Err(RecruitmentError::PendingRecruitmentApproval { decision: pending });
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
    let history = validate_recruitment_history_event(state, &plan)?;
    let outcome_information = validate_recruitment_outcome_information(state, &plan)?;
    let member_report = validate_recruitment_member_report(state, &plan)?;
    Ok(ValidatedRecruitmentAttempt {
        plan,
        authority,
        delegated_guard,
        reassignment,
        history,
        outcome_information,
        member_report,
    })
}

pub(crate) fn recruitment_defection_history_summary(candidate: &str) -> String {
    format!("{candidate} left their former organization.")
}

pub(crate) fn recruitment_join_history_summary(
    candidate: &str,
    organization: &str,
    recruiter: &str,
) -> String {
    format!("{candidate} joined {organization} after recruitment by {recruiter}.")
}

pub(crate) fn recruitment_outcome_summary(
    candidate: &str,
    recruiter: &str,
    organization: &str,
    outcome: RecruitmentOutcome,
) -> String {
    match outcome {
        RecruitmentOutcome::Accepted => {
            format!(
                "{candidate} accepted {recruiter}'s recruitment approach and joined {organization}."
            )
        }
        RecruitmentOutcome::Refused => {
            format!(
                "{candidate} refused {recruiter}'s recruitment approach on behalf of {organization}."
            )
        }
    }
}

/// Campaign history for an accepted attempt. When the candidate is poached from another
/// organization, history must not leak the hidden recruiting organization: the defector's
/// former organization is told only that the member left, and the player discovers the
/// destination through surveillance, not a global history read. So a defection event omits
/// the destination organization entity and its name — and also the recruiter, whose
/// membership would resolve straight back to that organization.
fn validate_recruitment_history_event(
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<Option<ValidatedHistoryEvent>, RecruitmentError> {
    if plan.context.outcome != RecruitmentOutcome::Accepted {
        return Ok(None);
    }
    let candidate = state
        .world
        .get_character(plan.draft.candidate)
        .expect("validated recruitment candidate must exist");
    let event = if plan.context.previous_organization.is_some() {
        HistoryEventDraft {
            occurred_at: plan.context.occurred_at,
            kind: HistoryEventKind::Recruitment,
            summary: recruitment_defection_history_summary(candidate.name()),
            entities: BTreeSet::from([EntityRef::Character(plan.draft.candidate)]),
        }
    } else {
        let recruiter = state
            .world
            .get_character(plan.draft.recruiter)
            .expect("validated recruiter must exist");
        let organization = state
            .world
            .get_organization(plan.draft.target_organization)
            .expect("validated target organization must exist");
        HistoryEventDraft {
            occurred_at: plan.context.occurred_at,
            kind: HistoryEventKind::Recruitment,
            summary: recruitment_join_history_summary(
                candidate.name(),
                organization.name(),
                recruiter.name(),
            ),
            entities: BTreeSet::from([
                EntityRef::Character(plan.draft.candidate),
                EntityRef::Character(plan.draft.recruiter),
                EntityRef::Organization(plan.draft.target_organization),
            ]),
        }
    };
    Ok(Some(validate_record_event(state, event)?))
}

/// The recruiting organization's own personnel knowledge of the attempt's result.
fn validate_recruitment_outcome_information(
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<ValidatedInformation, RecruitmentError> {
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
    Ok(validate_record_information(
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
            summary: recruitment_outcome_summary(
                candidate.name(),
                recruiter.name(),
                organization.name(),
                plan.context.outcome,
            ),
        },
    )?)
}

/// Player-facing report to the candidate's organization: the departure notice on an accepted
/// defection, or the refused-approach loyalty report when membership holds. The defector case
/// stays deliberately silent about the destination because a departing member tells nobody.
fn validate_recruitment_member_report(
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<Option<ValidatedReport>, RecruitmentError> {
    match (plan.context.outcome, plan.context.previous_organization) {
        (RecruitmentOutcome::Accepted, Some(previous_organization)) => {
            let candidate = state
                .world
                .get_character(plan.draft.candidate)
                .expect("validated recruitment candidate must exist");
            let previous = state
                .world
                .get_organization(previous_organization)
                .expect("valid previous membership must reference an organization");
            Ok(Some(validate_record_report(
                state,
                ReportDraft {
                    recipient: previous_organization,
                    kind: ReportKind::AfterAction,
                    title: recruitment_member_report_title(plan.context.outcome).to_owned(),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Notable,
                        summary: recruitment_member_report_summary(
                            plan.context.outcome,
                            candidate.name(),
                            previous.name(),
                            None,
                        ),
                        sources: Vec::new(),
                        entities: recruitment_member_report_entities(
                            plan.context.outcome,
                            plan.draft.candidate,
                            plan.draft.recruiter,
                            plan.draft.target_organization,
                            previous_organization,
                        ),
                        decision: None,
                    }],
                },
            )?))
        }
        // A member who turns down an outside pitch reports that approach to their own
        // leadership, including who made it: loyalty keeps the organization informed even
        // when membership does not move.
        (RecruitmentOutcome::Refused, Some(current_organization)) => {
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
            let current = state
                .world
                .get_organization(current_organization)
                .expect("candidate membership must reference an existing organization");
            Ok(Some(validate_record_report(
                state,
                ReportDraft {
                    recipient: current_organization,
                    kind: ReportKind::AfterAction,
                    title: recruitment_member_report_title(plan.context.outcome).to_owned(),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Notable,
                        summary: recruitment_member_report_summary(
                            plan.context.outcome,
                            candidate.name(),
                            current.name(),
                            Some((recruiter.name(), organization.name())),
                        ),
                        sources: Vec::new(),
                        entities: recruitment_member_report_entities(
                            plan.context.outcome,
                            plan.draft.candidate,
                            plan.draft.recruiter,
                            plan.draft.target_organization,
                            current_organization,
                        ),
                        decision: None,
                    }],
                },
            )?))
        }
        (RecruitmentOutcome::Accepted, None) | (RecruitmentOutcome::Refused, None) => Ok(None),
    }
}

pub struct ValidatedRecruitmentAttempt {
    plan: RecruitmentPlan,
    authority: RecruitmentAuthority,
    delegated_guard: Option<MandateRecruitmentGuard>,
    reassignment: Option<ValidatedCharacterReassignment>,
    history: Option<ValidatedHistoryEvent>,
    outcome_information: ValidatedInformation,
    /// Player-facing report to the candidate's organization: the departure notice on an
    /// accepted defection, or the refused-approach loyalty report when membership holds.
    member_report: Option<ValidatedReport>,
}

impl ValidatedRecruitmentAttempt {
    fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let mut budget = Vec::new();
        if self.history.is_some() {
            budget.push((IdKind::HistoryEvent, 1));
        }
        budget.push((IdKind::Information, 1));
        if self.member_report.is_some() {
            budget.push((IdKind::Report, 1));
        }
        budget.push((IdKind::RecruitmentAttempt, 1));
        budget
    }

    pub(crate) fn preflight_ids(&self, state: &AppState) -> Result<(), RecruitmentError> {
        state.ids.reserve_many(&self.id_budget())?;
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<RecruitmentAttemptId, RecruitmentError> {
        self.preflight_ids(state)?;
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
        let (history_event, resulting_candidate_version) = match self.plan.context.outcome {
            RecruitmentOutcome::Accepted => {
                self.reassignment
                    .expect("accepted recruitment must carry a reassignment token")
                    .commit(state)?;
                let version = state
                    .world
                    .get_character(self.plan.draft.candidate)
                    .expect("reassigned recruitment candidate must still exist")
                    .version();
                (
                    Some(
                        self.history
                            .expect("accepted recruitment must carry a history token")
                            .commit(state)
                            .expect("recruitment history ID was preflighted before reassignment"),
                    ),
                    version,
                )
            }
            RecruitmentOutcome::Refused => {
                debug_assert!(self.reassignment.is_none());
                debug_assert!(self.history.is_none());
                (None, self.plan.dependencies.expected_candidate_version)
            }
        };
        let outcome_information = self
            .outcome_information
            .commit(state)
            .expect("recruitment information ID was preflighted before mutation");
        let member_report = self.member_report.map(|report| {
            report
                .commit(state)
                .expect("recruitment report ID was preflighted before mutation")
        });
        let id = state
            .ids
            .next_recruitment_attempt()
            .expect("recruitment-attempt ID was preflighted before mutation");
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
                    resulting_candidate_version,
                    outcome_information,
                    history_event,
                    member_report,
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
    if organization.kind() != OrganizationKind::Criminal {
        return Err(RecruitmentError::InvalidTargetOrganizationKind(
            target_organization,
        ));
    }
    let recruiter_record = state
        .world
        .get_character(recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(recruiter))?;
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
    /// The recruiting organization's underworld competence reputation as resolved when the
    /// decision was made. Callers snapshot this from the canonical reputation surface; the
    /// invariant pass re-derives with each attempt's own frozen value, because impressions
    /// legitimately move after an attempt and must not retroactively invalidate it.
    pub organization_competence: u8,
    pub had_previous_organization: bool,
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
    if candidate_pressure_information_ids(
        state,
        plan.draft.candidate,
        state.now(),
        plan.dependencies.pressure_information_max_age,
    ) != plan.dependencies.pressure_information_snapshot
    {
        return Err(RecruitmentError::StalePressureKnowledge {
            candidate: plan.draft.candidate,
        });
    }
    let expected_competence = plan.context.factors.organization_competence();
    let found_competence = state
        .reputation()
        .get_record(
            plan.draft.target_organization,
            crate::reputation::AudienceKind::Underworld,
        )
        .map(|record| record.score(crate::reputation::ReputationDimension::Competence))
        .unwrap_or(plan.dependencies.reputation_baseline);
    if found_competence != expected_competence {
        return Err(RecruitmentError::StaleOrganizationCompetence {
            organization: plan.draft.target_organization,
            expected: expected_competence,
            found: found_competence,
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
    let (pressure_information, perceived_legal_pressure) = resolve_perceived_legal_pressure_at(
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
        organization_competence: plan.context.factors.organization_competence(),
        had_previous_organization: plan.context.previous_organization.is_some(),
    })
    .expect("validated recruitment plan must preserve its recruiter relationship");
    debug_assert_eq!(factors, plan.context.factors);
    debug_assert_eq!(
        resolve_recruitment_margin(definition, factors, plan.draft.approach),
        plan.context.margin
    );
    debug_assert_eq!(
        resolve_recruitment_outcome(plan.context.margin),
        plan.context.outcome
    );
    Ok(())
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
