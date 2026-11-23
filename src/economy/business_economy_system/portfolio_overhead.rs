//! Actual carrying costs of a growing criminal organization's legitimate holding company.
//!
//! Established shops employ their own workers and earn their own revenue. But central
//! ownership of many shops across wards incurs accounting, licensing, compliance and
//! integration expense. The owner pays this expense out of every real profit distribution,
//! so leaving money in the till does not evade it. The same calculation applies to direct
//! owner draws, automatic sweeps and the political income-coverage test.

use crate::core::id::OrganizationId;
use crate::core::state::AppState;
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::Money;
use crate::world::AutonomyLevel;
use std::collections::BTreeSet;

/// Three percent for each owned storefront beyond two, plus one percent for every
/// additional geographic district, capped at 55 percent. Up to half the fee can
/// be avoided by financing actual independent financial managers: each qualified,
/// undetained manager with an active Finance mandate removes four percentage
/// points. This benefits the player only by retaining people on payroll and
/// maintaining genuine delegation, not by hiring nominal unassigned personnel.
/// Suspended owned sites still carry a central administration burden.
pub fn portfolio_overhead_basis_points(state: &AppState, owner: OrganizationId) -> u16 {
    let assets = state
        .world()
        .businesses_owned_by_organization(owner)
        .collect::<Vec<_>>();
    let districts = assets
        .iter()
        .map(|business| business.neighborhood())
        .collect::<BTreeSet<_>>()
        .len();
    let properties = assets.len().saturating_sub(2).saturating_mul(300);
    let geography = districts.saturating_sub(1).saturating_mul(100);
    let gross = properties.saturating_add(geography).min(5_500);
    let directors = state
        .delegation()
        .active_for_scope(ResponsibilityScope::Function(
            ResponsibilityFunction::Finance,
        ))
        .filter(|mandate| mandate.organization() == owner)
        .filter_map(|mandate| {
            let member = state.world().get_character(mandate.manager())?;
            (member.organization() == Some(owner)
                && member.supervisor().is_some()
                && matches!(
                    member.autonomy(),
                    AutonomyLevel::Delegated | AutonomyLevel::Broad
                )
                && state
                    .legal()
                    .active_arrest_for_character(member.id())
                    .is_none())
            .then_some(member.id())
        })
        .collect::<BTreeSet<_>>()
        .len();
    let savings = directors.saturating_mul(400).min(gross / 2);
    u16::try_from(gross - savings).expect("administration fee fits the bounded basis-point range")
}

/// Round up so that breaking a transfer into many one-cent withdrawals cannot
/// reduce the expense. The rate is capped, so the owner retains positive proceeds.
pub fn portfolio_overhead_charge(gross: Money, basis_points: u16) -> Money {
    debug_assert!(gross >= Money::ZERO && basis_points <= 5_500);
    if gross == Money::ZERO || basis_points == 0 {
        return Money::ZERO;
    }
    let cents = (i128::from(gross.cents()) * i128::from(basis_points) + 9_999) / 10_000;
    Money::from_cents(i64::try_from(cents).expect("fee never exceeds gross payment"))
}
