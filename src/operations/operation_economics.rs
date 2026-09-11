//! Operation take economics: recency-depleted proceeds and after-action property clauses.
//!
//! Sibling `operation_execution` composes these into resolution plans; the executive brief
//! references the clause builders when refreshing liquidated property. All amounts flow through
//! the canonical ledger at commit time; this module derives plans only.
//!
//! Take sizing is deliberately static: authored basis points of the target's registry-derived
//! gross potential model the venue's *typical contents*, not its live register or operating
//! status. A suspended storefront still holds goods worth taking; only the recent-take
//! depletion index (persisted at completion commit) decays a repeated target.
use super::operation_execution::OperationResolutionError;
use crate::core::entity::EntityRef;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::finance::Money;
use crate::finance::helpers::apply_basis_point_multiplier;
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationPropertyProceedsRecord,
};
use crate::registry::Registry;

#[derive(Clone, Copy, Debug)]
struct TakeEconomics {
    full_basis_points: u32,
    partial_basis_points: u16,
    recovery_window: SimDuration,
    immediate_repeat_value_basis_points: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PropertyProceedsPlan {
    pub(crate) proceeds: Option<OperationPropertyProceedsRecord>,
    /// True when a recent successful same-kind take on the same target reduced this haul.
    pub(crate) depleted_by_recent_take: bool,
}

pub(crate) fn resolve_property_proceeds(
    registry: &Registry,
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<PropertyProceedsPlan, OperationResolutionError> {
    let Some(definition) = registry
        .get_operation(operation.kind())
        .execution()
        .property_proceeds()
    else {
        return Ok(PropertyProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    };
    let OperationObjective::AcquireProperty {
        target: EntityRef::Business(business),
    } = operation.objective()
    else {
        return Ok(PropertyProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    };
    if outcome == OperationObjectiveOutcome::Failed {
        return Ok(PropertyProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    }

    let gross = resolve_business_gross_potential(registry, state, *business)?;
    let reference_at = take_reference_time(state, operation);
    let recent_hits = recent_take_times(
        state,
        operation,
        *business,
        definition.recent_take_recovery_window(),
    );
    let cents = resolve_take_cents(
        operation.id(),
        gross.cents(),
        TakeEconomics {
            full_basis_points: definition.business_gross_basis_points(),
            partial_basis_points: definition.partial_recovery_basis_points(),
            recovery_window: definition.recent_take_recovery_window(),
            immediate_repeat_value_basis_points: definition.immediate_repeat_value_basis_points(),
        },
        outcome,
        reference_at,
        &recent_hits,
        |operation| OperationResolutionError::PropertyProceedsOverflow { operation },
    )?;
    if cents <= 0 {
        return Ok(PropertyProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: !recent_hits.is_empty(),
        });
    }
    Ok(PropertyProceedsPlan {
        proceeds: Some(OperationPropertyProceedsRecord::new(
            EntityRef::Business(*business),
            crate::finance::Money::from_cents(cents),
        )),
        depleted_by_recent_take: !recent_hits.is_empty(),
    })
}

/// Recent successful same-kind takes against the same target at this operation's own resolution
/// instant. A committed operation must keep validating against exactly the take history it saw
/// when it resolved.
fn take_reference_time(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
) -> SimTime {
    operation
        .resolution()
        .map(|resolution| resolution.resolved_at())
        .unwrap_or_else(|| state.now())
}

pub(crate) fn recent_take_times(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    business: crate::core::id::BusinessId,
    recovery_window: SimDuration,
) -> Vec<SimTime> {
    let reference_at = take_reference_time(state, operation);
    // Served from the depletion index maintained at completion commit time.
    state.operations.recent_successful_take_times(
        business,
        operation.kind(),
        reference_at,
        recovery_window,
        operation.id(),
    )
}

/// Shared take economics: authored basis points of the target's gross potential, scaled down on a
/// partial outcome and again by each recent successful same-kind hit against the same target.
fn resolve_take_cents(
    operation: crate::core::id::OperationId,
    gross_cents: i64,
    economics: TakeEconomics,
    outcome: OperationObjectiveOutcome,
    reference_at: SimTime,
    recent_hits: &[SimTime],
    overflow: fn(crate::core::id::OperationId) -> OperationResolutionError,
) -> Result<i64, OperationResolutionError> {
    let full_value =
        apply_basis_point_multiplier(Money::from_cents(gross_cents), economics.full_basis_points)
            .ok_or(overflow(operation))?;
    let mut value = match outcome {
        OperationObjectiveOutcome::Achieved => full_value,
        OperationObjectiveOutcome::Partial => {
            apply_basis_point_multiplier(full_value, u32::from(economics.partial_basis_points))
                .ok_or(overflow(operation))?
        }
        OperationObjectiveOutcome::Failed => {
            unreachable!("failed takes return early")
        }
    };
    let window_minutes = u64::from(economics.recovery_window.as_minutes());
    let depletion_span = 10_000_u64 - u64::from(economics.immediate_repeat_value_basis_points);
    // Each prior hit starts at the authored immediate-repeat penalty and recovers linearly to
    // full value over the authored window. Multiple recent hits compound independently, which
    // keeps repeated farming unattractive without an all-or-nothing replenishment cliff.
    for hit_at in recent_hits {
        let age = reference_at
            .as_minutes()
            .checked_sub(hit_at.as_minutes())
            .expect("recent-take index must not return a future operation")
            .min(window_minutes);
        let unrecovered = window_minutes.saturating_sub(age);
        let depletion = depletion_span * unrecovered / window_minutes;
        let value_basis_points = 10_000_u64.saturating_sub(depletion);
        let scaled = apply_basis_point_multiplier(
            value,
            u32::try_from(value_basis_points)
                .expect("bounded take-recovery basis points must fit u32"),
        )
        .ok_or(overflow(operation))?;
        // Repeat-target depletion is a penalty, not neutral pricing. Cent rounding must not
        // erase a still-active penalty by mapping a positive amount back to itself, otherwise
        // one-cent proceeds become an immortal fixed point under repeated hits. Preserve the
        // shared half-away rule when it moves value, but force at least one cent of reduction
        // while the hit is not fully recovered.
        value = if value_basis_points < 10_000 && value > Money::ZERO && scaled >= value {
            value
                .checked_sub(Money::from_cents(1))
                .expect("positive take value can lose one cent")
        } else {
            scaled
        };
    }
    Ok(value.cents())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CashProceedsPlan {
    pub(crate) proceeds: Option<crate::operations::OperationCashProceedsRecord>,
    pub(crate) depleted_by_recent_take: bool,
}

/// Derives the cash a successful take carries home. Mirrors the property-proceeds economics:
/// authored basis points of the target business's gross potential, scaled down on a partial
/// outcome and by each recent successful hit against the same target.
pub(crate) fn resolve_cash_proceeds(
    registry: &Registry,
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<CashProceedsPlan, OperationResolutionError> {
    let Some(definition) = registry
        .get_operation(operation.kind())
        .execution()
        .cash_proceeds()
    else {
        return Ok(CashProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    };
    let OperationObjective::ObtainCash {
        target: EntityRef::Business(business),
    } = operation.objective()
    else {
        return Ok(CashProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    };
    if outcome == OperationObjectiveOutcome::Failed {
        return Ok(CashProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: false,
        });
    }

    let gross = resolve_business_gross_potential(registry, state, *business)?;
    let reference_at = take_reference_time(state, operation);
    let recent_hits = recent_take_times(
        state,
        operation,
        *business,
        definition.recent_take_recovery_window(),
    );
    let cents = resolve_take_cents(
        operation.id(),
        gross.cents(),
        TakeEconomics {
            full_basis_points: definition.business_take_basis_points(),
            partial_basis_points: definition.partial_take_basis_points(),
            recovery_window: definition.recent_take_recovery_window(),
            immediate_repeat_value_basis_points: definition.immediate_repeat_value_basis_points(),
        },
        outcome,
        reference_at,
        &recent_hits,
        |operation| OperationResolutionError::CashProceedsOverflow { operation },
    )?;
    if cents <= 0 {
        return Ok(CashProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: !recent_hits.is_empty(),
        });
    }
    Ok(CashProceedsPlan {
        proceeds: Some(crate::operations::OperationCashProceedsRecord::new(
            EntityRef::Business(*business),
            crate::finance::Money::from_cents(cents),
        )),
        depleted_by_recent_take: !recent_hits.is_empty(),
    })
}

/// Historical after-action phrasing for property secured by an operation. It describes what
/// happened at resolution time without claiming the property is still held when a later report
/// in the same executive window records its liquidation.
pub(crate) fn held_property_clause(est_value_cents: i64) -> String {
    format!(
        "The crew secured property with an estimated held value of {}; the haul was held for later liquidation.",
        crate::finance::helpers::format_money_cents(est_value_cents)
    )
}

/// Historical after-action phrasing for direct cash proceeds. The economic event differs by
/// operation kind even though every result enters the same held-cash disposition lifecycle.
/// A later deposit remains a separate financial event rather than requiring history rewriting.
pub(crate) fn held_cash_clause(kind: OperationKind, cents: i64) -> String {
    let amount = crate::finance::helpers::format_money_cents(cents);
    match kind {
        OperationKind::Robbery => {
            format!("The crew took {amount} in cash from the target and held it for later deposit.")
        }
        OperationKind::Intimidation => {
            format!(
                "The collection brought in {amount} in cash, and the crew held it for later deposit."
            )
        }
        OperationKind::Smuggling => {
            format!("The delivery paid {amount} in cash, and the crew held it for later deposit.")
        }
        OperationKind::GamblingEvent => {
            format!(
                "The event cleared {amount} in cash for the house, and the crew held it for later deposit."
            )
        }
        OperationKind::Burglary
        | OperationKind::Hijacking
        | OperationKind::DocumentTheft
        | OperationKind::Surveillance
        | OperationKind::WitnessPressure
        | OperationKind::Extraction
        | OperationKind::Sabotage
        | OperationKind::Arson => {
            unreachable!("operation kind has no authored cash proceeds")
        }
    }
}

/// After-action phrasing for same-kind recency depletion. Different proceeds models represent
/// different practical bottlenecks, so the explanation must not describe gambling receipts or
/// delivery payment as unreplaced physical stock.
pub(crate) fn depleted_take_clause(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Burglary | OperationKind::Hijacking | OperationKind::DocumentTheft => {
            "The take came in lighter than usual; this target has not fully replaced stock from a recent score."
        }
        OperationKind::Robbery | OperationKind::Intimidation => {
            "The take came in lighter than usual; available cash at this target has not fully recovered from a recent collection."
        }
        OperationKind::Smuggling => {
            "The run paid less than usual; this destination is still absorbing a recent delivery."
        }
        OperationKind::GamblingEvent => {
            "The event took in less than usual; betting volume at this venue has not fully recovered from a recent event."
        }
        OperationKind::Surveillance
        | OperationKind::WitnessPressure
        | OperationKind::Extraction
        | OperationKind::Sabotage
        | OperationKind::Arson => {
            unreachable!("operation kind has no authored take proceeds")
        }
    }
}

/// After-action phrasing for successful sabotage: the target's earning power is degraded for
/// the authored disruption horizon.
pub(crate) const SABOTAGE_DISRUPTION_CLAUSE: &str =
    "The target's operations are disrupted and will earn well below normal until repairs catch up.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::id::OperationId;

    fn proceeds_overflow(operation: OperationId) -> OperationResolutionError {
        OperationResolutionError::PropertyProceedsOverflow { operation }
    }

    #[test]
    fn take_economics_rounds_pricing_consistently_without_erasing_depletion() {
        let operation = OperationId::from_raw(1);
        let partial = resolve_take_cents(
            operation,
            3,
            TakeEconomics {
                full_basis_points: 5_000,
                partial_basis_points: 5_000,
                recovery_window: SimDuration::from_minutes(10),
                immediate_repeat_value_basis_points: 5_000,
            },
            OperationObjectiveOutcome::Partial,
            SimTime::from_minutes(10),
            &[],
            proceeds_overflow,
        )
        .expect("small partial take should remain representable");
        assert_eq!(
            partial, 1,
            "3c at 50%, then 50% again should round 2c to 1c rather than truncate to zero"
        );

        let depleted = resolve_take_cents(
            operation,
            1,
            TakeEconomics {
                full_basis_points: 10_000,
                partial_basis_points: 10_000,
                recovery_window: SimDuration::from_minutes(10),
                immediate_repeat_value_basis_points: 5_000,
            },
            OperationObjectiveOutcome::Achieved,
            SimTime::from_minutes(10),
            &[SimTime::from_minutes(10)],
            proceeds_overflow,
        )
        .expect("small depleted take should remain representable");
        assert_eq!(
            depleted, 0,
            "an active repeat-target penalty must not round a one-cent take back to one cent"
        );
    }
}
