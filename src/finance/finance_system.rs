//! Financial validation and atomic ledger commits; sibling finance state owns balances and indexes.

use crate::core::entity::EntityRef;
use crate::core::id::{
    FinancialAccountId, IdExhaustionError, IdKind, LedgerTransactionId, MandateId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::delegation::delegation_system::{
    DelegationError, ensure_mandate_authority_current, resolve_mandate_authority,
};
use crate::delegation::{MandateStatus, ResolvedMandateAuthority};
use crate::finance::{
    AccountKind, BudgetUsageRecord, FinancialAccountDraft, FinancialAccountRecord, FinancialOwner,
    LedgerPosting, LedgerTransactionDraft, LedgerTransactionRecord, Money, build_budget_usage,
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
    #[error("too many financial accounts were requested in one atomic opening")]
    AccountOpeningCountOverflow,
    #[error(
        "financial account allocation changed after planning; expected next raw id {expected}, found {found}"
    )]
    StaleAccountAllocation { expected: u32, found: u32 },
    #[error("ledger transaction must contain at least two postings")]
    TooFewPostings,
    #[error("ledger transaction repeats account {0}")]
    DuplicateAccount(FinancialAccountId),
    #[error("ledger transaction contains a zero-value posting for account {0}")]
    ZeroPosting(FinancialAccountId),
    #[error(
        "business operating account {0} is attached to a legitimate economy and cannot be posted through the generic ledger path"
    )]
    ProtectedBusinessOperatingAccount(FinancialAccountId),
    #[error("ledger transaction postings do not balance to zero; net cents {net_cents}")]
    Unbalanced { net_cents: i64 },
    #[error("ledger transaction posting sum overflowed the balance accumulator")]
    PostingSumOverflow,
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error("ledger transaction would overflow account {0}")]
    BalanceOverflow(FinancialAccountId),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
    #[error(
        "ledger transaction occurrence time {occurred_at:?} must match current simulation time {now:?}"
    )]
    NonCurrentTransactionTime { occurred_at: SimTime, now: SimTime },
    #[error(
        "financial account {account} changed after validation; expected version {expected}, found {found}"
    )]
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
    #[error(
        "mandate {mandate} budget transaction cannot debit account {account}; designated funding account is {funding_account}"
    )]
    UnauthorizedBudgetOutflow {
        mandate: MandateId,
        account: FinancialAccountId,
        funding_account: FinancialAccountId,
    },
    #[error(
        "mandate {mandate} funding account {account} holds {available_cents} cents but transaction requires {requested_cents}"
    )]
    InsufficientBudgetFunds {
        mandate: MandateId,
        account: FinancialAccountId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error("budget arithmetic overflow for mandate {0}")]
    BudgetOverflow(MandateId),
    #[error(
        "mandate {mandate} budget exceeded: limit {limit_cents} cents, used {used_cents}, requested {requested_cents}"
    )]
    BudgetExceeded {
        mandate: MandateId,
        limit_cents: i64,
        used_cents: i64,
        requested_cents: i64,
    },
}

/// Opens one zero-balance account after checking its owner exists. This is the finance
/// owner's single-record primitive (like the relationship setter): multi-account openings
/// for composite transactions go through `validate_open_accounts` so a stale plan cannot
/// strand half-opened books. Adapters, setup, and tests use this same path; there is no
/// separate test-only mint.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlannedFinancialAccount {
    id: FinancialAccountId,
    draft: FinancialAccountDraft,
}

/// Read-only reservation plan for accounts that a larger atomic finance operation will open.
/// IDs are predicted from the allocator without consuming them; commit rejects stale plans
/// before any mutation if another account was opened in the meantime.
#[derive(Clone, Debug)]
pub(crate) struct ValidatedFinancialAccountOpenings {
    expected_next: u32,
    accounts: Vec<PlannedFinancialAccount>,
}

impl ValidatedFinancialAccountOpenings {
    pub(crate) fn len(&self) -> usize {
        self.accounts.len()
    }

    pub(crate) fn account_id(&self, index: usize) -> Option<FinancialAccountId> {
        self.accounts.get(index).map(|account| account.id)
    }

    pub(crate) fn account_matches(
        &self,
        id: FinancialAccountId,
        owner: FinancialOwner,
        kind: AccountKind,
    ) -> bool {
        self.accounts.iter().any(|account| {
            account.id == id && account.draft.owner == owner && account.draft.kind == kind
        })
    }

    fn account(&self, id: FinancialAccountId) -> Option<FinancialAccountDraft> {
        self.accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.draft)
    }

    fn count_u32(&self) -> u32 {
        u32::try_from(self.accounts.len())
            .expect("validated financial account opening count must fit u32")
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), FinanceError> {
        let found = state.ids.next_raw(IdKind::FinancialAccount);
        if found != self.expected_next {
            return Err(FinanceError::StaleAccountAllocation {
                expected: self.expected_next,
                found,
            });
        }
        for account in &self.accounts {
            if !crate::core::entity::is_entity_present(state, account.draft.owner.entity()) {
                return Err(FinanceError::MissingEntity(account.draft.owner.entity()));
            }
        }
        Ok(())
    }

    /// Opens all planned accounts after the caller has preflighted any additional IDs needed
    /// by its composite operation. Every fallible check occurs before the first insertion.
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<Vec<FinancialAccountId>, FinanceError> {
        self.ensure_current(state)?;
        state
            .ids
            .reserve(IdKind::FinancialAccount, self.count_u32())?;
        Ok(self.commit_after_preflight(state))
    }

    fn commit_after_preflight(self, state: &mut AppState) -> Vec<FinancialAccountId> {
        let mut opened = Vec::with_capacity(self.accounts.len());
        for account in self.accounts {
            let id = state
                .ids
                .next_financial_account()
                .expect("validated account-opening ID preflight must make allocation infallible");
            debug_assert_eq!(
                id, account.id,
                "planned financial account id must stay current"
            );
            state.finance.insert_account(FinancialAccountRecord {
                id,
                owner: account.draft.owner,
                kind: account.draft.kind,
                balance: Money::ZERO,
                version: 1,
            });
            opened.push(id);
        }
        opened
    }
}

pub(crate) fn validate_open_accounts(
    state: &AppState,
    drafts: Vec<FinancialAccountDraft>,
) -> Result<ValidatedFinancialAccountOpenings, FinanceError> {
    let count =
        u32::try_from(drafts.len()).map_err(|_| FinanceError::AccountOpeningCountOverflow)?;
    state.ids.reserve(IdKind::FinancialAccount, count)?;
    for draft in &drafts {
        if !crate::core::entity::is_entity_present(state, draft.owner.entity()) {
            return Err(FinanceError::MissingEntity(draft.owner.entity()));
        }
    }
    let expected_next = state.ids.next_raw(IdKind::FinancialAccount);
    let accounts = drafts
        .into_iter()
        .enumerate()
        .map(|(offset, draft)| {
            let offset = u32::try_from(offset)
                .expect("validated financial account opening offset must fit u32");
            PlannedFinancialAccount {
                id: FinancialAccountId::from_raw(
                    expected_next
                        .checked_add(offset)
                        .expect("account-opening reservation already proved id range"),
                ),
                draft,
            }
        })
        .collect();
    Ok(ValidatedFinancialAccountOpenings {
        expected_next,
        accounts,
    })
}

pub struct ValidatedLedgerTransaction {
    draft: LedgerTransactionDraft,
    balances: BTreeMap<FinancialAccountId, Money>,
    expected_versions: BTreeMap<FinancialAccountId, u32>,
    budget_usage: Option<BudgetUsageRecord>,
    authority_snapshot: Option<ResolvedMandateAuthority>,
    openings: Option<ValidatedFinancialAccountOpenings>,
    access: LedgerTransactionAccess,
}

impl ValidatedLedgerTransaction {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, FinanceError> {
        self.ensure_current(state)?;
        state.ids.reserve_many(&self.id_budget())?;
        Ok(self.commit_preflighted(state))
    }

    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let opening_count = self
            .openings
            .as_ref()
            .map_or(0, ValidatedFinancialAccountOpenings::count_u32);
        vec![
            (IdKind::FinancialAccount, opening_count),
            (IdKind::LedgerTransaction, 1),
        ]
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), FinanceError> {
        crate::core::time::ensure_time_current(state.now(), self.draft.occurred_at).map_err(
            |(occurred_at, now)| FinanceError::NonCurrentTransactionTime { occurred_at, now },
        )?;
        if let Some(openings) = &self.openings {
            openings.ensure_current(state)?;
        }
        for (account, expected) in &self.expected_versions {
            let record = state
                .finance
                .get_account(*account)
                .ok_or(FinanceError::MissingAccount(*account))?;
            ensure_transaction_account_access(
                state,
                *account,
                record.owner(),
                record.kind(),
                self.access,
            )?;
            if record.version() != *expected {
                return Err(FinanceError::StaleAccount {
                    account: *account,
                    expected: *expected,
                    found: record.version(),
                });
            }
            ensure_version_can_advance(record.version(), "financial account")?;
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
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) -> LedgerTransactionId {
        if let Some(openings) = self.openings {
            openings.commit_after_preflight(state);
        }
        let id = state
            .ids
            .next_ledger_transaction()
            .expect("validated ledger ID preflight must make allocation infallible");
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
        id
    }
}

/// Low-level balanced-ledger owner primitive for trusted orchestration and setup.
///
/// Domain actions with stronger semantics still use their dedicated validators. In
/// particular, once a business-owned legitimate operating account is attached to a
/// live business economy, generic ledger postings can no longer touch that till.
pub fn validate_record_transaction(
    state: &AppState,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    validate_record_transaction_with_optional_openings(
        state,
        draft,
        None,
        LedgerTransactionAccess::Generic,
    )
}

pub(crate) fn validate_record_business_transaction(
    state: &AppState,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    validate_record_transaction_with_optional_openings(
        state,
        draft,
        None,
        LedgerTransactionAccess::BusinessOperating,
    )
}

pub(crate) fn validate_record_transaction_with_openings(
    state: &AppState,
    openings: ValidatedFinancialAccountOpenings,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    openings.ensure_current(state)?;
    validate_record_transaction_with_optional_openings(
        state,
        draft,
        Some(openings),
        LedgerTransactionAccess::Generic,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LedgerTransactionAccess {
    Generic,
    BusinessOperating,
}

fn validate_record_transaction_with_optional_openings(
    state: &AppState,
    draft: LedgerTransactionDraft,
    openings: Option<ValidatedFinancialAccountOpenings>,
    access: LedgerTransactionAccess,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    if draft.memo.trim().is_empty() {
        return Err(FinanceError::EmptyMemo);
    }
    if draft.postings.len() < 2 {
        return Err(FinanceError::TooFewPostings);
    }
    crate::core::time::ensure_time_current(state.now(), draft.occurred_at).map_err(
        |(occurred_at, now)| FinanceError::NonCurrentTransactionTime { occurred_at, now },
    )?;

    let mut seen = BTreeSet::new();
    let mut net_cents: i128 = 0;
    let mut balances = BTreeMap::new();
    let mut expected_versions = BTreeMap::new();
    for posting in &draft.postings {
        validate_ledger_posting_shape(&mut seen, &mut net_cents, posting)?;
        let (balance, expected_version) =
            resolve_ledger_posting(state, openings.as_ref(), posting, access)?;
        balances.insert(posting.account, balance);
        if let Some(expected_version) = expected_version {
            expected_versions.insert(posting.account, expected_version);
        }
    }
    if net_cents != 0 {
        let diagnostic = i64::try_from(net_cents).map_err(|_| FinanceError::PostingSumOverflow)?;
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
        openings,
        access,
    })
}

fn validate_ledger_posting_shape(
    seen: &mut BTreeSet<FinancialAccountId>,
    net_cents: &mut i128,
    posting: &LedgerPosting,
) -> Result<(), FinanceError> {
    if !seen.insert(posting.account) {
        return Err(FinanceError::DuplicateAccount(posting.account));
    }
    if posting.amount == Money::ZERO {
        return Err(FinanceError::ZeroPosting(posting.account));
    }
    *net_cents = net_cents
        .checked_add(i128::from(posting.amount.cents()))
        .ok_or(FinanceError::PostingSumOverflow)?;
    Ok(())
}

fn resolve_ledger_posting(
    state: &AppState,
    openings: Option<&ValidatedFinancialAccountOpenings>,
    posting: &LedgerPosting,
    access: LedgerTransactionAccess,
) -> Result<(Money, Option<u32>), FinanceError> {
    if let Some(account) = state.finance.get_account(posting.account) {
        ensure_transaction_account_access(
            state,
            posting.account,
            account.owner(),
            account.kind(),
            access,
        )?;
        ensure_version_can_advance(account.version(), "financial account")?;
        let balance = account
            .balance()
            .checked_add(posting.amount)
            .ok_or(FinanceError::BalanceOverflow(posting.account))?;
        return Ok((balance, Some(account.version())));
    }
    let planned = openings
        .and_then(|planned| planned.account(posting.account))
        .ok_or(FinanceError::MissingAccount(posting.account))?;
    ensure_transaction_account_access(state, posting.account, planned.owner, planned.kind, access)?;
    Ok((posting.amount, None))
}

fn ensure_transaction_account_access(
    state: &AppState,
    account: FinancialAccountId,
    owner: FinancialOwner,
    kind: AccountKind,
    access: LedgerTransactionAccess,
) -> Result<(), FinanceError> {
    if access == LedgerTransactionAccess::Generic
        && kind == AccountKind::LegitimateOperating
        && let FinancialOwner::Business(business) = owner
        && state
            .economy
            .get_business_economy(business)
            .is_some_and(|economy| economy.operating_account() == account)
    {
        return Err(FinanceError::ProtectedBusinessOperatingAccount(account));
    }
    Ok(())
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
    // Served from campaign-day aggregates maintained at ledger commit. Summing the one or seven
    // buckets covered by the *current* authored window means revising a mandate's cadence,
    // funding account, or limit cannot erase spending that already occurred inside that window.
    // Lookup stays bounded instead of rescanning the mandate's campaign-length history.
    let used = state
        .finance
        .budget_used_in_window(mandate, window.start(), window.end())
        .ok_or(FinanceError::BudgetOverflow(mandate))?;
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
    // A mandate budget is spending authority over one designated pool, not a generic ledger
    // signature. Every outflow in an authorized transaction must come from that exact account;
    // otherwise a small budget posting could disguise a larger debit from unrelated funds.
    if let Some(other) = draft.postings.iter().find(|candidate| {
        candidate.amount < Money::ZERO && candidate.account != budget.funding_account
    }) {
        return Err(FinanceError::UnauthorizedBudgetOutflow {
            mandate,
            account: other.account,
            funding_account: budget.funding_account,
        });
    }
    // The authored limit caps how much the manager may spend. It does not create financing.
    // Delegated spending therefore requires the designated accounted-funds balance to cover the
    // outflow at validation time. Account version pinning below makes that solvency snapshot stale
    // if any intervening transaction changes the source before commit.
    let funding = state
        .finance
        .get_account(budget.funding_account)
        .ok_or(FinanceError::MissingAccount(budget.funding_account))?;
    let available_cents = funding.spendable_balance().cents();
    if available_cents < requested_cents {
        return Err(FinanceError::InsufficientBudgetFunds {
            mandate,
            account: budget.funding_account,
            available_cents,
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

mod laundering;
#[cfg(test)]
use laundering::resolve_laundering_split;
pub use laundering::{
    LaunderingDraft, LaunderingError, ValidatedLaundering, validate_launder_funds,
};

#[cfg(test)]
mod tests;
