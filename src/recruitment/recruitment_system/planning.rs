//! Read-only recruitment planning and frozen-dependency revalidation.
//!
//! The parent module owns authority channels and the canonical recruitment commit. This child
//! owns only deterministic plan construction and the checks that prove a held plan still
//! describes the same personnel, relationships, knowledge, reputation, and recruitment history.

use super::{
    RecruitmentError, RecruitmentPlan, RecruitmentPlanContext, RecruitmentPlanDependencies,
    validate_recruitment_request, validate_recruitment_request_base,
};
use crate::core::id::DecisionRequestId;
use crate::core::state::AppState;
use crate::recruitment::scoring::{
    candidate_pressure_information_ids, resolve_perceived_legal_pressure_at,
    resolve_perceived_legal_pressure_from_ids, resolve_recruitment_factors_from_context,
    resolve_recruitment_margin, resolve_recruitment_outcome,
};
use crate::recruitment::{
    RecruitmentApproach, RecruitmentDraft, RecruitmentRelationshipSnapshot,
    build_recruitment_relationship_snapshot,
};
use crate::registry::{RecruitmentDefinition, Registry};

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

pub(crate) fn decide_recruitment_attempt(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentDraft,
) -> Result<RecruitmentPlan, RecruitmentError> {
    decide_recruitment_attempt_with_approval(registry, state, draft, None)
}

pub(super) fn decide_recruitment_attempt_with_approval(
    registry: &Registry,
    state: &AppState,
    draft: RecruitmentDraft,
    allowed_recruitment_approval: Option<DecisionRequestId>,
) -> Result<RecruitmentPlan, RecruitmentError> {
    let (candidate, recruiter) =
        validate_recruitment_request(registry, state, draft, allowed_recruitment_approval)?;
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
            registry.information_quality(),
            registry.recruitment(),
            state,
            &pressure_information_snapshot,
            state.now(),
        );
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

pub(super) fn validate_plan_state_snapshot(
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<(), RecruitmentError> {
    crate::core::time::ensure_time_current(state.now(), plan.context.occurred_at)
        .map_err(|(expected, found)| RecruitmentError::StaleTime { expected, found })?;
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
    if let Some(arrest) = state
        .legal
        .active_arrest_for_character(plan.draft.recruiter)
    {
        return Err(RecruitmentError::DetainedRecruiter {
            recruiter: plan.draft.recruiter,
            arrest: arrest.id(),
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

pub(super) fn validate_plan_definition(
    registry: &Registry,
    state: &AppState,
    plan: &RecruitmentPlan,
) -> Result<(), RecruitmentError> {
    let definition = registry.recruitment();
    let candidate = state
        .world
        .get_character(plan.draft.candidate)
        .ok_or(RecruitmentError::MissingCandidate(plan.draft.candidate))?;
    let recruiter = state
        .world
        .get_character(plan.draft.recruiter)
        .ok_or(RecruitmentError::MissingRecruiter(plan.draft.recruiter))?;
    let (pressure_information, perceived_legal_pressure) = resolve_perceived_legal_pressure_at(
        registry.information_quality(),
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
