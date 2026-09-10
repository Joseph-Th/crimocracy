//! Recruitment-owned history, information, and incumbent-organization report construction.

use super::RecruitmentOutcome;
use super::recruitment_system::{RecruitmentError, RecruitmentPlan};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::state::AppState;
use crate::history::history_system::{ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{ValidatedInformation, validate_record_information};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::BTreeSet;

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
        RecruitmentOutcome::Accepted => format!(
            "{candidate} accepted {recruiter}'s recruitment approach and joined {organization}."
        ),
        RecruitmentOutcome::Refused => format!(
            "{candidate} refused {recruiter}'s recruitment approach on behalf of {organization}."
        ),
    }
}

/// Campaign history for an accepted attempt. A defection deliberately omits the destination
/// organization and recruiter so global history cannot reveal where the former member landed.
pub(super) fn validate_recruitment_history_event(
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

/// The recruiting organization's own direct personnel knowledge of the attempt's outcome.
pub(super) fn validate_recruitment_outcome_information(
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

/// Player-facing report to the candidate's incumbent organization: a departure notice after an
/// accepted defection, or a loyalty report naming the outside approach after a refusal.
pub(super) fn validate_recruitment_member_report(
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
