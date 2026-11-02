//! Exact business-cycle boundaries that distinguish losses, break-even, and owner attention.

use super::*;

#[test]
fn saturated_business_till_does_not_freeze_independent_due_books() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture();
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("fixture business exists")
        .neighborhood();

    // This is an ordinary, balanced opening capitalization, not a private-state edit.
    // The next positive retail settlement cannot fit one more credit into the till.
    let counterparty = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("capitalization counterparty should open");
    let capital = Money::from_cents(i64::MAX - 1);
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Capitalize a business up to the finite money rail".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: counterparty,
                    amount: capital
                        .checked_neg()
                        .expect("capital has a negative counterpart"),
                },
                LedgerPosting {
                    account: fixture.operating,
                    amount: capital,
                },
            ],
            authorization: None,
        },
    )
    .expect("balanced capitalization should validate")
    .commit(&mut fixture.state)
    .expect("balanced capitalization should commit");
    establish_business_economy(&registry, &mut fixture);

    // Give the same owner an independent storefront with separate operating books.
    let other_business = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Second Market Grocer".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .expect("independent second business should validate");
    let other_operating = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(other_business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("second operating account should open");
    let other_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(other_business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second settlement account should open");
    validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: other_business,
            operating_account: other_operating,
            settlement_account: other_settlement,
        },
    )
    .expect("second economy should establish")
    .commit(&mut fixture.state)
    .expect("second economy should commit");

    let cycle_duration = registry
        .get_business(BusinessKind::Retail)
        .economics()
        .cycle();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(cycle_duration.as_minutes() - 1));
    let blocked = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
        .expect_err("not yet due");
    assert!(matches!(blocked, BusinessEconomyError::CycleNotDue { .. }));
    validate_state_against_registry(&registry, &fixture.state)
        .expect("large but balanced capitalization must be a valid campaign");

    // Suspending just the saturated shop is a control for the random draw seen by the
    // independent shop. A blocked first shop must not consume that draw.
    let mut control = fixture.state.clone();
    validate_suspend_business_economy(&control, fixture.business)
        .expect("control may suspend the saturated shop")
        .commit(&mut control)
        .expect("control suspension should commit");
    let actual = run_tick(&registry, &mut fixture.state);
    let expected = run_tick(&registry, &mut control);
    assert_eq!(actual.business_cycles.len(), 1);
    assert_eq!(expected.business_cycles.len(), 1);
    let actual_cycle = fixture
        .state
        .economy()
        .get_cycle(actual.business_cycles[0])
        .expect("independent business must settle");
    let expected_cycle = control
        .economy()
        .get_cycle(expected.business_cycles[0])
        .expect("independent control business must settle");
    assert_eq!(actual_cycle.business(), other_business);
    assert_eq!(
        actual_cycle.variance_basis_points(),
        expected_cycle.variance_basis_points()
    );
    assert_eq!(actual_cycle.net_cash(), expected_cycle.net_cash());
    assert_eq!(
        fixture.state.economy().cycles_for(fixture.business).count(),
        0,
        "a numerically impossible settlement must remain due rather than minting money",
    );
    assert_eq!(
        find_due_businesses(&fixture.state),
        vec![fixture.business],
        "only the saturated shop should remain blocked",
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("isolated settlement must preserve all registry invariants");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("isolated settlement should save"),
    )
    .expect("derived due indexes must reconstruct after restore");
    assert_eq!(find_due_businesses(&restored), vec![fixture.business]);
    validate_invariants(&restored);
}

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
