//! Read-only planning for one daily autonomous expansion cohort.
//!
//! The parent module owns canonical establishment and commit. This child projects only the
//! same-pass consequences needed to choose the exact deterministic sequence before mutation:
//! working-capital reservations, occupied racket slots, district influence, and settlement-account
//! claims. That lets the parent preflight the exact persistent-ID budget for the whole cohort.

use super::candidate_planning::{compare_expansion_plans, compare_phase_expansion_candidates};
use super::{
    AutonomousExpansionPlan, ExpansionIterationContext, enumerate_autonomous_expansion_candidates,
    resolve_autonomous_cash_account, resolve_available_owned_businesses,
    resolve_max_autonomous_working_capital, settlement_account_is_available,
};
use crate::core::id::{BusinessId, FinancialAccountId, MandateId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::delegation::{MandateRecord, ResponsibilityFunction, ResponsibilityScope};
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, reserve_working_capital, resolve_committed_working_capital,
    resolve_observed_district_pressure,
};
use crate::enterprises::enterprise_execution::{
    ProjectedCommandJob, projected_command_jobs_fit, resolve_location_neighborhood,
};
use crate::enterprises::{EnterpriseKind, EnterpriseLocation, EnterpriseStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::territory_conflict::{DEFENSIVE_ADVANTAGE, local_defensive_forces};
use crate::world::territory_influence::{resolve_neighborhood_influence, unique_economic_leader};
use crate::world::{AutonomyLevel, BusinessRecord, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct PhaseExpansionCandidate {
    organization: OrganizationId,
    mandate_index: usize,
    mandate: MandateId,
    leads_district: bool,
    plan: AutonomousExpansionPlan,
}

#[derive(Clone, Copy)]
pub(super) enum PlannedSettlementAccount {
    Existing(FinancialAccountId),
    Fresh,
}

pub(super) struct PlannedAutonomousExpansionAction {
    pub(super) organization: OrganizationId,
    pub(super) mandate: MandateRecord,
    pub(super) plan: AutonomousExpansionPlan,
    pub(super) cash_account: FinancialAccountId,
    pub(super) settlement: PlannedSettlementAccount,
}

pub(super) struct AutonomousExpansionCohort {
    pub(super) actions: Vec<PlannedAutonomousExpansionAction>,
    fresh_settlement_accounts: u32,
}

impl AutonomousExpansionCohort {
    pub(super) fn enterprise_count(&self) -> u32 {
        u32::try_from(self.actions.len())
            .expect("persistent autonomous expansion count must fit the enterprise id space")
    }

    pub(super) fn fresh_settlement_accounts(&self) -> u32 {
        self.fresh_settlement_accounts
    }
}

pub(super) struct ProjectedTerritoryInfluence {
    counts: BTreeMap<NeighborhoodId, BTreeMap<OrganizationId, u32>>,
    strongest_racket_force: BTreeMap<OrganizationId, u32>,
    exposed_commerce_defense: BTreeMap<NeighborhoodId, u32>,
}

pub(super) fn plan_due_autonomous_expansions(
    registry: &Registry,
    state: &AppState,
    excluded_mandates: &BTreeSet<MandateId>,
) -> Result<AutonomousExpansionCohort, AutonomousEnterpriseError> {
    let mut mandates = resolve_eligible_expansion_mandates(
        registry,
        state,
        state.player_organization(),
        excluded_mandates,
    )?;
    // On a campaign with no eligible delegated rival managers, reconstructing recent racket
    // histories and reserving every active racket's operating runway cannot affect the outcome.
    if mandates.is_empty() {
        return Ok(AutonomousExpansionCohort {
            actions: Vec::new(),
            fresh_settlement_accounts: 0,
        });
    }
    // Ownership and operating status cannot change while this pass plans read-only. Snapshot
    // each organization's viable venues once; only projected working capital, occupied slots,
    // and district leadership change between iterations. Re-scanning every owned storefront
    // for every competing mandate would make large autonomous cohorts needlessly quadratic.
    let available_venues: BTreeMap<OrganizationId, BTreeMap<BusinessId, &BusinessRecord>> =
        mandates
            .keys()
            .map(|organization| {
                (
                    *organization,
                    resolve_available_owned_businesses(state, *organization),
                )
            })
            .collect();
    let observed_district_pressure = resolve_observed_district_pressure(registry, state)?;
    // Working capital is a capacity constraint, not a balance-presence check. Existing active
    // rackets already rely on their cash accounts for one current operating cycle, and each new
    // establishment in this pass makes another claim on that same pool.
    let mut working_capital_reservations =
        resolve_committed_working_capital(registry, state, &observed_district_pressure)?;
    let mut territory = ProjectedTerritoryInfluence::from_state(state);
    let mut occupied_locations = resolve_occupied_enterprise_locations(state);
    let mut free_settlement_accounts = resolve_free_settlement_accounts(state, &mandates);
    let mut fresh_settlement_accounts = 0_u32;
    let mut actions = Vec::new();

    // Organizations compete in one phase-wide queue. Lightweight projections model exactly the
    // authoritative changes earlier planned establishments would make, but state stays untouched
    // until the parent has reserved the complete cohort's finite resources.
    while !mandates.is_empty() {
        let district_leaders = territory.leaders();
        let iteration = ExpansionIterationContext {
            registry,
            state,
            observed_district_pressure: &observed_district_pressure,
            district_leaders: &district_leaders,
            territory: &territory,
        };
        let Some(selected) = select_phase_expansion_candidate(
            &iteration,
            &working_capital_reservations,
            &occupied_locations,
            &mandates,
            &available_venues,
            &actions,
        )?
        else {
            break;
        };
        let organization = selected.organization;
        let (mandate, remove_organization) = {
            let organization_mandates = mandates
                .get_mut(&organization)
                .expect("selected autonomous organization must retain its mandate queue");
            debug_assert_eq!(
                organization_mandates[selected.mandate_index].id(),
                selected.mandate
            );
            let mandate = organization_mandates.remove(selected.mandate_index);
            (mandate, organization_mandates.is_empty())
        };
        if remove_organization {
            mandates.remove(&organization);
        }

        let cash_account = resolve_autonomous_cash_account(
            state,
            organization,
            selected.plan.required_working_capital,
            &working_capital_reservations,
        )
        .expect("selected autonomous plan must retain a sufficiently funded cash account");
        reserve_working_capital(
            &mut working_capital_reservations,
            cash_account,
            selected.plan.required_working_capital,
        );
        let settlement = free_settlement_accounts
            .get_mut(&organization)
            .and_then(BTreeSet::pop_first)
            .map(PlannedSettlementAccount::Existing)
            .unwrap_or_else(|| {
                fresh_settlement_accounts = fresh_settlement_accounts
                    .checked_add(1)
                    .expect("persistent autonomous expansion count must fit u32");
                PlannedSettlementAccount::Fresh
            });

        occupied_locations.insert((selected.plan.kind, selected.plan.location));
        territory.record_establishment(
            state,
            organization,
            selected.plan.kind,
            selected.plan.location,
        );
        actions.push(PlannedAutonomousExpansionAction {
            organization,
            mandate,
            plan: selected.plan,
            cash_account,
            settlement,
        });
    }

    Ok(AutonomousExpansionCohort {
        actions,
        fresh_settlement_accounts,
    })
}

fn select_phase_expansion_candidate(
    iteration: &ExpansionIterationContext<'_>,
    working_capital_reservations: &BTreeMap<FinancialAccountId, Money>,
    occupied_locations: &BTreeSet<(EnterpriseKind, EnterpriseLocation)>,
    mandates: &BTreeMap<OrganizationId, Vec<MandateRecord>>,
    available_venues: &BTreeMap<OrganizationId, BTreeMap<BusinessId, &BusinessRecord>>,
    planned_actions: &[PlannedAutonomousExpansionAction],
) -> Result<Option<PhaseExpansionCandidate>, AutonomousEnterpriseError> {
    let mut winner: Option<PhaseExpansionCandidate> = None;
    let planned_jobs = planned_actions
        .iter()
        .map(|action| ProjectedCommandJob {
            organization: action.organization,
            captain: action.mandate.manager(),
            kind: action.plan.kind,
            location: action.plan.location,
            associates: action.plan.kind.baseline_associates(),
            supporting_businesses: action.plan.supporting_businesses.len(),
        })
        .collect::<Vec<_>>();
    for (organization, organization_mandates) in mandates {
        let Some(available_working_capital) = resolve_max_autonomous_working_capital(
            iteration.state,
            *organization,
            working_capital_reservations,
        ) else {
            continue;
        };
        let owned_venues = available_venues
            .get(organization)
            .expect("eligible expansion organization must have a venue snapshot");
        let mut prospective_jobs: Vec<_> = planned_jobs
            .iter()
            .copied()
            .filter(|job| job.organization == *organization)
            .collect();
        // Resolve internal governance before comparing organizations. A mixed comparator
        // would otherwise let mandate iteration order change the contested-slot winner.
        let mut preferred: Option<PhaseExpansionCandidate> = None;
        for (mandate_index, mandate) in organization_mandates.iter().enumerate() {
            let candidates = enumerate_autonomous_expansion_candidates(
                iteration,
                *organization,
                mandate,
                available_working_capital,
                occupied_locations,
                owned_venues,
            )?;
            // Rank only feasible plans. A larger preferred racket may no
            // longer fit after a subordinate adds a command line, while a
            // smaller profitable alternative still fits. Avoid rechecking
            // unrelated organizations or plans that cannot beat the best
            // feasible candidate already found for this mandate.
            let mut best_plan = None;
            for plan in candidates {
                if best_plan
                    .as_ref()
                    .is_some_and(|best| !compare_expansion_plans(&plan, best).is_lt())
                {
                    continue;
                }
                if !prospective_jobs.is_empty() {
                    prospective_jobs.push(ProjectedCommandJob {
                        organization: *organization,
                        captain: mandate.manager(),
                        kind: plan.kind,
                        location: plan.location,
                        associates: plan.kind.baseline_associates(),
                        supporting_businesses: plan.supporting_businesses.len(),
                    });
                    let fits = projected_command_jobs_fit(iteration.state, &prospective_jobs);
                    prospective_jobs.pop();
                    if !fits {
                        continue;
                    }
                }
                best_plan = Some(plan);
            }
            let Some(plan) = best_plan else { continue };
            let neighborhood = resolve_location_neighborhood(iteration.state, plan.location)
                .expect("autonomous candidate location must resolve to a live neighborhood");
            let district_leader = iteration
                .district_leaders
                .get(&neighborhood)
                .copied()
                .expect("autonomous candidate neighborhood must retain its influence snapshot");
            let candidate = PhaseExpansionCandidate {
                organization: *organization,
                mandate_index,
                mandate: mandate.id(),
                leads_district: district_leader == Some(*organization),
                plan,
            };
            let replace = preferred.as_ref().is_none_or(|selected| {
                compare_expansion_plans(&candidate.plan, &selected.plan)
                    .then(candidate.mandate.cmp(&selected.mandate))
                    .is_lt()
            });
            if replace {
                preferred = Some(candidate);
            }
        }
        let Some(candidate) = preferred else {
            continue;
        };
        let replace = winner.as_ref().is_none_or(|selected| {
            compare_phase_expansion_candidates(
                candidate.leads_district,
                &candidate.plan,
                selected.leads_district,
                &selected.plan,
            )
            .then(candidate.organization.cmp(&selected.organization))
            .is_lt()
        });
        if replace {
            winner = Some(candidate);
        }
    }
    Ok(winner)
}

fn resolve_eligible_expansion_mandates(
    registry: &Registry,
    state: &AppState,
    player_organization: Option<OrganizationId>,
    excluded_mandates: &BTreeSet<MandateId>,
) -> Result<BTreeMap<OrganizationId, Vec<MandateRecord>>, AutonomousEnterpriseError> {
    let mut by_organization = BTreeMap::new();
    for mandate in state.delegation().active_mandates() {
        if excluded_mandates.contains(&mandate.id()) {
            continue;
        }
        // Personnel, finance, legal, and other function-only mandates cannot establish an
        // enterprise. Exclude them before building district and financial projections rather
        // than repeatedly examining an empty enterprise candidate set.
        if !mandate.scopes().iter().any(|scope| {
            matches!(
                scope,
                ResponsibilityScope::Neighborhood(_)
                    | ResponsibilityScope::Business(_)
                    | ResponsibilityScope::Function(ResponsibilityFunction::Enterprise)
            )
        }) {
            continue;
        }
        let organization = mandate.organization();
        if Some(organization) == player_organization
            || state
                .world()
                .get_organization(organization)
                .is_none_or(|record| record.kind() != OrganizationKind::Criminal)
        {
            continue;
        }
        let police_fear = crate::reputation::reputation_system::resolve_score(
            registry,
            state.reputation(),
            organization,
            crate::reputation::AudienceKind::Police,
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
            .or_insert_with(Vec::new)
            .push(mandate.clone());
    }
    Ok(by_organization)
}

impl ProjectedTerritoryInfluence {
    pub(super) fn from_state(state: &AppState) -> Self {
        let counts: BTreeMap<_, BTreeMap<_, _>> = state
            .world()
            .neighborhoods()
            .map(|neighborhood| {
                let neighborhood_id = neighborhood.id();
                let counts = resolve_neighborhood_influence(state, neighborhood_id)
                    .expect("world neighborhood must resolve for autonomous expansion")
                    .standings
                    .into_iter()
                    .map(|standing| (standing.organization, standing.active_enterprises))
                    .collect();
                (neighborhood_id, counts)
            })
            .collect();
        let mut strongest_racket_force = BTreeMap::<_, u32>::new();
        for racket in state.enterprises().active_enterprises() {
            let strongest = strongest_racket_force
                .entry(racket.organization())
                .or_default();
            *strongest = (*strongest).max(u32::from(racket.associates()));
        }

        // The player never participates in the expansion cohort, so the exposed
        // commercial wards and their distributed defense remain fixed throughout it.
        // Snapshot only the player's owned, operating storefronts once, not the
        // entire city directory once per ward, mandate, and selection iteration.
        let mut exposed_commerce_defense = BTreeMap::new();
        if let Some(player) = state.player_organization()
            && state
                .world()
                .get_organization(player)
                .is_some_and(|org| org.kind() == OrganizationKind::Criminal)
        {
            let defenses = local_defensive_forces(state, player);
            for shop in state.world().businesses_owned_by_organization(player) {
                let ward = shop.neighborhood();
                if counts[&ward].contains_key(&player)
                    || !state
                        .economy()
                        .get_business_economy(shop.id())
                        .is_some_and(|books| books.is_operating())
                {
                    continue;
                }
                exposed_commerce_defense
                    .entry(ward)
                    .or_insert_with(|| defenses.get(&ward).copied().unwrap_or_default().total());
            }
        }
        Self {
            counts,
            strongest_racket_force,
            exposed_commerce_defense,
        }
    }

    pub(super) fn vacated_commercial_districts(
        &self,
        attacker: OrganizationId,
    ) -> BTreeSet<NeighborhoodId> {
        let force = self
            .strongest_racket_force
            .get(&attacker)
            .copied()
            .unwrap_or(u32::from(EnterpriseKind::Protection.baseline_associates()));
        self.exposed_commerce_defense
            .iter()
            .filter_map(|(&ward, &defense)| {
                (!self.counts[&ward].contains_key(&attacker)
                    && force > defense.saturating_add(DEFENSIVE_ADVANTAGE))
                .then_some(ward)
            })
            .collect()
    }

    fn leaders(&self) -> BTreeMap<NeighborhoodId, Option<OrganizationId>> {
        self.counts
            .iter()
            .map(|(neighborhood, counts)| {
                (
                    *neighborhood,
                    unique_economic_leader(
                        counts
                            .iter()
                            .map(|(organization, count)| (*organization, *count)),
                    ),
                )
            })
            .collect()
    }

    fn record_establishment(
        &mut self,
        state: &AppState,
        organization: OrganizationId,
        kind: EnterpriseKind,
        location: EnterpriseLocation,
    ) {
        let neighborhood = resolve_location_neighborhood(state, location)
            .expect("planned autonomous enterprise location must resolve to a neighborhood");
        let count = self
            .counts
            .get_mut(&neighborhood)
            .expect("planned autonomous enterprise neighborhood must persist")
            .entry(organization)
            .or_default();
        *count = count
            .checked_add(1)
            .expect("active enterprise count cannot exceed the persistent enterprise id space");
        let strongest = self.strongest_racket_force.entry(organization).or_default();
        *strongest = (*strongest).max(u32::from(kind.baseline_associates()));
    }
}

fn resolve_occupied_enterprise_locations(
    state: &AppState,
) -> BTreeSet<(EnterpriseKind, EnterpriseLocation)> {
    state
        .enterprises()
        .enterprises()
        .filter(|enterprise| enterprise.status() != EnterpriseStatus::Retired)
        .map(|enterprise| (enterprise.kind(), enterprise.location()))
        .collect()
}

fn resolve_free_settlement_accounts(
    state: &AppState,
    mandates: &BTreeMap<OrganizationId, Vec<MandateRecord>>,
) -> BTreeMap<OrganizationId, BTreeSet<FinancialAccountId>> {
    mandates
        .keys()
        .map(|organization| {
            let accounts = state
                .finance()
                .accounts_for(FinancialOwner::Organization(*organization))
                .filter(|account| {
                    account.kind() == AccountKind::Settlement
                        && settlement_account_is_available(state, account.id())
                })
                .map(|account| account.id())
                .collect();
            (*organization, accounts)
        })
        .collect()
}
