//! Persisted proceeds-disposition validation for completed operations.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{InformationId, LedgerTransactionId, ReportId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::operations::property_disposition::{
    write_deposit_memo, write_deposit_summary, write_disposition_summary, write_liquidation_memo,
};
use crate::operations::{OperationRecord, OperationResolutionRecord};
use crate::reports::ReportKind;
use crate::world::{BusinessFunction, BusinessOwner};
use std::collections::BTreeSet;

pub(super) fn validate_operation_property_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    transactions: &mut BTreeSet<LedgerTransactionId>,
    information_ids: &mut BTreeSet<InformationId>,
    reports: &mut BTreeSet<ReportId>,
    text_scratch: &mut String,
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
        || settlement_account_was_enterprise_reserved_at(
            state,
            disposition.settlement_account(),
            disposition.disposed_at(),
        )
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
    text_scratch.clear();
    write_liquidation_memo(text_scratch, operation.id(), disposition.venue())
        .expect("String buffer writes are infallible");
    if transaction.occurred_at() != disposition.disposed_at()
        || transaction.memo() != text_scratch.as_str()
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid());
    }

    // The disposition summary is re-rendered once per record from the same template the
    // commit path used and compared against both the persisted information and the
    // persisted report entry.
    text_scratch.clear();
    write_disposition_summary(
        text_scratch,
        operation.title(),
        venue.name(),
        proceeds.estimated_value(),
        disposition.realized_value(),
    )
    .expect("String buffer writes are infallible");
    let expected_summary = text_scratch.as_str();
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
        || information.summary() != expected_summary
    {
        return Err(invalid());
    }
    let report = state
        .reports
        .get_report(disposition.report())
        .ok_or_else(invalid)?;
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
        || entry.entities.len() != 2
        || !entry
            .entities
            .contains(&EntityRef::Operation(operation.id()))
        || !entry
            .entities
            .contains(&EntityRef::Business(disposition.venue()))
        || entry.decision.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn validate_operation_cash_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    transactions: &mut BTreeSet<LedgerTransactionId>,
    information_ids: &mut BTreeSet<InformationId>,
    reports: &mut BTreeSet<ReportId>,
    text_scratch: &mut String,
) -> Result<(), StateValidationError> {
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
        || settlement_account_was_enterprise_reserved_at(
            state,
            disposition.settlement_account(),
            disposition.disposed_at(),
        )
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
    text_scratch.clear();
    write_deposit_memo(text_scratch, operation.id()).expect("String buffer writes are infallible");
    if transaction.occurred_at() != disposition.disposed_at()
        || transaction.memo() != text_scratch.as_str()
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid());
    }

    text_scratch.clear();
    write_deposit_summary(text_scratch, operation.title(), proceeds.amount())
        .expect("String buffer writes are infallible");
    let summary = text_scratch.as_str();
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
        || entry.entities.len() != 2
        || !entry
            .entities
            .contains(&EntityRef::Operation(operation.id()))
        || !entry.entities.contains(&proceeds.target())
        || entry.decision.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

/// Whether an enterprise had already dedicated this organization settlement account when a
/// historical operation disposition occurred. A later enterprise may legitimately reuse the
/// account: disposition uses it as a one-off external balancing counterparty and does not reserve
/// it forever. Equal timestamps are accepted because the persisted model has no cross-domain
/// sequence number and the canonical disposition-then-establishment order is valid in one minute.
fn settlement_account_was_enterprise_reserved_at(
    state: &AppState,
    account: crate::core::id::FinancialAccountId,
    disposed_at: crate::core::time::SimTime,
) -> bool {
    state
        .enterprises
        .get_by_settlement_account(account)
        .is_some_and(|enterprise| enterprise.established_at() < disposed_at)
}
