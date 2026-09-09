//! Release-safe structural validation for the recruitment subsystem.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::decisions::{DecisionContext, DecisionResponse, DecisionStatus};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::recruitment::recruitment_system::{
    RecruitmentFactorContext, recruitment_defection_history_summary,
    recruitment_join_history_summary, recruitment_member_report_entities,
    recruitment_member_report_summary, recruitment_member_report_title,
    recruitment_outcome_summary,
};
use crate::recruitment::scoring::{
    resolve_perceived_legal_pressure_at, resolve_recruitment_factors_from_context,
    resolve_recruitment_margin, resolve_recruitment_outcome,
};
use crate::recruitment::{
    RecruitmentAttemptRecord, RecruitmentAuthority, RecruitmentOutcome, RecruitmentPolicySource,
};
use crate::registry::Registry;
use crate::reports::ReportKind;
use crate::world::{ApprovalPolicy, CharacterRecord, OrganizationKind, OrganizationRecord};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct SeenRecruitmentArtifacts {
    previous_attempt_by_pair: BTreeMap<(CharacterId, OrganizationId), crate::core::time::SimTime>,
    history_events: BTreeSet<crate::core::id::HistoryEventId>,
    outcome_information: BTreeSet<crate::core::id::InformationId>,
    member_reports: BTreeSet<crate::core::id::ReportId>,
}

struct RecruitmentAttemptRefs<'a> {
    candidate: &'a CharacterRecord,
    recruiter: &'a CharacterRecord,
    target: &'a OrganizationRecord,
}

pub(super) fn validate_recruitment(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen = SeenRecruitmentArtifacts::default();
    for attempt in state.recruitment.attempts() {
        validate_recruitment_attempt(state, attempt, &mut seen)?;
    }
    Ok(())
}

fn validate_recruitment_attempt(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    let refs = resolve_attempt_references(state, attempt)?;
    validate_attempt_base(state, attempt, &refs)?;
    validate_relationship_snapshots(attempt)?;
    validate_attempt_authority(state, attempt, refs.recruiter)?;
    validate_pressure_information(state, attempt)?;
    validate_outcome_information(state, attempt, &refs, seen)?;
    validate_member_report(state, attempt, &refs, seen)?;
    validate_factor_bounds(attempt, refs.candidate)?;
    validate_attempt_chronology(attempt, seen)?;
    validate_outcome_consequence(state, attempt, &refs, seen)
}

fn resolve_attempt_references<'a>(
    state: &'a AppState,
    attempt: &RecruitmentAttemptRecord,
) -> Result<RecruitmentAttemptRefs<'a>, StateValidationError> {
    let invalid = || invalid_attempt(attempt);
    Ok(RecruitmentAttemptRefs {
        candidate: state
            .world
            .get_character(attempt.candidate())
            .ok_or_else(invalid)?,
        recruiter: state
            .world
            .get_character(attempt.recruiter())
            .ok_or_else(invalid)?,
        target: state
            .world
            .get_organization(attempt.target_organization())
            .ok_or_else(invalid)?,
    })
}

fn validate_attempt_base(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    refs: &RecruitmentAttemptRefs<'_>,
) -> Result<(), StateValidationError> {
    if refs.candidate.id() == refs.recruiter.id()
        || refs.target.kind() != OrganizationKind::Criminal
        || attempt.occurred_at() > state.now()
        || attempt.previous_organization() == Some(attempt.target_organization())
        || attempt
            .previous_supervisor()
            .is_some_and(|supervisor| state.world.get_character(supervisor).is_none())
        || (attempt.previous_supervisor().is_some() && attempt.previous_organization().is_none())
    {
        return Err(invalid_attempt(attempt));
    }
    if let Some(previous_organization) = attempt.previous_organization() {
        let previous = state
            .world
            .get_organization(previous_organization)
            .ok_or_else(|| invalid_attempt(attempt))?;
        if previous.kind() != OrganizationKind::Criminal {
            return Err(invalid_attempt(attempt));
        }
    }
    Ok(())
}

fn validate_member_report(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    refs: &RecruitmentAttemptRefs<'_>,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    let Some(incumbent_organization) = attempt.previous_organization() else {
        return if attempt.member_report().is_none() {
            Ok(())
        } else {
            Err(invalid_attempt(attempt))
        };
    };
    let report_id = attempt
        .member_report()
        .ok_or_else(|| invalid_attempt(attempt))?;
    if !seen.member_reports.insert(report_id) {
        return Err(invalid_attempt(attempt));
    }
    let report = state
        .reports
        .get_report(report_id)
        .ok_or_else(|| invalid_attempt(attempt))?;
    let incumbent = state
        .world
        .get_organization(incumbent_organization)
        .ok_or_else(|| invalid_attempt(attempt))?;
    let expected_summary = recruitment_member_report_summary(
        attempt.outcome(),
        refs.candidate.name(),
        incumbent.name(),
        (attempt.outcome() == RecruitmentOutcome::Refused)
            .then_some((refs.recruiter.name(), refs.target.name())),
    );
    let expected_entities = recruitment_member_report_entities(
        attempt.outcome(),
        attempt.candidate(),
        attempt.recruiter(),
        attempt.target_organization(),
        incumbent_organization,
    );
    let Some(entry) = report.entries().first() else {
        return Err(invalid_attempt(attempt));
    };
    if report.recipient() != incumbent_organization
        || report.kind() != ReportKind::AfterAction
        || report.title() != recruitment_member_report_title(attempt.outcome())
        || report.generated_at() != attempt.occurred_at()
        || report.entries().len() != 1
        || entry.attention != AttentionClass::Notable
        || entry.summary != expected_summary
        || !entry.sources.is_empty()
        || entry.entities != expected_entities
        || entry.decision.is_some()
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_relationship_snapshots(
    attempt: &RecruitmentAttemptRecord,
) -> Result<(), StateValidationError> {
    let recruiter_relationship = attempt.recruiter_relationship();
    if recruiter_relationship.from() != attempt.candidate()
        || recruiter_relationship.to() != attempt.recruiter()
        || recruiter_relationship.dimensions().is_none()
        || recruiter_relationship.version().is_none()
        || recruiter_relationship.version() == Some(0)
    {
        return Err(invalid_attempt(attempt));
    }
    match (
        attempt.previous_supervisor(),
        attempt.incumbent_relationship(),
    ) {
        (None, None) => Ok(()),
        (Some(supervisor), Some(snapshot)) => {
            let snapshot_shape_is_valid = match (snapshot.dimensions(), snapshot.version()) {
                (Some(_), Some(version)) => version > 0,
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
            };
            if snapshot.from() != attempt.candidate()
                || snapshot.to() != supervisor
                || !snapshot_shape_is_valid
            {
                return Err(invalid_attempt(attempt));
            }
            Ok(())
        }
        (None, Some(_)) | (Some(_), None) => Err(invalid_attempt(attempt)),
    }
}

fn validate_attempt_authority(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    recruiter: &CharacterRecord,
) -> Result<(), StateValidationError> {
    match attempt.authority() {
        RecruitmentAuthority::ExecutiveApproval => Ok(()),
        RecruitmentAuthority::ApprovedDecision {
            decision,
            mandate,
            manager,
            scope,
            mandate_version,
            manager_version,
            policy: _,
            policy_source,
        } => {
            let invalid = || invalid_attempt(attempt);
            let decision_record = state.decisions.get_decision(decision).ok_or_else(invalid)?;
            let approval_context = match decision_record.context() {
                DecisionContext::RecruitmentApproval(context) => context,
                DecisionContext::OperationPoliceArrival { .. } => return Err(invalid()),
            };
            let approval_resolution = decision_record.resolution().ok_or_else(invalid)?;
            if !recruitment_authority_snapshot_is_valid(
                state,
                attempt,
                recruiter.version(),
                attempt.authority(),
                ApprovalPolicy::RequireApproval,
            ) || decision_record.status() != DecisionStatus::Resolved
                || decision_record.requester() != attempt.recruiter()
                || decision_record.recipient() != attempt.target_organization()
                || approval_resolution.response() != DecisionResponse::Approve
                || approval_resolution.resolved_at() != attempt.occurred_at()
                || approval_context.target_organization() != attempt.target_organization()
                || approval_context.recruiter() != attempt.recruiter()
                || approval_context.candidate() != attempt.candidate()
                || approval_context.approach() != attempt.approach()
                || approval_context.authority().authority().mandate != mandate
                || approval_context.authority().authority().manager != manager
                || approval_context.authority().authority().scope != scope
                || approval_context.authority().mandate_version() != mandate_version
                || approval_context.authority().manager_version() != manager_version
                || approval_context.authority().policy_source() != policy_source
                || state
                    .recruitment
                    .get_attempt_for_approval_decision(decision)
                    .map(|record| record.id())
                    != Some(attempt.id())
            {
                return Err(invalid());
            }
            Ok(())
        }
        RecruitmentAuthority::Delegated { .. } => {
            if !recruitment_authority_snapshot_is_valid(
                state,
                attempt,
                recruiter.version(),
                attempt.authority(),
                ApprovalPolicy::Delegated,
            ) {
                return Err(invalid_attempt(attempt));
            }
            Ok(())
        }
    }
}

fn validate_pressure_information(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
) -> Result<(), StateValidationError> {
    let Some(information_id) = attempt.pressure_information() else {
        return Ok(());
    };
    let information = state
        .intelligence
        .get_information(information_id)
        .ok_or_else(|| invalid_attempt(attempt))?;
    if information.holder() != KnowledgeHolder::Character(attempt.candidate())
        || information.topic() != InformationTopic::PoliceActivity
        || information.subject() != EntityRef::Character(attempt.candidate())
        || information.recorded_at() > attempt.occurred_at()
        || information.observed_at() > attempt.occurred_at()
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_outcome_information(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    refs: &RecruitmentAttemptRefs<'_>,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    let information = state
        .intelligence
        .get_information(attempt.outcome_information())
        .ok_or_else(|| invalid_attempt(attempt))?;
    let expected_summary = recruitment_outcome_summary(
        refs.candidate.name(),
        refs.recruiter.name(),
        refs.target.name(),
        attempt.outcome(),
    );
    if !seen
        .outcome_information
        .insert(attempt.outcome_information())
        || information.holder() != KnowledgeHolder::Organization(attempt.target_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::Personnel
        || information.source_entity() != Some(EntityRef::Character(attempt.recruiter()))
        || information.subject() != EntityRef::Character(attempt.candidate())
        || information.observed_at() != attempt.occurred_at()
        || information.recorded_at() != attempt.occurred_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || !information.derived_from().is_empty()
        || information.summary() != expected_summary
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_factor_bounds(
    attempt: &RecruitmentAttemptRecord,
    candidate: &CharacterRecord,
) -> Result<(), StateValidationError> {
    let factors = attempt.factors();
    if attempt.resulting_candidate_version() == 0
        || attempt.resulting_candidate_version() > candidate.version()
        || factors.recruiter_influence() > 100
        || factors.drive_alignment() > 100
        || factors.relationship_support() > 100
        || factors.incumbent_attachment() > 100
        || factors.incumbent_resentment() > 100
        || factors.perceived_legal_pressure() > 100
        || attempt.outcome() != resolve_recruitment_outcome(attempt.margin())
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_attempt_chronology(
    attempt: &RecruitmentAttemptRecord,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    let pair = (attempt.candidate(), attempt.target_organization());
    if let Some(previous_time) = seen
        .previous_attempt_by_pair
        .insert(pair, attempt.occurred_at())
        && attempt.occurred_at() < previous_time
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_outcome_consequence(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    refs: &RecruitmentAttemptRefs<'_>,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    match attempt.outcome() {
        RecruitmentOutcome::Accepted => validate_accepted_outcome(state, attempt, refs, seen),
        RecruitmentOutcome::Refused => validate_refused_outcome(attempt, refs.candidate),
    }
}

fn validate_accepted_outcome(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    refs: &RecruitmentAttemptRefs<'_>,
    seen: &mut SeenRecruitmentArtifacts,
) -> Result<(), StateValidationError> {
    let history_id = attempt
        .history_event()
        .ok_or_else(|| invalid_attempt(attempt))?;
    if !seen.history_events.insert(history_id) {
        return Err(invalid_attempt(attempt));
    }
    let history = state
        .history
        .get_event(history_id)
        .ok_or_else(|| invalid_attempt(attempt))?;
    let expected_summary = if attempt.previous_organization().is_some() {
        recruitment_defection_history_summary(refs.candidate.name())
    } else {
        recruitment_join_history_summary(
            refs.candidate.name(),
            refs.target.name(),
            refs.recruiter.name(),
        )
    };
    if history.kind() != HistoryEventKind::Recruitment
        || history.occurred_at() != attempt.occurred_at()
        || history.summary() != expected_summary
        || !history
            .entities()
            .contains(&EntityRef::Character(attempt.candidate()))
        || (attempt.previous_organization().is_none()
            && !history
                .entities()
                .contains(&EntityRef::Character(attempt.recruiter())))
        || (attempt.previous_organization().is_some()
            && history
                .entities()
                .contains(&EntityRef::Character(attempt.recruiter())))
        || history
            .entities()
            .contains(&EntityRef::Organization(attempt.target_organization()))
            == attempt.previous_organization().is_some()
        || (refs.candidate.version() == attempt.resulting_candidate_version()
            && (refs.candidate.organization() != Some(attempt.target_organization())
                || refs.candidate.supervisor() != Some(attempt.recruiter())))
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn validate_refused_outcome(
    attempt: &RecruitmentAttemptRecord,
    candidate: &CharacterRecord,
) -> Result<(), StateValidationError> {
    if attempt.history_event().is_some()
        || (candidate.version() == attempt.resulting_candidate_version()
            && (candidate.organization() != attempt.previous_organization()
                || candidate.supervisor() != attempt.previous_supervisor()))
    {
        return Err(invalid_attempt(attempt));
    }
    Ok(())
}

fn invalid_attempt(attempt: &RecruitmentAttemptRecord) -> StateValidationError {
    StateValidationError::InvalidRecruitmentAttempt {
        attempt: attempt.id(),
    }
}

fn recruitment_authority_snapshot_is_valid(
    state: &AppState,
    attempt: &RecruitmentAttemptRecord,
    recruiter_version: u32,
    authority: RecruitmentAuthority,
    expected_policy: ApprovalPolicy,
) -> bool {
    let (mandate, manager, scope, mandate_version, manager_version, policy, policy_source) =
        match authority {
            RecruitmentAuthority::ApprovedDecision {
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
                ..
            }
            | RecruitmentAuthority::Delegated {
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            } => (
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            ),
            RecruitmentAuthority::ExecutiveApproval => return false,
        };
    let Some(mandate_record) = state.delegation.get_mandate(mandate) else {
        return false;
    };
    let personnel_scope = crate::delegation::ResponsibilityScope::Function(
        crate::delegation::ResponsibilityFunction::Personnel,
    );
    if manager != attempt.recruiter()
        || mandate_record.manager() != manager
        || mandate_record.organization() != attempt.target_organization()
        || scope != personnel_scope
        || mandate_version == 0
        || mandate_version > mandate_record.version()
        || manager_version == 0
        || manager_version > recruiter_version
        || policy != expected_policy
        || (mandate_version == mandate_record.version()
            && !mandate_record.scopes().contains(&personnel_scope))
    {
        return false;
    }
    match policy_source {
        RecruitmentPolicySource::Organization(organization) => {
            organization == attempt.target_organization()
        }
        RecruitmentPolicySource::Mandate(source_mandate) => source_mandate == mandate,
    }
}

pub(super) fn validate_recruitment_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let definition = registry.recruitment();
    let mut previous_attempt_by_pair: BTreeMap<
        (CharacterId, OrganizationId),
        crate::core::time::SimTime,
    > = BTreeMap::new();
    for attempt in state.recruitment.attempts() {
        let candidate = state.world.get_character(attempt.candidate()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let recruiter = state.world.get_character(attempt.recruiter()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let (expected_pressure_information, expected_legal_pressure) =
            resolve_perceived_legal_pressure_at(
                definition,
                state,
                attempt.candidate(),
                attempt.occurred_at(),
            );
        let expected_factors = resolve_recruitment_factors_from_context(RecruitmentFactorContext {
            definition,
            candidate,
            recruiter,
            approach: attempt.approach(),
            recruiter_relationship: attempt.recruiter_relationship(),
            incumbent_relationship: attempt.incumbent_relationship(),
            perceived_legal_pressure: expected_legal_pressure,
            // The attempt's own frozen reputation read: impressions legitimately move after
            // an attempt, so re-deriving with current standing would falsify history.
            organization_competence: attempt.factors().organization_competence(),
            had_previous_organization: attempt.previous_organization().is_some(),
        });
        if expected_factors != Some(attempt.factors())
            || attempt.pressure_information() != expected_pressure_information
            || attempt.margin()
                != resolve_recruitment_margin(definition, attempt.factors(), attempt.approach())
            || attempt.outcome() != resolve_recruitment_outcome(attempt.margin())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let pair = (attempt.candidate(), attempt.target_organization());
        if let Some(previous_time) = previous_attempt_by_pair.insert(pair, attempt.occurred_at())
            && previous_time
                .checked_add(definition.cooldown())
                .is_none_or(|next_eligible| attempt.occurred_at() < next_eligible)
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }
    }
    Ok(())
}
