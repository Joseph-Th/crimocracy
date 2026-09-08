//! Durable monetary accounts and balanced ledger records; `finance_system` owns all financial mutation.

pub mod finance_system;
pub mod helpers;

use crate::core::entity::EntityRef;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    BusinessId, CharacterId, FinancialAccountId, LedgerTransactionId, MandateId, OrganizationId,
};
use crate::core::time::SimTime;
use crate::delegation::{MandateAuthority, ResponsibilityScope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Self = Self(0);

    pub const fn from_cents(cents: i64) -> Self {
        Self(cents)
    }

    pub const fn cents(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    pub fn checked_mul(self, factor: i64) -> Option<Self> {
        self.0.checked_mul(factor).map(Self)
    }

    pub fn checked_neg(self) -> Option<Self> {
        self.0.checked_neg().map(Self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AccountKind {
    StreetCash,
    ConcealedCash,
    AccountedFunds,
    LegitimateOperating,
    Settlement,
}

impl AccountKind {
    /// Money the account owner can actually spend. Settlement accounts are ledger
    /// counterparties/clearing sinks and never constitute available operating liquidity.
    pub const fn is_liquid(self) -> bool {
        matches!(
            self,
            Self::StreetCash
                | Self::ConcealedCash
                | Self::AccountedFunds
                | Self::LegitimateOperating
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FinancialOwner {
    Organization(OrganizationId),
    Character(CharacterId),
    Business(BusinessId),
}

impl FinancialOwner {
    pub const fn entity(self) -> EntityRef {
        match self {
            Self::Organization(id) => EntityRef::Organization(id),
            Self::Character(id) => EntityRef::Character(id),
            Self::Business(id) => EntityRef::Business(id),
        }
    }
}

/// Accounts are opened once and never transition; freeze/close flows are out of modeled scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinancialAccountRecord {
    id: FinancialAccountId,
    owner: FinancialOwner,
    kind: AccountKind,
    balance: Money,
    version: u32,
}

impl FinancialAccountRecord {
    pub fn id(&self) -> FinancialAccountId {
        self.id
    }
    pub fn owner(&self) -> FinancialOwner {
        self.owner
    }
    pub fn kind(&self) -> AccountKind {
        self.kind
    }
    pub fn balance(&self) -> Money {
        self.balance
    }
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerPosting {
    pub account: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsageRecord {
    mandate: MandateId,
    mandate_version: u32,
    manager: CharacterId,
    scope: ResponsibilityScope,
    funding_account: FinancialAccountId,
    period_start: SimTime,
    period_end: SimTime,
    amount: Money,
}

impl BudgetUsageRecord {
    pub fn mandate(self) -> MandateId {
        self.mandate
    }
    pub fn mandate_version(self) -> u32 {
        self.mandate_version
    }
    pub fn manager(self) -> CharacterId {
        self.manager
    }
    pub fn scope(self) -> ResponsibilityScope {
        self.scope
    }
    pub fn funding_account(self) -> FinancialAccountId {
        self.funding_account
    }
    pub fn period_start(self) -> SimTime {
        self.period_start
    }
    pub fn period_end(self) -> SimTime {
        self.period_end
    }
    pub fn amount(self) -> Money {
        self.amount
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerTransactionRecord {
    id: LedgerTransactionId,
    occurred_at: SimTime,
    memo: String,
    postings: Vec<LedgerPosting>,
    budget_usage: Option<BudgetUsageRecord>,
}

impl LedgerTransactionRecord {
    pub fn id(&self) -> LedgerTransactionId {
        self.id
    }
    pub fn occurred_at(&self) -> SimTime {
        self.occurred_at
    }
    pub fn memo(&self) -> &str {
        &self.memo
    }
    pub fn postings(&self) -> &[LedgerPosting] {
        &self.postings
    }
    pub fn budget_usage(&self) -> Option<BudgetUsageRecord> {
        self.budget_usage
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FinanceState {
    accounts: BTreeMap<FinancialAccountId, FinancialAccountRecord>,
    transactions: BTreeMap<LedgerTransactionId, LedgerTransactionRecord>,
    #[serde(skip)]
    accounts_by_owner: BTreeMap<FinancialOwner, BTreeSet<FinancialAccountId>>,
    #[serde(skip)]
    transactions_by_mandate: BTreeMap<MandateId, BTreeSet<LedgerTransactionId>>,
    /// Running per-(mandate, period) charged totals, updated at ledger commit. Budget
    /// authority checks read this O(log n) instead of rescanning the mandate's full
    /// transaction history (which grows for the life of the campaign) on every spend.
    #[serde(skip)]
    budget_used_by_period: BTreeMap<(MandateId, SimTime, SimTime), Money>,
}

impl FinanceState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn rebuild_derived_indexes(&mut self) -> bool {
        self.accounts_by_owner.clear();
        self.transactions_by_mandate.clear();
        self.budget_used_by_period.clear();
        for account in self.accounts.values() {
            self.accounts_by_owner
                .entry(account.owner())
                .or_default()
                .insert(account.id());
        }
        let mut budget_cents: BTreeMap<(MandateId, SimTime, SimTime), i128> = BTreeMap::new();
        for transaction in self.transactions.values() {
            let Some(usage) = transaction.budget_usage() else {
                continue;
            };
            self.transactions_by_mandate
                .entry(usage.mandate())
                .or_default()
                .insert(transaction.id());
            let key = (usage.mandate(), usage.period_start(), usage.period_end());
            let Some(total) = budget_cents
                .get(&key)
                .copied()
                .unwrap_or(0)
                .checked_add(i128::from(usage.amount().cents()))
            else {
                return false;
            };
            budget_cents.insert(key, total);
        }
        for (key, cents) in budget_cents {
            let Ok(cents) = i64::try_from(cents) else {
                return false;
            };
            self.budget_used_by_period
                .insert(key, Money::from_cents(cents));
        }
        true
    }

    pub fn get_account(&self, id: FinancialAccountId) -> Option<&FinancialAccountRecord> {
        self.accounts.get(&id)
    }

    pub fn get_transaction(&self, id: LedgerTransactionId) -> Option<&LedgerTransactionRecord> {
        self.transactions.get(&id)
    }

    pub fn accounts_for(
        &self,
        owner: FinancialOwner,
    ) -> impl Iterator<Item = &FinancialAccountRecord> {
        self.accounts_by_owner
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|id| {
                self.accounts
                    .get(id)
                    .expect("financial owner index must reference an account")
            })
    }

    pub fn transactions_for_mandate(
        &self,
        mandate: MandateId,
    ) -> impl Iterator<Item = &LedgerTransactionRecord> {
        self.transactions_by_mandate
            .get(&mandate)
            .into_iter()
            .flatten()
            .map(|id| {
                self.transactions
                    .get(id)
                    .expect("mandate transaction index must reference a transaction")
            })
    }

    /// The running charged total for one (mandate, period) window, maintained at ledger
    /// commit time.
    pub(crate) fn budget_used_for(
        &self,
        mandate: MandateId,
        period_start: SimTime,
        period_end: SimTime,
    ) -> Money {
        self.budget_used_by_period
            .get(&(mandate, period_start, period_end))
            .copied()
            .unwrap_or(Money::ZERO)
    }

    pub(crate) fn budget_used_entries(
        &self,
    ) -> impl Iterator<Item = (&(MandateId, SimTime, SimTime), &Money)> {
        self.budget_used_by_period.iter()
    }

    pub(crate) fn budget_used_entry_count(&self) -> usize {
        self.budget_used_by_period.len()
    }

    pub(crate) fn accounts(&self) -> impl Iterator<Item = &FinancialAccountRecord> {
        self.accounts.values()
    }
    pub(crate) fn account_id_bounds(&self) -> Option<(u32, u32)> {
        self.accounts.id_bounds()
    }
    pub(crate) fn transaction_id_bounds(&self) -> Option<(u32, u32)> {
        self.transactions.id_bounds()
    }

    pub(crate) fn has_consistent_primary_keys(&self) -> bool {
        self.accounts
            .iter()
            .all(|(id, account)| *id == account.id())
            && self
                .transactions
                .iter()
                .all(|(id, transaction)| *id == transaction.id())
    }

    pub(crate) fn transactions(&self) -> impl Iterator<Item = &LedgerTransactionRecord> {
        self.transactions.values()
    }

    pub(crate) fn insert_account(&mut self, record: FinancialAccountRecord) {
        self.accounts_by_owner
            .entry(record.owner())
            .or_default()
            .insert(record.id());
        let previous = self.accounts.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate financial account ID inserted"
        );
    }

    pub(crate) fn apply_transaction(
        &mut self,
        record: LedgerTransactionRecord,
        balances: &BTreeMap<FinancialAccountId, Money>,
    ) {
        for (account, balance) in balances {
            let account_record = self
                .accounts
                .get_mut(account)
                .expect("validated account disappeared before ledger commit");
            account_record.balance = *balance;
            account_record.version = account_record
                .version
                .checked_add(1)
                .expect("financial account version counter exhausted");
        }
        if let Some(usage) = record.budget_usage() {
            self.transactions_by_mandate
                .entry(usage.mandate())
                .or_default()
                .insert(record.id());
            let key = (usage.mandate(), usage.period_start(), usage.period_end());
            let used = self
                .budget_used_by_period
                .get(&key)
                .copied()
                .unwrap_or(Money::ZERO);
            let total = used
                .checked_add(usage.amount())
                .expect("budget usage accumulator overflowed");
            self.budget_used_by_period.insert(key, total);
        }
        let previous = self.transactions.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate ledger transaction ID inserted"
        );
    }

    /// Forward index check for one account's owner membership; the fused finance audit in
    /// `core::invariants` walks accounts once and calls this per record.
    pub(crate) fn account_is_indexed_for_owner(
        &self,
        id: FinancialAccountId,
        owner: FinancialOwner,
    ) -> bool {
        self.accounts_by_owner
            .get(&owner)
            .is_some_and(|ids| ids.contains(&id))
    }

    /// Total entries across the per-owner account index; compared against the account count
    /// by the fused finance audit to prove no stale or duplicate membership.
    pub(crate) fn indexed_account_entries(&self) -> usize {
        self.accounts_by_owner.values().map(BTreeSet::len).sum()
    }

    pub(crate) fn transaction_is_indexed_for_mandate(
        &self,
        transaction: LedgerTransactionId,
        mandate: MandateId,
    ) -> bool {
        self.transactions_by_mandate
            .get(&mandate)
            .is_some_and(|ids| ids.contains(&transaction))
    }

    /// Total entries across the per-mandate transaction index; compared against the count of
    /// budget-backed transactions by the fused finance audit.
    pub(crate) fn indexed_mandate_entries(&self) -> usize {
        self.transactions_by_mandate
            .values()
            .map(BTreeSet::len)
            .sum()
    }

    /// Balance-agreement half of the fused finance audit: stored balances must equal the
    /// caller's single-pass derivation from the full ledger, indexed densely by raw account
    /// id (missing slots derive zero).
    pub(crate) fn balances_agree_with_derived_cents(&self, derived_cents: &[i64]) -> bool {
        self.accounts.values().all(|account| {
            derived_cents
                .get(account.id().raw() as usize)
                .copied()
                .unwrap_or(0)
                == account.balance().cents()
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinancialAccountDraft {
    pub owner: FinancialOwner,
    pub kind: AccountKind,
}

pub struct LedgerTransactionDraft {
    pub occurred_at: SimTime,
    pub memo: String,
    pub postings: Vec<LedgerPosting>,
    pub authorization: Option<MandateAuthority>,
}

pub(crate) fn build_budget_usage(
    authorization: MandateAuthority,
    mandate_version: u32,
    funding_account: FinancialAccountId,
    period_start: SimTime,
    period_end: SimTime,
    amount: Money,
) -> BudgetUsageRecord {
    let MandateAuthority {
        mandate,
        manager,
        scope,
    } = authorization;
    BudgetUsageRecord {
        mandate,
        mandate_version,
        manager,
        scope,
        funding_account,
        period_start,
        period_end,
        amount,
    }
}
