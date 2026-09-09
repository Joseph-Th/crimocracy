//! Enterprise revenue, cost, projection, and historical financial re-derivation.

use super::{EnterpriseError, resolve_location_profile};
use crate::core::id::EnterpriseId;
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::{EnterpriseCycleRecord, EnterpriseKind, EnterpriseLocation};
use crate::finance::Money;
use crate::registry::{EnterpriseEconomicsDefinition, Registry};
use crate::world::{CapabilityKind, NeighborhoodProfile, Rating};

pub(super) fn resolve_gross_before_variance(
    enterprise: EnterpriseId,
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    management: Option<Rating>,
) -> Result<Money, EnterpriseError> {
    resolve_gross_before_variance_value(economics, profile, management)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}

/// Enterprise gross at zero variance, independent of an already-persisted enterprise ID.
/// Production settlement and delegated expansion both use this exact composition so a planner
/// cannot rank proposed rackets with economics that differ from the cycle they will actually run.
fn resolve_gross_before_variance_value(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    management: Option<Rating>,
) -> Option<Money> {
    let components = [
        crate::finance::helpers::weighted_rating(
            economics.demand_revenue_per_point(),
            profile.economy.illicit_demand.value(),
        )?,
        crate::finance::helpers::weighted_rating(
            economics.commerce_revenue_per_point(),
            profile.economy.commercial_activity.value(),
        )?,
        crate::finance::helpers::weighted_rating(
            economics.wealth_revenue_per_point(),
            profile.economy.wealth.value(),
        )?,
        match management {
            Some(value) => crate::finance::helpers::weighted_rating(
                economics.management_revenue_per_point(),
                value.value(),
            )?,
            None => Money::ZERO,
        },
    ];
    let mut gross = economics.base_gross();
    for component in components {
        gross = gross.checked_add(component)?;
    }
    Some(gross)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OperatingCostBreakdown {
    pub(super) total: Money,
    /// Portion of `total` caused by active investigations in the enterprise's district.
    pub(super) investigation_heat: Money,
}

pub(super) fn resolve_operating_cost(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    supporting_business_count: usize,
    active_district_cases: u32,
    enterprise: EnterpriseId,
) -> Result<OperatingCostBreakdown, EnterpriseError> {
    let heat = economics
        .heat_surcharge_per_active_case()
        .checked_mul(i64::from(active_district_cases))
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    resolve_operating_cost_with_heat(
        economics,
        profile,
        supporting_business_count,
        heat,
        enterprise,
    )
}

fn resolve_operating_cost_with_heat(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    supporting_business_count: usize,
    investigation_heat: Money,
    enterprise: EnterpriseId,
) -> Result<OperatingCostBreakdown, EnterpriseError> {
    let predictable =
        resolve_predictable_operating_cost(economics, profile, supporting_business_count)
            .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let total = predictable
        .checked_add(investigation_heat)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    Ok(OperatingCostBreakdown {
        total,
        investigation_heat,
    })
}

/// One-cycle operating runway for a proposed enterprise configuration, before any gross is
/// earned. `observed_active_cases` is an explicit decision input owned by the caller so planning
/// cannot silently acquire omniscient access to the live legal graph. The arithmetic remains the
/// same composition used by production settlement once its actual case count is known.
pub(crate) fn resolve_enterprise_operating_cost_projection(
    registry: &Registry,
    state: &AppState,
    kind: EnterpriseKind,
    location: EnterpriseLocation,
    supporting_business_count: usize,
    observed_active_cases: u32,
) -> Result<Money, EnterpriseError> {
    let profile = resolve_location_profile(state, location)?;
    let economics = registry.get_enterprise(kind).economics();
    let projection_overflow = || EnterpriseError::ProjectionArithmeticOverflow { kind, location };
    let predictable =
        resolve_predictable_operating_cost(economics, profile, supporting_business_count)
            .ok_or_else(projection_overflow)?;
    let heat = economics
        .heat_surcharge_per_active_case()
        .checked_mul(i64::from(observed_active_cases))
        .ok_or_else(projection_overflow)?;
    predictable
        .checked_add(heat)
        .ok_or_else(projection_overflow)
}

/// Zero-variance financial projection for a proposed enterprise under the caller's observed
/// district pressure. Returns `(operating_cost, expected_net_cash)`. This is a read-only decision
/// input, not a persisted forecast: an eventual cycle still draws its authored variance and reads
/// the actual case pressure through the production settlement path.
pub(crate) fn resolve_enterprise_financial_projection(
    registry: &Registry,
    state: &AppState,
    kind: EnterpriseKind,
    location: EnterpriseLocation,
    supporting_business_count: usize,
    management: Option<Rating>,
    observed_active_cases: u32,
) -> Result<(Money, Money), EnterpriseError> {
    let profile = resolve_location_profile(state, location)?;
    let economics = registry.get_enterprise(kind).economics();
    let projection_overflow = || EnterpriseError::ProjectionArithmeticOverflow { kind, location };
    let gross = resolve_gross_before_variance_value(economics, profile, management)
        .ok_or_else(projection_overflow)?;
    let operating_cost = resolve_enterprise_operating_cost_projection(
        registry,
        state,
        kind,
        location,
        supporting_business_count,
        observed_active_cases,
    )?;
    let expected_net_cash = gross
        .checked_sub(operating_cost)
        .ok_or_else(projection_overflow)?;
    Ok((operating_cost, expected_net_cash))
}

fn resolve_predictable_operating_cost(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    supporting_business_count: usize,
) -> Option<Money> {
    let police = crate::finance::helpers::weighted_rating(
        economics.police_cost_per_point(),
        profile.institutions.police_presence.value(),
    )?;
    let supporting_business_count = i64::try_from(supporting_business_count).ok()?;
    let support = economics
        .support_surcharge_per_business()
        .checked_mul(supporting_business_count)?;
    economics
        .base_operating_cost()
        .checked_add(police)?
        .checked_add(support)
}

/// Re-derives a persisted enterprise cycle from immutable enterprise/district authorship plus
/// the cycle's frozen variance and street-heat surcharge. Historical validation deliberately
/// does not consult the current active-investigation count because those cases may have closed.
pub(crate) fn resolve_historical_enterprise_cycle_financials(
    registry: &Registry,
    state: &AppState,
    cycle: &EnterpriseCycleRecord,
) -> Result<(Money, Money, Money), EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(cycle.enterprise())
        .ok_or(EnterpriseError::MissingEnterprise(cycle.enterprise()))?;
    let economics = registry.get_enterprise(record.kind()).economics();
    let variance = cycle.variance_basis_points();
    let variance_limit = economics.gross_variance_basis_points();
    if i32::from(variance).unsigned_abs() > u32::from(variance_limit) {
        return Err(EnterpriseError::VarianceOutOfRange {
            basis_points: variance,
            limit: variance_limit,
        });
    }
    let profile = resolve_location_profile(state, record.location())?;
    let manager = state
        .world
        .get_character(record.manager())
        .ok_or(DelegationError::MissingManager(record.manager()))?;
    let gross_before_variance = resolve_gross_before_variance(
        record.id(),
        economics,
        profile,
        manager.capability(CapabilityKind::Management),
    )?;
    let gross_revenue = resolve_basis_point_variance(record.id(), gross_before_variance, variance)?;
    let heat = cycle.investigation_heat();
    let per_case_heat = economics.heat_surcharge_per_active_case().cents();
    if heat.cents() < 0
        || (per_case_heat == 0 && heat != Money::ZERO)
        || (per_case_heat > 0 && heat.cents() % per_case_heat != 0)
    {
        return Err(EnterpriseError::ArithmeticOverflow(record.id()));
    }
    let operating_cost = resolve_operating_cost_with_heat(
        economics,
        profile,
        record.supporting_businesses().len(),
        heat,
        record.id(),
    )?
    .total;
    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(EnterpriseError::ArithmeticOverflow(record.id()))?;
    Ok((gross_revenue, operating_cost, net_cash))
}

pub(super) fn resolve_basis_point_variance(
    enterprise: EnterpriseId,
    amount: Money,
    basis_points: i16,
) -> Result<Money, EnterpriseError> {
    crate::finance::helpers::resolve_basis_point_variance(amount, basis_points)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}
