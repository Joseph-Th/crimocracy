//! Read-only enterprise cycle planning: economics, legal-pressure conversion, and recurrence snapshots.

use super::economics::{
    resolve_basis_point_variance, resolve_gross_before_variance, resolve_operating_cost,
};
use super::support::{
    build_enforcement_incident_draft, count_district_originated_cases,
    has_active_enterprise_inquiry, resolve_location_neighborhood, resolve_location_profile,
    snapshot_supporting_business_versions, validate_enterprise_accounts,
    validate_enterprise_business_dependencies, validate_enterprise_environment,
};
use super::{
    EnterpriseCycleAccounts, EnterpriseCycleEconomics, EnterpriseCyclePlan,
    EnterpriseCycleRandomness, EnterpriseCycleSnapshot, EnterpriseError,
};
use crate::core::attention::AttentionClass;
use crate::core::id::EnterpriseId;
use crate::core::state::AppState;
use crate::delegation::delegation_system::resolve_mandate_authority;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::Money;
use crate::legal::jurisdiction_system::resolve_case_intake_authority_snapshot;
use crate::registry::Registry;
use crate::world::CapabilityKind;

pub fn decide_enterprise_cycle(
    registry: &Registry,
    state: &AppState,
    enterprise: EnterpriseId,
    randomness: EnterpriseCycleRandomness,
) -> Result<EnterpriseCyclePlan, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() != EnterpriseStatus::Active {
        return Err(EnterpriseError::EnterpriseNotActive(enterprise));
    }
    let Some(due_at) = record.next_cycle_at() else {
        return Err(EnterpriseError::SimulationTimeOverflow);
    };
    if state.now() < due_at {
        return Err(EnterpriseError::CycleNotDue { enterprise, due_at });
    }

    let definition = registry.get_enterprise(record.kind());
    let variance_basis_points = randomness.variance_basis_points();
    let variance_limit = definition.economics().gross_variance_basis_points();
    if i32::from(variance_basis_points).unsigned_abs() > u32::from(variance_limit) {
        return Err(EnterpriseError::VarianceOutOfRange {
            basis_points: variance_basis_points,
            limit: variance_limit,
        });
    }
    let authority = resolve_mandate_authority(state, record.authority())?;
    validate_enterprise_environment(
        state,
        record.organization(),
        record.authority(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_business_dependencies(
        definition,
        state,
        record.organization(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_accounts(
        state,
        record.organization(),
        record.cash_account(),
        record.settlement_account(),
        Some(record.id()),
    )?;

    let neighborhood = resolve_location_profile(state, record.location())?;
    let district = resolve_location_neighborhood(state, record.location())?;
    // Sustained originated casework in the racket's district is one shared pressure input for
    // both street-heat cost and enforcement-attention probability.
    let active_district_cases = count_district_originated_cases(state, district);
    let manager = state
        .world
        .get_character(record.manager())
        .expect("resolved enterprise authority manager must exist");
    let economics = definition.economics();
    let gross_before_variance = resolve_gross_before_variance(
        enterprise,
        economics,
        neighborhood,
        manager.capability(CapabilityKind::Management),
    )?;
    let gross_revenue =
        resolve_basis_point_variance(enterprise, gross_before_variance, variance_basis_points)?;
    let cost = resolve_operating_cost(
        economics,
        neighborhood,
        record.supporting_businesses().len(),
        active_district_cases,
        enterprise,
    )?;
    let operating_cost = cost.total;

    // Existing dedicated inquiry suppresses another concurrent inquiry. A zero-pressure district
    // naturally produces a zero chance, so the half-open roll comparison remains the sole gate.
    let enforcement_chance_basis_points = u32::try_from(
        (u64::from(economics.enforcement_attention_basis_points_per_active_case())
            * u64::from(active_district_cases))
        .min(10_000),
    )
    .expect("vice chance is explicitly capped to basis-point range");
    let had_active_enterprise_inquiry = has_active_enterprise_inquiry(state, enterprise);
    let enforcement_roll_hits = !had_active_enterprise_inquiry
        && u32::from(randomness.enforcement_attention_roll()) < enforcement_chance_basis_points;
    let enforcement_authority =
        enforcement_roll_hits.then(|| resolve_case_intake_authority_snapshot(state, district));
    let enforcement_incident = enforcement_authority.and_then(|authority| {
        authority.organization.map(|owner| {
            build_enforcement_incident_draft(state, enterprise, record, owner, state.now())
        })
    });

    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let previous_heat = latest_cycle_investigation_heat(state, enterprise);
    let heat_reportable =
        enterprise_heat_change_is_reportable(previous_heat, cost.investigation_heat);
    let variance_notable = i32::from(variance_basis_points).unsigned_abs()
        >= u32::from(economics.notable_variance_basis_points());
    let attention = if variance_notable
        || net_cash < Money::ZERO
        || enforcement_incident.is_some()
        || heat_reportable
    {
        AttentionClass::Notable
    } else {
        AttentionClass::Routine
    };
    let trailing_losing_cycles = count_trailing_losing_cycles(
        state,
        enterprise,
        economics.losing_cycles_before_suspension(),
    );
    let suspends_after_settlement = net_cash < Money::ZERO
        && trailing_losing_cycles + 1 >= u32::from(economics.losing_cycles_before_suspension());

    let supporting_business_versions =
        snapshot_supporting_business_versions(state, record.supporting_businesses())?;
    let host_business_version = match record.location() {
        EnterpriseLocation::Business(business_id) => {
            let business = state
                .world
                .get_business(business_id)
                .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
            Some((business_id, business.version()))
        }
        EnterpriseLocation::Neighborhood(_) => None,
    };
    Ok(EnterpriseCyclePlan {
        snapshot: EnterpriseCycleSnapshot {
            enterprise,
            expected_enterprise_version: record.version(),
            authority,
            occurred_at: state.now(),
            // Re-anchor after delayed settlement instead of paying missed cycles in a burst.
            next_cycle_at: state.now().checked_add(economics.cycle()),
            suspends_after_settlement,
            supporting_business_versions,
            host_business_version,
            active_district_cases,
            had_active_enterprise_inquiry,
        },
        economics: EnterpriseCycleEconomics {
            gross_revenue,
            operating_cost,
            net_cash,
            variance_basis_points,
            investigation_heat: cost.investigation_heat,
            previous_investigation_heat: previous_heat,
            attention,
        },
        accounts: EnterpriseCycleAccounts {
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
        },
        enforcement_incident,
        enforcement_authority,
    })
}

fn latest_cycle_investigation_heat(state: &AppState, enterprise: EnterpriseId) -> Option<Money> {
    state
        .enterprises
        .latest_cycle(enterprise)
        .map(|cycle| cycle.investigation_heat())
}

/// One owner for whether a heat transition deserves a fresh manager report. The first positive
/// surcharge is new information, any positive-level change is new information, and recovery to
/// zero is worth surfacing. Never-hot zero and unchanged heat stay routine.
pub(crate) fn enterprise_heat_change_is_reportable(
    previous_heat: Option<Money>,
    current_heat: Money,
) -> bool {
    previous_heat != Some(current_heat)
        && (current_heat > Money::ZERO || previous_heat.is_some_and(|heat| heat > Money::ZERO))
}

fn count_trailing_losing_cycles(state: &AppState, enterprise: EnterpriseId, limit: u8) -> u32 {
    let anchor = state
        .enterprises
        .get_enterprise(enterprise)
        .and_then(|record| record.loss_streak_anchor());
    crate::finance::helpers::count_trailing_losing_cycles(
        state
            .enterprises
            .cycles_for(enterprise)
            .rev()
            .take(usize::from(limit)),
        |cycle| cycle.occurred_at(),
        |cycle| cycle.net_cash(),
        anchor,
        limit,
    )
}
