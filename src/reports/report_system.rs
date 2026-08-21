//! Report validation and insertion; reports expose known information rather than world truth.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    DecisionRequestId, IdExhaustionError, InformationId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::reports::{ReportDraft, ReportKind, ReportRecord};
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
    #[error("information record {information} is not available to report recipient {recipient}")]
    InformationUnavailable {
        information: InformationId,
        recipient: OrganizationId,
    },
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("report kind {0:?} is reserved for its owning synthesis path")]
    ReservedKind(ReportKind),
    #[error("decision request {0} does not exist")]
    MissingDecision(DecisionRequestId),
    #[error("decision request {decision} belongs to organization {decision_recipient}, not report recipient {report_recipient}")]
    DecisionRecipientMismatch {
        decision: DecisionRequestId,
        decision_recipient: OrganizationId,
        report_recipient: OrganizationId,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedReport {
    draft: ReportDraft,
}
impl ValidatedReport {
    pub fn commit(self, state: &mut AppState) -> Result<ReportId, ReportError> {
        let id = state.ids.next_report()?;
        state.reports.insert(ReportRecord {
            id,
            recipient: self.draft.recipient,
            kind: self.draft.kind,
            title: self.draft.title,
            generated_at: state.now(),
            entries: self.draft.entries,
        });
        Ok(id)
    }
}

pub fn validate_record_report(
    state: &AppState,
    draft: ReportDraft,
) -> Result<ValidatedReport, ReportError> {
    // Executive briefs are produced only by their owning synthesis path, which enforces the
    // one-brief-per-cadence-boundary invariant; a forged brief could panic or desync it.
    if draft.kind == ReportKind::ExecutiveBrief {
        return Err(ReportError::ReservedKind(draft.kind));
    }
    validate_report_draft(state, draft)
}

pub(crate) fn validate_report_draft(
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
            let information = state
                .intelligence
                .get_information(*source)
                .ok_or(ReportError::MissingInformation(*source))?;
            let is_available = match information.holder() {
                KnowledgeHolder::Organization(organization) => organization == draft.recipient,
                KnowledgeHolder::Character(_) => false,
            };
            if !is_available {
                return Err(ReportError::InformationUnavailable {
                    information: *source,
                    recipient: draft.recipient,
                });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::AttentionClass;
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
    };
    use crate::reports::{ReportEntry, ReportKind};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};
    use std::collections::BTreeSet;

    #[test]
    fn generic_report_path_cannot_forge_an_executive_brief() {
        let registry = build_registry();
        let mut state = AppState::new(0xB12E_F194);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Brief Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient fixture should validate");

        let error = match validate_record_report(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::ExecutiveBrief,
                title: "Forged brief".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "Only the synthesis path may produce briefs.".to_owned(),
                    sources: Vec::new(),
                    entities: BTreeSet::new(),
                    decision: None,
                }],
            },
        ) {
            Ok(_) => panic!("generic report path must reject executive briefs"),
            Err(error) => error,
        };
        assert_eq!(error, ReportError::ReservedKind(ReportKind::ExecutiveBrief));
        assert!(
            state
                .reports()
                .latest_for_kind(recipient, ReportKind::ExecutiveBrief)
                .is_none(),
            "forged brief must not be recorded"
        );
    }

    #[test]
    fn report_cannot_cite_information_held_by_another_organization() {
        let registry = build_registry();
        let mut state = AppState::new(0xB12E_F193);
        let holder = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Information Holder".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("information holder fixture should validate");
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Uninformed Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient fixture should validate");
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(holder),
                source_kind: InformationSourceKind::DirectObservation,
                topic: crate::intelligence::InformationTopic::General,
                source_entity: None,
                subject: EntityRef::Organization(holder),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Only the holder organization knows this fact.".to_owned(),
            },
        )
        .expect("information fixture should validate")
        .commit(&mut state)
        .expect("information fixture should commit");

        let error = match validate_record_report(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Leaked intelligence".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "This report must not cross the knowledge boundary.".to_owned(),
                    sources: vec![information],
                    entities: BTreeSet::from([EntityRef::Organization(holder)]),
                    decision: None,
                }],
            },
        ) {
            Ok(_) => panic!("report must reject information held by another organization"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ReportError::InformationUnavailable {
                information,
                recipient,
            }
        );
    }
}
