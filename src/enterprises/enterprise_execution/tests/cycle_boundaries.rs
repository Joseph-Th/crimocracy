//! Exact enterprise-cycle boundaries that protect economic and enforcement semantics.

use super::*;

#[test]
fn enterprise_variance_accepts_the_authored_limit_and_rejects_the_next_basis_point() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let limit = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .gross_variance_basis_points();
    let limit = i16::try_from(limit).expect("authored variance limit must fit i16");
    for accepted in [limit, -limit] {
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(
                accepted,
                EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
            ),
        )
        .expect("the authored variance boundary itself must remain valid");
    }

    let rejected = limit
        .checked_add(1)
        .expect("authored enterprise variance leaves one wider i16 value");
    assert_eq!(
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(
                rejected,
                EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
            ),
        )
        .expect_err("the first basis point beyond the authored variance limit must reject"),
        EnterpriseError::VarianceOutOfRange {
            basis_points: rejected,
            limit: u16::try_from(limit).expect("positive authored variance limit must fit u16"),
        }
    );
}

#[test]
fn enforcement_attention_uses_a_half_open_basis_point_roll() {
    let registry = build_registry();
    let chance = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .enforcement_attention_basis_points_per_active_case();
    assert!(chance > 0, "fixture requires a positive enforcement chance");

    for (roll, expected_hit) in [(chance - 1, true), (chance, false)] {
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        let neighborhood = match fixture.location {
            EnterpriseLocation::Neighborhood(id) => id,
            EnterpriseLocation::Business(_) => panic!("fixture must use a neighborhood location"),
        };
        let police = insert_district_police(
            &registry,
            &mut fixture,
            "Boundary Vice Bureau",
            neighborhood,
        );
        open_district_pressure_case(
            &registry,
            &mut fixture,
            police,
            "Boundary district pressure",
            neighborhood,
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, roll),
        )
        .expect("hot enterprise cycle should decide at the enforcement boundary");
        assert_eq!(
            plan.enforcement_incident.is_some(),
            expected_hit,
            "roll {roll} must use the [0, chance) basis-point convention"
        );
    }
}

#[test]
fn break_even_cycle_is_routine_and_breaks_a_chronic_loss_streak() {
    let registry = build_registry();
    // Protection economics resolve exactly to $40.40 gross and $40.40 cost here:
    // $40 base + 2 demand points at $0.20 each, versus $25 base cost plus
    // 44 police points at $0.35 each. No Management capability adds revenue.
    let mut fixture = make_test_enterprise_fixture_with_inputs(
        OrganizationKind::Criminal,
        AutonomyLevel::Delegated,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(0),
                commercial_activity: rating(0),
                illicit_demand: rating(2),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(44),
            },
        },
        None,
    );
    let enterprise = establish_protection(&registry, &mut fixture);
    let threshold = usize::from(
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .losing_cycles_before_suspension(),
    );
    assert!(
        threshold >= 3,
        "fixture requires room for a pre-break-even loss streak"
    );

    for _ in 0..(threshold - 1) {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(
                -800,
                EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
            ),
        )
        .expect("negative-variance setup cycle should decide");
        assert!(plan.economics.net_cash < Money::ZERO);
        validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("negative-variance setup cycle should validate")
            .commit(&mut fixture.state)
            .expect("negative-variance setup cycle should settle");
    }

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let break_even = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(
            0,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .expect("break-even cycle should decide");
    assert_eq!(break_even.economics.net_cash, Money::ZERO);
    assert_eq!(break_even.economics.attention, AttentionClass::Routine);
    let break_even_cycle = validate_enterprise_cycle_plan(&fixture.state, break_even)
        .expect("break-even cycle should validate")
        .commit(&mut fixture.state)
        .expect("break-even cycle should settle");
    assert!(
        fixture
            .state
            .enterprises()
            .get_cycle(break_even_cycle)
            .expect("break-even cycle must persist")
            .transaction()
            .is_none(),
        "a zero-net settlement must not manufacture a zero-value ledger transaction"
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise must persist")
            .status(),
        EnterpriseStatus::Active,
        "break-even is not a loss and must not trigger chronic-loss suspension"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let next_loss = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(
            -800,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .expect("post-break-even loss should decide");
    assert!(next_loss.economics.net_cash < Money::ZERO);
    validate_enterprise_cycle_plan(&fixture.state, next_loss)
        .expect("post-break-even loss should validate")
        .commit(&mut fixture.state)
        .expect("post-break-even loss should settle");
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise must persist")
            .status(),
        EnterpriseStatus::Active,
        "break-even must reset the consecutive-loss streak"
    );
    validate_invariants(&fixture.state);
}
