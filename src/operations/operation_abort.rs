//! Operation abort validation, causal artifacts, and custody/police/deadline/opportunity/objective preemption.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, DecisionRequestId, IdKind, OperationId, OpportunityId, PoliceResponseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::ensure_version_can_advance;
use crate::history::history_system::{ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{ValidatedInformation, validate_record_information};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_objective::resolve_objective_blocker;
use crate::operations::operation_scheduling::has_missed_operation_deadline;
use crate::operations::operation_system::OperationError;
use crate::operations::{
    OperationAbortArtifacts, OperationAbortCause, OperationAbortPhase, OperationAbortRecord,
    OperationObjectiveBlocker, OperationRecord, OperationStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::BTreeSet;
use std::fmt::Write;

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

pub(crate) fn validate_expired_opportunity_operation(
    state: &AppState,
    operation: OperationId,
    opportunity: OpportunityId,
) -> Result<ValidatedOperationAbort, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    if !opportunity_expiry_can_abort(state, record, opportunity) {
        return Err(OperationError::InvalidAbortCause {
            operation,
            status: record.status(),
            cause: OperationAbortCause::OpportunityExpired(opportunity),
        });
    }
    validate_operation_abort(
        state,
        operation,
        OperationAbortCause::OpportunityExpired(opportunity),
    )
}

pub(crate) fn validate_objective_unavailable_operation(
    state: &AppState,
    operation: OperationId,
    blocker: OperationObjectiveBlocker,
) -> Result<ValidatedOperationAbort, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    if record.status() != OperationStatus::Authorized
        || resolve_objective_blocker(state, record) != Some(blocker)
    {
        return Err(OperationError::InvalidAbortCause {
            operation,
            status: record.status(),
            cause: OperationAbortCause::ObjectiveUnavailable(blocker),
        });
    }
    validate_operation_abort(
        state,
        operation,
        OperationAbortCause::ObjectiveUnavailable(blocker),
    )
}

pub struct ValidatedOperationAbort {
    operation: OperationId,
    expected_operation_version: u32,
    expected_status: OperationStatus,
    aborted_at: SimTime,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    information: ValidatedInformation,
    police_activity_information: Option<ValidatedInformation>,
    report: ValidatedReport,
    history: ValidatedHistoryEvent,
}

impl std::fmt::Debug for ValidatedOperationAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedOperationAbort")
            .field("operation", &self.operation)
            .field(
                "expected_operation_version",
                &self.expected_operation_version,
            )
            .field("expected_status", &self.expected_status)
            .field("aborted_at", &self.aborted_at)
            .field("phase", &self.phase)
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl ValidatedOperationAbort {
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let mut budget = vec![
            (IdKind::Information, 1),
            (IdKind::HistoryEvent, 1),
            (IdKind::Report, 1),
        ];
        if self.police_activity_information.is_some() {
            budget.push((IdKind::Information, 1));
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
        let information = self
            .information
            .commit(state)
            .expect("operation-abort information ID was preflighted before mutation");
        let police_activity_information = self.police_activity_information.map(|information| {
            information
                .commit(state)
                .expect("operation-abort police information ID was preflighted before mutation")
        });
        let history_event = self
            .history
            .commit(state)
            .expect("operation-abort history ID was preflighted before mutation");
        let report = self
            .report
            .commit(state)
            .expect("operation-abort report ID was preflighted before mutation");
        let artifacts = OperationAbortArtifacts {
            information,
            report,
            history_event,
            police_activity_information,
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
        self.ensure_current_without_detention_custody(state)?;
        if let OperationAbortCause::ParticipantDetained(character) = self.cause
            && state.legal.active_arrest_for_character(character).is_none()
        {
            let record = state
                .operations
                .get_operation(self.operation)
                .expect("preflighted abort operation must still exist");
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        Ok(())
    }

    /// Revalidates every dependency of a detention abort except the custody record whose
    /// insertion this abort is composed with. The arrest owner calls this before mutating
    /// custody so a stale operation/decision cannot make the larger arrest transaction fail
    /// after the arrest itself has already become authoritative.
    pub(crate) fn ensure_current_before_detention(
        &self,
        state: &AppState,
    ) -> Result<(), OperationError> {
        debug_assert!(matches!(
            self.cause,
            OperationAbortCause::ParticipantDetained(_)
        ));
        self.ensure_current_without_detention_custody(state)
    }

    fn ensure_current_without_detention_custody(
        &self,
        state: &AppState,
    ) -> Result<(), OperationError> {
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
        ensure_version_can_advance(record.version(), "operation")?;
        if record.status() != self.expected_status {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        crate::core::time::ensure_time_current(state.now(), self.aborted_at).map_err(
            |(expected, found)| OperationError::StaleAbortTime {
                operation: self.operation,
                expected,
                found,
            },
        )?;
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
        if let OperationAbortCause::OpportunityExpired(opportunity) = self.cause
            && !opportunity_expiry_can_abort(state, record, opportunity)
        {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        if let OperationAbortCause::ObjectiveUnavailable(blocker) = self.cause
            && resolve_objective_blocker(state, record) != Some(blocker)
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
    ensure_version_can_advance(record.version(), "operation")?;
    let phase = resolve_abort_phase(state, record, cause)?;
    let artifacts = validate_abort_artifacts(state, record, phase, cause)?;

    Ok(ValidatedOperationAbort {
        operation,
        expected_operation_version: record.version(),
        expected_status: record.status(),
        aborted_at: state.now(),
        phase,
        cause,
        information: artifacts.information,
        police_activity_information: artifacts.police_activity_information,
        report: artifacts.report,
        history: artifacts.history,
    })
}

fn resolve_abort_phase(
    state: &AppState,
    record: &OperationRecord,
    cause: OperationAbortCause,
) -> Result<OperationAbortPhase, OperationError> {
    let operation = record.id();
    match (record.status(), cause) {
        (OperationStatus::Authorized, OperationAbortCause::AuthorityOrder) => {
            Ok(OperationAbortPhase::BeforeStart)
        }
        (OperationStatus::Authorized, OperationAbortCause::DeadlineMissed)
        | (OperationStatus::Authorized, OperationAbortCause::OpportunityExpired(_))
        | (OperationStatus::Authorized, OperationAbortCause::ObjectiveUnavailable(_))
        | (OperationStatus::Authorized, OperationAbortCause::ParticipantDetained(_)) => {
            Ok(OperationAbortPhase::BeforeStart)
        }
        (OperationStatus::InProgress, OperationAbortCause::DeadlineMissed) => {
            Ok(OperationAbortPhase::InProgress)
        }
        (OperationStatus::AwaitingDecision, OperationAbortCause::DeadlineMissed) => {
            Ok(OperationAbortPhase::AwaitingDecision)
        }
        (OperationStatus::InProgress, OperationAbortCause::AuthorityOrder) => {
            Ok(OperationAbortPhase::InProgress)
        }
        (OperationStatus::InProgress, OperationAbortCause::ParticipantDetained(character))
            if record.participants().contains(&character) =>
        {
            Ok(OperationAbortPhase::InProgress)
        }
        (
            OperationStatus::AwaitingDecision,
            OperationAbortCause::ParticipantDetained(character),
        ) if record.participants().contains(&character) => {
            Ok(OperationAbortPhase::AwaitingDecision)
        }
        (OperationStatus::AwaitingDecision, OperationAbortCause::Decision(decision))
            if state.decisions.pending_for_operation(operation) == Some(decision) =>
        {
            Ok(OperationAbortPhase::AwaitingDecision)
        }
        (OperationStatus::InProgress, OperationAbortCause::PoliceArrival(response))
            if police_arrival_can_abort(state, record, response) =>
        {
            Ok(OperationAbortPhase::InProgress)
        }
        (status, cause) => Err(OperationError::InvalidAbortCause {
            operation,
            status,
            cause,
        }),
    }
}

struct ValidatedAbortArtifacts {
    information: ValidatedInformation,
    police_activity_information: Option<ValidatedInformation>,
    report: ValidatedReport,
    history: ValidatedHistoryEvent,
}

fn validate_abort_artifacts(
    state: &AppState,
    record: &OperationRecord,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
) -> Result<ValidatedAbortArtifacts, OperationError> {
    let operation = record.id();
    let summary = build_abort_summary(state, record, phase, cause, state.now())?;
    let entities = resolve_abort_entities(state, record, cause)?;
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
    let police_activity_information =
        validate_abort_police_activity_information(state, record, cause)?;
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
            kind: HistoryEventKind::Operation,
            summary,
            entities,
        },
    )
    .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;

    Ok(ValidatedAbortArtifacts {
        information,
        police_activity_information,
        report,
        history,
    })
}

/// A police-arrival abort leaves the organization district-scoped enforcement knowledge: its own
/// crew saw this authority respond here before entry. Other abort causes do not create this second
/// observation because they reveal no new authority movement.
fn validate_abort_police_activity_information(
    state: &AppState,
    record: &OperationRecord,
    cause: OperationAbortCause,
) -> Result<Option<ValidatedInformation>, OperationError> {
    let OperationAbortCause::PoliceArrival(response_id) = cause else {
        return Ok(None);
    };
    let operation = record.id();
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
    let mut summary = String::new();
    write_abort_police_activity_summary(
        &mut summary,
        record.title(),
        authority.name(),
        neighborhood.name(),
    )
    .expect("String buffer writes are infallible");
    validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(record.responsible_organization()),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::PoliceActivity,
            source_entity: Some(EntityRef::Organization(response.authority())),
            subject: EntityRef::Neighborhood(response.neighborhood()),
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary,
        },
    )
    .map(Some)
    .map_err(|_| OperationError::InvalidAbortArtifacts { operation })
}

/// Canonical player-facing debrief text for the district-scoped police observation created by a
/// police-arrival abort. Restore validation re-renders this exact content so the artifact cannot
/// be detached from the operation whose crew actually observed the response.
pub(crate) fn write_abort_police_activity_summary(
    output: &mut String,
    operation_title: &str,
    authority_name: &str,
    neighborhood_name: &str,
) -> std::fmt::Result {
    write!(
        output,
        "The crew of {operation_title} was debriefed after a {authority_name} response reached the target before entry; the organization expects active enforcement around {neighborhood_name} at that hour."
    )
}

pub(crate) fn build_abort_summary(
    state: &AppState,
    operation: &OperationRecord,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    aborted_at: SimTime,
) -> Result<String, OperationError> {
    match cause {
        OperationAbortCause::AuthorityOrder => {
            // A pre-start cancellation never reached the objective, so it reads as a
            // stand-down rather than an interrupted execution.
            if phase == OperationAbortPhase::BeforeStart {
                Ok(format!(
                    "{} was cancelled by leadership before execution began. The crew stood down without attempting the objective.",
                    operation.title()
                ))
            } else {
                Ok(format!(
                    "{} was aborted by leadership after execution began. Objective resolution was not completed.",
                    operation.title()
                ))
            }
        }
        OperationAbortCause::Decision(decision) => {
            let decision = state.decisions.get_decision(decision).ok_or(
                OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                },
            )?;
            Ok(format!(
                "{} was aborted after leadership reviewed an execution exception: {}. Objective resolution was not completed.",
                operation.title(),
                decision.summary().trim_end().trim_end_matches('.')
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
            let phase_text = match phase {
                OperationAbortPhase::BeforeStart => "was cancelled before execution",
                OperationAbortPhase::InProgress | OperationAbortPhase::AwaitingDecision => {
                    "was aborted during execution"
                }
            };
            Ok(format!(
                "{} {phase_text} because {name} was detained and could no longer participate. Objective resolution was not completed.",
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
        OperationAbortCause::OpportunityExpired(opportunity) => {
            let opportunity = state
                .opportunities()
                .opportunity_for_operation(operation.id())
                .filter(|record| record.id() == opportunity)
                .ok_or(OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                })?;
            let valid_until =
                opportunity
                    .valid_until()
                    .ok_or(OperationError::InvalidAbortArtifacts {
                        operation: operation.id(),
                    })?;
            Ok(format!(
                "{} was cancelled before execution because its linked opportunity window expired at minute {}. Objective resolution was not completed.",
                operation.title(),
                valid_until.as_minutes(),
            ))
        }
        OperationAbortCause::ObjectiveUnavailable(blocker) => {
            let reason = match blocker {
                OperationObjectiveBlocker::TargetBusinessOwnershipMismatch
                    if operation.kind() == crate::operations::OperationKind::GamblingEvent =>
                {
                    "the gambling venue was no longer under the sponsoring organization's control"
                }
                OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
                    "the target had come under the sponsoring organization's ownership"
                }
                OperationObjectiveBlocker::TargetEconomyInactive => {
                    "the target business was no longer operating"
                }
                OperationObjectiveBlocker::NoPressureableWitnessCase => {
                    "there was no remaining witness cooperation the crew could affect"
                }
                OperationObjectiveBlocker::ExtractionCustodyEnded => {
                    "the target was no longer detained"
                }
            };
            Ok(format!(
                "{} was cancelled before execution because {reason}. Objective resolution was not attempted.",
                operation.title()
            ))
        }
        OperationAbortCause::DeadlineMissed => {
            let deadline = operation
                .completion_deadline()
                .expect("validated deadline abort must retain a completion deadline");
            let phase_text = match phase {
                OperationAbortPhase::BeforeStart => "before execution could begin",
                OperationAbortPhase::InProgress | OperationAbortPhase::AwaitingDecision => {
                    "before execution could complete"
                }
            };
            if phase == OperationAbortPhase::BeforeStart && aborted_at < deadline {
                Ok(format!(
                    "{} could no longer meet its completion deadline at minute {} {}.",
                    operation.title(),
                    deadline.as_minutes(),
                    phase_text,
                ))
            } else {
                Ok(format!(
                    "{} missed its completion deadline at minute {} {}.",
                    operation.title(),
                    deadline.as_minutes(),
                    phase_text,
                ))
            }
        }
    }
}

pub(crate) fn resolve_abort_entities(
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
        OperationAbortCause::DeadlineMissed
        | OperationAbortCause::OpportunityExpired(_)
        | OperationAbortCause::ObjectiveUnavailable(_) => {}
        OperationAbortCause::ParticipantDetained(character) => {
            entities.insert(EntityRef::Character(character));
        }
    }
    Ok(entities)
}

fn opportunity_expiry_can_abort(
    state: &AppState,
    operation: &OperationRecord,
    opportunity: OpportunityId,
) -> bool {
    operation.status() == OperationStatus::Authorized
        && state
            .opportunities()
            .expired_window_for_operation(operation.id(), state.now())
            .is_some_and(|(record, _)| record.id() == opportunity)
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
