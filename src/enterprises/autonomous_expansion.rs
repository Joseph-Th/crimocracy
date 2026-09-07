//! Daily delegated-autonomy enterprise expansion for non-player organizations: deterministic
//! candidate selection and canonical establishment through the same validated path a player
//! command uses.

use crate::core::id::{
    BusinessId, EnterpriseId, FinancialAccountId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::delegation::{MandateAuthority, ResponsibilityScope};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, can_authority_cover_location, resolve_current_enterprise_operating_cost,
    validate_establish_enterprise, validate_establish_enterprise_with_openings,
};
use crate::enterprises::{
    ALL_ENTERPRISE_KINDS, EnterpriseDraft, EnterpriseKind, EnterpriseLocation,
};
use crate::finance::finance_system::{FinanceError, validate_open_accounts};
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::world::AutonomyLevel;
use crate::world::territory_influence::resolve_neighborhood_influence;
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutonomousExpansionPlan {
    kind: EnterpriseKind,
    scope: ResponsibilityScope,
    location: EnterpriseLocation,
    supporting_businesses: BTreeSet<BusinessId>,
    required_working_capital: crate::finance::Money,
}

/// Daily delegated-autonomy expansion for organizations other than the player's: every active
/// mandate whose manager holds Delegated or Broad autonomy may open one enterprise per pass
/// inside its territorial scope, through the exact canonical establishment path a player
/// command uses. Selection is deterministic and consumes no randomness: kinds are tried in
/// authored registry order, governed districts keep their influence priority, and equally ranked
/// configurations minimize current operating runway with stable location/support ID tie-breaks.
/// Matched-seed branches therefore observe identical rival growth unless their own actions changed
/// rival-governed state.
///
/// Rival organizations without governed territory (no mandate, a Tight/Guided manager, or no
/// usable cash and settlement accounts) simply do not expand.
pub(crate) fn apply_due_autonomous_enterprises(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<EnterpriseId>, AutonomousExpansionError> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Ok(Vec::new());
    }
    let player_organization = state.player_organization();
    // Active mandates iterate in mandate-id order, so every eligible authority is evaluated
    // in a single stable sequence; the active-mandate index keeps revoked history out of
    // this daily scan.
    let mandates: Vec<_> = state.delegation().active_mandates().cloned().collect();
    let mut established = Vec::new();
    for mandate in mandates {
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
        let Some(available_working_capital) =
            resolve_max_autonomous_working_capital(state, organization)
        else {
            continue;
        };
        let Some(plan) = decide_autonomous_expansion(
            registry,
            state,
            organization,
            &mandate,
            available_working_capital,
        ) else {
            continue;
        };
        // Delegated managers do not open a new racket from token cash. The decision already
        // priced one current cycle of runway using the canonical operating-cost composition,
        // including police burden, support network, and current district heat. Direct player
        // establishment may still accept a deliberately undercapitalized risk. This money is
        // not consumed here because no startup fee exists in authored economics.
        let Some((cash_account, existing_settlement)) = resolve_existing_autonomous_accounts(
            state,
            organization,
            plan.required_working_capital,
        ) else {
            continue;
        };
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
        match existing_settlement {
            Some(settlement_account) => {
                let enterprise =
                    validate_establish_enterprise(registry, state, draft(settlement_account))?
                        .commit(state)?;
                established.push(enterprise);
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
                let enterprise = validate_establish_enterprise_with_openings(
                    registry,
                    state,
                    draft(fresh),
                    openings,
                )?
                .commit(state)?;
                established.push(enterprise);
            }
        }
    }
    Ok(established)
}

/// Read-only deterministic decision over authored kind order and governed-location candidates.
/// Venue requirements and support-network requirements stay distinct: a hosted racket needs one
/// owned business carrying every venue function, while network functions may be assembled from a
/// deterministic minimum-cardinality set of other owned businesses covered by the selected
/// authority scope. This mirrors canonical establishment instead of incorrectly requiring one
/// storefront to embody an entire supply chain or letting a district manager bind remote assets.
fn decide_autonomous_expansion(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
    available_working_capital: crate::finance::Money,
) -> Option<AutonomousExpansionPlan> {
    // District preference is influence-aware, not id order: districts the organization
    // already leads consolidate first, contested or empty districts follow. Ties break on
    // neighborhood id, so the ordering stays deterministic. Tiers are computed once per
    // mandate rather than per comparison.
    let mut district_scopes: Vec<(u8, NeighborhoodId)> = mandate
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
            (u8::from(!leads), id)
        })
        .collect();
    district_scopes.sort_unstable();
    let district_scopes: Vec<ResponsibilityScope> = district_scopes
        .into_iter()
        .map(|(_, id)| ResponsibilityScope::Neighborhood(id))
        .collect();
    let business_scopes: Vec<ResponsibilityScope> = mandate
        .scopes()
        .iter()
        .filter(|scope| matches!(scope, ResponsibilityScope::Business(_)))
        .copied()
        .collect();

    let owned_venues: BTreeMap<BusinessId, &crate::world::BusinessRecord> = state
        .world()
        .businesses_owned_by_organization(organization)
        .map(|business| (business.id(), business))
        .collect();

    for kind in ALL_ENTERPRISE_KINDS {
        let definition = registry.get_enterprise(kind);
        let requires_host = !definition.required_business_functions().is_empty();
        let mut candidates: Vec<(usize, AutonomousExpansionPlan)> = Vec::new();
        for (scope_rank, scope) in district_scopes.iter().copied().enumerate() {
            let ResponsibilityScope::Neighborhood(neighborhood) = scope else {
                continue;
            };
            // Rackets with no authored venue requirement live at district scope. Their support
            // network is still real and is assembled separately from owned infrastructure.
            if !requires_host {
                if let Some(supporting_businesses) =
                    resolve_support_network(definition, &owned_venues, None, scope)
                {
                    let location = EnterpriseLocation::Neighborhood(neighborhood);
                    let Some(required_working_capital) = resolve_current_enterprise_operating_cost(
                        registry,
                        state,
                        kind,
                        location,
                        supporting_businesses.len(),
                    ) else {
                        continue;
                    };
                    if required_working_capital > available_working_capital {
                        continue;
                    }
                    candidates.push((
                        scope_rank,
                        AutonomousExpansionPlan {
                            kind,
                            scope,
                            location,
                            supporting_businesses,
                            required_working_capital,
                        },
                    ));
                }
            } else {
                for (business_id, business) in &owned_venues {
                    if business.neighborhood() != neighborhood
                        || !host_satisfies_business_requirements(definition, business)
                    {
                        continue;
                    }
                    let Some(supporting_businesses) = resolve_support_network(
                        definition,
                        &owned_venues,
                        Some(*business_id),
                        scope,
                    ) else {
                        continue;
                    };
                    let location = EnterpriseLocation::Business(*business_id);
                    let Some(required_working_capital) = resolve_current_enterprise_operating_cost(
                        registry,
                        state,
                        kind,
                        location,
                        supporting_businesses.len(),
                    ) else {
                        continue;
                    };
                    if required_working_capital > available_working_capital {
                        continue;
                    }
                    // The district scope covers venues inside its own neighborhood.
                    candidates.push((
                        scope_rank,
                        AutonomousExpansionPlan {
                            kind,
                            scope,
                            location,
                            supporting_businesses,
                            required_working_capital,
                        },
                    ));
                }
            }
        }
        // A specific-business mandate is less general than any governed district scope, but
        // specific businesses are peers with one another: when only that authority is usable,
        // pick the configuration with the lower recurring runway rather than raw business ID.
        let business_scope_rank = district_scopes.len();
        for scope in &business_scopes {
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
                resolve_support_network(definition, &owned_venues, Some(*business_id), *scope)
            else {
                continue;
            };
            let location = EnterpriseLocation::Business(*business_id);
            let Some(required_working_capital) = resolve_current_enterprise_operating_cost(
                registry,
                state,
                kind,
                location,
                supporting_businesses.len(),
            ) else {
                continue;
            };
            if required_working_capital > available_working_capital {
                continue;
            }
            candidates.push((
                business_scope_rank,
                AutonomousExpansionPlan {
                    kind,
                    scope: *scope,
                    location,
                    supporting_businesses,
                    required_working_capital,
                },
            ));
        }

        // Preserve district-consolidation priority first. Within the same authority rank, choose
        // the cheapest current-cycle configuration; location and support IDs are complete stable
        // tie-breakers. This keeps deterministic selection without making a delegated manager
        // pay avoidable recurring support costs merely because a worse venue has a lower ID.
        if let Some((_, plan)) = candidates
            .into_iter()
            .filter(|(_, plan)| {
                !state
                    .enterprises()
                    .enterprises_at(plan.location)
                    .any(|record| record.kind() == kind)
            })
            .min_by(|(left_rank, left), (right_rank, right)| {
                left_rank
                    .cmp(right_rank)
                    .then(
                        left.required_working_capital
                            .cmp(&right.required_working_capital),
                    )
                    .then(left.location.cmp(&right.location))
                    .then(left.supporting_businesses.cmp(&right.supporting_businesses))
            })
        {
            return Some(plan);
        }
    }
    None
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
) -> Option<crate::finance::Money> {
    state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash
            )
        })
        .map(|account| account.balance())
        .max()
}

/// Resolves the rival's operating accounts read-only: first org-owned street-or-concealed
/// cash in ascending account-id order, plus an unreserved settlement account when one exists.
/// Cash must cover the selected configuration's current one-cycle operating runway. Returns
/// `None` only when no sufficiently funded cash account exists; a missing settlement account is
/// reported as `None` on the second slot so the caller can open one atomically with the
/// establishment it backs.
fn resolve_existing_autonomous_accounts(
    state: &AppState,
    organization: OrganizationId,
    minimum_working_capital: crate::finance::Money,
) -> Option<(FinancialAccountId, Option<FinancialAccountId>)> {
    let owner = FinancialOwner::Organization(organization);
    let mut cash = None;
    let mut settlement = None;
    for account in state.finance().accounts_for(owner) {
        let id = account.id();
        // Exhaustive per repo rule: only sufficiently funded street-or-concealed cash backs an
        // autonomous racket, and only an unreserved settlement account can back its cycle
        // ledger. Account existence alone is not funding.
        match account.kind() {
            AccountKind::StreetCash | AccountKind::ConcealedCash
                if cash.is_none() && account.balance() >= minimum_working_capital =>
            {
                cash = Some(id);
            }
            AccountKind::Settlement
                if settlement.is_none()
                    && state.enterprises().get_by_settlement_account(id).is_none()
                    && state.economy().get_by_settlement_account(id).is_none() =>
            {
                settlement = Some(id);
            }
            AccountKind::StreetCash
            | AccountKind::ConcealedCash
            | AccountKind::Settlement
            | AccountKind::AccountedFunds
            | AccountKind::LegitimateOperating => {}
        }
    }
    Some((cash?, settlement))
}
