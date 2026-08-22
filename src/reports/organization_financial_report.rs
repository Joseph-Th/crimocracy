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
    resolve_organization_business_financial_summary, BusinessReportingError,
};
use crate::enterprises::enterprise_reporting::{
    resolve_organization_enterprise_financial_summary, EnterpriseReportingError,
};
use crate::finance::Money;
use crate::operations::OperationStatus;
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    let (property_operation_count, held_property_value) =
        resolve_held_operation_property(state, recipient, period_end)?;
    let (property_disposition_count, realized_property_cash) =
        resolve_liquidated_operation_property(state, recipient, period_start, period_end)?;
    let money = crate::finance::helpers::format_money_cents;
    let mut entries = vec![ReportEntry {
        attention: AttentionClass::Routine,
        summary: format!(
            "Legitimate businesses: {} businesses, {} cycles, gross {}, operating cost {}, net {}. Illicit enterprises: {} enterprises, {} cycles, gross {}, operating cost {}, net {}. Held operation property at period end: {} operation(s), estimated value {}, unliquidated. Liquidated operation property during period: {} disposition(s), realized cash {}.",
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
            property_operation_count,
            money(held_property_value.cents()),
            property_disposition_count,
            money(realized_property_cash.cents()),
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
    notable.extend(collect_operation_property_items(
        state,
        recipient,
        period_start,
        period_end,
    ));
    notable.extend(collect_operation_property_disposition_items(
        state,
        recipient,
        period_start,
        period_end,
    ));
    notable.sort_by_key(|(occurred_at, item)| (*occurred_at, *item));
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

fn resolve_held_operation_property(
    state: &AppState,
    recipient: OrganizationId,
    period_end: SimTime,
) -> Result<(u32, Money), OrganizationFinancialReportError> {
    let mut count = 0_u32;
    let mut value = Money::ZERO;
    for operation in state.operations().operations_for_organization(recipient) {
        let Some(resolution) = operation.resolution() else {
            continue;
        };
        if resolution.resolved_at() > period_end {
            continue;
        }
        let Some(proceeds) = resolution.property_proceeds() else {
            continue;
        };
        if operation
            .property_disposition()
            .is_some_and(|disposition| disposition.disposed_at() <= period_end)
        {
            continue;
        }
        count = count
            .checked_add(1)
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        value = value
            .checked_add(proceeds.estimated_value())
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    }
    Ok((count, value))
}

fn resolve_liquidated_operation_property(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<(u32, Money), OrganizationFinancialReportError> {
    let mut count = 0_u32;
    let mut value = Money::ZERO;
    for operation in state.operations().operations_for_organization(recipient) {
        let Some(disposition) = operation.property_disposition() else {
            continue;
        };
        if disposition.disposed_at() < period_start || disposition.disposed_at() > period_end {
            continue;
        }
        count = count
            .checked_add(1)
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
        value = value
            .checked_add(disposition.realized_value())
            .ok_or(OrganizationFinancialReportError::ArithmeticOverflow)?;
    }
    Ok((count, value))
}

fn collect_operation_property_items(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Vec<(SimTime, NotableFinancialItem)> {
    state
        .operations()
        .operations_for_organization(recipient)
        .filter(|operation| operation.status() == OperationStatus::Completed)
        .filter_map(|operation| {
            let resolution = operation.resolution()?;
            (resolution.resolved_at() >= period_start
                && resolution.resolved_at() <= period_end
                && resolution.property_proceeds().is_some())
            .then_some((
                resolution.resolved_at(),
                NotableFinancialItem::OperationProperty {
                    operation: operation.id(),
                    information: resolution.after_action_information(),
                },
            ))
        })
        .collect()
}

fn collect_operation_property_disposition_items(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Vec<(SimTime, NotableFinancialItem)> {
    state
        .operations()
        .operations_for_organization(recipient)
        .filter_map(|operation| {
            let disposition = operation.property_disposition()?;
            (disposition.disposed_at() >= period_start && disposition.disposed_at() <= period_end)
                .then_some((
                    disposition.disposed_at(),
                    NotableFinancialItem::OperationPropertyDisposition {
                        operation: operation.id(),
                        information: disposition.information(),
                    },
                ))
        })
        .collect()
}

fn collect_notable_business_cycles(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<Vec<(SimTime, NotableFinancialItem)>, OrganizationFinancialReportError> {
    let mut items = Vec::new();
    for business in state.world.businesses().filter(|business| {
        state
            .economy()
            .get_business_economy(business.id())
            .is_some()
    }) {
        for cycle in state.economy().cycles_for(business.id()).filter(|cycle| {
            cycle.occurred_at() >= period_start
                && cycle.occurred_at() <= period_end
                && cycle.owner() == BusinessOwner::Organization(recipient)
                && cycle.attention() == AttentionClass::Notable
        }) {
            let information = cycle
                .information()
                .ok_or(OrganizationFinancialReportError::MissingNotableInformation)?;
            items.push((
                cycle.occurred_at(),
                NotableFinancialItem::Business {
                    cycle: cycle.id(),
                    business: business.id(),
                    information,
                },
            ));
        }
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
    for enterprise in state.enterprises().enterprises_for_organization(recipient) {
        for cycle in state
            .enterprises()
            .cycles_for(enterprise.id())
            .filter(|cycle| {
                cycle.occurred_at() >= period_start
                    && cycle.occurred_at() <= period_end
                    && cycle.attention() == AttentionClass::Notable
            })
        {
            let information = cycle
                .information()
                .ok_or(OrganizationFinancialReportError::MissingNotableInformation)?;
            items.push((
                cycle.occurred_at(),
                NotableFinancialItem::Enterprise {
                    cycle: cycle.id(),
                    enterprise: enterprise.id(),
                    information,
                },
            ));
        }
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
