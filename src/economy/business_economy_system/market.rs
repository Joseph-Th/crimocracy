//! Shared local sales, including partial competition between related trades.
//!
//! Direct competitors share a market; related trades compete for a portion of the same
//! district's customer budget. No owner identity, account balance or criminal status enters
//! the calculation. Losing a competitor's capacity leaves additional custom for the others.

use super::BusinessEconomyError;
use crate::core::id::BusinessId;
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::economy::{BusinessOperatingStatus, MarketSnapshot};
use crate::registry::Registry;
use crate::world::{BusinessKind, BusinessRecord};

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
        // Use twice the weight to preserve exact half-competitor arithmetic even when the
        // authored disruption weight is odd.
        let market_weight = 2 * u64::from(own_weight)
            + 2 * u64::from(self.healthy_competitors) * u64::from(FULL)
            + 2 * u64::from(self.damaged_competitors) * u64::from(damaged_weight)
            + u64::from(self.related_healthy_competitors) * u64::from(FULL)
            + u64::from(self.related_damaged_competitors) * u64::from(damaged_weight);
        let contested = u64::from(CONTESTED_BASIS_POINTS) * 2 * u64::from(FULL)
            / market_weight.max(2 * u64::from(FULL));
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

/// The subject is counted hypothetically as open even when forecasting a resumption.
/// Other stores with no established operating books or a suspended economy cannot serve
/// customers. Capture facts before any same-minute settlement closes another shop.
pub(super) fn resolve_market_snapshot(
    state: &AppState,
    business: BusinessId,
) -> Result<MarketSnapshot, BusinessEconomyError> {
    let record = super::validate_business(state, business)?;
    let mut snapshot = MarketSnapshot::default();
    for (peer, kind) in local_peers(state, record) {
        let Some(economy) = state.economy().get_business_economy(peer.id()) else {
            continue;
        };
        if economy.status() != BusinessOperatingStatus::Active {
            continue;
        }
        snapshot.add_competitor(kind, economy.is_disrupted(state.now()));
    }
    Ok(snapshot)
}

/// Acquisition forecasts must not act as a free live reconnaissance query. Assume all
/// neighboring directory peers are operating normally, regardless of actual private books.
pub(super) fn forecast_market_snapshot(
    state: &AppState,
    record: &BusinessRecord,
) -> MarketSnapshot {
    let mut snapshot = MarketSnapshot::default();
    for (_, kind) in local_peers(state, record) {
        snapshot.add_competitor(kind, false);
    }
    snapshot
}

/// Aggregate the forecast additional daily revenue across the planner's own active
/// businesses if the target loses capacity. Only public directory peers enter the
/// counterfactual; private foreign books or damage are never consulted.
pub(crate) fn forecast_competitor_disruption_gain(
    registry: &Registry,
    state: &AppState,
    target: BusinessId,
    active_stakes: &[BusinessId],
) -> Result<i128, BusinessEconomyError> {
    let target_record = super::validate_business(state, target)?;
    let damage_weight = registry.business_disruption().gross_basis_points();
    let mut total = 0_i128;
    for &stake in active_stakes {
        let record = super::validate_business(state, stake)?;
        if stake == target || record.neighborhood() != target_record.neighborhood() {
            continue;
        }
        let Some(kind) = competition(record.kind(), target_record.kind()) else {
            continue;
        };
        let healthy = forecast_market_snapshot(state, record);
        let mut damaged = healthy;
        match kind {
            Competition::Direct => damaged.healthy_competitors -= 1,
            Competition::Related => damaged.related_healthy_competitors -= 1,
        }
        damaged.add_competitor(kind, true);
        let improvement = damaged.gross_basis_points(false, damage_weight)
            - healthy.gross_basis_points(false, damage_weight);
        let gross = super::resolve_business_gross_potential(registry, state, stake)?;
        let cycle = registry
            .get_business(record.kind())
            .economics()
            .cycle()
            .as_minutes();
        total += i128::from(gross.cents()) * i128::from(improvement) * i128::from(DAY_MINUTES)
            / i128::from(cycle)
            / i128::from(FULL);
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
}
