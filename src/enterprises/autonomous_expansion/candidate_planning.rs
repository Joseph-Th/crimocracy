//! Pure candidate enumeration and ranking for autonomous enterprise expansion.
//!
//! The parent module owns cadence, authority eligibility, shared working-capital reservation, and
//! canonical establishment. This module is deliberately read-only: it turns one immutable
//! expansion context into deterministic candidate plans without mutating simulation state.

use super::{AutonomousExpansionPlan, ExpansionEconomicsContext, NeighborhoodExpansionAuthority};
use crate::core::id::BusinessId;
use crate::delegation::ResponsibilityScope;
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, resolve_observed_district_case_count,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseProjectionStaffing, can_authority_cover_location, ensure_manager_capacity,
    resolve_enterprise_financial_projection,
};
use crate::enterprises::{EnterpriseKind, EnterpriseLocation};
use crate::finance::Money;
use crate::registry::{EnterpriseDefinition, EnterpriseNetworkMode};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn collect_district_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    district_scopes: &[(usize, ResponsibilityScope)],
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousEnterpriseError> {
    for (authority_rank, scope) in district_scopes.iter().copied() {
        let ResponsibilityScope::Neighborhood(neighborhood) = scope else {
            continue;
        };
        collect_neighborhood_scope_candidates(
            economics,
            kind,
            definition,
            NeighborhoodExpansionAuthority {
                rank: authority_rank,
                scope,
                neighborhood,
            },
            owned_venues,
            candidates,
        )?;
    }
    Ok(())
}

pub(super) fn collect_neighborhood_scope_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    authority: NeighborhoodExpansionAuthority,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousEnterpriseError> {
    if definition.required_business_functions().is_empty() {
        let Some(supporting_businesses) =
            resolve_support_network(definition, owned_venues, None, authority.scope)
        else {
            return Ok(());
        };
        if let Some(candidate) = build_ranked_candidate(
            economics,
            kind,
            authority.rank,
            authority.scope,
            EnterpriseLocation::Neighborhood(authority.neighborhood),
            supporting_businesses,
        )? {
            candidates.push(candidate);
        }
        return Ok(());
    }

    collect_hosted_candidates_in_neighborhood(
        economics,
        kind,
        definition,
        authority,
        owned_venues,
        candidates,
    )
}

fn collect_hosted_candidates_in_neighborhood(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    authority: NeighborhoodExpansionAuthority,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousEnterpriseError> {
    for (business_id, business) in owned_venues {
        if business.neighborhood() != authority.neighborhood
            || !host_satisfies_business_requirements(definition, business)
        {
            continue;
        }
        let Some(supporting_businesses) = resolve_support_network(
            definition,
            owned_venues,
            Some(*business_id),
            authority.scope,
        ) else {
            continue;
        };
        if let Some(candidate) = build_ranked_candidate(
            economics,
            kind,
            authority.rank,
            authority.scope,
            EnterpriseLocation::Business(*business_id),
            supporting_businesses,
        )? {
            candidates.push(candidate);
        }
    }
    Ok(())
}

pub(super) fn collect_business_scope_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    authority_rank: usize,
    business_scopes: &[ResponsibilityScope],
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousEnterpriseError> {
    let requires_host = !definition.required_business_functions().is_empty();
    for scope in business_scopes {
        let ResponsibilityScope::Business(business_id) = scope else {
            continue;
        };
        let Some(business) = owned_venues.get(business_id) else {
            continue;
        };
        if requires_host && !host_satisfies_business_requirements(definition, business) {
            continue;
        }
        let Some(supporting_businesses) =
            resolve_support_network(definition, owned_venues, Some(*business_id), *scope)
        else {
            continue;
        };
        if let Some(candidate) = build_ranked_candidate(
            economics,
            kind,
            authority_rank,
            *scope,
            EnterpriseLocation::Business(*business_id),
            supporting_businesses,
        )? {
            candidates.push(candidate);
        }
    }
    Ok(())
}

fn build_ranked_candidate(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    authority_rank: usize,
    scope: ResponsibilityScope,
    location: EnterpriseLocation,
    supporting_businesses: BTreeSet<BusinessId>,
) -> Result<Option<AutonomousExpansionPlan>, AutonomousEnterpriseError> {
    if economics.occupied_locations.contains(&(kind, location))
        || crate::world::territory_conflict::replacement_suppression(
            economics.state,
            economics.organization,
            kind,
            location,
        )
        .is_some()
    {
        return Ok(None);
    }
    // A gang cannot offer a planned expansion on a district it just agreed
    // to freeze. The production establishment owner independently checks the
    // same rule, so projected cohorts never select an impossible pact-breaking
    // candidate and cannot panic during commit.
    let district = match location {
        EnterpriseLocation::Neighborhood(district) => Some(district),
        EnterpriseLocation::Business(business) => economics
            .state
            .world()
            .get_business(business)
            .map(|venue| venue.neighborhood()),
    };
    if district.is_some_and(|district| {
        crate::world::territory_influence::active_standstill(
            economics.state,
            economics.organization,
            district,
        )
        .is_some()
    }) {
        return Ok(None);
    }
    // Managerial span is an operational constraint for rivals as well as
    // players. Do not propose an enterprise its delegated captain cannot run.
    if ensure_manager_capacity(
        economics.state,
        economics.organization,
        economics.manager,
        crate::enterprises::enterprise_execution::ManagedJob {
            kind,
            location,
            associates: kind.baseline_associates(),
            supporting_businesses: supporting_businesses.len(),
        },
        None,
    )
    .is_err()
    {
        return Ok(None);
    }
    // Autonomous governance cannot create work whose first recurrence is already outside the
    // representable simulation horizon. Direct establishment still reports the typed overflow;
    // the daily AI pass simply treats that terminal rail as no actionable expansion.
    if economics
        .state
        .now()
        .checked_add(economics.registry.get_enterprise(kind).economics().cycle())
        .is_none()
    {
        return Ok(None);
    }
    let observed_active_cases = resolve_observed_district_case_count(
        economics.state,
        economics.observed_district_pressure,
        economics.organization,
        location,
    )?;
    let (required_working_capital, expected_net_cash) = resolve_enterprise_financial_projection(
        economics.registry,
        economics.state,
        kind,
        location,
        supporting_businesses.len(),
        observed_active_cases,
        EnterpriseProjectionStaffing {
            management: economics.management,
            associates: kind.baseline_associates(),
        },
    )?;
    if required_working_capital > economics.available_working_capital
        || expected_net_cash <= Money::ZERO
    {
        return Ok(None);
    }
    Ok(Some(AutonomousExpansionPlan {
        authority_rank,
        expected_net_cash,
        kind,
        scope,
        location,
        supporting_businesses,
        required_working_capital,
    }))
}

pub(super) fn compare_expansion_plans(
    left: &AutonomousExpansionPlan,
    right: &AutonomousExpansionPlan,
) -> std::cmp::Ordering {
    left.authority_rank
        .cmp(&right.authority_rank)
        .then_with(|| compare_expansion_economics(left, right))
}

fn compare_expansion_economics(
    left: &AutonomousExpansionPlan,
    right: &AutonomousExpansionPlan,
) -> std::cmp::Ordering {
    right
        .expected_net_cash
        .cmp(&left.expected_net_cash)
        .then(
            left.required_working_capital
                .cmp(&right.required_working_capital),
        )
        .then(left.kind.cmp(&right.kind))
        .then(left.location.cmp(&right.location))
        .then(left.supporting_businesses.cmp(&right.supporting_businesses))
}

/// Orders the preferred plan of each organization for the phase-wide competition.
/// Internal mandate preferences have already been resolved and cannot grant a rival priority.
pub(super) fn compare_phase_expansion_candidates(
    left_leads_district: bool,
    left: &AutonomousExpansionPlan,
    right_leads_district: bool,
    right: &AutonomousExpansionPlan,
) -> std::cmp::Ordering {
    right_leads_district
        .cmp(&left_leads_district)
        .then_with(|| compare_expansion_economics(left, right))
}

fn host_satisfies_business_requirements(
    definition: &EnterpriseDefinition,
    business: &crate::world::BusinessRecord,
) -> bool {
    definition
        .required_business_functions()
        .iter()
        .all(|function| business.has_function(*function))
}

/// Chooses the smallest supporting-business set that covers the authored network requirements.
/// The host, when any, contributes its functions for free and is never repeated in the support
/// set. Dynamic programming is bounded by the small authored BusinessFunction vocabulary, so
/// exact minimum cardinality is cheap; equal-size solutions use lexicographic business-ID order.
/// This prevents planner heuristics from attaching redundant surcharge-producing businesses.
fn resolve_support_network(
    definition: &EnterpriseDefinition,
    owned_businesses: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    host: Option<BusinessId>,
    scope: ResponsibilityScope,
) -> Option<BTreeSet<BusinessId>> {
    let mut required = definition.required_network_functions().clone();
    if definition.network_mode() == EnterpriseNetworkMode::HostMayContribute
        && let Some(host) = host
        && let Some(business) = owned_businesses.get(&host)
    {
        required.retain(|function| !business.has_function(*function));
    }
    if required.is_empty() {
        return Some(BTreeSet::new());
    }

    let mut best_by_coverage = BTreeMap::from([(BTreeSet::new(), BTreeSet::new())]);
    for (business_id, business) in owned_businesses {
        if Some(*business_id) == host
            || !can_authority_cover_location(
                scope,
                EnterpriseLocation::Business(*business_id),
                business.neighborhood(),
            )
        {
            continue;
        }
        let business_coverage: BTreeSet<_> = required
            .iter()
            .copied()
            .filter(|function| business.has_function(*function))
            .collect();
        if business_coverage.is_empty() {
            continue;
        }

        // Snapshot before considering this business so a state transition cannot reuse the same
        // business twice. At most 2^N coverage states exist for N authored network functions.
        let prior_states: Vec<_> = best_by_coverage
            .iter()
            .map(|(coverage, selected)| (coverage.clone(), selected.clone()))
            .collect();
        for (mut coverage, mut selected) in prior_states {
            let previous_coverage = coverage.len();
            coverage.extend(business_coverage.iter().copied());
            if coverage.len() == previous_coverage {
                continue;
            }
            selected.insert(*business_id);
            let replace = best_by_coverage.get(&coverage).is_none_or(|existing| {
                selected.len() < existing.len()
                    || (selected.len() == existing.len()
                        && selected.iter().cmp(existing.iter()).is_lt())
            });
            if replace {
                best_by_coverage.insert(coverage, selected);
            }
        }
    }
    best_by_coverage.remove(&required)
}
