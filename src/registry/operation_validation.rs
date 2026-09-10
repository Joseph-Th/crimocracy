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
    validate_role_coverage(kind, definition)?;
    validate_approach_coverage(kind, definition)?;
    validate_outcome_reachability(kind, &definition.execution)?;
    validate_proceeds(kind, &definition.execution)?;
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
    if execution.difficulty.role_capability_weight > 100
        || execution.difficulty.leader_capability_weight > 100
        || (execution.difficulty.role_capability_weight == 0
            && execution.difficulty.leader_capability_weight == 0)
    {
        return Err(RegistryBuildError::InvalidOperationAbilityWeights(kind));
    }
    if execution.difficulty.police_pressure_weight > 100 {
        return Err(RegistryBuildError::InvalidOperationPoliceWeight(kind));
    }
    if !(1..=100).contains(&execution.difficulty.max_time_pressure) {
        return Err(RegistryBuildError::InvalidOperationTimePressure(kind));
    }
    if execution.difficulty.variance_limit > 50 {
        return Err(RegistryBuildError::InvalidOperationVariance(kind));
    }
    if execution.difficulty.partial_margin >= execution.difficulty.achieved_margin {
        return Err(RegistryBuildError::InvalidOperationOutcomeMargins(kind));
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

/// Rejects thresholds that the operation's own authored factor ranges can never cross. This is
/// deliberately definition-relative rather than a project-wide coarse bound: an operation with a
/// mild base difficulty and narrow variance must not author an achieved threshold that only some
/// entirely different, more favorable definition could theoretically reach.
fn validate_outcome_reachability(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    let most_favorable_approach = execution
        .difficulty
        .approach_difficulty_adjustments
        .values()
        .min()
        .copied()
        .expect("operation approach coverage is validated before reachability");
    let least_favorable_approach = execution
        .difficulty
        .approach_difficulty_adjustments
        .values()
        .max()
        .copied()
        .expect("operation approach coverage is validated before reachability");

    // Best case: maximal crew ability, no police pressure/arrival or time pressure, strongest
    // possible authored intelligence benefit, best approach, and maximal favorable variance.
    let maximum_margin = 100_i16 - i16::from(execution.difficulty.base_difficulty)
        + i16::from(execution.intelligence.max_difficulty_reduction)
        - i16::from(most_favorable_approach)
        + i16::from(execution.difficulty.variance_limit);

    // Worst case: zero crew ability, maximal ambient police pressure, police arrival, worst
    // approach, maximal time compression, no intelligence benefit, and adverse variance.
    let minimum_margin = -i16::from(execution.difficulty.base_difficulty)
        - i16::from(execution.difficulty.police_pressure_weight)
        - i16::from(execution.police_response.arrival_difficulty_penalty)
        - i16::from(least_favorable_approach)
        - i16::from(execution.difficulty.max_time_pressure)
        - i16::from(execution.difficulty.variance_limit);

    // Achieved requires margin >= achieved; Failed requires margin < partial. If either boundary
    // lies outside the operation's own theoretical range, that outcome is impossible regardless
    // of runtime state.
    if execution.difficulty.achieved_margin > maximum_margin
        || execution.difficulty.partial_margin <= minimum_margin
    {
        return Err(RegistryBuildError::InvalidOperationOutcomeMarginRange(kind));
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
    let patrol_bucket = execution
        .intelligence
        .patrol_observation_bucket
        .as_minutes();
    if patrol_bucket == 0 || patrol_bucket > 1_440 || 1_440 % patrol_bucket != 0 {
        return Err(RegistryBuildError::InvalidOperationPatrolObservationBucket(
            kind,
        ));
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
    if execution.exposure.witness_reluctant_police_presence > 100
        || execution.exposure.witness_cooperative_police_presence > 100
        || execution.exposure.high_police_presence_narrative_threshold > 100
        || execution.exposure.witness_reluctant_police_presence
            >= execution.exposure.witness_cooperative_police_presence
    {
        return Err(RegistryBuildError::InvalidOperationWitnessPoliceThresholds(
            kind,
        ));
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
        if property.recent_take_recovery_window.as_minutes() == 0
            || !(1..=10_000).contains(&property.immediate_repeat_value_basis_points)
        {
            return Err(RegistryBuildError::InvalidOperationTakeRecovery(kind));
        }
        if property.liquidation_police_neutral_rating > 100
            || property.liquidation_police_adjustment_basis_points_per_point > 100
            || !(1..=10_000).contains(&property.liquidation_min_recovery_basis_points)
            || !(1..=10_000).contains(&property.liquidation_max_recovery_basis_points)
            || property.liquidation_min_recovery_basis_points
                > property.liquidation_recovery_basis_points
            || property.liquidation_recovery_basis_points
                > property.liquidation_max_recovery_basis_points
        {
            return Err(
                RegistryBuildError::InvalidOperationPropertyLiquidationPoliceAdjustment(kind),
            );
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
        if cash.recent_take_recovery_window.as_minutes() == 0
            || !(1..=10_000).contains(&cash.immediate_repeat_value_basis_points)
        {
            return Err(RegistryBuildError::InvalidOperationTakeRecovery(kind));
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
