//! Document-theft integration that converts stolen internal records into durable organization
//! knowledge. Documents are information assets, not generic resale inventory.

use crate::core::entity::EntityRef;
use crate::core::id::{BusinessId, OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::{Money, helpers::format_money_cents};
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_system_information,
};
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSignal, InformationSourceKind,
    InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationRecord,
};
use crate::world::BusinessFunction;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum DocumentTheftError {
    #[error("document theft requires a gather-information objective against a business")]
    InvalidObjective,
    #[error("document-theft target business {0} no longer exists")]
    MissingTarget(BusinessId),
    #[error("document-theft target business {0} changed after resolution planning")]
    StaleTarget(BusinessId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DocumentTheftIntelligencePlan {
    target: BusinessId,
    observed_at: SimTime,
    snapshot: DocumentTargetSnapshot,
    observations: Vec<DocumentObservation>,
}

impl DocumentTheftIntelligencePlan {
    pub(crate) fn observation_count(&self) -> usize {
        self.observations.len()
    }

    pub(crate) fn discovery_signatures(
        &self,
    ) -> BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)> {
        self.observations
            .iter()
            .map(|observation| (observation.topic, EntityRef::Business(self.target), None))
            .collect()
    }

    pub(crate) fn observation_findings(&self) -> impl Iterator<Item = &str> {
        self.observations
            .iter()
            .map(|observation| observation.finding.as_str())
    }
}

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentTargetSnapshot {
    business_version: u32,
    name: String,
    functions: BTreeSet<BusinessFunction>,
    latest_financials: Option<DocumentFinancialSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DocumentFinancialSnapshot {
    occurred_at: SimTime,
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentObservation {
    topic: InformationTopic,
    reliability: Reliability,
    specificity: Specificity,
    summary: String,
    finding: String,
}

pub(crate) fn decide_document_theft_intelligence(
    state: &AppState,
    operation: &OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<Option<DocumentTheftIntelligencePlan>, DocumentTheftError> {
    if operation.kind() != OperationKind::DocumentTheft {
        return Ok(None);
    }
    let OperationObjective::GatherInformation {
        target: EntityRef::Business(target),
    } = operation.objective()
    else {
        return Err(DocumentTheftError::InvalidObjective);
    };
    let snapshot = resolve_target_snapshot(state, *target)?;
    let observations = build_observations(&snapshot, outcome);
    Ok(Some(DocumentTheftIntelligencePlan {
        target: *target,
        observed_at: state.now(),
        snapshot,
        observations,
    }))
}

pub(crate) fn validate_document_theft_plan_snapshot(
    state: &AppState,
    plan: &DocumentTheftIntelligencePlan,
) -> Result<(), DocumentTheftError> {
    crate::core::time::ensure_time_current(state.now(), plan.observed_at)
        .map_err(|_| DocumentTheftError::StaleTarget(plan.target))?;
    if resolve_target_snapshot(state, plan.target)? != plan.snapshot {
        return Err(DocumentTheftError::StaleTarget(plan.target));
    }
    Ok(())
}

pub(crate) fn validate_document_theft_information(
    state: &AppState,
    organization: OrganizationId,
    source_operation: OperationId,
    plan: &DocumentTheftIntelligencePlan,
) -> Result<Vec<ValidatedInformation>, crate::intelligence::intelligence_system::IntelligenceError>
{
    plan.observations
        .iter()
        .map(|observation| {
            validate_record_system_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Organization(organization),
                    source_kind: InformationSourceKind::AcquiredRecords,
                    topic: observation.topic,
                    source_entity: Some(EntityRef::Operation(source_operation)),
                    subject: EntityRef::Business(plan.target),
                    observed_at: plan.observed_at,
                    reliability: observation.reliability,
                    specificity: observation.specificity,
                    summary: observation.summary.clone(),
                },
            )
        })
        .collect()
}

pub(crate) fn is_valid_persisted_document_theft_information(
    operation: &OperationRecord,
    information: &InformationRecord,
) -> bool {
    let Some(resolution) = operation.resolution() else {
        return false;
    };
    let Some((expected_reliability, expected_specificity)) =
        observation_quality(resolution.objective_outcome())
    else {
        return false;
    };
    let OperationObjective::GatherInformation {
        target: EntityRef::Business(target),
    } = operation.objective()
    else {
        return false;
    };
    let topic_is_valid = match information.topic() {
        InformationTopic::MarketAccess => true,
        InformationTopic::FinancialPerformance => {
            resolution.objective_outcome() == OperationObjectiveOutcome::Achieved
        }
        InformationTopic::TargetSecurity
        | InformationTopic::Personnel
        | InformationTopic::Schedule
        | InformationTopic::PoliceActivity
        | InformationTopic::Route
        | InformationTopic::LegalActivity
        | InformationTopic::OperationalOutcome
        | InformationTopic::EnterpriseActivity => false,
    };
    operation.kind() == OperationKind::DocumentTheft
        && topic_is_valid
        && information.holder()
            == KnowledgeHolder::Organization(operation.responsible_organization())
        && information.source_kind() == InformationSourceKind::AcquiredRecords
        && information.source_entity() == Some(EntityRef::Operation(operation.id()))
        && information.subject() == EntityRef::Business(*target)
        && information.observed_at() == resolution.resolved_at()
        && information.recorded_at() == resolution.resolved_at()
        && information.reliability() == expected_reliability
        && information.specificity() == expected_specificity
        && information.signal().is_none()
        && information.derived_from().is_empty()
        && resolution.discovery_signatures().contains(&(
            information.topic(),
            information.subject(),
            None,
        ))
}

pub(crate) fn document_theft_after_action_clause(
    plan: Option<&DocumentTheftIntelligencePlan>,
    outcome: OperationObjectiveOutcome,
) -> Option<String> {
    let plan = plan?;
    Some(render_document_theft_clause(
        plan.observation_count(),
        &plan.observation_findings().collect::<Vec<_>>().join("; "),
        outcome,
    ))
}

pub(crate) fn persisted_document_theft_after_action_clause(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<Option<String>, ()> {
    if operation.kind() != OperationKind::DocumentTheft {
        return Ok(None);
    }
    let resolution = operation.resolution().ok_or(())?;
    if resolution.objective_outcome() == OperationObjectiveOutcome::Failed {
        return Ok(Some(render_document_theft_clause(
            0,
            "",
            resolution.objective_outcome(),
        )));
    }
    let findings = resolution
        .discovered_information()
        .iter()
        .map(|information| {
            let record = state.intelligence.get_information(*information).ok_or(())?;
            match (record.topic(), record.subject()) {
                (InformationTopic::MarketAccess, EntityRef::Business(business)) => state
                    .world
                    .get_business(business)
                    .map(|record| format!("internal access records from {}", record.name()))
                    .ok_or(()),
                (InformationTopic::FinancialPerformance, EntityRef::Business(business)) => state
                    .world
                    .get_business(business)
                    .map(|record| format!("recent financial records from {}", record.name()))
                    .ok_or(()),
                _ => Err(()),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(render_document_theft_clause(
        findings.len(),
        &findings.join("; "),
        resolution.objective_outcome(),
    )))
}

fn resolve_target_snapshot(
    state: &AppState,
    target: BusinessId,
) -> Result<DocumentTargetSnapshot, DocumentTheftError> {
    let business = state
        .world
        .get_business(target)
        .ok_or(DocumentTheftError::MissingTarget(target))?;
    let latest_financials =
        state
            .economy
            .latest_cycle(target)
            .map(|cycle| DocumentFinancialSnapshot {
                occurred_at: cycle.occurred_at(),
                gross_revenue: cycle.gross_revenue(),
                operating_cost: cycle.operating_cost(),
                net_cash: cycle.net_cash(),
            });
    Ok(DocumentTargetSnapshot {
        business_version: business.version(),
        name: business.name().to_owned(),
        functions: business.functions().clone(),
        latest_financials,
    })
}

fn build_observations(
    snapshot: &DocumentTargetSnapshot,
    outcome: OperationObjectiveOutcome,
) -> Vec<DocumentObservation> {
    let Some((reliability, specificity)) = observation_quality(outcome) else {
        return Vec::new();
    };
    let access = snapshot
        .functions
        .iter()
        .map(|function| function.description())
        .collect::<Vec<_>>();
    let access_summary = if access.is_empty() {
        format!(
            "Acquired internal records from {} show ordinary operating access with no specialized business capability recorded.",
            snapshot.name
        )
    } else {
        format!(
            "Acquired internal records from {} document operating access associated with {}.",
            snapshot.name,
            access.join(", ")
        )
    };
    let mut observations = vec![DocumentObservation {
        topic: InformationTopic::MarketAccess,
        reliability,
        specificity,
        summary: access_summary,
        finding: format!("internal access records from {}", snapshot.name),
    }];
    if outcome == OperationObjectiveOutcome::Achieved
        && let Some(financials) = snapshot.latest_financials
    {
        observations.push(DocumentObservation {
            topic: InformationTopic::FinancialPerformance,
            reliability,
            specificity,
            summary: format!(
                "Acquired books from {} show a latest settled cycle with {} gross revenue, {} operating cost, and {} net cash.",
                snapshot.name,
                format_money_cents(financials.gross_revenue.cents()),
                format_money_cents(financials.operating_cost.cents()),
                format_money_cents(financials.net_cash.cents()),
            ),
            finding: format!("recent financial records from {}", snapshot.name),
        });
    }
    observations
}

fn observation_quality(outcome: OperationObjectiveOutcome) -> Option<(Reliability, Specificity)> {
    match outcome {
        OperationObjectiveOutcome::Achieved => {
            Some((Reliability::DirectAccess, Specificity::Precise))
        }
        OperationObjectiveOutcome::Partial => {
            Some((Reliability::GenerallyReliable, Specificity::Specific))
        }
        OperationObjectiveOutcome::Failed => None,
    }
}

fn render_document_theft_clause(
    observation_count: usize,
    findings: &str,
    outcome: OperationObjectiveOutcome,
) -> String {
    match outcome {
        OperationObjectiveOutcome::Achieved => format!(
            "Document theft recovered {} usable record set{}{}.",
            observation_count,
            if observation_count == 1 { "" } else { "s" },
            if findings.is_empty() {
                String::new()
            } else {
                format!(": {findings}")
            }
        ),
        OperationObjectiveOutcome::Partial => format!(
            "Document theft recovered {} incomplete but usable record set{}; important details remain unresolved.{}",
            observation_count,
            if observation_count == 1 { "" } else { "s" },
            if findings.is_empty() {
                String::new()
            } else {
                format!(" Covered: {findings}.")
            }
        ),
        OperationObjectiveOutcome::Failed => {
            "Document theft recovered no records reliable enough for planning.".to_owned()
        }
    }
}
