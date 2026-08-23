//! Investigator-held case-activity knowledge: the institutional side of counterintelligence.
//!
//! When a detective takes over a case they personally know its activity status, and when the
//! institution shelves or closes that case the knowledge is refreshed. The knowledge lives as
//! ordinary provenance-bearing information held by the investigator character, so every consumer
//! — a police-channel institutional contact, a surveillance read of the precinct, a future
//! informant — reaches it through the canonical intelligence paths instead of case-graph reads.
//! Summaries carry stable activity markers ("actively developing", "shelved", "closed") so
//! player-facing readers can parse the sightline without hidden state.

use crate::core::entity::EntityRef;
use crate::core::id::{InformationId, InvestigationId};
use crate::core::state::AppState;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::InvestigationStatus;
use crate::world::OrganizationKind;

/// The activity signal a case's lead investigator personally holds about their own case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseActivityStatus {
    Active,
    Shelved,
    Closed,
}

impl CaseActivityStatus {
    fn from_status(status: InvestigationStatus) -> Option<Self> {
        match status {
            InvestigationStatus::Active => Some(Self::Active),
            InvestigationStatus::Suspended => Some(Self::Shelved),
            InvestigationStatus::Closed => Some(Self::Closed),
        }
    }

    fn summary(self, authority_name: &str, case_title: &str) -> String {
        match self {
            Self::Active => format!(
        "{authority_name} detectives are still actively developing the case \"{case_title}\"."
      ),
            Self::Shelved => {
                format!("{authority_name} has already shelved the case \"{case_title}\".")
            }
            Self::Closed => format!("{authority_name} has closed the case \"{case_title}\"."),
        }
    }

    /// Whether the authority still visibly works the matter. `None` for an unknown status so a
    /// parsed sightline never invents certainty the summary did not carry.
    pub fn is_hot(self) -> Option<bool> {
        match self {
            Self::Active => Some(true),
            Self::Shelved | Self::Closed => Some(false),
        }
    }

    /// Parses a player-visible case-activity summary into its activity marker. Both
    /// counterintelligence channels (precinct surveillance and contact disclosure) phrase their
    /// summaries with these exact markers so no reader needs hidden state.
    pub fn parse_summary_marker(summary: &str) -> Option<Self> {
        if summary.contains("actively developing") {
            Some(Self::Active)
        } else if summary.contains("shelved") {
            Some(Self::Shelved)
        } else if summary.contains("has closed") {
            Some(Self::Closed)
        } else {
            None
        }
    }
}

/// Records (or refreshes) the lead investigator's personal knowledge of a case's activity.
/// Returns `None` when the case has no lead to hold the knowledge; an unstaffed case has no
/// institutional knower yet. A fresh material state of the same case produces a fresh information
/// record, so a contact channel can disclose each new development exactly once.
pub(crate) fn record_lead_case_activity_knowledge(
    state: &mut AppState,
    investigation: InvestigationId,
) -> Option<InformationId> {
    let record = state.legal.get_investigation(investigation)?;
    let lead = record.lead_investigator()?;
    let status = CaseActivityStatus::from_status(record.status())?;
    let owner = record.owner();
    let authority_kind = state.world.get_organization(owner).map(|org| org.kind());
    if authority_kind != Some(OrganizationKind::LawEnforcement) {
        return None;
    }
    let authority_name = state
        .world
        .get_organization(owner)
        .expect("checked organization must exist")
        .name()
        .to_owned();
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
        summary: status.summary(&authority_name, &case_title),
    };
    validate_record_information(state, draft)
        .ok()?
        .commit(state)
        .ok()
}

#[cfg(test)]
mod tests;
