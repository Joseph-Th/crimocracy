//! Authored legitimate-business economics, disruption, laundering, and shared venue functions.

use crate::core::time::SimDuration;
use crate::finance::Money;
use crate::registry::{
    BusinessDisruptionSpec, BusinessEconomicsDefinition, LaunderingConfigSpec, RegistryBuilder,
};
use crate::world::{BusinessFunction, BusinessKind};
use std::collections::BTreeSet;

pub(super) fn gambling_venue_functions() -> BTreeSet<BusinessFunction> {
    BTreeSet::from([
        BusinessFunction::CashIntensive,
        BusinessFunction::MeetingSpace,
        BusinessFunction::CustomerAccess,
    ])
}

pub(super) fn register_business_disruption(builder: &mut RegistryBuilder) {
    builder
        .register_business_disruption(BusinessDisruptionSpec {
            // Sabotage degrades a target's earning power for roughly two operating cycles:
            // long enough to matter strategically, short enough that repeated attacks,
            // not one attack, strangle a business.
            duration: SimDuration::from_minutes(2_880),
            gross_basis_points: 4_000,
        })
        .unwrap_or_else(|error| panic!("invalid business disruption registry: {error}"));
}

pub(super) fn register_laundering(builder: &mut RegistryBuilder) {
    builder
        .register_laundering(LaunderingConfigSpec {
            // The front keeps a meaningful cut: laundering is a service the legitimate
            // business charges for, not a free conversion button.
            fee_basis_points: 1_500,
            // A front can plausibly hide aggregate volume up to 100% of one legitimate cycle's
            // current gross. Splitting one sum across several transfers does not reset that
            // allowance, so larger diversification still requires additional fronts.
            plausibility_gross_basis_points: 10_000,
        })
        .unwrap_or_else(|error| panic!("invalid laundering registry: {error}"));
}

pub(super) fn register_businesses(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            BusinessKind::Retail,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(12_000),
                base_operating_cost: Money::from_cents(10_000),
                wealth_revenue_per_point: Money::from_cents(40),
                commerce_revenue_per_point: Money::from_cents(80),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 1_000,
                notable_variance_basis_points: 800,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(36_000),
            },
        ),
        (
            BusinessKind::Hospitality,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(15_000),
                base_operating_cost: Money::from_cents(12_000),
                wealth_revenue_per_point: Money::from_cents(60),
                commerce_revenue_per_point: Money::from_cents(90),
                police_cost_per_point: Money::from_cents(30),
                gross_variance_basis_points: 1_200,
                notable_variance_basis_points: 900,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(48_000),
            },
        ),
        (
            BusinessKind::Automotive,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(14_000),
                base_operating_cost: Money::from_cents(11_000),
                wealth_revenue_per_point: Money::from_cents(50),
                commerce_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 700,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(56_000),
            },
        ),
        (
            BusinessKind::Transportation,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(18_000),
                base_operating_cost: Money::from_cents(15_000),
                wealth_revenue_per_point: Money::from_cents(40),
                commerce_revenue_per_point: Money::from_cents(100),
                police_cost_per_point: Money::from_cents(35),
                gross_variance_basis_points: 700,
                notable_variance_basis_points: 600,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(90_000),
            },
        ),
        (
            BusinessKind::Warehouse,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(9_000),
                base_operating_cost: Money::from_cents(7_500),
                wealth_revenue_per_point: Money::from_cents(10),
                commerce_revenue_per_point: Money::from_cents(60),
                police_cost_per_point: Money::from_cents(15),
                gross_variance_basis_points: 500,
                notable_variance_basis_points: 450,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(27_000),
            },
        ),
        (
            BusinessKind::ProfessionalServices,
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(16_000),
                base_operating_cost: Money::from_cents(11_000),
                wealth_revenue_per_point: Money::from_cents(100),
                commerce_revenue_per_point: Money::from_cents(40),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 700,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(64_000),
            },
        ),
        (
            BusinessKind::Brewery,
            BusinessEconomicsDefinition {
                // Illicit production front: high throughput, high police attention, swings hard with
                // district demand. Priced as a premium infrastructure acquisition.
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(20_000),
                base_operating_cost: Money::from_cents(13_000),
                wealth_revenue_per_point: Money::from_cents(30),
                commerce_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(45),
                gross_variance_basis_points: 1_800,
                notable_variance_basis_points: 1_200,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(85_000),
            },
        ),
        (
            BusinessKind::Nightclub,
            BusinessEconomicsDefinition {
                // Speakeasy venue: cash-heavy nightlife front, the natural laundering and
                // customer-access hub for the Prohibition economy. Also the most plausible
                // storefront: customer throughput and till camouflage make it the best laundering
                // front per-dollar of gross.
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(17_000),
                base_operating_cost: Money::from_cents(11_000),
                wealth_revenue_per_point: Money::from_cents(90),
                commerce_revenue_per_point: Money::from_cents(110),
                police_cost_per_point: Money::from_cents(50),
                gross_variance_basis_points: 1_400,
                notable_variance_basis_points: 1_000,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(70_000),
            },
        ),
    ];
    for (kind, economics) in definitions {
        builder
            .register_business(kind, economics)
            .unwrap_or_else(|error| panic!("invalid business registry: {error}"));
    }
}
