//! Daily delegated-autonomy enterprise expansion for non-player organizations: deterministic
//! candidate selection and canonical establishment through the same validated path a player
//! command uses.

use crate::core::id::{
    BusinessId, EnterpriseId, FinancialAccountId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::{MandateAuthority, MandateStatus, ResponsibilityScope};
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{
    EnterpriseDraft, EnterpriseKind, EnterpriseLocation, ALL_ENTERPRISE_KINDS,
};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::world::territory_influence::resolve_neighborhood_influence;
use crate::world::AutonomyLevel;
use std::collections::{BTreeMap, BTreeSet};

/// Daily delegated-autonomy expansion for organizations other than the player's: every active
/// mandate whose manager holds Delegated or Broad autonomy may open one enterprise per pass
/// inside its territorial scope, through the exact canonical establishment path a player
/// command uses. Selection is deterministic and consumes no randomness: kinds are tried in
/// authored registry order, locations in stable id order, so matched-seed branches observe
/// identical rival growth unless their own actions changed rival-governed state.
///
/// Rival organizations without governed territory (no mandate, a Tight/Guided manager, or no
/// usable cash and settlement accounts) simply do not expand.
pub(crate) fn apply_due_autonomous_enterprises(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<EnterpriseId> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Vec::new();
    }
    let player_organization = state.player_organization();
    let mandates: Vec<_> = state
        .delegation()
        .mandates()
        .filter(|mandate| mandate.status() == MandateStatus::Active)
        .cloned()
        .collect();
    let mut established = Vec::new();
    // Records iterate in mandate-id order, so every eligible authority is evaluated in a
    // single stable sequence.
    for mandate in mandates {
        let organization = mandate.organization();
        if Some(organization) == player_organization {
            continue;
        }
        // Posture gate: an outfit whose police fear runs above the authored ceiling keeps
        // its head down for the day. Reputation therefore throttles rival growth, not just
        // how candidates judge it.
        let police_fear = crate::reputation::reputation_system::resolve_score(
            registry,
            &state.reputation,
            organization,
            crate::reputation::AudienceKind::Police,
            crate::reputation::ReputationDimension::Fear,
        );
        if police_fear > registry.reputation().expansion_police_fear_ceiling() {
            continue;
        }
        let manager = mandate.manager();
        let Some(manager_record) = state.world().get_character(manager) else {
            continue;
        };
        if state.legal().active_arrest_for_character(manager).is_some()
            || !matches!(
                manager_record.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
        {
            continue;
        }
        let Some((kind, scope, location)) =
            decide_autonomous_expansion(registry, state, organization, &mandate)
        else {
            continue;
        };
        let Some((cash_account, existing_settlement)) =
            resolve_existing_autonomous_accounts(state, organization)
        else {
            continue;
        };
        let draft = |settlement_account| EnterpriseDraft {
            kind,
            organization,
            authority: MandateAuthority {
                mandate: mandate.id(),
                manager,
                scope,
            },
            location,
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        };
        // A free settlement account establishes directly. When every settlement account is
        // already reserved, a fresh one is opened through the canonical insertion path so
        // governed expansion is never silently blocked by bookkeeping exhaustion — and if
        // the establishment is then rejected, that reservation is removed again through the
        // canonical cleanup so a rejected expansion leaves no orphaned account behind.
        match existing_settlement {
            Some(settlement_account) => {
                establish_autonomous_enterprise(
                    registry,
                    state,
                    draft(settlement_account),
                    &mut established,
                );
            }
            None => {
                let Ok(fresh) = insert_account(
                    state,
                    FinancialAccountDraft {
                        owner: FinancialOwner::Organization(organization),
                        kind: AccountKind::Settlement,
                    },
                ) else {
                    continue;
                };
                if !establish_autonomous_enterprise(registry, state, draft(fresh), &mut established)
                {
                    crate::finance::finance_system::remove_unused_account(state, fresh);
                }
            }
        }
    }
    established
}

/// Validates and commits one autonomous establishment; a failure must not abort the pass.
fn establish_autonomous_enterprise(
    registry: &Registry,
    state: &mut AppState,
    draft: EnterpriseDraft,
    established: &mut Vec<EnterpriseId>,
) -> bool {
    match validate_establish_enterprise(registry, state, draft) {
        // One authority that cannot commit must not abort the rest of the pass.
        Ok(validated) => match validated.commit(state) {
            Ok(enterprise) => {
                established.push(enterprise);
                true
            }
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Read-only first-fit decision over authored kind order and stable location order.
/// Returns the kind, the specific covering scope, and the location to establish at.
fn decide_autonomous_expansion(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
) -> Option<(EnterpriseKind, ResponsibilityScope, EnterpriseLocation)> {
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
        let asset_free = definition.required_business_functions().is_empty()
            && definition.required_network_functions().is_empty();

        let mut candidates: Vec<(ResponsibilityScope, EnterpriseLocation)> = Vec::new();
        for scope in district_scopes.iter().copied() {
            let ResponsibilityScope::Neighborhood(neighborhood) = scope else {
                continue;
            };
            // Asset-free rackets run at the district itself; venue-hosted rackets need an
            // owned venue in the same district whose functions carry every requirement.
            if asset_free {
                candidates.push((scope, EnterpriseLocation::Neighborhood(neighborhood)));
            }
            for (business_id, business) in &owned_venues {
                if business.neighborhood() != neighborhood
                    || !is_valid_host_kind(definition, business)
                {
                    continue;
                }
                // The district scope covers venues inside its own neighborhood.
                candidates.push((scope, EnterpriseLocation::Business(*business_id)));
            }
        }
        for scope in &business_scopes {
            let ResponsibilityScope::Business(business_id) = scope else {
                continue;
            };
            let Some(business) = owned_venues.get(business_id) else {
                continue;
            };
            if is_valid_host_kind(definition, business) {
                candidates.push((*scope, EnterpriseLocation::Business(*business_id)));
            }
        }

        // Skip candidates already occupied by the same kind under the exact rule the
        // canonical establishment path enforces — including suspended rackets, so selection
        // never proposes a draft the owner would reject.
        let open_candidate = candidates.into_iter().find(|(_, location)| {
            !state
                .enterprises()
                .enterprises_at(*location)
                .any(|record| record.kind() == kind)
        });
        if let Some((covering_scope, location)) = open_candidate {
            return Some((kind, covering_scope, location));
        }
    }
    None
}

fn is_valid_host_kind(
    definition: &EnterpriseDefinition,
    business: &crate::world::BusinessRecord,
) -> bool {
    definition
        .required_business_functions()
        .iter()
        .chain(definition.required_network_functions())
        .all(|function| business.has_function(*function))
}

/// Resolves the rival's operating accounts read-only: first org-owned street-or-concealed
/// cash in ascending account-id order, plus an unreserved settlement account when one
/// exists. Returns `None` only when no fundable cash exists; a missing settlement account is
/// reported as `None` on the second slot so the caller can open one atomically with the
/// establishment it backs.
fn resolve_existing_autonomous_accounts(
    state: &AppState,
    organization: OrganizationId,
) -> Option<(FinancialAccountId, Option<FinancialAccountId>)> {
    let owner = FinancialOwner::Organization(organization);
    let mut cash = None;
    let mut settlement = None;
    for account in state.finance().accounts_for(owner) {
        let id = account.id();
        // Exhaustive per repo rule: only street-or-concealed cash funds a racket, and only
        // an unreserved settlement account can back its cycle ledger.
        match account.kind() {
            AccountKind::StreetCash | AccountKind::ConcealedCash if cash.is_none() => {
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
