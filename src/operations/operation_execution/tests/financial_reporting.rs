//! Operation cash proceeds and organization-level financial-report integration tests.

use super::*;
use crate::finance::finance_system::validate_record_transaction;
use crate::finance::{LedgerPosting, LedgerTransactionDraft};
use crate::reports::organization_financial_report::validate_organization_financial_report;

#[test]
fn organization_financial_report_accounts_for_held_and_deposited_operation_cash() {
    let (registry, mut state, organization, operation) = make_operation_fixture();
    run_until_operation_resolved(&registry, &mut state, operation);
    let proceeds = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .and_then(|resolution| resolution.cash_proceeds())
        .expect("completed intimidation fixture must retain held cash proceeds");
    let money = crate::finance::helpers::format_money_cents(proceeds.amount().cents());

    let held_report =
        validate_organization_financial_report(&state, organization, SimTime::ZERO, state.now())
            .expect("held-cash financial report should validate")
            .commit(&mut state)
            .expect("held-cash financial report should commit");
    let held_report = state
        .reports()
        .get_report(held_report)
        .expect("held-cash financial report should persist");
    assert!(
        held_report.entries()[0]
            .summary
            .starts_with("Liquid organization cash: $0.00.")
    );
    assert!(held_report.entries()[0].summary.contains(&format!(
        "Held operation cash at period end: 1 operation(s), amount {money}, undeposited."
    )));
    assert!(
        held_report
            .entries()
            .iter()
            .any(|entry| entry.summary.starts_with("Operation cash proceeds "))
    );

    let cash_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("street cash account should validate");
    let settlement_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    validate_deposit_operation_cash(
        &state,
        CashDispositionDraft {
            operation,
            cash_account,
            settlement_account,
        },
    )
    .expect("held cash should be depositable")
    .commit(&mut state)
    .expect("held cash deposit should commit");
    let deposit_time = state.now();

    let deposited_report =
        validate_organization_financial_report(&state, organization, SimTime::ZERO, state.now())
            .expect("deposited-cash financial report should validate")
            .commit(&mut state)
            .expect("deposited-cash financial report should commit");
    let deposited_report = state
        .reports()
        .get_report(deposited_report)
        .expect("deposited-cash financial report should persist");
    assert!(
        deposited_report.entries()[0]
            .summary
            .starts_with(&format!("Liquid organization cash: {money}."))
    );
    assert!(
        deposited_report.entries()[0].summary.contains(
            "Held operation cash at period end: 0 operation(s), amount $0.00, undeposited."
        )
    );
    assert!(deposited_report.entries()[0].summary.contains(&format!(
        "Deposited operation cash during period: 1 deposit(s), amount {money}."
    )));
    assert!(
        deposited_report
            .entries()
            .iter()
            .any(|entry| entry.summary.starts_with("Cash deposit "))
    );

    state.advance_clock(SimDuration::ONE_MINUTE);
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Later treasury movement after historical report window".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: cash_account,
                    amount: Money::from_cents(100),
                },
                LedgerPosting {
                    account: settlement_account,
                    amount: Money::from_cents(-100),
                },
            ],
            authorization: None,
        },
    )
    .expect("later treasury movement should validate")
    .commit(&mut state)
    .expect("later treasury movement should commit");
    let historical_report =
        validate_organization_financial_report(&state, organization, SimTime::ZERO, deposit_time)
            .expect("historical financial report should validate")
            .commit(&mut state)
            .expect("historical financial report should commit");
    let historical_report = state
        .reports()
        .get_report(historical_report)
        .expect("historical financial report should persist");
    assert!(
        historical_report.entries()[0]
            .summary
            .starts_with(&format!("Liquid organization cash: {money}.")),
        "historical cash must be reconstructed at period end instead of reading later balances"
    );
    validate_state(&state).expect("operation-cash financial reporting should preserve valid state");
    validate_invariants(&state);
}
