//! Production-path evidence for non-player owner withdrawals and their payroll consequence.

use super::*;
use crate::core::id::{BusinessId, NeighborhoodId, OrganizationId};
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

const ONE_DAY: u32 = 1_440;

fn add_boss(fixture: &mut BusinessEconomyFixture) {
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Owner Boss".to_owned(),
            organization: Some(fixture.organization),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("rival boss should validate");
}

fn add_organized_business(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    name: &str,
) -> (BusinessId, FinancialAccountId) {
    let business = insert_business(
        registry,
        state,
        BusinessDraft {
            name: name.to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("second business should validate");
    let operating = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("second business till should validate");
    let settlement = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .expect("second business settlement should validate");
    validate_establish_business_economy(
        registry,
        state,
        BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("second business economy should validate")
    .commit(state)
    .expect("second business economy should establish");
    (business, operating)
}

#[test]
fn npc_business_profit_reaches_payroll_without_a_prefunded_treasury() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    add_boss(&mut fixture);
    // The retained working-capital basis is set by the canonical establishment path.
    let opening_capital = Money::from_cents(10_000);
    validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Retain storefront opening capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: fixture.settlement,
                    amount: opening_capital
                        .checked_neg()
                        .expect("opening capital negates"),
                },
                LedgerPosting {
                    account: fixture.operating,
                    amount: opening_capital,
                },
            ],
            authorization: None,
        },
    )
    .expect("opening capitalization should validate")
    .commit(&mut fixture.state)
    .expect("opening capitalization should commit");
    establish_business_economy(&registry, &mut fixture);
    assert_eq!(
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(fixture.organization))
            .count(),
        0,
        "rival begins with no treasury to cover wages"
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY - 1));
    let outcome = run_tick(&registry, &mut fixture.state);

    let sweep = outcome
        .autonomous_profit_sweeps
        .expect("rival owner must distribute legitimate earnings before payroll");
    assert_eq!(outcome.business_cycles.len(), 1);
    let payroll = outcome
        .payrolls
        .iter()
        .find(|record| record.organization() == fixture.organization)
        .expect("staffed rival should receive payroll");
    assert!(
        payroll.paid() > Money::ZERO,
        "actual business earnings must fund wages"
    );
    assert!(
        payroll.paid() <= payroll.owed(),
        "payroll cannot pay more than its obligation"
    );
    assert!(
        payroll.transaction().is_some(),
        "earnings must become a real ledger-backed wage payment"
    );
    let business_record = fixture
        .state
        .economy()
        .get_business_economy(fixture.business)
        .expect("business economy must persist");
    assert_eq!(business_record.operating_capital_floor(), opening_capital);
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .expect("business till must persist")
            .balance(),
        opening_capital,
        "the sweep cannot consume retained working capital"
    );
    let transaction = fixture
        .state
        .finance()
        .get_transaction(sweep)
        .expect("withdrawal journal entry must persist");
    assert!(
        transaction.postings().iter().any(|posting| {
            posting.account == fixture.operating && posting.amount < Money::ZERO
        })
    );
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("NPC income and wages should save"),
    )
    .expect("NPC income and wage journals should survive restore");
    validate_state_against_registry(&registry, &restored)
        .expect("restored automatic owner distribution must remain registry-valid");
    validate_invariants(&restored);
}

#[test]
fn player_business_profits_remain_under_explicit_owner_control() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    add_boss(&mut fixture);
    designate_player_organization(&mut fixture.state, fixture.organization)
        .expect("criminal owner can be designated as player");
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY - 1));
    let outcome = run_tick(&registry, &mut fixture.state);

    assert_eq!(outcome.autonomous_profit_sweeps, None);
    assert_eq!(outcome.business_cycles.len(), 1);
    assert!(
        fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .expect("player till should persist")
            .balance()
            > Money::ZERO
    );
    let payroll = outcome
        .payrolls
        .iter()
        .find(|record| record.organization() == fixture.organization)
        .expect("player wages should still be assessed");
    assert_eq!(payroll.paid(), Money::ZERO);
    assert_eq!(payroll.short(), payroll.owed());
    assert!(
        fixture
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(fixture.organization))
            .all(|account| account.kind() != AccountKind::AccountedFunds),
        "automatic policy must never open player-owned withdrawal accounts"
    );
    validate_invariants(&fixture.state);
}

#[test]
fn multiple_businesses_for_one_owner_share_one_balanced_distribution() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .expect("first business should exist")
        .neighborhood();
    let (second_business, second_operating) = add_organized_business(
        &registry,
        &mut fixture.state,
        fixture.organization,
        neighborhood,
        "Second Grocery",
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    for business in [fixture.business, second_business] {
        let plan = decide_business_cycle(&registry, &fixture.state, business, 0)
            .expect("both business cycles should be due");
        validate_business_cycle_plan(&fixture.state, plan)
            .expect("due cycle should validate")
            .commit(&mut fixture.state)
            .expect("due cycle should settle");
    }
    let first_profit = fixture
        .state
        .finance()
        .get_account(fixture.operating)
        .expect("first till should persist")
        .balance();
    let second_profit = fixture
        .state
        .finance()
        .get_account(second_operating)
        .expect("second till should persist")
        .balance();
    assert!(first_profit > Money::ZERO && second_profit > Money::ZERO);

    let sweep = apply_due_autonomous_profit_sweeps(&mut fixture.state)
        .expect("withdrawal should validate")
        .expect("a single batch should combine all positive storefront profits");
    let transaction = fixture
        .state
        .finance()
        .get_transaction(sweep)
        .expect("combined owner withdrawal must persist");
    assert_eq!(transaction.postings().len(), 3);
    let deposited = first_profit
        .checked_add(second_profit)
        .expect("combined earned profit should fit");
    assert!(transaction.postings().iter().any(|posting| {
        posting.amount == deposited
            && fixture
                .state
                .finance()
                .get_account(posting.account)
                .is_some_and(|account| {
                    account.owner() == FinancialOwner::Organization(fixture.organization)
                        && account.kind() == AccountKind::AccountedFunds
                })
    }));
    for till in [fixture.operating, second_operating] {
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(till)
                .expect("till should persist")
                .balance(),
            Money::ZERO
        );
    }
    assert_eq!(
        apply_due_autonomous_profit_sweeps(&mut fixture.state)
            .expect("second same-day scan should remain valid"),
        None,
        "withdrawn earnings cannot be double-counted"
    );
    validate_state_against_registry(&registry, &fixture.state)
        .expect("multi-business owner distribution must remain registry-valid");
}

#[test]
fn no_earned_surplus_does_not_create_empty_accounts_or_transactions() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    let before = bincode::serialize(&fixture.state).expect("state should serialize");
    assert_eq!(
        apply_due_autonomous_profit_sweeps(&mut fixture.state)
            .expect("a zero-surplus front is an ordinary no-action state"),
        None
    );
    assert_eq!(
        bincode::serialize(&fixture.state).expect("state should serialize"),
        before,
        "zero-surplus daily governance cannot create empty accounting artifacts"
    );
}
