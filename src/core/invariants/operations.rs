//! Release-safe structural validation for the operations subsystem.

use super::opportunities::validate_operation_exposure_links;
use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{InformationId, LedgerTransactionId, ReportId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::{DecisionContext, DecisionResponse, DecisionStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::operations::operation_execution::build_legal_activity_summary;
use crate::operations::operation_system::{
    is_information_subject_relevant, is_valid_operation_objective,
};
use crate::operations::property_disposition::build_disposition_summary;
use crate::operations::surveillance_integration::{
    expected_persisted_surveillance_signatures, is_supported_surveillance_target,
    is_valid_persisted_surveillance_information,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationConstraint, OperationContingency,
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationRecord,
    OperationResolutionRecord, OperationStatus,
};
use crate::reports::ReportKind;
use crate::world::{BusinessFunction, BusinessOwner};
use std::collections::BTreeSet;

pub(super) fn validate_operations(state: &AppState) -> Result<(), StateValidationError> {
    let mut operation_after_action_information = BTreeSet::new();
    let mut operation_legal_activity_information = BTreeSet::new();
    let mut operation_discovered_information = BTreeSet::new();
    let mut operation_after_action_reports = BTreeSet::new();
    let mut operation_history_events = BTreeSet::new();
    let mut property_disposition_transactions = BTreeSet::new();
    let mut property_disposition_information = BTreeSet::new();
    let mut property_disposition_reports = BTreeSet::new();
    for operation in state.operations.operations() {
        let leader = state.world.get_character(operation.leader()).ok_or(
            StateValidationError::MissingEntity {
                context: "operation leader",
                entity: EntityRef::Character(operation.leader()),
            },
        )?;
        let requires_active_participants = match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => true,
            OperationStatus::Completed | OperationStatus::Aborted => false,
        };
        for participant in operation.roles().values() {
            let participant_record = state.world.get_character(*participant).ok_or(
                StateValidationError::MissingEntity {
                    context: "operation participant",
                    entity: EntityRef::Character(*participant),
                },
            )?;
            if requires_active_participants
                && participant_record.organization() != Some(operation.responsible_organization())
            {
                return Err(StateValidationError::ActiveOperationForeignParticipant {
                    operation: operation.id(),
                    participant: *participant,
                });
            }
        }
        for information in operation.intelligence() {
            let record = state.intelligence.get_information(*information).ok_or(
                StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                },
            )?;
            if record.holder()
                != KnowledgeHolder::Organization(operation.responsible_organization())
                || !is_information_subject_relevant(state, operation.objective(), record.subject())
            {
                return Err(StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                });
            }
        }
        for entity in operation.objective().referenced_entities() {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "operation objective",
                    entity,
                });
            }
        }
        if !is_valid_operation_objective(operation.kind(), operation.objective()) {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
        for constraint in operation.constraints() {
            match constraint {
                OperationConstraint::CompleteBefore(deadline) => {
                    if operation.scheduled_for() >= *deadline {
                        return Err(StateValidationError::InvalidOperationRuntime {
                            operation: operation.id(),
                        });
                    }
                }
                OperationConstraint::RequireIntelligenceTopic(topic) => {
                    // The authorization gate must remain satisfiable by the persisted plan:
                    // organization-held intelligence of the required topic must back it.
                    let covered = operation.intelligence().iter().any(|information| {
                        state
                            .intelligence
                            .get_information(*information)
                            .is_some_and(|record| record.topic() == *topic)
                    });
                    if !covered {
                        return Err(StateValidationError::InvalidOperationRuntime {
                            operation: operation.id(),
                        });
                    }
                }
            }
        }
        // Exhaustiveness canary: a new OperationContingency must be classified in the
        // resolution and abort paths before persisted operations remain validatable.
        for contingency in operation.contingencies() {
            match contingency {
                OperationContingency::AbortOnPoliceArrivalBeforeEntry
                | OperationContingency::RequestDecisionOnUnexpectedCondition => {}
            }
        }
        if operation.entry_at().is_some_and(|entry_at| {
            operation
                .started_at()
                .is_none_or(|started_at| entry_at <= started_at)
        }) {
            return Err(StateValidationError::InvalidOperationRuntime {
                operation: operation.id(),
            });
        }
        if let Some(response_id) = operation.police_response() {
            if state
                .legal
                .get_police_response(response_id)
                .is_none_or(|response| response.source_operation() != operation.id())
            {
                return Err(StateValidationError::InvalidOperationRuntime {
                    operation: operation.id(),
                });
            }
        }
        match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => {
                if leader.organization() != Some(operation.responsible_organization()) {
                    return Err(StateValidationError::ActiveOperationInvalidLeader {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::Completed | OperationStatus::Aborted => {}
        }
        if operation.status() != OperationStatus::Completed
            && (operation.property_disposition().is_some()
                || operation.cash_disposition().is_some())
        {
            return Err(StateValidationError::InvalidOperationPropertyDisposition {
                operation: operation.id(),
            });
        }
        match operation.status() {
            OperationStatus::Authorized => {
                if operation.started_at().is_some()
                    || operation.resolution_due_at().is_some()
                    || operation.entry_at().is_some()
                    || operation.police_response().is_some()
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::InProgress => {
                let (Some(started_at), Some(due_at)) =
                    (operation.started_at(), operation.resolution_due_at())
                else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || started_at > state.now()
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::AwaitingDecision => {
                let (Some(started_at), Some(due_at), Some(paused_at)) = (
                    operation.started_at(),
                    operation.resolution_due_at(),
                    operation.awaiting_decision_since(),
                ) else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || started_at > paused_at
                    || paused_at > state.now()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::Completed => {
                let (Some(started_at), Some(due_at), Some(resolution)) = (
                    operation.started_at(),
                    operation.resolution_due_at(),
                    operation.resolution(),
                ) else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || resolution.resolved_at() < due_at
                    || resolution.resolved_at() > state.now()
                    || operation.awaiting_decision_since().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
                if let Some(proceeds) = resolution.property_proceeds() {
                    let valid_target = matches!(
                      operation.objective(),
                      OperationObjective::AcquireProperty { target }
                        if *target == proceeds.target()
                    );
                    if !valid_target
                        || resolution.objective_outcome() == OperationObjectiveOutcome::Failed
                        || proceeds.estimated_value().cents() <= 0
                    {
                        return Err(StateValidationError::InvalidOperationDefinition {
                            operation: operation.id(),
                        });
                    }
                }
                if let Some(proceeds) = resolution.cash_proceeds() {
                    let valid_target = matches!(
                      operation.objective(),
                      OperationObjective::ObtainCash { target }
                        if *target == proceeds.target()
                    );
                    if !valid_target
                        || resolution.objective_outcome() == OperationObjectiveOutcome::Failed
                        || proceeds.amount().cents() <= 0
                    {
                        return Err(StateValidationError::InvalidOperationCashProceeds {
                            operation: operation.id(),
                        });
                    }
                }
                validate_operation_cash_disposition(
                    state,
                    operation,
                    resolution,
                    &mut property_disposition_transactions,
                    &mut property_disposition_information,
                    &mut property_disposition_reports,
                )?;
                validate_operation_property_disposition(
                    state,
                    operation,
                    resolution,
                    &mut property_disposition_transactions,
                    &mut property_disposition_information,
                    &mut property_disposition_reports,
                )?;
                let information = state
                    .intelligence
                    .get_information(resolution.after_action_information())
                    .ok_or(StateValidationError::InvalidOperationAfterAction {
                        operation: operation.id(),
                    })?;
                if !operation_after_action_information.insert(resolution.after_action_information())
                    || information.holder()
                        != KnowledgeHolder::Organization(operation.responsible_organization())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.topic() != InformationTopic::OperationalOutcome
                    || information.source_entity() != Some(EntityRef::Character(operation.leader()))
                    || information.subject() != EntityRef::Operation(operation.id())
                    || information.observed_at() != resolution.resolved_at()
                {
                    return Err(StateValidationError::InvalidOperationAfterAction {
                        operation: operation.id(),
                    });
                }
                let report = state
                    .reports
                    .get_report(resolution.after_action_report())
                    .ok_or(StateValidationError::InvalidOperationAfterActionReport {
                        operation: operation.id(),
                    })?;
                let report_entry = report.entries().first();
                if !operation_after_action_reports.insert(report.id())
                    || report.recipient() != operation.responsible_organization()
                    || report.kind() != ReportKind::AfterAction
                    || report.title() != format!("{} after-action report", operation.title())
                    || report.generated_at() != resolution.resolved_at()
                    || report.entries().len() != 1
                    || !report_entry.is_some_and(|entry| {
                        entry.attention == AttentionClass::Notable
                            && entry.summary == information.summary()
                            && entry.sources.is_empty()
                            && entry.decision.is_none()
                            && entry
                                .entities
                                .contains(&EntityRef::Operation(operation.id()))
                            && entry.entities.contains(&EntityRef::Organization(
                                operation.responsible_organization(),
                            ))
                            && entry
                                .entities
                                .contains(&EntityRef::Character(operation.leader()))
                    })
                {
                    return Err(StateValidationError::InvalidOperationAfterActionReport {
                        operation: operation.id(),
                    });
                }
                match resolution.legal_activity_information() {
                    Some(information_id) => {
                        let investigation_id = resolution.exposure().investigation().ok_or(
                            StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            },
                        )?;
                        let investigation = state.legal.get_investigation(investigation_id).ok_or(
                            StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            },
                        )?;
                        let legal_information = state
                            .intelligence
                            .get_information(information_id)
                            .ok_or(StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            })?;
                        if !operation_legal_activity_information.insert(information_id)
                            || legal_information.holder()
                                != KnowledgeHolder::Organization(
                                    operation.responsible_organization(),
                                )
                            || legal_information.source_kind() != InformationSourceKind::AfterAction
                            || legal_information.topic() != InformationTopic::LegalActivity
                            || legal_information.source_entity()
                                != Some(EntityRef::Character(operation.leader()))
                            || legal_information.subject() != EntityRef::Operation(operation.id())
                            || legal_information.observed_at() != resolution.resolved_at()
                            || legal_information.recorded_at() != resolution.resolved_at()
                            || legal_information.reliability() != Reliability::GenerallyReliable
                            || legal_information.specificity() != Specificity::Specific
                            || legal_information.summary()
                                != build_legal_activity_summary(
                                    state,
                                    operation,
                                    investigation.owner(),
                                )
                        {
                            return Err(StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            });
                        }
                    }
                    None if resolution.exposure().investigation().is_some() => {
                        return Err(StateValidationError::InvalidOperationLegalActivity {
                            operation: operation.id(),
                        });
                    }
                    None => {}
                }
                let valid_history = state
                    .history
                    .get_event(resolution.history_event())
                    .is_some_and(|event| {
                        operation_history_events.insert(event.id())
                            && event.kind() == HistoryEventKind::Operation
                            && event.occurred_at() == resolution.resolved_at()
                            && event
                                .entities()
                                .contains(&EntityRef::Operation(operation.id()))
                            && event.entities().contains(&EntityRef::Organization(
                                operation.responsible_organization(),
                            ))
                            && event
                                .entities()
                                .contains(&EntityRef::Character(operation.leader()))
                    });
                if !valid_history {
                    return Err(StateValidationError::InvalidOperationHistory {
                        operation: operation.id(),
                    });
                }
                validate_operation_discoveries(
                    state,
                    operation,
                    resolution,
                    &mut operation_discovered_information,
                )?;
                validate_operation_exposure_links(state, operation, resolution)?;
            }
            OperationStatus::Aborted => {
                let abort = operation.abort_record().ok_or(
                    StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    },
                )?;
                let pause_shape_valid = match abort.phase() {
                    OperationAbortPhase::AwaitingDecision => operation
                        .awaiting_decision_since()
                        .is_some_and(|paused_at| paused_at <= abort.aborted_at()),
                    OperationAbortPhase::BeforeStart | OperationAbortPhase::InProgress => {
                        operation.awaiting_decision_since().is_none()
                    }
                };
                if !pause_shape_valid || operation.resolution().is_some() {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
                validate_operation_abort_links(
                    state,
                    operation,
                    abort,
                    &mut operation_after_action_information,
                    &mut operation_after_action_reports,
                    &mut operation_history_events,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_operation_property_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    transactions: &mut BTreeSet<LedgerTransactionId>,
    information_ids: &mut BTreeSet<InformationId>,
    reports: &mut BTreeSet<ReportId>,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.property_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationPropertyDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.property_proceeds().ok_or_else(invalid)?;
    if disposition.disposed_at() < resolution.resolved_at()
        || disposition.disposed_at() > state.now()
        || disposition.realized_value().cents() <= 0
        || disposition.realized_value().cents() > proceeds.estimated_value().cents()
        || !transactions.insert(disposition.transaction())
        || !information_ids.insert(disposition.information())
        || !reports.insert(disposition.report())
    {
        return Err(invalid());
    }

    let venue = state
        .world
        .get_business(disposition.venue())
        .ok_or_else(invalid)?;
    let ownership = state
        .world
        .get_business_ownership_change_for_version(disposition.venue(), disposition.venue_version())
        .ok_or_else(invalid)?;
    let next_ownership_at_disposition = disposition
        .venue_version()
        .checked_add(1)
        .and_then(|version| {
            state
                .world
                .get_business_ownership_change_for_version(disposition.venue(), version)
        })
        .is_some_and(|next| next.changed_at() <= disposition.disposed_at());
    if disposition.venue_version() > venue.version()
        || ownership.new_owner()
            != BusinessOwner::Organization(operation.responsible_organization())
        || ownership.changed_at() > disposition.disposed_at()
        || next_ownership_at_disposition
        || state
            .world
            .business_owner_at(disposition.venue(), disposition.disposed_at())
            != Some(BusinessOwner::Organization(
                operation.responsible_organization(),
            ))
        || !venue.has_function(BusinessFunction::ResaleMarket)
    {
        return Err(invalid());
    }

    let cash = state
        .finance
        .get_account(disposition.cash_account())
        .ok_or_else(invalid)?;
    let settlement = state
        .finance
        .get_account(disposition.settlement_account())
        .ok_or_else(invalid)?;
    let expected_owner = FinancialOwner::Organization(operation.responsible_organization());
    if disposition.cash_account() == disposition.settlement_account()
        || cash.owner() != expected_owner
        || settlement.owner() != expected_owner
        || !matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        )
        || settlement.kind() != AccountKind::Settlement
    {
        return Err(invalid());
    }

    let transaction = state
        .finance
        .get_transaction(disposition.transaction())
        .ok_or_else(invalid)?;
    let negative_value = disposition
        .realized_value()
        .cents()
        .checked_neg()
        .map(Money::from_cents)
        .ok_or_else(invalid)?;
    let has_cash_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.cash_account()
            && posting.amount == disposition.realized_value()
    });
    let has_settlement_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.settlement_account() && posting.amount == negative_value
    });
    if transaction.occurred_at() != disposition.disposed_at()
        || transaction.memo()
            != format!(
                "Property liquidation for {} through {}",
                operation.id(),
                disposition.venue()
            )
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid());
    }

    let information = state
        .intelligence
        .get_information(disposition.information())
        .ok_or_else(invalid)?;
    if information.holder() != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::Accountant
        || information.topic() != InformationTopic::FinancialPerformance
        || information.source_entity() != Some(EntityRef::Business(disposition.venue()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != disposition.disposed_at()
        || information.recorded_at() != disposition.disposed_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || information.summary()
            != build_disposition_summary(
                operation.title(),
                venue.name(),
                proceeds.estimated_value(),
                disposition.realized_value(),
            )
    {
        return Err(invalid());
    }
    let report = state
        .reports
        .get_report(disposition.report())
        .ok_or_else(invalid)?;
    let expected_summary = build_disposition_summary(
        operation.title(),
        venue.name(),
        proceeds.estimated_value(),
        disposition.realized_value(),
    );
    if report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::Financial
        || report.title() != "Property disposition"
        || report.generated_at() != disposition.disposed_at()
        || report.entries().len() != 1
    {
        return Err(invalid());
    }
    let entry = &report.entries()[0];
    if entry.attention != AttentionClass::Notable
        || entry.summary != expected_summary
        || !entry.sources.is_empty()
        || entry.entities
            != BTreeSet::from([
                EntityRef::Operation(operation.id()),
                EntityRef::Business(disposition.venue()),
            ])
        || entry.decision.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_operation_cash_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    transactions: &mut BTreeSet<LedgerTransactionId>,
    information_ids: &mut BTreeSet<InformationId>,
    reports: &mut BTreeSet<ReportId>,
) -> Result<(), StateValidationError> {
    use crate::operations::property_disposition::build_deposit_summary;

    let Some(disposition) = operation.cash_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationCashDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.cash_proceeds().ok_or_else(invalid)?;
    if disposition.disposed_at() < resolution.resolved_at()
        || disposition.disposed_at() > state.now()
        || disposition.realized_value() != proceeds.amount()
        || !transactions.insert(disposition.transaction())
        || !information_ids.insert(disposition.information())
        || !reports.insert(disposition.report())
    {
        return Err(invalid());
    }

    let cash = state
        .finance
        .get_account(disposition.cash_account())
        .ok_or_else(invalid)?;
    let settlement = state
        .finance
        .get_account(disposition.settlement_account())
        .ok_or_else(invalid)?;
    let expected_owner = FinancialOwner::Organization(operation.responsible_organization());
    if disposition.cash_account() == disposition.settlement_account()
        || cash.owner() != expected_owner
        || settlement.owner() != expected_owner
        || !matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        )
        || settlement.kind() != AccountKind::Settlement
    {
        return Err(invalid());
    }

    let transaction = state
        .finance
        .get_transaction(disposition.transaction())
        .ok_or_else(invalid)?;
    let negative_value = disposition
        .realized_value()
        .cents()
        .checked_neg()
        .map(Money::from_cents)
        .ok_or_else(invalid)?;
    let has_cash_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.cash_account()
            && posting.amount == disposition.realized_value()
    });
    let has_settlement_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.settlement_account() && posting.amount == negative_value
    });
    if transaction.occurred_at() != disposition.disposed_at()
        || transaction.memo() != format!("Cash deposit for {}", operation.id())
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid());
    }

    let summary = build_deposit_summary(operation.title(), proceeds.amount());
    let information = state
        .intelligence
        .get_information(disposition.information())
        .ok_or_else(invalid)?;
    if information.holder() != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::Accountant
        || information.topic() != InformationTopic::FinancialPerformance
        || information.source_entity() != Some(proceeds.target())
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != disposition.disposed_at()
        || information.recorded_at() != disposition.disposed_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || information.summary() != summary
    {
        return Err(invalid());
    }
    let report = state
        .reports
        .get_report(disposition.report())
        .ok_or_else(invalid)?;
    if report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::Financial
        || report.title() != "Cash deposit"
        || report.generated_at() != disposition.disposed_at()
        || report.entries().len() != 1
    {
        return Err(invalid());
    }
    let entry = &report.entries()[0];
    if entry.attention != AttentionClass::Notable
        || entry.summary != summary
        || !entry.sources.is_empty()
        || entry.entities
            != BTreeSet::from([EntityRef::Operation(operation.id()), proceeds.target()])
        || entry.decision.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_operation_discoveries(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    discovered_information: &mut BTreeSet<InformationId>,
) -> Result<(), StateValidationError> {
    match operation.kind() {
        OperationKind::Surveillance => {
            let OperationObjective::GatherInformation { target } = operation.objective() else {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            };
            if !is_supported_surveillance_target(*target) {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
            match resolution.objective_outcome() {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    if resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Failed
                    if !resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Achieved
                | OperationObjectiveOutcome::Partial
                | OperationObjectiveOutcome::Failed => {}
            }
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::WitnessPressure
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Extraction
        | OperationKind::Sabotage => {
            if !resolution.discovered_information().is_empty() {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
        }
    }

    let expected_signatures = expected_persisted_surveillance_signatures(state, operation);
    let mut actual_signatures = BTreeSet::new();
    for information_id in resolution.discovered_information() {
        let information = state.intelligence.get_information(*information_id).ok_or(
            StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            },
        )?;
        if !discovered_information.insert(*information_id)
            || !actual_signatures.insert((information.topic(), information.subject()))
            || state
                .operations
                .operation_for_discovered_information(*information_id)
                .is_none_or(|source| source.id() != operation.id())
            || information.recorded_at() != resolution.resolved_at()
            || !is_valid_persisted_surveillance_information(state, operation, information)
        {
            return Err(StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            });
        }
    }
    if operation.kind() == OperationKind::Surveillance
        && expected_signatures.as_ref() != Some(&actual_signatures)
    {
        return Err(StateValidationError::InvalidOperationDiscovery {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_operation_abort_links(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    abort: crate::operations::OperationAbortRecord,
    operation_after_action_information: &mut BTreeSet<InformationId>,
    operation_after_action_reports: &mut BTreeSet<ReportId>,
    operation_history_events: &mut BTreeSet<crate::core::id::HistoryEventId>,
) -> Result<(), StateValidationError> {
    if abort.aborted_at() > state.now() {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    match (abort.phase(), abort.cause(), abort.artifacts()) {
        (OperationAbortPhase::BeforeStart, OperationAbortCause::AuthorityOrder, None) => {
            if operation.started_at().is_some() || operation.resolution_due_at().is_some() {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            return Ok(());
        }
        (
            OperationAbortPhase::BeforeStart,
            OperationAbortCause::DeadlineMissed,
            Some(artifacts),
        ) => {
            let deadline = operation
                .constraints()
                .iter()
                .filter_map(|constraint| match constraint {
                    OperationConstraint::CompleteBefore(deadline) => Some(*deadline),
                    OperationConstraint::RequireIntelligenceTopic(_) => None,
                })
                .min();
            // Deadline-miss fires at `now >= deadline`, so an abort on the exact deadline
            // minute is valid — the same boundary the InProgress phase accepts.
            if operation.started_at().is_some()
                || operation.resolution_due_at().is_some()
                || deadline.is_none_or(|deadline| deadline > abort.aborted_at())
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (OperationAbortPhase::InProgress, OperationAbortCause::DeadlineMissed, Some(artifacts)) => {
            let (Some(started_at), Some(due_at)) =
                (operation.started_at(), operation.resolution_due_at())
            else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let deadline = operation
                .constraints()
                .iter()
                .filter_map(|constraint| match constraint {
                    OperationConstraint::CompleteBefore(deadline) => Some(*deadline),
                    OperationConstraint::RequireIntelligenceTopic(_) => None,
                })
                .min();
            if started_at > due_at
                || abort.aborted_at() < started_at
                || deadline.is_none_or(|deadline| deadline > abort.aborted_at())
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (OperationAbortPhase::InProgress, OperationAbortCause::AuthorityOrder, Some(artifacts)) => {
            let (Some(started_at), Some(due_at)) =
                (operation.started_at(), operation.resolution_due_at())
            else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            if started_at > due_at || abort.aborted_at() < started_at {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::InProgress,
            OperationAbortCause::PoliceArrival(response_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at), Some(entry_at)) = (
                operation.started_at(),
                operation.resolution_due_at(),
                operation.entry_at(),
            ) else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let response = state.legal.get_police_response(response_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || operation.police_response() != Some(response_id)
                || response.source_operation() != operation.id()
                || response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at > abort.aborted_at() || arrived_at >= entry_at
                })
                || !operation
                    .contingencies()
                    .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::AwaitingDecision,
            OperationAbortCause::DeadlineMissed,
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at), Some(paused_at)) = (
                operation.started_at(),
                operation.resolution_due_at(),
                operation.awaiting_decision_since(),
            ) else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let deadline = operation
                .constraints()
                .iter()
                .filter_map(|constraint| match constraint {
                    OperationConstraint::CompleteBefore(deadline) => Some(*deadline),
                    OperationConstraint::RequireIntelligenceTopic(_) => None,
                })
                .min();
            if started_at > due_at
                || started_at > paused_at
                || paused_at > abort.aborted_at()
                || deadline.is_none_or(|deadline| deadline > abort.aborted_at())
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::AwaitingDecision,
            OperationAbortCause::PoliceArrival(response_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at), Some(entry_at), Some(paused_at)) = (
                operation.started_at(),
                operation.resolution_due_at(),
                operation.entry_at(),
                operation.awaiting_decision_since(),
            ) else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let response = state.legal.get_police_response(response_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            let paused_minutes = abort
                .aborted_at()
                .as_minutes()
                .checked_sub(paused_at.as_minutes())
                .ok_or(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                })?;
            let projected_entry = if entry_at > paused_at {
                SimTime::from_minutes(entry_at.as_minutes().checked_add(paused_minutes).ok_or(
                    StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    },
                )?)
            } else {
                entry_at
            };
            let matching_continue_decisions = state
                .decisions
                .decisions()
                .filter(|decision| {
                    matches!(
                      decision.context(),
                      DecisionContext::OperationException {
                        operation: decision_operation,
                        reason: _,
                      } if decision_operation == operation.id()
                    ) && decision.resolution().is_some_and(|resolution| {
                        resolution.response() == DecisionResponse::Continue
                            && resolution.resolved_at() == abort.aborted_at()
                    })
                })
                .count();
            if started_at > due_at
                || started_at > paused_at
                || operation.police_response() != Some(response_id)
                || response.source_operation() != operation.id()
                || response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at > abort.aborted_at() || arrived_at >= projected_entry
                })
                || !operation
                    .contingencies()
                    .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
                || matching_continue_decisions != 1
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::AwaitingDecision,
            OperationAbortCause::Decision(decision_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at)) =
                (operation.started_at(), operation.resolution_due_at())
            else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let decision = state.decisions.get_decision(decision_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            let decision_matches = matches!(
              decision.context(),
              DecisionContext::OperationException {
                operation: decision_operation,
                reason: _,
              } if decision_operation == operation.id()
            );
            let resolution =
                decision
                    .resolution()
                    .ok_or(StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    })?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || !decision_matches
                || decision.status() != DecisionStatus::Resolved
                || decision.recipient() != operation.responsible_organization()
                || decision.requester() != operation.leader()
                || resolution.response() != DecisionResponse::Abort
                || resolution.resolved_at() != abort.aborted_at()
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (OperationAbortPhase::BeforeStart, _, Some(_))
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::DeadlineMissed, None)
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::Decision(_), None)
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::PoliceArrival(_), None)
        | (OperationAbortPhase::InProgress, _, None)
        | (OperationAbortPhase::InProgress, OperationAbortCause::Decision(_), Some(_))
        | (OperationAbortPhase::AwaitingDecision, _, None)
        | (OperationAbortPhase::AwaitingDecision, OperationAbortCause::AuthorityOrder, Some(_)) => {
            return Err(StateValidationError::InvalidOperationAbort {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_abort_artifacts(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    abort: crate::operations::OperationAbortRecord,
    artifacts: crate::operations::OperationAbortArtifacts,
    operation_after_action_information: &mut BTreeSet<InformationId>,
    operation_after_action_reports: &mut BTreeSet<ReportId>,
    operation_history_events: &mut BTreeSet<crate::core::id::HistoryEventId>,
) -> Result<(), StateValidationError> {
    let information = state
        .intelligence
        .get_information(artifacts.information())
        .ok_or(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        })?;
    if !operation_after_action_information.insert(information.id())
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::OperationalOutcome
        || information.source_entity() != Some(EntityRef::Character(operation.leader()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != abort.aborted_at()
        || information.recorded_at() != abort.aborted_at()
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    // District-scoped enforcement knowledge exists exactly when the abort was caused by a
    // pre-entry police arrival: that is the only abort path where the debriefed crew gives
    // the organization first-hand knowledge of a response in the target's neighborhood.
    let expected_police_activity = match abort.cause() {
        OperationAbortCause::PoliceArrival(response) => state
            .legal
            .get_police_response(response)
            .map(|response| (response.authority(), response.neighborhood())),
        OperationAbortCause::AuthorityOrder
        | OperationAbortCause::Decision(_)
        | OperationAbortCause::DeadlineMissed => None,
    };
    match (
        artifacts.police_activity_information(),
        expected_police_activity,
    ) {
        (Some(information_id), Some((authority, neighborhood))) => {
            let police = state.intelligence.get_information(information_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            if police.holder()
                != KnowledgeHolder::Organization(operation.responsible_organization())
                || police.source_kind() != InformationSourceKind::AfterAction
                || police.topic() != InformationTopic::PoliceActivity
                || police.source_entity() != Some(EntityRef::Organization(authority))
                || police.subject() != EntityRef::Neighborhood(neighborhood)
                || police.observed_at() != abort.aborted_at()
                || police.recorded_at() != abort.aborted_at()
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
        }
        (None, None) => {}
        _ => {
            return Err(StateValidationError::InvalidOperationAbort {
                operation: operation.id(),
            });
        }
    }

    let report = state.reports.get_report(artifacts.report()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    let report_entry = report.entries().first();
    if !operation_after_action_reports.insert(report.id())
        || report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::AfterAction
        || report.title() != format!("{} after-action report", operation.title())
        || report.generated_at() != abort.aborted_at()
        || report.entries().len() != 1
        || !report_entry.is_some_and(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == information.summary()
                && entry.sources.is_empty()
                && entry.decision.is_none()
                && entry
                    .entities
                    .contains(&EntityRef::Operation(operation.id()))
                && entry.entities.contains(&EntityRef::Organization(
                    operation.responsible_organization(),
                ))
                && entry
                    .entities
                    .contains(&EntityRef::Character(operation.leader()))
                && match abort.cause() {
                    OperationAbortCause::AuthorityOrder => true,
                    OperationAbortCause::Decision(decision) => entry
                        .entities
                        .contains(&EntityRef::DecisionRequest(decision)),
                    OperationAbortCause::PoliceArrival(response) => state
                        .legal
                        .get_police_response(response)
                        .is_some_and(|response| {
                            entry
                                .entities
                                .contains(&EntityRef::Organization(response.authority()))
                                && entry
                                    .entities
                                    .contains(&EntityRef::Neighborhood(response.neighborhood()))
                        }),
                    OperationAbortCause::DeadlineMissed => true,
                }
        })
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    let history = state.history.get_event(artifacts.history_event()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    if !operation_history_events.insert(history.id())
        || history.kind() != HistoryEventKind::Operation
        || history.occurred_at() != abort.aborted_at()
        || history.summary() != information.summary()
        || !history
            .entities()
            .contains(&EntityRef::Operation(operation.id()))
        || !history.entities().contains(&EntityRef::Organization(
            operation.responsible_organization(),
        ))
        || !history
            .entities()
            .contains(&EntityRef::Character(operation.leader()))
        || match abort.cause() {
            OperationAbortCause::AuthorityOrder => false,
            OperationAbortCause::Decision(decision) => !history
                .entities()
                .contains(&EntityRef::DecisionRequest(decision)),
            OperationAbortCause::PoliceArrival(response) => state
                .legal
                .get_police_response(response)
                .is_none_or(|response| {
                    !history
                        .entities()
                        .contains(&EntityRef::Organization(response.authority()))
                        || !history
                            .entities()
                            .contains(&EntityRef::Neighborhood(response.neighborhood()))
                }),
            OperationAbortCause::DeadlineMissed => false,
        }
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }
    Ok(())
}
