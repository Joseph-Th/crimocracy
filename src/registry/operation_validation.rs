//! Validation of authored operation definitions before registry insertion.
//!
//! `builder.rs` remains the sole registry mutation owner. This module keeps the independent
//! operation authoring contracts small enough to audit and test without mixing them into the
//! insertion path.

use super::builder::RegistryBuildError;
use super::definitions::{OperationDefinition, OperationExecutionDefinition};
use crate::operations::OperationKind;

pub(super) fn validate_operation_definition(
    kind: OperationKind,
    definition: &OperationDefinition,
) -> Result<(), RegistryBuildError> {
    if definition.supported_approaches.is_empty() {
        return Err(RegistryBuildError::MissingOperationApproaches(kind));
    }
    validate_difficulty(kind, &definition.execution)?;
    validate_intelligence(kind, &definition.execution)?;
    validate_exposure(kind, &definition.execution)?;
    validate_police_response(kind, &definition.execution)?;
    validate_proceeds(kind, &definition.execution)?;
    validate_role_coverage(kind, definition)?;
    validate_approach_coverage(kind, definition)?;
    Ok(())
}

fn validate_difficulty(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    if execution.difficulty.duration.as_minutes() == 0 {
        return Err(RegistryBuildError::InvalidOperationDuration(kind));
    }
    if execution.difficulty.base_difficulty > 100 {
        return Err(RegistryBuildError::InvalidOperationDifficulty(kind));
    }
    if execution.difficulty.police_pressure_weight > 100 {
        return Err(RegistryBuildError::InvalidOperationPoliceWeight(kind));
    }
    if execution.difficulty.variance_limit > 50 {
        return Err(RegistryBuildError::InvalidOperationVariance(kind));
    }
    if execution.difficulty.partial_margin >= execution.difficulty.achieved_margin {
        return Err(RegistryBuildError::InvalidOperationOutcomeMargins(kind));
    }
    // The runtime margin is weighted ability (0..=100) minus difficulty terms
    // (base <= 100, police pressure <= 100, arrival penalty <= 100, intelligence
    // reduction <= 50, approach adjustment within the bound checked below, time
    // pressure <= 30) plus variance (-limit..=limit, <= 50), so margins outside
    // -480..=150 make one outcome unreachable for every crew and target.
    if !(-480..=150).contains(&execution.difficulty.partial_margin)
        || !(-480..=150).contains(&execution.difficulty.achieved_margin)
    {
        return Err(RegistryBuildError::InvalidOperationOutcomeMarginRange(kind));
    }
    if execution
        .difficulty
        .approach_difficulty_adjustments
        .values()
        .any(|adjustment| !(-50..=50).contains(adjustment))
    {
        return Err(RegistryBuildError::InvalidOperationApproachAdjustment(kind));
    }
    Ok(())
}

fn validate_intelligence(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    if execution.intelligence.relevant_topics.is_empty() {
        return Err(RegistryBuildError::MissingOperationIntelligenceTopics(kind));
    }
    if execution.intelligence.max_difficulty_reduction > 50 {
        return Err(RegistryBuildError::InvalidOperationIntelligenceReduction(
            kind,
        ));
    }
    if execution.intelligence.max_useful_age.as_minutes() == 0 {
        return Err(RegistryBuildError::InvalidOperationIntelligenceAge(kind));
    }
    Ok(())
}

fn validate_exposure(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    if execution.exposure.base_exposure > 100
        || execution.exposure.police_observation_weight > 100
        || execution.exposure.stealth_mitigation_weight > 100
        || execution.exposure.intelligence_mitigation_weight > 100
    {
        return Err(RegistryBuildError::InvalidOperationExposureWeight(kind));
    }
    if execution.exposure.variance_limit > 50 {
        return Err(RegistryBuildError::InvalidOperationExposureVariance(kind));
    }
    if execution.exposure.trace_threshold >= execution.exposure.witnessed_threshold
        || execution.exposure.witnessed_threshold >= execution.exposure.identifying_threshold
    {
        return Err(RegistryBuildError::InvalidOperationExposureThresholds(kind));
    }
    Ok(())
}

fn validate_police_response(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    if !(0..=100).contains(&execution.police_response.dispatch_threshold) {
        return Err(RegistryBuildError::InvalidOperationResponseThreshold(kind));
    }
    let base_delay = execution.police_response.base_response_delay.as_minutes();
    let minimum_delay = execution
        .police_response
        .minimum_response_delay
        .as_minutes();
    if base_delay == 0 || minimum_delay == 0 || minimum_delay > base_delay {
        return Err(RegistryBuildError::InvalidOperationResponseDelay(kind));
    }
    if u32::from(execution.police_response.patrol_reduction_minutes)
        > base_delay.saturating_sub(minimum_delay)
    {
        return Err(RegistryBuildError::InvalidOperationResponseReduction(kind));
    }
    if execution
        .police_response
        .entry_offset
        .is_some_and(|offset| {
            offset.as_minutes() == 0
                || offset.as_minutes() >= execution.difficulty.duration.as_minutes()
        })
    {
        return Err(RegistryBuildError::InvalidOperationEntryOffset(kind));
    }
    if execution.police_response.arrival_difficulty_penalty > 100
        || execution.police_response.arrival_exposure_penalty > 100
    {
        return Err(RegistryBuildError::InvalidOperationResponsePenalty(kind));
    }
    Ok(())
}

fn validate_proceeds(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    if let Some(property) = execution.property_proceeds {
        if !(1..=100_000).contains(&property.business_gross_basis_points) {
            return Err(RegistryBuildError::InvalidOperationPropertyValueMultiplier(
                kind,
            ));
        }
        if !(1..=10_000).contains(&property.partial_recovery_basis_points) {
            return Err(RegistryBuildError::InvalidOperationPartialPropertyRecovery(
                kind,
            ));
        }
        if !(1..=10_000).contains(&property.liquidation_recovery_basis_points) {
            return Err(RegistryBuildError::InvalidOperationPropertyLiquidationRecovery(kind));
        }
    }
    if execution.property_proceeds.is_some() != kind.can_acquire_property() {
        return Err(RegistryBuildError::OperationPropertyObjectiveContractMismatch(kind));
    }
    if let Some(cash) = execution.cash_proceeds {
        if !(1..=100_000).contains(&cash.business_take_basis_points) {
            return Err(RegistryBuildError::InvalidOperationCashTakeMultiplier(kind));
        }
        let partial = u32::from(cash.partial_take_basis_points);
        if partial == 0 || partial > cash.business_take_basis_points {
            return Err(RegistryBuildError::InvalidOperationPartialCashTake(kind));
        }
    }
    if execution.cash_proceeds.is_some() != kind.can_take_cash() {
        return Err(RegistryBuildError::OperationCashObjectiveContractMismatch(
            kind,
        ));
    }
    Ok(())
}

fn validate_role_coverage(
    kind: OperationKind,
    definition: &OperationDefinition,
) -> Result<(), RegistryBuildError> {
    for role in &definition.required_roles {
        if !definition
            .execution
            .difficulty
            .role_capabilities
            .contains_key(role)
        {
            return Err(RegistryBuildError::MissingOperationRoleCapability {
                operation: kind,
                role: *role,
            });
        }
    }
    Ok(())
}

fn validate_approach_coverage(
    kind: OperationKind,
    definition: &OperationDefinition,
) -> Result<(), RegistryBuildError> {
    for approach in &definition.supported_approaches {
        if !definition
            .execution
            .difficulty
            .approach_difficulty_adjustments
            .contains_key(approach)
        {
            return Err(RegistryBuildError::MissingOperationApproachAdjustment {
                operation: kind,
                approach: *approach,
            });
        }
        if !definition
            .execution
            .exposure
            .approach_adjustments
            .contains_key(approach)
        {
            return Err(
                RegistryBuildError::MissingOperationExposureApproachAdjustment {
                    operation: kind,
                    approach: *approach,
                },
            );
        }
    }
    for approach in definition
        .execution
        .difficulty
        .approach_difficulty_adjustments
        .keys()
    {
        if !definition.supported_approaches.contains(approach) {
            return Err(RegistryBuildError::UnexpectedOperationApproachAdjustment {
                operation: kind,
                approach: *approach,
            });
        }
    }
    for approach in definition.execution.exposure.approach_adjustments.keys() {
        if !definition.supported_approaches.contains(approach) {
            return Err(
                RegistryBuildError::UnexpectedOperationExposureApproachAdjustment {
                    operation: kind,
                    approach: *approach,
                },
            );
        }
    }
    Ok(())
}
