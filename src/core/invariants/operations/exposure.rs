//! Persisted operation-exposure links to legal incident state.

use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::legal::{EvidenceReliability, EvidenceStrength};
use crate::operations::operation_execution::{resolve_exposure_level, resolve_exposure_score};
use crate::operations::{
    OperationExposureLevel, OperationExposureRecord, OperationRecord, OperationResolutionRecord,
};
use crate::registry::OperationExecutionDefinition;
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

pub(super) fn validate_operation_exposure_links(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
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

pub(super) fn validate_authored_operation_exposure(
    state: &AppState,
    operation: &OperationRecord,
    execution: &OperationExecutionDefinition,
    resolution: &OperationResolutionRecord,
    expected_police_response_arrived: bool,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    let exposure_factors = exposure.factors();
    let expected_intelligence_mitigation =
        u16::from(resolution.factors().intelligence_quality().value())
            * u16::from(execution.intelligence_mitigation_weight())
            / 100;
    let expected_exposure_score = resolve_exposure_score(execution, exposure_factors);
    let expected_exposure_level = resolve_exposure_level(execution, expected_exposure_score);
    if exposure_factors.variance().unsigned_abs() > execution.exposure_variance_limit()
        || exposure_factors.approach_adjustment()
            != execution
                .exposure_approach_adjustment(operation.approach())
                .expect("validated operation approach must have an exposure adjustment")
        || exposure_factors.intelligence_mitigation()
            != u8::try_from(expected_intelligence_mitigation)
                .expect("bounded exposure intelligence mitigation must fit u8")
        || exposure_factors.police_response_arrived() != expected_police_response_arrived
        || exposure.score() != expected_exposure_score
        || exposure.level() != expected_exposure_level
    {
        return Err(invalid_operation_exposure(operation));
    }
    if let Some(evidence_id) = exposure.evidence().iter().next() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or_else(|| invalid_operation_exposure(operation))?;
        if evidence.kind() != execution.exposure_evidence_kind() {
            return Err(invalid_operation_exposure(operation));
        }
    }
    Ok(())
}

fn validate_exposure_location(
    state: &AppState,
    operation: &OperationRecord,
    exposure: &OperationExposureRecord,
) -> Result<(), StateValidationError> {
    if let Some(neighborhood) = exposure.neighborhood()
        && state.world.get_neighborhood(neighborhood).is_none()
    {
        return Err(invalid_operation_exposure(operation));
    }
    Ok(())
}

fn validate_exposure_identity(
    operation: &OperationRecord,
    exposure: &OperationExposureRecord,
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
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
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
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
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

fn invalid_operation_exposure(operation: &OperationRecord) -> StateValidationError {
    StateValidationError::InvalidOperationExposure {
        operation: operation.id(),
    }
}
