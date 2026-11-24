//! Public organizational prominence used by independent criminal and civic opponents.
//!
//! Property titles and paid political machine influence are public. Hidden balances,
//! investigations, racket locations, and unshared intelligence are not inputs. Deriving
//! this from authoritative records avoids a second notoriety meter or save migration.

use crate::core::id::OrganizationId;
use crate::core::state::AppState;
use std::collections::BTreeSet;

/// Bounded public footprint, from an inconspicuous new outfit (0) to a citywide
/// power broker (6). Ownership across districts and visible political activity
/// matter in addition to raw property count; an opening two-shop organization
/// remains at the baseline.
pub fn public_prominence_tier(state: &AppState, owner: OrganizationId) -> u8 {
    let mut districts = BTreeSet::new();
    let mut titles = 0_u32;
    for business in state.world().businesses_owned_by_organization(owner) {
        titles = titles.saturating_add(1);
        districts.insert(business.neighborhood());
    }
    // Political activity is public even in wards without owned properties.
    let visible_machine = state.politics().organization(owner).map_or(0, |campaign| {
        campaign.district_campaigns().fold(0_u32, |total, office| {
            total.saturating_add(u32::from(office.influence()) / 250)
        })
    });
    let footprint = titles.min(30)
        + u32::try_from(districts.len()).unwrap_or(u32::MAX).min(8) * 2
        + visible_machine.min(30);
    (footprint / 5).min(6) as u8
}

/// Per-cycle vice attention on an operating racket. Visibility matters, but a
/// portfolio of legitimate storefronts does not make every criminal enterprise
/// an independent new police lead every day. The first two commercial tiers
/// and the first four operating rackets do not multiply routine vice exposure;
/// additional footprint raises scrutiny in steps. Casework adds at most two
/// exposure sources, since ten related open files do not create ten independent
/// daily chances to discover the same operating business.
/// The case count is supplied by the legal owner; callers with only an old
/// observation can use zero or that observed count without consulting hidden files.
pub fn vice_inquiry_chance_basis_points(
    state: &AppState,
    owner: OrganizationId,
    active_rackets: usize,
    district_cases: u32,
    authored_per_source: u16,
) -> u32 {
    let sources = vice_exposure_sources(
        public_prominence_tier(state, owner),
        active_rackets,
        district_cases,
    );
    u32::try_from((u64::from(authored_per_source).saturating_mul(sources)).min(10_000))
        .expect("bounded basis-point chance fits u32")
}

fn vice_exposure_sources(prominence: u8, active_rackets: usize, district_cases: u32) -> u64 {
    let portfolio_sources = (active_rackets.saturating_sub(1) / 4).min(2) as u64;
    u64::from(district_cases.min(2))
        .saturating_add(1)
        .saturating_add(u64::from(prominence / 3))
        .saturating_add(portfolio_sources)
}

#[cfg(test)]
mod tests {
    use super::vice_exposure_sources;

    #[test]
    fn criminal_vice_exposure_scales_with_real_footprint_without_runaway_feedback() {
        assert_eq!(vice_exposure_sources(0, 1, 0), 1);
        assert_eq!(vice_exposure_sources(2, 4, 0), 1);
        assert_eq!(vice_exposure_sources(2, 5, 0), 2);
        assert_eq!(vice_exposure_sources(3, 5, 0), 3);
        assert_eq!(vice_exposure_sources(6, 10, 20), 7);
        assert_eq!(vice_exposure_sources(6, 100, u32::MAX), 7);
    }
}
