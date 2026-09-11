//! Release-safe structural validation for the opportunities subsystem.

use crate::core::attention::AttentionClass;
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::legal::{EvidenceReliability, EvidenceStrength};
use crate::operations::OperationExposureLevel;
use crate::operations::operation_scheduling::resolve_operation_earliest_start;
use crate::operations::operation_system::is_valid_operation_objective;
use crate::opportunities::opportunity_system::{
    source_information_is_usable, source_information_proves_operation_basis,
};
use crate::opportunities::{OpportunityRecord, OpportunityResolution};
use crate::registry::Registry;
use crate::reports::{ReportKind, ReportRecord};
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

pub(super) fn validate_opportunities(state: &AppState) -> Result<(), StateValidationError> {
    let mut covered_targets = BTreeSet::<EntityRef>::new();
    for opportunity in state.opportunities.opportunities() {
        validate_opportunity(state, opportunity, &mut covered_targets)?;
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
                            && source_information_is_usable(
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
        Some(OpportunityResolution::Dismissed { at }) => {
            validate_dismissed_opportunity(state, opportunity, at)
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
) -> Result<(), StateValidationError> {
    if opportunity.version() != 2
        || at < opportunity.discovered_at()
        || at > state.now()
        || opportunity
            .valid_until()
            .is_some_and(|valid_until| at >= valid_until)
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

pub(super) fn validate_operation_exposure_links(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    validate_exposure_location(state, operation, exposure)?;
    validate_exposure_identity(operation, exposure)?;
    match exposure.investigation() {
        None => {
            if !exposure.evidence().is_empty() {
                return Err(invalid_operation_exposure(operation));
            }
            Ok(())
        }
        Some(investigation_id) => {
            validate_exposure_investigation(state, operation, resolution, investigation_id)
        }
    }
}

fn validate_exposure_location(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    exposure: &crate::operations::OperationExposureRecord,
) -> Result<(), StateValidationError> {
    if let Some(neighborhood) = exposure.neighborhood()
        && state.world.get_neighborhood(neighborhood).is_none()
    {
        return Err(invalid_operation_exposure(operation));
    }
    Ok(())
}

fn validate_exposure_identity(
    operation: &crate::operations::OperationRecord,
    exposure: &crate::operations::OperationExposureRecord,
) -> Result<(), StateValidationError> {
    let participants: BTreeSet<_> = std::iter::once(operation.leader())
        .chain(operation.roles().values().copied())
        .collect();
    match exposure.level() {
        OperationExposureLevel::Identifying => {
            if !exposure
                .identified_character()
                .is_some_and(|character| participants.contains(&character))
            {
                return Err(invalid_operation_exposure(operation));
            }
        }
        OperationExposureLevel::None
        | OperationExposureLevel::Trace
        | OperationExposureLevel::Witnessed => {
            if exposure.identified_character().is_some() {
                return Err(invalid_operation_exposure(operation));
            }
        }
    }
    Ok(())
}

fn validate_exposure_investigation(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    investigation_id: crate::core::id::InvestigationId,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    if exposure.level() == OperationExposureLevel::None
        || exposure.neighborhood().is_none()
        || exposure.evidence().len() != 1
    {
        return Err(invalid_operation_exposure(operation));
    }
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or_else(|| invalid_operation_exposure(operation))?;
    let owner = state
        .world
        .get_organization(investigation.owner())
        .ok_or_else(|| invalid_operation_exposure(operation))?;
    if !matches!(
        owner.kind(),
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    ) || investigation.opened_at() != resolution.resolved_at()
        || !investigation
            .subjects()
            .contains(&EntityRef::Operation(operation.id()))
    {
        return Err(invalid_operation_exposure(operation));
    }
    if let Some(character) = exposure.identified_character()
        && !investigation
            .subjects()
            .contains(&EntityRef::Character(character))
    {
        return Err(invalid_operation_exposure(operation));
    }
    validate_exposure_evidence(state, operation, resolution, investigation)
}

fn validate_exposure_evidence(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    investigation: &crate::legal::InvestigationRecord,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    let evidence_id = *exposure
        .evidence()
        .iter()
        .next()
        .expect("validated operation exposure contains one evidence record");
    let evidence = state
        .legal
        .get_evidence(evidence_id)
        .ok_or_else(|| invalid_operation_exposure(operation))?;
    let expected_subject = exposure
        .identified_character()
        .map(EntityRef::Character)
        .unwrap_or(EntityRef::Operation(operation.id()));
    let (expected_strength, expected_reliability) = exposure_evidence_quality(exposure.level());
    if evidence.investigation() != investigation.id()
        || evidence.custodian() != investigation.owner()
        || evidence.subject() != expected_subject
        || evidence.origin() != Some(EntityRef::Operation(operation.id()))
        || evidence.strength() != expected_strength
        || evidence.reliability() != expected_reliability
        || evidence.discovered_at() != resolution.resolved_at()
    {
        return Err(invalid_operation_exposure(operation));
    }
    Ok(())
}

fn exposure_evidence_quality(
    level: OperationExposureLevel,
) -> (EvidenceStrength, EvidenceReliability) {
    match level {
        OperationExposureLevel::None => unreachable!("non-exposure cannot have legal evidence"),
        OperationExposureLevel::Trace => {
            (EvidenceStrength::Weak, EvidenceReliability::Questionable)
        }
        OperationExposureLevel::Witnessed => (
            EvidenceStrength::Corroborating,
            EvidenceReliability::Credible,
        ),
        OperationExposureLevel::Identifying => (
            EvidenceStrength::Strong,
            EvidenceReliability::HighlyReliable,
        ),
    }
}

fn invalid_operation_exposure(
    operation: &crate::operations::OperationRecord,
) -> StateValidationError {
    StateValidationError::InvalidOperationExposure {
        operation: operation.id(),
    }
}
