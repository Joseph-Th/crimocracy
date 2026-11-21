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
