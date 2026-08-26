//! Focused tests for the daily payroll pass: funding order, member wage accounts,
//! proportional shortfall consequences, and day-boundary cadence.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::core::time::{SimDuration, SimTime};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

const DAY_MINUTES: u32 = 1_440;

struct PayrollFixture {
    state: AppState,
    organization: OrganizationId,
    boss: CharacterId,
    member: CharacterId,
    treasury: FinancialAccountId,
}

fn make_test_payroll_fixture() -> PayrollFixture {
    let registry = build_registry();
    let mut state = AppState::new(7);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Payroll Test Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Test Boss".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("boss fixture should validate");
    let member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Test Soldier".to_owned(),
            organization: Some(organization),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("member fixture should validate");
    let treasury = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("treasury fixture should validate");
    PayrollFixture {
        state,
        organization,
        boss,
        member,
        treasury,
    }
}

/// Seeds a treasury balance from an external counterparty so balances stay ledger-consistent.
fn credit_account(
    state: &mut AppState,
    payer: CharacterId,
    account: FinancialAccountId,
    cents: i64,
) {
    let counterparty = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Character(payer),
            kind: AccountKind::ConcealedCash,
        },
    )
    .expect("counterparty account should validate");
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: SimTime::ZERO,
            memo: "test seed capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account,
                    amount: Money::from_cents(cents),
                },
                LedgerPosting {
                    account: counterparty,
                    amount: Money::from_cents(-cents),
                },
            ],
            authorization: None,
        },
    )
    .expect("seed transaction should validate")
    .commit(state)
    .expect("seed transaction should commit");
}

#[test]
fn payroll_is_due_only_on_nonzero_day_boundaries() {
    assert!(!is_payroll_due(SimTime::ZERO));
    assert!(!is_payroll_due(SimTime::from_minutes(1_439)));
    assert!(is_payroll_due(SimTime::from_minutes(1_440)));
    assert!(!is_payroll_due(SimTime::from_minutes(2_879)));
    assert!(is_payroll_due(SimTime::from_minutes(2_880)));
}

#[test]
fn funded_payroll_moves_wages_into_member_pockets() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let per_member = registry.upkeep().per_member_daily();
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 100_000);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut fixture.state);

    assert_eq!(outcomes.len(), 1);
    let outcome = &outcomes[0];
    assert_eq!(outcome.organization(), fixture.organization);
    assert_eq!(outcome.short(), Money::ZERO);
    assert_eq!(
        outcome.paid(),
        per_member
            .checked_mul(2)
            .expect("two-member payroll must fit money"),
        "boss and soldier are both paid members"
    );

    // The soldier's pocket was created and funded; the treasury paid both wages.
    let pocket = fixture
        .state
        .finance
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("paid member must hold a wage account");
    assert_eq!(pocket.balance(), per_member);
    let treasury_balance = fixture
        .state
        .finance
        .get_account(fixture.treasury)
        .expect("treasury must persist")
        .balance();
    assert_eq!(
        treasury_balance,
        Money::from_cents(100_000)
            .checked_sub(
                per_member
                    .checked_mul(2)
                    .expect("two-member payroll must fit money")
            )
            .expect("payroll cost must fit money")
    );
    // No shortfall means no resentment edge toward the supervisor was manufactured.
    assert!(fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .is_none());
    validate_invariants(&fixture.state);
}

#[test]
fn shortfall_distributes_available_cash_and_breeds_supervisor_resentment() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    // Even a tiny treasury is distributed across the active crew.
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 10);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut fixture.state);

    let outcome = outcomes
        .iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("a staffed criminal organization must run payroll");
    assert_eq!(outcome.paid(), Money::from_cents(10));
    assert_eq!(
        outcome.short(),
        outcome
            .owed()
            .checked_sub(Money::from_cents(10))
            .expect("partial payment cannot exceed payroll owed")
    );

    // The shorted member resents their supervisor through the canonical relationship path.
    let relationship = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions())
        .expect("shortfall must create a resentment edge toward the supervisor");
    assert_eq!(
        relationship.resentment.value(),
        registry.upkeep().shortfall_resentment(),
        "fresh resentment edge carries exactly the authored increment"
    );

    // The available ten cents are split evenly across the two active members.
    let pocket = fixture
        .state
        .finance
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("a partially paid member receives a wage account");
    assert_eq!(pocket.balance(), Money::from_cents(5));
    validate_invariants(&fixture.state);
}

#[test]
fn one_cent_short_only_shorts_one_member_in_stable_member_order() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let per_member = registry.upkeep().per_member_daily();
    let owed = per_member.checked_mul(2).expect("two wages must fit money");
    credit_account(
        &mut fixture.state,
        fixture.boss,
        fixture.treasury,
        owed.cents() - 1,
    );

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcome = apply_daily_payroll(&registry, &mut fixture.state)
        .into_iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("staffed organization must run payroll");

    assert_eq!(outcome.paid(), Money::from_cents(owed.cents() - 1));
    assert_eq!(outcome.short(), Money::from_cents(1));
    let boss_pocket = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.boss))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("boss receives the deterministic remainder cent");
    let member_pocket = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.member))
        .find(|account| account.kind() == AccountKind::StreetCash)
        .expect("member receives partial wage");
    assert_eq!(boss_pocket.balance(), per_member);
    assert_eq!(
        member_pocket.balance(),
        Money::from_cents(per_member.cents() - 1)
    );
    assert!(
        fixture
            .state
            .social()
            .get_relationship(fixture.boss, fixture.boss)
            .is_none(),
        "fully paid member does not receive a shortfall consequence"
    );
    assert_eq!(
        fixture
            .state
            .social()
            .get_relationship(fixture.member, fixture.boss)
            .expect("the one-cent-shorted member resents their supervisor")
            .dimensions()
            .resentment
            .value(),
        registry.upkeep().shortfall_resentment()
    );
    validate_invariants(&fixture.state);
}

/// A chronically insolvent organization must clamp crew resentment at the authored rail
/// instead of panicking once accumulated resentment leaves the bounded 0..=100 range.
#[test]
fn repeated_shortfalls_clamp_resentment_at_the_authored_rail() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    let increment = registry.upkeep().shortfall_resentment();

    // Enough consecutive unpaid days that fresh resentment crosses the authored rail;
    // without the saturating raise the crossing day panicked on the bounded range.
    let days_to_cross_rail =
        u32::from(crate::social::RelationshipLevel::MAX_VALUE).div_ceil(u32::from(increment));
    for _ in 0..days_to_cross_rail {
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
        apply_daily_payroll(&registry, &mut fixture.state);
    }
    let at_rail = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions().resentment.value())
        .expect("repeated shortfalls must create a resentment edge");
    assert_eq!(at_rail, crate::social::RelationshipLevel::MAX_VALUE);

    // The day that crosses the rail clamps instead of panicking, and stays clamped after.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    apply_daily_payroll(&registry, &mut fixture.state);
    let clamped = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .expect("clamped edge must persist")
        .dimensions()
        .resentment
        .value();
    assert_eq!(clamped, crate::social::RelationshipLevel::MAX_VALUE);
    validate_invariants(&fixture.state);
}

/// An organization holding no general cash accounts at all is fully short on payroll,
/// exactly like one whose accounts hold too little; only enterprise floats are exempt
/// because they are delegated working capital governed by mandate authority.
#[test]
fn organization_without_any_funding_accounts_still_incurs_full_shortfall() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let outcomes = apply_daily_payroll(&registry, &mut fixture.state);

    let outcome = outcomes
        .iter()
        .find(|outcome| outcome.organization() == fixture.organization)
        .expect("a staffed criminal organization with no cash still owes wages");
    assert_eq!(outcome.paid(), Money::ZERO);
    assert_eq!(outcome.short(), outcome.owed());
    let relationship = fixture
        .state
        .social
        .get_relationship(fixture.member, fixture.boss)
        .map(|record| record.dimensions())
        .expect("unpaid crew resents the supervisor even with no accounts");
    assert_eq!(
        relationship.resentment.value(),
        registry.upkeep().shortfall_resentment()
    );
    validate_invariants(&fixture.state);
}

#[test]
fn shortfall_reports_once_to_the_player_organization_only() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .expect("criminal organization should be eligible as the player organization");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));

    let outcomes = apply_daily_payroll(&registry, &mut fixture.state);
    assert!(!outcomes.is_empty());

    let payroll_reports: Vec<_> = fixture
        .state
        .reports()
        .reports_for(fixture.organization)
        .filter(|report| report.title() == "Payroll ran short")
        .collect();
    assert_eq!(
        payroll_reports.len(),
        1,
        "one shortfall produces exactly one notable report"
    );
    let entry = &payroll_reports[0].entries()[0];
    assert!(entry.summary.contains("went uncovered"));
    validate_invariants(&fixture.state);
}

#[test]
fn player_shortfall_report_exhaustion_rejects_before_money_or_resentment_moves() {
    let registry = build_registry();
    let mut fixture = make_test_payroll_fixture();
    credit_account(&mut fixture.state, fixture.boss, fixture.treasury, 10);
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .expect("criminal organization should be eligible as the player organization");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES));
    let funding = find_funding_accounts(&fixture.state, fixture.organization);
    let treasury_before = fixture
        .state
        .finance()
        .get_account(fixture.treasury)
        .expect("treasury should persist")
        .balance();
    let account_next_before = fixture.state.ids.next_raw(IdKind::FinancialAccount);
    let transaction_next_before = fixture.state.ids.next_raw(IdKind::LedgerTransaction);
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Report, u32::MAX);

    let error = apply_organization_payroll(
        &registry,
        &mut fixture.state,
        fixture.organization,
        &funding,
    )
    .expect_err("report exhaustion must reject before payroll mutation");
    assert!(matches!(
        error,
        PayrollError::IdExhaustion(IdExhaustionError::Exhausted { kind: "report", .. })
    ));
    assert_eq!(
        fixture
            .state
            .finance()
            .get_account(fixture.treasury)
            .expect("treasury should persist")
            .balance(),
        treasury_before
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::FinancialAccount),
        account_next_before
    );
    assert_eq!(
        fixture.state.ids.next_raw(IdKind::LedgerTransaction),
        transaction_next_before
    );
    assert!(fixture
        .state
        .social()
        .get_relationship(fixture.member, fixture.boss)
        .is_none());
    assert!(fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Character(fixture.member))
        .all(|account| account.kind() != AccountKind::StreetCash));
}
