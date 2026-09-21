//! Authored enterprise economics and physical/network infrastructure requirements.

use crate::core::time::DAY_DURATION;
use crate::enterprises::EnterpriseKind;
use crate::finance::Money;
use crate::registry::{EnterpriseEconomicsDefinition, EnterpriseNetworkMode, RegistryBuilder};
use crate::world::BusinessFunction;
use std::collections::BTreeSet;

pub(super) fn register_enterprises(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            EnterpriseKind::Protection,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(4_000),
                base_operating_cost: Money::from_cents(2_500),
                demand_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(140),
                wealth_revenue_per_point: Money::from_cents(60),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(35),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 450,
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 600,
                losing_cycles_before_suspension: 3,
            },
            // No special standing-order authority is required to manage a protection racket.
            BTreeSet::new(),
            BTreeSet::new(),
        ),
        (
            EnterpriseKind::Gambling,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(8_000),
                base_operating_cost: Money::from_cents(4_500),
                demand_revenue_per_point: Money::from_cents(160),
                commerce_revenue_per_point: Money::from_cents(40),
                wealth_revenue_per_point: Money::from_cents(100),
                management_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(45),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 620,
                gross_variance_basis_points: 1_200,
                notable_variance_basis_points: 900,
                losing_cycles_before_suspension: 3,
            },
            super::businesses::gambling_venue_functions(),
            BTreeSet::new(),
        ),
        (
            EnterpriseKind::AlcoholDistribution,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(16_000),
                base_operating_cost: Money::from_cents(10_000),
                demand_revenue_per_point: Money::from_cents(130),
                commerce_revenue_per_point: Money::from_cents(50),
                wealth_revenue_per_point: Money::from_cents(25),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(40),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 600,
                gross_variance_basis_points: 1_800,
                notable_variance_basis_points: 1_200,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::new(),
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
                BusinessFunction::CustomerAccess,
            ]),
        ),
        (
            // Off-track wagering requires a cash/customer-facing book plus timely outside race
            // information. The racing wire is a network dependency rather than a delivery task:
            // the organization owns or controls access to the service, while individual calls,
            // sheets, and race updates remain below the strategic simulation layer.
            EnterpriseKind::Bookmaking,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(6_500),
                base_operating_cost: Money::from_cents(4_000),
                demand_revenue_per_point: Money::from_cents(190),
                commerce_revenue_per_point: Money::from_cents(30),
                wealth_revenue_per_point: Money::from_cents(70),
                management_revenue_per_point: Money::from_cents(60),
                police_cost_per_point: Money::from_cents(40),
                // A wire feed is a paid information dependency, but it should not carry the
                // same overhead as a warehouse, fleet, or other physically intensive support
                // business. This keeps the service economically meaningful without making a
                // historically essential input erase bookmaking's strategic niche.
                support_surcharge_per_business: Money::from_cents(3_000),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 650,
                gross_variance_basis_points: 2_200,
                notable_variance_basis_points: 1_400,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            BTreeSet::from([BusinessFunction::RacingWire]),
        ),
        (
            // Collection-driven lending: revenue follows district wealth rather than
            // commerce foot traffic, with the lowest police cost of the cash rackets.
            EnterpriseKind::LoanSharking,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(7_000),
                base_operating_cost: Money::from_cents(3_500),
                demand_revenue_per_point: Money::from_cents(80),
                commerce_revenue_per_point: Money::from_cents(20),
                wealth_revenue_per_point: Money::from_cents(180),
                management_revenue_per_point: Money::from_cents(65),
                police_cost_per_point: Money::from_cents(25),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 250,
                gross_variance_basis_points: 700,
                notable_variance_basis_points: 550,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([BusinessFunction::CashIntensive]),
            BTreeSet::new(),
        ),
        (
            // Resale channel for stolen property: depends on legitimate commercial churn
            // to move goods, not on district demand for vice.
            EnterpriseKind::Fencing,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(5_500),
                base_operating_cost: Money::from_cents(3_000),
                demand_revenue_per_point: Money::from_cents(40),
                commerce_revenue_per_point: Money::from_cents(160),
                wealth_revenue_per_point: Money::from_cents(50),
                management_revenue_per_point: Money::from_cents(50),
                police_cost_per_point: Money::from_cents(30),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 180,
                gross_variance_basis_points: 1_000,
                notable_variance_basis_points: 750,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::ResaleMarket,
                BusinessFunction::Warehousing,
            ]),
            BTreeSet::new(),
        ),
        (
            // The Prohibition core: a concealed nightlife venue selling illicit alcohol.
            // High gross and police cost, scales with wealth and commerce, and is the
            // most vice-exposed racket. Requires a nightlife venue and a supply chain
            // that can produce and move alcohol.
            EnterpriseKind::Speakeasy,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(12_000),
                base_operating_cost: Money::from_cents(6_500),
                demand_revenue_per_point: Money::from_cents(120),
                commerce_revenue_per_point: Money::from_cents(80),
                wealth_revenue_per_point: Money::from_cents(100),
                management_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(55),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 720,
                gross_variance_basis_points: 1_600,
                notable_variance_basis_points: 1_100,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::Nightlife,
                BusinessFunction::CustomerAccess,
            ]),
            BTreeSet::from([
                BusinessFunction::AlcoholProduction,
                BusinessFunction::DistributionInfrastructure,
            ]),
        ),
        (
            // Infiltration of the union hiring hall: extorts local businesses for
            // payroll kickbacks. Stable and management-driven, with lower vice
            // visibility than vice rackets but dependent on union access.
            EnterpriseKind::LaborRacketeering,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(9_500),
                base_operating_cost: Money::from_cents(5_200),
                demand_revenue_per_point: Money::from_cents(60),
                commerce_revenue_per_point: Money::from_cents(90),
                wealth_revenue_per_point: Money::from_cents(40),
                management_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(30),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 280,
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 650,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::new(),
            BTreeSet::from([BusinessFunction::UnionAccess, BusinessFunction::Warehousing]),
        ),
        (
            // Neighborhood "policy" / numbers gambling is deliberately not another card-room
            // venue. Writers and collectors can work a district through ordinary customer-facing
            // cash fronts feeding a record-keeping office/bank, which gives multiple legitimate
            // business functions a strategic role without turning the game into runner micro.
            EnterpriseKind::NumbersRacket,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(5_000),
                base_operating_cost: Money::from_cents(3_600),
                demand_revenue_per_point: Money::from_cents(165),
                commerce_revenue_per_point: Money::from_cents(20),
                wealth_revenue_per_point: Money::from_cents(30),
                management_revenue_per_point: Money::from_cents(65),
                police_cost_per_point: Money::from_cents(30),
                support_surcharge_per_business: Money::from_cents(4_000),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 360,
                gross_variance_basis_points: 700,
                notable_variance_basis_points: 500,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::new(),
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
        ),
        (
            // Distributed coin-machine gambling: machines sit in ordinary stores, clubs, and
            // back rooms while an operator services and collects the route. Model the route at
            // district scale instead of making the player dispatch collectors to every cabinet.
            // It therefore rewards a vehicle/customer/distribution network with a cash-handling
            // collection point and remains distinct from a fixed gambling venue, a generic
            // liquor route, or the record-heavy numbers bank.
            EnterpriseKind::SlotMachineRoute,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(7_500),
                base_operating_cost: Money::from_cents(4_800),
                demand_revenue_per_point: Money::from_cents(125),
                commerce_revenue_per_point: Money::from_cents(85),
                wealth_revenue_per_point: Money::from_cents(45),
                management_revenue_per_point: Money::from_cents(60),
                police_cost_per_point: Money::from_cents(40),
                support_surcharge_per_business: Money::from_cents(4_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 500,
                gross_variance_basis_points: 1_000,
                notable_variance_basis_points: 700,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::new(),
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::VehicleFleet,
                BusinessFunction::CustomerAccess,
                BusinessFunction::DistributionInfrastructure,
            ]),
        ),
        (
            // A fixed prostitution house concealed behind a lodging business. Unlike a
            // speakeasy it does not depend on an alcohol production/distribution chain, making
            // it a durable vice option before and after repeal. Lodging keeps the front
            // semantically distinct from a generic restaurant or card room.
            EnterpriseKind::Brothel,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(10_500),
                base_operating_cost: Money::from_cents(6_000),
                demand_revenue_per_point: Money::from_cents(95),
                commerce_revenue_per_point: Money::from_cents(35),
                wealth_revenue_per_point: Money::from_cents(155),
                management_revenue_per_point: Money::from_cents(65),
                police_cost_per_point: Money::from_cents(55),
                support_surcharge_per_business: Money::from_cents(5_000),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 760,
                gross_variance_basis_points: 1_100,
                notable_variance_basis_points: 800,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::Lodging,
            ]),
            BTreeSet::new(),
        ),
        (
            // Staged prizefights with an attached betting book. This is a venue racket rather
            // than generic off-track bookmaking: the organization needs an actual sporting hall
            // plus cash-handling capacity. High variance reflects irregular gates and betting
            // volume while keeping routine fight promotion delegated instead of tactical.
            EnterpriseKind::PrizeFighting,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(7_000),
                base_operating_cost: Money::from_cents(4_200),
                demand_revenue_per_point: Money::from_cents(130),
                commerce_revenue_per_point: Money::from_cents(50),
                wealth_revenue_per_point: Money::from_cents(90),
                management_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(35),
                support_surcharge_per_business: Money::from_cents(4_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 420,
                gross_variance_basis_points: 2_000,
                notable_variance_basis_points: 1_200,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
                BusinessFunction::SportingVenue,
            ]),
            BTreeSet::from([BusinessFunction::CashIntensive]),
        ),
        (
            // Counterfeit notes require both manufacture and passing. The press and records
            // live at a printing front, while a separate commercial network can distribute the
            // notes through ordinary transactions. Modeling the network at enterprise scale
            // avoids making the player route individual bundles or merchant purchases.
            EnterpriseKind::Counterfeiting,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(8_500),
                base_operating_cost: Money::from_cents(5_500),
                demand_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(125),
                wealth_revenue_per_point: Money::from_cents(45),
                management_revenue_per_point: Money::from_cents(85),
                police_cost_per_point: Money::from_cents(30),
                support_surcharge_per_business: Money::from_cents(5_000),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 550,
                gross_variance_basis_points: 1_400,
                notable_variance_basis_points: 900,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::PrintingPress,
                BusinessFunction::ProfessionalRecords,
            ]),
            BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::DistributionInfrastructure,
            ]),
        ),
        (
            // Commercial fraud is a records-and-credit racket rather than another street vice
            // venue. It represents fraudulent credit, insurance, invoice, and bankruptcy schemes
            // run through a plausible financial office and fed by customer-facing access
            // somewhere in its business network. The organization governs the continuing scheme, not individual forged
            // invoices or claims.
            EnterpriseKind::Fraud,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(7_800),
                base_operating_cost: Money::from_cents(4_600),
                demand_revenue_per_point: Money::from_cents(30),
                commerce_revenue_per_point: Money::from_cents(95),
                wealth_revenue_per_point: Money::from_cents(110),
                management_revenue_per_point: Money::from_cents(90),
                police_cost_per_point: Money::from_cents(25),
                support_surcharge_per_business: Money::from_cents(4_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 400,
                gross_variance_basis_points: 1_500,
                notable_variance_basis_points: 900,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::FinancialServices,
                BusinessFunction::ProfessionalRecords,
            ]),
            BTreeSet::from([BusinessFunction::CustomerAccess]),
        ),
        (
            // A stolen-automobile ring needs more than generic fencing: a workshop must be able
            // to alter or disguise vehicle identity, while records and a resale/customer channel
            // support disposal. This keeps the ring at enterprise scale instead of introducing
            // individual car inventory or tactical driving.
            EnterpriseKind::AutoTheftRing,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(9_000),
                base_operating_cost: Money::from_cents(5_200),
                demand_revenue_per_point: Money::from_cents(35),
                commerce_revenue_per_point: Money::from_cents(120),
                wealth_revenue_per_point: Money::from_cents(100),
                management_revenue_per_point: Money::from_cents(75),
                police_cost_per_point: Money::from_cents(45),
                support_surcharge_per_business: Money::from_cents(4_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 520,
                gross_variance_basis_points: 1_300,
                notable_variance_basis_points: 850,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::VehicleWorkshop,
                BusinessFunction::ResaleMarket,
            ]),
            BTreeSet::from([
                BusinessFunction::ProfessionalRecords,
                BusinessFunction::CustomerAccess,
            ]),
        ),
        (
            // Standing smuggling is an import-and-distribution network, not an inventory minigame.
            // The host must have real dock access and storage; the wider business network must be
            // able to document, move, and distribute incoming contraband. This distinguishes the
            // racket from generic trucking, alcohol distribution, and one-off smuggling jobs.
            EnterpriseKind::Smuggling,
            EnterpriseEconomicsDefinition {
                cycle: DAY_DURATION,
                base_gross: Money::from_cents(10_000),
                base_operating_cost: Money::from_cents(6_800),
                demand_revenue_per_point: Money::from_cents(90),
                commerce_revenue_per_point: Money::from_cents(115),
                wealth_revenue_per_point: Money::from_cents(35),
                management_revenue_per_point: Money::from_cents(80),
                police_cost_per_point: Money::from_cents(50),
                support_surcharge_per_business: Money::from_cents(4_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                enforcement_attention_basis_points_per_active_case: 600,
                gross_variance_basis_points: 1_600,
                notable_variance_basis_points: 1_000,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([BusinessFunction::DockAccess, BusinessFunction::Warehousing]),
            BTreeSet::from([
                BusinessFunction::ProfessionalRecords,
                BusinessFunction::VehicleFleet,
                BusinessFunction::DistributionInfrastructure,
            ]),
        ),
    ];
    for (kind, economics, required_business_functions, required_network_functions) in definitions {
        let network_mode = match kind {
            // These rackets explicitly depend on infrastructure distinct from the physical host:
            // the betting room consumes an outside racing wire, while the print shop needs a
            // separate commercial channel to pass counterfeit notes.
            EnterpriseKind::Bookmaking | EnterpriseKind::Counterfeiting => {
                EnterpriseNetworkMode::SupportingBusinessesOnly
            }
            EnterpriseKind::Protection
            | EnterpriseKind::Gambling
            | EnterpriseKind::AlcoholDistribution
            | EnterpriseKind::LoanSharking
            | EnterpriseKind::Fencing
            | EnterpriseKind::Speakeasy
            | EnterpriseKind::LaborRacketeering
            | EnterpriseKind::NumbersRacket
            | EnterpriseKind::SlotMachineRoute
            | EnterpriseKind::Brothel
            | EnterpriseKind::PrizeFighting
            | EnterpriseKind::Fraud
            | EnterpriseKind::AutoTheftRing
            | EnterpriseKind::Smuggling => EnterpriseNetworkMode::HostMayContribute,
        };
        builder
            .register_enterprise(
                kind,
                economics,
                required_business_functions,
                required_network_functions,
                network_mode,
            )
            .unwrap_or_else(|error| panic!("invalid enterprise registry: {error}"));
    }
}
