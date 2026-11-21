//! Optional, record-backed campaign endings. Sandbox play never ends for merely meeting a threshold:
//! the player must explicitly claim a qualified victory. A typed history event freezes the result
//! without changing the serialized AppState layout, so existing campaign saves remain loadable.

use crate::contacts::{ContactKind, ContactStatus};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, CharacterId, HistoryEventId, IdExhaustionError, IdKind, LedgerTransactionId,
    NeighborhoodId, OrganizationId, ReportId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, ensure_time_current};
use crate::economy::BusinessOperatingStatus;
use crate::enterprises::EnterpriseStatus;
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::history_system::{
    HistoryError, ValidatedHistoryEvent, validate_campaign_victory_event,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_report_draft};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::territory_influence::{
    NeighborhoodInfluenceSummary, resolve_neighborhood_influence,
};
use crate::world::{BusinessOwner, OrganizationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

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
            Self::PoliticalPower => "legitimate political power",
            Self::FundedRetirement => "funded retirement",
            Self::CityUnderboss => "city underboss",
        }
    }
}

/// A late-game personal retirement fund. Crew severance is additional, not part of this amount.
pub const RETIREMENT_NEST_EGG: Money = Money::from_cents(100_000_000);
pub const RETIREMENT_SEVERANCE_DAYS: i64 = 180;
/// Economic and territorial coalitions must prove they can sustain operations.
pub const VICTORY_ESTABLISHMENT_DAYS: u64 = 90;
pub const POLITICAL_CONTACT_DAYS: u64 = 30;
pub const VICTORY_PROFITABLE_CYCLES: usize = 20;
pub const VICTORY_INCOME_WINDOW_DAYS: u64 = 30;
const VICTORY_FRESH_SETTLEMENT_DAYS: u64 = 7;
/// Named members are the organization's command cadre, not a complete simulation of
/// every runner or employee working below its racket managers. Even a small city
/// needs a headquarters reserve and multiple leaders per controlled district before it can
/// plausibly claim control of the entire underworld.
pub const UNDERBOSS_MIN_KEY_PERSONNEL: usize = 10;
pub const UNDERBOSS_HEADQUARTERS_PERSONNEL: usize = 4;
pub const UNDERBOSS_KEY_PERSONNEL_PER_DISTRICT: usize = 2;
pub const UNDERBOSS_CAPTAINS_PER_DISTRICT: usize = 2;

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
    pub required_underboss_personnel: usize,
    pub required_political_districts: usize,
    pub owned_operating_business_districts: usize,
    pub active_political_contacts: usize,
    pub established_political_contacts: usize,
    pub established_business_districts: usize,
    pub required_underboss_districts: usize,
    pub own_active_racket_districts: usize,
    pub established_racket_districts: usize,
    /// Uncontested mature profitable districts with independent, non-reused captains.
    pub staffed_underboss_districts: usize,
    /// Average realized daily profits from eligible 90-day businesses, over 30 days.
    pub legitimate_daily_net: Money,
    /// Average realized daily profits from eligible 90-day rackets, over 30 days.
    pub racket_daily_net: Money,
    pub daily_payroll: Money,
    /// Whether mature legitimate operations can cover the organization's wages.
    pub legitimate_self_sufficient: bool,
    /// Whether mature criminal enterprises can cover the organization's wages.
    pub racket_self_sufficient: bool,
    pub accounted_net: Money,
    pub retirement_total: Money,
    /// Only valid in a full simulation context; do not expose hidden rival standing in the player client.
    underboss_ready: bool,
}

/// Political progress using only the player's owned business titles and settlements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoliticalDistrictProgress {
    pub district: NeighborhoodId,
    pub active_owned: usize,
    pub suspended_owned: usize,
    /// Active businesses continuously owned at least 90 days.
    pub mature_active: usize,
    /// Mature businesses with fresh settlements and at least 20 profitable cycles in 30 days.
    pub established_active: usize,
    /// Last 30 days' net from all mature, active owned storefronts in this district.
    pub mature_window_net: Money,
    /// Has an established storefront and enough combined mature net to fund one member for 30 days.
    pub qualifies: bool,
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
        "legitimate political power requires a 30-day political contact and 90-day businesses earning at least one member's wages per district across three quarters of the city, with a mature legitimate portfolio covering total payroll"
    )]
    PoliticalNotReady,
    #[error(
        "funded retirement needs one leader, no pending decisions or active operations, and enough accounted money for the leader's $1,000,000 nest egg plus 180 days of wages for each subordinate"
    )]
    RetirementNotReady,
    #[error(
        "city underboss requires enough named command staff, two independent mature-racket captains per qualifying district, profitable local majorities in three quarters of districts, racket income covering payroll, and more active rackets than any rival"
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

fn retirement_members(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
) -> Result<(Vec<(CharacterId, Money)>, Money), VictoryError> {
    let members: Vec<_> = state
        .world()
        .characters_in_organization(organization)
        .collect();
    let leaders: Vec<_> = members
        .iter()
        .filter(|member| member.supervisor().is_none())
        .map(|member| member.id())
        .collect();
    if leaders.len() != 1 || members.is_empty() {
        return Err(VictoryError::RetirementNotReady);
    }
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .ok_or(VictoryError::Arithmetic)?;
    let mut payees = Vec::with_capacity(members.len());
    for member in members {
        let amount = if member.id() == leaders[0] {
            RETIREMENT_NEST_EGG
        } else {
            severance
        };
        payees.push((member.id(), amount));
    }
    let required = RETIREMENT_NEST_EGG
        .checked_add(
            severance
                .checked_mul(i64::try_from(payees.len() - 1).map_err(|_| VictoryError::Arithmetic)?)
                .ok_or(VictoryError::Arithmetic)?,
        )
        .ok_or(VictoryError::Arithmetic)?;
    Ok((payees, required))
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

/// Net operating profit, not laundering volume or appraisal, earned by the
/// present owner during the recent income window. Losses must offset profits.
fn recent_business_net(state: &AppState, business: BusinessId, owner: OrganizationId) -> i128 {
    let Some(record) = state.world().get_business(business) else {
        return 0;
    };
    state
        .economy()
        .cycles_for(business)
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| {
            cycle.occurred_at() <= state.now()
                && cycle.business_version() == record.version()
                && cycle.owner() == BusinessOwner::Organization(owner)
        })
        .map(|cycle| i128::from(cycle.net_cash().cents()))
        .sum()
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

/// A manager can hold multiple mandates and run rackets in several districts, but
/// cannot count as two independent field captains or supply concurrent district
/// command in two places for the underboss ending. Match each district's two
/// command posts against distinct real managers, with augmenting paths so the
/// result does not depend on a greedy choice of which manager to assign first.
fn staffed_racket_districts(
    districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
) -> usize {
    type Post = (NeighborhoodId, usize);

    fn match_post(
        post: Post,
        captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
        assignments: &mut BTreeMap<CharacterId, Post>,
        visited: &mut BTreeSet<CharacterId>,
    ) -> bool {
        let Some(candidates) = captains.get(&post.0) else {
            return false;
        };
        for &candidate in candidates {
            if !visited.insert(candidate) {
                continue;
            }
            let previous = assignments.get(&candidate).copied();
            if previous.is_none()
                || previous
                    .is_some_and(|previous| match_post(previous, captains, assignments, visited))
            {
                assignments.insert(candidate, post);
                return true;
            }
        }
        false
    }

    let mut assignments = BTreeMap::new();
    for &district in districts {
        for post in 0..UNDERBOSS_CAPTAINS_PER_DISTRICT {
            match_post(
                (district, post),
                captains,
                &mut assignments,
                &mut BTreeSet::new(),
            );
        }
    }
    let mut posts_by_district = BTreeMap::<NeighborhoodId, usize>::new();
    for &(district, _) in assignments.values() {
        *posts_by_district.entry(district).or_default() += 1;
    }
    posts_by_district
        .values()
        .filter(|&&posts| posts == UNDERBOSS_CAPTAINS_PER_DISTRICT)
        .count()
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

fn mature_business_title(state: &AppState, business: BusinessId, owner: OrganizationId) -> bool {
    let Some(record) = state.world().get_business(business) else {
        return false;
    };
    state
        .world()
        .get_business_ownership_change_for_version(business, record.version())
        .is_some_and(|change| {
            change.new_owner() == BusinessOwner::Organization(owner)
                && established_for(state.now(), change.changed_at(), VICTORY_ESTABLISHMENT_DAYS)
        })
}

/// A recent settlement and a substantial record of profitable cycles under
/// current title prevent an instant acquisition from inheriting the seller's
/// political standing.
fn established_business(state: &AppState, business: BusinessId, owner: OrganizationId) -> bool {
    let Some(record) = state.world().get_business(business) else {
        return false;
    };
    if !mature_business_title(state, business, owner)
        || !state.economy().latest_cycle(business).is_some_and(|cycle| {
            cycle.owner() == BusinessOwner::Organization(owner)
                && settled_recently(state.now(), cycle.occurred_at())
        })
    {
        return false;
    }
    state
        .economy()
        .cycles_for(business)
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > recent_income_start(state.now()))
        .filter(|cycle| {
            cycle.occurred_at() <= state.now()
                && cycle.business_version() == record.version()
                && cycle.owner() == BusinessOwner::Organization(owner)
                && cycle.net_cash() > Money::ZERO
        })
        .take(VICTORY_PROFITABLE_CYCLES)
        .count()
        == VICTORY_PROFITABLE_CYCLES
}

#[derive(Clone, Copy, Debug, Default)]
struct BusinessDistrictStanding {
    active: usize,
    suspended: usize,
    mature: usize,
    established: usize,
    window_net: i128,
}

impl BusinessDistrictStanding {
    fn qualifies(self, local_income_floor: i128) -> bool {
        self.established > 0 && self.window_net >= local_income_floor && self.window_net > 0
    }
}

/// Own-books political readiness by district. This excludes unknown rival
/// operations and shares the same mature-title, settlement and income rules
/// as the authoritative victory validator.
pub fn political_district_progress(
    registry: &Registry,
    state: &AppState,
) -> Result<Vec<PoliticalDistrictProgress>, VictoryError> {
    let organization = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let local_income_floor = i128::from(registry.upkeep().per_member_daily().cents())
        * i128::from(VICTORY_INCOME_WINDOW_DAYS);
    Ok(business_district_standings(state, organization)
        .into_iter()
        .map(|(district, standing)| PoliticalDistrictProgress {
            district,
            active_owned: standing.active,
            suspended_owned: standing.suspended,
            mature_active: standing.mature,
            established_active: standing.established,
            mature_window_net: Money::from_cents(
                standing
                    .window_net
                    .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
            ),
            qualifies: standing.qualifies(local_income_floor),
        })
        .collect())
}

fn business_district_standings(
    state: &AppState,
    owner: OrganizationId,
) -> BTreeMap<NeighborhoodId, BusinessDistrictStanding> {
    let mut standings = BTreeMap::<NeighborhoodId, BusinessDistrictStanding>::new();
    for business in state.world().businesses_owned_by_organization(owner) {
        let Some(economy) = state.economy().get_business_economy(business.id()) else {
            continue;
        };
        let entry = standings.entry(business.neighborhood()).or_default();
        if economy.status() == BusinessOperatingStatus::Suspended {
            entry.suspended += 1;
            continue;
        }
        if economy.status() != BusinessOperatingStatus::Active {
            continue;
        }
        entry.active += 1;
        if mature_business_title(state, business.id(), owner) {
            entry.mature += 1;
            entry.window_net += recent_business_net(state, business.id(), owner);
            if established_business(state, business.id(), owner) {
                entry.established += 1;
            }
        }
    }
    standings
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

pub fn progress(registry: &Registry, state: &AppState) -> Result<VictoryProgress, VictoryError> {
    let organization = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let total_districts = state.world().neighborhoods().count();
    let key_personnel = state
        .world()
        .characters_in_organization(organization)
        .count();
    let required_political_districts = (total_districts * 3).div_ceil(4);
    let required_underboss_districts = (total_districts * 3).div_ceil(4);
    let required_underboss_personnel = required_underboss_personnel(required_underboss_districts);
    let per_member_wage = i128::from(registry.upkeep().per_member_daily().cents());
    let daily_payroll =
        per_member_wage * i128::try_from(key_personnel).expect("member count fits i128");
    let income_window = i128::from(VICTORY_INCOME_WINDOW_DAYS);
    // A credible district foothold must pay at least one crew member's wage
    // from that district's own mature operations. A few barely-profitable
    // storefronts cannot substitute for actual local economic leverage.
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
    let legitimate_window_net = business_standings
        .values()
        .map(|standing| standing.window_net)
        .sum::<i128>();
    let political_contacts = state
        .contacts()
        .contacts_for_sponsor(organization)
        .filter(|contact| {
            contact.kind() == ContactKind::Political
                && contact.status() == ContactStatus::Active
                && state
                    .world()
                    .get_organization(contact.institution())
                    .is_some_and(|institution| institution.kind() == OrganizationKind::Political)
                && state
                    .world()
                    .get_character(contact.handler())
                    .is_some_and(|handler| handler.organization() == Some(organization))
        })
        .collect::<Vec<_>>();
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
    let mut rackets_by_district: BTreeMap<_, (i128, u64)> = BTreeMap::new();
    let mut captains_by_district = BTreeMap::<NeighborhoodId, BTreeSet<CharacterId>>::new();
    for racket in own_rackets.iter().filter(|enterprise| {
        established_for(
            state.now(),
            enterprise.established_at(),
            VICTORY_ESTABLISHMENT_DAYS,
        )
    }) {
        if let Some(district) = racket_district(racket) {
            let (net, count) = rackets_by_district.entry(district).or_default();
            *net += recent_racket_net(state, racket);
            if established_racket(state, racket) {
                *count += 1;
                if proven_racket_captain(state, racket) {
                    captains_by_district
                        .entry(district)
                        .or_default()
                        .insert(racket.manager());
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
    // Territorial control and independent command must hold in the SAME
    // districts; two disjoint sets of three cannot combine into a victory.
    let staffed_underboss_districts =
        staffed_racket_districts(&uncontested_districts, &captains_by_district);
    let own_active = state
        .enterprises()
        .enterprises_for_organization(organization)
        .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
        .count();
    let rival_max = state
        .world()
        .organizations()
        .filter(|owner| owner.kind() == OrganizationKind::Criminal && owner.id() != organization)
        .map(|owner| {
            state
                .enterprises()
                .enterprises_for_organization(owner.id())
                .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
                .count()
        })
        .max()
        .unwrap_or(0);
    // Include negative accounted balances; a debt cannot be ignored by spending another pocket.
    let net = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| i128::from(account.balance().cents()))
        .sum::<i128>()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    let retirement_total = retirement_members(registry, state, organization)
        .map(|(_, total)| total)
        .unwrap_or(RETIREMENT_NEST_EGG);
    Ok(VictoryProgress {
        total_districts,
        key_personnel,
        required_underboss_personnel,
        required_political_districts,
        owned_operating_business_districts,
        active_political_contacts,
        established_political_contacts,
        established_business_districts,
        required_underboss_districts,
        own_active_racket_districts,
        established_racket_districts: established_racket_districts.len(),
        staffed_underboss_districts,
        legitimate_daily_net: Money::from_cents(
            (legitimate_window_net / income_window)
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        ),
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
        accounted_net: Money::from_cents(net as i64),
        retirement_total,
        underboss_ready: total_districts > 0
            && uncontested_districts.len() >= required_underboss_districts
            && key_personnel >= required_underboss_personnel
            && staffed_underboss_districts >= required_underboss_districts
            && own_active >= 2
            && own_active > rival_max
            && daily_payroll > 0
            && racket_window_net >= daily_payroll * income_window,
    })
}

fn ensure_eligible(
    registry: &Registry,
    state: &AppState,
    path: VictoryPath,
) -> Result<(), VictoryError> {
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
            if standing.established_political_contacts == 0
                || standing.total_districts == 0
                || standing.established_business_districts
                    < standing.required_political_districts
                || !standing.legitimate_self_sufficient =>
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
            if standing.accounted_net < required
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
    }
}

fn prepare_retirement_payment(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
) -> Result<ValidatedLedgerTransaction, VictoryError> {
    let (payees, required) = retirement_members(registry, state, owner)?;
    let mut remaining = required;
    let mut postings = Vec::new();
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
    {
        if remaining == Money::ZERO {
            break;
        }
        let debit = account.available_payment_balance().min(remaining);
        if debit <= Money::ZERO {
            continue;
        }
        postings.push(LedgerPosting {
            account: account.id(),
            amount: debit.checked_neg().ok_or(VictoryError::Arithmetic)?,
        });
        remaining = remaining
            .checked_sub(debit)
            .ok_or(VictoryError::Arithmetic)?;
    }
    if remaining != Money::ZERO {
        return Err(VictoryError::RetirementNotReady);
    }
    let mut missing = Vec::new();
    let mut destinations = Vec::new();
    for (person, amount) in payees {
        let destination = state
            .finance()
            .accounts_for(FinancialOwner::Character(person))
            .find(|account| {
                account.kind() == AccountKind::AccountedFunds
                    && account.can_receive_liquid_payment(amount)
            })
            .map(|account| account.id());
        if destination.is_none() {
            missing.push(FinancialAccountDraft {
                owner: FinancialOwner::Character(person),
                kind: AccountKind::AccountedFunds,
            });
        }
        destinations.push((destination, amount));
    }
    let openings = if missing.is_empty() {
        None
    } else {
        Some(validate_open_accounts(state, missing)?)
    };
    let mut opened = 0;
    for (destination, amount) in destinations {
        let id = match destination {
            Some(id) => id,
            None => {
                let id = openings
                    .as_ref()
                    .and_then(|plan| plan.account_id(opened))
                    .ok_or(VictoryError::Arithmetic)?;
                opened += 1;
                id
            }
        };
        postings.push(LedgerPosting {
            account: id,
            amount,
        });
    }
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: "Final campaign retirement settlement: leader nest egg and crew severance".to_owned(),
        postings,
        authorization: None,
    };
    Ok(match openings {
        Some(plan) => validate_record_transaction_with_openings(state, plan, draft)?,
        None => validate_record_transaction(state, draft)?,
    })
}

pub struct ValidatedCampaignVictory {
    owner: OrganizationId,
    path: VictoryPath,
    validated_at: SimTime,
    history: ValidatedHistoryEvent,
    report: ValidatedReport,
    settlement: Option<ValidatedLedgerTransaction>,
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
    ensure_eligible(registry, state, path)?;
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let settlement = if path == VictoryPath::FundedRetirement {
        Some(prepare_retirement_payment(registry, state, owner)?)
    } else {
        None
    };
    let settlement_id = settlement
        .as_ref()
        .map(|_| LedgerTransactionId::from_raw(state.ids.next_raw(IdKind::LedgerTransaction)));
    let history = validate_campaign_victory_event(state, owner, path, settlement_id)?;
    let report = validate_report_draft(state, ReportDraft {
        recipient: owner,
        kind: ReportKind::CampaignVictory,
        title: format!("Campaign victory: {}", path.label()),
        entries: vec![ReportEntry {
            attention: AttentionClass::Crisis,
            summary: match path {
                VictoryPath::PoliticalPower =>
                    "Legitimate businesses across the city and a live political sponsor secured a governing coalition.".to_owned(),
                VictoryPath::FundedRetirement =>
                    "The leader retired with a funded nest egg; each subordinate received 180 days of wages in an accounted personal account.".to_owned(),
                VictoryPath::CityUnderboss =>
                    "Uncontested economic influence across the city's districts and superior underworld strength secured the underboss position.".to_owned(),
            },
            sources: Vec::new(),
            entities: BTreeSet::from([EntityRef::Organization(owner)]),
            decision: None,
        }],
    })?;
    let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
    if let Some(payment) = &settlement {
        budget.extend(payment.id_budget());
    }
    state.ids.reserve_many(&budget)?;
    Ok(ValidatedCampaignVictory {
        owner,
        path,
        validated_at: state.now(),
        history,
        report,
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
        ensure_eligible(registry, state, self.path)?;
        if let Some(settlement) = &self.settlement {
            settlement.ensure_current(state)?;
        }
        let mut budget = vec![(IdKind::HistoryEvent, 1), (IdKind::Report, 1)];
        if let Some(settlement) = &self.settlement {
            budget.extend(settlement.id_budget());
        }
        state.ids.reserve_many(&budget)?;
        let settlement = self.settlement.map(|plan| plan.commit_preflighted(state));
        let history = self.history.commit(state)?;
        let report = self.report.commit(state)?;
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
