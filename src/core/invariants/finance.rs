//! Release-safe finance index, ledger, budget, balance, and account-version validation.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::MandateId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::{BudgetUsageRecord, FinancialAccountRecord, LedgerTransactionRecord, Money};
use std::collections::{BTreeMap, BTreeSet};

type BudgetPeriodKey = (
    MandateId,
    crate::core::time::SimTime,
    crate::core::time::SimTime,
);

struct FinanceValidationScratch {
    account_present: Vec<bool>,
    derived_balance_cents: Vec<i64>,
    derived_account_versions: Vec<u32>,
    expected_mandate_entries: usize,
    derived_budget_totals: BTreeMap<BudgetPeriodKey, i64>,
    seen_posting_accounts: BTreeSet<crate::core::id::FinancialAccountId>,
}

/// Finance ownership, ledger, and balance coherence in ONE pass over the append-only
/// transaction history: every per-transaction index-membership, posting, arithmetic, and
/// budget-authority check runs while the referenced-account set and derived balances are
/// accumulated, so per-tick validation walks campaign-length history once instead of once
/// per concern. Posting-level membership and balance accumulation use dense vectors keyed
/// by raw account id (ids are allocated monotonically), keeping the hottest loop free of
/// ordered-map traversals without weakening any check.
pub(super) fn validate_finance_indexes_and_ledger(
    state: &AppState,
) -> Result<(), StateValidationError> {
    if !state.finance.has_consistent_primary_keys() {
        return Err(finance_index_error());
    }
    validate_account_owner_index(state)?;
    let mut scratch = initialize_ledger_scratch(state);
    for transaction in state.finance.transactions() {
        validate_transaction(state, transaction, &mut scratch)?;
    }
    validate_finance_aggregates(state, &scratch)
}

fn validate_account_owner_index(state: &AppState) -> Result<(), StateValidationError> {
    let mut account_count = 0_usize;
    for account in state.finance.accounts() {
        validate_account(state, account)?;
        account_count += 1;
    }
    if state.finance.indexed_account_entries() != account_count {
        return Err(finance_index_error());
    }
    Ok(())
}

fn validate_account(
    state: &AppState,
    account: &FinancialAccountRecord,
) -> Result<(), StateValidationError> {
    if account.version() == 0 {
        return Err(StateValidationError::InvalidFinancialAccount {
            account: account.id(),
        });
    }
    let owner = account.owner().entity();
    if !is_entity_present(state, owner) {
        return Err(StateValidationError::MissingEntity {
            context: "financial account owner",
            entity: owner,
        });
    }
    if !state
        .finance
        .account_is_indexed_for_owner(account.id(), account.owner())
    {
        return Err(finance_index_error());
    }
    Ok(())
}

fn initialize_ledger_scratch(state: &AppState) -> FinanceValidationScratch {
    let highest_account = state
        .finance
        .account_id_bounds()
        .map_or(0, |(_, highest)| highest) as usize;
    let mut scratch = FinanceValidationScratch {
        account_present: vec![false; highest_account + 1],
        derived_balance_cents: vec![0_i64; highest_account + 1],
        derived_account_versions: vec![0_u32; highest_account + 1],
        expected_mandate_entries: 0,
        derived_budget_totals: BTreeMap::new(),
        // Reused for every transaction to avoid allocating a new ordered set in the
        // campaign-length ledger loop.
        seen_posting_accounts: BTreeSet::new(),
    };
    for account in state.finance.accounts() {
        let raw = account.id().raw() as usize;
        scratch.account_present[raw] = true;
        // Every account opens at version 1. The ledger pass below advances this once for each
        // transaction that touched the account, exactly mirroring `apply_transaction`.
        scratch.derived_account_versions[raw] = 1;
    }
    scratch
}

fn validate_transaction(
    state: &AppState,
    transaction: &LedgerTransactionRecord,
    scratch: &mut FinanceValidationScratch,
) -> Result<(), StateValidationError> {
    if transaction.memo().trim().is_empty() || transaction.postings().len() < 2 {
        return Err(invalid_transaction(transaction));
    }
    if transaction.occurred_at() > state.now() {
        return Err(StateValidationError::FutureTimestamp {
            context: "ledger transaction",
        });
    }
    let net_cents = validate_postings(transaction, scratch)?;
    if net_cents != 0 {
        return Err(StateValidationError::UnbalancedLedgerTransaction {
            transaction: transaction.id(),
            net_cents,
        });
    }
    if let Some(usage) = transaction.budget_usage() {
        validate_budget_usage(state, transaction, usage, scratch)?;
    }
    Ok(())
}

fn validate_postings(
    transaction: &LedgerTransactionRecord,
    scratch: &mut FinanceValidationScratch,
) -> Result<i64, StateValidationError> {
    scratch.seen_posting_accounts.clear();
    let mut net_cents = 0_i64;
    for posting in transaction.postings() {
        if posting.amount == Money::ZERO || !scratch.seen_posting_accounts.insert(posting.account) {
            return Err(invalid_transaction(transaction));
        }
        let raw = posting.account.raw() as usize;
        if raw >= scratch.account_present.len() || !scratch.account_present[raw] {
            return Err(StateValidationError::MissingEntity {
                context: "ledger posting account",
                entity: EntityRef::FinancialAccount(posting.account),
            });
        }
        net_cents = net_cents.checked_add(posting.amount.cents()).ok_or(
            StateValidationError::LedgerArithmeticOverflow {
                transaction: transaction.id(),
            },
        )?;
        scratch.derived_balance_cents[raw] = scratch.derived_balance_cents[raw]
            .checked_add(posting.amount.cents())
            .ok_or(StateValidationError::FinancialBalanceMismatch)?;
        scratch.derived_account_versions[raw] = scratch.derived_account_versions[raw]
            .checked_add(1)
            .ok_or(StateValidationError::InvalidFinancialAccount {
                account: posting.account,
            })?;
    }
    Ok(net_cents)
}

fn validate_budget_usage(
    state: &AppState,
    transaction: &LedgerTransactionRecord,
    usage: BudgetUsageRecord,
    scratch: &mut FinanceValidationScratch,
) -> Result<(), StateValidationError> {
    scratch.expected_mandate_entries += 1;
    if !state
        .finance
        .transaction_is_indexed_for_mandate(transaction.id(), usage.mandate())
    {
        return Err(finance_index_error());
    }
    let mandate = state.delegation.get_mandate(usage.mandate()).ok_or(
        StateValidationError::MissingEntity {
            context: "ledger budget mandate",
            entity: EntityRef::Mandate(usage.mandate()),
        },
    )?;
    if state.world.get_character(usage.manager()).is_none() {
        return Err(StateValidationError::MissingEntity {
            context: "ledger budget manager",
            entity: EntityRef::Character(usage.manager()),
        });
    }
    if state.finance.get_account(usage.funding_account()).is_none() {
        return Err(StateValidationError::MissingEntity {
            context: "ledger budget funding account",
            entity: EntityRef::FinancialAccount(usage.funding_account()),
        });
    }
    let expected_outflow = usage.amount().cents().checked_neg();
    let matching_posting = expected_outflow.is_some_and(|expected| {
        transaction.postings().iter().any(|posting| {
            posting.account == usage.funding_account() && posting.amount.cents() == expected
        })
    });
    let current_budget_matches = if usage.mandate_version() == mandate.version() {
        mandate.budget().is_some_and(|budget| {
            let window = budget.period.window(transaction.occurred_at());
            mandate.status() == crate::delegation::MandateStatus::Active
                && budget.funding_account == usage.funding_account()
                && usage.amount() <= budget.limit
                && usage.period_start() == window.start()
                && usage.period_end() == window.end()
                && window.contains(transaction.occurred_at())
        })
    } else {
        true
    };
    if usage.amount().cents() <= 0
        || mandate.manager() != usage.manager()
        || usage.mandate_version() == 0
        || usage.mandate_version() > mandate.version()
        || (usage.mandate_version() == mandate.version()
            && !mandate.scopes().contains(&usage.scope()))
        || !current_budget_matches
        || usage.period_start() >= usage.period_end()
        || !persisted_budget_window_contains(
            usage.period_start(),
            usage.period_end(),
            transaction.occurred_at(),
        )
        || !matching_posting
    {
        return Err(StateValidationError::InvalidBudgetUsage {
            transaction: transaction.id(),
        });
    }
    let key = (usage.mandate(), usage.period_start(), usage.period_end());
    let total = scratch.derived_budget_totals.entry(key).or_insert(0);
    *total = total.checked_add(usage.amount().cents()).ok_or(
        StateValidationError::LedgerArithmeticOverflow {
            transaction: transaction.id(),
        },
    )?;
    Ok(())
}

/// Historical mandate revisions can replace the current period definition, so old budget usage
/// must validate its persisted bounds without consulting today's mandate configuration. Normal
/// windows are half-open; an end at `u64::MAX` denotes the clamped final partial period and
/// therefore includes the last representable instant.
fn persisted_budget_window_contains(start: SimTime, end: SimTime, at: SimTime) -> bool {
    at >= start
        && if end.as_minutes() == u64::MAX {
            at <= end
        } else {
            at < end
        }
}

fn validate_finance_aggregates(
    state: &AppState,
    scratch: &FinanceValidationScratch,
) -> Result<(), StateValidationError> {
    if state.finance.indexed_mandate_entries() != scratch.expected_mandate_entries {
        return Err(finance_index_error());
    }
    let aggregate_matches = state.finance.budget_used_entries().all(|(key, total)| {
        scratch
            .derived_budget_totals
            .get(key)
            .is_some_and(|derived| *derived == total.cents())
    }) && scratch.derived_budget_totals.len()
        == state.finance.budget_used_entry_count();
    if !aggregate_matches {
        return Err(finance_index_error());
    }
    if !state
        .finance
        .balances_agree_with_derived_cents(&scratch.derived_balance_cents)
    {
        return Err(StateValidationError::FinancialBalanceMismatch);
    }
    for account in state.finance.accounts() {
        if scratch.derived_account_versions[account.id().raw() as usize] != account.version() {
            return Err(StateValidationError::InvalidFinancialAccount {
                account: account.id(),
            });
        }
    }
    Ok(())
}

fn invalid_transaction(transaction: &LedgerTransactionRecord) -> StateValidationError {
    StateValidationError::InvalidLedgerTransaction {
        transaction: transaction.id(),
    }
}

fn finance_index_error() -> StateValidationError {
    StateValidationError::IndexInconsistency {
        subsystem: "finance",
    }
}
