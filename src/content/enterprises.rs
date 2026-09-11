//! Authored enterprise economics and physical/network infrastructure requirements.

use crate::core::time::DAY_DURATION;
use crate::enterprises::EnterpriseKind;
use crate::finance::Money;
use crate::registry::{EnterpriseEconomicsDefinition, RegistryBuilder};
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
                vice_attention_basis_points_per_active_case: 450,
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
                vice_attention_basis_points_per_active_case: 620,
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
                vice_attention_basis_points_per_active_case: 600,
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
            // Off-track style wagering on outside events: cash book with customer-facing
            // settlement. Lower ceiling than venue gambling but scales harder with illicit
            // demand and is less dependent on a specific social venue.
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
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                vice_attention_basis_points_per_active_case: 650,
                gross_variance_basis_points: 2_200,
                notable_variance_basis_points: 1_400,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            BTreeSet::new(),
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
                vice_attention_basis_points_per_active_case: 250,
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
                vice_attention_basis_points_per_active_case: 180,
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
                vice_attention_basis_points_per_active_case: 720,
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
                vice_attention_basis_points_per_active_case: 280,
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 650,
                losing_cycles_before_suspension: 3,
            },
            BTreeSet::new(),
            BTreeSet::from([BusinessFunction::UnionAccess, BusinessFunction::Warehousing]),
        ),
    ];
    for (kind, economics, required_business_functions, required_network_functions) in definitions {
        builder
            .register_enterprise(
                kind,
                economics,
                required_business_functions,
                required_network_functions,
            )
            .unwrap_or_else(|error| panic!("invalid enterprise registry: {error}"));
    }
}
