//! Laundering validation, ledger settlement, and live front-capacity regressions.

use super::*;

struct LaunderingFixture {
    state: AppState,
    organization: crate::core::id::OrganizationId,
    street: FinancialAccountId,
    accounted: FinancialAccountId,
    business: crate::core::id::BusinessId,
}

fn make_laundering_fixture() -> LaunderingFixture {
    make_laundering_fixture_for_kind(OrganizationKind::Criminal)
}

fn make_laundering_fixture_for_kind(kind: OrganizationKind) -> LaunderingFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xC0FFEE);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Laundering Test Organization".to_owned(),
            kind,
        },
    )
    .expect("organization fixture should validate");
    let owner = FinancialOwner::Organization(organization);
    let street = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::StreetCash,
        },
    )
    .expect("street account should validate");
    let accounted = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("accounted account should validate");
    let neighborhood = crate::world::world_system::insert_neighborhood(
        &mut state,
        crate::world::NeighborhoodDraft {
            name: "Laundering Ward".to_owned(),
            profile: crate::world::NeighborhoodProfile {
                economy: crate::world::NeighborhoodEconomyProfile {
                    wealth: crate::world::Rating::try_new(60)
                        .expect("fixture rating should validate"),
                    commercial_activity: crate::world::Rating::try_new(70)
                        .expect("fixture rating should validate"),
                    illicit_demand: crate::world::Rating::try_new(30)
                        .expect("fixture rating should validate"),
                },
                institutions: crate::world::NeighborhoodInstitutionProfile {
                    police_presence: crate::world::Rating::try_new(40)
                        .expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    let business = crate::world::world_system::insert_business(
        &registry,
        &mut state,
        crate::world::BusinessDraft {
            name: "Clean Laundromat".to_owned(),
            kind: crate::world::BusinessKind::Retail,
            functions: BTreeSet::from([crate::world::BusinessFunction::CashIntensive]),
            neighborhood,
            owner: crate::world::BusinessOwner::Organization(organization),
        },
    )
    .expect("business fixture should validate");
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    crate::economy::business_economy_system::validate_establish_business_economy(
        &registry,
        &state,
        crate::economy::BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("business economy fixture should validate")
    .commit(&mut state)
    .expect("business economy fixture should commit");
    LaunderingFixture {
        state,
        organization,
        street,
        accounted,
        business,
    }
}

fn fund_street_cash(fixture: &mut LaunderingFixture) {
    // Seed street cash so validation reaches the economy gate instead of failing early
    // on an empty source account.
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Move take to street".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");
}

#[test]
fn exhausted_laundering_front_agrees_with_its_capacity_query() {
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan,
    };
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    fund_street_cash(&mut fixture);
    let duration = registry
        .get_business(crate::world::BusinessKind::Retail)
        .economics()
        .cycle();
    fixture.state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(duration.as_minutes()) + 1,
    ));
    let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap();
    validate_business_cycle_plan(&fixture.state, plan)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        remaining_laundering_capacity(
            &registry,
            &fixture.state,
            fixture.organization,
            fixture.business,
        ),
        None
    );
    let draft = LaunderingDraft {
        organization: fixture.organization,
        street_account: fixture.street,
        business: fixture.business,
        accounted_account: fixture.accounted,
        amount: Money::from_cents(100),
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    let expected = LaunderingError::EconomyRecurrenceExhausted(fixture.business);
    assert_eq!(
        validate_launder_funds(&registry, &fixture.state, draft.clone()).err(),
        Some(expected.clone())
    );
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    fixture.state = crate::core::persistence::restore_save(
        &registry,
        crate::core::persistence::build_save(&registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        validate_launder_funds(&registry, &fixture.state, draft).err(),
        Some(expected)
    );
    validate_invariants(&fixture.state);
}

/// A suspended front names its real state: the books exist but cannot route revenue,
/// so the rejection must say suspended rather than missing.
#[test]
fn laundering_names_suspended_economy_instead_of_missing_books() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    crate::economy::business_economy_system::validate_suspend_business_economy(
        &fixture.state,
        fixture.business,
    )
    .expect("suspension should validate")
    .commit(&mut fixture.state)
    .expect("suspension should commit");
    fund_street_cash(&mut fixture);
    let error = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(100),
        },
    )
    .err()
    .expect("suspended front must reject laundering");
    assert_eq!(error, LaunderingError::EconomySuspended(fixture.business));
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_rejects_non_criminal_organization() {
    let registry = build_registry();
    let fixture = make_laundering_fixture_for_kind(OrganizationKind::Commercial);
    assert_eq!(
        remaining_laundering_capacity(
            &registry,
            &fixture.state,
            fixture.organization,
            fixture.business,
        ),
        None
    );
    let error = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(100),
        },
    )
    .err()
    .expect("money laundering is a criminal-organization action");
    assert_eq!(
        error,
        LaunderingError::InvalidOrganizationKind(fixture.organization)
    );
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_rejects_amount_that_would_bypass_the_laundering_fee() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Seed tiny laundering amount".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-3),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(3),
                },
            ],
            authorization: None,
        },
    )
    .expect("tiny seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("tiny seed transfer should commit");

    let error = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(3),
        },
    )
    .err()
    .expect("a laundering transfer must produce a nonzero laundering fee");
    assert_eq!(error, LaunderingError::AmountTooSmallForSplit);
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("front economy should remain present")
            .laundered_this_cycle(),
        Money::ZERO
    );
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_split_rejects_rounding_that_leaves_no_accounted_credit() {
    let error = resolve_laundering_split(Money::from_cents(1), 9_999)
        .expect_err("a transfer consumed entirely by the rounded fee does not launder funds");
    assert_eq!(error, LaunderingError::AmountTooSmallForSplit);
}

#[test]
fn laundering_moves_street_cash_to_accounted_funds_minus_the_authored_fee() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();

    // Seed the street account from an off-book concealed reserve through a balanced transfer.
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Move take to street".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");

    // Capacity is the authored fraction of the front's legitimate gross potential; stay inside it.
    let gross_potential =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("gross potential should resolve");
    let capacity = crate::finance::helpers::apply_basis_point_multiplier(
        gross_potential,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("capacity should fit money");
    assert!(
        capacity.cents() < 1_000_000,
        "fixture expects a capacity below the seeded cash"
    );

    let validated = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: capacity,
        },
    )
    .expect("in-capacity laundering should validate")
    .commit(&mut fixture.state)
    .expect("in-capacity laundering should commit");

    let fee = crate::finance::helpers::apply_basis_point_multiplier(
        capacity,
        registry.laundering().fee_basis_points(),
    )
    .expect("fee should fit money");
    let transaction = fixture
        .state
        .finance()
        .get_transaction(validated)
        .expect("laundering transaction should persist");
    let street_delta: i64 = transaction
        .postings()
        .iter()
        .filter(|posting| posting.account == fixture.street)
        .map(|posting| posting.amount.cents())
        .sum();
    let accounted_delta: i64 = transaction
        .postings()
        .iter()
        .filter(|posting| posting.account == fixture.accounted)
        .map(|posting| posting.amount.cents())
        .sum();
    assert_eq!(street_delta, -capacity.cents());
    assert_eq!(accounted_delta, capacity.cents() - fee.cents());
    let economy = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("laundering front economy should persist");
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(economy.operating_account())
            .expect("front operating account should persist")
            .balance(),
        Money::ZERO,
        "laundering fees are costs, not recoverable front operating profit"
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(economy.settlement_account())
            .expect("front settlement account should persist")
            .balance(),
        fee,
        "the authored fee must settle into the non-liquid clearing account"
    );
    let sweep_error = validate_sweep_business_profits(
        &fixture.state,
        BusinessProfitSweepDraft {
            organization: fixture.organization,
            business: fixture.business,
            destination: fixture.accounted,
            amount: fee,
        },
    )
    .expect_err("laundering fees must not be recoverable through the owner-profit sweep");
    assert!(matches!(
        sweep_error,
        BusinessProfitSweepError::InsufficientTillCash { .. }
    ));
    validate_invariants(&fixture.state);
}

#[test]
fn held_laundering_rechecks_capacity_after_local_market_changes() {
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::validate_establish_business_economy;
    use crate::world::world_system::insert_business;
    use crate::world::{BusinessDraft, BusinessKind};

    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    let capacity = remaining_laundering_capacity(
        &registry,
        &fixture.state,
        fixture.organization,
        fixture.business,
    )
    .unwrap();
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund market capacity regression".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: capacity.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: capacity,
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let draft = LaunderingDraft {
        organization: fixture.organization,
        street_account: fixture.street,
        business: fixture.business,
        accounted_account: fixture.accounted,
        amount: capacity,
    };
    let held = validate_launder_funds(&registry, &fixture.state, draft.clone()).unwrap();
    let front = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap();
    let neighborhood = front.neighborhood();
    let competitor = insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "New competing storefront".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::new(),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .unwrap();
    let operating_account = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(competitor),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement_account = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(competitor),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: competitor,
            operating_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let reduced = remaining_laundering_capacity(
        &registry,
        &fixture.state,
        fixture.organization,
        fixture.business,
    )
    .unwrap();
    assert!(reduced > Money::ZERO && reduced < capacity);
    let expected = LaunderingError::CapacityExceeded {
        business: fixture.business,
        requested_cents: capacity.cents(),
        capacity_cents: reduced.cents(),
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_launder_funds(&registry, &fixture.state, draft.clone()).err(),
        Some(expected.clone())
    );
    assert_eq!(held.commit(&mut fixture.state), Err(expected));
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());

    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_launder_funds(
            &registry,
            state,
            LaunderingDraft {
                amount: reduced,
                ..draft.clone()
            },
        )
        .unwrap()
        .commit(state)
        .unwrap();
        assert_eq!(
            remaining_laundering_capacity(&registry, state, draft.organization, draft.business),
            Some(Money::ZERO)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn remaining_laundering_capacity_tracks_the_budget_and_clamps_after_sabotage() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    let before = remaining_laundering_capacity(
        &registry,
        &fixture.state,
        fixture.organization,
        fixture.business,
    )
    .expect("owned active cash-intensive front should report capacity");
    assert!(before.cents() > 0);
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund capacity probe".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-(before.cents() + 1)),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(before.cents() + 1),
                },
            ],
            authorization: None,
        },
    )
    .expect("fixture funding should validate")
    .commit(&mut fixture.state)
    .expect("fixture funding should commit");
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(before.cents() + 1),
        },
    ) {
        Ok(_) => panic!("over-capacity laundering must be rejected"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        LaunderingError::CapacityExceeded {
            business: fixture.business,
            requested_cents: before.cents() + 1,
            capacity_cents: before.cents(),
        }
    );

    // A valid full-capacity transfer precedes physical damage. Sabotage reduces the
    // currently plausible ceiling, but it must not turn the reported remaining budget
    // or rejection's capacity field negative.
    validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: before,
        },
    )
    .expect("the healthy front should absorb its full authored capacity")
    .commit(&mut fixture.state)
    .expect("the original valid transfer must settle");
    crate::economy::business_economy_system::validate_disrupt_business_economy(
        &registry,
        &fixture.state,
        fixture.business,
    )
    .expect("sabotage should be valid for the active front")
    .commit(&mut fixture.state)
    .expect("disruption must commit");
    assert_eq!(
        remaining_laundering_capacity(
            &registry,
            &fixture.state,
            fixture.organization,
            fixture.business,
        ),
        Some(Money::ZERO),
        "the damaged front has no remaining capacity rather than negative capacity"
    );
    assert_eq!(
        validate_launder_funds(
            &registry,
            &fixture.state,
            LaunderingDraft {
                organization: fixture.organization,
                street_account: fixture.street,
                business: fixture.business,
                accounted_account: fixture.accounted,
                amount: Money::from_cents(1),
            },
        )
        .err(),
        Some(LaunderingError::CapacityExceeded {
            business: fixture.business,
            requested_cents: 1,
            capacity_cents: 0,
        }),
        "validator and planning query must agree that the overcommitted front has zero room"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_token_rejects_after_front_changes_owner() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    let rival = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Rival Front Buyer".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival organization should validate");
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Fund stale laundering test".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-100),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(100),
                },
            ],
            authorization: None,
        },
    )
    .expect("fixture funding should validate")
    .commit(&mut fixture.state)
    .expect("fixture funding should commit");
    let token = validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(100),
        },
    )
    .expect("laundering should validate while the front is owned");
    let street_before = fixture
        .state
        .finance()
        .get_account(fixture.street)
        .expect("street account should persist")
        .balance();
    let accounted_before = fixture
        .state
        .finance()
        .get_account(fixture.accounted)
        .expect("accounted account should persist")
        .balance();

    validate_transfer_business_ownership(
        &fixture.state,
        fixture.business,
        BusinessOwner::Organization(rival),
    )
    .expect("front ownership transfer should validate")
    .commit(&mut fixture.state)
    .expect("front ownership transfer should commit");
    let error = token
        .commit(&mut fixture.state)
        .expect_err("a laundering token cannot survive loss of the front");
    assert!(matches!(
        error,
        LaunderingError::StaleBusiness {
            business,
            ..
        } if business == fixture.business
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.street)
            .expect("street account should persist")
            .balance(),
        street_before
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.accounted)
            .expect("accounted account should persist")
            .balance(),
        accounted_before
    );
    validate_invariants(&fixture.state);
}

#[test]
fn laundering_above_plausible_capacity_is_rejected_without_state_change() {
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    // Seed more street cash than any plausible capacity so the rejection proves the
    // plausibility ceiling, not an empty source (the empty-source case has its own test).
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "seed street cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");
    let gross_potential =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("gross potential should resolve");
    let capacity = crate::finance::helpers::apply_basis_point_multiplier(
        gross_potential,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("capacity should fit money");
    let before_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(capacity.cents() + 1),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("over-capacity laundering must be rejected"),
    };
    assert!(matches!(error, LaunderingError::CapacityExceeded { .. }));
    let after_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    assert_eq!(before_balances, after_balances);
}

#[test]
fn laundering_requires_a_street_cash_source_account() {
    let registry = build_registry();
    let fixture = make_laundering_fixture();

    // An AccountedFunds source fails the street-cash kind check.
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.accounted, // wrong kind: AccountedFunds as source
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(1_000),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("non-street-cash source must be rejected"),
    };
    assert!(matches!(
        error,
        LaunderingError::InvalidStreetAccountKind(_)
    ));
}

/// A source that does not actually hold the requested cash must be rejected: debiting a
/// phantom balance would mint accounted funds out of nothing.
#[test]
fn laundering_more_than_the_street_balance_is_rejected_without_minting_funds() {
    let registry = build_registry();
    let fixture = make_laundering_fixture();
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.street)
            .expect("street account")
            .balance(),
        Money::ZERO,
        "fixture leaves the street account empty"
    );

    let before_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(1_000),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("laundering beyond the street balance must be rejected"),
    };
    let LaunderingError::InsufficientStreetCash {
        account,
        balance_cents,
        requested_cents,
    } = error
    else {
        panic!("expected InsufficientStreetCash, found {error}");
    };
    assert_eq!(account, fixture.street);
    assert_eq!(balance_cents, 0);
    assert_eq!(requested_cents, 1_000);
    let after_balances: Vec<(crate::core::id::FinancialAccountId, i64)> = fixture
        .state
        .finance()
        .accounts()
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    assert_eq!(before_balances, after_balances);
}

/// Plausibility tracks the front's current earning power: a sabotage-disrupted front earns
/// the authored degraded fraction, so its plausible laundering capacity shrinks with it.
#[test]
fn disrupted_front_capacity_shrinks_with_degraded_books() {
    use crate::economy::business_economy_system::{
        resolve_business_current_gross, validate_disrupt_business_economy,
    };
    let registry = build_registry();
    let mut fixture = make_laundering_fixture();
    // Seed ample street cash up front so capacity rejections prove the plausibility ceiling.
    let reserve = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("reserve account should validate");
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "seed street cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve,
                    amount: Money::from_cents(-1_000_000),
                },
                LedgerPosting {
                    account: fixture.street,
                    amount: Money::from_cents(1_000_000),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transfer should validate")
    .commit(&mut fixture.state)
    .expect("seed transfer should commit");
    validate_disrupt_business_economy(&registry, &fixture.state, fixture.business)
        .expect("disruption should validate")
        .commit(&mut fixture.state)
        .expect("disruption should commit");

    let current_gross = resolve_business_current_gross(&registry, &fixture.state, fixture.business)
        .expect("disrupted gross should resolve");
    let degraded_capacity = crate::finance::helpers::apply_basis_point_multiplier(
        current_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("degraded capacity should fit money");
    // The disruption must actually bite: degraded books support strictly less volume than
    // the front's healthy gross potential would.
    let healthy_gross =
        resolve_business_gross_potential(&registry, &fixture.state, fixture.business)
            .expect("healthy gross should resolve");
    let healthy_capacity = crate::finance::helpers::apply_basis_point_multiplier(
        healthy_gross,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .expect("healthy capacity should fit money");
    assert!(
        degraded_capacity.cents() > 0 && degraded_capacity < healthy_capacity,
        "fixture expects a nontrivial degraded capacity below the healthy one"
    );

    // One cent beyond the degraded capacity is rejected even though the same volume would
    // have fit the front's healthy books.
    let error = match validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: Money::from_cents(degraded_capacity.cents() + 1),
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("over-degraded-capacity laundering must be rejected"),
    };
    assert!(matches!(error, LaunderingError::CapacityExceeded { .. }));

    // And exactly the degraded capacity still launders through the canonical path.
    validate_launder_funds(
        &registry,
        &fixture.state,
        LaunderingDraft {
            organization: fixture.organization,
            street_account: fixture.street,
            business: fixture.business,
            accounted_account: fixture.accounted,
            amount: degraded_capacity,
        },
    )
    .expect("exactly-degraded-capacity laundering should validate")
    .commit(&mut fixture.state)
    .expect("exactly-degraded-capacity laundering should commit");
    validate_invariants(&fixture.state);
}
