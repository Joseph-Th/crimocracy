//! Relationship-gated recruitment decisions with causal factors, cooldowns, and atomic accepted membership changes.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, DecisionRequestId, InformationId, OrganizationId, RecruitmentAttemptId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::delegation::delegation_system::{
    resolve_mandate_authority, resolve_policy_for_manager, validate_mandate_authority_snapshot,
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
        let policy =
            resolve_policy_for_manager(state, manager, PolicyKind::IndependentRecruitment)?;
        if policy.setting != PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated) {
            continue;
        }
        let approach = autonomous_recruitment_approach(manager_record);
        let Some(candidate) = find_recruitment_candidates(registry, state, organization, manager)?
            .into_iter()
            .next()
        else {
            continue;
        };
        let authority = MandateAuthority {
            mandate,
            manager,
            scope: personnel_scope,
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
    let factors = calculate_recruitment_factors_from_context(RecruitmentFactorContext {
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
    let margin = calculate_recruitment_margin(registry.recruitment(), factors);
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
    validate_recruitment_plan_with_authority(
        registry,
        state,
        decide_recruitment_attempt(registry, state, draft)?,
        RecruitmentAuthority::ExecutiveApproval,
        None,
    )
}

pub fn validate_delegated_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    authority: MandateAuthority,
    draft: RecruitmentDraft,
) -> Result<ValidatedRecruitmentAttempt, RecruitmentError> {
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
    let approval = match policy.setting {
        PolicySetting::IndependentRecruitment(approval) => approval,
        PolicySetting::CollectionForce(_)
        | PolicySetting::PatrolBribery(_)
        | PolicySetting::CasualtyResponse(_)
        | PolicySetting::AssociateLegalSupport(_) => {
            unreachable!("policy kind resolution returned the wrong policy variant")
        }
    };
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
    let approval = match policy.setting {
        PolicySetting::IndependentRecruitment(approval) => approval,
        PolicySetting::CollectionForce(_)
        | PolicySetting::PatrolBribery(_)
        | PolicySetting::CasualtyResponse(_)
        | PolicySetting::AssociateLegalSupport(_) => {
            unreachable!("policy kind resolution returned the wrong policy variant")
        }
    };
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
        if let Some(guard) = self.delegated_guard {
            validate_mandate_authority_snapshot(state, guard.authority)?;
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
                        .commit(state),
                )
            }
            RecruitmentOutcome::Refused => {
                debug_assert!(self.reassignment.is_none());
                debug_assert!(self.history.is_none());
                None
            }
        };
        let outcome_information = self.outcome_information.commit(state);
        if let Some(report) = self.departure_report {
            report.commit(state);
        }
        let id = state.ids.next_recruitment_attempt();
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

fn recruitment_policy_source(source: PolicySource) -> RecruitmentPolicySource {
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

fn recruitment_is_on_cooldown(
    definition: &RecruitmentDefinition,
    state: &AppState,
    candidate: CharacterId,
    organization: OrganizationId,
) -> bool {
    state
        .recruitment
        .latest_attempt_for(candidate, organization)
        .is_some_and(|attempt| state.now() < attempt.occurred_at() + definition.cooldown())
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

pub(crate) fn calculate_recruitment_factors_from_context(
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

pub(crate) fn calculate_recruitment_margin(
    definition: &RecruitmentDefinition,
    factors: RecruitmentFactors,
) -> i16 {
    let weights = definition.weights();
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
        + weighted(
            factors.perceived_legal_pressure(),
            i16::from(weights.perceived_legal_pressure),
        )
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
    let factors = calculate_recruitment_factors_from_context(RecruitmentFactorContext {
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
        calculate_recruitment_margin(definition, factors),
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
        .map(|information| {
            (
                information.id(),
                perceived_legal_pressure_score(definition, information, at),
                information.observed_at(),
            )
        })
        .filter(|(_, score, _)| *score > 0)
        .max_by_key(|(id, score, observed_at)| (*score, *observed_at, *id))
        .map_or((None, 0), |(id, score, _)| (Some(id), score))
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::time::SimDuration;
    use crate::decisions::decision_system::{
        validate_request_recruitment_approval, validate_resolve_decision, DecisionError,
    };
    use crate::decisions::{
        DecisionContext, DecisionResponse, DecisionStatus, RecruitmentApprovalRequestDraft,
    };
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{InformationDraft, InformationSourceKind, Reliability, Specificity};
    use crate::reports::ReportKind;
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{insert_character, insert_organization, set_policy};
    use crate::world::{
        ApprovalPolicy, AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft,
        PolicyKind, PolicySetting, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        registry: Registry,
        state: AppState,
        source: OrganizationId,
        target: OrganizationId,
        incumbent: CharacterId,
        recruiter: CharacterId,
        candidate: CharacterId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn level(value: u8) -> RelationshipLevel {
        RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
    }

    fn relationship(
        trust: u8,
        respect: u8,
        fear: u8,
        affection: u8,
        dependence: u8,
        resentment: u8,
        debt: u8,
    ) -> RelationshipDimensions {
        RelationshipDimensions {
            trust: level(trust),
            respect: level(respect),
            fear: level(fear),
            affection: level(affection),
            dependence: level(dependence),
            resentment: level(resentment),
            debt: level(debt),
        }
    }

    fn assign_personnel_mandate(
        registry: &crate::registry::Registry,
        fixture: &mut Fixture,
        recruitment_policy: Option<ApprovalPolicy>,
    ) -> crate::core::id::MandateId {
        let standing_orders = recruitment_policy
            .map(|policy| {
                BTreeMap::from([(
                    PolicyKind::IndependentRecruitment,
                    PolicySetting::IndependentRecruitment(policy),
                )])
            })
            .unwrap_or_default();
        validate_assign_mandate(
            registry,
            &fixture.state,
            MandateDraft {
                organization: fixture.target,
                manager: fixture.recruiter,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Personnel,
                )]),
                standing_orders,
                budget: None,
            },
        )
        .expect("personnel mandate should validate")
        .commit(&mut fixture.state)
        .expect("personnel mandate should commit")
    }

    fn personnel_authority(
        fixture: &Fixture,
        mandate: crate::core::id::MandateId,
    ) -> MandateAuthority {
        MandateAuthority {
            mandate,
            manager: fixture.recruiter,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
        }
    }

    fn fixture() -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0x5EC2_1933);
        let source = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "North Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("source organization should validate");
        let target = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "South Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("target organization should validate");
        let incumbent = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Incumbent Lieutenant".to_owned(),
                organization: Some(source),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("incumbent should validate");
        let recruiter = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Rival Recruiter".to_owned(),
                organization: Some(target),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(90))]),
                traits: BTreeSet::from([TraitKind::Charismatic]),
                drives: BTreeMap::new(),
            },
        )
        .expect("recruiter should validate");
        let candidate = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Frightened Associate".to_owned(),
                organization: Some(source),
                supervisor: Some(incumbent),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::from([TraitKind::EasilyFrightened]),
                drives: BTreeMap::from([(DriveKind::Safety, rating(90))]),
            },
        )
        .expect("candidate should validate");
        validate_set_relationship(
            &state,
            candidate,
            recruiter,
            relationship(55, 65, 10, 35, 15, 5, 10),
        )
        .expect("candidate-recruiter relationship should validate")
        .commit(&mut state);
        Fixture {
            registry,
            state,
            source,
            target,
            incumbent,
            recruiter,
            candidate,
        }
    }

    fn protection_draft(fixture: &Fixture) -> RecruitmentDraft {
        RecruitmentDraft {
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
        }
    }

    #[test]
    fn candidate_discovery_follows_incoming_relationships_not_global_roster() {
        let registry = build_registry();
        let mut fixture = fixture();
        let unrelated = insert_character(
            &registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Unrelated Associate".to_owned(),
                organization: Some(fixture.source),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("unrelated character should validate");
        validate_set_relationship(
            &fixture.state,
            fixture.recruiter,
            unrelated,
            relationship(90, 90, 0, 50, 0, 0, 0),
        )
        .expect("reverse-direction relationship should validate")
        .commit(&mut fixture.state);

        let candidates = find_recruitment_candidates(
            &fixture.registry,
            &fixture.state,
            fixture.target,
            fixture.recruiter,
        )
        .expect("candidate discovery should validate");
        assert_eq!(candidates, vec![fixture.candidate]);
        validate_invariants(&fixture.state);
    }

    #[test]
    fn delegated_broad_manager_attempts_recruitment_on_authored_cadence() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate =
            assign_personnel_mandate(&registry, &mut fixture, Some(ApprovalPolicy::Delegated));

        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        assert!(
            resolve_due_autonomous_recruitment(&registry, &mut fixture.state)
                .expect("autonomous recruitment before cadence should be a no-op")
                .is_empty()
        );
        fixture.state.advance_clock(SimDuration::ONE_MINUTE);
        let attempts = resolve_due_autonomous_recruitment(&registry, &mut fixture.state)
            .expect("delegated broad recruiter should act at the authored cadence");
        assert_eq!(attempts.len(), 1);
        let attempt = fixture
            .state
            .recruitment()
            .get_attempt(attempts[0])
            .expect("autonomous attempt should persist");
        assert_eq!(attempt.recruiter(), fixture.recruiter);
        assert_eq!(attempt.candidate(), fixture.candidate);
        assert_eq!(attempt.approach(), RecruitmentApproach::PersonalAppeal);
        assert!(matches!(
            attempt.authority(),
            RecruitmentAuthority::Delegated {
                mandate: found_mandate,
                manager,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                policy: ApprovalPolicy::Delegated,
                ..
            } if found_mandate == mandate && manager == fixture.recruiter
        ));
        assert!(
            resolve_due_autonomous_recruitment(&registry, &mut fixture.state)
                .expect("same-minute repeat should be blocked by recruitment history")
                .is_empty()
        );
        validate_state(&fixture.state).expect("autonomous recruitment state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn delegated_recruitment_requires_personnel_authority_and_delegated_policy() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let error = match validate_delegated_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            personnel_authority(&fixture, mandate),
            protection_draft(&fixture),
        ) {
            Ok(_) => panic!("default approval-required policy must block independent recruitment"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            RecruitmentError::IndependentRecruitmentNotDelegated {
                manager: fixture.recruiter,
                policy: ApprovalPolicy::RequireApproval,
            }
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn approval_required_recruitment_executes_only_after_approval() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let request = validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: personnel_authority(&fixture, mandate),
                target_organization: fixture.target,
                recruiter: fixture.recruiter,
                candidate: fixture.candidate,
                approach: RecruitmentApproach::Protection,
                attention: crate::core::attention::AttentionClass::Exception,
                summary: "Personnel manager requests authority to recruit a rival associate."
                    .to_owned(),
            },
        )
        .expect("approval-required recruitment proposal should validate")
        .commit(&mut fixture.state)
        .expect("approval request should commit");

        assert_eq!(
            fixture.state.decisions().pending_for_recruitment_approval(
                fixture.target,
                fixture.recruiter,
                fixture.candidate,
            ),
            Some(request.decision)
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.candidate)
                .expect("candidate should exist before approval")
                .organization(),
            Some(fixture.source)
        );

        let resolution = validate_resolve_decision(
            &fixture.registry,
            &fixture.state,
            request.decision,
            fixture.target,
            DecisionResponse::Approve,
        )
        .expect("fresh personnel approval should validate")
        .commit(&mut fixture.state)
        .expect("approved personnel action should commit atomically");
        let attempt = resolution
            .recruitment_attempt
            .expect("approval should execute one recruitment attempt");
        let record = fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("approved recruitment attempt should persist");
        assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
        assert_eq!(
            record.authority(),
            RecruitmentAuthority::ApprovedDecision {
                decision: request.decision,
                mandate,
                manager: fixture.recruiter,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                mandate_version: 1,
                manager_version: 1,
                policy: ApprovalPolicy::RequireApproval,
                policy_source: RecruitmentPolicySource::Organization(fixture.target),
            }
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempt_for_approval_decision(request.decision)
                .map(|record| record.id()),
            Some(attempt)
        );
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.candidate)
                .expect("accepted candidate should persist")
                .organization(),
            Some(fixture.target)
        );
        let decision = fixture
            .state
            .decisions()
            .get_decision(request.decision)
            .expect("approval decision should persist historically");
        assert_eq!(decision.status(), DecisionStatus::Resolved);
        assert_eq!(
            decision
                .resolution()
                .expect("resolved approval should persist resolution")
                .response(),
            DecisionResponse::Approve
        );
        validate_state(&fixture.state).expect("approved recruitment state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn rejected_recruitment_approval_records_no_attempt() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let request = validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: personnel_authority(&fixture, mandate),
                target_organization: fixture.target,
                recruiter: fixture.recruiter,
                candidate: fixture.candidate,
                approach: RecruitmentApproach::Protection,
                attention: crate::core::attention::AttentionClass::Exception,
                summary: "Personnel manager requests authority to approach a rival associate."
                    .to_owned(),
            },
        )
        .expect("approval request should validate")
        .commit(&mut fixture.state)
        .expect("approval request should commit");
        let resolution = validate_resolve_decision(
            &fixture.registry,
            &fixture.state,
            request.decision,
            fixture.target,
            DecisionResponse::Reject,
        )
        .expect("rejection should validate even though no recruitment executes")
        .commit(&mut fixture.state)
        .expect("rejection should commit");

        assert_eq!(resolution.recruitment_attempt, None);
        assert!(fixture
            .state
            .recruitment()
            .attempt_for_approval_decision(request.decision)
            .is_none());
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.candidate)
                .expect("rejected candidate should remain in world")
                .organization(),
            Some(fixture.source)
        );
        validate_state(&fixture.state).expect("rejected approval state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn stale_recruitment_approval_cannot_execute_but_can_be_rejected() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let request = validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: personnel_authority(&fixture, mandate),
                target_organization: fixture.target,
                recruiter: fixture.recruiter,
                candidate: fixture.candidate,
                approach: RecruitmentApproach::Protection,
                attention: crate::core::attention::AttentionClass::Exception,
                summary: "Personnel manager requests approval before making an approach."
                    .to_owned(),
            },
        )
        .expect("approval request should validate")
        .commit(&mut fixture.state)
        .expect("approval request should commit");

        set_policy(
            &registry,
            &mut fixture.state,
            fixture.target,
            PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
        )
        .expect("organization should be able to delegate recruitment later");
        let error = match validate_resolve_decision(
            &fixture.registry,
            &fixture.state,
            request.decision,
            fixture.target,
            DecisionResponse::Approve,
        ) {
            Ok(_) => panic!("stale approval must not execute under changed authority"),
            Err(error) => error,
        };
        assert_eq!(error, DecisionError::StaleRecruitmentApprovalAuthority);
        assert!(fixture
            .state
            .recruitment()
            .attempt_for_approval_decision(request.decision)
            .is_none());

        validate_resolve_decision(
            &fixture.registry,
            &fixture.state,
            request.decision,
            fixture.target,
            DecisionResponse::Reject,
        )
        .expect("stale request should remain dismissible")
        .commit(&mut fixture.state)
        .expect("stale request rejection should commit");
        validate_state(&fixture.state).expect("dismissed stale approval state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn save_round_trip_preserves_pending_recruitment_approval() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let request = validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: personnel_authority(&fixture, mandate),
                target_organization: fixture.target,
                recruiter: fixture.recruiter,
                candidate: fixture.candidate,
                approach: RecruitmentApproach::Protection,
                attention: crate::core::attention::AttentionClass::Exception,
                summary: "Personnel manager requests approval for a recruitment approach."
                    .to_owned(),
            },
        )
        .expect("approval request should validate")
        .commit(&mut fixture.state)
        .expect("approval request should commit");

        let envelope = build_save(&registry, &fixture.state).expect("pending approval should save");
        let bytes = bincode::serialize(&envelope).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let mut restored = restore_save(&registry, decoded).expect("save should restore");
        let restored_decision = restored
            .decisions()
            .get_decision(request.decision)
            .expect("pending approval should survive save/load");
        assert_eq!(restored_decision.status(), DecisionStatus::Pending);
        match restored_decision.context() {
            DecisionContext::RecruitmentApproval(context) => {
                assert_eq!(context.candidate(), fixture.candidate);
                assert_eq!(context.recruiter(), fixture.recruiter);
            }
            DecisionContext::OperationException { .. } => {
                panic!("restored personnel approval changed context")
            }
        }
        let resolution = validate_resolve_decision(
            &registry,
            &restored,
            request.decision,
            fixture.target,
            DecisionResponse::Approve,
        )
        .expect("restored approval should remain executable")
        .commit(&mut restored)
        .expect("restored approval should commit");
        assert!(resolution.recruitment_attempt.is_some());
        validate_state(&restored).expect("restored approved recruitment state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn delegated_recruitment_persists_exact_mandate_and_policy_authority() {
        let registry = build_registry();
        let mut fixture = fixture();
        let mandate =
            assign_personnel_mandate(&registry, &mut fixture, Some(ApprovalPolicy::Delegated));
        let attempt = validate_delegated_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            personnel_authority(&fixture, mandate),
            protection_draft(&fixture),
        )
        .expect("delegated personnel recruitment should validate")
        .commit(&mut fixture.state)
        .expect("delegated personnel recruitment should commit");
        let record = fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("delegated recruitment should persist");
        assert_eq!(
            record.authority(),
            RecruitmentAuthority::Delegated {
                mandate,
                manager: fixture.recruiter,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                mandate_version: 1,
                manager_version: 1,
                policy: ApprovalPolicy::Delegated,
                policy_source: RecruitmentPolicySource::Mandate(mandate),
            }
        );
        assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
        validate_state(&fixture.state).expect("delegated recruitment state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn delegated_recruitment_token_rejects_organization_policy_change_without_mutation() {
        let registry = build_registry();
        let mut fixture = fixture();
        set_policy(
            &registry,
            &mut fixture.state,
            fixture.target,
            PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
        )
        .expect("delegated organization policy should validate");
        let mandate = assign_personnel_mandate(&registry, &mut fixture, None);
        let token = validate_delegated_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            personnel_authority(&fixture, mandate),
            protection_draft(&fixture),
        )
        .expect("organization policy should initially authorize recruitment");
        set_policy(
            &registry,
            &mut fixture.state,
            fixture.target,
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
        )
        .expect("policy revision should validate");
        let error = token
            .commit(&mut fixture.state)
            .expect_err("stale delegated recruitment must not survive policy revocation");
        assert_eq!(error, RecruitmentError::StaleRecruitmentPolicy);
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.candidate)
                .expect("candidate should persist")
                .organization(),
            Some(fixture.source)
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn protection_offer_uses_only_candidate_known_legal_pressure_and_stales_when_knowledge_changes()
    {
        let mut fixture = fixture();
        let draft = protection_draft(&fixture);
        let token = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("recruitment should validate before candidate learns new information");
        let before = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("internal decision calculation should succeed");
        assert_eq!(before.context.factors.perceived_legal_pressure(), 0);

        let information = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(fixture.candidate),
                source_kind: InformationSourceKind::PoliceContact,
                topic: InformationTopic::PoliceActivity,
                source_entity: None,
                subject: EntityRef::Character(fixture.candidate),
                observed_at: fixture.state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "The candidate learned that detectives are actively asking about them."
                    .to_owned(),
            },
        )
        .expect("candidate-held legal pressure information should validate")
        .commit(&mut fixture.state);

        let error = token
            .commit(&mut fixture.state)
            .expect_err("new candidate knowledge must invalidate an older social decision");
        assert_eq!(
            error,
            RecruitmentError::StalePressureKnowledge {
                candidate: fixture.candidate,
            }
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );

        let after = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("fresh decision should incorporate candidate-held police pressure");
        assert_eq!(after.context.pressure_information, Some(information));
        assert_eq!(after.context.factors.perceived_legal_pressure(), 100);
        let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("fresh pressure-aware recruitment should validate")
            .commit(&mut fixture.state)
            .expect("fresh pressure-aware recruitment should commit");
        let record = fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("pressure-aware attempt should persist");
        assert_eq!(record.pressure_information(), Some(information));
        assert_eq!(record.factors().perceived_legal_pressure(), 100);
        validate_state(&fixture.state).expect("pressure-aware recruitment state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn protection_offer_uses_drives_and_relationships_and_moves_accepted_candidate_atomically() {
        let mut fixture = fixture();
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.incumbent,
            relationship(15, 25, 20, 10, 20, 75, 0),
        )
        .expect("incumbent relationship should validate")
        .commit(&mut fixture.state);

        let plan = decide_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect("recruitment should produce a causal plan");
        assert_eq!(plan.context.outcome, RecruitmentOutcome::Accepted);
        assert!(plan.context.factors.drive_alignment() >= 90);
        assert!(plan.context.factors.incumbent_resentment() >= 75);
        assert!(plan.context.margin >= 0);
        let attempt = validate_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect("fresh recruitment should validate")
        .commit(&mut fixture.state)
        .expect("accepted recruitment should commit atomically");

        let candidate = fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist");
        assert_eq!(candidate.organization(), Some(fixture.target));
        assert_eq!(candidate.supervisor(), Some(fixture.recruiter));
        let record = fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("attempt should persist");
        assert_eq!(record.previous_organization(), Some(fixture.source));
        assert_eq!(record.previous_supervisor(), Some(fixture.incumbent));
        assert_eq!(record.authority(), RecruitmentAuthority::ExecutiveApproval);
        assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
        let outcome_information = fixture
            .state
            .intelligence()
            .get_information(record.outcome_information())
            .expect("recruiting organization should retain personnel outcome information");
        assert_eq!(
            outcome_information.holder(),
            KnowledgeHolder::Organization(fixture.target)
        );
        assert_eq!(outcome_information.topic(), InformationTopic::Personnel);
        assert_eq!(
            outcome_information.source_entity(),
            Some(EntityRef::Character(fixture.recruiter))
        );
        assert_eq!(
            outcome_information.subject(),
            EntityRef::Character(fixture.candidate)
        );
        assert!(!fixture
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(fixture.source))
            .any(|information| information.id() == record.outcome_information()));
        let history = fixture
            .state
            .history()
            .get_event(
                record
                    .history_event()
                    .expect("acceptance should create history"),
            )
            .expect("recruitment history should persist");
        assert_eq!(history.kind(), HistoryEventKind::Recruitment);
        assert!(history
            .entities()
            .contains(&EntityRef::Character(fixture.candidate)));
        assert!(history
            .entities()
            .contains(&EntityRef::Character(fixture.recruiter)));
        assert!(history
            .entities()
            .contains(&EntityRef::Organization(fixture.target)));
        let departure_reports: Vec<_> = fixture
            .state
            .reports()
            .reports_for(fixture.source)
            .filter(|report| report.kind() == ReportKind::AfterAction)
            .filter(|report| report.title() == "Personnel change")
            .collect();
        assert_eq!(departure_reports.len(), 1);
        assert_eq!(departure_reports[0].entries().len(), 1);
        assert!(departure_reports[0].entries()[0]
            .summary
            .contains("Frightened Associate left North Crew"));
        assert!(departure_reports[0].entries()[0]
            .entities
            .contains(&EntityRef::Character(fixture.candidate)));
        assert!(!departure_reports[0].entries()[0]
            .entities
            .contains(&EntityRef::Organization(fixture.target)));
        validate_state(&fixture.state).expect("accepted recruitment state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn persisted_relationship_snapshots_keep_recruitment_history_valid_after_social_change() {
        let mut fixture = fixture();
        let recruiter_dimensions = relationship(55, 65, 10, 35, 15, 5, 10);
        let incumbent_dimensions = relationship(20, 30, 15, 10, 25, 70, 0);
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.incumbent,
            incumbent_dimensions,
        )
        .expect("incumbent relationship should validate")
        .commit(&mut fixture.state);
        let attempt = validate_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect("recruitment should validate")
        .commit(&mut fixture.state)
        .expect("recruitment should commit");

        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.recruiter,
            relationship(5, 10, 80, 0, 0, 60, 0),
        )
        .expect("later recruiter relationship change should validate")
        .commit(&mut fixture.state);
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.incumbent,
            relationship(80, 80, 0, 60, 60, 5, 0),
        )
        .expect("later incumbent relationship change should validate")
        .commit(&mut fixture.state);

        let record = fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("recruitment attempt should persist");
        assert_eq!(
            record.recruiter_relationship().dimensions(),
            Some(recruiter_dimensions)
        );
        assert_eq!(
            record
                .incumbent_relationship()
                .expect("historical incumbent snapshot should persist")
                .dimensions(),
            Some(incumbent_dimensions)
        );
        assert_ne!(
            fixture
                .state
                .social()
                .get_relationship(fixture.candidate, fixture.recruiter)
                .expect("current recruiter relationship should exist")
                .dimensions(),
            recruiter_dimensions
        );
        validate_state(&fixture.state)
            .expect("later social changes must not invalidate recruitment history");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn strong_incumbent_attachment_can_produce_refusal_without_membership_mutation() {
        let mut fixture = fixture();
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.incumbent,
            relationship(95, 95, 10, 85, 90, 0, 0),
        )
        .expect("strong incumbent relationship should validate")
        .commit(&mut fixture.state);
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.recruiter,
            relationship(10, 20, 30, 5, 0, 0, 0),
        )
        .expect("weak recruiter relationship should validate")
        .commit(&mut fixture.state);
        let draft = RecruitmentDraft {
            approach: RecruitmentApproach::Advancement,
            ..protection_draft(&fixture)
        };
        let plan = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("valid recruitment should still produce a refusal plan");
        assert_eq!(plan.context.outcome, RecruitmentOutcome::Refused);
        assert!(plan.context.factors.incumbent_attachment() >= 90);
        let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
            .expect("refusal should validate")
            .commit(&mut fixture.state)
            .expect("refusal should persist without moving candidate");
        let candidate = fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist");
        assert_eq!(candidate.organization(), Some(fixture.source));
        assert_eq!(candidate.supervisor(), Some(fixture.incumbent));
        assert_eq!(
            fixture
                .state
                .recruitment()
                .get_attempt(attempt)
                .expect("refusal should persist")
                .history_event(),
            None
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn recruitment_cooldown_blocks_spam_and_allows_a_later_social_reassessment() {
        let mut accepted_fixture = fixture();
        let draft = protection_draft(&accepted_fixture);
        let first = validate_recruitment_attempt(
            &accepted_fixture.registry,
            &accepted_fixture.state,
            draft,
        )
        .expect("initial recruitment should validate")
        .commit(&mut accepted_fixture.state)
        .expect("initial recruitment should commit");
        let first_record = accepted_fixture
            .state
            .recruitment()
            .get_attempt(first)
            .expect("first recruitment should persist");
        let expected_next =
            first_record.occurred_at() + accepted_fixture.registry.recruitment().cooldown();
        assert_eq!(
            decide_recruitment_attempt(&accepted_fixture.registry, &accepted_fixture.state, draft)
                .expect_err("immediate retry must fail"),
            RecruitmentError::CandidateAlreadyMember {
                candidate: accepted_fixture.candidate,
                organization: accepted_fixture.target,
            }
        );

        let mut refusal_fixture = fixture();
        validate_set_relationship(
            &refusal_fixture.state,
            refusal_fixture.candidate,
            refusal_fixture.incumbent,
            relationship(100, 100, 0, 100, 100, 0, 0),
        )
        .expect("attachment relationship should validate")
        .commit(&mut refusal_fixture.state);
        let refusal_draft = RecruitmentDraft {
            approach: RecruitmentApproach::Advancement,
            ..protection_draft(&refusal_fixture)
        };
        validate_recruitment_attempt(
            &refusal_fixture.registry,
            &refusal_fixture.state,
            refusal_draft,
        )
        .expect("refusal should validate")
        .commit(&mut refusal_fixture.state)
        .expect("refusal should commit");
        let error = decide_recruitment_attempt(
            &refusal_fixture.registry,
            &refusal_fixture.state,
            refusal_draft,
        )
        .expect_err("refused candidate should not be rerolled immediately");
        assert_eq!(
            error,
            RecruitmentError::Cooldown {
                candidate: refusal_fixture.candidate,
                organization: refusal_fixture.target,
                next_eligible_at: expected_next,
            }
        );
        refusal_fixture
            .state
            .advance_clock(refusal_fixture.registry.recruitment().cooldown());
        decide_recruitment_attempt(
            &refusal_fixture.registry,
            &refusal_fixture.state,
            refusal_draft,
        )
        .expect("candidate should become approachable when cooldown expires");
        validate_invariants(&refusal_fixture.state);
    }

    #[test]
    fn relationship_change_invalidates_validated_attempt_without_partial_mutation() {
        let mut fixture = fixture();
        let token = validate_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect("recruitment should initially validate");
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.recruiter,
            relationship(5, 5, 80, 0, 0, 0, 0),
        )
        .expect("relationship mutation should validate")
        .commit(&mut fixture.state);
        let error = token
            .commit(&mut fixture.state)
            .expect_err("stale social decision must not commit");
        assert!(matches!(error, RecruitmentError::StaleRelationship { .. }));
        let candidate = fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should remain present");
        assert_eq!(candidate.organization(), Some(fixture.source));
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn canonical_world_dependencies_block_poaching_a_manager_with_direct_reports() {
        let registry = build_registry();
        let mut fixture = fixture();
        let subordinate = insert_character(
            &registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Dependent Soldier".to_owned(),
                organization: Some(fixture.source),
                supervisor: Some(fixture.candidate),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("subordinate should validate");
        let error = decide_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect_err("manager must hand off direct reports before defecting");
        assert_eq!(
            error,
            RecruitmentError::World(WorldError::DirectReportAssignment {
                character: fixture.candidate,
                direct_report: subordinate,
            })
        );
        assert_eq!(
            fixture
                .state
                .recruitment()
                .attempts_for_candidate(fixture.candidate)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn save_round_trip_preserves_recruitment_history_and_drive_authorship() {
        let registry = build_registry();
        assert_eq!(
            registry.get_drive(DriveKind::Safety).display_name(),
            "Safety"
        );
        let mut fixture = fixture();
        let attempt = validate_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            protection_draft(&fixture),
        )
        .expect("recruitment should validate")
        .commit(&mut fixture.state)
        .expect("recruitment should commit");
        let envelope = build_save(&registry, &fixture.state).expect("state should save");
        let bytes = bincode::serialize(&envelope).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let restored = restore_save(&registry, decoded).expect("save should restore");
        let record = restored
            .recruitment()
            .get_attempt(attempt)
            .expect("recruitment attempt should survive save/load");
        assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
        assert_eq!(
            restored
                .world()
                .get_character(fixture.candidate)
                .expect("candidate should survive save/load")
                .organization(),
            Some(fixture.target)
        );
        assert!(record
            .history_event()
            .and_then(|history| restored.history().get_event(history))
            .is_some());
        validate_state(&restored).expect("restored recruitment state should validate");
        validate_invariants(&restored);
    }
}
