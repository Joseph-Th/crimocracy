//! Actual carrying costs of a growing criminal organization's legitimate holding company.
//!
//! Established shops employ their own workers and earn their own revenue. But central
//! ownership of many shops across wards incurs accounting, licensing, compliance and
//! integration expense. The owner pays this expense out of every real profit distribution,
//! so leaving money in the till does not evade it. The same calculation applies to direct
//! owner draws, automatic sweeps and the political income-coverage test.

use crate::core::id::{BusinessId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
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
    PortfolioAdministration::for_owner(state, owner, None).basis_points()
}

/// Read-only ownership projection shared by actual distributions and investment decisions.
/// Excluding a sold property lets the seller assess its remaining wage base without mutating
/// title or inspecting another owner's private books.
pub(crate) struct PortfolioAdministration {
    properties: usize,
    districts: BTreeSet<NeighborhoodId>,
    directors: usize,
}

impl PortfolioAdministration {
    pub(crate) fn for_owner(
        state: &AppState,
        owner: OrganizationId,
        excluded: Option<BusinessId>,
    ) -> Self {
        let mut properties = 0;
        let mut districts = BTreeSet::new();
        for asset in state.world().businesses_owned_by_organization(owner) {
            if Some(asset.id()) != excluded {
                properties += 1;
                districts.insert(asset.neighborhood());
            }
        }
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
        Self {
            properties,
            districts,
            directors,
        }
    }

    pub(crate) fn property_count(&self) -> usize {
        self.properties
    }

    pub(crate) fn contains_district(&self, district: NeighborhoodId) -> bool {
        self.districts.contains(&district)
    }

    pub(crate) fn basis_points(&self) -> u16 {
        self.rate(self.properties, self.districts.len())
    }

    pub(crate) fn basis_points_after_purchase(&self, adds_district: bool) -> u16 {
        self.rate(
            self.properties.saturating_add(1),
            self.districts.len() + usize::from(adds_district),
        )
    }

    fn rate(&self, properties: usize, districts: usize) -> u16 {
        let property_cost = properties.saturating_sub(2).saturating_mul(300);
        let geography = districts.saturating_sub(1).saturating_mul(100);
        let gross = property_cost.saturating_add(geography).min(5_500);
        let savings = self.directors.saturating_mul(400).min(gross / 2);
        u16::try_from(gross - savings)
            .expect("administration fee fits the bounded basis-point range")
    }
}

/// Sustainable daily owner earnings: only a positive cycle can incur a distribution fee.
/// A loss remains a full economic loss rather than becoming cheaper at a higher fee rate.
pub(crate) fn daily_owner_net(cycle_net: Money, cycle_minutes: u32, basis_points: u16) -> i128 {
    let fee = portfolio_overhead_charge(cycle_net.max(Money::ZERO), basis_points);
    (i128::from(cycle_net.cents()) - i128::from(fee.cents())) * i128::from(DAY_MINUTES)
        / i128::from(cycle_minutes)
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
