//! Report validation and insertion; reports expose known information rather than world truth.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    DecisionRequestId, IdExhaustionError, InformationId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::intelligence::intelligence_system::PlannedInformationSource;
use crate::reports::{ReportDraft, ReportEntry, ReportKind, ReportRecord};
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
    #[error(
        "decision request {decision} belongs to organization {decision_recipient}, not report recipient {report_recipient}"
    )]
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
    planned_information: Option<PlannedInformationSource>,
}
impl ValidatedReport {
    pub fn commit(self, state: &mut AppState) -> Result<ReportId, ReportError> {
        if let Some(planned) = self.planned_information {
            let information = state
                .intelligence
                .get_information(planned.id())
                .ok_or(ReportError::MissingInformation(planned.id()))?;
            if information.holder() != planned.holder() {
                return Err(ReportError::InformationUnavailable {
                    information: planned.id(),
                    recipient: self.draft.recipient,
                });
            }
        }
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
    validate_report_draft_with_planned_information(state, draft, None)
}

/// Validates a report whose source information is part of the same larger atomic operation and
/// therefore has a predicted ID but is not persisted yet. The caller owns allocator freshness;
/// this report owner only treats the exact planned ID/holder pair as available while validating
/// the otherwise-normal report contract.
pub(crate) fn validate_record_report_with_planned_information(
    state: &AppState,
    draft: ReportDraft,
    planned: PlannedInformationSource,
) -> Result<ValidatedReport, ReportError> {
    if draft.kind == ReportKind::ExecutiveBrief {
        return Err(ReportError::ReservedKind(draft.kind));
    }
    validate_report_draft_with_planned_information(state, draft, Some(planned))
}

pub(crate) fn validate_report_draft(
    state: &AppState,
    draft: ReportDraft,
) -> Result<ValidatedReport, ReportError> {
    validate_report_draft_with_planned_information(state, draft, None)
}

fn validate_report_draft_with_planned_information(
    state: &AppState,
    draft: ReportDraft,
    planned: Option<PlannedInformationSource>,
) -> Result<ValidatedReport, ReportError> {
    if draft.title.trim().is_empty() {
        return Err(ReportError::EmptyTitle);
    }
    if state.world.get_organization(draft.recipient).is_none() {
        return Err(ReportError::MissingOrganization(draft.recipient));
    }
    for (index, entry) in draft.entries.iter().enumerate() {
        validate_report_entry(state, draft.recipient, index, entry, planned)?;
    }
    Ok(ValidatedReport {
        draft,
        planned_information: planned,
    })
}

fn validate_report_entry(
    state: &AppState,
    recipient: OrganizationId,
    index: usize,
    entry: &ReportEntry,
    planned: Option<PlannedInformationSource>,
) -> Result<(), ReportError> {
    if entry.summary.trim().is_empty() {
        return Err(ReportError::EmptyEntry(index));
    }
    for source in &entry.sources {
        validate_report_source(state, recipient, *source, planned)?;
    }
    for entity in &entry.entities {
        if !is_entity_present(state, *entity) {
            return Err(ReportError::MissingEntity(*entity));
        }
    }
    if let Some(decision) = entry.decision {
        validate_report_decision(state, recipient, decision)?;
    }
    Ok(())
}

fn validate_report_source(
    state: &AppState,
    recipient: OrganizationId,
    source: InformationId,
    planned: Option<PlannedInformationSource>,
) -> Result<(), ReportError> {
    let holder = state
        .intelligence
        .get_information(source)
        .map(|information| information.holder())
        .or_else(|| {
            planned
                .filter(|planned| planned.id() == source)
                .map(PlannedInformationSource::holder)
        })
        .ok_or(ReportError::MissingInformation(source))?;
    if holder != KnowledgeHolder::Organization(recipient) {
        return Err(ReportError::InformationUnavailable {
            information: source,
            recipient,
        });
    }
    Ok(())
}

fn validate_report_decision(
    state: &AppState,
    recipient: OrganizationId,
    decision: DecisionRequestId,
) -> Result<(), ReportError> {
    let record = state
        .decisions
        .get_decision(decision)
        .ok_or(ReportError::MissingDecision(decision))?;
    if record.recipient() != recipient {
        return Err(ReportError::DecisionRecipientMismatch {
            decision,
            decision_recipient: record.recipient(),
            report_recipient: recipient,
        });
    }
    Ok(())
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

    #[test]
    fn report_can_validate_against_one_exact_planned_organization_information_source() {
        let registry = build_registry();
        let mut state = AppState::new(0xB12E_F195);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Planned Source Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient fixture should validate");
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(recipient),
                source_kind: InformationSourceKind::DirectObservation,
                topic: crate::intelligence::InformationTopic::General,
                source_entity: None,
                subject: EntityRef::Organization(recipient),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Future information source for the same atomic report.".to_owned(),
            },
        )
        .expect("planned information should validate");
        let planned = information.planned_source(&state);

        validate_record_report_with_planned_information(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Planned-source report".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "The source will be committed by the same atomic operation."
                        .to_owned(),
                    sources: vec![planned.id()],
                    entities: BTreeSet::from([EntityRef::Organization(recipient)]),
                    decision: None,
                }],
            },
            planned,
        )
        .expect("the exact planned organization-held source should validate");
        assert!(state.intelligence().get_information(planned.id()).is_none());
    }

    #[test]
    fn planned_information_source_does_not_authorize_a_different_missing_source() {
        let registry = build_registry();
        let mut state = AppState::new(0xB12E_F196);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Exact Planned Source Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient fixture should validate");
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(recipient),
                source_kind: InformationSourceKind::DirectObservation,
                topic: crate::intelligence::InformationTopic::General,
                source_entity: None,
                subject: EntityRef::Organization(recipient),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Future exact information source.".to_owned(),
            },
        )
        .expect("planned information should validate");
        let planned = information.planned_source(&state);
        let missing = InformationId::from_raw(
            planned
                .id()
                .raw()
                .checked_add(1)
                .expect("fixture planned information id must leave one successor"),
        );

        let error = match validate_record_report_with_planned_information(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Wrong planned source".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "A different absent source must still be rejected.".to_owned(),
                    sources: vec![missing],
                    entities: BTreeSet::from([EntityRef::Organization(recipient)]),
                    decision: None,
                }],
            },
            planned,
        ) {
            Ok(_) => panic!("planned source authorization must be exact"),
            Err(error) => error,
        };
        assert_eq!(error, ReportError::MissingInformation(missing));
    }

    #[test]
    fn planned_source_report_cannot_commit_before_its_information() {
        let registry = build_registry();
        let mut state = AppState::new(0xB12E_F197);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Planned Commit Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient fixture should validate");
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(recipient),
                source_kind: InformationSourceKind::DirectObservation,
                topic: crate::intelligence::InformationTopic::General,
                source_entity: None,
                subject: EntityRef::Organization(recipient),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Future source must exist before its report commits.".to_owned(),
            },
        )
        .expect("planned information should validate");
        let planned = information.planned_source(&state);
        let report = validate_record_report_with_planned_information(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Premature planned-source report".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "This report must wait for its source.".to_owned(),
                    sources: vec![planned.id()],
                    entities: BTreeSet::from([EntityRef::Organization(recipient)]),
                    decision: None,
                }],
            },
            planned,
        )
        .expect("planned-source report should validate before the composite commit");
        let before_report_id = state.ids.next_raw(crate::core::id::IdKind::Report);

        let error = report
            .commit(&mut state)
            .expect_err("planned-source report must not commit before its source exists");
        assert_eq!(error, ReportError::MissingInformation(planned.id()));
        assert_eq!(
            state.ids.next_raw(crate::core::id::IdKind::Report),
            before_report_id
        );
        assert!(
            state
                .reports()
                .latest_for_kind(recipient, ReportKind::Financial)
                .is_none()
        );
    }
}
