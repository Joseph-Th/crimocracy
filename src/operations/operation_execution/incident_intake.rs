//! Police incident intake derived from persisted operation exposure.

use super::{OperationExposurePlan, OperationResolutionError};
use crate::core::entity::EntityRef;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::investigation_system::{ValidatedIncidentIntake, validate_incident_intake};
use crate::legal::jurisdiction_system::{
    CaseIntakeAuthoritySnapshot, resolve_case_intake_authority_snapshot,
};
use crate::legal::{
    Admissibility, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft, IncidentWitnessDraft, WitnessCooperation,
};
use crate::operations::{OperationExposureLevel, OperationObjective, OperationRecord};
use crate::registry::{OperationExecutionDefinition, Registry};
use crate::world::{BusinessOwner, Rating};
use std::collections::BTreeSet;

fn resolve_incident_witness(
    execution: &OperationExecutionDefinition,
    state: &AppState,
    operation: &OperationRecord,
    exposure: &OperationExposurePlan,
    target_police_presence: Option<Rating>,
) -> Option<IncidentWitnessDraft> {
    if !matches!(
        exposure.level,
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ) {
        return None;
    }
    // Only business targets have an identifiable on-scene witness today: the owner.
    let target = match operation.objective() {
        OperationObjective::AcquireProperty { target }
        | OperationObjective::ObtainCash { target }
        | OperationObjective::DisruptBusiness { target } => Some(*target),
        OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::FreeDetainee { .. } => None,
    }?;
    let EntityRef::Business(business) = target else {
        return None;
    };
    let record = state.world.get_business(business)?;
    let BusinessOwner::Character(character) = record.owner() else {
        return None;
    };
    let witness = state.world.get_character(character)?;
    // The identified participant cannot witness their own crime, and an organization's own
    // member is not treated as the case's named witness against it.
    if Some(character) == exposure.identified_character
        || witness.organization() == Some(operation.responsible_organization())
    {
        return None;
    }
    // Patrol presence shapes whether a witness is willing to stand behind an account (§31).
    let cooperation = match target_police_presence.map(Rating::value) {
        Some(presence) if presence >= execution.witness_cooperative_police_presence() => {
            WitnessCooperation::Cooperative
        }
        Some(presence) if presence >= execution.witness_reluctant_police_presence() => {
            WitnessCooperation::Reluctant
        }
        _ => WitnessCooperation::Hostile,
    };
    Some(IncidentWitnessDraft {
        character,
        subject: exposure
            .identified_character
            .map(EntityRef::Character)
            .unwrap_or(EntityRef::Operation(operation.id())),
        cooperation,
    })
}

pub(super) fn validate_exposure_incident(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    exposure: &OperationExposurePlan,
    target_police_presence: Option<Rating>,
    discovered_at: SimTime,
) -> Result<
    (
        Option<ValidatedIncidentIntake>,
        Option<CaseIntakeAuthoritySnapshot>,
    ),
    OperationResolutionError,
> {
    if exposure.level == OperationExposureLevel::None {
        return Ok((None, None));
    }
    let Some(neighborhood) = exposure.neighborhood else {
        return Ok((None, None));
    };
    let authority_snapshot = resolve_case_intake_authority_snapshot(state, neighborhood);
    let Some(owner) = authority_snapshot.organization else {
        return Ok((None, Some(authority_snapshot)));
    };
    let subject = exposure
        .identified_character
        .map(EntityRef::Character)
        .unwrap_or(EntityRef::Operation(operation.id()));
    let strength = match exposure.level {
        OperationExposureLevel::None => unreachable!("non-exposure cannot create an incident"),
        OperationExposureLevel::Trace => EvidenceStrength::Weak,
        OperationExposureLevel::Witnessed => EvidenceStrength::Corroborating,
        OperationExposureLevel::Identifying => EvidenceStrength::Strong,
    };
    let reliability = match exposure.level {
        OperationExposureLevel::None => unreachable!("non-exposure cannot create an incident"),
        OperationExposureLevel::Trace => EvidenceReliability::Questionable,
        OperationExposureLevel::Witnessed => EvidenceReliability::Credible,
        OperationExposureLevel::Identifying => EvidenceReliability::HighlyReliable,
    };
    let mut subjects = BTreeSet::from([EntityRef::Operation(operation.id())]);
    if let Some(character) = exposure.identified_character {
        subjects.insert(EntityRef::Character(character));
    }
    let execution = registry.get_operation(operation.kind()).execution();
    let kind = execution.exposure_evidence_kind();
    // A witnessed or identifying exposure leaves a named witness when the target is a
    // character-owned business: the owner saw it happen. Members of the responsible
    // organization and the identified participant never count as the case's witness.
    let witness = resolve_incident_witness(
        execution,
        state,
        operation,
        exposure,
        target_police_presence,
    );
    let incident = validate_incident_intake(
        state,
        IncidentIntakeDraft {
            owner,
            title: format!("Incident linked to {}", operation.title()),
            subjects,
            evidence: vec![IncidentEvidenceDraft {
                subject,
                origin: Some(EntityRef::Operation(operation.id())),
                kind,
                strength,
                reliability,
                admissibility: Admissibility::Unknown,
                discovered_at,
            }],
            origin: Some(EntityRef::Operation(operation.id())),
            witness,
        },
    )?;
    Ok((Some(incident), Some(authority_snapshot)))
}
