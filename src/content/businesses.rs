//! Authored legitimate-business economics, disruption, laundering, and shared venue functions.

use crate::core::time::{DAY_DURATION, SimDuration};
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
                cycle: DAY_DURATION,
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
        (
            // Construction contractors are capital-heavy legitimate businesses whose value is
            // driven primarily by commercial activity rather than neighborhood wealth. In the
            // 1929-1935 setting they are also a natural authored home for UnionAccess,
            // Warehousing, and VehicleFleet functions, making one company strategically useful
            // to labor rackets without hard-coding a racket to one business kind.
            BusinessKind::Construction,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(19_000),
                base_operating_cost: Money::from_cents(16_000),
                wealth_revenue_per_point: Money::from_cents(25),
                commerce_revenue_per_point: Money::from_cents(130),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 650,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(105_000),
            },
        ),
        (
            // Wholesale distributors turn commercial throughput into legitimate revenue without
            // duplicating a trucking fleet or retail storefront. Authored instances can carry
            // Warehousing, DistributionInfrastructure, and CustomerAccess functions, making
            // distribution networks useful before and after Prohibition.
            BusinessKind::Wholesale,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(17_500),
                base_operating_cost: Money::from_cents(15_000),
                wealth_revenue_per_point: Money::from_cents(15),
                commerce_revenue_per_point: Money::from_cents(115),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 650,
                notable_variance_basis_points: 500,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(82_000),
            },
        ),
        (
            // Pawnshops are modest legitimate businesses with steady neighborhood trade and a
            // natural authored home for ResaleMarket, CashIntensive, and CustomerAccess. They
            // make stolen-property and cash networks useful without requiring a purpose-built
            // criminal venue.
            BusinessKind::Pawnshop,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(11_500),
                base_operating_cost: Money::from_cents(8_500),
                wealth_revenue_per_point: Money::from_cents(45),
                commerce_revenue_per_point: Money::from_cents(65),
                police_cost_per_point: Money::from_cents(30),
                gross_variance_basis_points: 850,
                notable_variance_basis_points: 650,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(38_000),
            },
        ),
        (
            // Coin-machine distributors are period-specific equipment businesses: legitimate
            // commercial revenue backed by route sales and service work, with authored instances
            // able to supply vehicle, customer, and distribution functions to several rackets.
            BusinessKind::CoinMachineDistribution,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(15_500),
                base_operating_cost: Money::from_cents(12_500),
                wealth_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(105),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 750,
                notable_variance_basis_points: 550,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(68_000),
            },
        ),
        (
            // Hotels and boarding houses are wealth-sensitive legitimate businesses that create
            // a distinct lodging front instead of treating every restaurant or tavern as suitable
            // private accommodation. Authored properties can combine Lodging, CustomerAccess,
            // MeetingSpace, and CashIntensive functions depending on the actual establishment.
            BusinessKind::Lodging,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(16_500),
                base_operating_cost: Money::from_cents(13_000),
                wealth_revenue_per_point: Money::from_cents(95),
                commerce_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 600,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(76_000),
            },
        ),
        (
            // Athletic clubs and neighborhood gyms are moderate-margin recreation businesses.
            // Their strategic value comes from authored SportingVenue and MeetingSpace functions,
            // allowing period prizefighting without making every social club a boxing arena.
            BusinessKind::AthleticClub,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(12_500),
                base_operating_cost: Money::from_cents(10_000),
                wealth_revenue_per_point: Money::from_cents(55),
                commerce_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 650,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(45_000),
            },
        ),
        (
            // Commercial laundries are intentionally modest, stable fronts. Period organized
            // crime records show both direct pressure on laundry operators and later investment
            // in laundries; authored instances can contribute cash handling, customer access,
            // or a delivery fleet without becoming a bespoke criminal building.
            BusinessKind::Laundry,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(11_000),
                base_operating_cost: Money::from_cents(9_000),
                wealth_revenue_per_point: Money::from_cents(25),
                commerce_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(15),
                gross_variance_basis_points: 550,
                notable_variance_basis_points: 450,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(34_000),
            },
        ),
        (
            // Commercial printers are ordinary record-heavy businesses whose presses and trade
            // relationships make them strategically distinct from generic professional offices.
            // Authored print shops can expose PrintingPress and ProfessionalRecords functions,
            // while customer/distribution capacity remains a property of the actual business.
            BusinessKind::Printing,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(13_500),
                base_operating_cost: Money::from_cents(10_500),
                wealth_revenue_per_point: Money::from_cents(35),
                commerce_revenue_per_point: Money::from_cents(85),
                police_cost_per_point: Money::from_cents(15),
                gross_variance_basis_points: 650,
                notable_variance_basis_points: 500,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(48_000),
            },
        ),
        (
            // Finance companies, insurance agencies, and similar offices are wealth-sensitive
            // legitimate businesses. Authored instances can expose FinancialServices,
            // ProfessionalRecords, and CustomerAccess without treating every accountant or law
            // office as a source of credit and financial-paper access.
            BusinessKind::FinancialServices,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(15_000),
                base_operating_cost: Money::from_cents(10_500),
                wealth_revenue_per_point: Money::from_cents(120),
                commerce_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(15),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 600,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(72_000),
            },
        ),
        (
            // Garment and fur manufacturing were major urban industries and historically fertile
            // ground for labor and industrial racketeering. These are capital-heavy,
            // commerce-driven businesses whose authored instances can expose UnionAccess,
            // Warehousing, and DistributionInfrastructure to the wider criminal network.
            BusinessKind::GarmentFactory,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(18_000),
                base_operating_cost: Money::from_cents(15_500),
                wealth_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(125),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 700,
                notable_variance_basis_points: 500,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(88_000),
            },
        ),
        (
            // Stevedoring firms sit at the physical boundary between ships and the city's
            // warehouses and trucks. They are capital-heavy commerce businesses whose authored
            // instances can expose DockAccess alongside warehousing, union access, and
            // distribution infrastructure without modeling individual piers or cargo lots.
            BusinessKind::Stevedoring,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(19_500),
                base_operating_cost: Money::from_cents(17_000),
                wealth_revenue_per_point: Money::from_cents(15),
                commerce_revenue_per_point: Money::from_cents(140),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 650,
                notable_variance_basis_points: 500,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(96_000),
            },
        ),
        (
            // News and wire services monetize timely commercial information rather than a
            // physical retail flow. Authored instances can expose RacingWire when they carry
            // rapid track reports, making that infrastructure distinct from owning a printer.
            BusinessKind::NewsService,
            BusinessEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(13_000),
                base_operating_cost: Money::from_cents(10_500),
                wealth_revenue_per_point: Money::from_cents(30),
                commerce_revenue_per_point: Money::from_cents(95),
                police_cost_per_point: Money::from_cents(10),
                gross_variance_basis_points: 550,
                notable_variance_basis_points: 450,
                losing_cycles_before_suspension: 3,
                acquisition_cost: Money::from_cents(52_000),
            },
        ),
    ];
    for (kind, economics) in definitions {
        builder
            .register_business(kind, economics)
            .unwrap_or_else(|error| panic!("invalid business registry: {error}"));
    }
}
