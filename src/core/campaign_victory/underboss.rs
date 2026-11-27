//! Read-only institutional, executive and operating-capital evidence for city command.
//! The victory owner derives these from live relationships, the reporting tree and
//! canonical enterprise cost projections; no parallel prestige counter is persisted.

use super::command_staffing::supports_senior_command;
use super::district_matching::Assignments;
use super::{
    UNDERBOSS_CONTACT_DAYS, UNDERBOSS_OPERATING_RUNWAY_DAYS, UNDERBOSS_SENIOR_MANAGEMENT_RATING,
    VICTORY_ESTABLISHMENT_DAYS, VICTORY_PROFITABLE_CYCLES, VictoryError, established_for,
    recent_income_start, settled_recently,
};
use crate::contacts::ContactKind;
use crate::contacts::contact_system::contact_is_currently_usable;
use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::enterprises::enterprise_execution::{
    EnterpriseProjectionStaffing, manager_workload, resolve_enterprise_financial_projection,
};
use crate::enterprises::{EnterpriseRecord, EnterpriseStatus};
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::{AutonomyLevel, CapabilityKind};
use std::collections::{BTreeMap, BTreeSet};

/// Recent earned income, operating maturity and demonstrated staffing capacity are
/// derived from the same settlement window. No separate workforce credit is stored.
pub(super) struct RacketStanding {
    pub(super) net: i128,
    pub(super) established: bool,
    pub(super) proven_associates: u64,
}

pub(super) fn racket_standing(state: &AppState, racket: &EnterpriseRecord) -> RacketStanding {
    let now = state.now();
    let maturity_possible = racket.status() == EnterpriseStatus::Active
        && established_for(now, racket.established_at(), VICTORY_ESTABLISHMENT_DAYS)
        && racket
            .last_cycle_at()
            .is_some_and(|at| settled_recently(now, at));
    let cutoff = recent_income_start(now);
    let mut net = 0_i128;
    let mut profitable = 0;
    let mut commanded_workforces = Vec::new();
    for cycle in state
        .enterprises()
        .cycles_for(racket.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > cutoff)
        .filter(|cycle| cycle.occurred_at() <= now)
    {
        net += i128::from(cycle.net_cash().cents());
        if maturity_possible && cycle.net_cash() > Money::ZERO {
            profitable += 1;
            if racket.authority_for_cycle_at(cycle.occurred_at()).manager == racket.manager() {
                commanded_workforces.push(racket.associates_at(cycle.occurred_at()));
            }
        }
    }
    // The current captain must have demonstrated a workforce at least this
    // large in twenty profitable settlements. A hire adds unproven capacity,
    // not a penalty to the existing workforce; a layoff cannot count workers
    // that are no longer employed. The historical measure is aggregate staffing,
    // not individual employment tenure. The twentieth-largest complement is
    // exactly the largest size supported by the required number of settlements.
    let proven_associates = commanded_workforces
        .len()
        .checked_sub(VICTORY_PROFITABLE_CYCLES)
        .map_or(0, |index| {
            let (_, proven, _) = commanded_workforces.select_nth_unstable(index);
            u64::from((*proven).min(racket.associates()))
        });
    RacketStanding {
        net,
        established: maturity_possible && profitable >= VICTORY_PROFITABLE_CYCLES,
        proven_associates,
    }
}

/// Proven operating experience does not grant current independent discretion.
pub(super) fn independent_captain(
    state: &AppState,
    owner: OrganizationId,
    captain: CharacterId,
) -> bool {
    state.world().get_character(captain).is_some_and(|person| {
        person.organization() == Some(owner)
            && person.supervisor().is_some()
            && matches!(
                person.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
            && state.legal().active_arrest_for_character(captain).is_none()
    })
}

/// Longstanding, currently usable civic relationships, not purchased reputation.
/// Police contacts are optional; an organization need not cultivate law enforcement.
pub(super) fn institutional_reach(state: &AppState, owner: OrganizationId) -> (usize, usize) {
    let mut kinds = [false; 4];
    let mut handlers = BTreeSet::new();
    for contact in state.contacts().contacts_for_sponsor(owner) {
        let kind = match contact.kind() {
            ContactKind::Legal => 0,
            ContactKind::Political => 1,
            ContactKind::Labor => 2,
            ContactKind::Professional => 3,
            ContactKind::Police => continue,
        };
        if !contact_is_currently_usable(state, contact)
            || !established_for(
                state.now(),
                contact.established_at(),
                UNDERBOSS_CONTACT_DAYS,
            )
        {
            continue;
        }
        kinds[kind] = true;
        handlers.insert(contact.handler());
    }
    (
        kinds.into_iter().filter(|present| *present).count(),
        handlers.len(),
    )
}

/// Count senior commanders that can participate in a complete qualifying
/// coalition, not merely those who supervise two candidates on paper.
pub(super) fn senior_command_count(
    state: &AppState,
    owner: OrganizationId,
    eligible_districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
    ordinary: &Assignments,
    required_districts: usize,
) -> usize {
    if ordinary.len() < required_districts.max(2) {
        return 0;
    }
    let mut reports = BTreeMap::<CharacterId, BTreeSet<CharacterId>>::new();
    for district in eligible_districts {
        for captain in captains.get(district).into_iter().flatten() {
            if let Some(supervisor) = state
                .world()
                .get_character(*captain)
                .and_then(|person| person.supervisor())
            {
                reports.entry(supervisor).or_default().insert(*captain);
            }
        }
    }
    reports
        .into_iter()
        .filter(|(supervisor, people)| {
            people.len() >= 2
                && independent_captain(state, owner, *supervisor)
                && state
                    .world()
                    .get_character(*supervisor)
                    .is_some_and(|person| {
                        person
                            .capability(CapabilityKind::Management)
                            .is_some_and(|rating| {
                                rating.value() >= UNDERBOSS_SENIOR_MANAGEMENT_RATING
                            })
                    })
                && manager_workload(state, *supervisor)
                    .is_some_and(|load| load.command_lines >= 2 && load.required <= load.available)
                && supports_senior_command(
                    eligible_districts,
                    captains,
                    people,
                    ordinary,
                    required_districts,
                )
        })
        .count()
}

/// Reserve for the actual active network's current staff, venues and support costs.
/// Ask the canonical projection for maximum modeled heat, not private police truth.
/// Round up the number of cycles needed to cover the runway (including partial days).
pub(super) fn operating_reserve(
    registry: &Registry,
    state: &AppState,
    rackets: &[&EnterpriseRecord],
) -> Result<i128, VictoryError> {
    let runway_minutes = UNDERBOSS_OPERATING_RUNWAY_DAYS * DAY_MINUTES;
    let mut total = 0_i128;
    for racket in rackets {
        let manager = state
            .world()
            .get_character(racket.manager())
            .ok_or(VictoryError::Arithmetic)?;
        let (cost, _) = resolve_enterprise_financial_projection(
            registry,
            state,
            racket.kind(),
            racket.location(),
            racket.supporting_businesses().len(),
            true,
            EnterpriseProjectionStaffing {
                management: manager.capability(CapabilityKind::Management),
                associates: racket.associates(),
            },
        )
        .map_err(|_| VictoryError::Arithmetic)?;
        let cycle_minutes = u64::from(
            registry
                .get_enterprise(racket.kind())
                .economics()
                .cycle()
                .as_minutes(),
        );
        let cycles = runway_minutes.div_ceil(cycle_minutes);
        total = total
            .checked_add(i128::from(cost.cents()) * i128::from(cycles))
            .ok_or(VictoryError::Arithmetic)?;
    }
    Ok(total)
}
