//! Daily recovery decisions for suspended non-player legitimate businesses.
//!
//! Cycle settlement owns suspension. This module owns only the autonomous decision to reuse the
//! canonical resume token later, so there is still one status-mutation path. Businesses controlled
//! by the player organization remain explicit player decisions; independent, commercial, rival,
//! and character-owned businesses can recover when their current zero-variance economics are
//! positive again.

use super::{
    BusinessEconomyError, ValidatedBusinessEconomyStatusChange, resolve_cycle_financials,
    validate_resume_business_economy,
};
use crate::core::id::{BusinessId, NeighborhoodId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::{BusinessKind, BusinessOwner};

struct RecoveryCandidate {
    business: BusinessId,
    neighborhood: NeighborhoodId,
    kind: BusinessKind,
    projected_daily_net: i128,
    resume: ValidatedBusinessEconomyStatusChange,
}

pub(crate) fn apply_due_autonomous_business_lifecycle(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<BusinessId>, BusinessEconomyError> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Ok(Vec::new());
    }

    let player_organization = state.player_organization();
    // The owner maintains a suspended-only derived index in BusinessId order. Collect first
    // because canonical resume mutates that same live-work projection.
    let suspended: Vec<_> = state
        .economy()
        .suspended_business_economies()
        .map(|economy| economy.business())
        .collect();
    // Rank the entire viable recovery queue before allocating scarce local customer demand.
    // Business-ID order reflects when a shop was created, not which closure is worth reversing.
    // Two simultaneous entrants would each forecast a market without the other, so only the
    // strongest projected daily earner in a competing sector may reopen on this boundary.
    let mut candidates = Vec::new();

    for business_id in suspended {
        let economy = state
            .economy()
            .get_business_economy(business_id)
            .ok_or(BusinessEconomyError::MissingBusinessEconomy(business_id))?;
        // A loss threshold can suspend exactly on this daily boundary. Reopening immediately
        // would erase the consequence and reset the loss streak without a real recovery interval.
        if economy.last_cycle_at() == Some(state.now()) {
            continue;
        }
        let business = state
            .world()
            .get_business(business_id)
            .ok_or(BusinessEconomyError::MissingBusiness(business_id))?;
        if matches!(
            business.owner(),
            BusinessOwner::Organization(owner) if Some(owner) == player_organization
        ) {
            continue;
        }
        let neighborhood = state
            .world()
            .get_neighborhood(business.neighborhood())
            .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(
                business_id,
            ))?;
        let (_, _, expected_net_cash) = resolve_cycle_financials(
            business_id,
            registry.get_business(business.kind()).economics(),
            neighborhood.profile(),
            registry.business_disruption().gross_basis_points(),
            economy.is_disrupted(state.now()),
            super::market::resolve_market_snapshot(state, business_id)?,
            0,
        )?;
        if expected_net_cash <= Money::ZERO {
            continue;
        }

        match validate_resume_business_economy(registry, state, business_id) {
            Ok(resume) => {
                // Compare earnings on the same time basis even if authored cycles differ.
                // The i128 intermediate handles every finite Money value without overflow.
                let daily_net = i128::from(expected_net_cash.cents()) * i128::from(DAY_MINUTES)
                    / i128::from(
                        registry
                            .get_business(business.kind())
                            .economics()
                            .cycle()
                            .as_minutes(),
                    );
                candidates.push(RecoveryCandidate {
                    business: business_id,
                    neighborhood: business.neighborhood(),
                    kind: business.kind(),
                    projected_daily_net: daily_net,
                    resume,
                });
            }
            // Finite version or clock capacity is a valid terminal rail. The business remains
            // suspended rather than turning routine autonomous maintenance into a campaign
            // failure merely because no future recurring cycle can be represented.
            Err(BusinessEconomyError::VersionCapacity(_))
            | Err(BusinessEconomyError::SimulationTimeOverflow) => {}
            Err(error) => return Err(error),
        }
    }

    candidates.sort_unstable_by(|left, right| {
        right
            .projected_daily_net
            .cmp(&left.projected_daily_net)
            .then(left.business.cmp(&right.business))
    });
    let mut reopening_sectors = std::collections::BTreeSet::new();
    let mut planned = Vec::new();
    for candidate in candidates {
        if reopening_sectors.iter().any(|&(district, kind)| {
            district == candidate.neighborhood
                && super::market::market_trades_compete(kind, candidate.kind)
        }) {
            continue;
        }
        reopening_sectors.insert((candidate.neighborhood, candidate.kind));
        planned.push((candidate.business, candidate.resume));
    }
    // The economic choice is independent of ID order; committing and reporting in stable ID
    // order keeps unrelated sectors' recovery chronology deterministic and easy to inspect.
    planned.sort_unstable_by_key(|(business_id, _)| *business_id);

    // The daily recovery pass is one fallible planning cohort. All businesses were evaluated
    // against the same immutable state above, and status changes touch distinct business-economy
    // records, so no later validation error can follow an earlier recovery mutation.
    let mut resumed = Vec::with_capacity(planned.len());
    for (business_id, resume) in planned {
        resume
            .commit(state)
            .expect("preplanned autonomous business recovery must remain current within one pass");
        resumed.push(business_id);
    }
    Ok(resumed)
}
