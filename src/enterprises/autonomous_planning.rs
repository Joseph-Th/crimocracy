//! Shared read-only planning projections for autonomous enterprise governance.
//!
//! Expansion and suspended-racket maintenance consume the same bounded district-pressure
//! knowledge and working-capital reservations. Keeping those projections here prevents either
//! autonomous behavior from depending on the other's implementation details.

use crate::core::id::{FinancialAccountId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::EnterpriseLocation;
use crate::enterprises::enterprise_execution::{
    EnterpriseError, decode_enterprise_investigation_case_count,
    resolve_enterprise_operating_cost_projection, resolve_location_neighborhood,
};
use crate::finance::finance_system::FinanceError;
use crate::finance::{FinancialAccountRecord, Money};
use crate::registry::Registry;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum AutonomousEnterpriseError {
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Enterprise(#[from] EnterpriseError),
}

pub(crate) type ObservedDistrictPressure = BTreeMap<(OrganizationId, NeighborhoodId), u32>;

pub(crate) fn resolve_committed_working_capital(
    registry: &Registry,
    state: &AppState,
    observed_district_pressure: &ObservedDistrictPressure,
) -> Result<BTreeMap<FinancialAccountId, Money>, AutonomousEnterpriseError> {
    let mut reservations = BTreeMap::new();
    for enterprise in state.enterprises().active_enterprises() {
        let observed_active_cases = resolve_observed_district_case_count(
            state,
            observed_district_pressure,
            enterprise.organization(),
            enterprise.location(),
        )?;
        let required = resolve_enterprise_operating_cost_projection(
            registry,
            state,
            enterprise.kind(),
            enterprise.location(),
            enterprise.supporting_businesses().len(),
            observed_active_cases,
        )?;
        reserve_working_capital(&mut reservations, enterprise.cash_account(), required)?;
    }
    Ok(reservations)
}

/// Most recent district-pressure count the organization can infer from its own settled rackets.
/// Live investigations are intentionally excluded: only the organization's bounded recent
/// operating history can feed autonomous planning.
pub(crate) fn resolve_observed_district_pressure(
    registry: &Registry,
    state: &AppState,
) -> Result<ObservedDistrictPressure, AutonomousEnterpriseError> {
    let mut latest_observations = BTreeMap::new();
    let maximum_age = u64::from(registry.legal().cold_case_window().as_minutes());
    let lower_bound = state
        .now()
        .as_minutes()
        .checked_sub(maximum_age)
        .map(crate::core::time::SimTime::from_minutes);
    for cycle in state
        .enterprises()
        .cycles_after_through(lower_bound, state.now())
    {
        let enterprise = state
            .enterprises()
            .get_enterprise(cycle.enterprise())
            .expect("enterprise cycle must reference its persisted enterprise");
        let economics = registry.get_enterprise(enterprise.kind()).economics();
        if economics.heat_surcharge_per_active_case() <= Money::ZERO {
            continue;
        }
        let inferred =
            decode_enterprise_investigation_case_count(economics, cycle.investigation_heat())
                .expect("validated enterprise heat must encode a representable active-case count");
        let key = (cycle.occurred_at(), cycle.id());
        let neighborhood = resolve_location_neighborhood(state, enterprise.location())?;
        let observation = latest_observations
            .entry((enterprise.organization(), neighborhood))
            .or_insert((key, inferred));
        if key > observation.0 {
            *observation = (key, inferred);
        }
    }
    Ok(latest_observations
        .into_iter()
        .map(|(district, (_, count))| (district, count))
        .collect())
}

pub(crate) fn resolve_observed_district_case_count(
    state: &AppState,
    observed_district_pressure: &ObservedDistrictPressure,
    organization: OrganizationId,
    location: EnterpriseLocation,
) -> Result<u32, AutonomousEnterpriseError> {
    let neighborhood = resolve_location_neighborhood(state, location)?;
    Ok(observed_district_pressure
        .get(&(organization, neighborhood))
        .copied()
        .unwrap_or(0))
}

pub(crate) fn reserve_working_capital(
    reservations: &mut BTreeMap<FinancialAccountId, Money>,
    account: FinancialAccountId,
    amount: Money,
) -> Result<(), AutonomousEnterpriseError> {
    debug_assert!(
        amount >= Money::ZERO,
        "enterprise working-capital requirements must be nonnegative"
    );
    let current = reservations.get(&account).copied().unwrap_or(Money::ZERO);
    // This is a planning claim, not authoritative money. If many live rackets share one account,
    // their combined runway can exceed i64 even though every enterprise and the account itself are
    // individually valid. Saturating means "more reserved than any account can fund", which is
    // exactly the only fact later candidate selection needs and avoids turning scale into a tick
    // failure.
    let reserved = current
        .checked_add(amount)
        .unwrap_or(Money::from_cents(i64::MAX));
    reservations.insert(account, reserved);
    Ok(())
}

pub(crate) fn available_working_capital(
    account: &FinancialAccountRecord,
    reservations: &BTreeMap<FinancialAccountId, Money>,
) -> Money {
    if account.version() == u32::MAX {
        return Money::ZERO;
    }
    let reserved = reservations
        .get(&account.id())
        .copied()
        .unwrap_or(Money::ZERO);
    let spendable = account.spendable_balance();
    if spendable <= reserved {
        return Money::ZERO;
    }
    spendable
        .checked_sub(reserved)
        .expect("positive balance above a nonnegative reservation must subtract safely")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_capital_projection_saturates_when_shared_claims_exceed_money_range() {
        let account = FinancialAccountId::from_raw(1);
        let mut reservations = BTreeMap::new();
        reserve_working_capital(&mut reservations, account, Money::from_cents(i64::MAX - 5))
            .expect("first representable reservation should succeed");
        reserve_working_capital(&mut reservations, account, Money::from_cents(10))
            .expect("projection overflow should saturate rather than fail autonomous planning");
        assert_eq!(
            reservations.get(&account).copied(),
            Some(Money::from_cents(i64::MAX))
        );
    }
}
