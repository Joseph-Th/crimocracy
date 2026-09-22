//! Historical surveillance after-action reconstruction.
//!
//! The live integration owner freezes observations and persists intelligence. This child owns only
//! the player-facing reconstruction of that already-persisted history, so later world changes
//! cannot rewrite what the operation originally learned.

use super::SurveillanceIntelligencePlan;
use super::observation_text::enterprise_kind_label;
use crate::core::entity::EntityRef;
use crate::core::state::AppState;
use crate::enterprises::EnterpriseLocation;
use crate::intelligence::{InformationRecord, InformationTopic};
use crate::operations::{OperationKind, OperationObjectiveOutcome, OperationRecord};

pub(crate) fn surveillance_after_action_clause(
    plan: Option<&SurveillanceIntelligencePlan>,
    outcome: OperationObjectiveOutcome,
) -> Option<String> {
    let plan = plan?;
    let findings = plan.observation_findings().collect::<Vec<_>>().join("; ");
    Some(render_surveillance_after_action_clause(
        plan.observation_count(),
        &findings,
        outcome,
    ))
}

fn render_surveillance_after_action_clause(
    observation_count: usize,
    findings: &str,
    outcome: OperationObjectiveOutcome,
) -> String {
    match outcome {
        OperationObjectiveOutcome::Achieved => format!(
            "Surveillance produced {} usable target observation{}{}.",
            observation_count,
            if observation_count == 1 { "" } else { "s" },
            if findings.is_empty() {
                String::new()
            } else {
                format!(": {findings}")
            }
        ),
        OperationObjectiveOutcome::Partial => format!(
            "Surveillance produced {} limited target observation{}; important details remain unresolved.{}",
            observation_count,
            if observation_count == 1 { "" } else { "s" },
            if findings.is_empty() {
                String::new()
            } else {
                format!(" Covered: {findings}.")
            }
        ),
        OperationObjectiveOutcome::Failed => {
            "Surveillance produced no target observation reliable enough for planning.".to_owned()
        }
    }
}

/// Rebuilds the after-action surveillance clause from durable observation records rather than
/// current target state. `Err(())` means one persisted observation no longer maps to the
/// canonical surveillance vocabulary and therefore cannot support a valid historical narrative.
pub(crate) fn persisted_surveillance_after_action_clause(
    state: &AppState,
    operation: &OperationRecord,
) -> Result<Option<String>, ()> {
    if operation.kind() != OperationKind::Surveillance {
        return Ok(None);
    }
    let resolution = operation.resolution().ok_or(())?;
    if resolution.objective_outcome() == OperationObjectiveOutcome::Failed {
        return Ok(Some(render_surveillance_after_action_clause(
            0,
            "",
            resolution.objective_outcome(),
        )));
    }
    let findings = resolution
        .discovered_information()
        .iter()
        .map(|information| {
            state
                .intelligence
                .get_information(*information)
                .and_then(|record| persisted_surveillance_finding(state, record))
                .ok_or(())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(render_surveillance_after_action_clause(
        findings.len(),
        &findings.join("; "),
        resolution.objective_outcome(),
    )))
}

fn persisted_surveillance_finding(
    state: &AppState,
    information: &InformationRecord,
) -> Option<String> {
    match (information.topic(), information.subject()) {
        (InformationTopic::PoliceActivity, EntityRef::Neighborhood(id)) => state
            .world
            .get_neighborhood(id)
            .map(|record| format!("police activity around {}", record.name())),
        (InformationTopic::MarketAccess, EntityRef::Business(id)) => state
            .world
            .get_business(id)
            .map(|record| format!("access intelligence at {}", record.name())),
        (InformationTopic::Personnel, EntityRef::Character(id)) => state
            .world
            .get_character(id)
            .map(|record| format!("the movements of {}", record.name())),
        (InformationTopic::LegalActivity, EntityRef::Organization(id)) => state
            .world
            .get_organization(id)
            .map(|record| format!("case activity at {}", record.name())),
        (InformationTopic::Personnel, EntityRef::Organization(id)) => state
            .world
            .get_organization(id)
            .map(|record| format!("personnel around {}", record.name())),
        (InformationTopic::LegalActivity, EntityRef::Investigation(id)) => state
            .legal
            .get_investigation(id)
            .map(|record| format!("the status of {}", record.title())),
        (InformationTopic::EnterpriseActivity, EntityRef::Enterprise(id)) => {
            let enterprise = state.enterprises.get_enterprise(id)?;
            let location = match enterprise.location() {
                EnterpriseLocation::Neighborhood(neighborhood) => {
                    state.world.get_neighborhood(neighborhood)?.name()
                }
                EnterpriseLocation::Business(business) => {
                    state.world.get_business(business)?.name()
                }
            };
            Some(format!(
                "{} activity at {location}",
                enterprise_kind_label(enterprise.kind())
            ))
        }
        (InformationTopic::OperationalOutcome, EntityRef::Operation(id)) => {
            let observed = state.operations.get_operation(id)?;
            let organization = state
                .world
                .get_organization(observed.responsible_organization())?;
            Some(format!("activity linked to {}", organization.name()))
        }
        _ => None,
    }
}
