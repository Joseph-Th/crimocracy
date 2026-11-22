//! Territorial confrontation through canonical enterprise suspension and typed history.
//!
//! A clash is a district-level attempt to displace a rival's active racket.
//! Rank-and-file employees are the actual committed force, including the staffing
//! paid for by enterprise settlements. A defender has a small positional edge.
//! Losing a clash closes one racket for seven days, making fighting costly even
//! for an attacker that misjudges the opposition. No extra combat currency or
//! invisible territory ownership is maintained.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    EnterpriseId, HistoryEventId, IdExhaustionError, IdKind, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, is_day_boundary};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, ValidatedEnterpriseStatusChange, validate_suspend_enterprise,
};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::history::HistoryEventKind;
use crate::history::history_system::{
    HistoryError, ValidatedHistoryEvent, validate_territory_clash_event,
};
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::OrganizationKind;
use crate::world::territory_influence::active_standstill;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const CLASH_COOLDOWN_DAYS: u64 = 7;
pub const CLASH_SUPPRESSION_DAYS: u64 = 7;
const DEFENSIVE_ADVANTAGE: u32 = 2;

pub fn district_of_racket(
    state: &AppState,
    location: EnterpriseLocation,
) -> Option<NeighborhoodId> {
    match location {
        EnterpriseLocation::Neighborhood(district) => Some(district),
        EnterpriseLocation::Business(venue) => state
            .world()
            .get_business(venue)
            .map(|business| business.neighborhood()),
    }
}

/// How many staffed rank-and-file associates an outfit actually has active here.
pub fn active_district_force(
    state: &AppState,
    organization: OrganizationId,
    district: NeighborhoodId,
) -> u32 {
    state
        .enterprises()
        .active_for_organization(organization)
        .filter(|racket| district_of_racket(state, racket.location()) == Some(district))
        .map(|racket| u32::from(racket.associates()))
        .sum()
}

pub fn suppression_ends_at(state: &AppState, racket: EnterpriseId) -> Option<SimTime> {
    state
        .history()
        .events()
        .filter_map(|event| {
            let HistoryEventKind::TerritoryClash { loser, .. } = event.kind() else {
                return None;
            };
            if loser != racket {
                return None;
            }
            let end = event
                .occurred_at()
                .as_minutes()
                .checked_add(CLASH_SUPPRESSION_DAYS * DAY_MINUTES)?;
            (state.now().as_minutes() < end).then_some(SimTime::from_minutes(end))
        })
        .last()
}

fn next_clash_allowed_at(
    state: &AppState,
    attacker: OrganizationId,
    district: NeighborhoodId,
) -> Option<SimTime> {
    state
        .history()
        .events()
        .filter_map(|event| {
            let HistoryEventKind::TerritoryClash {
                attacker: previous,
                district: location,
                ..
            } = event.kind()
            else {
                return None;
            };
            if previous != attacker || location != district {
                return None;
            }
            let end = event
                .occurred_at()
                .as_minutes()
                .checked_add(CLASH_COOLDOWN_DAYS * DAY_MINUTES)?;
            (state.now().as_minutes() < end).then_some(SimTime::from_minutes(end))
        })
        .last()
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TerritoryConflictError {
    #[error("racket {0} does not exist")]
    MissingRacket(EnterpriseId),
    #[error("only active rackets may fight for territory")]
    RacketNotActive,
    #[error("cannot challenge your own organization")]
    SameOrganization,
    #[error("both participants must be criminal gangs")]
    InvalidGang,
    #[error("your gang needs an active racket in this district to contest it")]
    NoLocalFootprint,
    #[error("district {0} has a binding territorial agreement; no fighting is allowed")]
    Standstill(NeighborhoodId),
    #[error("this gang cannot challenge the same district again until {0:?}")]
    Cooldown(SimTime),
    #[error("the simulation cannot represent another seven-day clash cooldown")]
    TimeExhausted,
    #[error("a prepared clash is stale; time changed from {expected:?} to {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    Enterprise(#[from] EnterpriseError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedTerritoryClash {
    at: SimTime,
    attacker: OrganizationId,
    defender: OrganizationId,
    district: NeighborhoodId,
    target: EnterpriseId,
    assault: EnterpriseId,
    loser: EnterpriseId,
    suspension: ValidatedEnterpriseStatusChange,
    history: ValidatedHistoryEvent,
    reports: Vec<ValidatedReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerritoryClashOutcome {
    pub history: HistoryEventId,
    pub attacker_won: bool,
    pub suspended_racket: EnterpriseId,
    pub district: NeighborhoodId,
}

impl ValidatedTerritoryClash {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<TerritoryClashOutcome, TerritoryConflictError> {
        if self.at != state.now() {
            return Err(TerritoryConflictError::StaleTime {
                expected: self.at,
                found: state.now(),
            });
        }
        // A held plan cannot ignore a new treaty, new workers, a departed
        // manager, a different winner, or another attack in the same district.
        let fresh = validate_clash_terms(state, self.attacker, self.target)?;
        if fresh.0 != self.district
            || fresh.1 != self.defender
            || fresh.2 != self.assault
            || fresh.3 != self.loser
        {
            return Err(TerritoryConflictError::RacketNotActive);
        }
        self.suspension.ensure_suspension_current(state)?;
        state.ids.reserve_many(&[
            (IdKind::HistoryEvent, 1),
            (
                IdKind::Report,
                u32::try_from(self.reports.len()).expect("two report recipients at most"),
            ),
        ])?;
        // History allocation is the last possible fallible step after the
        // preflights. Suspension itself allocates no IDs; reports were validated
        // and their ID budget was reserved above.
        let history = self.history.commit(state)?;
        self.suspension.commit_suspension_preflighted(state);
        for report in self.reports {
            report
                .commit(state)
                .expect("validated clash report and reserved ID must commit");
        }
        Ok(TerritoryClashOutcome {
            history,
            attacker_won: self.loser == self.target,
            suspended_racket: self.loser,
            district: self.district,
        })
    }
}

/// Returns (district, defender, assault, loser) without changing state.
fn validate_clash_terms(
    state: &AppState,
    attacker: OrganizationId,
    target: EnterpriseId,
) -> Result<(NeighborhoodId, OrganizationId, EnterpriseId, EnterpriseId), TerritoryConflictError> {
    let enemy = state
        .enterprises()
        .get_enterprise(target)
        .ok_or(TerritoryConflictError::MissingRacket(target))?;
    if enemy.status() != EnterpriseStatus::Active {
        return Err(TerritoryConflictError::RacketNotActive);
    }
    let defender = enemy.organization();
    if defender == attacker {
        return Err(TerritoryConflictError::SameOrganization);
    }
    if [attacker, defender].into_iter().any(|id| {
        state
            .world()
            .get_organization(id)
            .is_none_or(|record| record.kind() != OrganizationKind::Criminal)
    }) {
        return Err(TerritoryConflictError::InvalidGang);
    }
    let district = district_of_racket(state, enemy.location())
        .ok_or(TerritoryConflictError::MissingRacket(target))?;
    if active_standstill(state, attacker, district).is_some()
        || active_standstill(state, defender, district).is_some()
    {
        return Err(TerritoryConflictError::Standstill(district));
    }
    if let Some(next) = next_clash_allowed_at(state, attacker, district) {
        return Err(TerritoryConflictError::Cooldown(next));
    }
    if state
        .now()
        .as_minutes()
        .checked_add(CLASH_SUPPRESSION_DAYS * DAY_MINUTES)
        .is_none()
    {
        return Err(TerritoryConflictError::TimeExhausted);
    }
    let assault = state
        .enterprises()
        .active_for_organization(attacker)
        .filter(|racket| district_of_racket(state, racket.location()) == Some(district))
        .max_by_key(|racket| (racket.associates(), std::cmp::Reverse(racket.id())))
        .map(|record| record.id())
        .ok_or(TerritoryConflictError::NoLocalFootprint)?;
    let own_force = active_district_force(state, attacker, district);
    let enemy_force = active_district_force(state, defender, district);
    let loser = if own_force > enemy_force.saturating_add(DEFENSIVE_ADVANTAGE) {
        target
    } else {
        assault
    };
    Ok((district, defender, assault, loser))
}

/// The same decision for players and autonomous gangs; reports contain only
/// direct involvement, and do not reveal unrelated gang holdings.
pub fn validate_territory_clash(
    state: &AppState,
    attacker: OrganizationId,
    target: EnterpriseId,
) -> Result<ValidatedTerritoryClash, TerritoryConflictError> {
    let (district, defender, assault, loser) = validate_clash_terms(state, attacker, target)?;
    let suspension = validate_suspend_enterprise(state, loser)?;
    let history = validate_territory_clash_event(
        state, district, attacker, defender, assault, target, loser,
    )?;
    let district_name = state
        .world()
        .get_neighborhood(district)
        .expect("valid rival racket must occupy a district")
        .name();
    let mut reports = Vec::new();
    for (organization, role) in [(attacker, "Attacker"), (defender, "Defender")] {
        // A defender sees the raid on its own people, even without having
        // discovered the enemy's confidential network of rackets.
        if state.player_organization() != Some(organization) {
            continue;
        }
        let message = match (role, loser == target) {
            ("Attacker", true) => format!(
                "Your enforcers displaced a rival racket in {district_name}. Its operations are disrupted for seven days."
            ),
            ("Attacker", false) => format!(
                "Your challenge in {district_name} was repelled. Your front {loser} is suspended for seven days."
            ),
            ("Defender", true) => format!(
                "Rival enforcers overwhelmed your racket {loser} in {district_name}. It cannot reopen for seven days."
            ),
            ("Defender", false) => format!(
                "Your people repelled a territorial challenge in {district_name}; the attacking gang lost a front."
            ),
            _ => unreachable!(),
        };
        reports.push(validate_record_report(
            state,
            ReportDraft {
                recipient: organization,
                kind: ReportKind::AfterAction,
                title: format!("Turf conflict: {district_name}"),
                entries: vec![ReportEntry {
                    attention: if organization == defender && loser == target {
                        AttentionClass::Crisis
                    } else {
                        AttentionClass::Notable
                    },
                    summary: message,
                    sources: Vec::new(),
                    entities: BTreeSet::from([
                        EntityRef::Organization(organization),
                        EntityRef::Neighborhood(district),
                    ]),
                    decision: None,
                }],
            },
        )?);
    }
    state.ids.reserve_many(&[
        (IdKind::HistoryEvent, 1),
        (
            IdKind::Report,
            u32::try_from(reports.len()).expect("at most two player reports"),
        ),
    ])?;
    Ok(ValidatedTerritoryClash {
        at: state.now(),
        attacker,
        defender,
        district,
        target,
        assault,
        loser,
        suspension,
        history,
        reports,
    })
}

/// One deliberate territorial move per non-player gang per day. The challenger
/// only fights from a district where it already employs people, and only when
/// its own strength appears enough to dislodge a visible rival footprint.
/// It can infer baseline workers from visible racket types, not secretly hired
/// extra associates. Defenders can therefore ambush an overconfident NPC by
/// investing in local guards. A failed challenge discourages another raid for
/// thirty days. Rival operations are visible only within the attacker's own
/// occupied district; the player still needs sourced reconnaissance.
fn estimated_defender_force(
    state: &AppState,
    defender: OrganizationId,
    district: NeighborhoodId,
) -> u32 {
    state
        .enterprises()
        .active_for_organization(defender)
        .filter(|racket| district_of_racket(state, racket.location()) == Some(district))
        .map(|racket| u32::from(racket.kind().baseline_associates()))
        .sum()
}

fn recently_repulsed(state: &AppState, attacker: OrganizationId, district: NeighborhoodId) -> bool {
    state.history().events().any(|event| {
        let HistoryEventKind::TerritoryClash {
            attacker: author,
            district: ward,
            assault,
            loser,
            ..
        } = event.kind()
        else {
            return false;
        };
        author == attacker
            && ward == district
            && loser == assault
            && state
                .now()
                .as_minutes()
                .saturating_sub(event.occurred_at().as_minutes())
                < 30 * DAY_MINUTES
    })
}

pub(crate) fn apply_due_autonomous_territory_clashes(
    state: &mut AppState,
) -> Vec<TerritoryClashOutcome> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let mut candidates = BTreeMap::<OrganizationId, Vec<(NeighborhoodId, EnterpriseId)>>::new();
    let active: Vec<_> = state
        .enterprises()
        .active_enterprises()
        .filter_map(|racket| {
            district_of_racket(state, racket.location())
                .map(|district| (district, racket.organization(), racket.id()))
        })
        .collect();
    for &(district, organization, target) in &active {
        for &(other_district, attacker, _) in &active {
            if other_district != district
                || attacker == organization
                || state.player_organization() == Some(attacker)
                || recently_repulsed(state, attacker, district)
                || active_district_force(state, attacker, district)
                    <= estimated_defender_force(state, organization, district)
                        .saturating_add(DEFENSIVE_ADVANTAGE)
            {
                continue;
            }
            candidates
                .entry(attacker)
                .or_default()
                .push((district, target));
        }
    }
    let mut results = Vec::new();
    for (attacker, mut targets) in candidates {
        targets.sort_unstable();
        targets.dedup();
        for (_, target) in targets {
            if let Ok(plan) = validate_territory_clash(state, attacker, target)
                && let Ok(outcome) = plan.commit(state)
            {
                results.push(outcome);
                break;
            }
        }
    }
    results
}
