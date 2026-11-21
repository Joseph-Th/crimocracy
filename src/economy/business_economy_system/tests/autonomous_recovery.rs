//! Recovery competition is governed by current economics, not creation order.

use super::*;

fn suspend_new_business(
    registry: &Registry,
    fixture: &mut BusinessEconomyFixture,
    kind: BusinessKind,
) -> BusinessId {
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("base business persists")
        .neighborhood();
    let business = insert_business(
        registry,
        &mut fixture.state,
        BusinessDraft {
            name: format!("Recovery candidate {kind:?}"),
            kind,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("second business inserts");
    let operating_account = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account inserts");
    let settlement_account = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account inserts");
    validate_establish_business_economy(
        registry,
        &fixture.state,
        BusinessEconomyDraft {
            business,
            operating_account,
            settlement_account,
        },
    )
    .expect("second business economy validates")
    .commit(&mut fixture.state)
    .expect("second business economy establishes");
    validate_suspend_business_economy(&fixture.state, business)
        .expect("new business suspends")
        .commit(&mut fixture.state)
        .expect("new suspension commits");
    business
}

#[test]
fn competing_suspended_shops_reopen_by_projected_daily_profit_not_creation_order() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let stronger = suspend_new_business(&registry, &mut fixture, BusinessKind::Pharmacy);
    let unrelated = suspend_new_business(&registry, &mut fixture, BusinessKind::Laundry);
    assert!(fixture.business < stronger);
    let weak_net = forecast_normal_business_net(&registry, &fixture.state, fixture.business)
        .expect("retail forecast must be available");
    let strong_net = forecast_normal_business_net(&registry, &fixture.state, stronger)
        .expect("pharmacy forecast must be available");
    assert!(strong_net > weak_net && weak_net > Money::ZERO);
    assert!(
        forecast_normal_business_net(&registry, &fixture.state, unrelated).unwrap() > Money::ZERO,
        "non-competing laundry must also be viable",
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let mut restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap())
        .expect("suspended contenders must restore");
    for state in [&mut fixture.state, &mut restored] {
        assert_eq!(
            apply_due_autonomous_business_lifecycle(&registry, state).unwrap(),
            vec![stronger, unrelated],
        );
        assert_eq!(
            state
                .economy()
                .get_business_economy(stronger)
                .unwrap()
                .status(),
            BusinessOperatingStatus::Active,
        );
        assert_eq!(
            state
                .economy()
                .get_business_economy(unrelated)
                .unwrap()
                .status(),
            BusinessOperatingStatus::Active,
        );
        assert_eq!(
            state
                .economy()
                .get_business_economy(fixture.business)
                .unwrap()
                .status(),
            BusinessOperatingStatus::Suspended,
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}

#[test]
fn autonomous_recovery_does_not_reopen_a_business_that_can_never_settle_again() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let original = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .unwrap()
        .clone();
    let mut near_terminal = business_economy_wire(&original);
    near_terminal.version = u32::MAX - 1;
    fixture.state = restore_save(
        &registry,
        replace_serialized_economy(
            build_save(&registry, &fixture.state).unwrap(),
            &original,
            &near_terminal,
        ),
    )
    .unwrap();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    // The canonical player-directed lifecycle still allows its final transition,
    // but that transition cannot schedule a cycle and must never be an NPC choice.
    let mut explicit = fixture.state.clone();
    validate_resume_business_economy(&registry, &explicit, fixture.business)
        .unwrap()
        .commit(&mut explicit)
        .unwrap();
    assert_eq!(
        explicit
            .economy()
            .get_business_economy(fixture.business)
            .unwrap()
            .next_cycle_at(),
        None,
    );
    assert!(
        apply_due_autonomous_business_lifecycle(&registry, &mut fixture.state)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .unwrap()
            .status(),
        BusinessOperatingStatus::Suspended,
    );
    validate_invariants(&fixture.state);
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn equal_recovery_economics_keep_stable_id_tiebreaker() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    establish_business_economy(&registry, &mut fixture);
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let second = suspend_new_business(&registry, &mut fixture, BusinessKind::Retail);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(
        apply_due_autonomous_business_lifecycle(&registry, &mut fixture.state).unwrap(),
        vec![fixture.business],
    );
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(second)
            .unwrap()
            .status(),
        BusinessOperatingStatus::Suspended,
    );
    validate_invariants(&fixture.state);
}

#[test]
fn marginally_profitable_reopening_does_not_destroy_owner_portfolio_income() {
    // In this high-policing, low-commerce district, the dormant retail shop
    // earns a tiny positive margin alongside a pharmacy. Resuming retail would
    // take much more revenue away from the same owner's operating pharmacy.
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_with_profile(
        OrganizationKind::Commercial,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(20),
                commercial_activity: rating(21),
                illicit_demand: rating(30),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(100),
            },
        },
    );
    establish_business_economy(&registry, &mut fixture);
    validate_suspend_business_economy(&fixture.state, fixture.business)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let pharmacy = suspend_new_business(&registry, &mut fixture, BusinessKind::Pharmacy);
    validate_resume_business_economy(&registry, &fixture.state, pharmacy)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();

    let dormant = fixture.business;
    let retail = fixture.state.world().get_business(dormant).unwrap();
    let district = fixture
        .state
        .world()
        .get_neighborhood(retail.neighborhood())
        .unwrap();
    let (_, _, retail_net) = resolve_cycle_financials(
        dormant,
        registry.get_business(retail.kind()).economics(),
        district.profile(),
        registry.business_disruption().gross_basis_points(),
        false,
        super::super::market::resolve_market_snapshot(&fixture.state, dormant).unwrap(),
        0,
    )
    .unwrap();
    let lost = super::super::market::owner_reopening_daily_displacement(
        &registry,
        &fixture.state,
        dormant,
    )
    .unwrap();
    assert!(
        retail_net > Money::ZERO,
        "dormant retail has a positive standalone margin"
    );
    assert!(
        lost > i128::from(retail_net.cents()),
        "reopening would reduce overall owner profit"
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert!(
            apply_due_autonomous_business_lifecycle(&registry, state)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            state
                .economy()
                .get_business_economy(dormant)
                .unwrap()
                .status(),
            BusinessOperatingStatus::Suspended,
        );
        assert_eq!(
            state
                .economy()
                .get_business_economy(pharmacy)
                .unwrap()
                .status(),
            BusinessOperatingStatus::Active,
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}
