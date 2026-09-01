//! Financial validation and atomic ledger commits; sibling finance state owns balances and indexes.

use crate::core::entity::EntityRef;
use crate::core::id::{
    FinancialAccountId, IdExhaustionError, IdKind, LedgerTransactionId, MandateId,
};
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    DelegationError, ensure_mandate_authority_current, resolve_mandate_authority,
};
use crate::delegation::{MandateStatus, ResolvedMandateAuthority};
use crate::economy::business_economy_system::resolve_business_current_gross;
use crate::finance::{
    AccountKind, BudgetUsageRecord, FinancialAccountDraft, FinancialAccountRecord, FinancialOwner,
    LedgerPosting, LedgerTransactionDraft, LedgerTransactionRecord, Money, build_budget_usage,
    helpers::resolve_basis_point_share,
};
use crate::registry::Registry;
use crate::world::BusinessOwner;
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
}

impl ValidatedLedgerTransaction {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, FinanceError> {
        if let Some(openings) = &self.openings {
            openings.ensure_current(state)?;
        }
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
        let opening_count = self
            .openings
            .as_ref()
            .map_or(0, ValidatedFinancialAccountOpenings::count_u32);
        state.ids.reserve_many(&[
            (IdKind::FinancialAccount, opening_count),
            (IdKind::LedgerTransaction, 1),
        ])?;
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
        Ok(id)
    }
}

pub fn validate_record_transaction(
    state: &AppState,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    validate_record_transaction_with_optional_openings(state, draft, None)
}

pub(crate) fn validate_record_transaction_with_openings(
    state: &AppState,
    openings: ValidatedFinancialAccountOpenings,
    draft: LedgerTransactionDraft,
) -> Result<ValidatedLedgerTransaction, FinanceError> {
    openings.ensure_current(state)?;
    validate_record_transaction_with_optional_openings(state, draft, Some(openings))
}

fn validate_record_transaction_with_optional_openings(
    state: &AppState,
    draft: LedgerTransactionDraft,
    openings: Option<ValidatedFinancialAccountOpenings>,
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
        if let Some(account) = state.finance.get_account(posting.account) {
            let next = account
                .balance()
                .checked_add(posting.amount)
                .ok_or(FinanceError::BalanceOverflow(posting.account))?;
            balances.insert(posting.account, next);
            expected_versions.insert(posting.account, account.version());
        } else if openings
            .as_ref()
            .and_then(|planned| planned.account(posting.account))
            .is_some()
        {
            balances.insert(posting.account, posting.amount);
        } else {
            return Err(FinanceError::MissingAccount(posting.account));
        }
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
        openings,
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
    // Served from the running per-period aggregate maintained at ledger commit, so a
    // delegated spend costs O(log n) instead of rescanning the mandate's whole history.
    let used = state
        .finance
        .budget_used_for(mandate, window.start(), window.end());
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

/// A dirty-to-accounted funds conversion routed through an owned cash-intensive front.
///
/// The canonical laundering path: street cash leaves a StreetCash account, arrives in the
/// organization's AccountedFunds minus the authored front fee, and the fee lands in the
/// front's legitimate operating account as revenue. Plausibility is enforced against the
/// front's legitimate gross potential, so volume requires larger or additional fronts.
#[derive(Clone, Debug)]
pub struct LaunderingDraft {
    pub organization: crate::core::id::OrganizationId,
    pub street_account: FinancialAccountId,
    pub business: crate::core::id::BusinessId,
    pub accounted_account: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LaunderingError {
    #[error("laundering amount must be positive")]
    NonPositiveAmount,
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("account {account} is not owned by organization {organization}")]
    AccountOwnerMismatch {
        account: FinancialAccountId,
        organization: crate::core::id::OrganizationId,
    },
    #[error("street-cash source account {0} must hold street cash")]
    InvalidStreetAccountKind(FinancialAccountId),
    #[error("destination account {0} must hold accounted funds")]
    InvalidAccountedAccountKind(FinancialAccountId),
    #[error("business {0} does not exist")]
    MissingBusiness(crate::core::id::BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(crate::core::id::BusinessId),
    #[error("business {business} changed after laundering validation")]
    StaleBusiness {
        business: crate::core::id::BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {0} lacks the cash-intensive function required to absorb illicit cash")]
    NotCashIntensive(crate::core::id::BusinessId),
    #[error("business {0} has no active operating economy to route laundered revenue through")]
    MissingBusinessEconomy(crate::core::id::BusinessId),
    #[error(
        "street-cash account {account} holds {balance_cents} cents and cannot launder {requested_cents}"
    )]
    InsufficientStreetCash {
        account: FinancialAccountId,
        balance_cents: i64,
        requested_cents: i64,
    },
    #[error(
        "amount {requested_cents} exceeds business {business}'s plausible laundering capacity {capacity_cents}"
    )]
    CapacityExceeded {
        business: crate::core::id::BusinessId,
        requested_cents: i64,
        capacity_cents: i64,
    },
    #[error("laundering arithmetic overflowed")]
    ArithmeticOverflow,
    #[error("business {business}'s operating economy changed after laundering validation")]
    StaleEconomy {
        business: crate::core::id::BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    BusinessEconomy(#[from] crate::economy::business_economy_system::BusinessEconomyError),
}

pub struct ValidatedLaundering {
    transaction: ValidatedLedgerTransaction,
    business: crate::core::id::BusinessId,
    organization: crate::core::id::OrganizationId,
    expected_business_version: u32,
    /// Pre-computed per-cycle total (`already laundered + this transfer`) validated to stay
    /// within the front's plausibility capacity. Committing writes it as a total so the
    /// ledger leg and the budget leg of one laundering cannot half-apply.
    new_cycle_total: Money,
    expected_economy_version: u32,
}

impl ValidatedLaundering {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, LaunderingError> {
        let business = state
            .world
            .get_business(self.business)
            .ok_or(LaunderingError::MissingBusiness(self.business))?;
        if business.version() != self.expected_business_version {
            return Err(LaunderingError::StaleBusiness {
                business: self.business,
                expected: self.expected_business_version,
                found: business.version(),
            });
        }
        if business.owner() != BusinessOwner::Organization(self.organization) {
            return Err(LaunderingError::ForeignBusiness(self.business));
        }
        // The capacity decision rested on the front's economy at validation time. A cycle
        // settling in between resets the plausibility budget (and a chronic-loss suspension
        // deactivates the front), so the version pin must be re-checked before anything mutates.
        let economy = state
            .economy
            .get_business_economy(self.business)
            .ok_or(LaunderingError::MissingBusinessEconomy(self.business))?;
        if economy.version() != self.expected_economy_version {
            return Err(LaunderingError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        let id = self.transaction.commit(state)?;
        // The transfer committed, so the front's plausibility budget shrinks by the same
        // volume. The version check above guarantees the budget window is unchanged, and the
        // total was validated to fit before any mutation.
        state
            .economy
            .set_laundered_this_cycle(self.business, self.new_cycle_total);
        Ok(id)
    }

    /// The front business that absorbed the transfer; callers use this for reporting.
    pub fn business(&self) -> crate::core::id::BusinessId {
        self.business
    }
}

pub fn validate_launder_funds(
    registry: &Registry,
    state: &AppState,
    draft: LaunderingDraft,
) -> Result<ValidatedLaundering, LaunderingError> {
    if draft.amount.cents() <= 0 {
        return Err(LaunderingError::NonPositiveAmount);
    }
    let street = state
        .finance
        .get_account(draft.street_account)
        .ok_or(LaunderingError::MissingAccount(draft.street_account))?;
    if street.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.street_account,
            organization: draft.organization,
        });
    }
    if street.kind() != AccountKind::StreetCash {
        return Err(LaunderingError::InvalidStreetAccountKind(
            draft.street_account,
        ));
    }
    // The source must actually hold the cash being cleaned: debiting a phantom balance would
    // mint accounted funds out of nothing, so laundering gates on available street cash the
    // same way every other spend path does.
    if street.balance().cents() < draft.amount.cents() {
        return Err(LaunderingError::InsufficientStreetCash {
            account: draft.street_account,
            balance_cents: street.balance().cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    let accounted = state
        .finance
        .get_account(draft.accounted_account)
        .ok_or(LaunderingError::MissingAccount(draft.accounted_account))?;
    if accounted.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.accounted_account,
            organization: draft.organization,
        });
    }
    if accounted.kind() != AccountKind::AccountedFunds {
        return Err(LaunderingError::InvalidAccountedAccountKind(
            draft.accounted_account,
        ));
    }
    let business_record = state
        .world
        .get_business(draft.business)
        .ok_or(LaunderingError::MissingBusiness(draft.business))?;
    if business_record.owner() != BusinessOwner::Organization(draft.organization) {
        return Err(LaunderingError::ForeignBusiness(draft.business));
    }
    if !business_record
        .functions()
        .contains(&crate::world::BusinessFunction::CashIntensive)
    {
        return Err(LaunderingError::NotCashIntensive(draft.business));
    }
    let economy = state
        .economy
        .get_business_economy(draft.business)
        .ok_or(LaunderingError::MissingBusinessEconomy(draft.business))?;
    if economy.status() != crate::economy::BusinessOperatingStatus::Active {
        return Err(LaunderingError::MissingBusinessEconomy(draft.business));
    }
    // Plausibility: the front can hide only the authored fraction of what it legitimately
    // earns per cycle, and the budget is cumulative — a front that already absorbed volume
    // this cycle has less plausible room left, so volume requires larger or additional fronts
    // rather than many small transfers. The basis is the front's current earning power, so a
    // sabotage-disrupted front cannot hide cash its degraded books cannot explain.
    let gross_potential = resolve_business_current_gross(registry, state, draft.business)?;
    let capacity_basis_points = registry.laundering().plausibility_gross_basis_points();
    let capacity = resolve_basis_point_share(gross_potential, capacity_basis_points)
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let already_laundered = economy.laundered_this_cycle();
    let remaining = capacity
        .checked_sub(already_laundered)
        .unwrap_or(Money::ZERO);
    if draft.amount > remaining {
        return Err(LaunderingError::CapacityExceeded {
            business: draft.business,
            requested_cents: draft.amount.cents(),
            capacity_cents: remaining.cents(),
        });
    }
    let new_cycle_total = already_laundered
        .checked_add(draft.amount)
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    // Fee split: the front keeps the authored cut as legitimate revenue.
    let fee = resolve_basis_point_share(draft.amount, registry.laundering().fee_basis_points())
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let credited = draft
        .amount
        .checked_sub(fee)
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let mut postings = vec![
        LedgerPosting {
            account: draft.street_account,
            amount: Money::ZERO
                .checked_sub(draft.amount)
                .ok_or(LaunderingError::ArithmeticOverflow)?,
        },
        LedgerPosting {
            account: draft.accounted_account,
            amount: credited,
        },
    ];
    if fee > Money::ZERO {
        postings.push(LedgerPosting {
            account: economy.operating_account(),
            amount: fee,
        });
    }
    let business_name = business_record.name().to_owned();
    let transaction = validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Laundered {} through {business_name}",
                crate::finance::helpers::format_money_cents(draft.amount.cents())
            ),
            postings,
            authorization: None,
        },
    )?;
    Ok(ValidatedLaundering {
        transaction,
        business: draft.business,
        organization: draft.organization,
        expected_business_version: business_record.version(),
        new_cycle_total,
        expected_economy_version: economy.version(),
    })
}

#[cfg(test)]
mod tests;
