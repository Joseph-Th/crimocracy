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
use crate::core::time::SimDuration;
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationPropertyProceedsRecord,
};
use crate::registry::Registry;
/// A successful same-kind take from the same business inside this window finds only partially
/// replenished value in that operation channel, so repeat scores decay without unrelated kinds
/// suppressing each other.
pub(crate) const RECENT_HIT_WINDOW: SimDuration = SimDuration::from_minutes(3 * 24 * 60);
/// Each recent prior successful take leaves this share of the remaining loot value.
pub(crate) const RECENT_HIT_VALUE_BASIS_POINTS: i128 = 5_000;

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
    let recent_hits = recent_take_hits(state, operation, *business);
    let cents = resolve_take_cents(
        operation.id(),
        gross.cents(),
        definition.business_gross_basis_points(),
        definition.partial_recovery_basis_points(),
        outcome,
        recent_hits,
        |operation| OperationResolutionError::PropertyProceedsOverflow { operation },
    )?;
    if cents <= 0 {
        return Ok(PropertyProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: recent_hits > 0,
        });
    }
    Ok(PropertyProceedsPlan {
        proceeds: Some(OperationPropertyProceedsRecord::new(
            EntityRef::Business(*business),
            crate::finance::Money::from_cents(cents),
        )),
        depleted_by_recent_take: recent_hits > 0,
    })
}

/// Recent successful same-kind takes against the same target at this operation's own resolution
/// instant. A committed operation must keep validating against exactly the take history it saw
/// when it resolved.
pub(crate) fn recent_take_hits(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    business: crate::core::id::BusinessId,
) -> u32 {
    let reference_at = operation
        .resolution()
        .map(|resolution| resolution.resolved_at())
        .unwrap_or_else(|| state.now());
    // Served from the depletion index maintained at completion commit time.
    state.operations.recent_successful_takes(
        business,
        operation.kind(),
        reference_at,
        RECENT_HIT_WINDOW,
        operation.id(),
    )
}

/// Shared take economics: authored basis points of the target's gross potential, scaled down on a
/// partial outcome and again by each recent successful same-kind hit against the same target.
pub(crate) fn resolve_take_cents(
    operation: crate::core::id::OperationId,
    gross_cents: i64,
    full_basis_points: u32,
    partial_basis_points: u16,
    outcome: OperationObjectiveOutcome,
    recent_hits: u32,
    overflow: fn(crate::core::id::OperationId) -> OperationResolutionError,
) -> Result<i64, OperationResolutionError> {
    let full_value = i128::from(gross_cents)
        .checked_mul(i128::from(full_basis_points))
        .ok_or(overflow(operation))?
        / 10_000_i128;
    let mut value = match outcome {
        OperationObjectiveOutcome::Achieved => full_value,
        OperationObjectiveOutcome::Partial => {
            full_value
                .checked_mul(i128::from(partial_basis_points))
                .ok_or(overflow(operation))?
                / 10_000_i128
        }
        OperationObjectiveOutcome::Failed => {
            unreachable!("failed takes return early")
        }
    };
    // Each prior hit inside the recency window multiplies the remaining take down so farming
    // one target decays.
    for _ in 0..recent_hits {
        value = value
            .checked_mul(RECENT_HIT_VALUE_BASIS_POINTS)
            .ok_or(overflow(operation))?
            / 10_000_i128;
    }
    i64::try_from(value).map_err(|_| overflow(operation))
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
    let recent_hits = recent_take_hits(state, operation, *business);
    let cents = resolve_take_cents(
        operation.id(),
        gross.cents(),
        definition.business_take_basis_points(),
        definition.partial_take_basis_points(),
        outcome,
        recent_hits,
        |operation| OperationResolutionError::CashProceedsOverflow { operation },
    )?;
    if cents <= 0 {
        return Ok(CashProceedsPlan {
            proceeds: None,
            depleted_by_recent_take: recent_hits > 0,
        });
    }
    Ok(CashProceedsPlan {
        proceeds: Some(crate::operations::OperationCashProceedsRecord::new(
            EntityRef::Business(*business),
            crate::finance::Money::from_cents(cents),
        )),
        depleted_by_recent_take: recent_hits > 0,
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

/// Historical after-action phrasing for cash taken by an operation. A later deposit remains a
/// separate financial event rather than requiring this historical report to be rewritten.
pub(crate) fn held_cash_clause(cents: i64) -> String {
    format!(
        "The crew took {} in cash and held it for later deposit.",
        crate::finance::helpers::format_money_cents(cents)
    )
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
