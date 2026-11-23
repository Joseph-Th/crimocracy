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
    let visible_machine: u32 = districts
        .iter()
        .map(|district| {
            state
                .politics()
                .organization(owner)
                .and_then(|campaign| campaign.district(*district))
                .map_or(0, |office| u32::from(office.influence()) / 250)
        })
        .sum();
    let footprint = titles.min(30)
        + u32::try_from(districts.len()).unwrap_or(u32::MAX).min(8) * 2
        + visible_machine;
    (footprint / 5).min(6) as u8
}

/// Per-cycle vice attention on an operating racket. The public footprint and
/// number of *actually operating* rackets raise scrutiny, but neither creates
/// admissible evidence or a police case without the normal cycle/intake roll.
/// The case count is supplied by the legal owner; callers with only an old
/// observation can use zero or that observed count without consulting hidden files.
pub fn vice_inquiry_chance_basis_points(
    state: &AppState,
    owner: OrganizationId,
    active_rackets: usize,
    district_cases: u32,
    authored_per_source: u16,
) -> u32 {
    let portfolio_sources = (active_rackets.saturating_sub(1) / 2).min(3) as u64;
    let sources = u64::from(district_cases)
        .saturating_add(1)
        .saturating_add(u64::from(public_prominence_tier(state, owner)))
        .saturating_add(portfolio_sources);
    u32::try_from((u64::from(authored_per_source).saturating_mul(sources)).min(10_000))
        .expect("bounded basis-point chance fits u32")
}
