//! Daily recovery decisions for suspended non-player legitimate businesses.
//!
//! Cycle settlement owns suspension. This module owns only the autonomous decision to reuse the
//! canonical resume token later, so there is still one status-mutation path. Businesses controlled
//! by the player organization remain explicit player decisions; independent, commercial, rival,
//! and character-owned businesses can recover when their current zero-variance economics are
//! positive again.

use super::{BusinessEconomyError, resolve_cycle_financials, validate_resume_business_economy};
use crate::core::id::BusinessId;
use crate::core::state::AppState;
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::BusinessOwner;

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
    let mut resumed = Vec::new();

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
            0,
        )?;
        if expected_net_cash <= Money::ZERO {
            continue;
        }

        match validate_resume_business_economy(registry, state, business_id) {
            Ok(resume) => {
                resume.commit(state)?;
                resumed.push(business_id);
            }
            // Finite version capacity is a valid terminal rail. The business remains suspended
            // rather than turning routine autonomous maintenance into a campaign failure.
            Err(BusinessEconomyError::VersionCapacity(_)) => {}
            Err(error) => return Err(error),
        }
    }

    Ok(resumed)
}
