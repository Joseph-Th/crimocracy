//! Financial validation and atomic ledger commits; sibling finance state owns balances and indexes.

use crate::core::entity::EntityRef;
use crate::core::id::{FinancialAccountId, IdExhaustionError, LedgerTransactionId, MandateId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    ensure_mandate_authority_current, resolve_mandate_authority, DelegationError,
};
use crate::delegation::{MandateStatus, ResolvedMandateAuthority};
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::finance::{
    build_budget_usage, AccountKind, BudgetUsageRecord, FinancialAccountDraft,
    FinancialAccountRecord, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    LedgerTransactionRecord, Money,
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
    #[error("business {0} does not exist")]
    MissingBusiness(crate::core::id::BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(crate::core::id::BusinessId),
    #[error("business {0} lacks the cash-intensive function required to absorb illicit cash")]
    NotCashIntensive(crate::core::id::BusinessId),
    #[error("business {0} has no active operating economy to route laundered revenue through")]
    MissingBusinessEconomy(crate::core::id::BusinessId),
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
    laundered_amount: Money,
    expected_economy_version: u32,
}

impl ValidatedLaundering {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, LaunderingError> {
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
        // volume. The version check above guarantees the budget window is unchanged.
        state
            .economy
            .record_laundered_volume(self.business, self.laundered_amount);
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
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.street_account,
            organization: draft.organization,
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
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.accounted_account,
            organization: draft.organization,
        });
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
    // rather than many small transfers.
    let gross_potential = resolve_business_gross_potential(registry, state, draft.business)?;
    let capacity_basis_points = registry.laundering().plausibility_gross_basis_points();
    let capacity_cents = i128::from(gross_potential.cents())
        .checked_mul(i128::from(capacity_basis_points))
        .and_then(|value| value.checked_div(10_000))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let already_laundered = economy.laundered_this_cycle().cents();
    let remaining_cents = capacity_cents.saturating_sub(already_laundered);
    if draft.amount.cents() > remaining_cents {
        return Err(LaunderingError::CapacityExceeded {
            business: draft.business,
            requested_cents: draft.amount.cents(),
            capacity_cents: remaining_cents,
        });
    }
    // Fee split: the front keeps the authored cut as legitimate revenue.
    let fee_cents = i128::from(draft.amount.cents())
        .checked_mul(i128::from(registry.laundering().fee_basis_points()))
        .and_then(|value| value.checked_div(10_000))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let credited = draft
        .amount
        .checked_sub(Money::from_cents(fee_cents))
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
    if fee_cents > 0 {
        postings.push(LedgerPosting {
            account: economy.operating_account(),
            amount: Money::from_cents(fee_cents),
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
        laundered_amount: draft.amount,
        expected_economy_version: economy.version(),
    })
}

#[cfg(test)]
mod tests;
