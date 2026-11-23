//! Daily delegated-autonomy enterprise expansion for non-player organizations: deterministic
//! candidate selection and canonical establishment through the same validated path a player
//! command uses.

mod candidate_planning;
mod cohort_planning;

use crate::core::id::{
    BusinessId, CharacterId, EnterpriseId, FinancialAccountId, IdKind, MandateId, NeighborhoodId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, ObservedDistrictPressure, available_working_capital,
};
use crate::enterprises::enterprise_execution::{
    validate_establish_enterprise, validate_establish_enterprise_with_openings,
};
use crate::enterprises::{
    ALL_ENTERPRISE_KINDS, EnterpriseDraft, EnterpriseKind, EnterpriseLocation,
};
use crate::finance::finance_system::validate_open_accounts;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner, Money};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::world::{CapabilityKind, Rating};
use candidate_planning::{
    collect_business_scope_candidates, collect_district_candidates,
    collect_neighborhood_scope_candidates, compare_expansion_plans,
};
use cohort_planning::{
    AutonomousExpansionCohort, PlannedAutonomousExpansionAction, PlannedSettlementAccount,
    plan_due_autonomous_expansions,
};
use std::collections::{BTreeMap, BTreeSet};

const UNGUARDED_CRIMINAL_COMMERCE_AUTHORITY_RANK: usize = 0;
const LEAD_NEIGHBORHOOD_AUTHORITY_RANK: usize = 1;
const OTHER_NEIGHBORHOOD_AUTHORITY_RANK: usize = 2;
const BUSINESS_AUTHORITY_RANK: usize = 3;
const ENTERPRISE_FUNCTION_LEAD_AUTHORITY_RANK: usize = 4;
const ENTERPRISE_FUNCTION_OTHER_AUTHORITY_RANK: usize = 5;

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
    manager: CharacterId,
    observed_district_pressure: &'a ObservedDistrictPressure,
    occupied_locations: &'a BTreeSet<(EnterpriseKind, EnterpriseLocation)>,
    management: Option<Rating>,
    available_working_capital: Money,
}

struct ExpansionIterationContext<'a> {
    registry: &'a Registry,
    state: &'a AppState,
    observed_district_pressure: &'a ObservedDistrictPressure,
    district_leaders: &'a BTreeMap<NeighborhoodId, Option<OrganizationId>>,
}

#[derive(Clone, Copy)]
struct NeighborhoodExpansionAuthority {
    rank: usize,
    scope: ResponsibilityScope,
    neighborhood: NeighborhoodId,
}

struct EnterpriseFunctionDistricts<'a> {
    leaders: &'a BTreeMap<NeighborhoodId, Option<OrganizationId>>,
    vacated_commerce: &'a BTreeSet<NeighborhoodId>,
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
/// Same deterministic expansion pass, excluding mandates that already spent their daily
/// enterprise-governance action on lifecycle maintenance.
pub(crate) fn apply_due_autonomous_enterprises_excluding(
    registry: &Registry,
    state: &mut AppState,
    excluded_mandates: &BTreeSet<MandateId>,
) -> Result<Vec<EnterpriseId>, AutonomousEnterpriseError> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Ok(Vec::new());
    }
    let cohort = plan_due_autonomous_expansions(registry, state, excluded_mandates)?;
    if state
        .ids
        .reserve(IdKind::Enterprise, cohort.enterprise_count())
        .is_err()
    {
        return Ok(Vec::new());
    }
    if state
        .ids
        .reserve(IdKind::FinancialAccount, cohort.fresh_settlement_accounts())
        .is_err()
    {
        return Ok(Vec::new());
    }
    Ok(cohort.commit(registry, state))
}

impl AutonomousExpansionCohort {
    fn commit(self, registry: &Registry, state: &mut AppState) -> Vec<EnterpriseId> {
        self.actions
            .into_iter()
            .map(|action| action.commit_preflighted(registry, state))
            .collect()
    }
}

impl PlannedAutonomousExpansionAction {
    fn commit_preflighted(self, registry: &Registry, state: &mut AppState) -> EnterpriseId {
        let manager = self.mandate.manager();
        let draft = |settlement_account| EnterpriseDraft {
            kind: self.plan.kind,
            organization: self.organization,
            authority: MandateAuthority {
                mandate: self.mandate.id(),
                manager,
                scope: self.plan.scope,
            },
            location: self.plan.location,
            supporting_businesses: self.plan.supporting_businesses.clone(),
            cash_account: self.cash_account,
            settlement_account,
        };
        match self.settlement {
            PlannedSettlementAccount::Existing(settlement_account) => {
                validate_establish_enterprise(registry, state, draft(settlement_account))
                    .expect("read-only autonomous cohort plan must remain establishment-valid")
                    .commit(state)
                    .expect("enterprise ID capacity was preflighted for the autonomous cohort")
            }
            PlannedSettlementAccount::Fresh => {
                let openings = validate_open_accounts(
                    state,
                    vec![FinancialAccountDraft {
                        owner: FinancialOwner::Organization(self.organization),
                        kind: AccountKind::Settlement,
                    }],
                )
                .expect("fresh settlement-account capacity was preflighted for the cohort");
                let settlement_account = openings
                    .account_id(0)
                    .expect("one planned settlement account must expose one id");
                validate_establish_enterprise_with_openings(
                    registry,
                    state,
                    draft(settlement_account),
                    openings,
                )
                .expect("read-only autonomous cohort plan must remain establishment-valid")
                .commit(state)
                .expect("cohort ID capacity makes composite establishment infallible")
            }
        }
    }
}

/// Read-only deterministic decision over governed-location and authored enterprise candidates.
/// Venue requirements and support-network requirements stay distinct: a hosted racket needs one
/// owned business carrying every venue function, while network functions may be assembled from a
/// deterministic minimum-cardinality set of other owned businesses covered by the selected
/// authority scope. This mirrors canonical establishment instead of incorrectly requiring one
/// storefront to embody an entire supply chain or letting a district manager bind remote assets.
/// Candidate economics come from the same gross/cost composition as production cycle settlement.
fn decide_autonomous_expansion(
    iteration: &ExpansionIterationContext<'_>,
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
    available_working_capital: Money,
    occupied_locations: &BTreeSet<(EnterpriseKind, EnterpriseLocation)>,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
) -> Result<Option<AutonomousExpansionPlan>, AutonomousEnterpriseError> {
    let district_scopes =
        resolve_ranked_district_scopes(organization, mandate, iteration.district_leaders);
    let business_scopes: Vec<ResponsibilityScope> = mandate
        .scopes()
        .iter()
        .filter(|scope| matches!(scope, ResponsibilityScope::Business(_)))
        .copied()
        .collect();
    let enterprise_function_scope =
        ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    let has_enterprise_function_scope = mandate.scopes().contains(&enterprise_function_scope);
    let vacated_commercial_districts = if has_enterprise_function_scope {
        iteration
            .district_leaders
            .keys()
            .copied()
            .filter(|district| {
                unguarded_criminal_commerce(iteration.state, organization, *district)
            })
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };

    let management = iteration
        .state
        .world()
        .get_character(mandate.manager())
        .expect("active mandate must reference a live manager")
        .capability(CapabilityKind::Management);
    let economics = ExpansionEconomicsContext {
        registry: iteration.registry,
        state: iteration.state,
        organization,
        manager: mandate.manager(),
        observed_district_pressure: iteration.observed_district_pressure,
        occupied_locations,
        management,
        available_working_capital,
    };
    let mut candidates = Vec::new();
    for kind in ALL_ENTERPRISE_KINDS {
        let definition = iteration.registry.get_enterprise(kind);
        collect_district_candidates(
            &economics,
            kind,
            definition,
            &district_scopes,
            owned_venues,
            &mut candidates,
        )?;
        collect_business_scope_candidates(
            &economics,
            kind,
            definition,
            BUSINESS_AUTHORITY_RANK,
            &business_scopes,
            owned_venues,
            &mut candidates,
        )?;
        if has_enterprise_function_scope {
            collect_enterprise_function_candidates(
                &economics,
                kind,
                definition,
                enterprise_function_scope,
                owned_venues,
                EnterpriseFunctionDistricts {
                    leaders: iteration.district_leaders,
                    vacated_commerce: &vacated_commercial_districts,
                },
                &mut candidates,
            )?;
        }
    }

    Ok(candidates.into_iter().min_by(compare_expansion_plans))
}

fn resolve_ranked_district_scopes(
    organization: OrganizationId,
    mandate: &crate::delegation::MandateRecord,
    district_leaders: &BTreeMap<NeighborhoodId, Option<OrganizationId>>,
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
            let leader = district_leaders
                .get(&id)
                .copied()
                .expect("mandate scopes reference live neighborhoods");
            let leads = leader == Some(organization);
            (
                if leads {
                    LEAD_NEIGHBORHOOD_AUTHORITY_RANK
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
    kind: EnterpriseKind,
    definition: &EnterpriseDefinition,
    scope: ResponsibilityScope,
    owned_venues: &BTreeMap<BusinessId, &crate::world::BusinessRecord>,
    districts: EnterpriseFunctionDistricts<'_>,
    candidates: &mut Vec<AutonomousExpansionPlan>,
) -> Result<(), AutonomousEnterpriseError> {
    for (neighborhood_id, leader) in districts.leaders {
        let leads = *leader == Some(economics.organization);
        // A commercial portfolio with no active local racket is an opening for an
        // organized rival. Under delegated citywide authority, pursue a financially
        // viable foothold there before adding another front in an already held ward.
        let authority_rank = if districts.vacated_commerce.contains(neighborhood_id) {
            UNGUARDED_CRIMINAL_COMMERCE_AUTHORITY_RANK
        } else if leads {
            ENTERPRISE_FUNCTION_LEAD_AUTHORITY_RANK
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
                neighborhood: *neighborhood_id,
            },
            owned_venues,
            candidates,
        )?;
    }
    Ok(())
}

fn unguarded_criminal_commerce(
    state: &AppState,
    attacker: OrganizationId,
    district: NeighborhoodId,
) -> bool {
    state
        .world()
        .public_business_directory()
        .filter(|(_, _, _, neighborhood)| *neighborhood == district)
        .any(|(business_id, _, _, _)| {
            let Some(business) = state.world().get_business(business_id) else {
                return false;
            };
            let crate::world::BusinessOwner::Organization(owner) = business.owner() else {
                return false;
            };
            owner != attacker
                && state
                    .world()
                    .get_organization(owner)
                    .is_some_and(|org| org.kind() == crate::world::OrganizationKind::Criminal)
                && state.economy().get_business_economy(business_id).is_some_and(|books| {
                    books.status() == crate::economy::BusinessOperatingStatus::Active
                })
                // This is a genuine vacated criminal foothold, not simply a shop
                // purchased in a district where its owner never operated a racket.
                && state.enterprises().enterprises_for_organization(owner).any(|racket| {
                    racket.status() != crate::enterprises::EnterpriseStatus::Active
                        && crate::world::territory_conflict::district_of_racket(
                            state,
                            racket.location(),
                        ) == Some(district)
                })
                && !state.enterprises().active_for_organization(owner).any(|racket| {
                    crate::world::territory_conflict::district_of_racket(
                        state,
                        racket.location(),
                    ) == Some(district)
                })
        })
}

/// One organization's currently usable business network for an immutable expansion selection
/// pass. Hosting and support both require live organization-owned businesses, so one snapshot can
/// serve every mandate without repeatedly traversing the same owner index.
fn resolve_available_owned_businesses(
    state: &AppState,
    organization: OrganizationId,
) -> BTreeMap<BusinessId, &crate::world::BusinessRecord> {
    state
        .world()
        .businesses_owned_by_organization(organization)
        .filter(|business| {
            crate::enterprises::enterprise_execution::business_is_available_for_enterprise(
                state,
                business.id(),
            )
        })
        .map(|business| (business.id(), business))
        .collect()
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

/// Resolves the rival's cash account read-only: the smallest sufficiently funded
/// street-or-concealed cash pool, with account ID as the stable tie-breaker. Best-fit cash
/// preserves larger pools for later plans instead of letting account creation order strand
/// otherwise usable working capital. The account must cover the selected configuration's current
/// one-cycle operating runway. Settlement-account planning is kept separate because the cohort
/// projects those finite reusable accounts across multiple same-day establishments.
fn resolve_autonomous_cash_account(
    state: &AppState,
    organization: OrganizationId,
    minimum_working_capital: Money,
    reservations: &BTreeMap<FinancialAccountId, Money>,
) -> Option<FinancialAccountId> {
    let owner = FinancialOwner::Organization(organization);
    let mut cash: Option<(Money, FinancialAccountId)> = None;
    for account in state.finance().accounts_for(owner) {
        let id = account.id();
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
            | AccountKind::AccountedFunds
            | AccountKind::LegitimateOperating => {}
        }
    }
    cash.map(|(_, id)| id)
}

fn settlement_account_is_available(state: &AppState, account: FinancialAccountId) -> bool {
    state
        .finance()
        .get_account(account)
        .is_some_and(|record| record.version() < u32::MAX)
        && state
            .enterprises()
            .get_by_settlement_account(account)
            .is_none()
        && state.economy().get_by_settlement_account(account).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::city_generation::{CityScale, generate_city};
    use crate::core::invariants::validate_invariants;
    use crate::core::persistence::{build_save, restore_save};
    use crate::finance::finance_system::{insert_account, validate_record_transaction};
    use crate::finance::{LedgerPosting, LedgerTransactionDraft};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};

    #[test]
    fn rival_enterprise_planning_prioritizes_abandoned_criminal_commerce() {
        use crate::enterprises::enterprise_execution::validate_suspend_enterprise;

        let registry = build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(82_734_113)).unwrap();
        let attacker = city.rivals[0];
        assert!(!unguarded_criminal_commerce(
            &city.state,
            attacker,
            city.home
        ));
        validate_suspend_enterprise(&city.state, city.starter_racket)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        assert!(unguarded_criminal_commerce(
            &city.state,
            attacker,
            city.home
        ));
        let mut replayed =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        let first =
            plan_due_autonomous_expansions(&registry, &city.state, &BTreeSet::new()).unwrap();
        let replay =
            plan_due_autonomous_expansions(&registry, &replayed, &BTreeSet::new()).unwrap();
        let first_locations = first
            .actions
            .iter()
            .map(|action| (action.organization, action.plan.location))
            .collect::<Vec<_>>();
        assert_eq!(
            first_locations,
            replay
                .actions
                .iter()
                .map(|action| (action.organization, action.plan.location))
                .collect::<Vec<_>>()
        );
        assert!(
            first_locations.iter().any(|(organization, location)| {
                *organization == attacker
                    && *location == EnterpriseLocation::Neighborhood(city.home)
            }),
            "a rival with viable citywide authority should establish a racket in the vacated commercial ward: {first_locations:?}"
        );
        // Execute every canonical minute of the first day, not a hand-advanced clock
        // with overdue opportunities and an invalid save. This exercises the real tick.
        for _ in 0..1_440 {
            crate::core::simulation::run_test_tick(&registry, &mut city.state);
            crate::core::simulation::run_test_tick(&registry, &mut replayed);
        }
        assert!(
            city.state
                .enterprises()
                .active_for_organization(attacker)
                .any(|racket| { racket.location() == EnterpriseLocation::Neighborhood(city.home) }),
            "a rival with viable citywide authority should actually establish in the vacated ward"
        );
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&replayed).unwrap()
        );
        validate_invariants(&city.state);
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    }

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

        let selected = resolve_autonomous_cash_account(
            &state,
            organization,
            Money::from_cents(6_000),
            &BTreeMap::new(),
        )
        .expect("one cash pool should satisfy the requested runway");
        assert_eq!(
            selected, small,
            "best-fit allocation must preserve the larger pool for a later larger runway"
        );
        assert!(settlement_account_is_available(&state, settlement));
        validate_invariants(&state);
    }
}
