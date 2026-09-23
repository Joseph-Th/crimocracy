//! Organization-level financial report synthesis across legitimate businesses and illicit enterprises.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessCycleId, BusinessId, EnterpriseCycleId, EnterpriseId, InformationId, OperationId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::economy::business_reporting::{
    BusinessReportingError, resolve_organization_business_financial_summary,
};
use crate::enterprises::enterprise_reporting::{
    EnterpriseReportingError, resolve_organization_enterprise_financial_summary,
};
use crate::finance::{FinancialOwner, Money};
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::BusinessOwner;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OrganizationFinancialReportError {
    #[error("notable financial cycle is missing its information record")]
    MissingNotableInformation,
    #[error("notable financial cycle references missing information {0}")]
    MissingNotableInformationRecord(InformationId),
    #[error("organization financial aggregation overflowed")]
    ArithmeticOverflow,
    #[error(transparent)]
    Business(#[from] BusinessReportingError),
    #[error(transparent)]
    Enterprise(#[from] EnterpriseReportingError),
    #[error(transparent)]
    Report(#[from] ReportError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotableFinancialItem {
    Business {
        cycle: BusinessCycleId,
        business: BusinessId,
        information: InformationId,
    },
    Enterprise {
        cycle: EnterpriseCycleId,
        enterprise: EnterpriseId,
        information: InformationId,
    },
    OperationProperty {
        operation: OperationId,
        information: InformationId,
    },
    OperationPropertyDisposition {
        operation: OperationId,
        information: InformationId,
    },
    OperationCash {
        operation: OperationId,
        information: InformationId,
    },
    OperationCashDisposition {
        operation: OperationId,
        information: InformationId,
    },
}

impl NotableFinancialItem {
    /// Stable ordering key for same-minute entries: the underlying record ids first, so
    /// entries order by the records they describe rather than by enum declaration order.
    /// The trailing tag only breaks exact raw-id collisions across different id types.
    fn sort_key(self) -> (u32, u32, u32, u8) {
        match self {
            Self::Business {
                cycle,
                business,
                information,
            } => (cycle.raw(), business.raw(), information.raw(), 0),
            Self::Enterprise {
                cycle,
                enterprise,
                information,
            } => (cycle.raw(), enterprise.raw(), information.raw(), 1),
            Self::OperationProperty {
                operation,
                information,
            } => (operation.raw(), information.raw(), 0, 2),
            Self::OperationPropertyDisposition {
                operation,
                information,
            } => (operation.raw(), information.raw(), 0, 3),
            Self::OperationCash {
                operation,
                information,
            } => (operation.raw(), information.raw(), 0, 4),
            Self::OperationCashDisposition {
                operation,
                information,
            } => (operation.raw(), information.raw(), 0, 5),
        }
    }
}

#[derive(Default)]
struct OperationFinancialSummary {
    held_property_count: u32,
    held_property_value: Money,
    property_disposition_count: u32,
    realized_property_cash: Money,
    held_cash_count: u32,
    held_cash_value: Money,
    cash_deposit_count: u32,
    deposited_cash: Money,
    notable: Vec<(SimTime, NotableFinancialItem)>,
}

pub fn validate_organization_financial_report(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<ValidatedReport, OrganizationFinancialReportError> {
    let business_summary = resolve_organization_business_financial_summary(
        state,
        recipient,
        period_start,
        period_end,
    )?;
    let enterprise_summary = resolve_organization_enterprise_financial_summary(
        state,
        recipient,
        period_start,
        period_end,
    )?;
    let operation_summary =
        resolve_operation_financial_summary(state, recipient, period_start, period_end)?;
    let liquid_cash = resolve_organization_liquid_cash(state, recipient, period_end)?;
    let money = crate::finance::helpers::format_money_cents;
    let mut entries = vec![ReportEntry {
        attention: AttentionClass::Routine,
        summary: format!(
            "Liquid organization cash: {}. Legitimate businesses: {} businesses, {} cycles, gross {}, operating cost {}, net {}. Illicit enterprises: {} enterprises, {} cycles, gross {}, operating cost {}, net {}. Held operation property at period end: {} operation(s), estimated value {}, unliquidated. Liquidated operation property during period: {} disposition(s), realized cash {}. Held operation cash at period end: {} operation(s), amount {}, undeposited. Deposited operation cash during period: {} deposit(s), amount {}.",
            money(liquid_cash.cents()),
            business_summary.totals.business_count,
            business_summary.totals.cycle_count,
            money(business_summary.totals.gross_revenue.cents()),
            money(business_summary.totals.operating_cost.cents()),
            money(business_summary.totals.net_cash.cents()),
            enterprise_summary.totals.enterprise_count,
            enterprise_summary.totals.cycle_count,
            money(enterprise_summary.totals.gross_revenue.cents()),
            money(enterprise_summary.totals.operating_cost.cents()),
            money(enterprise_summary.totals.net_cash.cents()),
            operation_summary.held_property_count,
            money(operation_summary.held_property_value.cents()),
            operation_summary.property_disposition_count,
            money(operation_summary.realized_property_cash.cents()),
            operation_summary.held_cash_count,
            money(operation_summary.held_cash_value.cents()),
            operation_summary.cash_deposit_count,
            money(operation_summary.deposited_cash.cents()),
        ),
        sources: Vec::new(),
        entities: BTreeSet::new(),
        decision: None,
    }];

    let mut notable = collect_notable_business_cycles(state, recipient, period_start, period_end)?;
    notable.extend(collect_notable_enterprise_cycles(
        state,
        recipient,
        period_start,
        period_end,
    )?);
    notable.extend(operation_summary.notable);
    notable.sort_by_key(|(occurred_at, item)| (*occurred_at, item.sort_key()));
    for (_, item) in notable {
        entries.push(build_notable_entry(state, item)?);
    }

    Ok(validate_record_report(
        state,
        ReportDraft {
            recipient,
            kind: ReportKind::Financial,
            title: "Organization financial report".to_owned(),
            entries,
        },
    )?)
}

fn resolve_organization_liquid_cash(
    state: &AppState,
    recipient: OrganizationId,
    period_end: SimTime,
) -> Result<Money, OrganizationFinancialReportError> {
    let owner = FinancialOwner::Organization(recipient);
    if period_end == state.now() {
        return state
            .finance()
            .accounts_for(owner)
            .try_fold(Money::ZERO, |total, account| {
                total
                    .checked_add(account.spendable_balance())
                    .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)
            });
    }

    // A historical report must not mix current materialized balances with an earlier reporting
    // window. Account ownership and kind are immutable, so replay only each owned liquid
    // account's indexed transaction chronology through the requested end minute.
    let accounts = state
        .finance()
        .accounts_for(owner)
        .filter(|account| account.kind().is_liquid())
        .map(|account| account.id())
        .collect::<Vec<_>>();
    let mut total = Money::ZERO;
    for account in accounts {
        let mut balance = Money::ZERO;
        for transaction in state
            .finance()
            .transactions_for_account_through(account, period_end)
        {
            let posting = transaction
                .postings()
                .iter()
                .find(|posting| posting.account == account)
                .expect("account transaction index must correspond to one ledger posting");
            balance = balance
                .checked_add(posting.amount)
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        }
        if balance > Money::ZERO {
            total = total
                .checked_add(balance)
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        }
    }
    Ok(total)
}

fn resolve_operation_financial_summary(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<OperationFinancialSummary, OrganizationFinancialReportError> {
    let mut summary = OperationFinancialSummary::default();
    if period_end == state.now() {
        for operation in state.operations().held_property_for_organization(recipient) {
            let proceeds = operation
                .resolution()
                .and_then(|resolution| resolution.property_proceeds())
                .expect("held-property index must reference persisted property proceeds");
            summary.held_property_count = summary
                .held_property_count
                .checked_add(1)
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
            summary.held_property_value = summary
                .held_property_value
                .checked_add(proceeds.estimated_value())
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        }
        for operation in state.operations().held_cash_for_organization(recipient) {
            let proceeds = operation
                .resolution()
                .and_then(|resolution| resolution.cash_proceeds())
                .expect("held-cash index must reference persisted cash proceeds");
            summary.held_cash_count = summary
                .held_cash_count
                .checked_add(1)
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
            summary.held_cash_value = summary
                .held_cash_value
                .checked_add(proceeds.amount())
                .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        }
    } else {
        // Historical holdings are reconstructed from sparse financial resolutions only. A
        // disposition after the requested end leaves the proceeds held at that historical instant.
        for operation in state
            .operations()
            .financial_resolutions_for_organization_through(recipient, period_end)
        {
            let resolution = operation
                .resolution()
                .expect("financial-resolution index must reference a completed resolution");
            accumulate_held_operation_proceeds(&mut summary, operation, resolution, period_end)?;
        }
    }
    for operation in state
        .operations()
        .financial_resolutions_for_organization_from_through(recipient, period_start, period_end)
    {
        accumulate_operation_resolution_notable(&mut summary, operation)?;
    }
    for operation in state
        .operations()
        .property_dispositions_for_organization_from_through(recipient, period_start, period_end)
    {
        accumulate_property_disposition(&mut summary, operation)?;
    }
    for operation in state
        .operations()
        .cash_dispositions_for_organization_from_through(recipient, period_start, period_end)
    {
        accumulate_cash_disposition(&mut summary, operation)?;
    }
    Ok(summary)
}

fn accumulate_operation_resolution_notable(
    summary: &mut OperationFinancialSummary,
    operation: &crate::operations::OperationRecord,
) -> Result<(), OrganizationFinancialReportError> {
    let resolution = operation
        .resolution()
        .expect("financial-resolution index must reference a completed resolution");
    if resolution.property_proceeds().is_some() {
        summary.notable.push((
            resolution.resolved_at(),
            NotableFinancialItem::OperationProperty {
                operation: operation.id(),
                information: resolution.after_action_information(),
            },
        ));
    }
    if resolution.cash_proceeds().is_some() {
        summary.notable.push((
            resolution.resolved_at(),
            NotableFinancialItem::OperationCash {
                operation: operation.id(),
                information: resolution.after_action_information(),
            },
        ));
    }
    Ok(())
}

fn accumulate_held_operation_proceeds(
    summary: &mut OperationFinancialSummary,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    period_end: SimTime,
) -> Result<(), OrganizationFinancialReportError> {
    if let Some(proceeds) = resolution.property_proceeds()
        && !operation
            .property_disposition()
            .is_some_and(|disposition| disposition.disposed_at() <= period_end)
    {
        summary.held_property_count = summary
            .held_property_count
            .checked_add(1)
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        summary.held_property_value = summary
            .held_property_value
            .checked_add(proceeds.estimated_value())
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    }
    if let Some(proceeds) = resolution.cash_proceeds()
        && !operation
            .cash_disposition()
            .is_some_and(|disposition| disposition.disposed_at() <= period_end)
    {
        summary.held_cash_count = summary
            .held_cash_count
            .checked_add(1)
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        summary.held_cash_value = summary
            .held_cash_value
            .checked_add(proceeds.amount())
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    }
    Ok(())
}

fn accumulate_property_disposition(
    summary: &mut OperationFinancialSummary,
    operation: &crate::operations::OperationRecord,
) -> Result<(), OrganizationFinancialReportError> {
    let disposition = operation
        .property_disposition()
        .expect("property-disposition index must reference a persisted disposition");
    summary.property_disposition_count = summary
        .property_disposition_count
        .checked_add(1)
        .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    summary.realized_property_cash = summary
        .realized_property_cash
        .checked_add(disposition.realized_value())
        .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    summary.notable.push((
        disposition.disposed_at(),
        NotableFinancialItem::OperationPropertyDisposition {
            operation: operation.id(),
            information: disposition.information(),
        },
    ));
    Ok(())
}

fn accumulate_cash_disposition(
    summary: &mut OperationFinancialSummary,
    operation: &crate::operations::OperationRecord,
) -> Result<(), OrganizationFinancialReportError> {
    let disposition = operation
        .cash_disposition()
        .expect("cash-disposition index must reference a persisted disposition");
    summary.cash_deposit_count = summary
        .cash_deposit_count
        .checked_add(1)
        .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    summary.deposited_cash = summary
        .deposited_cash
        .checked_add(disposition.realized_value())
        .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    summary.notable.push((
        disposition.disposed_at(),
        NotableFinancialItem::OperationCashDisposition {
            operation: operation.id(),
            information: disposition.information(),
        },
    ));
    Ok(())
}

fn collect_notable_business_cycles(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<Vec<(SimTime, NotableFinancialItem)>, OrganizationFinancialReportError> {
    let mut items = Vec::new();
    for cycle in state
        .economy()
        .cycles_from_through(period_start, period_end)
        .filter(|cycle| {
            cycle.owner() == BusinessOwner::Organization(recipient)
                && cycle.attention() == AttentionClass::Notable
        })
    {
        let information = cycle
            .information()
            .ok_or(OrganizationFinancialReportError::MissingNotableInformation)?;
        items.push((
            cycle.occurred_at(),
            NotableFinancialItem::Business {
                cycle: cycle.id(),
                business: cycle.business(),
                information,
            },
        ));
    }
    Ok(items)
}

fn collect_notable_enterprise_cycles(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<Vec<(SimTime, NotableFinancialItem)>, OrganizationFinancialReportError> {
    let mut items = Vec::new();
    for cycle in state
        .enterprises()
        .cycles_from_through(period_start, period_end)
        .filter(|cycle| cycle.attention() == AttentionClass::Notable)
    {
        let enterprise = state
            .enterprises()
            .get_enterprise(cycle.enterprise())
            .expect("enterprise cycle must reference its persisted enterprise");
        if enterprise.organization() != recipient {
            continue;
        }
        let information = cycle
            .information()
            .ok_or(OrganizationFinancialReportError::MissingNotableInformation)?;
        items.push((
            cycle.occurred_at(),
            NotableFinancialItem::Enterprise {
                cycle: cycle.id(),
                enterprise: cycle.enterprise(),
                information,
            },
        ));
    }
    Ok(items)
}

fn build_notable_entry(
    state: &AppState,
    item: NotableFinancialItem,
) -> Result<ReportEntry, OrganizationFinancialReportError> {
    let (information, entity, label) = match item {
        NotableFinancialItem::Business {
            cycle,
            business,
            information,
        } => (
            information,
            EntityRef::Business(business),
            format!("Business cycle {cycle}"),
        ),
        NotableFinancialItem::Enterprise {
            cycle,
            enterprise,
            information,
        } => (
            information,
            EntityRef::Enterprise(enterprise),
            format!("Enterprise cycle {cycle}"),
        ),
        NotableFinancialItem::OperationProperty {
            operation,
            information,
        } => (
            information,
            EntityRef::Operation(operation),
            format!("Operation proceeds {operation}"),
        ),
        NotableFinancialItem::OperationPropertyDisposition {
            operation,
            information,
        } => (
            information,
            EntityRef::Operation(operation),
            format!("Property disposition {operation}"),
        ),
        NotableFinancialItem::OperationCash {
            operation,
            information,
        } => (
            information,
            EntityRef::Operation(operation),
            format!("Operation cash proceeds {operation}"),
        ),
        NotableFinancialItem::OperationCashDisposition {
            operation,
            information,
        } => (
            information,
            EntityRef::Operation(operation),
            format!("Cash deposit {operation}"),
        ),
    };
    let record = state
        .intelligence()
        .get_information(information)
        .ok_or(OrganizationFinancialReportError::MissingNotableInformationRecord(information))?;
    Ok(ReportEntry {
        attention: AttentionClass::Notable,
        summary: format!("{label}: {}", record.summary()),
        sources: vec![information],
        entities: BTreeSet::from([entity]),
        decision: None,
    })
}
