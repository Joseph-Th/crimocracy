//! Organization-level financial report synthesis across legitimate businesses and illicit enterprises.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessCycleId, BusinessId, EnterpriseCycleId, EnterpriseId, InformationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::economy::business_reporting::{
    resolve_organization_business_financial_summary, BusinessReportingError,
};
use crate::enterprises::enterprise_reporting::{
    resolve_organization_enterprise_financial_summary, EnterpriseReportingError,
};
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
    let mut entries = vec![ReportEntry {
        attention: AttentionClass::Routine,
        summary: format!(
            "Legitimate businesses: {} businesses, {} cycles, gross {} cents, operating cost {} cents, net {} cents. Illicit enterprises: {} enterprises, {} cycles, gross {} cents, operating cost {} cents, net {} cents.",
            business_summary.totals.business_count,
            business_summary.totals.cycle_count,
            business_summary.totals.gross_revenue.cents(),
            business_summary.totals.operating_cost.cents(),
            business_summary.totals.net_cash.cents(),
            enterprise_summary.totals.enterprise_count,
            enterprise_summary.totals.cycle_count,
            enterprise_summary.totals.gross_revenue.cents(),
            enterprise_summary.totals.operating_cost.cents(),
            enterprise_summary.totals.net_cash.cents(),
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

fn collect_notable_business_cycles(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<Vec<(SimTime, NotableFinancialItem)>, OrganizationFinancialReportError> {
    let mut items = Vec::new();
    for business in state.world.businesses().filter(|business| {
        business.owner() == BusinessOwner::Organization(recipient)
            && state
                .economy()
                .get_business_economy(business.id())
                .is_some()
    }) {
        for cycle in state.economy().cycles_for(business.id()).filter(|cycle| {
            cycle.occurred_at() >= period_start
                && cycle.occurred_at() <= period_end
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
