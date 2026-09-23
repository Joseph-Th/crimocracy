//! Release-safe structural validation for the opportunities subsystem.

use crate::core::attention::AttentionClass;
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::operations::operation_basis_knowledge::{
    source_information_is_usable_for_operation_basis, source_information_proves_operation_basis,
};
use crate::operations::operation_scheduling::resolve_operation_earliest_start;
use crate::operations::operation_system::is_valid_operation_objective;
use crate::opportunities::opportunity_system::operation_matches_opportunity_basis;
use crate::opportunities::{OpportunityRecord, OpportunityResolution};
use crate::registry::Registry;
use crate::reports::{ReportKind, ReportRecord};
use crate::world::OrganizationKind;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_opportunities(state: &AppState) -> Result<(), StateValidationError> {
    let mut covered_targets = BTreeSet::<EntityRef>::new();
    let mut open_targets = BTreeMap::<
        (
            crate::core::id::OrganizationId,
            crate::operations::OperationKind,
        ),
        BTreeSet<EntityRef>,
    >::new();
    for opportunity in state.opportunities.opportunities() {
        validate_opportunity(state, opportunity, &mut covered_targets)?;
        if opportunity.resolution().is_none() {
            let context = opportunity.context();
            let targets = open_targets
                .entry((opportunity.organization(), context.operation_kind()))
                .or_default();
            if !targets.is_disjoint(context.targets()) {
                return Err(invalid_opportunity(opportunity));
            }
            targets.extend(context.targets().iter().copied());
        }
    }
    Ok(())
}

pub(super) fn validate_opportunities_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for opportunity in state.opportunities.opportunities() {
        let context = opportunity.context();
        let kind = context.operation_kind();
        let definition = registry.get_operation(kind);
        let has_authored_target = context.targets().iter().any(|target| {
            let Some(objective) = kind.objective_for_target(*target) else {
                return false;
            };
            if !is_valid_operation_objective(kind, &objective) {
                return false;
            }
            let Some(ownership) = kind.business_target_ownership() else {
                return true;
            };
            let EntityRef::Business(business) = target else {
                return false;
            };
            let Some(record) = state.world.get_business(*business) else {
                return false;
            };
            let Some(requirement) = definition.execution().business_target() else {
                return false;
            };
            if !requirement
                .required_functions()
                .iter()
                .all(|function| record.has_function(*function))
            {
                return false;
            }
            let (could_be_sponsor_owned, definitely_sponsor_owned) =
                state.world.business_owner_evidence_at(
                    *business,
                    crate::world::BusinessOwner::Organization(opportunity.organization()),
                    opportunity.discovered_at(),
                );
            match ownership {
                crate::operations::OperationBusinessTargetOwnership::Foreign => {
                    !definitely_sponsor_owned
                }
                crate::operations::OperationBusinessTargetOwnership::SponsorOwned => {
                    could_be_sponsor_owned
                }
            }
        });
        let every_target_has_usable_source = context.targets().iter().all(|target| {
            opportunity.source_information().iter().any(|source| {
                state
                    .intelligence
                    .get_information(*source)
                    .is_some_and(|information| {
                        information.subject() == *target
                            && source_information_is_usable_for_operation_basis(
                                registry,
                                kind,
                                information,
                                opportunity.discovered_at(),
                            )
                    })
            })
        });
        let has_supported_action_basis = context.targets().iter().any(|target| {
            opportunity.source_information().iter().any(|source| {
                state
                    .intelligence
                    .get_information(*source)
                    .is_some_and(|information| {
                        information.subject() == *target
                            && source_information_proves_operation_basis(
                                registry,
                                state,
                                kind,
                                *target,
                                information,
                                opportunity.discovered_at(),
                            )
                    })
            })
        });
        let report = state
            .reports
            .get_report(opportunity.report())
            .ok_or_else(|| invalid_opportunity(opportunity))?;
        if !has_authored_target
            || !every_target_has_usable_source
            || !has_supported_action_basis
            || report.title()
                != crate::opportunities::opportunity_system::discovery_report_title(
                    definition.display_name(),
                )
        {
            return Err(invalid_opportunity(opportunity));
        }
        if let Some(OpportunityResolution::Expired {
            report: expiry_report,
            ..
        }) = opportunity.resolution()
        {
            let report = state
                .reports
                .get_report(expiry_report)
                .ok_or_else(|| invalid_opportunity(opportunity))?;
            if report.title()
                != crate::opportunities::opportunity_system::expiry_report_title(
                    definition.display_name(),
                )
            {
                return Err(invalid_opportunity(opportunity));
            }
        }
    }
    Ok(())
}

fn validate_opportunity(
    state: &AppState,
    opportunity: &OpportunityRecord,
    covered_targets: &mut BTreeSet<EntityRef>,
) -> Result<(), StateValidationError> {
    validate_opportunity_definition(state, opportunity)?;
    validate_opportunity_targets(state, opportunity)?;
    validate_opportunity_sources(state, opportunity, covered_targets)?;
    validate_opportunity_report(state, opportunity)?;
    validate_opportunity_resolution(state, opportunity, covered_targets)
}

fn validate_opportunity_definition(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), StateValidationError> {
    let organization = state
        .world
        .get_organization(opportunity.organization())
        .ok_or_else(|| invalid_opportunity(opportunity))?;
    let context = opportunity.context();
    if organization.kind() != OrganizationKind::Criminal
        || context.targets().is_empty()
        || opportunity.source_information().is_empty()
        || opportunity.summary().trim().is_empty()
        || opportunity.discovered_at() > state.now()
        || opportunity.version() == 0
        || opportunity
            .valid_until()
            .is_some_and(|valid_until| valid_until <= opportunity.discovered_at())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_opportunity_targets(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), StateValidationError> {
    let kind = opportunity.context().operation_kind();
    let mut has_compatible_target = false;
    for target in opportunity.context().targets() {
        if !is_entity_present(state, *target) {
            return Err(invalid_opportunity(opportunity));
        }
        has_compatible_target |= kind
            .objective_for_target(*target)
            .is_some_and(|objective| is_valid_operation_objective(kind, &objective));
    }
    if !has_compatible_target {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_opportunity_sources(
    state: &AppState,
    opportunity: &OpportunityRecord,
    covered_targets: &mut BTreeSet<EntityRef>,
) -> Result<(), StateValidationError> {
    let context = opportunity.context();
    covered_targets.clear();
    for source in opportunity.source_information() {
        let information = state
            .intelligence
            .get_information(*source)
            .ok_or_else(|| invalid_opportunity(opportunity))?;
        if information.holder() != KnowledgeHolder::Organization(opportunity.organization())
            || information.recorded_at() > opportunity.discovered_at()
            || !context.targets().contains(&information.subject())
        {
            return Err(invalid_opportunity(opportunity));
        }
        covered_targets.insert(information.subject());
    }
    if covered_targets != context.targets() {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_opportunity_report(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), StateValidationError> {
    let report = state
        .reports
        .get_report(opportunity.report())
        .ok_or_else(|| invalid_opportunity(opportunity))?;
    if report.recipient() != opportunity.organization()
        || report.kind() != ReportKind::Opportunity
        || report.generated_at() != opportunity.discovered_at()
        || !opportunity_report_entry_matches(opportunity, report, opportunity.summary())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn opportunity_report_entry_matches(
    opportunity: &OpportunityRecord,
    report: &ReportRecord,
    expected_summary: &str,
) -> bool {
    let context = opportunity.context();
    let expected_sources = opportunity.source_information();
    report.entries().len() == 1
        && report.entries().first().is_some_and(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == expected_summary
                && entry.sources.len() == expected_sources.len()
                && entry
                    .sources
                    .iter()
                    .zip(expected_sources.iter())
                    .all(|(source, expected)| source == expected)
                && entry.entities.len() == context.targets().len() + 1
                && entry
                    .entities
                    .contains(&EntityRef::Organization(opportunity.organization()))
                && context
                    .targets()
                    .iter()
                    .all(|target| entry.entities.contains(target))
                && entry.decision.is_none()
        })
}

fn validate_opportunity_resolution(
    state: &AppState,
    opportunity: &OpportunityRecord,
    operation_targets: &mut BTreeSet<EntityRef>,
) -> Result<(), StateValidationError> {
    match opportunity.resolution() {
        None => validate_open_opportunity(state, opportunity),
        Some(OpportunityResolution::Dismissed { at, report }) => {
            validate_dismissed_opportunity(state, opportunity, at, report)
        }
        Some(OpportunityResolution::Expired { at, report }) => {
            validate_expired_opportunity(state, opportunity, at, report)
        }
        Some(OpportunityResolution::Converted { at, operation }) => {
            validate_converted_opportunity(state, opportunity, at, operation, operation_targets)
        }
    }
}

fn validate_open_opportunity(
    state: &AppState,
    opportunity: &OpportunityRecord,
) -> Result<(), StateValidationError> {
    if opportunity.version() != 1
        || opportunity
            .valid_until()
            .is_some_and(|valid_until| valid_until <= state.now())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_dismissed_opportunity(
    state: &AppState,
    opportunity: &OpportunityRecord,
    at: crate::core::time::SimTime,
    report: crate::core::id::ReportId,
) -> Result<(), StateValidationError> {
    let dismissal_report = state
        .reports
        .get_report(report)
        .ok_or_else(|| invalid_opportunity(opportunity))?;
    let expected_summary =
        crate::opportunities::opportunity_system::dismissal_report_summary(opportunity.summary());
    if opportunity.version() != 2
        || at < opportunity.discovered_at()
        || at > state.now()
        || opportunity
            .valid_until()
            .is_some_and(|valid_until| at >= valid_until)
        || dismissal_report.recipient() != opportunity.organization()
        || dismissal_report.kind() != ReportKind::Opportunity
        || dismissal_report.generated_at() != at
        || !opportunity_report_entry_matches(opportunity, dismissal_report, &expected_summary)
        || state
            .opportunities
            .opportunity_for_report(report)
            .map(|record| record.id())
            != Some(opportunity.id())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_expired_opportunity(
    state: &AppState,
    opportunity: &OpportunityRecord,
    at: crate::core::time::SimTime,
    report: crate::core::id::ReportId,
) -> Result<(), StateValidationError> {
    let expiry_report = state
        .reports
        .get_report(report)
        .ok_or_else(|| invalid_opportunity(opportunity))?;
    let expected_summary =
        crate::opportunities::opportunity_system::expiry_report_summary(opportunity.summary());
    if opportunity.version() != 2
        || opportunity.valid_until() != Some(at)
        || at > state.now()
        || expiry_report.recipient() != opportunity.organization()
        || expiry_report.kind() != ReportKind::Opportunity
        || expiry_report.generated_at() < at
        || expiry_report.generated_at() > state.now()
        || !opportunity_report_entry_matches(opportunity, expiry_report, &expected_summary)
        || state
            .opportunities
            .opportunity_for_report(report)
            .map(|record| record.id())
            != Some(opportunity.id())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn validate_converted_opportunity(
    state: &AppState,
    opportunity: &OpportunityRecord,
    at: crate::core::time::SimTime,
    operation_id: crate::core::id::OperationId,
    operation_targets: &mut BTreeSet<EntityRef>,
) -> Result<(), StateValidationError> {
    let operation = state
        .operations
        .get_operation(operation_id)
        .ok_or_else(|| invalid_opportunity(opportunity))?;
    operation_targets.clear();
    operation_targets.extend(operation.objective().referenced_entities());
    let context = opportunity.context();
    if opportunity.version() != 2
        || at < opportunity.discovered_at()
        || at > state.now()
        || at < operation.authorized_at()
        || at > operation.scheduled_for()
        || opportunity
            .valid_until()
            .is_some_and(|valid_until| at >= valid_until)
        || opportunity.valid_until().is_some_and(|valid_until| {
            resolve_operation_earliest_start(operation) >= valid_until
                || operation
                    .started_at()
                    .is_some_and(|started_at| started_at >= valid_until)
        })
        || operation.responsible_organization() != opportunity.organization()
        || operation.kind() != context.operation_kind()
        || operation_targets.len() != 1
        || !operation_targets
            .iter()
            .all(|target| context.targets().contains(target))
        || !operation_matches_opportunity_basis(state, opportunity, operation)
        || state
            .opportunities
            .opportunity_for_operation(operation.id())
            .map(|record| record.id())
            != Some(opportunity.id())
    {
        return Err(invalid_opportunity(opportunity));
    }
    Ok(())
}

fn invalid_opportunity(opportunity: &OpportunityRecord) -> StateValidationError {
    StateValidationError::InvalidOpportunity {
        opportunity: opportunity.id(),
    }
}
