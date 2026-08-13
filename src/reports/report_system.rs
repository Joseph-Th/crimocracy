//! Report validation and insertion; reports expose known information rather than world truth.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{DecisionRequestId, InformationId, OrganizationId, ReportId};
use crate::core::state::AppState;
use crate::reports::{ReportDraft, ReportRecord};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReportError {
    #[error("report title must not be empty")]
    EmptyTitle,
    #[error("report entry {0} has an empty summary")]
    EmptyEntry(usize),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("decision request {0} does not exist")]
    MissingDecision(DecisionRequestId),
    #[error("decision request {decision} belongs to organization {decision_recipient}, not report recipient {report_recipient}")]
    DecisionRecipientMismatch {
        decision: DecisionRequestId,
        decision_recipient: OrganizationId,
        report_recipient: OrganizationId,
    },
}

pub struct ValidatedReport {
    draft: ReportDraft,
}
impl ValidatedReport {
    pub fn commit(self, state: &mut AppState) -> ReportId {
        let id = state.ids.next_report();
        state.reports.insert(ReportRecord {
            id,
            recipient: self.draft.recipient,
            kind: self.draft.kind,
            title: self.draft.title,
            generated_at: state.now(),
            entries: self.draft.entries,
        });
        id
    }
}

pub fn validate_record_report(
    state: &AppState,
    draft: ReportDraft,
) -> Result<ValidatedReport, ReportError> {
    if draft.title.trim().is_empty() {
        return Err(ReportError::EmptyTitle);
    }
    if state.world.get_organization(draft.recipient).is_none() {
        return Err(ReportError::MissingOrganization(draft.recipient));
    }
    for (index, entry) in draft.entries.iter().enumerate() {
        if entry.summary.trim().is_empty() {
            return Err(ReportError::EmptyEntry(index));
        }
        for source in &entry.sources {
            if state.intelligence.get_information(*source).is_none() {
                return Err(ReportError::MissingInformation(*source));
            }
        }
        for entity in &entry.entities {
            if !is_entity_present(state, *entity) {
                return Err(ReportError::MissingEntity(*entity));
            }
        }
        if let Some(decision) = entry.decision {
            let record = state
                .decisions
                .get_decision(decision)
                .ok_or(ReportError::MissingDecision(decision))?;
            if record.recipient() != draft.recipient {
                return Err(ReportError::DecisionRecipientMismatch {
                    decision,
                    decision_recipient: record.recipient(),
                    report_recipient: draft.recipient,
                });
            }
        }
    }
    Ok(ValidatedReport { draft })
}
