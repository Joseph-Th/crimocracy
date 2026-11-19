//! Shared local sales: most custom is venue-specific, a small portion is contested.
//!
//! The same kind in the same district is the market boundary. No owner identity, account
//! balance or hidden criminal status enters the calculation. A damaged venue loses capacity;
//! the contested custom it cannot serve becomes available to its healthy competitors. The
//! pool cannot make a venue earn more than its undisputed gross potential.

use super::BusinessEconomyError;
use crate::core::id::BusinessId;
use crate::core::state::AppState;
use crate::economy::{BusinessOperatingStatus, MarketSnapshot};
use crate::world::BusinessRecord;

/// One fifth of normal potential is contested custom. The other four fifths represent
/// venue-specific regulars and location advantage. This is a market structure assumption,
/// not a player-specific difficulty or time multiplier.
const CONTESTED_BASIS_POINTS: u32 = 2_000;
const FULL: u32 = 10_000;

impl MarketSnapshot {
    pub(crate) fn gross_basis_points(self, disrupted: bool, damaged_weight: u32) -> u32 {
        let own_weight = if disrupted { damaged_weight } else { FULL };
        let market_weight = u64::from(own_weight)
            + u64::from(self.healthy_competitors) * u64::from(FULL)
            + u64::from(self.damaged_competitors) * u64::from(damaged_weight);
        // Cap the recoverable contested portion at the venue's own normal potential.
        let contested = u64::from(CONTESTED_BASIS_POINTS) * u64::from(FULL)
            / market_weight.max(u64::from(FULL));
        FULL - CONTESTED_BASIS_POINTS + u32::try_from(contested).expect("contested share bounded")
    }

    pub(crate) fn competitor_count(self) -> u64 {
        u64::from(self.healthy_competitors) + u64::from(self.damaged_competitors)
    }
}

fn local_peers<'a>(
    state: &'a AppState,
    business: &'a BusinessRecord,
) -> impl Iterator<Item = &'a BusinessRecord> {
    state
        .world()
        .businesses_in_neighborhood(business.neighborhood())
        .filter(move |peer| peer.id() != business.id() && peer.kind() == business.kind())
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
    for peer in local_peers(state, record) {
        let Some(economy) = state.economy().get_business_economy(peer.id()) else {
            continue;
        };
        if economy.status() != BusinessOperatingStatus::Active {
            continue;
        }
        if economy.is_disrupted(state.now()) {
            snapshot.damaged_competitors += 1;
        } else {
            snapshot.healthy_competitors += 1;
        }
    }
    Ok(snapshot)
}

/// Acquisition forecasts must not act as a free live reconnaissance query. Assume all
/// neighboring directory peers are operating normally, regardless of actual private books.
pub(super) fn forecast_market_snapshot(
    state: &AppState,
    record: &BusinessRecord,
) -> MarketSnapshot {
    MarketSnapshot {
        healthy_competitors: u32::try_from(local_peers(state, record).count())
            .expect("business count fits the persistent ID space"),
        damaged_competitors: 0,
    }
}

/// Historical status is a frozen input like disruption and variance. The immutable
/// district/kind directory bounds the number of distinct competitors it can represent.
pub(super) fn validate_historical_market(
    state: &AppState,
    record: &BusinessRecord,
    snapshot: MarketSnapshot,
    occurred_at: crate::core::time::SimTime,
) -> Result<(), BusinessEconomyError> {
    let established_peers = local_peers(state, record)
        .filter(|peer| {
            state
                .economy()
                .get_business_economy(peer.id())
                .is_some_and(|economy| economy.established_at() <= occurred_at)
        })
        .count();
    if snapshot.competitor_count()
        > u64::try_from(established_peers).expect("business count fits the persistent ID space")
    {
        return Err(BusinessEconomyError::InvalidMarketSnapshot(record.id()));
    }
    Ok(())
}
