//! Operation abort validation, causal artifacts, and custody/police/deadline preemption.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, DecisionRequestId, IdKind, OperationId, PoliceResponseId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::history::history_system::{ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{ValidatedInformation, validate_record_information};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_system::{
    OperationError, has_missed_operation_deadline, resolve_earliest_operation_deadline,
};
use crate::operations::{
    OperationAbortArtifacts, OperationAbortCause, OperationAbortPhase, OperationAbortRecord,
    OperationRecord, OperationStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::BTreeSet;

pub(crate) fn validate_deadline_missed_operation(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationAbort, OperationError> {
    if !has_missed_operation_deadline(registry, state, operation) {
        return Err(OperationError::DeadlineNotMissed { operation });
    }
    validate_operation_abort(state, operation, OperationAbortCause::DeadlineMissed)
}

pub struct ValidatedOperationAbort {
    operation: OperationId,
    expected_operation_version: u32,
    expected_status: OperationStatus,
    aborted_at: SimTime,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    information: Option<ValidatedInformation>,
    police_activity_information: Option<ValidatedInformation>,
    report: Option<ValidatedReport>,
    history: Option<ValidatedHistoryEvent>,
}

impl ValidatedOperationAbort {
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let mut budget = Vec::new();
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        if self.police_activity_information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        if self.history.is_some() {
            budget.push((IdKind::HistoryEvent, 1));
        }
        if self.report.is_some() {
            budget.push((IdKind::Report, 1));
        }
        budget
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), OperationError> {
        self.ensure_current(state)?;
        let budget = self.id_budget();
        state.ids.reserve_many(&budget)?;
        self.commit_preflighted(state);
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        let artifacts = match (self.information, self.report, self.history) {
            (None, None, None) => None,
            (Some(information), Some(report), Some(history)) => {
                let information = information
                    .commit(state)
                    .expect("operation-abort information ID was preflighted before mutation");
                let police_activity_information =
                    self.police_activity_information.map(|information| {
                        information.commit(state).expect(
                            "operation-abort police information ID was preflighted before mutation",
                        )
                    });
                let history_event = history
                    .commit(state)
                    .expect("operation-abort history ID was preflighted before mutation");
                let report = report
                    .commit(state)
                    .expect("operation-abort report ID was preflighted before mutation");
                Some(OperationAbortArtifacts {
                    information,
                    report,
                    history_event,
                    police_activity_information,
                })
            }
            (None, Some(_), _) | (None, _, Some(_)) | (Some(_), None, _) | (Some(_), _, None) => {
                unreachable!("validated operation abort artifacts are all present or all absent")
            }
        };
        state.operations.abort(
            self.operation,
            OperationAbortRecord {
                aborted_at: self.aborted_at,
                phase: self.phase,
                cause: self.cause,
                artifacts,
            },
        );
    }

    pub(crate) fn operation(&self) -> OperationId {
        self.operation
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), OperationError> {
        // Staleness re-checks precede ID reservation: a rejected commit must leave the
        // serialized high-water marks exactly as it found them.
        let record = state
            .operations
            .get_operation(self.operation)
            .ok_or(OperationError::MissingOperation(self.operation))?;
        if record.version() != self.expected_operation_version {
            return Err(OperationError::StaleAbortOperation {
                operation: self.operation,
                expected: self.expected_operation_version,
                found: record.version(),
            });
        }
        if record.status() != self.expected_status {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        if state.now() != self.aborted_at {
            return Err(OperationError::StaleAbortTime {
                operation: self.operation,
                expected: self.aborted_at,
                found: state.now(),
            });
        }
        if let OperationAbortCause::Decision(decision) = self.cause
            && state.decisions.pending_for_operation(self.operation) != Some(decision)
        {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        if let OperationAbortCause::PoliceArrival(response) = self.cause
            && !police_arrival_can_abort(state, record, response)
        {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        if let OperationAbortCause::ParticipantDetained(character) = self.cause
            && !record.participants().contains(&character)
        {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        Ok(())
    }
}

pub(crate) fn validate_authority_abort_operation(
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(state, operation, OperationAbortCause::AuthorityOrder)
}

pub(crate) fn validate_participant_detention_abort_operation(
    state: &AppState,
    operation: OperationId,
    character: CharacterId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(
        state,
        operation,
        OperationAbortCause::ParticipantDetained(character),
    )
}

pub(crate) fn validate_decision_abort_operation(
    state: &AppState,
    operation: OperationId,
    decision: DecisionRequestId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(state, operation, OperationAbortCause::Decision(decision))
}

pub(crate) fn validate_police_arrival_abort_operation(
    state: &AppState,
    operation: OperationId,
    response: PoliceResponseId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(
        state,
        operation,
        OperationAbortCause::PoliceArrival(response),
    )
}

pub(crate) fn validate_police_arrival_abort_if_applicable(
    state: &AppState,
    operation: OperationId,
) -> Result<Option<ValidatedOperationAbort>, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    let Some(response) = record.police_response() else {
        return Ok(None);
    };
    if police_arrival_can_abort(state, record, response) {
        validate_police_arrival_abort_operation(state, operation, response).map(Some)
    } else {
        Ok(None)
    }
}

fn validate_operation_abort(
    state: &AppState,
    operation: OperationId,
    cause: OperationAbortCause,
) -> Result<ValidatedOperationAbort, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    let phase = match (record.status(), cause) {
        (OperationStatus::Authorized, OperationAbortCause::AuthorityOrder) => {
            OperationAbortPhase::BeforeStart
        }
        (OperationStatus::Authorized, OperationAbortCause::DeadlineMissed) => {
            OperationAbortPhase::BeforeStart
        }
        (OperationStatus::InProgress, OperationAbortCause::DeadlineMissed) => {
            OperationAbortPhase::InProgress
        }
        (OperationStatus::AwaitingDecision, OperationAbortCause::DeadlineMissed) => {
            OperationAbortPhase::AwaitingDecision
        }
        (OperationStatus::InProgress, OperationAbortCause::AuthorityOrder) => {
            OperationAbortPhase::InProgress
        }
        (OperationStatus::Authorized, OperationAbortCause::ParticipantDetained(character))
            if record.participants().contains(&character) =>
        {
            OperationAbortPhase::BeforeStart
        }
        (OperationStatus::InProgress, OperationAbortCause::ParticipantDetained(character))
            if record.participants().contains(&character) =>
        {
            OperationAbortPhase::InProgress
        }
        (
            OperationStatus::AwaitingDecision,
            OperationAbortCause::ParticipantDetained(character),
        ) if record.participants().contains(&character) => OperationAbortPhase::AwaitingDecision,
        (OperationStatus::AwaitingDecision, OperationAbortCause::Decision(decision))
            if state.decisions.pending_for_operation(operation) == Some(decision) =>
        {
            OperationAbortPhase::AwaitingDecision
        }
        (OperationStatus::InProgress, OperationAbortCause::PoliceArrival(response))
            if police_arrival_can_abort(state, record, response) =>
        {
            OperationAbortPhase::InProgress
        }
        (status, cause) => {
            return Err(OperationError::InvalidAbortCause {
                operation,
                status,
                cause,
            });
        }
    };

    let (information, police_activity_information, report, history) = match (phase, cause) {
        (OperationAbortPhase::BeforeStart, OperationAbortCause::AuthorityOrder) => {
            (None, None, None, None)
        }
        (OperationAbortPhase::BeforeStart, OperationAbortCause::DeadlineMissed)
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::ParticipantDetained(_))
        | (OperationAbortPhase::InProgress, _)
        | (OperationAbortPhase::AwaitingDecision, _) => {
            let summary = build_abort_summary(state, record, cause)?;
            let entities = abort_entities(state, record, cause)?;
            let information = validate_record_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Organization(record.responsible_organization()),
                    source_kind: InformationSourceKind::AfterAction,
                    topic: InformationTopic::OperationalOutcome,
                    source_entity: Some(EntityRef::Character(record.leader())),
                    subject: EntityRef::Operation(operation),
                    observed_at: state.now(),
                    reliability: Reliability::DirectAccess,
                    specificity: Specificity::Precise,
                    summary: summary.clone(),
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            // A police-arrival abort leaves the organization district-scoped enforcement
            // knowledge: its own crew saw this authority respond here before entry. Holding
            // it as PoliceActivity information is what lets later planning in the same
            // neighborhood learn from the failed approach without fresh surveillance.
            let police_activity_information = match cause {
                OperationAbortCause::PoliceArrival(response_id) => {
                    let response = state
                        .legal
                        .get_police_response(response_id)
                        .ok_or(OperationError::InvalidAbortArtifacts { operation })?;
                    let authority = state
                        .world
                        .get_organization(response.authority())
                        .ok_or(OperationError::InvalidAbortArtifacts { operation })?;
                    let neighborhood = state
                        .world
                        .get_neighborhood(response.neighborhood())
                        .ok_or(OperationError::InvalidAbortArtifacts { operation })?;
                    Some(
                        validate_record_information(
                            state,
                            InformationDraft {
                                holder: KnowledgeHolder::Organization(
                                    record.responsible_organization(),
                                ),
                                source_kind: InformationSourceKind::AfterAction,
                                topic: InformationTopic::PoliceActivity,
                                source_entity: Some(EntityRef::Organization(response.authority())),
                                subject: EntityRef::Neighborhood(response.neighborhood()),
                                observed_at: state.now(),
                                reliability: Reliability::GenerallyReliable,
                                specificity: Specificity::Specific,
                                summary: format!(
                                    "The crew of {} was debriefed after a {} response reached the target before entry; the organization expects active enforcement around {} at that hour.",
                                    record.title(),
                                    authority.name(),
                                    neighborhood.name(),
                                ),
                            },
                        )
                        .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?,
                    )
                }
                OperationAbortCause::AuthorityOrder
                | OperationAbortCause::Decision(_)
                | OperationAbortCause::DeadlineMissed
                | OperationAbortCause::ParticipantDetained(_) => None,
            };
            let report = validate_record_report(
                state,
                ReportDraft {
                    recipient: record.responsible_organization(),
                    kind: ReportKind::AfterAction,
                    title: format!("{} after-action report", record.title()),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Notable,
                        summary: summary.clone(),
                        sources: Vec::new(),
                        entities: entities.clone(),
                        decision: None,
                    }],
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            let history = validate_record_event(
                state,
                HistoryEventDraft {
                    occurred_at: state.now(),
                    kind: HistoryEventKind::Operation,
                    summary,
                    entities,
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            (
                Some(information),
                police_activity_information,
                Some(report),
                Some(history),
            )
        }
        (OperationAbortPhase::BeforeStart, OperationAbortCause::Decision(_))
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::PoliceArrival(_)) => {
            unreachable!("pre-start operation aborts cannot use execution-only causes")
        }
    };

    Ok(ValidatedOperationAbort {
        operation,
        expected_operation_version: record.version(),
        expected_status: record.status(),
        aborted_at: state.now(),
        phase,
        cause,
        information,
        police_activity_information,
        report,
        history,
    })
}

fn build_abort_summary(
    state: &AppState,
    operation: &OperationRecord,
    cause: OperationAbortCause,
) -> Result<String, OperationError> {
    match cause {
        OperationAbortCause::AuthorityOrder => Ok(format!(
            "{} was aborted by leadership after execution began. Objective resolution was not completed.",
            operation.title()
        )),
        OperationAbortCause::Decision(decision) => {
            let decision = state.decisions.get_decision(decision).ok_or(
                OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                },
            )?;
            Ok(format!(
                "{} was aborted after leadership reviewed an execution exception: {}. Objective resolution was not completed.",
                operation.title(),
                decision.summary()
            ))
        }
        OperationAbortCause::ParticipantDetained(character) => {
            let name = state
                .world
                .get_character(character)
                .ok_or(OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                })?
                .name();
            let phase = match operation.status() {
                OperationStatus::Authorized => "was cancelled before execution",
                OperationStatus::InProgress | OperationStatus::AwaitingDecision => {
                    "was aborted during execution"
                }
                OperationStatus::Completed | OperationStatus::Aborted => {
                    unreachable!("terminal operations cannot receive detention aborts")
                }
            };
            Ok(format!(
                "{} {phase} because {name} was detained and could no longer participate. Objective resolution was not completed.",
                operation.title()
            ))
        }
        OperationAbortCause::PoliceArrival(response) => {
            let response = state.legal.get_police_response(response).ok_or(
                OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                },
            )?;
            let authority = state.world.get_organization(response.authority()).ok_or(
                OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                },
            )?;
            Ok(format!(
                "{} was aborted under its standing contingency when a {} response was due to reach the target before entry. Objective resolution was not completed.",
                operation.title(),
                authority.name()
            ))
        }
        OperationAbortCause::DeadlineMissed => {
            let deadline = resolve_earliest_operation_deadline(operation)
                .expect("validated deadline abort must retain a completion deadline");
            let phase = match operation.status() {
                OperationStatus::Authorized => "before execution could begin",
                OperationStatus::InProgress | OperationStatus::AwaitingDecision => {
                    "before execution could complete"
                }
                OperationStatus::Completed | OperationStatus::Aborted => {
                    unreachable!("terminal operations cannot miss an active deadline")
                }
            };
            if operation.status() == OperationStatus::Authorized && state.now() < deadline {
                Ok(format!(
                    "{} could no longer meet its completion deadline at minute {} {}.",
                    operation.title(),
                    deadline.as_minutes(),
                    phase,
                ))
            } else {
                Ok(format!(
                    "{} missed its completion deadline at minute {} {}.",
                    operation.title(),
                    deadline.as_minutes(),
                    phase,
                ))
            }
        }
    }
}

fn abort_entities(
    state: &AppState,
    record: &OperationRecord,
    cause: OperationAbortCause,
) -> Result<BTreeSet<EntityRef>, OperationError> {
    let mut entities = BTreeSet::from([
        EntityRef::Operation(record.id()),
        EntityRef::Organization(record.responsible_organization()),
        EntityRef::Character(record.leader()),
    ]);
    entities.extend(record.objective().referenced_entities());
    entities.extend(record.roles().values().copied().map(EntityRef::Character));
    match cause {
        OperationAbortCause::AuthorityOrder => {}
        OperationAbortCause::Decision(decision) => {
            entities.insert(EntityRef::DecisionRequest(decision));
        }
        OperationAbortCause::PoliceArrival(response) => {
            let response = state.legal.get_police_response(response).ok_or(
                OperationError::InvalidAbortArtifacts {
                    operation: record.id(),
                },
            )?;
            entities.insert(EntityRef::Organization(response.authority()));
            entities.insert(EntityRef::Neighborhood(response.neighborhood()));
        }
        OperationAbortCause::DeadlineMissed => {}
        OperationAbortCause::ParticipantDetained(character) => {
            entities.insert(EntityRef::Character(character));
        }
    }
    Ok(entities)
}

/// Pre-entry police-arrival abort gate. Each operation receives at most one police
/// response, dispatched at begin while the operation is `InProgress`; a decision pause can
/// only originate from that same response's post-arrival request, so an arrival can always
/// only abort an operation that is still `InProgress` and has not entered yet.
pub(crate) fn police_arrival_can_abort(
    state: &AppState,
    operation: &OperationRecord,
    response: PoliceResponseId,
) -> bool {
    if operation.police_response() != Some(response)
        || !operation
            .contingencies()
            .contains(&crate::operations::OperationContingency::AbortOnPoliceArrivalBeforeEntry)
    {
        return false;
    }
    let response = state
        .legal
        .get_police_response(response)
        .expect("operation police-response link must reference a persisted response");
    let effective_arrival = match response.status() {
        crate::legal::PoliceResponseStatus::Dispatched
            if operation.status() == OperationStatus::InProgress
                && response.arrival_due_at() <= state.now() =>
        {
            state.now()
        }
        crate::legal::PoliceResponseStatus::Arrived => response
            .arrived_at()
            .expect("arrived police response must persist its arrival time"),
        crate::legal::PoliceResponseStatus::Dispatched => return false,
    };
    let entry_at = match operation.status() {
        OperationStatus::InProgress => operation
            .entry_at()
            .expect("in-progress operation must persist its entry milestone"),
        OperationStatus::Authorized
        | OperationStatus::AwaitingDecision
        | OperationStatus::Completed
        | OperationStatus::Aborted => return false,
    };
    effective_arrival < entry_at
}
