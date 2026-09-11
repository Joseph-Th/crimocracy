//! Validation of objective-specific side effects for a resolved operation.
//!
//! The parent `operation_execution` module remains the transaction owner. This child keeps the
//! cross-domain effect checks together so the canonical resolution boundary does not mix
//! proceeds staleness, custody release, witness pressure, and business disruption branching with
//! artifact planning.

use super::{OperationResolutionError, OperationResolutionOutcomePlan};
use crate::core::entity::EntityRef;
use crate::core::state::AppState;
use crate::economy::business_economy_system::{
    ValidatedBusinessDisruption, validate_disrupt_business_economy,
};
use crate::legal::WitnessCooperation;
use crate::legal::arrest_system::{ValidatedRelease, validate_release_arrest};
use crate::legal::witness_system::{ValidatedWitnessCooperation, validate_set_witness_cooperation};
use crate::operations::operation_economics::{resolve_cash_proceeds, resolve_property_proceeds};
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationRecord,
};
use crate::registry::Registry;

pub(super) struct ValidatedResolutionEffects {
    pub(super) detainee_release: Option<ValidatedRelease>,
    pub(super) witness_intimidation: Vec<ValidatedWitnessCooperation>,
    pub(super) business_disruption: Option<ValidatedBusinessDisruption>,
}

pub(super) fn validate_resolution_effects(
    registry: &Registry,
    state: &AppState,
    record: &OperationRecord,
    outcome: &OperationResolutionOutcomePlan,
) -> Result<ValidatedResolutionEffects, OperationResolutionError> {
    validate_proceeds_context(registry, state, record, outcome)?;
    Ok(ValidatedResolutionEffects {
        detainee_release: validate_detainee_release(state, record, outcome)?,
        witness_intimidation: validate_witness_intimidation(state, record, outcome)?,
        business_disruption: validate_business_disruption(registry, state, record, outcome)?,
    })
}

fn validate_proceeds_context(
    registry: &Registry,
    state: &AppState,
    record: &OperationRecord,
    outcome: &OperationResolutionOutcomePlan,
) -> Result<(), OperationResolutionError> {
    let expected_property_proceeds =
        resolve_property_proceeds(registry, state, record, outcome.objective_outcome)?;
    if outcome.property_proceeds_plan != expected_property_proceeds {
        return Err(OperationResolutionError::StalePropertyProceedsContext {
            operation: record.id(),
        });
    }
    let expected_cash_proceeds =
        resolve_cash_proceeds(registry, state, record, outcome.objective_outcome)?;
    if outcome.cash_proceeds_plan != expected_cash_proceeds {
        return Err(OperationResolutionError::StaleCashProceedsContext {
            operation: record.id(),
        });
    }
    Ok(())
}

fn validate_detainee_release(
    state: &AppState,
    record: &OperationRecord,
    outcome: &OperationResolutionOutcomePlan,
) -> Result<Option<ValidatedRelease>, OperationResolutionError> {
    let OperationObjective::FreeDetainee { target } = record.objective() else {
        return Ok(None);
    };
    if outcome.objective_outcome == OperationObjectiveOutcome::Failed {
        return Ok(None);
    }
    let arrest =
        outcome
            .extraction_arrest
            .ok_or(OperationResolutionError::StaleExtractionContext {
                operation: record.id(),
            })?;
    let release = validate_release_arrest(state, arrest).map_err(|error| {
        OperationResolutionError::DetaineeRelease {
            operation: record.id(),
            character: *target,
            error,
        }
    })?;
    debug_assert_eq!(release.arrest(), arrest);
    Ok(Some(release))
}

fn validate_witness_intimidation(
    state: &AppState,
    record: &OperationRecord,
    outcome: &OperationResolutionOutcomePlan,
) -> Result<Vec<ValidatedWitnessCooperation>, OperationResolutionError> {
    if outcome.objective_outcome == OperationObjectiveOutcome::Failed
        || !matches!(
            (record.kind(), record.objective()),
            (
                OperationKind::WitnessPressure,
                OperationObjective::Frighten {
                    target: EntityRef::Character(_),
                },
            )
        )
    {
        return Ok(Vec::new());
    }

    outcome
        .witness_pressure_targets
        .iter()
        .map(|(case_witness, cooperation)| {
            let degraded = match cooperation {
                WitnessCooperation::Cooperative => WitnessCooperation::Reluctant,
                WitnessCooperation::Reluctant => WitnessCooperation::Hostile,
                WitnessCooperation::Hostile => {
                    unreachable!("pressureable witness targets exclude hostile cooperation")
                }
            };
            validate_set_witness_cooperation(state, *case_witness, degraded).map_err(Into::into)
        })
        .collect()
}

fn validate_business_disruption(
    registry: &Registry,
    state: &AppState,
    record: &OperationRecord,
    outcome: &OperationResolutionOutcomePlan,
) -> Result<Option<ValidatedBusinessDisruption>, OperationResolutionError> {
    if outcome.objective_outcome == OperationObjectiveOutcome::Failed {
        return Ok(None);
    }
    let (
        OperationKind::Sabotage | OperationKind::Arson,
        OperationObjective::DisruptBusiness {
            target: EntityRef::Business(business),
        },
    ) = (record.kind(), record.objective())
    else {
        return Ok(None);
    };
    Ok(Some(validate_disrupt_business_economy(
        registry, state, *business,
    )?))
}
