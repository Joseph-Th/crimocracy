//! Read-only planning for one daily autonomous expansion cohort.
//!
//! The parent module owns canonical establishment and commit. This child projects only the
//! same-pass consequences needed to choose the exact deterministic sequence before mutation:
//! working-capital reservations, occupied racket slots, district influence, and settlement-account
//! claims. That lets the parent preflight the exact persistent-ID budget for the whole cohort.

use super::candidate_planning::compare_phase_expansion_candidates;
use super::{
    AutonomousExpansionPlan, ExpansionIterationContext, decide_autonomous_expansion,
    resolve_autonomous_cash_account, resolve_available_owned_businesses,
    resolve_max_autonomous_working_capital, settlement_account_is_available,
};
use crate::core::id::{FinancialAccountId, MandateId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::MandateRecord;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, ObservedDistrictPressure, reserve_working_capital,
    resolve_committed_working_capital, resolve_observed_district_pressure,
};
use crate::enterprises::enterprise_execution::resolve_location_neighborhood;
use crate::enterprises::{EnterpriseKind, EnterpriseLocation, EnterpriseStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::AutonomyLevel;
use crate::world::territory_influence::{resolve_neighborhood_influence, unique_economic_leader};
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

struct ProjectedTerritoryInfluence {
    counts: BTreeMap<NeighborhoodId, BTreeMap<OrganizationId, u32>>,
}

pub(super) fn plan_due_autonomous_expansions(
    registry: &Registry,
    state: &AppState,
    excluded_mandates: &BTreeSet<MandateId>,
) -> Result<AutonomousExpansionCohort, AutonomousEnterpriseError> {
    let observed_district_pressure = resolve_observed_district_pressure(registry, state)?;
    // Working capital is a capacity constraint, not a balance-presence check. Existing active
    // rackets already rely on their cash accounts for one current operating cycle, and each new
    // establishment in this pass makes another claim on that same pool.
    let mut working_capital_reservations =
        resolve_committed_working_capital(registry, state, &observed_district_pressure)?;
    let mut mandates = resolve_eligible_expansion_mandates(
        registry,
        state,
        state.player_organization(),
        excluded_mandates,
    )?;
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
        let Some(selected) = select_phase_expansion_candidate(
            registry,
            state,
            &observed_district_pressure,
            &working_capital_reservations,
            &occupied_locations,
            &district_leaders,
            &mandates,
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
        )?;
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
        territory.record_establishment(state, organization, selected.plan.location);
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
    registry: &Registry,
    state: &AppState,
    observed_district_pressure: &ObservedDistrictPressure,
    working_capital_reservations: &BTreeMap<FinancialAccountId, Money>,
    occupied_locations: &BTreeSet<(EnterpriseKind, EnterpriseLocation)>,
    district_leaders: &BTreeMap<NeighborhoodId, Option<OrganizationId>>,
    mandates: &BTreeMap<OrganizationId, Vec<MandateRecord>>,
) -> Result<Option<PhaseExpansionCandidate>, AutonomousEnterpriseError> {
    let iteration = ExpansionIterationContext {
        registry,
        state,
        observed_district_pressure,
        district_leaders,
    };
    let mut winner: Option<PhaseExpansionCandidate> = None;
    for (organization, organization_mandates) in mandates {
        let Some(available_working_capital) = resolve_max_autonomous_working_capital(
            state,
            *organization,
            working_capital_reservations,
        ) else {
            continue;
        };
        let owned_venues = resolve_available_owned_businesses(state, *organization);
        for (mandate_index, mandate) in organization_mandates.iter().enumerate() {
            let Some(plan) = decide_autonomous_expansion(
                &iteration,
                *organization,
                mandate,
                available_working_capital,
                occupied_locations,
                &owned_venues,
            )?
            else {
                continue;
            };
            let neighborhood = resolve_location_neighborhood(state, plan.location)
                .expect("autonomous candidate location must resolve to a live neighborhood");
            let district_leader = district_leaders
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
            let replace = winner.as_ref().is_none_or(|selected| {
                compare_phase_expansion_candidates(
                    candidate.organization,
                    candidate.leads_district,
                    &candidate.plan,
                    selected.organization,
                    selected.leads_district,
                    &selected.plan,
                )
                .then(candidate.organization.cmp(&selected.organization))
                .then(candidate.mandate.cmp(&selected.mandate))
                .is_lt()
            });
            if replace {
                winner = Some(candidate);
            }
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
        let organization = mandate.organization();
        if Some(organization) == player_organization {
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
    fn from_state(state: &AppState) -> Self {
        let counts = state
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
        Self { counts }
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
