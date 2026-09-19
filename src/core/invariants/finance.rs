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
    account_ids: Vec<crate::core::id::FinancialAccountId>,
    derived_balance_cents: Vec<i64>,
    derived_account_versions: Vec<u32>,
    expected_mandate_entries: usize,
    derived_budget_totals: BTreeMap<BudgetPeriodKey, i64>,
    seen_posting_accounts: BTreeSet<crate::core::id::FinancialAccountId>,
    last_transaction_time: Option<SimTime>,
}

impl FinanceValidationScratch {
    fn account_slot(&self, account: crate::core::id::FinancialAccountId) -> Option<usize> {
        self.account_ids.binary_search(&account).ok()
    }
}

/// Finance ownership, ledger, and balance coherence in ONE pass over the append-only
/// transaction history: every per-transaction index-membership, posting, arithmetic, and
/// budget-authority check runs while the referenced-account set and derived balances are
/// accumulated, so per-tick validation walks campaign-length history once instead of once per
/// concern. Scratch balances remain dense by account count, while a sorted account-id vector maps
/// persistent ids to slots with binary search. This keeps memory proportional to real state size
/// even if a malformed current-version save carries a sparse high account id.
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
    let account_count = state.finance.accounts().count();
    let mut scratch = FinanceValidationScratch {
        account_ids: Vec::with_capacity(account_count),
        derived_balance_cents: Vec::with_capacity(account_count),
        derived_account_versions: Vec::with_capacity(account_count),
        expected_mandate_entries: 0,
        derived_budget_totals: BTreeMap::new(),
        // Reused for every transaction to avoid allocating a new ordered set in the
        // campaign-length ledger loop.
        seen_posting_accounts: BTreeSet::new(),
        last_transaction_time: None,
    };
    for account in state.finance.accounts() {
        scratch.account_ids.push(account.id());
        scratch.derived_balance_cents.push(0);
        // Every account opens at version 1. The ledger pass below advances this once for each
        // transaction that touched the account, exactly mirroring `apply_transaction`.
        scratch.derived_account_versions.push(1);
    }
    debug_assert!(scratch.account_ids.is_sorted());
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
    if let Some(previous_at) = scratch.last_transaction_time
        && transaction.occurred_at() < previous_at
    {
        return Err(StateValidationError::InvalidLedgerTransactionChronology {
            transaction: transaction.id(),
            previous_at,
            occurred_at: transaction.occurred_at(),
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
    scratch.last_transaction_time = Some(transaction.occurred_at());
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
        let Some(slot) = scratch.account_slot(posting.account) else {
            return Err(StateValidationError::MissingEntity {
                context: "ledger posting account",
                entity: EntityRef::FinancialAccount(posting.account),
            });
        };
        net_cents = net_cents.checked_add(posting.amount.cents()).ok_or(
            StateValidationError::LedgerArithmeticOverflow {
                transaction: transaction.id(),
            },
        )?;
        scratch.derived_balance_cents[slot] = scratch.derived_balance_cents[slot]
            .checked_add(posting.amount.cents())
            .ok_or(StateValidationError::FinancialBalanceMismatch)?;
        scratch.derived_account_versions[slot] = scratch.derived_account_versions[slot]
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
    let only_designated_outflow = transaction
        .postings()
        .iter()
        .filter(|posting| posting.amount < Money::ZERO)
        .all(|posting| posting.account == usage.funding_account());
    let funding_slot = scratch.account_slot(usage.funding_account());
    // `validate_postings` has already applied this transaction to the running historical balance,
    // so this is the exact post-spend balance at the transaction instant, unaffected by later
    // ledger activity. Canonical delegated spending cannot overdraw its designated funding pool.
    let funding_remains_solvent =
        funding_slot.is_some_and(|slot| scratch.derived_balance_cents[slot] >= 0);
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
        || !only_designated_outflow
        || !funding_remains_solvent
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
    for account in state.finance.accounts() {
        let slot = scratch
            .account_slot(account.id())
            .expect("ledger scratch is initialized from every persisted account");
        if scratch.derived_balance_cents[slot] != account.balance().cents() {
            return Err(StateValidationError::FinancialBalanceMismatch);
        }
        if scratch.derived_account_versions[slot] != account.version() {
            return Err(StateValidationError::InvalidFinancialAccount {
                account: account.id(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::id::IdKind;
    use crate::core::persistence::{build_save, restore_save};
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};

    #[test]
    fn ledger_scratch_memory_tracks_account_count_not_sparse_id_high_water() {
        let registry = build_registry();
        let mut state = AppState::new(0xF1A4_CE55);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Sparse Finance Fixture".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("finance fixture organization should validate");
        state
            .ids
            .set_next_raw_for_test(IdKind::FinancialAccount, 1_000_000_000);
        let account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
            },
        )
        .expect("sparse high-id account should validate through the canonical owner");

        let scratch = initialize_ledger_scratch(&state);
        assert_eq!(scratch.account_ids, vec![account]);
        assert_eq!(scratch.derived_balance_cents.len(), 1);
        assert_eq!(scratch.derived_account_versions.len(), 1);
        assert_eq!(scratch.account_slot(account), Some(0));
        validate_finance_indexes_and_ledger(&state).expect(
            "sparse persistent IDs must not make finance validation allocate by high water",
        );
        let restored = restore_save(
            &registry,
            build_save(&registry, &state)
                .expect("sparse high-id finance state should remain saveable"),
        )
        .expect("restore must validate sparse high-id finance state without high-water allocation");
        assert!(
            restored.finance().get_account(account).is_some(),
            "restore must preserve the sparse high-id account"
        );
    }
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
