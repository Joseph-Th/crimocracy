//! Production-path evidence for authorized owner withdrawals and their payroll consequence.

#[test]
fn player_daily_distribution_survives_restore_and_funds_payroll() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    add_boss(&mut fixture);
    designate_player_organization(&mut fixture.state, fixture.organization).unwrap();
    establish_business_economy(&registry, &mut fixture);
    assert!(!fixture.state.automatic_profit_sweeps());
    fixture.state.set_automatic_profit_sweeps(true);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY - 1));
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert!(restored.automatic_profit_sweeps());
    let outcome = run_tick(&registry, &mut fixture.state);
    let replay = run_tick(&registry, &mut restored);
    assert_eq!(
        outcome.autonomous_profit_sweeps,
        replay.autonomous_profit_sweeps
    );
    assert!(outcome.autonomous_profit_sweeps.is_some());
    assert!(
        outcome
            .payrolls
            .iter()
            .any(|payroll| payroll.organization() == fixture.organization
                && payroll.paid() > Money::ZERO)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .unwrap()
            .balance(),
        fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .unwrap()
            .operating_capital_floor()
    );
    assert_eq!(
        bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
    restored.set_automatic_profit_sweeps(false);
    restored.advance_clock(SimDuration::from_minutes(ONE_DAY - 1));
    let next = run_tick(&registry, &mut restored);
    assert!(next.autonomous_profit_sweeps.is_none());
    validate_invariants(&restored);
}

use super::*;
use crate::core::id::{BusinessId, IdKind, NeighborhoodId, OrganizationId};
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

const ONE_DAY: u32 = 1_440;

#[test]
fn portfolio_overhead_is_paid_from_both_manual_and_automatic_profit_distributions() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let district = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap()
        .neighborhood();
    for index in 0..3 {
        insert_business(
            &registry,
            &mut fixture.state,
            BusinessDraft {
                name: format!("Additional holding {index}"),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
                neighborhood: district,
                owner: BusinessOwner::Organization(fixture.organization),
            },
        )
        .unwrap();
    }
    let rate = portfolio_overhead_basis_points(&fixture.state, fixture.organization);
    assert_eq!(rate, 600, "four holdings incur six percent group overhead");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let gross =
        resolve_available_business_profit(&fixture.state, fixture.organization, fixture.business)
            .unwrap();
    assert!(gross > Money::ZERO);
    let fee = portfolio_overhead_charge(gross, rate);
    let retained = gross.checked_sub(fee).unwrap();
    let old_settlement = fixture
        .state
        .finance()
        .get_account(fixture.settlement)
        .unwrap()
        .balance();

    let mut manual = fixture.state.clone();
    let destination = insert_account(
        &mut manual,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let before_penny = bincode::serialize(&manual).unwrap();
    assert_eq!(
        validate_sweep_business_profits(
            &manual,
            BusinessProfitSweepDraft {
                organization: fixture.organization,
                business: fixture.business,
                amount: Money::from_cents(1),
            }
        )
        .err(),
        Some(BusinessProfitSweepError::NonPositiveNetReceipt),
        "a one-cent draw must not silently consume all of the owner's proceeds",
    );
    assert_eq!(bincode::serialize(&manual).unwrap(), before_penny);
    let withdrawal = validate_sweep_business_profits(
        &manual,
        BusinessProfitSweepDraft {
            organization: fixture.organization,
            business: fixture.business,
            amount: gross,
        },
    )
    .unwrap()
    .commit(&mut manual)
    .unwrap();
    assert_eq!(withdrawal.gross, gross);
    assert_eq!(withdrawal.administration, fee);
    assert_eq!(withdrawal.received, retained);
    assert!(
        manual
            .finance()
            .get_transaction(withdrawal.transaction)
            .is_some()
    );
    assert_eq!(
        manual.finance().get_account(destination).unwrap().balance(),
        retained
    );
    assert_eq!(
        manual
            .finance()
            .get_account(fixture.settlement)
            .unwrap()
            .balance(),
        old_settlement.checked_add(fee).unwrap()
    );

    let mut automatic = fixture.state.clone();
    apply_due_autonomous_profit_sweeps(&mut automatic)
        .unwrap()
        .unwrap();
    let accounted: i64 = automatic
        .finance()
        .accounts_for(FinancialOwner::Organization(fixture.organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.balance().cents())
        .sum();
    assert_eq!(accounted, retained.cents());
    assert_eq!(
        automatic
            .finance()
            .get_account(fixture.settlement)
            .unwrap()
            .balance(),
        old_settlement.checked_add(fee).unwrap()
    );
    assert_eq!(
        automatic
            .finance()
            .get_account(fixture.operating)
            .unwrap()
            .balance(),
        manual
            .finance()
            .get_account(fixture.operating)
            .unwrap()
            .balance()
    );
    for state in [&manual, &automatic] {
        validate_invariants(state);
        validate_state_against_registry(&registry, state).unwrap();
        let restored = restore_save(&registry, build_save(&registry, state).unwrap()).unwrap();
        assert_eq!(
            bincode::serialize(&restored).unwrap(),
            bincode::serialize(state).unwrap()
        );
    }
}

#[test]
fn finance_directors_reduce_overhead_but_cannot_preserve_stale_withdrawal_quotes() {
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};

    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let district = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap()
        .neighborhood();
    for index in 0..3 {
        insert_business(
            &registry,
            &mut fixture.state,
            BusinessDraft {
                name: format!("Commercial interest {index}"),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
                neighborhood: district,
                owner: BusinessOwner::Organization(fixture.organization),
            },
        )
        .unwrap();
    }
    add_boss(&mut fixture);
    let boss = fixture
        .state
        .world()
        .characters_in_organization(fixture.organization)
        .find(|member| member.supervisor().is_none())
        .unwrap()
        .id();
    let manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Financial director".into(),
            organization: Some(fixture.organization),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let destination = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let funding = Money::from_cents(10_000);
    validate_record_business_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: "Proven operating cash".into(),
            postings: vec![
                LedgerPosting {
                    account: fixture.settlement,
                    amount: funding.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: fixture.operating,
                    amount: funding,
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    assert_eq!(
        portfolio_overhead_basis_points(&fixture.state, fixture.organization),
        600
    );
    let draft = BusinessProfitSweepDraft {
        organization: fixture.organization,
        business: fixture.business,
        amount: funding,
    };
    let held = validate_sweep_business_profits(&fixture.state, draft.clone()).unwrap();
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager,
            scopes: BTreeSet::from([
                ResponsibilityScope::Function(ResponsibilityFunction::Finance),
                ResponsibilityScope::Neighborhood(district),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    assert_eq!(
        portfolio_overhead_basis_points(&fixture.state, fixture.organization),
        300
    );
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        held.commit(&mut fixture.state),
        Err(BusinessProfitSweepError::StalePortfolio)
    ));
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    validate_sweep_business_profits(&fixture.state, draft)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(destination)
            .unwrap()
            .balance(),
        funding
            .checked_sub(portfolio_overhead_charge(funding, 300))
            .unwrap()
    );
    validate_state_against_registry(&registry, &fixture.state).unwrap();
}

fn accounted_treasury_with_headroom(
    state: &mut AppState,
    organization: OrganizationId,
    headroom: i64,
) -> FinancialAccountId {
    let owner = FinancialOwner::Organization(organization);
    let accounted = insert_account(
        state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("treasury pocket opens");
    let clearing = insert_account(
        state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .expect("capital counterparty opens");
    let opening = Money::from_cents(i64::MAX - headroom);
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Near-capacity legitimate treasury".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: clearing,
                    amount: opening.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: accounted,
                    amount: opening,
                },
            ],
            authorization: None,
        },
    )
    .expect("near-capacity credit is balanced")
    .commit(state)
    .expect("treasury capitalizes");
    accounted
}

#[test]
fn manual_owner_distribution_reuses_headroom_and_opens_only_the_remainder() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let surplus =
        resolve_available_business_profit(&fixture.state, fixture.organization, fixture.business)
            .unwrap();
    assert!(surplus.cents() > 6);
    let first_headroom = surplus.cents() / 3;
    let second_headroom = first_headroom + 1;
    let first =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, first_headroom);
    let second =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, second_headroom);
    let next = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let draft = BusinessProfitSweepDraft {
        organization: fixture.organization,
        business: fixture.business,
        amount: surplus,
    };
    let original = bincode::serialize(&fixture.state).unwrap();
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    let mut automatic = fixture.state.clone();
    let automatic_id = apply_due_autonomous_profit_sweeps(&mut automatic)
        .unwrap()
        .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        let held = validate_sweep_business_profits(state, draft.clone()).unwrap();
        assert_eq!(bincode::serialize(state).unwrap(), original);
        let result = held.commit(state).unwrap();
        assert_eq!(result.received, surplus);
        assert_eq!(result.administration, Money::ZERO);
        assert_eq!(state.ids.next_raw(IdKind::FinancialAccount), next + 1);
        assert_eq!(
            state
                .finance()
                .get_account(first)
                .unwrap()
                .balance()
                .cents(),
            i64::MAX
        );
        assert_eq!(
            state
                .finance()
                .get_account(second)
                .unwrap()
                .balance()
                .cents(),
            i64::MAX
        );
        assert_eq!(
            state
                .finance()
                .get_account(FinancialAccountId::from_raw(next))
                .unwrap()
                .balance()
                .cents(),
            surplus.cents() - first_headroom - second_headroom
        );
        let postings = state
            .finance()
            .get_transaction(result.transaction)
            .unwrap()
            .postings();
        assert_eq!(
            postings,
            automatic
                .finance()
                .get_transaction(automatic_id)
                .unwrap()
                .postings()
        );
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn automatic_owner_distribution_fills_combined_partial_treasury_headroom_without_new_accounts() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let surplus =
        resolve_available_business_profit(&fixture.state, fixture.organization, fixture.business)
            .expect("earned business cash should be withdrawable");
    assert!(surplus.cents() > 2);
    let first_headroom = surplus.cents() / 2;
    let second_headroom = surplus.cents() - first_headroom;
    let first =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, first_headroom);
    let second =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, second_headroom);
    let next_account = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let mut restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap())
        .expect("near-capacity treasury must restore");

    for state in [&mut fixture.state, &mut restored] {
        let transaction_id = apply_due_autonomous_profit_sweeps(state)
            .expect("existing treasury capacity should accept the entire distribution")
            .expect("positive earned cash should be distributed");
        assert_eq!(state.ids.next_raw(IdKind::FinancialAccount), next_account);
        let transaction = state.finance().get_transaction(transaction_id).unwrap();
        assert!(
            transaction
                .postings()
                .iter()
                .any(|entry| entry.account == first && entry.amount.cents() == first_headroom)
        );
        assert!(
            transaction
                .postings()
                .iter()
                .any(|entry| entry.account == second && entry.amount.cents() == second_headroom)
        );
        assert_eq!(
            state
                .finance()
                .get_account(first)
                .unwrap()
                .balance()
                .cents(),
            i64::MAX
        );
        assert_eq!(
            state
                .finance()
                .get_account(second)
                .unwrap()
                .balance()
                .cents(),
            i64::MAX
        );
        assert_eq!(
            state
                .finance()
                .get_account(fixture.operating)
                .unwrap()
                .balance(),
            Money::ZERO
        );
        validate_state_against_registry(&registry, state)
            .expect("split receipts must preserve economy invariants");
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}

#[test]
fn automatic_owner_distribution_opens_only_the_unallocated_remainder() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let surplus =
        resolve_available_business_profit(&fixture.state, fixture.organization, fixture.business)
            .unwrap();
    let headroom = surplus.cents() / 2;
    assert!(headroom > 0);
    let existing =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, headroom);
    let next_account = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let transaction_id = apply_due_autonomous_profit_sweeps(&mut fixture.state)
        .expect("the remaining proceeds should open a new accounted pocket")
        .expect("the earned surplus must be distributed");
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        next_account + 1,
    );
    let transaction = fixture
        .state
        .finance()
        .get_transaction(transaction_id)
        .unwrap();
    assert!(
        transaction
            .postings()
            .iter()
            .any(|entry| { entry.account == existing && entry.amount.cents() == headroom })
    );
    let receipts: Vec<_> = transaction
        .postings()
        .iter()
        .filter(|entry| entry.amount > Money::ZERO)
        .collect();
    assert_eq!(receipts.len(), 2);
    assert!(receipts.iter().any(|entry| {
        entry.account != existing && entry.amount.cents() == surplus.cents() - headroom
    }));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(existing)
            .unwrap()
            .balance()
            .cents(),
        i64::MAX,
    );
    validate_state_against_registry(&registry, &fixture.state).unwrap();
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn automatic_owner_distribution_prefers_one_large_receiver_over_several_small_postings() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let surplus =
        resolve_available_business_profit(&fixture.state, fixture.organization, fixture.business)
            .unwrap();
    assert!(surplus.cents() > 1);
    let small = accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, 1);
    let large =
        accounted_treasury_with_headroom(&mut fixture.state, fixture.organization, surplus.cents());
    assert!(small < large);
    let next_account = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let transaction_id = apply_due_autonomous_profit_sweeps(&mut fixture.state)
        .unwrap()
        .expect("earned cash should reach one usable treasury pocket");
    let transaction = fixture
        .state
        .finance()
        .get_transaction(transaction_id)
        .unwrap();
    assert!(
        transaction
            .postings()
            .iter()
            .any(|entry| { entry.account == large && entry.amount == surplus })
    );
    assert!(
        !transaction
            .postings()
            .iter()
            .any(|entry| entry.account == small)
    );
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(small)
            .unwrap()
            .balance()
            .cents(),
        i64::MAX - 1,
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        next_account
    );
    validate_state_against_registry(&registry, &fixture.state).unwrap();
}

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
fn saturated_administration_account_does_not_block_other_owner_distributions() {
    let registry = build_registry();
    let mut fixture = make_business_economy_fixture_for_kind(OrganizationKind::Criminal);
    establish_business_economy(&registry, &mut fixture);
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap()
        .neighborhood();
    let (second_business, second_operating) = add_organized_business(
        &registry,
        &mut fixture.state,
        fixture.organization,
        neighborhood,
        "Second Grocery",
    );
    insert_business(
        &registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Additional holding".into(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood,
            owner: BusinessOwner::Organization(fixture.organization),
        },
    )
    .unwrap();
    assert_eq!(
        portfolio_overhead_basis_points(&fixture.state, fixture.organization),
        300
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(ONE_DAY));
    for business in [fixture.business, second_business] {
        let plan = decide_business_cycle(&registry, &fixture.state, business, 0).unwrap();
        validate_business_cycle_plan(&fixture.state, plan)
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap();
    }
    let first_profit = fixture
        .state
        .finance()
        .get_account(fixture.operating)
        .unwrap()
        .balance();
    let second_profit = fixture
        .state
        .finance()
        .get_account(second_operating)
        .unwrap()
        .balance();
    assert!(first_profit > Money::ZERO && second_profit > Money::ZERO);
    // Legitimate settlement accounts have signed finite balances. Fill the first
    // source's fee receiver using balanced transactions, without changing its till.
    loop {
        let balance = fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .unwrap()
            .balance();
        let credit =
            (i128::from(i64::MAX) - i128::from(balance.cents())).min(i128::from(i64::MAX)) as i64;
        if credit == 0 {
            break;
        }
        let clearing = insert_account(
            &mut fixture.state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(fixture.organization),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_record_business_transaction(
            &fixture.state,
            LedgerTransactionDraft {
                occurred_at: fixture.state.now(),
                memo: "Fill settlement account".into(),
                postings: vec![
                    LedgerPosting {
                        account: clearing,
                        amount: Money::from_cents(-credit),
                    },
                    LedgerPosting {
                        account: fixture.settlement,
                        amount: Money::from_cents(credit),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    }
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut restored] {
        let sweep = apply_due_autonomous_profit_sweeps(state).unwrap().unwrap();
        let transaction = state.finance().get_transaction(sweep).unwrap();
        assert!(
            transaction
                .postings()
                .iter()
                .all(|entry| entry.account != fixture.operating
                    && entry.account != fixture.settlement)
        );
        assert_eq!(
            state
                .finance()
                .get_account(fixture.operating)
                .unwrap()
                .balance(),
            first_profit
        );
        assert_eq!(
            state
                .finance()
                .get_account(second_operating)
                .unwrap()
                .balance(),
            Money::ZERO
        );
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
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
