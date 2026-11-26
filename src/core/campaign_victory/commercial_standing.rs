//! Read-only commercial and political district qualification from current titles and settled books.

use super::{
    POLITICAL_BUSINESS_ESTABLISHMENT_DAYS, POLITICAL_BUSINESS_KINDS_PER_DISTRICT,
    POLITICAL_BUSINESSES_PER_DISTRICT, PoliticalBusinessProgress, PoliticalDistrictProgress,
    VICTORY_INCOME_WINDOW_DAYS, VICTORY_PROFITABLE_CYCLES, VictoryError, political_staffing,
    recent_income_start, settled_recently,
};
use crate::core::id::{BusinessId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime};
use crate::economy::business_economy_system::portfolio_overhead_basis_points;
use crate::economy::{BusinessCycleRecord, BusinessOperatingStatus};
use crate::finance::Money;
use crate::politics::political_system::{held_electoral_majority, rival_pressure};
use crate::registry::Registry;
use crate::world::{BusinessKind, BusinessOwner, BusinessRecord};
use std::collections::{BTreeMap, BTreeSet};

/// One title period's realized operating evidence, excluding transfers and appraisals.
#[derive(Default)]
struct TitleWindow {
    net: i128,
    profitable_cycles: usize,
    latest: Option<SimTime>,
}

impl TitleWindow {
    fn include(&mut self, cycle: &BusinessCycleRecord) {
        self.net += i128::from(cycle.net_cash().cents());
        self.profitable_cycles += usize::from(cycle.net_cash() > Money::ZERO);
        self.latest = self.latest.max(Some(cycle.occurred_at()));
    }

    fn established(&self, now: SimTime, owned_minutes: u64) -> bool {
        owned_minutes >= POLITICAL_BUSINESS_ESTABLISHMENT_DAYS * DAY_MINUTES
            && self.profitable_cycles >= VICTORY_PROFITABLE_CYCLES
            && self.latest.is_some_and(|at| settled_recently(now, at))
    }
}

fn title_age_minutes(state: &AppState, business: &BusinessRecord, owner: OrganizationId) -> u64 {
    state
        .world()
        .get_business_ownership_change_for_version(business.id(), business.version())
        .filter(|change| change.new_owner() == BusinessOwner::Organization(owner))
        .map_or(0, |change| {
            state
                .now()
                .as_minutes()
                .saturating_sub(change.changed_at().as_minutes())
        })
}

fn current_title_window(
    state: &AppState,
    business: &BusinessRecord,
    owner: OrganizationId,
) -> TitleWindow {
    let mut window = TitleWindow::default();
    let cutoff = recent_income_start(state.now());
    for cycle in state
        .economy()
        .cycles_for(business.id())
        .rev()
        .take_while(|cycle| cycle.occurred_at().as_minutes() > cutoff)
        .filter(|cycle| {
            cycle.occurred_at() <= state.now()
                && cycle.business_version() == business.version()
                && cycle.owner() == BusinessOwner::Organization(owner)
        })
    {
        window.include(cycle);
    }
    window
}

#[derive(Clone, Debug, Default)]
pub(super) struct BusinessDistrictStanding {
    pub(super) active: usize,
    suspended: usize,
    mature: usize,
    established: usize,
    established_kinds: BTreeSet<BusinessKind>,
    pub(super) window_net: i128,
}

impl BusinessDistrictStanding {
    pub(super) fn qualifies(&self, local_income_floor: i128) -> bool {
        self.established_kinds.len() >= POLITICAL_BUSINESS_KINDS_PER_DISTRICT
            && self.established >= POLITICAL_BUSINESSES_PER_DISTRICT
            && self.window_net >= local_income_floor
            && self.window_net > 0
    }
}

/// Own-books political readiness for every public district, including wards
/// without owned storefronts. Personnel candidates are live; only mature
/// commercial constituencies receive counted teams for victory.
pub fn political_district_progress(
    registry: &Registry,
    state: &AppState,
) -> Result<Vec<PoliticalDistrictProgress>, VictoryError> {
    let organization = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let local_income_floor = i128::from(registry.upkeep().per_member_daily().cents())
        * i128::from(VICTORY_INCOME_WINDOW_DAYS);
    let standings = business_district_standings(state, organization);
    Ok(political_district_rows(
        state,
        organization,
        &standings,
        local_income_floor,
        &BTreeSet::new(),
    ))
}

pub(super) fn political_district_rows(
    state: &AppState,
    organization: OrganizationId,
    standings: &BTreeMap<NeighborhoodId, BusinessDistrictStanding>,
    local_income_floor: i128,
    ballot_winners: &BTreeSet<NeighborhoodId>,
) -> Vec<PoliticalDistrictProgress> {
    let economically_ready = standings
        .iter()
        .filter(|(_, standing)| standing.qualifies(local_income_floor))
        .map(|(&district, _)| district)
        .collect();
    let staffing =
        political_staffing::allocate(state, organization, &economically_ready, ballot_winners);
    let empty = BusinessDistrictStanding::default();
    state
        .world()
        .neighborhoods()
        .map(|ward| {
            let district = ward.id();
            let standing = standings.get(&district).unwrap_or(&empty);
            let economically_qualified = standing.qualifies(local_income_floor);
            let team = staffing.assigned.get(&district);
            let steward = team.map(|(political, _)| *political);
            let financial_steward = team.map(|(_, finance)| *finance);
            let office = state
                .politics()
                .organization(organization)
                .and_then(|campaign| campaign.district(district));
            let own_influence = office.map_or(0, |office| office.influence());
            let grassroots_support = office.map_or(0, |office| office.grassroots());
            let institutional_support = office.map_or(0, |office| office.institutions());
            let rival_influence = rival_pressure(state, organization, district);
            let majority_held_days = office
                .and_then(|office| office.leading_since())
                .map_or(0, |since| {
                    state.now().as_minutes().saturating_sub(since.as_minutes()) / DAY_MINUTES
                });
            PoliticalDistrictProgress {
                district,
                active_owned: standing.active,
                suspended_owned: standing.suspended,
                mature_active: standing.mature,
                established_active: standing.established,
                established_kinds: standing.established_kinds.len(),
                available_stewards: staffing
                    .candidates
                    .get(&district)
                    .map_or(0, |local| local.political.len()),
                available_financial_stewards: staffing
                    .candidates
                    .get(&district)
                    .map_or(0, |local| local.finance.len()),
                steward,
                financial_steward,
                mature_window_net: Money::from_cents(
                    standing
                        .window_net
                        .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                        as i64,
                ),
                economically_qualified,
                qualifies: economically_qualified
                    && steward.is_some()
                    && financial_steward.is_some(),
                own_influence,
                grassroots_support,
                institutional_support,
                rival_influence,
                majority_held_days,
                mayoral_ready: economically_qualified
                    && steward.is_some()
                    && financial_steward.is_some()
                    && held_electoral_majority(state, organization, district),
            }
        })
        .collect()
}

/// Exact own-books evidence behind each district's commercial readiness.
pub fn political_business_progress(
    state: &AppState,
    district: NeighborhoodId,
) -> Result<Vec<PoliticalBusinessProgress>, VictoryError> {
    let owner = state.player_organization().ok_or(VictoryError::NoPlayer)?;
    let overhead = portfolio_overhead_basis_points(state, owner);
    Ok(state
        .world()
        .businesses_owned_by_organization(owner)
        .filter(|business| business.neighborhood() == district)
        .filter_map(|business| {
            let economy = state.economy().get_business_economy(business.id())?;
            let active = economy.is_operating();
            let owned_minutes = title_age_minutes(state, business, owner);
            let window = current_title_window(state, business, owner);
            let owned_days = owned_minutes / DAY_MINUTES;
            let days_until_mature = (POLITICAL_BUSINESS_ESTABLISHMENT_DAYS * DAY_MINUTES)
                .saturating_sub(owned_minutes)
                .div_ceil(DAY_MINUTES);
            let last_owned_settlement = state
                .economy()
                .latest_cycle(business.id())
                .filter(|cycle| {
                    cycle.owner() == BusinessOwner::Organization(owner)
                        && cycle.business_version() == business.version()
                        && cycle.occurred_at() <= state.now()
                })
                .map(|cycle| cycle.occurred_at());
            Some(PoliticalBusinessProgress {
                business: business.id(),
                kind: business.kind(),
                active,
                owned_days,
                days_until_mature,
                profitable_cycles: window.profitable_cycles,
                last_owned_settlement,
                recent_net: Money::from_cents(
                    net_after_portfolio_overhead(window.net, overhead)
                        .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                        as i64,
                ),
                established: active && window.established(state.now(), owned_minutes),
            })
        })
        .collect())
}

fn net_after_portfolio_overhead(net: i128, basis_points: u16) -> i128 {
    if net <= 0 {
        net
    } else {
        net * (10_000 - i128::from(basis_points)) / 10_000
    }
}

pub(super) fn business_district_standings(
    state: &AppState,
    owner: OrganizationId,
) -> BTreeMap<NeighborhoodId, BusinessDistrictStanding> {
    let mut standings = BTreeMap::<NeighborhoodId, BusinessDistrictStanding>::new();
    let overhead = portfolio_overhead_basis_points(state, owner);
    // Aggregate by actual settlement owner and title period, including shops sold
    // or suspended during the window. Only established active titles contribute
    // positive standing. Ineligible titles still contribute their net losses.
    let cutoff = recent_income_start(state.now());
    let mut remaining_income = BTreeMap::<(BusinessId, u32), TitleWindow>::new();
    for cycle in state
        .economy()
        .cycles_for_owner_from_through(
            BusinessOwner::Organization(owner),
            SimTime::from_minutes(cutoff),
            state.now(),
        )
        .filter(|cycle| cycle.occurred_at().as_minutes() > cutoff)
    {
        remaining_income
            .entry((cycle.business(), cycle.business_version()))
            .or_default()
            .include(cycle);
    }
    for business in state.world().businesses_owned_by_organization(owner) {
        let Some(economy) = state.economy().get_business_economy(business.id()) else {
            continue;
        };
        let entry = standings.entry(business.neighborhood()).or_default();
        if economy.status() == BusinessOperatingStatus::Suspended {
            entry.suspended += 1;
            continue;
        }
        if !economy.is_operating() {
            continue;
        }
        entry.active += 1;
        let owned_minutes = title_age_minutes(state, business, owner);
        if owned_minutes >= POLITICAL_BUSINESS_ESTABLISHMENT_DAYS * DAY_MINUTES {
            entry.mature += 1;
            let window = remaining_income
                .remove(&(business.id(), business.version()))
                .unwrap_or_default();
            if window.established(state.now(), owned_minutes) {
                entry.established += 1;
                entry.established_kinds.insert(business.kind());
                entry.window_net += net_after_portfolio_overhead(window.net, overhead);
            } else {
                entry.window_net += window.net.min(0);
            }
        }
    }
    for ((business, _), window) in remaining_income {
        if window.net < 0
            && let Some(record) = state.world().get_business(business)
        {
            standings
                .entry(record.neighborhood())
                .or_default()
                .window_net += window.net;
        }
    }
    standings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn political_business_coalition_requires_distinct_profitable_industries() {
        let mut standing = BusinessDistrictStanding {
            active: 3,
            mature: 3,
            established: 3,
            window_net: 250_000,
            ..BusinessDistrictStanding::default()
        };
        standing.established_kinds.insert(BusinessKind::Pawnshop);
        assert!(
            !standing.qualifies(120_000),
            "two pawnshops are still a single local commercial constituency"
        );
        standing.established_kinds.insert(BusinessKind::Hospitality);
        assert!(standing.qualifies(120_000));
        standing.window_net = 119_999;
        assert!(
            !standing.qualifies(120_000),
            "diversity cannot replace income"
        );
    }
}
