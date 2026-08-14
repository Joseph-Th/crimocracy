//! Organization financial-report synthesis over enterprise history and known notable after-action information.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{EnterpriseCycleId, EnterpriseId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::enterprises::enterprise_reporting::{
    resolve_organization_enterprise_financial_summary, EnterpriseReportingError,
};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EnterpriseFinancialReportError {
    #[error("notable enterprise cycle {0} is missing after-action information")]
    MissingNotableInformation(EnterpriseCycleId),
    #[error("notable enterprise cycle {cycle} references missing information {information}")]
    MissingNotableInformationRecord {
        cycle: EnterpriseCycleId,
        information: crate::core::id::InformationId,
    },
    #[error(transparent)]
    Enterprise(#[from] EnterpriseReportingError),
    #[error(transparent)]
    Report(#[from] ReportError),
}

pub fn validate_enterprise_financial_report(
    state: &AppState,
    recipient: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<ValidatedReport, EnterpriseFinancialReportError> {
    let summary = resolve_organization_enterprise_financial_summary(
        state,
        recipient,
        period_start,
        period_end,
    )?;
    let mut entries = vec![ReportEntry {
        attention: AttentionClass::Routine,
        summary: format!(
            "Enterprise performance: {} enterprises completed {} cycles, including {} notable variances; gross revenue {} cents, operating cost {} cents, net cash {} cents.",
            summary.totals.enterprise_count,
            summary.totals.cycle_count,
            summary.totals.notable_cycle_count,
            summary.totals.gross_revenue.cents(),
            summary.totals.operating_cost.cents(),
            summary.totals.net_cash.cents(),
        ),
        sources: Vec::new(),
        entities: BTreeSet::new(),
        decision: None,
    }];

    let mut notable_cycles: Vec<_> = state
        .enterprises()
        .enterprises_for_organization(recipient)
        .flat_map(|enterprise| {
            state
                .enterprises()
                .cycles_for(enterprise.id())
                .filter(|cycle| {
                    cycle.occurred_at() >= period_start
                        && cycle.occurred_at() <= period_end
                        && cycle.attention() == AttentionClass::Notable
                })
                .map(move |cycle| {
                    (
                        cycle.occurred_at(),
                        cycle.id(),
                        enterprise.id(),
                        cycle.information(),
                    )
                })
        })
        .collect();
    notable_cycles
        .sort_by_key(|(occurred_at, cycle, enterprise, _)| (*occurred_at, *cycle, *enterprise));

    for (_, cycle, enterprise, information) in notable_cycles {
        entries.push(build_notable_entry(state, cycle, enterprise, information)?);
    }

    Ok(validate_record_report(
        state,
        ReportDraft {
            recipient,
            kind: ReportKind::Financial,
            title: "Enterprise financial report".to_owned(),
            entries,
        },
    )?)
}

fn build_notable_entry(
    state: &AppState,
    cycle: EnterpriseCycleId,
    enterprise: EnterpriseId,
    information: Option<crate::core::id::InformationId>,
) -> Result<ReportEntry, EnterpriseFinancialReportError> {
    let information_id = information.ok_or(
        EnterpriseFinancialReportError::MissingNotableInformation(cycle),
    )?;
    let information_record = state.intelligence().get_information(information_id).ok_or(
        EnterpriseFinancialReportError::MissingNotableInformationRecord {
            cycle,
            information: information_id,
        },
    )?;
    let mut entities = BTreeSet::new();
    entities.insert(EntityRef::Enterprise(enterprise));
    Ok(ReportEntry {
        attention: AttentionClass::Notable,
        summary: format!("Cycle {cycle}: {}", information_record.summary()),
        sources: vec![information_id],
        entities,
        decision: None,
    })
}
