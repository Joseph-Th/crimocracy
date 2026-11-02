//! Exact enterprise-cycle boundaries that protect economic and enforcement semantics.

use super::*;

#[test]
fn saturated_racket_cash_does_not_freeze_an_independent_due_racket() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let blocked_enterprise = establish_protection(&registry, &mut fixture);

    // Canonical balanced ledger capitalization puts one racket's dedicated till at
    // its final representable cent. Its next positive cycle has no numeric headroom.
    let capital = Money::from_cents(i64::MAX - 1);
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Capitalize first racket at the finite cash rail".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.settlement,
                    amount: capital
                        .checked_neg()
                        .expect("positive capital has a negative leg"),
                },
                LedgerPosting {
                    account: fixture.cash,
                    amount: capital,
                },
            ],
            authorization: None,
        },
    )
    .expect("capitalization must be balanced")
    .commit(&mut fixture.state)
    .expect("capitalization should commit");

    // The same manager can supervise a second neighborhood under a function-scoped
    // mandate, but its settlement and cash accounts remain financially independent.
    let enterprise_scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([fixture.authority.scope, enterprise_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("expanded enterprise authority should validate")
    .commit(&mut fixture.state)
    .expect("expanded enterprise authority should commit");
    let second_neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Second Protection District".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(40),
                },
            },
        },
    )
    .expect("second neighborhood should validate");
    let second_cash = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("independent cash till should open");
    let second_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("independent settlement account should open");
    let viable_enterprise = validate_establish_enterprise(
        &registry,
        &fixture.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: fixture.organization,
            authority: MandateAuthority {
                scope: enterprise_scope,
                ..fixture.authority
            },
            location: EnterpriseLocation::Neighborhood(second_neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: second_cash,
            settlement_account: second_settlement,
        },
    )
    .expect("independent second racket should validate")
    .commit(&mut fixture.state)
    .expect("independent second racket should commit");

    let cycle = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .cycle();
    // Run one minute after the due boundary: due cycles remain due, but daily payroll
    // and rival-governance passes cannot confound the financial isolation check.
    fixture.state.advance_clock(cycle);
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let decided = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        blocked_enterprise,
        EnterpriseCycleRandomness::new(
            0,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .expect("saturated racket still has a due economic plan");
    let error = match validate_enterprise_cycle_plan(&fixture.state, decided) {
        Ok(_) => panic!("positive settlement cannot fit the saturated account"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        EnterpriseError::Finance(crate::finance::finance_system::FinanceError::BalanceOverflow(_))
    ));
    validate_state_against_registry(&registry, &fixture.state)
        .expect("balanced large capitalization must remain valid");

    let mut control = fixture.state.clone();
    validate_suspend_enterprise(&control, blocked_enterprise)
        .expect("isolated control suspension should validate")
        .commit(&mut control)
        .expect("isolated control suspension should commit");

    let actual = run_tick(&registry, &mut fixture.state);
    let expected = run_tick(&registry, &mut control);
    assert_eq!(actual.enterprise_cycles.len(), 1);
    assert_eq!(expected.enterprise_cycles.len(), 1);
    let actual_cycle = fixture
        .state
        .enterprises()
        .get_cycle(actual.enterprise_cycles[0])
        .expect("independent racket should settle");
    let expected_cycle = control
        .enterprises()
        .get_cycle(expected.enterprise_cycles[0])
        .expect("control racket should settle");
    assert_eq!(actual_cycle.enterprise(), viable_enterprise);
    assert_eq!(
        actual_cycle.variance_basis_points(),
        expected_cycle.variance_basis_points()
    );
    assert_eq!(actual_cycle.net_cash(), expected_cycle.net_cash());
    assert_eq!(
        fixture
            .state
            .enterprises()
            .cycles_for(blocked_enterprise)
            .count(),
        0,
        "a saturated till must not report phantom revenue",
    );
    assert_eq!(
        find_due_enterprises(&fixture.state),
        vec![blocked_enterprise],
        "only the saturated racket must remain due",
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("isolated enterprise settlement must remain registry-valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("isolated racket settlement should save"),
    )
    .expect("saved due-racket indexes should reconstruct");
    assert_eq!(find_due_enterprises(&restored), vec![blocked_enterprise]);
    validate_invariants(&restored);
}

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

    // One standing racket-exposure source plus one active district case both contribute to
    // the draw. The exact upper bound remains excluded.
    let exposed_chance = chance * 2;
    for (roll, expected_hit) in [(exposed_chance - 1, true), (exposed_chance, false)] {
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
