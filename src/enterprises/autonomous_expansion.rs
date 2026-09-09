//! Daily delegated-autonomy enterprise expansion for non-player organizations: deterministic
//! candidate selection and canonical establishment through the same validated path a player
//! command uses.

use crate::core::id::{
    BusinessId, EnterpriseId, FinancialAccountId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, can_authority_cover_location, enterprise_location_is_occupied,
    resolve_enterprise_financial_projection, resolve_enterprise_operating_cost_projection,
    resolve_location_neighborhood, validate_establish_enterprise,
    validate_establish_enterprise_with_openings,
};
use crate::enterprises::{
    ALL_ENTERPRISE_KINDS, EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseStatus,
};
use crate::finance::finance_system::{FinanceError, validate_open_accounts};
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner, Money};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::world::territory_influence::resolve_neighborhood_influence;
use crate::world::{AutonomyLevel, CapabilityKind, Rating};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum AutonomousExpansionError {
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Enterprise(#[from] EnterpriseError),
    #[error("working-capital reservations overflowed for account {account}")]
    WorkingCapitalOverflow { account: FinancialAccountId },
}

const LED_NEIGHBORHOOD_AUTHORITY_RANK: usize = 0;
const OTHER_NEIGHBORHOOD_AUTHORITY_RANK: usize = 1;
const BUSINESS_AUTHORITY_RANK: usize = 2;
const ENTERPRISE_FUNCTION_LED_AUTHORITY_RANK: usize = 3;
const ENTERPRISE_FUNCTION_OTHER_AUTHORITY_RANK: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutonomousExpansionPlan {
    authority_rank: usize,
    expected_net_cash: Money,
    kind: EnterpriseKind,
    scope: ResponsibilityScope,
    location: EnterpriseLocation,
    supporting_businesses: BTreeSet<BusinessId>,
    required_working_capital: Money,
}

struct ExpansionEconomicsContext<'a> {
    registry: &'a Registry,
    state: &'a AppState,
    organization: OrganizationId,
    observed_district_pressure: &'a ObservedDistrictPressure,
    management: Option<Rating>,
    available_working_capital: Money,
}

type ObservedDistrictPressure = BTreeMap<(OrganizationId, NeighborhoodId), u32>;

#[derive(Clone, Copy)]
struct NeighborhoodExpansionAuthority {
    rank: usize,
    scope: ResponsibilityScope,
    neighborhood: NeighborhoodId,
}

/// Daily delegated-autonomy expansion for organizations other than the player's: every active
/// mandate whose manager holds Delegated or Broad autonomy may open one enterprise per pass
/// inside its delegated responsibility scope, through the exact canonical establishment path a player
/// command uses. Selection is deterministic and consumes no randomness: governed districts keep
/// their influence priority, then viable configurations are ranked by current zero-variance net
/// economics, recurring runway, and stable kind/location/support tie-breaks. A delegated manager
/// will not deliberately open a racket projected to break even or lose money before variance.
/// Matched-seed branches therefore observe identical rival growth unless their own actions changed
/// rival-governed state.
///
/// Rival organizations without usable delegated enterprise authority (no eligible mandate, a
/// Tight/Guided manager, or no usable uncommitted cash and settlement accounts) simply do not
/// expand. Neighborhood and business scopes are preferred over the broad Enterprise function;
/// the function scope remains a real organization-wide fallback rather than inert authority.
pub(crate) fn apply_due_autonomous_enterprises(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<EnterpriseId>, AutonomousExpansionError> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Ok(Vec::new());
    }
    let player_organization = state.player_organization();
    // Working capital is a capacity constraint, not a balance-presence check. Existing active
    // rackets already rely on their cash accounts for one current operating cycle, and each new
    // establishment in this pass makes another claim on that same pool. Keep those commitments
    // as a read-only planning projection so delegated managers cannot multiply-count one dollar
    // of liquidity across several rackets without inventing a second authoritative ledger.
    let observed_district_pressure = resolve_observed_district_pressure(registry, state)?;
    let mut working_capital_reservations =
        resolve_committed_working_capital(registry, state, &observed_district_pressure)?;
    let mandates = resolve_eligible_expansion_mandates(registry, state, player_organization)?;
    let mut established = Vec::new();
    for (organization, organization_mandates) in mandates {
        established.extend(apply_organization_autonomous_expansion(
            registry,
            state,
            organization,
            organization_mandates,
            &observed_district_pressure,
            &mut working_capital_reservations,
        )?);
    }
    Ok(established)
}

fn resolve_eligible_expansion_mandates(
    registry: &Registry,
    state: &AppState,
    player_organization: Option<OrganizationId>,
) -> Result<BTreeMap<OrganizationId, Vec<crate::delegation::MandateRecord>>, AutonomousExpansionError>
{
    let mut by_organization: BTreeMap<OrganizationId, Vec<crate::delegation::MandateRecord>> =
        BTreeMap::new();
    for mandate in state.delegation().active_mandates() {
        let organization = mandate.organization();
        if Some(organization) == player_organization {
            continue;
        }
        // Posture gate: an outfit whose police fear reaches or exceeds the authored ceiling keeps
        // its head down for the day. Reputation therefore throttles rival growth, not just
        // how candidates judge it.
        let police_fear = crate::reputation::reputation_system::resolve_score(
            registry,
            &state.reputation,
            organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        );
        if police_fear >= registry.reputation().expansion_police_fear_ceiling() {
            continue;
        }
        let manager = mandate.manager();
        let manager_record = state
            .world()
            .get_character(manager)
            .ok_or(DelegationError::MissingManager(manager))?;
        if state.legal().active_arrest_for_character(manager).is_some()
            || !matches!(
                manager_record.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
        {
            continue;
        }
        by_organization
            .entry(organization)
            .or_default()
            .push(mandate.clone());
    }
    Ok(by_organization)
}

fn apply_organization_autonomous_expansion(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    mut mandates: Vec<crate::delegation::MandateRecord>,
    observed_district_pressure: &ObservedDistrictPressure,
    working_capital_reservations: &mut BTreeMap<FinancialAccountId, Money>,
) -> Result<Vec<EnterpriseId>, AutonomousExpansionError> {
    let mut established = Vec::new();
    while !mandates.is_empty() {
        let Some(available_working_capital) = resolve_max_autonomous_working_capital(
            state,
            organization,
            working_capital_reservations,
        ) else {
            break;
        };
        let mut selected: Option<(usize, crate::core::id::MandateId, AutonomousExpansionPlan)> =
            None;
        for (index, mandate) in mandates.iter().enumerate() {
            let Some(plan) = decide_autonomous_expansion(
                registry,
                state,
                organization,
                mandate,
                available_working_capital,
                observed_district_pressure,
            )?
            else {
                continue;
            };
            let candidate = (index, mandate.id(), plan);
            let replace = selected
                .as_ref()
                .is_none_or(|(_, selected_mandate, selected_plan)| {
                    compare_expansion_plans(&candidate.2, selected_plan)
                        .then(candidate.1.cmp(selected_mandate))
                        .is_lt()
                });
            if replace {
                selected = Some(candidate);
            }
        }
        let Some((mandate_index, _, plan)) = selected else {
            break;
        };
        let mandate = mandates.remove(mandate_index);
        established.push(commit_autonomous_expansion_plan(
            registry,
            state,
            organization,
            &mandate,
            plan,
            working_capital_reservations,
        )?);
    }
    Ok(established)
}

fn commit_autonomous_expansion_plan(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
    plan: AutonomousExpansionPlan,
    working_capital_reservations: &mut BTreeMap<FinancialAccountId, Money>,
) -> Result<EnterpriseId, AutonomousExpansionError> {
    let manager = mandate.manager();
    // Delegated managers do not open a new racket from token cash. The decision already
    // priced one current cycle of runway using the canonical operating-cost composition,
    // including police burden, support network, and current district heat. Direct player
    // establishment may still accept a deliberately undercapitalized risk. This money is
    // not consumed here because no startup fee exists in authored economics.
    let Some((cash_account, existing_settlement)) = resolve_existing_autonomous_accounts(
        state,
        organization,
        plan.required_working_capital,
        working_capital_reservations,
    ) else {
        unreachable!("selected autonomous plan must retain a sufficiently funded cash account");
    };
    reserve_working_capital(
        working_capital_reservations,
        cash_account,
        plan.required_working_capital,
    )?;
    let draft = |settlement_account| EnterpriseDraft {
        kind: plan.kind,
        organization,
        authority: MandateAuthority {
            mandate: mandate.id(),
            manager,
            scope: plan.scope,
        },
        location: plan.location,
        supporting_businesses: plan.supporting_businesses.clone(),
        cash_account,
        settlement_account,
    };
    // A free settlement account establishes directly. When every settlement account is
    // already reserved, plan a fresh one without mutating state and let the enterprise
    // commit open it only after every establishment dependency is current.
    let enterprise = match existing_settlement {
        Some(settlement_account) => {
            validate_establish_enterprise(registry, state, draft(settlement_account))?
                .commit(state)?
        }
        None => {
            let openings = validate_open_accounts(
                state,
                vec![FinancialAccountDraft {
                    owner: FinancialOwner::Organization(organization),
                    kind: AccountKind::Settlement,
                }],
            )?;
            let fresh = openings
                .account_id(0)
                .expect("one planned settlement account must expose one id");
            validate_establish_enterprise_with_openings(registry, state, draft(fresh), openings)?
                .commit(state)?
        }
    };
    Ok(enterprise)
}

/// Read-only deterministic decision over governed-location and authored enterprise candidates.
/// Venue requirements and support-network requirements stay distinct: a hosted racket needs one
/// owned business carrying every venue function, while network functions may be assembled from a
/// deterministic minimum-cardinality set of other owned businesses covered by the selected
/// authority scope. This mirrors canonical establishment instead of incorrectly requiring one
/// storefront to embody an entire supply chain or letting a district manager bind remote assets.
/// Candidate economics come from the same gross/cost composition as production cycle settlement.
fn decide_autonomous_expansion(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
    available_working_capital: Money,
    observed_district_pressure: &ObservedDistrictPressure,
) -> Result<Option<AutonomousExpansionPlan>, AutonomousExpansionError> {
    let district_scopes = resolve_ranked_district_scopes(state, organization, mandate);
    let business_scopes: Vec<ResponsibilityScope> = mandate
        .scopes()
        .iter()
        .filter(|scope| matches!(scope, ResponsibilityScope::Business(_)))
        .copied()
        .collect();
    let enterprise_function_scope =
        ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    let has_enterprise_function_scope = mandate.scopes().contains(&enterprise_function_scope);

    let owned_venues: BTreeMap<BusinessId, &crate::world::BusinessRecord> = state
        .world()
        .businesses_owned_by_organization(organization)
        .map(|business| (business.id(), business))
        .collect();
    let management = state
        .world()
        .get_character(mandate.manager())
        .expect("active mandate must reference a live manager")
        .capability(CapabilityKind::Management);
    let economics = ExpansionEconomicsContext {
        registry,
        state,
        organization,
        observed_district_pressure,
        management,
        available_working_capital,
    };
    let mut candidates = Vec::new();
    for kind in ALL_ENTERPRISE_KINDS {
        let definition = registry.get_enterprise(kind);
        collect_district_candidates(
            &economics,
            kind,
            definition,
            &district_scopes,
            &owned_venues,
            &mut candidates,
        )?;
        collect_business_scope_candidates(
            &economics,
            kind,
            definition,
            BUSINESS_AUTHORITY_RANK,
            &business_scopes,
            &owned_venues,
            &mut candidates,
        )?;
        if has_enterprise_function_scope {
            collect_enterprise_function_candidates(
                &economics,
                organization,
                kind,
                definition,
                enterprise_function_scope,
                &owned_venues,
                &mut candidates,
            )?;
        }
    }

    Ok(candidates.into_iter().min_by(compare_expansion_plans))
}

fn resolve_ranked_district_scopes(
    state: &AppState,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
) -> Vec<(usize, ResponsibilityScope)> {
    // District preference is influence-aware, not raw id order: districts the organization
    // already leads form the first authority tier, contested or empty districts form the second.
    // IDs only stabilize iteration inside a tier; economics decides among equally preferred
    // districts later instead of accidentally turning creation order into strategy.
    let mut districts: Vec<(usize, NeighborhoodId)> = mandate
        .scopes()
        .iter()
        .filter_map(|scope| match scope {
            ResponsibilityScope::Neighborhood(id) => Some(*id),
            ResponsibilityScope::Business(_) | ResponsibilityScope::Function(_) => None,
        })
        .map(|id| {
            let leads = resolve_neighborhood_influence(state, id)
                .expect("mandate scopes reference live neighborhoods")
                .economic_leader()
                .is_some_and(|leader| leader == organization);
            (
                if leads {
                    LED_NEIGHBORHOOD_AUTHORITY_RANK
                } else {
                    OTHER_NEIGHBORHOOD_AUTHORITY_RANK
                },
                id,
            )
        })
        .collect();
    districts.sort_unstable();
    districts
        .into_iter()
        .map(|(rank, id)| (rank, ResponsibilityScope::Neighborhood(id)))
        .collect()
}

fn collect_enterprise_function_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    organization: OrganizationId,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    scope: ResponsibilityScope,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousExpansionError> {
    for neighborhood in economics.state.world().neighborhoods() {
        let neighborhood_id = neighborhood.id();
        let leads = resolve_neighborhood_influence(economics.state, neighborhood_id)
            .expect("world neighborhood must resolve for influence")
            .economic_leader()
            .is_some_and(|leader| leader == organization);
        let authority_rank = if leads {
            ENTERPRISE_FUNCTION_LED_AUTHORITY_RANK
        } else {
            ENTERPRISE_FUNCTION_OTHER_AUTHORITY_RANK
        };
        collect_neighborhood_scope_candidates(
            economics,
            kind,
            definition,
            NeighborhoodExpansionAuthority {
                rank: authority_rank,
                scope,
                neighborhood: neighborhood_id,
            },
            owned_venues,
            candidates,
        )?;
    }
    Ok(())
}

fn collect_district_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    district_scopes: &[(usize, ResponsibilityScope)],
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousExpansionError> {
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

fn collect_neighborhood_scope_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    authority: NeighborhoodExpansionAuthority,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousExpansionError> {
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
) -> Result<(), AutonomousExpansionError> {
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

fn collect_business_scope_candidates(
    economics: &ExpansionEconomicsContext<'_>,
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    authority_rank: usize,
    business_scopes: &[ResponsibilityScope],
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousExpansionError> {
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
) -> Result<Option<AutonomousExpansionPlan>, AutonomousExpansionError> {
    if enterprise_location_is_occupied(economics.state, kind, location) {
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
        economics.management,
        observed_active_cases,
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

fn compare_expansion_plans(
    left: &AutonomousExpansionPlan,
    right: &AutonomousExpansionPlan,
) -> std::cmp::Ordering {
    left.authority_rank
        .cmp(&right.authority_rank)
        .then_with(|| right.expected_net_cash.cmp(&left.expected_net_cash))
        .then(
            left.required_working_capital
                .cmp(&right.required_working_capital),
        )
        .then(left.kind.cmp(&right.kind))
        .then(left.location.cmp(&right.location))
        .then(left.supporting_businesses.cmp(&right.supporting_businesses))
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
/// set. Dynamic programming is bounded by the small authored `BusinessFunction` vocabulary, so
/// exact minimum cardinality is cheap; equal-size solutions use lexicographic business-ID order.
/// This prevents planner heuristics from attaching redundant surcharge-producing businesses.
fn resolve_support_network(
    definition: &EnterpriseDefinition,
    owned_businesses: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    host: Option<BusinessId>,
    scope: ResponsibilityScope,
) -> Option<BTreeSet<BusinessId>> {
    let mut required = definition.required_network_functions().clone();
    if let Some(host) = host
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

fn resolve_max_autonomous_working_capital(
    state: &AppState,
    organization: OrganizationId,
    reservations: &BTreeMap<FinancialAccountId, Money>,
) -> Option<Money> {
    state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash
            )
        })
        .map(|account| available_working_capital(account, reservations))
        .max()
}

fn resolve_committed_working_capital(
    registry: &Registry,
    state: &AppState,
    observed_district_pressure: &ObservedDistrictPressure,
) -> Result<BTreeMap<FinancialAccountId, Money>, AutonomousExpansionError> {
    let mut reservations = BTreeMap::new();
    for enterprise in state.enterprises().enterprises() {
        if enterprise.status() != EnterpriseStatus::Active {
            continue;
        }
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
/// The autonomous planner must not inspect live investigations: an unseen case is uncertainty,
/// not free foresight. Once one of the organization's enterprises settles in the district, its
/// recorded street-heat surcharge becomes legitimate operating history for later planning. The
/// complete map is derived once per daily pass so candidate evaluation remains linear in world
/// history rather than rescanning an organization's rackets for every proposed location.
fn resolve_observed_district_pressure(
    registry: &Registry,
    state: &AppState,
) -> Result<ObservedDistrictPressure, AutonomousExpansionError> {
    let mut latest_observations = BTreeMap::new();
    for enterprise in state.enterprises().enterprises() {
        let Some(cycle) = state.enterprises().latest_cycle(enterprise.id()) else {
            continue;
        };
        let per_case = registry
            .get_enterprise(enterprise.kind())
            .economics()
            .heat_surcharge_per_active_case()
            .cents();
        if per_case <= 0 {
            continue;
        }
        let heat = cycle.investigation_heat().cents();
        debug_assert!(heat >= 0 && heat % per_case == 0);
        let inferred = u32::try_from(heat / per_case).unwrap_or(u32::MAX);
        let key = (cycle.occurred_at(), cycle.id());
        let neighborhood = resolve_location_neighborhood(state, enterprise.location())?;
        let observation = latest_observations
            .entry((enterprise.organization(), neighborhood))
            .or_insert((key, inferred));
        if key > observation.0 {
            *observation = (key, inferred);
        }
    }
    let maximum_age = u64::from(registry.legal().cold_case_window().as_minutes());
    Ok(latest_observations
        .into_iter()
        .filter_map(|(district, ((observed_at, _), count))| {
            let age = state
                .now()
                .as_minutes()
                .checked_sub(observed_at.as_minutes())
                .expect("persisted enterprise cycles cannot occur in the future");
            // Legal cold-case decay shelves originated cases once a complete inactivity window
            // has elapsed, and that phase runs before delegated expansion on the same tick.
            // Drop an equally old operating observation here as well so planning cannot retain
            // pressure the legal owner has already declared cold.
            (age < maximum_age).then_some((district, count))
        })
        .collect())
}

fn resolve_observed_district_case_count(
    state: &AppState,
    observed_district_pressure: &ObservedDistrictPressure,
    organization: OrganizationId,
    location: EnterpriseLocation,
) -> Result<u32, AutonomousExpansionError> {
    let neighborhood = resolve_location_neighborhood(state, location)?;
    Ok(observed_district_pressure
        .get(&(organization, neighborhood))
        .copied()
        .unwrap_or(0))
}

fn reserve_working_capital(
    reservations: &mut BTreeMap<FinancialAccountId, Money>,
    account: FinancialAccountId,
    amount: Money,
) -> Result<(), AutonomousExpansionError> {
    let current = reservations.get(&account).copied().unwrap_or(Money::ZERO);
    let reserved = current
        .checked_add(amount)
        .ok_or(AutonomousExpansionError::WorkingCapitalOverflow { account })?;
    reservations.insert(account, reserved);
    Ok(())
}

fn available_working_capital(
    account: &crate::finance::FinancialAccountRecord,
    reservations: &BTreeMap<FinancialAccountId, Money>,
) -> Money {
    let reserved = reservations
        .get(&account.id())
        .copied()
        .unwrap_or(Money::ZERO);
    if account.balance() <= reserved {
        return Money::ZERO;
    }
    account
        .balance()
        .checked_sub(reserved)
        .expect("positive balance above a nonnegative reservation must subtract safely")
}

/// Resolves the rival's operating accounts read-only: the smallest sufficiently funded
/// street-or-concealed cash pool, with account ID as the stable tie-breaker, plus an unreserved
/// settlement account when one exists. Best-fit cash preserves larger pools for later plans
/// instead of letting account creation order strand otherwise usable working capital. Cash must
/// cover the selected configuration's current one-cycle operating runway. Returns
/// `None` only when no sufficiently funded cash account exists; a missing settlement account is
/// reported as `None` on the second slot so the caller can open one atomically with the
/// establishment it backs.
fn resolve_existing_autonomous_accounts(
    state: &AppState,
    organization: OrganizationId,
    minimum_working_capital: Money,
    reservations: &BTreeMap<FinancialAccountId, Money>,
) -> Option<(FinancialAccountId, Option<FinancialAccountId>)> {
    let owner = FinancialOwner::Organization(organization);
    let mut cash: Option<(Money, FinancialAccountId)> = None;
    let mut settlement = None;
    for account in state.finance().accounts_for(owner) {
        let id = account.id();
        // Exhaustive per repo rule: only sufficiently funded street-or-concealed cash backs an
        // autonomous racket, and only an unreserved settlement account can back its cycle
        // ledger. Account existence alone is not funding.
        match account.kind() {
            AccountKind::StreetCash | AccountKind::ConcealedCash => {
                let available = available_working_capital(account, reservations);
                if available >= minimum_working_capital
                    && cash.is_none_or(|current| (available, id) < current)
                {
                    cash = Some((available, id));
                }
            }
            AccountKind::Settlement
                if settlement.is_none()
                    && state.enterprises().get_by_settlement_account(id).is_none()
                    && state.economy().get_by_settlement_account(id).is_none() =>
            {
                settlement = Some(id);
            }
            AccountKind::Settlement
            | AccountKind::AccountedFunds
            | AccountKind::LegitimateOperating => {}
        }
    }
    Some((cash?.1, settlement))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::finance::finance_system::{insert_account, validate_record_transaction};
    use crate::finance::{LedgerPosting, LedgerTransactionDraft};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};

    #[test]
    fn autonomous_cash_selection_uses_smallest_sufficient_pool() {
        let registry = build_registry();
        let mut state = AppState::new(0xCA55_1932);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Best Fit Treasury".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let large = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
            },
        )
        .expect("large cash account should validate");
        let small = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::ConcealedCash,
            },
        )
        .expect("small cash account should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
            },
        )
        .expect("settlement account should validate");
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Fund split autonomous treasury".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-16_000),
                    },
                    LedgerPosting {
                        account: large,
                        amount: Money::from_cents(10_000),
                    },
                    LedgerPosting {
                        account: small,
                        amount: Money::from_cents(6_000),
                    },
                ],
                authorization: None,
            },
        )
        .expect("treasury funding should validate")
        .commit(&mut state)
        .expect("treasury funding should commit");

        let selected = resolve_existing_autonomous_accounts(
            &state,
            organization,
            Money::from_cents(6_000),
            &BTreeMap::new(),
        )
        .expect("one cash pool should satisfy the requested runway");
        assert_eq!(
            selected.0, small,
            "best-fit allocation must preserve the larger pool for a later larger runway"
        );
        assert_eq!(selected.1, Some(settlement));
        validate_invariants(&state);
    }
}
