//! Court-imposed personal fines are collectible debts, not automatic confiscation of
//! an employer's assets. Actual payments use the finance owner. A defendant without
//! accessible cash owes the unpaid portion; subsequent daily payroll can fund it.

use super::ProsecutionError;
use crate::core::id::{LedgerTransactionId, ProsecutionCaseId};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::finance::finance_system::{
    validate_open_accounts, validate_record_transaction, validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::legal::{ProsecutionCaseRecord, ProsecutionCaseStatus};
use std::collections::{BTreeMap, BTreeSet};

fn memo(id: ProsecutionCaseId) -> String {
    format!("Court fine payment for {id}")
}

fn collectible(kind: AccountKind) -> bool {
    matches!(kind, AccountKind::StreetCash | AccountKind::AccountedFunds)
}

fn payment_amount(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    id: LedgerTransactionId,
) -> Option<Money> {
    let transaction = state.finance().get_transaction(id)?;
    if transaction.occurred_at() < case.resolved_at()?
        || transaction.memo() != memo(case.id())
        || !is_day_boundary(transaction.occurred_at())
        || transaction.budget_usage().is_some()
    {
        return None;
    }
    let mut total = Money::ZERO;
    let mut credits = 0;
    let mut debits = 0;
    for posting in transaction.postings() {
        let account = state.finance().get_account(posting.account)?;
        if posting.amount > Money::ZERO
            && account.owner() == FinancialOwner::Organization(case.prosecutor_office())
            && account.kind() == AccountKind::Settlement
        {
            credits += 1;
            total = total.checked_add(posting.amount)?;
        } else if posting.amount < Money::ZERO
            && account.owner() == FinancialOwner::Character(case.defendant())
            && collectible(account.kind())
        {
            debits += 1;
        } else {
            return None;
        }
    }
    (credits == 1 && debits > 0).then_some(total)
}

pub(super) fn outstanding(state: &AppState, case: &ProsecutionCaseRecord) -> Option<Money> {
    let judgment = case.judgment()?;
    let mut remainder = judgment.fine();
    for id in judgment.payments() {
        remainder = remainder.checked_sub(payment_amount(state, case, *id)?)?;
        if remainder < Money::ZERO {
            return None;
        }
    }
    Some(remainder)
}

/// Ledger history must reconcile to the individual court order. No payment can
/// discharge two debts and no organizational, business, or concealed balance
/// can be attributed to a defendant's fine.
fn derive_unpaid_fines(state: &AppState) -> Option<BTreeMap<ProsecutionCaseId, Money>> {
    let mut seen = BTreeSet::new();
    let mut debts = BTreeMap::new();
    for case in state.legal().prosecution_cases() {
        let Some(judgment) = case.judgment() else {
            continue;
        };
        let remaining = outstanding(state, case)?;
        if !judgment.payments().iter().all(|id| seen.insert(*id)) {
            return None;
        }
        if case.status() == ProsecutionCaseStatus::Convicted {
            if remaining > Money::ZERO {
                debts.insert(case.id(), remaining);
            }
        } else if !judgment.payments().is_empty() {
            return None;
        }
    }
    Some(debts)
}

pub(crate) fn validate_fine_history(state: &AppState) -> bool {
    derive_unpaid_fines(state).is_some_and(|expected| state.legal().unpaid_fines().eq(expected))
}

/// Restore after finance and legal indexes have been rebuilt. Malformed history
/// remains a structural validation error at the subsequent save/load boundary.
pub(crate) fn rebuild_fine_debts(state: &mut AppState) {
    let debts = derive_unpaid_fines(state).unwrap_or_default();
    state.legal.rebuild_unpaid_fines(debts);
}

pub(crate) fn apply_due_fine_collections(
    state: &mut AppState,
) -> Result<Vec<LedgerTransactionId>, ProsecutionError> {
    if !is_day_boundary(state.now()) {
        return Ok(Vec::new());
    }
    let cases: Vec<_> = state.legal().unpaid_fines().collect();
    let mut payments = Vec::new();
    // Each collection sees the balance left by previous orders. No overlapping
    // debit plan or stale account token can overdraw the same personal funds.
    for (id, mut needed) in cases {
        let case = state
            .legal()
            .get_prosecution_case(id)
            .expect("conviction exists");
        if case.version() == u32::MAX {
            continue;
        }
        debug_assert!(needed > Money::ZERO);
        let mut postings = Vec::new();
        let mut collected = Money::ZERO;
        for account in state
            .finance()
            .accounts_for(FinancialOwner::Character(case.defendant()))
        {
            if !collectible(account.kind()) {
                continue;
            }
            let payment = needed.min(account.available_payment_balance());
            if payment <= Money::ZERO {
                continue;
            }
            postings.push(LedgerPosting {
                account: account.id(),
                amount: payment.checked_neg().expect("positive fine debit"),
            });
            collected = collected
                .checked_add(payment)
                .expect("collection bounded by outstanding fine");
            needed = needed
                .checked_sub(payment)
                .expect("payment bounded by debt");
            if needed == Money::ZERO {
                break;
            }
        }
        if collected == Money::ZERO {
            continue;
        }
        let office = case.prosecutor_office();
        let destination = state
            .finance()
            .accounts_for(FinancialOwner::Organization(office))
            .find(|account| {
                account.kind() == AccountKind::Settlement
                    && account.version() < u32::MAX
                    && account.balance().checked_add(collected).is_some()
            })
            .map(|account| account.id());
        let draft = |destination| {
            postings.push(LedgerPosting {
                account: destination,
                amount: collected,
            });
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: memo(id),
                postings,
                authorization: None,
            }
        };
        let plan = if let Some(destination) = destination {
            validate_record_transaction(state, draft(destination))
        } else {
            let Ok(openings) = validate_open_accounts(
                state,
                vec![FinancialAccountDraft {
                    owner: FinancialOwner::Organization(office),
                    kind: AccountKind::Settlement,
                }],
            ) else {
                continue;
            };
            let destination = openings.account_id(0).expect("court collection account");
            validate_record_transaction_with_openings(state, openings, draft(destination))
        };
        let Ok(plan) = plan else { continue };
        let Ok(payment) = plan.commit(state) else {
            continue;
        };
        state.legal.record_fine_payment(id, payment, collected);
        payments.push(payment);
    }
    Ok(payments)
}
