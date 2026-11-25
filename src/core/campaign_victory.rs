//! Optional, record-backed campaign endings. Sandbox play never ends for merely meeting a threshold:
//! the player must explicitly claim a qualified victory. A typed history event freezes the result
//! through the current-schema history and persistence owners.

use crate::contacts::ContactKind;
use crate::contacts::contact_system::usable_contacts_of_kind;
use crate::core::id::{
    BusinessId, CharacterId, HistoryEventId, IdExhaustionError, IdKind, LedgerTransactionId,
    NeighborhoodId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, ensure_time_current};
use crate::economy::business_economy_system::portfolio_overhead_basis_points;
use crate::enterprises::EnterpriseStatus;
use crate::finance::finance_system::FinanceError;
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::history::history_system::{HistoryError, validate_campaign_victory_event};
use crate::politics::{MAYORAL_CAMPAIGN_SPEND_REQUIRED, MAYORAL_LIQUID_RESERVE_REQUIRED};
use crate::registry::Registry;
use crate::reports::report_system::ReportError;
use crate::world::BusinessKind;
use crate::world::territory_influence::{
    NeighborhoodInfluenceSummary, resolve_neighborhood_influence,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod certification_report;
mod command_staffing;
mod commercial_standing;
mod political_staffing;
mod retirement;
mod underboss;
use certification_report::prepare_victory_report;
use command_staffing::staffed_racket_districts;
use commercial_standing::{business_district_standings, political_district_rows};
pub use commercial_standing::{political_business_progress, political_district_progress};
pub(crate) use retirement::retirement_settlement_matches;
use retirement::{ValidatedRetirementSettlement, prepare_retirement_payment, retirement_members};
use underboss::{institutional_reach, operating_reserve, senior_command_count};

/// These are ending choices, not automatic achievements. Variant order is persistent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VictoryPath {
    PoliticalPower,
    FundedRetirement,
    CityUnderboss,
}

impl VictoryPath {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PoliticalPower => "mayor of the city",
            Self::FundedRetirement => "funded retirement",
            Self::CityUnderboss => "city underboss",
        }
    }
}

/// A late-game personal retirement fund. Crew severance is additional, not part of this amount.
pub const RETIREMENT_NEST_EGG: Money = Money::from_cents(100_000_000);
pub const RETIREMENT_SEVERANCE_DAYS: i64 = 180;
/// Retirement is an exit from a criminal career, not an idle landlord ending.
/// Political transition is the separate route for a long-term clean operator.
pub const RETIREMENT_MIN_KEY_PERSONNEL: usize = 4;
pub const RETIREMENT_CRIME_WINDOW_DAYS: u64 = 180;
pub const RETIREMENT_MIN_PROFITABLE_RACKET_CYCLES: usize = 90;
pub const RETIREMENT_MIN_RACKET_NET: Money = Money::from_cents(1_000_000);
/// Economic and territorial coalitions must prove they can sustain operations.
pub const VICTORY_ESTABLISHMENT_DAYS: u64 = 90;
/// Civic districts need three years of stable ownership; underbosses still use
/// the separate 90-day operational proof.
pub const POLITICAL_BUSINESS_ESTABLISHMENT_DAYS: u64 = 1_080;
pub const POLITICAL_CONTACT_DAYS: u64 = 180;
/// A full year without active rackets or criminal operating settlements is
/// evidence of an actual transition from racketeering to civic business.
pub const POLITICAL_CLEAN_OPERATION_DAYS: u64 = 365;
/// A citywide campaign needs a central leadership team and independently staffed
/// political *and* financial directors in every qualifying district.
pub const POLITICAL_HEADQUARTERS_PERSONNEL: usize = 4;
pub const POLITICAL_STAFF_PER_DISTRICT: usize = 2;
/// Local civic support must span multiple established kinds of commerce.
pub const POLITICAL_BUSINESS_KINDS_PER_DISTRICT: usize = 2;
pub const POLITICAL_BUSINESSES_PER_DISTRICT: usize = 3;
pub const VICTORY_PROFITABLE_CYCLES: usize = 20;
pub const VICTORY_INCOME_WINDOW_DAYS: u64 = 30;
const VICTORY_FRESH_SETTLEMENT_DAYS: u64 = 7;
/// Named members are the organization's command cadre, not a complete simulation of
/// every runner or employee working below its racket managers. Even a small city
/// needs a headquarters reserve and multiple leaders per controlled district before it can
/// plausibly claim control of the entire underworld.
pub const UNDERBOSS_MIN_KEY_PERSONNEL: usize = 24;
pub const UNDERBOSS_HEADQUARTERS_PERSONNEL: usize = 6;
pub const UNDERBOSS_KEY_PERSONNEL_PER_DISTRICT: usize = 6;
pub const UNDERBOSS_CAPTAINS_PER_DISTRICT: usize = 2;
/// Six months of named payroll plus three months of the actual current racket
/// operating costs, including associates and the maximum modeled heat surcharge.
pub const UNDERBOSS_LIQUID_RUNWAY_DAYS: i128 = 180;
pub const UNDERBOSS_OPERATING_RUNWAY_DAYS: u64 = 90;
/// Each controlled district must support a substantial proven local workforce,
/// a diversified portfolio, and available named defenders.
pub const UNDERBOSS_ASSOCIATES_PER_DISTRICT: u64 = 30;
pub const UNDERBOSS_RACKET_KINDS_PER_DISTRICT: usize = 2;
pub const UNDERBOSS_ENFORCERS_PER_DISTRICT: u32 = 3;
pub const UNDERBOSS_SENIOR_MANAGEMENT_RATING: u8 = 70;
pub const UNDERBOSS_CONTACT_DAYS: u64 = 90;
pub const UNDERBOSS_CONTACT_KINDS: usize = 3;
pub const UNDERBOSS_CONTACT_HANDLERS: usize = 2;
/// Underworld influence is not an electoral majority, but it must reach civic institutions.
pub const UNDERBOSS_LOCAL_MACHINE_INFLUENCE: u16 = 200;
pub const UNDERBOSS_LOCAL_INSTITUTIONAL_SUPPORT: u16 = 100;

fn required_underboss_personnel(required_districts: usize) -> usize {
    UNDERBOSS_MIN_KEY_PERSONNEL.max(
        UNDERBOSS_HEADQUARTERS_PERSONNEL.saturating_add(
            required_districts.saturating_mul(UNDERBOSS_KEY_PERSONNEL_PER_DISTRICT),
        ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VictoryProgress {
    pub total_districts: usize,
    /// Current named leadership, not an estimate of unnamed racket employees.
    pub key_personnel: usize,
    pub required_political_personnel: usize,
    pub required_underboss_personnel: usize,
    pub required_political_districts: usize,
    pub owned_operating_business_districts: usize,
    pub active_political_contacts: usize,
    pub established_political_contacts: usize,
    pub established_business_districts: usize,
    /// Economically qualified districts with distinct, active political organizers.
    pub staffed_political_districts: usize,
    /// Qualified local constituencies with sustained, actively defended electoral majorities.
    pub mayoral_majority_districts: usize,
    pub campaign_spent_cents: i64,
    pub required_campaign_spent_cents: i64,
    pub required_mayoral_reserve_cents: i64,
    pub mayoral_leader_eligible: bool,
    pub legitimate_transition_ready: bool,
    pub days_since_last_racket_cycle: Option<u64>,
    pub mayoral_nominated_for_day: Option<u64>,
    pub latest_election_day: Option<u64>,
    pub latest_election_won: bool,
    pub required_underboss_districts: usize,
    pub own_active_racket_districts: usize,
    pub established_racket_districts: usize,
    pub mature_associates: u64,
    pub workforce_ready_districts: usize,
    pub defended_underboss_districts: usize,
    /// The same independently captained districts also possess current civic leverage.
    pub influential_underboss_districts: usize,
    pub available_underboss_personnel: usize,
    pub senior_underboss_commands: usize,
    pub underboss_contact_kinds: usize,
    pub underboss_contact_handlers: usize,
    pub underboss_operating_buffer: Money,
    /// Current specialist contracts, including temporarily inactive offices.
    pub campaign_daily_payroll: Money,
    pub underboss_campaign_buffer: Money,
    /// Uncontested mature profitable districts with independent, non-reused captains.
    pub staffed_underboss_districts: usize,
    /// Mature operating earnings less net losses from other recent title periods, over 30 days.
    pub legitimate_daily_net: Money,
    /// Share of earned legitimate profits lost to current portfolio administration.
    pub portfolio_overhead_basis_points: u16,
    /// Mature operating earnings less net losses from other enterprises, over 30 days.
    pub racket_daily_net: Money,
    pub daily_payroll: Money,
    /// Whether mature legitimate operations can cover the organization's wages.
    pub legitimate_self_sufficient: bool,
    /// Whether mature criminal enterprises can cover the organization's wages.
    pub racket_self_sufficient: bool,
    /// Liquid gang treasury after current account deficits; storefront tills
    /// and non-liquid settlement accounts do not count toward this reserve.
    pub underboss_liquid_reserve: Money,
    pub required_underboss_liquid_reserve: Money,
    pub accounted_net: Money,
    pub retirement_total: Money,
    /// Recent settled proceeds and profitable cycles, including retired rackets.
    pub retirement_racket_net: Money,
    pub retirement_profitable_racket_cycles: usize,
    /// Only valid in a full simulation context; do not expose hidden rival standing in the player client.
    underboss_ready: bool,
}

/// Political progress using only the player's owned business titles and settlements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoliticalDistrictProgress {
    pub district: NeighborhoodId,
    pub active_owned: usize,
    pub suspended_owned: usize,
    /// Active businesses continuously owned for the political establishment period.
    pub mature_active: usize,
    /// Mature businesses with fresh settlements and at least 20 profitable cycles in 30 days.
    pub established_active: usize,
    /// Distinct business sectors with 20 profitable recent settlements each.
    pub established_kinds: usize,
    /// Possible local political delegates, prior to citywide one-person-one-district matching.
    pub available_stewards: usize,
    pub available_financial_stewards: usize,
    /// Assigned organizer, not reusable in another qualifying district.
    pub steward: Option<CharacterId>,
    /// Separate assigned financial director, not reused as any other district leader.
    pub financial_steward: Option<CharacterId>,
    /// Last 30 days' mature active storefront net, less other own title-period losses.
    pub mature_window_net: Money,
    /// Mature diversified commerce and sufficient realized local earnings.
    pub economically_qualified: bool,
    /// Economically qualified and independently staffed with a political delegate.
    pub qualifies: bool,
    pub own_influence: u16,
    pub grassroots_support: u16,
    pub institutional_support: u16,
    pub rival_influence: u16,
    pub majority_held_days: u64,
    pub mayoral_ready: bool,
}

/// Player-owned title and settlement evidence behind political readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoliticalBusinessProgress {
    pub business: BusinessId,
    pub kind: BusinessKind,
    pub active: bool,
    pub owned_days: u64,
    pub days_until_mature: u64,
    pub profitable_cycles: usize,
    pub last_owned_settlement: Option<SimTime>,
    pub recent_net: Money,
    pub established: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum VictoryError {
    #[error("no player organization is designated")]
    NoPlayer,
    #[error("Campaign over: your organization dissolved")]
    Defeated,
    #[error("Campaign over: victory has already been claimed")]
    AlreadyWon,
    #[error(
        "becoming mayor requires a long-standing political contact, a year without operating rackets, durable profitable diverse businesses and independent political delegates, sustained competitive majorities, major clean campaign expenditures and reserves, and legitimate income covering payroll"
    )]
    PoliticalNotReady,
    #[error(
        "funded retirement needs four named crew, 90 profitable racket settlements and $10,000 net racket income in the last 180 days, one leader, no pending decisions or active operations, and clean money for the $1,000,000 nest egg plus crew severance"
    )]
    RetirementNotReady,
    #[error(
        "city underboss requires available command staff, proven independent district captains, an experienced senior commander, mature diversified crews with local defenders and civic leverage in three quarters of districts, durable institutional reach, superiority over combined rivals, payroll coverage and liquid payroll plus operating reserves"
    )]
    UnderbossNotReady,
    #[error("retirement payment arithmetic overflow")]
    Arithmetic,
    #[error("campaign outcome changed since validation")]
    Stale,
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

fn established_for(now: SimTime, since: SimTime, days: u64) -> bool {
    now.as_minutes().saturating_sub(since.as_minutes()) >= days * DAY_MINUTES
}

fn settled_recently(now: SimTime, settled: SimTime) -> bool {
    settled <= now
        && now.as_minutes() - settled.as_minutes() <= VICTORY_FRESH_SETTLEMENT_DAYS * DAY_MINUTES
}

fn recent_income_start(now: SimTime) -> u64 {
    now.as_minutes()
        .saturating_sub(VICTORY_INCOME_WINDOW_DAYS * DAY_MINUTES)
}

/// New rackets may help defend the district today, but they cannot be used to
/// declare a long-established underboss majority. Established operating crews
/// themselves must outnumber the combined current rival presence.
fn mature_territorial_majority(
    summary: &NeighborhoodInfluenceSummary,
    organization: OrganizationId,
    mature_count: u64,
) -> bool {
    summary.economic_leader() == Some(organization)
        && summary
            .standings
            .iter()
            .filter(|standing| standing.organization != organization)
            .map(|standing| u64::from(standing.active_enterprises))
            .sum::<u64>()
            < mature_count
}

fn recent_racket_net(state: &AppState, racket: &crate::enterprises::EnterpriseRecord) -> i128 {
    state
        .enterprises()
        .cycles_for(racket.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| cycle.occurred_at() <= state.now())
        .map(|cycle| i128::from(cycle.net_cash().cents()))
        .sum()
}

/// Criminal proceeds earned recently, not money from property ownership or sales.
/// Past settlements remain relevant when a racket has been shut down or retired.
fn retirement_racket_record(state: &AppState, owner: OrganizationId) -> (i128, usize) {
    let cutoff = state
        .now()
        .as_minutes()
        .saturating_sub(RETIREMENT_CRIME_WINDOW_DAYS * DAY_MINUTES);
    let mut net = 0_i128;
    let mut profitable = 0_usize;
    for racket in state.enterprises().enterprises_for_organization(owner) {
        for cycle in state
            .enterprises()
            .cycles_for(racket.id())
            .rev()
            .take_while(|cycle| cycle.occurred_at().as_minutes() > cutoff)
        {
            if cycle.occurred_at() <= state.now() {
                net += i128::from(cycle.net_cash().cents());
                if cycle.net_cash() > Money::ZERO {
                    profitable += 1;
                }
            }
        }
    }
    (net, profitable)
}

fn established_racket(state: &AppState, racket: &crate::enterprises::EnterpriseRecord) -> bool {
    if !established_for(
        state.now(),
        racket.established_at(),
        VICTORY_ESTABLISHMENT_DAYS,
    ) || !racket
        .last_cycle_at()
        .is_some_and(|at| settled_recently(state.now(), at))
    {
        return false;
    }
    state
        .enterprises()
        .cycles_for(racket.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| cycle.occurred_at() <= state.now() && cycle.net_cash() > Money::ZERO)
        .take(VICTORY_PROFITABLE_CYCLES)
        .count()
        == VICTORY_PROFITABLE_CYCLES
}

/// The current captain must have personally directed a recent profitable run.
/// Reassigning a mature racket cannot confer an instant record of local command.
fn proven_racket_captain(state: &AppState, racket: &crate::enterprises::EnterpriseRecord) -> bool {
    state
        .enterprises()
        .cycles_for(racket.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| {
            cycle.occurred_at() <= state.now()
                && racket.authority_for_cycle_at(cycle.occurred_at()).manager == racket.manager()
                && cycle.net_cash() > Money::ZERO
        })
        .take(VICTORY_PROFITABLE_CYCLES)
        .count()
        == VICTORY_PROFITABLE_CYCLES
}

/// Extra people hired today do not confer instant territorial strength.
/// The current captain must have directed this actual complement through
/// twenty profitable settlements in the most recent thirty days.
fn proven_racket_associates(
    state: &AppState,
    racket: &crate::enterprises::EnterpriseRecord,
) -> u64 {
    let proven_cycles = state
        .enterprises()
        .cycles_for(racket.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| {
            cycle.occurred_at() <= state.now()
                && cycle.net_cash() > Money::ZERO
                && racket.associates_at(cycle.occurred_at()) >= racket.associates()
                && racket.authority_for_cycle_at(cycle.occurred_at()).manager == racket.manager()
        })
        .take(VICTORY_PROFITABLE_CYCLES)
        .count();
    if proven_cycles == VICTORY_PROFITABLE_CYCLES {
        u64::from(racket.associates())
    } else {
        0
    }
}

pub fn progress(registry: &Registry, state: &AppState) -> Result<VictoryProgress, VictoryError> {
    Ok(progress_and_political_districts(registry, state)?.0)
}

/// Single assessment for consumers that need aggregate victory progress
/// and the district teams that produced it. Do not solve the same exact
/// citywide matching twice for one election or dossier.
pub fn progress_and_political_districts(
    registry: &Registry,
    state: &AppState,
) -> Result<(VictoryProgress, Vec<PoliticalDistrictProgress>), VictoryError> {
    let organization = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let total_districts = state.world().neighborhoods().count();
    let key_personnel = state
        .world()
        .characters_in_organization(organization)
        .count();
    let required_political_districts = (total_districts * 3).div_ceil(4);
    let required_political_personnel = required_political_districts
        .saturating_mul(POLITICAL_STAFF_PER_DISTRICT)
        .saturating_add(POLITICAL_HEADQUARTERS_PERSONNEL);
    let required_underboss_districts = (total_districts * 3).div_ceil(4);
    let required_underboss_personnel = required_underboss_personnel(required_underboss_districts);
    let per_member_wage = i128::from(registry.upkeep().per_member_daily().cents());
    let daily_payroll =
        per_member_wage * i128::try_from(key_personnel).expect("member count fits i128");
    let income_window = i128::from(VICTORY_INCOME_WINDOW_DAYS);
    // A local holding company must fund at least one wage from earned net income.
    // The organization as a whole must cover *all* named directors' actual wages.
    let local_income_floor = per_member_wage * income_window;
    let business_standings = business_district_standings(state, organization);
    let owned_operating_business_districts = business_standings
        .values()
        .filter(|standing| standing.active > 0)
        .count();
    let established_business_districts = business_standings
        .values()
        .filter(|standing| standing.qualifies(local_income_floor))
        .count();
    let political_rows =
        political_district_rows(state, organization, &business_standings, local_income_floor);
    let staffed_political_districts = political_rows
        .iter()
        .filter(|district| district.qualifies)
        .count();
    let mayoral_majority_districts = political_rows
        .iter()
        .filter(|district| district.mayoral_ready)
        .count();
    let campaign_spent_cents = state
        .politics()
        .organization(organization)
        .map_or(0, |campaign| campaign.spent_cents());
    let mayoral_nominated_for_day = state
        .politics()
        .organization(organization)
        .and_then(|campaign| campaign.candidacy())
        .map(|candidacy| candidacy.election_day);
    let latest_election = state
        .politics()
        .organization(organization)
        .and_then(|campaign| campaign.latest_election());
    let mayoral_leader_eligible =
        crate::world::world_system::sole_available_executive(state, organization).is_some();
    let legitimate_window_net = business_standings
        .values()
        .map(|standing| standing.window_net)
        .sum::<i128>();
    let political_contacts =
        usable_contacts_of_kind(state, organization, ContactKind::Political).collect::<Vec<_>>();
    let active_political_contacts = political_contacts.len();
    let established_political_contacts = political_contacts
        .iter()
        .filter(|contact| {
            established_for(
                state.now(),
                contact.established_at(),
                POLITICAL_CONTACT_DAYS,
            )
        })
        .count();
    // A racketeer's district footprint is a set of its live operating locations.
    // Traverse the indexed roster once instead of rescanning every enterprise for
    // every neighborhood when the player opens the victory-progress view.
    let own_rackets = state
        .enterprises()
        .enterprises_for_organization(organization)
        .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
        .collect::<Vec<_>>();
    let racket_district =
        |enterprise: &crate::enterprises::EnterpriseRecord| match enterprise.location() {
            crate::enterprises::EnterpriseLocation::Neighborhood(id) => Some(id),
            crate::enterprises::EnterpriseLocation::Business(id) => state
                .world()
                .get_business(id)
                .map(|business| business.neighborhood()),
        };
    let own_active_racket_districts = own_rackets
        .iter()
        .filter_map(|enterprise| racket_district(enterprise))
        .collect::<BTreeSet<_>>()
        .len();
    let days_since_last_racket_cycle = state
        .enterprises()
        .enterprises_for_organization(organization)
        .filter_map(|racket| racket.last_cycle_at())
        .max()
        .map(|last| state.now().as_minutes().saturating_sub(last.as_minutes()) / DAY_MINUTES);
    let legitimate_transition_ready = own_rackets.is_empty()
        && days_since_last_racket_cycle.is_none_or(|days| days >= POLITICAL_CLEAN_OPERATION_DAYS);
    let mut rackets_by_district: BTreeMap<_, (i128, u64)> = BTreeMap::new();
    let mut captains_by_district = BTreeMap::<NeighborhoodId, BTreeSet<CharacterId>>::new();
    let mut mature_workers_by_district = BTreeMap::<NeighborhoodId, u64>::new();
    let mut mature_kinds_by_district =
        BTreeMap::<NeighborhoodId, BTreeSet<crate::enterprises::EnterpriseKind>>::new();
    for racket in state
        .enterprises()
        .enterprises_for_organization(organization)
    {
        if let Some(district) = racket_district(racket) {
            let recent_net = recent_racket_net(state, racket);
            let (net, count) = rackets_by_district.entry(district).or_default();
            let established =
                racket.status() == EnterpriseStatus::Active && established_racket(state, racket);
            *net += if established {
                recent_net
            } else {
                recent_net.min(0)
            };
            if established && recent_net > 0 {
                *count += 1;
                if proven_racket_captain(state, racket) {
                    captains_by_district
                        .entry(district)
                        .or_default()
                        .insert(racket.manager());
                    let proven_workers = proven_racket_associates(state, racket);
                    *mature_workers_by_district.entry(district).or_default() += proven_workers;
                    if proven_workers > 0 {
                        mature_kinds_by_district
                            .entry(district)
                            .or_default()
                            .insert(racket.kind());
                    }
                }
            }
        }
    }
    let established_racket_districts = rackets_by_district
        .iter()
        .filter(|(_, (net, count))| *count > 0 && *net >= local_income_floor && *net > 0)
        .map(|(district, _)| *district)
        .collect::<BTreeSet<_>>();
    let racket_window_net = rackets_by_district
        .values()
        .map(|(net, _)| net)
        .sum::<i128>();
    let uncontested_districts = established_racket_districts
        .iter()
        .filter(|district| {
            let mature_count = rackets_by_district
                .get(district)
                .expect("qualifying district came from the mature racket map")
                .1;
            resolve_neighborhood_influence(state, **district).is_ok_and(|summary| {
                mature_territorial_majority(&summary, organization, mature_count)
            })
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let mut rival_workers_by_district = BTreeMap::<NeighborhoodId, u64>::new();
    for racket in state
        .enterprises()
        .active_enterprises()
        .filter(|racket| racket.organization() != organization)
    {
        if let Some(district) = racket_district(racket) {
            *rival_workers_by_district.entry(district).or_default() +=
                u64::from(racket.associates());
        }
    }
    let workforce_ready = uncontested_districts
        .iter()
        .filter(|&&district| {
            let workers = mature_workers_by_district
                .get(&district)
                .copied()
                .unwrap_or(0);
            workers >= UNDERBOSS_ASSOCIATES_PER_DISTRICT
                && mature_kinds_by_district
                    .get(&district)
                    .is_some_and(|kinds| kinds.len() >= UNDERBOSS_RACKET_KINDS_PER_DISTRICT)
                && workers
                    > rival_workers_by_district
                        .get(&district)
                        .copied()
                        .unwrap_or(0)
        })
        .copied()
        .collect::<BTreeSet<_>>();
    // Territorial control, proven labor, named defenders and independent command
    // must hold in the SAME districts, not in unrelated sets of locations.
    let defended_districts = workforce_ready
        .iter()
        .filter(|&&district| {
            let force = crate::world::territory_conflict::local_defensive_force(
                state,
                organization,
                district,
            );
            let associates = crate::world::territory_conflict::active_district_force(
                state,
                organization,
                district,
            );
            force.saturating_sub(associates) >= UNDERBOSS_ENFORCERS_PER_DISTRICT
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let staffed_underboss_districts =
        staffed_racket_districts(&defended_districts, &captains_by_district);
    let influential_districts = defended_districts
        .iter()
        .filter(|&&district| {
            state
                .politics()
                .organization(organization)
                .and_then(|campaign| campaign.district(district))
                .is_some_and(|office| {
                    office.influence() >= UNDERBOSS_LOCAL_MACHINE_INFLUENCE
                        && office.institutions() >= UNDERBOSS_LOCAL_INSTITUTIONAL_SUPPORT
                })
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let influential_underboss_districts =
        staffed_racket_districts(&influential_districts, &captains_by_district);
    let available_underboss_personnel = state
        .world()
        .characters_in_organization(organization)
        .filter(|person| {
            state
                .legal()
                .active_arrest_for_character(person.id())
                .is_none()
        })
        .count();
    let senior_underboss_commands = senior_command_count(
        state,
        organization,
        &defended_districts,
        &captains_by_district,
    );
    let (underboss_contact_kinds, underboss_contact_handlers) =
        institutional_reach(state, organization);
    let rival_active = state
        .enterprises()
        .active_enterprises()
        .filter(|racket| racket.organization() != organization)
        .count();
    let mature_racket_count = rackets_by_district
        .values()
        .map(|(_, count)| *count)
        .sum::<u64>();
    let mature_associates = mature_workers_by_district.values().sum::<u64>();
    let rival_associates = rival_workers_by_district.values().sum::<u64>();
    // Include negative accounted balances; a debt cannot be ignored by spending another pocket.
    let net = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| i128::from(account.balance().cents()))
        .sum::<i128>()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    let underboss_liquid_reserve = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash | AccountKind::AccountedFunds
            )
        })
        .map(|account| {
            let balance = i128::from(account.balance().cents());
            if balance < 0 {
                balance
            } else {
                i128::from(account.available_payment_balance().cents())
            }
        })
        .sum::<i128>();
    let underboss_operating_buffer = operating_reserve(registry, state, &own_rackets)?;
    let campaign_daily_payroll =
        crate::politics::political_system::campaign_salary_commitment(state, organization)
            .contracted;
    let underboss_campaign_buffer =
        i128::from(campaign_daily_payroll.cents()) * i128::from(UNDERBOSS_OPERATING_RUNWAY_DAYS);
    let required_underboss_reserve = daily_payroll
        .saturating_mul(UNDERBOSS_LIQUID_RUNWAY_DAYS)
        .saturating_add(underboss_operating_buffer)
        .saturating_add(underboss_campaign_buffer);
    let retirement_total = retirement_members(registry, state, organization)
        .map(|(_, total)| total)
        .unwrap_or(RETIREMENT_NEST_EGG);
    let (retirement_net, retirement_profitable_racket_cycles) =
        retirement_racket_record(state, organization);
    let standing = VictoryProgress {
        total_districts,
        key_personnel,
        required_political_personnel,
        required_underboss_personnel,
        required_political_districts,
        owned_operating_business_districts,
        active_political_contacts,
        established_political_contacts,
        established_business_districts,
        staffed_political_districts,
        mayoral_majority_districts,
        campaign_spent_cents,
        required_campaign_spent_cents: MAYORAL_CAMPAIGN_SPEND_REQUIRED,
        required_mayoral_reserve_cents: MAYORAL_LIQUID_RESERVE_REQUIRED,
        mayoral_leader_eligible,
        legitimate_transition_ready,
        days_since_last_racket_cycle,
        mayoral_nominated_for_day,
        latest_election_day: latest_election.map(|election| election.election_day),
        latest_election_won: latest_election.is_some_and(|election| election.elected),
        required_underboss_districts,
        own_active_racket_districts,
        established_racket_districts: established_racket_districts.len(),
        mature_associates,
        workforce_ready_districts: workforce_ready.len(),
        defended_underboss_districts: defended_districts.len(),
        influential_underboss_districts,
        available_underboss_personnel,
        senior_underboss_commands,
        underboss_contact_kinds,
        underboss_contact_handlers,
        underboss_operating_buffer: Money::from_cents(
            underboss_operating_buffer.clamp(0, i128::from(i64::MAX)) as i64,
        ),
        campaign_daily_payroll,
        underboss_campaign_buffer: Money::from_cents(
            underboss_campaign_buffer.clamp(0, i128::from(i64::MAX)) as i64,
        ),
        staffed_underboss_districts,
        legitimate_daily_net: Money::from_cents(
            (legitimate_window_net / income_window)
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
        portfolio_overhead_basis_points: portfolio_overhead_basis_points(state, organization),
        racket_daily_net: Money::from_cents(
            (racket_window_net / income_window).clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                as i64,
        ),
        daily_payroll: Money::from_cents(
            daily_payroll.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
        legitimate_self_sufficient: daily_payroll > 0
            && legitimate_window_net >= daily_payroll * income_window,
        racket_self_sufficient: daily_payroll > 0
            && racket_window_net >= daily_payroll * income_window,
        underboss_liquid_reserve: Money::from_cents(
            underboss_liquid_reserve.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
        required_underboss_liquid_reserve: Money::from_cents(
            required_underboss_reserve.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
        accounted_net: Money::from_cents(net as i64),
        retirement_total,
        retirement_racket_net: Money::from_cents(
            retirement_net.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
        retirement_profitable_racket_cycles,
        underboss_ready: total_districts > 0
            && uncontested_districts.len() >= required_underboss_districts
            && available_underboss_personnel >= required_underboss_personnel
            && influential_underboss_districts >= required_underboss_districts
            && senior_underboss_commands > 0
            && underboss_contact_kinds >= UNDERBOSS_CONTACT_KINDS
            && underboss_contact_handlers >= UNDERBOSS_CONTACT_HANDLERS
            && mature_racket_count > rival_active as u64
            && mature_associates > rival_associates
            && mayoral_leader_eligible
            && daily_payroll > 0
            && racket_window_net >= daily_payroll * income_window
            && underboss_liquid_reserve >= required_underboss_reserve,
    };
    Ok((standing, political_rows))
}

/// Campaign readiness is tested at the municipal ballot, not rewarded as an
/// immediate victory. The election owner uses this same threshold snapshot.
pub(crate) fn mayoral_ballot_ready(standing: &VictoryProgress) -> bool {
    standing.legitimate_transition_ready
        && standing.established_political_contacts > 0
        && standing.total_districts > 0
        && standing.established_business_districts >= standing.required_political_districts
        && standing.key_personnel >= standing.required_political_personnel
        && standing.staffed_political_districts >= standing.required_political_districts
        && standing.mayoral_majority_districts >= standing.required_political_districts
        && standing.campaign_spent_cents >= MAYORAL_CAMPAIGN_SPEND_REQUIRED
        && standing.accounted_net.cents() >= MAYORAL_LIQUID_RESERVE_REQUIRED
        && standing.mayoral_leader_eligible
        && standing.legitimate_self_sufficient
}

fn ensure_eligible(
    registry: &Registry,
    state: &AppState,
    path: VictoryPath,
) -> Result<VictoryProgress, VictoryError> {
    if state.player_organization_defeated() {
        return Err(VictoryError::Defeated);
    }
    if state.campaign_victory().is_some() {
        return Err(VictoryError::AlreadyWon);
    }
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let standing = progress(registry, state)?;
    match path {
        VictoryPath::PoliticalPower
            if !state
                .politics()
                .organization(owner)
                .and_then(|campaign| campaign.latest_election())
                .is_some_and(|election| {
                    election.elected
                        && state
                            .world()
                            .get_character(election.candidate)
                            .is_some_and(|person| {
                                person.organization() == Some(owner)
                                    && person.supervisor().is_none()
                            })
                }) =>
        {
            Err(VictoryError::PoliticalNotReady)
        }
        VictoryPath::FundedRetirement => {
            let (_, required) = retirement_members(registry, state, owner)?;
            let spendable: i128 = state
                .finance()
                .accounts_for(FinancialOwner::Organization(owner))
                .filter(|account| account.kind() == AccountKind::AccountedFunds)
                .map(|account| i128::from(account.available_payment_balance().cents()))
                .sum();
            if standing.key_personnel < RETIREMENT_MIN_KEY_PERSONNEL
                || standing.retirement_profitable_racket_cycles
                    < RETIREMENT_MIN_PROFITABLE_RACKET_CYCLES
                || standing.retirement_racket_net < RETIREMENT_MIN_RACKET_NET
                || standing.accounted_net < required
                || spendable < i128::from(required.cents())
                || state
                    .decisions()
                    .pending_for_recipient(owner)
                    .next()
                    .is_some()
                || state
                    .operations()
                    .operations_for_organization(owner)
                    .any(|operation| {
                        !matches!(
                            operation.status(),
                            crate::operations::OperationStatus::Completed
                                | crate::operations::OperationStatus::Aborted
                        )
                    })
            {
                Err(VictoryError::RetirementNotReady)
            } else {
                Ok(())
            }
        }
        VictoryPath::CityUnderboss if !standing.underboss_ready => {
            Err(VictoryError::UnderbossNotReady)
        }
        VictoryPath::PoliticalPower | VictoryPath::CityUnderboss => Ok(()),
    }?;
    Ok(standing)
}

pub struct ValidatedCampaignVictory {
    owner: OrganizationId,
    path: VictoryPath,
    validated_at: SimTime,
    settlement: Option<ValidatedRetirementSettlement>,
}

pub struct CampaignVictoryOutcome {
    pub history: HistoryEventId,
    pub report: ReportId,
    pub settlement: Option<LedgerTransactionId>,
    pub path: VictoryPath,
}

pub fn validate_claim_victory(
    registry: &Registry,
    state: &AppState,
    path: VictoryPath,
) -> Result<ValidatedCampaignVictory, VictoryError> {
    let standing = ensure_eligible(registry, state, path)?;
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let settlement = if path == VictoryPath::FundedRetirement {
        Some(prepare_retirement_payment(registry, state, owner)?)
    } else {
        None
    };
    // Preflight the report at validation. Its observations are rebuilt at commit
    // after same-minute state changes, before any payment or history mutation.
    prepare_victory_report(registry, state, owner, path, &standing)?;
    let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
    if let Some(payment) = &settlement {
        budget.extend(payment.id_budget());
    }
    state.ids.reserve_many(&budget)?;
    Ok(ValidatedCampaignVictory {
        owner,
        path,
        validated_at: state.now(),
        settlement,
    })
}

impl ValidatedCampaignVictory {
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<CampaignVictoryOutcome, VictoryError> {
        ensure_time_current(state.now(), self.validated_at).map_err(|_| VictoryError::Stale)?;
        if state.player_organization() != Some(self.owner) {
            return Err(VictoryError::Stale);
        }
        let standing = ensure_eligible(registry, state, self.path)?;
        if let Some(settlement) = &self.settlement {
            settlement.ensure_current(registry, state, self.owner)?;
        }
        let report = prepare_victory_report(registry, state, self.owner, self.path, &standing)?;
        let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
        if let Some(settlement) = &self.settlement {
            budget.extend(settlement.id_budget());
        }
        state.ids.reserve_many(&budget)?;
        // Bind history to the next ledger ID only after every held dependency has
        // been rechecked. Unrelated same-minute payments may have advanced it.
        let settlement_id = self
            .settlement
            .as_ref()
            .map(|_| LedgerTransactionId::from_raw(state.ids.next_raw(IdKind::LedgerTransaction)));
        let history = validate_campaign_victory_event(state, self.owner, self.path, settlement_id)?;
        let settlement = self.settlement.map(|plan| plan.commit_preflighted(state));
        debug_assert_eq!(settlement, settlement_id);
        let history = history.commit(state)?;
        let report = report.commit(state)?;
        Ok(CampaignVictoryOutcome {
            history,
            report,
            settlement,
            path: self.path,
        })
    }
}

#[cfg(test)]
mod tests;
