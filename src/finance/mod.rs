//! Durable monetary accounts and balanced ledger records; `finance_system` owns all financial mutation.

pub mod finance_system;
pub mod helpers;

use crate::core::entity::EntityRef;
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
    Receivable,
    Payable,
    Settlement,
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
    accounts_by_owner: BTreeMap<FinancialOwner, BTreeSet<FinancialAccountId>>,
    transactions_by_mandate: BTreeMap<MandateId, BTreeSet<LedgerTransactionId>>,
}

impl FinanceState {
    pub(crate) fn new() -> Self {
        Self::default()
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
            .filter_map(|id| self.accounts.get(id))
    }

    pub fn transactions_for_mandate(
        &self,
        mandate: MandateId,
    ) -> impl Iterator<Item = &LedgerTransactionRecord> {
        self.transactions_by_mandate
            .get(&mandate)
            .into_iter()
            .flatten()
            .filter_map(|id| self.transactions.get(id))
    }

    pub(crate) fn accounts(&self) -> impl Iterator<Item = &FinancialAccountRecord> {
        self.accounts.values()
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
        }
        let previous = self.transactions.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate ledger transaction ID inserted"
        );
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for account in self.accounts.values() {
            if !self
                .accounts_by_owner
                .get(&account.owner())
                .is_some_and(|ids| ids.contains(&account.id()))
            {
                return false;
            }
        }
        for (owner, ids) in &self.accounts_by_owner {
            for id in ids {
                if !self
                    .accounts
                    .get(id)
                    .is_some_and(|account| account.owner() == *owner)
                {
                    return false;
                }
            }
        }
        for transaction in self.transactions.values() {
            if let Some(usage) = transaction.budget_usage() {
                if !self
                    .transactions_by_mandate
                    .get(&usage.mandate())
                    .is_some_and(|ids| ids.contains(&transaction.id()))
                {
                    return false;
                }
            }
        }
        for (mandate, ids) in &self.transactions_by_mandate {
            for id in ids {
                if !self.transactions.get(id).is_some_and(|transaction| {
                    transaction
                        .budget_usage()
                        .is_some_and(|usage| usage.mandate() == *mandate)
                }) {
                    return false;
                }
            }
        }
        true
    }

    pub(crate) fn has_consistent_balances(&self) -> bool {
        let mut derived = BTreeMap::new();
        for transaction in self.transactions.values() {
            for posting in transaction.postings() {
                let current = derived.entry(posting.account).or_insert(Money::ZERO);
                let Some(next) = current.checked_add(posting.amount) else {
                    return false;
                };
                *current = next;
            }
        }
        self.accounts.values().all(|account| {
            derived.get(&account.id()).copied().unwrap_or(Money::ZERO) == account.balance()
        })
    }

    #[cfg(debug_assertions)]
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: finance indexes disagree with source records"
        );
        debug_assert!(
            self.has_consistent_balances(),
            "Derived Data Consistency: financial account balances disagree with ledger postings"
        );
    }
}

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
