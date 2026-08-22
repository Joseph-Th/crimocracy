//! Financial validation and atomic ledger commits; sibling finance state owns balances and indexes.

use crate::core::entity::EntityRef;
use crate::core::id::{FinancialAccountId, IdExhaustionError, LedgerTransactionId, MandateId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    ensure_mandate_authority_current, resolve_mandate_authority, DelegationError,
};
use crate::delegation::{MandateStatus, ResolvedMandateAuthority};
use crate::finance::{
    build_budget_usage, BudgetUsageRecord, FinancialAccountDraft, FinancialAccountRecord,
    LedgerTransactionDraft, LedgerTransactionRecord, Money,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FinanceError {
    #[error("ledger transaction memo must not be empty")]
    EmptyMemo,
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("ledger transaction must contain at least two postings")]
    TooFewPostings,
    #[error("ledger transaction repeats account {0}")]
    DuplicateAccount(FinancialAccountId),
    #[error("ledger transaction contains a zero-value posting for account {0}")]
    ZeroPosting(FinancialAccountId),
    #[error("ledger transaction postings do not balance to zero; net cents {net_cents}")]
    Unbalanced { net_cents: i64 },
    #[error("ledger transaction posting sum overflowed the balance accumulator")]
    PostingSumOverflow,
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error("ledger transaction would overflow account {0}")]
    BalanceOverflow(FinancialAccountId),
    #[error("ledger transaction cannot occur in the future")]
    OccursInFuture,
    #[error("financial account {account} changed after validation; expected version {expected}, found {found}")]
    StaleAccount {
        account: FinancialAccountId,
        expected: u32,
        found: u32,
    },
    #[error("mandate {0} does not exist")]
    MissingMandate(MandateId),
    #[error("mandate {0} is not active")]
    InactiveMandate(MandateId),
    #[error("delegated authority is invalid: {0}")]
    Delegation(#[from] DelegationError),
    #[error("mandate {0} has no budget authority")]
    MissingBudget(MandateId),
    #[error("mandate {mandate} budget requires posting from funding account {account}")]
    MissingBudgetOutflow {
        mandate: MandateId,
        account: FinancialAccountId,
    },
    #[error("mandate {mandate} budget posting from account {account} must be an outflow")]
    InvalidBudgetOutflow {
        mandate: MandateId,
        account: FinancialAccountId,
    },
    #[error("budget arithmetic overflow for mandate {0}")]
    BudgetOverflow(MandateId),
    #[error("mandate {mandate} budget exceeded: limit {limit_cents} cents, used {used_cents}, requested {requested_cents}")]
    BudgetExceeded {
        mandate: MandateId,
        limit_cents: i64,
        used_cents: i64,
        requested_cents: i64,
    },
}

pub fn insert_account(
    state: &mut AppState,
    draft: FinancialAccountDraft,
) -> Result<FinancialAccountId, FinanceError> {
    if !crate::core::entity::is_entity_present(state, draft.owner.entity()) {
        return Err(FinanceError::MissingEntity(draft.owner.entity()));
    }
    let id = state.ids.next_financial_account()?;
    state.finance.insert_account(FinancialAccountRecord {
        id,
        owner: draft.owner,
        kind: draft.kind,
        balance: Money::ZERO,
        version: 1,
    });
    Ok(id)
}

pub struct ValidatedLedgerTransaction {
    draft: LedgerTransactionDraft,
    balances: BTreeMap<FinancialAccountId, Money>,
    expected_versions: BTreeMap<FinancialAccountId, u32>,
    budget_usage: Option<BudgetUsageRecord>,
    authority_snapshot: Option<ResolvedMandateAuthority>,
}

impl ValidatedLedgerTransaction {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, FinanceError> {
        for (account, expected) in &self.expected_versions {
            let record = state
                .finance
                .get_account(*account)
                .ok_or(FinanceError::MissingAccount(*account))?;
            if record.version() != *expected {
                return Err(FinanceError::StaleAccount {
                    account: *account,
                    expected: *expected,
                    found: record.version(),
                });
            }
        }
        if let Some(snapshot) = self.authority_snapshot {
            ensure_mandate_authority_current(state, snapshot)?;
        }
        if let Some(usage) = self.budget_usage {
            let summary = resolve_budget_usage(state, usage.mandate(), self.draft.occurred_at)?;
            let next_used = summary
                .used
                .checked_add(usage.amount())
                .ok_or(FinanceError::BudgetOverflow(usage.mandate()))?;
            if next_used > summary.limit {
                return Err(FinanceError::BudgetExceeded {
                    mandate: usage.mandate(),
                    limit_cents: summary.limit.cents(),
                    used_cents: summary.used.cents(),
                    requested_cents: usage.amount().cents(),
                });
            }
        }
        let id = state.ids.next_ledger_transaction()?;
        let LedgerTransactionDraft {
            occurred_at,
            memo,
            postings,
            authorization: _,
        } = self.draft;
        state.finance.apply_transaction(
            LedgerTransactionRecord {
                id,
                occurred_at,
                memo,
                postings,
                budget_usage: self.budget_usage,
            },
            &self.balances,
        );
        Ok(id)
    }
}

pub fn validate_record_transaction(
    state: &AppState,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    if draft.memo.trim().is_empty() {
        return Err(FinanceError::EmptyMemo);
    }
    if draft.postings.len() < 2 {
        return Err(FinanceError::TooFewPostings);
    }
    if draft.occurred_at > state.now() {
        return Err(FinanceError::OccursInFuture);
    }

    let mut seen = BTreeSet::new();
    let mut net_cents: i128 = 0;
    let mut balances = BTreeMap::new();
    let mut expected_versions = BTreeMap::new();
    for posting in &draft.postings {
        if !seen.insert(posting.account) {
            return Err(FinanceError::DuplicateAccount(posting.account));
        }
        if posting.amount == Money::ZERO {
            return Err(FinanceError::ZeroPosting(posting.account));
        }
        net_cents = net_cents
            .checked_add(i128::from(posting.amount.cents()))
            .ok_or(FinanceError::PostingSumOverflow)?;
        if net_cents > i128::from(i64::MAX) || net_cents < i128::from(i64::MIN) {
            return Err(FinanceError::PostingSumOverflow);
        }
        let account = state
            .finance
            .get_account(posting.account)
            .ok_or(FinanceError::MissingAccount(posting.account))?;
        let next = account
            .balance()
            .checked_add(posting.amount)
            .ok_or(FinanceError::BalanceOverflow(posting.account))?;
        balances.insert(posting.account, next);
        expected_versions.insert(posting.account, account.version());
    }
    if net_cents != 0 {
        let diagnostic =
            i64::try_from(net_cents).unwrap_or(if net_cents > 0 { i64::MAX } else { i64::MIN });
        return Err(FinanceError::Unbalanced {
            net_cents: diagnostic,
        });
    }
    let budget_validation = resolve_transaction_budget(state, &draft)?;
    Ok(ValidatedLedgerTransaction {
        draft,
        balances,
        expected_versions,
        budget_usage: budget_validation.usage,
        authority_snapshot: budget_validation.authority_snapshot,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetUsageSummary {
    pub mandate: MandateId,
    pub period_start: crate::core::time::SimTime,
    pub period_end: crate::core::time::SimTime,
    pub limit: Money,
    pub used: Money,
    pub remaining: Money,
}

pub fn resolve_budget_usage(
    state: &AppState,
    mandate: MandateId,
    at: crate::core::time::SimTime,
) -> Result<BudgetUsageSummary, FinanceError> {
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(FinanceError::MissingMandate(mandate))?;
    if record.status() != MandateStatus::Active {
        return Err(FinanceError::InactiveMandate(mandate));
    }
    let budget = record
        .budget()
        .ok_or(FinanceError::MissingBudget(mandate))?;
    let window = budget.period.window(at);
    let mut used = Money::ZERO;
    for transaction in state.finance.transactions_for_mandate(mandate) {
        if let Some(usage) = transaction.budget_usage() {
            if usage.period_start() == window.start() && usage.period_end() == window.end() {
                used = used
                    .checked_add(usage.amount())
                    .ok_or(FinanceError::BudgetOverflow(mandate))?;
            }
        }
    }
    let remaining = budget
        .limit
        .checked_sub(used)
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    Ok(BudgetUsageSummary {
        mandate,
        period_start: window.start(),
        period_end: window.end(),
        limit: budget.limit,
        used,
        remaining,
    })
}

struct ResolvedBudgetValidation {
    usage: Option<BudgetUsageRecord>,
    authority_snapshot: Option<ResolvedMandateAuthority>,
}

fn resolve_transaction_budget(
    state: &AppState,
    draft: &LedgerTransactionDraft,
) -> Result<ResolvedBudgetValidation, FinanceError> {
    let Some(authorization) = draft.authorization else {
        return Ok(ResolvedBudgetValidation {
            usage: None,
            authority_snapshot: None,
        });
    };
    let mandate = authorization.mandate;
    let authority_snapshot = resolve_mandate_authority(state, authorization)?;
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(FinanceError::MissingMandate(mandate))?;
    let budget = record
        .budget()
        .ok_or(FinanceError::MissingBudget(mandate))?;
    let posting = draft
        .postings
        .iter()
        .find(|posting| posting.account == budget.funding_account)
        .ok_or(FinanceError::MissingBudgetOutflow {
            mandate,
            account: budget.funding_account,
        })?;
    if posting.amount.cents() >= 0 {
        return Err(FinanceError::InvalidBudgetOutflow {
            mandate,
            account: budget.funding_account,
        });
    }
    let requested_cents = posting
        .amount
        .cents()
        .checked_neg()
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    let requested = Money::from_cents(requested_cents);
    let summary = resolve_budget_usage(state, mandate, draft.occurred_at)?;
    let next_used = summary
        .used
        .checked_add(requested)
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
    if next_used > summary.limit {
        return Err(FinanceError::BudgetExceeded {
            mandate,
            limit_cents: summary.limit.cents(),
            used_cents: summary.used.cents(),
            requested_cents,
        });
    }
    Ok(ResolvedBudgetValidation {
        usage: Some(build_budget_usage(
            authorization,
            authority_snapshot.mandate_version(),
            budget.funding_account,
            summary.period_start,
            summary.period_end,
            requested,
        )),
        authority_snapshot: Some(authority_snapshot),
    })
}

#[cfg(test)]
mod tests;
