//! Persisted proceeds-disposition validation for completed operations.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{FinancialAccountId, InformationId, LedgerTransactionId, ReportId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::operations::property_disposition::{
    write_deposit_memo, write_deposit_summary, write_disposition_summary, write_liquidation_memo,
};
use crate::operations::{
    OperationCashDispositionRecord, OperationPropertyDispositionRecord, OperationRecord,
    OperationResolutionRecord,
};
use crate::reports::ReportKind;
use crate::world::{BusinessFunction, BusinessOwner};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct DispositionInvariantContext {
    transactions: BTreeSet<LedgerTransactionId>,
    information: BTreeSet<InformationId>,
    reports: BTreeSet<ReportId>,
    memo: String,
    summary: String,
}

#[derive(Clone, Copy)]
struct CommonDisposition {
    disposed_at: SimTime,
    realized_value: Money,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    transaction: LedgerTransactionId,
    information: InformationId,
    report: ReportId,
}

#[derive(Clone, Copy)]
struct DispositionArtifactSpec<'a> {
    source_entity: EntityRef,
    report_title: &'a str,
    expected_summary: &'a str,
}

#[derive(Clone, Copy)]
struct DispositionValidationSpec {
    disposition: CommonDisposition,
    source_entity: EntityRef,
    report_title: &'static str,
}

impl From<OperationCashDispositionRecord> for CommonDisposition {
    fn from(record: OperationCashDispositionRecord) -> Self {
        Self {
            disposed_at: record.disposed_at(),
            realized_value: record.realized_value(),
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
            transaction: record.transaction(),
            information: record.information(),
            report: record.report(),
        }
    }
}

impl From<OperationPropertyDispositionRecord> for CommonDisposition {
    fn from(record: OperationPropertyDispositionRecord) -> Self {
        Self {
            disposed_at: record.disposed_at(),
            realized_value: record.realized_value(),
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
            transaction: record.transaction(),
            information: record.information(),
            report: record.report(),
        }
    }
}

pub(super) fn validate_operation_property_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    context: &mut DispositionInvariantContext,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.property_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationPropertyDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.property_proceeds().ok_or_else(invalid)?;
    if disposition.realized_value().cents() <= 0
        || disposition.realized_value().cents() > proceeds.estimated_value().cents()
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
    let next_ownership_before_disposition = disposition
        .venue_version()
        .checked_add(1)
        .and_then(|version| {
            state
                .world
                .get_business_ownership_change_for_version(disposition.venue(), version)
        })
        .is_some_and(|next| next.changed_at() < disposition.disposed_at());
    if disposition.venue_version() > venue.version()
        || ownership.new_owner()
            != BusinessOwner::Organization(operation.responsible_organization())
        || ownership.changed_at() > disposition.disposed_at()
        // `venue_version` is the exact ownership revision observed by the canonical disposition
        // transaction. Cross-domain events sharing one SimTime have no persisted sub-minute
        // order, so a later ownership revision at the same minute may legitimately have followed
        // the liquidation. Only a strictly earlier next revision proves the pinned version could
        // not have been current when disposition committed.
        || next_ownership_before_disposition
        || !venue.has_function(BusinessFunction::ResaleMarket)
    {
        return Err(invalid());
    }

    context.memo.clear();
    write_liquidation_memo(&mut context.memo, operation.id(), disposition.venue())
        .expect("String buffer writes are infallible");
    // The disposition summary is re-rendered once per record from the same template the
    // commit path used and compared against both the persisted information and the
    // persisted report entry.
    context.summary.clear();
    write_disposition_summary(
        &mut context.summary,
        operation.title(),
        venue.name(),
        proceeds.estimated_value(),
        disposition.realized_value(),
    )
    .expect("String buffer writes are infallible");
    validate_common_disposition(
        state,
        operation,
        resolution,
        DispositionValidationSpec {
            disposition: disposition.into(),
            source_entity: EntityRef::Business(disposition.venue()),
            report_title: "Property disposition",
        },
        context,
        invalid(),
    )
}

pub(super) fn validate_operation_cash_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    context: &mut DispositionInvariantContext,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.cash_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationCashDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.cash_proceeds().ok_or_else(invalid)?;
    if disposition.realized_value() != proceeds.amount() {
        return Err(invalid());
    }
    context.memo.clear();
    write_deposit_memo(&mut context.memo, operation.id())
        .expect("String buffer writes are infallible");
    context.summary.clear();
    write_deposit_summary(&mut context.summary, operation.title(), proceeds.amount())
        .expect("String buffer writes are infallible");
    validate_common_disposition(
        state,
        operation,
        resolution,
        DispositionValidationSpec {
            disposition: disposition.into(),
            source_entity: proceeds.target(),
            report_title: "Cash deposit",
        },
        context,
        invalid(),
    )
}

fn validate_common_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    spec: DispositionValidationSpec,
    context: &mut DispositionInvariantContext,
    invalid: StateValidationError,
) -> Result<(), StateValidationError> {
    let disposition = spec.disposition;
    if disposition.disposed_at < resolution.resolved_at()
        || disposition.disposed_at > state.now()
        || !context.transactions.insert(disposition.transaction)
        || !context.information.insert(disposition.information)
        || !context.reports.insert(disposition.report)
    {
        return Err(invalid);
    }
    validate_disposition_accounts(state, operation, disposition, &invalid)?;
    validate_disposition_transaction(state, disposition, context.memo.as_str(), &invalid)?;
    let artifacts = DispositionArtifactSpec {
        source_entity: spec.source_entity,
        report_title: spec.report_title,
        expected_summary: context.summary.as_str(),
    };
    validate_disposition_information(state, operation, disposition, artifacts, &invalid)?;
    validate_disposition_report(state, operation, disposition, artifacts, &invalid)
}

fn validate_disposition_accounts(
    state: &AppState,
    operation: &OperationRecord,
    disposition: CommonDisposition,
    invalid: &StateValidationError,
) -> Result<(), StateValidationError> {
    let cash = state
        .finance
        .get_account(disposition.cash_account)
        .ok_or_else(|| invalid.clone())?;
    let settlement = state
        .finance
        .get_account(disposition.settlement_account)
        .ok_or_else(|| invalid.clone())?;
    let expected_owner = FinancialOwner::Organization(operation.responsible_organization());
    if disposition.cash_account == disposition.settlement_account
        || cash.owner() != expected_owner
        || settlement.owner() != expected_owner
        || !matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        )
        || settlement.kind() != AccountKind::Settlement
        || settlement_account_was_enterprise_reserved_at(
            state,
            disposition.settlement_account,
            disposition.disposed_at,
        )
    {
        return Err(invalid.clone());
    }
    Ok(())
}

fn validate_disposition_transaction(
    state: &AppState,
    disposition: CommonDisposition,
    expected_memo: &str,
    invalid: &StateValidationError,
) -> Result<(), StateValidationError> {
    let transaction = state
        .finance
        .get_transaction(disposition.transaction)
        .ok_or_else(|| invalid.clone())?;
    let negative_value = disposition
        .realized_value
        .cents()
        .checked_neg()
        .map(Money::from_cents)
        .ok_or_else(|| invalid.clone())?;
    let has_cash_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.cash_account && posting.amount == disposition.realized_value
    });
    let has_settlement_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.settlement_account && posting.amount == negative_value
    });
    if transaction.occurred_at() != disposition.disposed_at
        || transaction.memo() != expected_memo
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid.clone());
    }
    Ok(())
}

fn validate_disposition_information(
    state: &AppState,
    operation: &OperationRecord,
    disposition: CommonDisposition,
    artifacts: DispositionArtifactSpec<'_>,
    invalid: &StateValidationError,
) -> Result<(), StateValidationError> {
    let information = state
        .intelligence
        .get_information(disposition.information)
        .ok_or_else(|| invalid.clone())?;
    if information.holder() != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::Accounting
        || information.topic() != InformationTopic::FinancialPerformance
        || information.source_entity() != Some(artifacts.source_entity)
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != disposition.disposed_at
        || information.recorded_at() != disposition.disposed_at
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || information.summary() != artifacts.expected_summary
    {
        return Err(invalid.clone());
    }
    Ok(())
}

fn validate_disposition_report(
    state: &AppState,
    operation: &OperationRecord,
    disposition: CommonDisposition,
    artifacts: DispositionArtifactSpec<'_>,
    invalid: &StateValidationError,
) -> Result<(), StateValidationError> {
    let report = state
        .reports
        .get_report(disposition.report)
        .ok_or_else(|| invalid.clone())?;
    if report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::Financial
        || report.title() != artifacts.report_title
        || report.generated_at() != disposition.disposed_at
        || report.entries().len() != 1
    {
        return Err(invalid.clone());
    }
    let entry = &report.entries()[0];
    if entry.attention != AttentionClass::Notable
        || entry.summary != artifacts.expected_summary
        || !entry.sources.is_empty()
        || entry.entities.len() != 2
        || !entry
            .entities
            .contains(&EntityRef::Operation(operation.id()))
        || !entry.entities.contains(&artifacts.source_entity)
        || entry.decision.is_some()
    {
        return Err(invalid.clone());
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
