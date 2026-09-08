//! Investigator-held case-activity knowledge: the institutional side of counterintelligence.
//!
//! When a detective takes over a case they personally know its activity status, and when the
//! institution shelves or closes that case the knowledge is refreshed. The knowledge lives as
//! ordinary provenance-bearing information held by the investigator character, so every consumer
//! — a police-channel institutional contact, a surveillance read of the precinct, a future
//! informant — reaches it through the canonical intelligence paths instead of case-graph reads.
//! Summaries use consistent activity phrasing for presentation. Authoritative status remains the
//! typed `InvestigationStatus` on the legal owner; prose is never parsed back into domain state.

use crate::core::entity::EntityRef;
use crate::core::id::CharacterId;
use crate::core::id::InvestigationId;
use crate::core::state::AppState;
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_information_with_signal,
};
use crate::intelligence::{
    CaseActivitySignal, InformationDraft, InformationSignal, InformationSourceKind,
    InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::InvestigationStatus;
use crate::world::OrganizationKind;

/// Shared display prefix for case-activity summaries. This is presentation consistency only;
/// consumers reason from `InformationSignal::CaseActivity`, never from this prose.
pub(crate) fn case_activity_summary_prefix(activity: CaseActivitySignal) -> &'static str {
    match activity {
        CaseActivitySignal::Active => "Case activity: actively developing.",
        CaseActivitySignal::Shelved => "Case activity: shelved.",
        CaseActivitySignal::Closed => "Case activity: has closed.",
    }
}

fn case_activity_summary(
    activity: CaseActivitySignal,
    authority_name: &str,
    case_title: &str,
) -> String {
    format!(
        "{} {}",
        case_activity_summary_prefix(activity),
        match activity {
            CaseActivitySignal::Active =>
                format!("{authority_name} detectives are still working the case \"{case_title}\"."),
            CaseActivitySignal::Shelved => {
                format!("{authority_name} has already shelved the case \"{case_title}\".")
            }
            CaseActivitySignal::Closed => {
                format!("{authority_name} has closed the case \"{case_title}\".")
            }
        }
    )
}

/// Builds (but does not commit) the validated refresh of a case lead's personal knowledge of
/// their case's activity. The caller names the incoming status and the seat holder so a
/// lifecycle transition or staffing commit can prepare the knowledge before mutating anything
/// and only then commit it, keeping one canonical path per record. Returns `None` only when the
/// case's authority is not law enforcement. A fresh material state of the same case produces a
/// fresh information record, so a contact channel can disclose each new development exactly once.
pub(crate) fn prepare_case_activity_knowledge(
    state: &AppState,
    investigation: InvestigationId,
    activity: CaseActivitySignal,
    lead: CharacterId,
) -> Result<Option<ValidatedInformation>, crate::intelligence::intelligence_system::IntelligenceError>
{
    let record = state
        .legal
        .get_investigation(investigation)
        .expect("case-activity knowledge must reference a persisted investigation");
    let owner = record.owner();
    let organization = state
        .world
        .get_organization(owner)
        .expect("investigation owner must reference a persisted organization");
    if organization.kind() != OrganizationKind::LawEnforcement {
        return Ok(None);
    }
    let authority_name = organization.name().to_owned();
    let case_title = record.title().to_owned();
    let subject = EntityRef::Organization(owner);
    let draft = InformationDraft {
        holder: KnowledgeHolder::Character(lead),
        source_kind: InformationSourceKind::DirectObservation,
        topic: InformationTopic::LegalActivity,
        source_entity: Some(subject),
        subject,
        observed_at: state.now(),
        reliability: Reliability::DirectAccess,
        specificity: Specificity::Specific,
        summary: case_activity_summary(activity, &authority_name, &case_title),
    };
    validate_record_information_with_signal(state, draft, InformationSignal::CaseActivity(activity))
        .map(Some)
}

/// Convenience mapping for callers that hold an `InvestigationStatus` (for example the
/// lifecycle transition path) and need the matching activity signal.
pub(crate) fn activity_for_status(status: InvestigationStatus) -> CaseActivitySignal {
    match status {
        InvestigationStatus::Active => CaseActivitySignal::Active,
        InvestigationStatus::Suspended => CaseActivitySignal::Shelved,
        InvestigationStatus::Closed => CaseActivitySignal::Closed,
    }
}

#[cfg(test)]
mod tests;
