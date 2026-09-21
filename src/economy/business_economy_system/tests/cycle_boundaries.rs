//! Exact business-cycle boundaries that distinguish losses, break-even, and owner attention.

use super::*;

#[test]
fn break_even_is_routine_resets_losses_and_a_small_loss_still_surfaces() {
    let registry = build_registry();
    // Retail resolves exactly to $120 gross and $120 cost here:
    // zero wealth/commerce leaves base gross at $120, while 80 police points add
    // $20 to the $100 base operating cost.
    let mut fixture = make_business_economy_fixture_with_profile(
        OrganizationKind::Commercial,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(0),
                commercial_activity: rating(0),
                illicit_demand: rating(0),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(80),
            },
        },
    );
    establish_business_economy(&registry, &mut fixture);
    let threshold = usize::from(
        registry
            .get_business(BusinessKind::Retail)
            .economics()
            .losing_cycles_before_suspension(),
    );
    assert!(
        threshold >= 3,
        "fixture requires room for a pre-break-even loss streak"
    );

    // One basis point of downside rounds gross down by one cent. It is deliberately far below
    // the authored notable-variance threshold, so the loss itself must be what raises attention.
    for _ in 0..(threshold - 1) {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, -1)
            .expect("small losing cycle should decide");
        assert_eq!(plan.economics.net_cash, Money::from_cents(-1));
        assert_eq!(
            plan.economics.attention,
            AttentionClass::Notable,
            "any actual loss must surface even when variance itself is routine"
        );
        validate_business_cycle_plan(&fixture.state, plan)
            .expect("small losing cycle should validate")
            .commit(&mut fixture.state)
            .expect("small losing cycle should settle");
    }

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let break_even = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect("break-even cycle should decide");
    assert_eq!(break_even.economics.net_cash, Money::ZERO);
    assert_eq!(
        break_even.economics.attention,
        AttentionClass::Routine,
        "break-even is not an owner-facing loss"
    );
    let cycle = validate_business_cycle_plan(&fixture.state, break_even)
        .expect("break-even cycle should validate")
        .commit(&mut fixture.state)
        .expect("break-even cycle should settle");
    assert!(
        fixture
            .state
            .economy()
            .get_cycle(cycle)
            .expect("break-even cycle should persist")
            .transaction()
            .is_none(),
        "zero net cash must not create a zero-value ledger transaction"
    );
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("business economy should persist")
            .status(),
        BusinessOperatingStatus::Active,
        "break-even must not complete a chronic-loss suspension streak"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let next_loss = decide_business_cycle(&registry, &fixture.state, fixture.business, -1)
        .expect("post-break-even loss should decide");
    assert_eq!(next_loss.economics.net_cash, Money::from_cents(-1));
    validate_business_cycle_plan(&fixture.state, next_loss)
        .expect("post-break-even loss should validate")
        .commit(&mut fixture.state)
        .expect("post-break-even loss should settle");
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("business economy should persist")
            .status(),
        BusinessOperatingStatus::Active,
        "break-even must reset the consecutive-loss streak"
    );
    validate_invariants(&fixture.state);
}
