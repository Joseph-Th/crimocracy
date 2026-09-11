//! Validation of authored operation definitions before registry insertion.
//!
//! `builder.rs` remains the sole registry mutation owner. This module keeps the independent
//! operation authoring contracts small enough to audit and test without mixing them into the
//! insertion path.

use super::builder::RegistryBuildError;
use super::definitions::{OperationDefinition, OperationExecutionDefinition};
use crate::core::time::DAY_MINUTES_U16;
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
    validate_business_target(kind, &definition.execution)?;
    validate_outcome_reachability(kind, &definition.execution)?;
    validate_exposure_reachability(kind, &definition.execution)?;
    validate_proceeds(kind, &definition.execution)?;
    Ok(())
}

fn validate_business_target(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    let business_target_kind = kind.business_target_ownership().is_some();
    let Some(target) = execution.business_target.as_ref() else {
        return if business_target_kind {
            Err(RegistryBuildError::InvalidOperationBusinessTarget(kind))
        } else {
            Ok(())
        };
    };
    if !business_target_kind {
        return Err(RegistryBuildError::InvalidOperationBusinessTarget(kind));
    }
    if kind == OperationKind::GamblingEvent && target.required_functions.is_empty() {
        return Err(RegistryBuildError::InvalidOperationBusinessTarget(kind));
    }
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
        || (execution.difficulty.role_capability_weight > 0
            && execution.difficulty.role_capabilities.is_empty())
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

    // Worst case must respect deadline/response chronology. A completion deadline must leave at
    // least one executable minute after entry. If police also arrives, the operation cannot
    // resolve before the fastest possible response. Compare both reachable scenarios instead of
    // summing independently maximal penalties that cannot necessarily coexist.
    let no_response_time_pressure = maximum_reachable_time_pressure(execution, None);
    let fastest_response =
        crate::operations::police_response_integration::resolve_police_arrival_delay(
            execution, 100,
        );
    let response_time_pressure = maximum_reachable_time_pressure(execution, Some(fastest_response));
    let common_minimum = -i16::from(execution.difficulty.base_difficulty)
        - i16::from(execution.difficulty.police_pressure_weight)
        - i16::from(least_favorable_approach)
        - i16::from(execution.difficulty.variance_limit);
    let minimum_without_response = common_minimum - i16::from(no_response_time_pressure);
    let minimum_with_response = common_minimum
        - i16::from(execution.police_response.arrival_difficulty_penalty)
        - i16::from(response_time_pressure);
    let minimum_margin = minimum_without_response.min(minimum_with_response);

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

/// Maximum authored time pressure a legal deadline can produce. `minimum_available` optionally
/// adds another event that must occur before resolution, such as the fastest police arrival.
fn maximum_reachable_time_pressure(
    execution: &OperationExecutionDefinition,
    minimum_available: Option<u32>,
) -> u8 {
    let duration = execution.difficulty.duration.as_minutes();
    let entry_offset = execution
        .police_response
        .entry_offset
        .map_or(0, |offset| offset.as_minutes());
    let minimum_executable_window = entry_offset
        .checked_add(1)
        .expect("validated operation entry offset must leave a representable execution minute");
    let available = minimum_available
        .unwrap_or(0)
        .max(minimum_executable_window);
    crate::operations::operation_execution::resolve_time_pressure(
        crate::core::time::SimTime::ZERO,
        crate::core::time::SimTime::from_minutes(u64::from(available)),
        duration,
        execution.difficulty.max_time_pressure,
    )
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
    let day_minutes = u32::from(DAY_MINUTES_U16);
    if patrol_bucket == 0
        || patrol_bucket > day_minutes
        || !day_minutes.is_multiple_of(patrol_bucket)
    {
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
    if execution
        .exposure
        .approach_adjustments
        .values()
        .any(|adjustment| !(-50..=50).contains(adjustment))
    {
        return Err(RegistryBuildError::InvalidOperationExposureApproachAdjustment(kind));
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

/// Rejects exposure bands and response thresholds that the operation's own authored score
/// factors can never cross. A definition that can never produce either a clean escape or an
/// identifying exposure has dead outcome content; an unreachable dispatch threshold likewise
/// makes the entire authored police-response path inert.
fn validate_exposure_reachability(
    kind: OperationKind,
    execution: &OperationExecutionDefinition,
) -> Result<(), RegistryBuildError> {
    let quietest_approach = execution
        .exposure
        .approach_adjustments
        .values()
        .min()
        .copied()
        .expect("operation approach coverage is validated before exposure reachability");
    let loudest_approach = execution
        .exposure
        .approach_adjustments
        .values()
        .max()
        .copied()
        .expect("operation approach coverage is validated before exposure reachability");

    // Best concealment: no observable police presence or arrived response, maximal stealth and
    // intelligence mitigation, the quietest approach, and maximal favorable exposure variance.
    let minimum_score = i16::from(execution.exposure.base_exposure) + i16::from(quietest_approach)
        - i16::from(execution.exposure.stealth_mitigation_weight)
        - i16::from(execution.exposure.intelligence_mitigation_weight)
        - i16::from(execution.exposure.variance_limit);

    // Worst exposure: maximal police observation, an arrived response, no stealth or intelligence
    // mitigation, the loudest approach, and maximal adverse exposure variance.
    let maximum_score = i16::from(execution.exposure.base_exposure)
        + i16::from(execution.exposure.police_observation_weight)
        + i16::from(execution.police_response.arrival_exposure_penalty)
        + i16::from(loudest_approach)
        + i16::from(execution.exposure.variance_limit);

    if execution.exposure.trace_threshold <= minimum_score
        || execution.exposure.identifying_threshold > maximum_score
    {
        return Err(RegistryBuildError::InvalidOperationExposureThresholdRange(
            kind,
        ));
    }

    // Dispatch is decided at operation start before a response exists and with no resolution
    // variance. If this threshold exceeds that alert score's own maximum, response timing and
    // arrival penalties are unreachable authored content.
    let maximum_alert_score = i16::from(execution.exposure.base_exposure)
        + i16::from(execution.exposure.police_observation_weight)
        + i16::from(loudest_approach);
    if execution.police_response.dispatch_threshold > maximum_alert_score {
        return Err(RegistryBuildError::InvalidOperationResponseThresholdRange(
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
        > base_delay
            .checked_sub(minimum_delay)
            .expect("minimum response delay was validated not to exceed base delay")
    {
        return Err(RegistryBuildError::InvalidOperationResponseReduction(kind));
    }
    // Arrival penalties and police-arrival contingencies are authored behavior, not decorative
    // fields. At maximal patrol presence the deterministic delay reaches its minimum theoretical
    // value; that fastest response must still be able to arrive by the operation's resolution.
    let earliest_delay =
        crate::operations::police_response_integration::resolve_police_arrival_delay(
            execution, 100,
        );
    if earliest_delay > execution.difficulty.duration.as_minutes() {
        return Err(RegistryBuildError::InvalidOperationResponseDelay(kind));
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
        if !(1..=10_000).contains(&cash.partial_take_basis_points) {
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
