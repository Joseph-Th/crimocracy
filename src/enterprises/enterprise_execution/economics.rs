//! Enterprise revenue, cost, projection, and historical financial re-derivation.

use super::{EnterpriseError, resolve_location_profile};
use crate::core::id::EnterpriseId;
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::{EnterpriseCycleRecord, EnterpriseKind, EnterpriseLocation};
use crate::finance::Money;
use crate::registry::{EnterpriseEconomicsDefinition, Registry};
use crate::world::{CapabilityKind, NeighborhoodProfile, Rating};

/// Baseline labor is already included in authored operating costs. Additional
/// associates earn $15 per cycle and generate demand-dependent additional gross.
pub const ASSOCIATE_ADDITIONAL_DAILY_WAGE: Money = Money::from_cents(1_500);

/// One hot district carries one police-pressure operating surcharge, regardless
/// of how many cases the authorities open there. Investigations still each
/// generate evidence, detention risks, and further inquiry opportunities. A
/// single surveillance lead must not compound into an unbounded daily cash tax
/// on every racket in the same neighborhood.
const MAX_HEAT_CASE_EQUIVALENTS: u32 = 1;

pub(crate) fn additional_associate_economics(
    kind: EnterpriseKind,
    profile: NeighborhoodProfile,
    associates: u16,
) -> Option<(Money, Money)> {
    let extra = i64::from(associates.checked_sub(kind.baseline_associates())?);
    let economy = profile.economy;
    let per_associate = i64::from(economy.illicit_demand.value()) * 20
        + i64::from(economy.commercial_activity.value()) * 12
        + i64::from(economy.wealth.value()) * 4;
    Some((
        Money::from_cents(per_associate.checked_mul(extra)?),
        ASSOCIATE_ADDITIONAL_DAILY_WAGE.checked_mul(extra)?,
    ))
}

/// Registry-time arithmetic proof for the most expensive and productive legal
/// staffing configuration. Every input is nonnegative when registration calls
/// this, so maximum ratings, staffing, variance and support count bound all
/// runtime compositions. Use the production rules rather than a second model.
pub(crate) fn enterprise_economics_fit_production_arithmetic(
    kind: EnterpriseKind,
    economics: &EnterpriseEconomicsDefinition,
) -> bool {
    use crate::world::{NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile};

    let maximum = Rating::try_new(Rating::MAX).expect("maximum rating is valid");
    let profile = NeighborhoodProfile {
        economy: NeighborhoodEconomyProfile {
            wealth: maximum,
            commercial_activity: maximum,
            illicit_demand: maximum,
        },
        institutions: NeighborhoodInstitutionProfile {
            police_presence: maximum,
        },
    };
    let Some((labor_revenue, labor_cost)) =
        additional_associate_economics(kind, profile, kind.max_associates())
    else {
        return false;
    };
    let maximum_gross = resolve_gross_before_variance_value(economics, profile, Some(maximum))
        .and_then(|gross| gross.checked_add(labor_revenue))
        .and_then(|gross| {
            crate::finance::helpers::apply_basis_point_multiplier(
                gross,
                10_000 + u32::from(economics.gross_variance_basis_points()),
            )
        });
    // Supporting businesses have persistent u32 IDs. This conservative count
    // bounds their cost, while the production heat rule charges only once.
    maximum_gross.is_some()
        && resolve_projected_operating_cost(
            economics,
            profile,
            u32::MAX as usize,
            u32::MAX,
            labor_cost,
        )
        .is_some()
}

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

/// Decodes the persisted street-heat charge into a zero-or-one surcharge equivalent.
/// Positive heat establishes a lower bound, not the number of active investigations.
pub(crate) fn decode_enterprise_investigation_case_count(
    economics: &EnterpriseEconomicsDefinition,
    investigation_heat: Money,
) -> Option<u32> {
    let per_case = economics.heat_surcharge_per_active_case().cents();
    let heat = investigation_heat.cents();
    if per_case < 0 || heat < 0 {
        return None;
    }
    if per_case == 0 {
        return (heat == 0).then_some(0);
    }
    if heat % per_case != 0 {
        return None;
    }
    let count = u32::try_from(heat / per_case).ok()?;
    (count <= MAX_HEAT_CASE_EQUIVALENTS).then_some(count)
}

fn resolve_district_heat(economics: &EnterpriseEconomicsDefinition, active_cases: u32) -> Money {
    if active_cases == 0 {
        Money::ZERO
    } else {
        economics.heat_surcharge_per_active_case()
    }
}

pub(super) fn resolve_operating_cost(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    supporting_business_count: usize,
    active_district_cases: u32,
    enterprise: EnterpriseId,
) -> Result<OperatingCostBreakdown, EnterpriseError> {
    let heat = resolve_district_heat(economics, active_district_cases);
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
    associates: u16,
) -> Result<Money, EnterpriseError> {
    let profile = resolve_location_profile(state, location)?;
    let economics = registry.get_enterprise(kind).economics();
    let projection_overflow = || EnterpriseError::ProjectionArithmeticOverflow { kind, location };
    let (_, labor_cost) = additional_associate_economics(kind, profile, associates)
        .ok_or_else(projection_overflow)?;
    resolve_projected_operating_cost(
        economics,
        profile,
        supporting_business_count,
        observed_active_cases,
        labor_cost,
    )
    .ok_or_else(projection_overflow)
}

fn resolve_projected_operating_cost(
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    supporting_business_count: usize,
    observed_active_cases: u32,
    labor_cost: Money,
) -> Option<Money> {
    let predictable =
        resolve_predictable_operating_cost(economics, profile, supporting_business_count)?;
    let heat = resolve_district_heat(economics, observed_active_cases);
    predictable.checked_add(heat)?.checked_add(labor_cost)
}

/// Zero-variance financial projection for a proposed enterprise under the caller's observed
/// district pressure. Returns `(operating_cost, expected_net_cash)`. This is a read-only decision
/// input, not a persisted forecast: an eventual cycle still draws its authored variance and reads
/// the actual case pressure through the production settlement path.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EnterpriseProjectionStaffing {
    pub management: Option<Rating>,
    pub associates: u16,
}

pub(crate) fn resolve_enterprise_financial_projection(
    registry: &Registry,
    state: &AppState,
    kind: EnterpriseKind,
    location: EnterpriseLocation,
    supporting_business_count: usize,
    observed_active_cases: u32,
    staffing: EnterpriseProjectionStaffing,
) -> Result<(Money, Money), EnterpriseError> {
    let profile = resolve_location_profile(state, location)?;
    let economics = registry.get_enterprise(kind).economics();
    let projection_overflow = || EnterpriseError::ProjectionArithmeticOverflow { kind, location };
    let (labor_revenue, labor_cost) =
        additional_associate_economics(kind, profile, staffing.associates)
            .ok_or_else(projection_overflow)?;
    let gross = resolve_gross_before_variance_value(economics, profile, staffing.management)
        .and_then(|gross| gross.checked_add(labor_revenue))
        .ok_or_else(projection_overflow)?;
    let operating_cost = resolve_projected_operating_cost(
        economics,
        profile,
        supporting_business_count,
        observed_active_cases,
        labor_cost,
    )
    .ok_or_else(projection_overflow)?;
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
    let manager_id = record.authority_for_cycle_at(cycle.occurred_at()).manager;
    let manager = state
        .world
        .get_character(manager_id)
        .ok_or(DelegationError::MissingManager(manager_id))?;
    let (labor_revenue, labor_cost) = additional_associate_economics(
        record.kind(),
        profile,
        record.associates_at(cycle.occurred_at()),
    )
    .ok_or(EnterpriseError::ArithmeticOverflow(record.id()))?;
    let gross_before_variance = resolve_gross_before_variance(
        record.id(),
        economics,
        profile,
        manager.capability(CapabilityKind::Management),
    )?
    .checked_add(labor_revenue)
    .ok_or(EnterpriseError::ArithmeticOverflow(record.id()))?;
    let gross_revenue = resolve_basis_point_variance(record.id(), gross_before_variance, variance)?;
    let heat = cycle.investigation_heat();
    if decode_enterprise_investigation_case_count(economics, heat).is_none() {
        return Err(EnterpriseError::ArithmeticOverflow(record.id()));
    }
    let operating_cost = resolve_operating_cost_with_heat(
        economics,
        profile,
        record.supporting_businesses().len(),
        heat,
        record.id(),
    )?
    .total
    .checked_add(labor_cost)
    .ok_or(EnterpriseError::ArithmeticOverflow(record.id()))?;
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
