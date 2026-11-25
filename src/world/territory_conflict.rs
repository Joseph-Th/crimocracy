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
use crate::history::history_system::{
    HistoryError, ValidatedHistoryEvent, validate_territory_clash_event,
};
use crate::operations::OperationStatus;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::territory_influence::active_standstill;
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const CLASH_COOLDOWN_DAYS: u64 = 7;
pub const CLASH_SUPPRESSION_DAYS: u64 = 7;
pub(crate) const DEFENSIVE_ADVANTAGE: u32 = 2;

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

/// Named muscle available for either defense or projecting force. Administrative
/// headcount, detained members, and personnel already in the field do not add
/// territorial fighting capacity.
pub(crate) fn available_named_enforcers(state: &AppState, organization: OrganizationId) -> usize {
    state
        .world()
        .characters_in_organization(organization)
        .filter(|member| {
            state
                .legal()
                .active_arrest_for_character(member.id())
                .is_none()
                && !state
                    .operations()
                    .active_operations_for_participant(member.id())
                    .any(|operation| {
                        matches!(
                            operation.status(),
                            OperationStatus::InProgress | OperationStatus::AwaitingDecision
                        )
                    })
        })
        .filter(|member| {
            [CapabilityKind::Violence, CapabilityKind::Intimidation]
                .into_iter()
                .any(|skill| {
                    member
                        .capability(skill)
                        .is_some_and(|rating| rating.value() >= 60)
                })
        })
        .count()
}

/// Street muscle is geographically finite. Associates defend only the ward
/// where their racket operates. Capable named members can cover other wards,
/// but must divide their attention across the organization's active commercial
/// districts and active racket posts rather than guarding every business at
/// full strength from anywhere in the city. People actually executing an
/// operation (including a decision pause) cannot defend another location.
/// Merely authorizing future work does not remove current defensive capacity.
///
/// The allocation is deterministic and recomputed from live world state:
/// sorted district IDs receive the remainder when named muscle cannot be
/// divided evenly. It is not an extra mutable staffing or territory currency.
pub fn local_defensive_force(
    state: &AppState,
    organization: OrganizationId,
    district: NeighborhoodId,
) -> u32 {
    let active = active_district_force(state, organization, district);
    let named = available_named_enforcers(state, organization);
    let mut districts = BTreeSet::new();
    for racket in state.enterprises().active_for_organization(organization) {
        if let Some(ward) = district_of_racket(state, racket.location()) {
            districts.insert(ward);
        }
    }
    for shop in state.world().businesses_owned_by_organization(organization) {
        if state
            .economy()
            .get_business_economy(shop.id())
            .is_some_and(|economy| {
                economy.status() == crate::economy::BusinessOperatingStatus::Active
            })
        {
            districts.insert(shop.neighborhood());
        }
    }
    let Some(rank) = districts.iter().position(|ward| *ward == district) else {
        return active;
    };
    let per_district = named / districts.len();
    let remainder = named % districts.len();
    let assigned = per_district + usize::from(rank < remainder);
    active.saturating_add(u32::try_from(assigned).unwrap_or(u32::MAX))
}

pub fn suppression_ends_at(state: &AppState, racket: EnterpriseId) -> Option<SimTime> {
    let event = state.history().latest_racket_suppression(racket)?;
    let end = event
        .occurred_at()
        .as_minutes()
        .checked_add(CLASH_SUPPRESSION_DAYS * DAY_MINUTES)?;
    (state.now().as_minutes() < end).then_some(SimTime::from_minutes(end))
}

fn next_clash_allowed_at(
    state: &AppState,
    attacker: OrganizationId,
    district: NeighborhoodId,
) -> Option<SimTime> {
    let event = state.history().latest_territory_clash(attacker, district)?;
    let end = event
        .occurred_at()
        .as_minutes()
        .checked_add(CLASH_COOLDOWN_DAYS * DAY_MINUTES)?;
    (state.now().as_minutes() < end).then_some(SimTime::from_minutes(end))
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
    // A bilateral ceasefire forbids fighting the named counterparty, not defending
    // against an unrelated gang or challenging a third party in the same district.
    // The separate expansion and business-operation restrictions still apply to
    // either signatory throughout the agreement.
    if active_standstill(state, attacker, district)
        .is_some_and(|standstill| standstill.counterparty == defender)
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

/// Actual and publicly inferable rank-and-file for a single gang in a district.
/// One daily pass constructs this snapshot: repeatedly resumming all rackets inside each
/// attacker/defender pairing would make autonomous planning cubic in the local racket count.
#[derive(Default)]
struct DistrictRackets {
    actual_force: u32,
    baseline_force: u32,
    targets: Vec<(EnterpriseId, u16)>,
}

fn recently_repulsed(state: &AppState, attacker: OrganizationId, district: NeighborhoodId) -> bool {
    state
        .history()
        .latest_territory_repulse(attacker, district)
        .is_some_and(|event| {
            state
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
    // Attacker-specific real staff and defender-specific visible baseline are distinct:
    // secret defender hires may still turn a seemingly safe raid into a real defeat.
    let mut districts =
        BTreeMap::<NeighborhoodId, BTreeMap<OrganizationId, DistrictRackets>>::new();
    for racket in state.enterprises().active_enterprises() {
        let Some(district) = district_of_racket(state, racket.location()) else {
            continue;
        };
        let position = districts
            .entry(district)
            .or_default()
            .entry(racket.organization())
            .or_default();
        position.actual_force += u32::from(racket.associates());
        position.baseline_force += u32::from(racket.kind().baseline_associates());
        position
            .targets
            .push((racket.id(), racket.kind().baseline_associates()));
    }
    // An NPC chooses among eligible raids by its *estimated* staffing advantage, not by
    // when a rival front happened to receive its entity ID. A larger visible baseline
    // workforce on the target itself breaks equal-risk ties: disrupting that front has
    // the larger immediate staffing impact. Never consult hidden defender hires here.
    let mut attacker_districts = BTreeMap::<OrganizationId, Vec<NeighborhoodId>>::new();
    for (&district, positions) in &districts {
        for &attacker in positions.keys() {
            if state.player_organization() != Some(attacker) {
                attacker_districts
                    .entry(attacker)
                    .or_default()
                    .push(district);
            }
        }
    }
    let mut results = Vec::new();
    for (attacker, local_districts) in attacker_districts {
        // Earlier clashes in this pass can remove a gang's own workers or a rival
        // front. Rank the current footprint at each gang's turn, not a frozen list
        // of attacks computed before any of the losses.
        let mut targets = Vec::new();
        for district in local_districts {
            let positions = &districts[&district];
            let own = &positions[&attacker];
            if own.targets.is_empty() || recently_repulsed(state, attacker, district) {
                continue;
            }
            for (&defender, rival) in positions {
                if attacker == defender
                    || own.actual_force <= rival.baseline_force.saturating_add(DEFENSIVE_ADVANTAGE)
                {
                    continue;
                }
                let estimated_margin =
                    own.actual_force - rival.baseline_force - DEFENSIVE_ADVANTAGE;
                targets.extend(rival.targets.iter().copied().map(|(target, baseline)| {
                    (
                        Reverse(estimated_margin),
                        Reverse(baseline),
                        district,
                        target,
                    )
                }));
            }
        }
        targets.sort_unstable();
        for (_, _, district, target) in targets {
            if let Ok(plan) = validate_territory_clash(state, attacker, target)
                && let Ok(outcome) = plan.commit(state)
            {
                let lost = state
                    .enterprises()
                    .get_enterprise(outcome.suspended_racket)
                    .expect("a suspended racket remains in the enterprise history");
                let position = districts
                    .get_mut(&district)
                    .and_then(|positions| positions.get_mut(&lost.organization()))
                    .expect("the suspended racket belonged to the active footprint");
                position.actual_force -= u32::from(lost.associates());
                position.baseline_force -= u32::from(lost.kind().baseline_associates());
                position.targets.retain(|(id, _)| *id != lost.id());
                results.push(outcome);
                break;
            }
        }
    }
    results
}
