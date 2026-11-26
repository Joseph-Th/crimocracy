//! Shared local sales, including partial competition between related trades.
//!
//! Direct competitors share a market; related trades compete for a portion of the same
//! district's customer budget. No owner identity, account balance or criminal status enters
//! the calculation. Losing a competitor's capacity leaves additional custom for the others.

use super::{BusinessEconomyError, daily_owner_net};
use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::economy::{BusinessOperatingStatus, MarketSnapshot};
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::{BusinessKind, BusinessOwner, BusinessRecord};

/// Sixty percent of potential is venue-specific; forty percent is contested. A related trade
/// counts as half a direct competitor. These are city-wide market structure assumptions,
/// not owner-specific penalties, and apply equally to independent and criminal owners.
const CONTESTED_BASIS_POINTS: u32 = 4_000;
const FULL: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Competition {
    Direct,
    Related,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sector {
    Retail,
    Leisure,
    Freight,
    BusinessServices,
    VehicleServices,
}

fn sector(kind: BusinessKind) -> Option<Sector> {
    use BusinessKind::*;
    match kind {
        Retail | Pharmacy | Pawnshop => Some(Sector::Retail),
        Hospitality | Nightclub | MovieTheater | AthleticClub | Lodging => Some(Sector::Leisure),
        Transportation | Warehouse | Wholesale | Stevedoring => Some(Sector::Freight),
        ProfessionalServices | FinancialServices => Some(Sector::BusinessServices),
        Automotive | TaxiGarage => Some(Sector::VehicleServices),
        Brewery
        | Construction
        | CoinMachineDistribution
        | Laundry
        | Printing
        | GarmentFactory
        | NewsService
        | FuneralParlor
        | SoupKitchen => None,
    }
}

fn competition(left: BusinessKind, right: BusinessKind) -> Option<Competition> {
    if left == right {
        Some(Competition::Direct)
    } else if sector(left).is_some_and(|sector_id| Some(sector_id) == sector(right)) {
        Some(Competition::Related)
    } else {
        None
    }
}

pub(super) fn market_trades_compete(left: BusinessKind, right: BusinessKind) -> bool {
    competition(left, right).is_some()
}

impl MarketSnapshot {
    fn add_competitor(&mut self, competition: Competition, damaged: bool) {
        match (competition, damaged) {
            (Competition::Direct, false) => self.healthy_competitors += 1,
            (Competition::Direct, true) => self.damaged_competitors += 1,
            (Competition::Related, false) => self.related_healthy_competitors += 1,
            (Competition::Related, true) => self.related_damaged_competitors += 1,
        }
    }

    pub(crate) fn gross_basis_points(self, disrupted: bool, damaged_weight: u32) -> u32 {
        let own_weight = if disrupted { damaged_weight } else { FULL };
        // Both sides of the market-share fraction must use the damaged shop's reduced
        // capacity. Weighting only the denominator would increase its contested share
        // after sabotage and create customer demand from nowhere.
        // Double weights preserve exact half-competitor arithmetic.
        let market_weight = 2 * u64::from(own_weight)
            + 2 * u64::from(self.healthy_competitors) * u64::from(FULL)
            + 2 * u64::from(self.damaged_competitors) * u64::from(damaged_weight)
            + u64::from(self.related_healthy_competitors) * u64::from(FULL)
            + u64::from(self.related_damaged_competitors) * u64::from(damaged_weight);
        // A sole damaged store still takes all contested local custom before the
        // separate damage-to-gross multiplier is applied.
        let contested =
            u64::from(CONTESTED_BASIS_POINTS) * 2 * u64::from(own_weight) / market_weight.max(1);
        FULL - CONTESTED_BASIS_POINTS + u32::try_from(contested).expect("contested share bounded")
    }

    fn counts(self) -> (u64, u64) {
        (
            u64::from(self.healthy_competitors) + u64::from(self.damaged_competitors),
            u64::from(self.related_healthy_competitors)
                + u64::from(self.related_damaged_competitors),
        )
    }
}

fn local_peers<'a>(
    state: &'a AppState,
    business: &'a BusinessRecord,
) -> impl Iterator<Item = (&'a BusinessRecord, Competition)> {
    state
        .world()
        .businesses_in_neighborhood(business.neighborhood())
        .filter(move |peer| peer.id() != business.id())
        .filter_map(move |peer| competition(business.kind(), peer.kind()).map(|kind| (peer, kind)))
}

/// The known condition of a productive storefront; closed or terminal books
/// do not contribute to future local sales.
fn operating_condition(state: &AppState, business: BusinessId) -> Option<bool> {
    let economy = state.economy().get_business_economy(business)?;
    economy
        .is_operating()
        .then(|| economy.is_disrupted(state.now()))
}

fn market_snapshot(
    state: &AppState,
    record: &BusinessRecord,
    mut condition: impl FnMut(&BusinessRecord) -> Option<bool>,
) -> MarketSnapshot {
    let mut snapshot = MarketSnapshot::default();
    for (peer, kind) in local_peers(state, record) {
        if let Some(disrupted) = condition(peer) {
            snapshot.add_competitor(kind, disrupted);
        }
    }
    snapshot
}

/// The subject is counted hypothetically as open even when forecasting a resumption.
/// Other stores with no established operating books, a suspended economy, or an exhausted
/// recurrence cannot compete for future sales. Capture facts before the same-minute cohort settles.
pub(super) fn resolve_market_snapshot(
    state: &AppState,
    business: BusinessId,
) -> Result<MarketSnapshot, BusinessEconomyError> {
    let record = super::validate_business(state, business)?;
    Ok(market_snapshot(state, record, |peer| {
        operating_condition(state, peer.id())
    }))
}

/// Revenue an owner's *other* productive storefronts would lose if this suspended
/// store resumed. An individually profitable reopening may still destroy more
/// customer revenue in the rest of the owner's portfolio than it brings in.
/// The same zero-variance settlement calculation is used on both sides of the
/// counterfactual; only the candidate's market presence changes.
pub(super) fn owner_reopening_daily_displacement(
    registry: &Registry,
    state: &AppState,
    candidate: BusinessId,
    overhead: u16,
) -> Result<i128, BusinessEconomyError> {
    let business = super::validate_business(state, candidate)?;
    if business.owner() == BusinessOwner::Independent {
        return Ok(0);
    }
    let economy = state
        .economy()
        .get_business_economy(candidate)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(candidate))?;
    if economy.status() != BusinessOperatingStatus::Suspended {
        return Err(BusinessEconomyError::EconomyNotSuspended(candidate));
    }
    activating_owner_daily_displacement(
        registry,
        state,
        business,
        economy.is_disrupted(state.now()),
        business.owner(),
        overhead,
        None,
    )
}

/// A suspended property bought by a new owner is restarted, not kept dormant. Estimate the
/// lost sales of that buyer's *existing* operating stores before deciding to purchase. An
/// already-active target has already competed for those customers, so changing its owner does
/// not create new market pressure. This read-only calculation only affects an internal NPC
/// purchasing decision; no foreign operating details are disclosed to another organization.
pub(crate) fn acquisition_owner_daily_displacement(
    registry: &Registry,
    state: &AppState,
    candidate: BusinessId,
    buyer: OrganizationId,
    overhead: u16,
    excluded: Option<BusinessId>,
) -> Result<i128, BusinessEconomyError> {
    let economy = state
        .economy()
        .get_business_economy(candidate)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(candidate))?;
    if economy.status() == BusinessOperatingStatus::Active {
        return Ok(0);
    }
    let business = super::validate_business(state, candidate)?;
    activating_owner_daily_displacement(
        registry,
        state,
        business,
        economy.is_disrupted(state.now()),
        BusinessOwner::Organization(buyer),
        overhead,
        excluded,
    )
}

fn activating_owner_daily_displacement(
    registry: &Registry,
    state: &AppState,
    business: &BusinessRecord,
    incoming_damaged: bool,
    owner: BusinessOwner,
    overhead: u16,
    excluded: Option<BusinessId>,
) -> Result<i128, BusinessEconomyError> {
    let mut lost_daily_net = 0_i128;
    for (peer, relationship) in local_peers(state, business) {
        if peer.owner() != owner || Some(peer.id()) == excluded {
            continue;
        }
        let Some(peer_economy) = state.economy().get_business_economy(peer.id()) else {
            continue;
        };
        // A terminal active record has no future cycle and therefore no future
        // revenue to cannibalize. Only scheduled, live peers belong in this sum.
        if !peer_economy.is_operating() {
            continue;
        }
        let current_market = resolve_market_snapshot(state, peer.id())?;
        let mut reopened_market = current_market;
        reopened_market.add_competitor(relationship, incoming_damaged);
        let cycle = registry
            .get_business(peer.kind())
            .economics()
            .cycle()
            .as_minutes();
        let disrupted = peer_economy.is_disrupted(state.now());
        let before = forecast_net_for_market(registry, state, peer, current_market, disrupted)?;
        let after = forecast_net_for_market(registry, state, peer, reopened_market, disrupted)?;
        lost_daily_net +=
            daily_owner_net(before, cycle, overhead) - daily_owner_net(after, cycle, overhead);
    }
    Ok(lost_daily_net)
}

/// Zero-variance live operating forecast for an owner's own books. Unlike the public
/// appraisal, this uses current operating competitors and actual disruption.
pub(crate) fn forecast_live_business_net(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<Money, BusinessEconomyError> {
    let record = super::validate_business(state, business)?;
    let economy = state
        .economy()
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    forecast_net_for_market(
        registry,
        state,
        record,
        resolve_market_snapshot(state, business)?,
        economy.is_disrupted(state.now()),
    )
}

pub(super) fn forecast_net_for_market(
    registry: &Registry,
    state: &AppState,
    record: &BusinessRecord,
    market: MarketSnapshot,
    disrupted: bool,
) -> Result<Money, BusinessEconomyError> {
    let neighborhood = state
        .world()
        .get_neighborhood(record.neighborhood())
        .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(
            record.id(),
        ))?;
    super::resolve_cycle_financials(
        record.id(),
        registry.get_business(record.kind()).economics(),
        neighborhood.profile(),
        registry.business_disruption().gross_basis_points(),
        disrupted,
        market,
        0,
    )
    .map(|(_, _, net)| net)
}

/// Acquisition forecasts must not act as a free live reconnaissance query. Assume all
/// neighboring directory peers are operating normally, regardless of actual private books.
pub(super) fn forecast_market_snapshot(
    state: &AppState,
    record: &BusinessRecord,
) -> MarketSnapshot {
    market_snapshot(state, record, |_| Some(false))
}

/// A portfolio owner knows which of its own businesses actually operate.
/// Treat foreign directory peers as healthy without consulting their books.
fn owner_forecast_market_snapshot(
    state: &AppState,
    record: &BusinessRecord,
    owner: BusinessOwner,
) -> MarketSnapshot {
    market_snapshot(state, record, |peer| {
        if peer.owner() == owner {
            operating_condition(state, peer.id())
        } else {
            Some(false)
        }
    })
}

/// Prospective income of a normally operating acquisition from the buyer's
/// perspective. The public appraisal remains independent of anyone's books.
pub(crate) fn forecast_acquisition_business_net(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
    buyer: OrganizationId,
) -> Result<Money, BusinessEconomyError> {
    let record = super::validate_business(state, business)?;
    forecast_net_for_market(
        registry,
        state,
        record,
        owner_forecast_market_snapshot(state, record, BusinessOwner::Organization(buyer)),
        false,
    )
}

/// Aggregate the forecast additional daily revenue across the planner's own active
/// businesses if a foreign target loses capacity. Use actual operating conditions for
/// the planner's own portfolio and public directory assumptions for foreign peers.
/// Both sides of the counterfactual use the canonical settlement calculation.
pub(crate) fn forecast_competitor_disruption_gain(
    registry: &Registry,
    state: &AppState,
    target: BusinessId,
    active_stakes: &[BusinessId],
) -> Result<i128, BusinessEconomyError> {
    let target_record = super::validate_business(state, target)?;
    let mut total = 0_i128;
    for &stake in active_stakes {
        let record = super::validate_business(state, stake)?;
        if record.owner() == target_record.owner()
            || record.neighborhood() != target_record.neighborhood()
        {
            continue;
        }
        let Some(kind) = competition(record.kind(), target_record.kind()) else {
            continue;
        };
        let Some(disrupted) = operating_condition(state, stake) else {
            continue;
        };
        let healthy = owner_forecast_market_snapshot(state, record, record.owner());
        let mut damaged = healthy;
        match kind {
            Competition::Direct => damaged.healthy_competitors -= 1,
            Competition::Related => damaged.related_healthy_competitors -= 1,
        }
        damaged.add_competitor(kind, true);
        // Operating costs are identical in both forecasts, so the difference in
        // net settlements is also the difference in gross receipts, including rounding.
        let before = forecast_net_for_market(registry, state, record, healthy, disrupted)?;
        let after = forecast_net_for_market(registry, state, record, damaged, disrupted)?;
        let cycle = registry
            .get_business(record.kind())
            .economics()
            .cycle()
            .as_minutes();
        total += (i128::from(after.cents()) - i128::from(before.cents())) * i128::from(DAY_MINUTES)
            / i128::from(cycle);
    }
    Ok(total)
}

/// Historical status is a frozen input like disruption and variance. Check each peer
/// category separately against the immutable directory and establishment chronology.
pub(super) fn validate_historical_market(
    state: &AppState,
    record: &BusinessRecord,
    snapshot: MarketSnapshot,
    occurred_at: crate::core::time::SimTime,
) -> Result<(), BusinessEconomyError> {
    let mut established = MarketSnapshot::default();
    for (peer, kind) in local_peers(state, record) {
        if state
            .economy()
            .get_business_economy(peer.id())
            .is_some_and(|economy| economy.established_at() <= occurred_at)
        {
            established.add_competitor(kind, false);
        }
    }
    let (direct, related) = snapshot.counts();
    let (max_direct, max_related) = established.counts();
    if direct > max_direct || related > max_related {
        return Err(BusinessEconomyError::InvalidMarketSnapshot(record.id()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn related_trades_compete_less_than_direct_substitutes() {
        assert_eq!(
            competition(BusinessKind::Hospitality, BusinessKind::Nightclub),
            Some(Competition::Related)
        );
        assert_eq!(
            competition(BusinessKind::Hospitality, BusinessKind::Brewery),
            None
        );
        let no_peers = MarketSnapshot::default();
        let mut direct = no_peers;
        direct.add_competitor(Competition::Direct, false);
        let mut related = no_peers;
        related.add_competitor(Competition::Related, false);
        assert_eq!(no_peers.gross_basis_points(false, 4_000), 10_000);
        assert_eq!(direct.gross_basis_points(false, 4_000), 8_000);
        assert_eq!(related.gross_basis_points(false, 4_000), 8_666);
        let mut damaged = no_peers;
        damaged.add_competitor(Competition::Related, true);
        assert_eq!(damaged.gross_basis_points(false, 4_000), 9_333);
    }

    #[test]
    fn disruption_redistributes_contested_customers_without_creating_demand() {
        let damage = 4_000;
        let mut original = MarketSnapshot::default();
        original.add_competitor(Competition::Direct, false);
        let healthy_pair = original.gross_basis_points(false, damage);
        assert_eq!(healthy_pair, 8_000);

        let damaged_shop = original.gross_basis_points(true, damage);
        let mut surviving_shop = MarketSnapshot::default();
        surviving_shop.add_competitor(Competition::Direct, true);
        let surviving_share = surviving_shop.gross_basis_points(false, damage);

        assert_eq!(damaged_shop, 7_142);
        assert_eq!(surviving_share, 8_857);
        assert!(damaged_shop < healthy_pair);
        assert!(surviving_share > healthy_pair);
        assert!(damaged_shop + surviving_share <= healthy_pair * 2);
        assert_eq!(
            MarketSnapshot::default().gross_basis_points(true, damage),
            FULL
        );
    }
}
