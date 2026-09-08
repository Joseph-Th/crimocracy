//! Player-facing record integrity: report holders and history-event references.

//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::reports::{ReportEntry, ReportRecord};

pub(super) fn validate_report_holders(state: &AppState) -> Result<(), StateValidationError> {
    for report in state.reports.reports() {
        validate_report(state, report)?;
    }

    Ok(())
}

fn validate_report(state: &AppState, report: &ReportRecord) -> Result<(), StateValidationError> {
    if report.title().trim().is_empty() {
        return Err(StateValidationError::EmptyReportTitle {
            report: report.id(),
        });
    }
    if state.world.get_organization(report.recipient()).is_none() {
        return Err(StateValidationError::MissingEntity {
            context: "report recipient",
            entity: EntityRef::Organization(report.recipient()),
        });
    }
    if report.generated_at() > state.now() {
        return Err(StateValidationError::FutureTimestamp { context: "report" });
    }
    for (entry_index, entry) in report.entries().iter().enumerate() {
        validate_report_entry(state, report, entry_index, entry)?;
    }
    Ok(())
}

fn validate_report_entry(
    state: &AppState,
    report: &ReportRecord,
    entry_index: usize,
    entry: &ReportEntry,
) -> Result<(), StateValidationError> {
    if entry.summary.trim().is_empty() {
        return Err(StateValidationError::EmptyReportEntrySummary {
            report: report.id(),
            entry: entry_index,
        });
    }
    validate_report_sources(state, report, entry)?;
    for entity in &entry.entities {
        if !is_entity_present(state, *entity) {
            return Err(StateValidationError::MissingEntity {
                context: "report entry",
                entity: *entity,
            });
        }
    }
    validate_report_decision(state, report, entry)
}

fn validate_report_sources(
    state: &AppState,
    report: &ReportRecord,
    entry: &ReportEntry,
) -> Result<(), StateValidationError> {
    for information in &entry.sources {
        let information_record = state.intelligence.get_information(*information).ok_or(
            StateValidationError::MissingReportInformation {
                report: report.id(),
                information: *information,
            },
        )?;
        let is_available = match information_record.holder() {
            KnowledgeHolder::Organization(organization) => organization == report.recipient(),
            KnowledgeHolder::Character(_) => false,
        };
        if !is_available {
            return Err(StateValidationError::ReportInformationUnavailable {
                report: report.id(),
                information: *information,
            });
        }
    }
    Ok(())
}

fn validate_report_decision(
    state: &AppState,
    report: &ReportRecord,
    entry: &ReportEntry,
) -> Result<(), StateValidationError> {
    let Some(decision) = entry.decision else {
        return Ok(());
    };
    let decision_record = state.decisions.get_decision(decision).ok_or(
        StateValidationError::MissingReportDecision {
            report: report.id(),
            decision,
        },
    )?;
    if decision_record.recipient() != report.recipient() {
        return Err(StateValidationError::ReportDecisionRecipientMismatch {
            report: report.id(),
            decision,
        });
    }
    Ok(())
}

pub(super) fn validate_history_event_references(
    state: &AppState,
) -> Result<(), StateValidationError> {
    for event in state.history.events() {
        if event.summary().trim().is_empty() {
            return Err(StateValidationError::EmptyHistorySummary { event: event.id() });
        }
        if event.entities().is_empty() {
            return Err(StateValidationError::HistoryEventHasNoEntities { event: event.id() });
        }
        if event.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "history event",
            });
        }
        for entity in event.entities() {
            if !is_entity_present(state, *entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "history event",
                    entity: *entity,
                });
            }
        }
    }
    Ok(())
}
