//! Owner withdrawals from legitimate business tills into accounted funds.

use super::{portfolio_overhead_basis_points, portfolio_overhead_charge};
use crate::core::id::{
    BusinessId, FinancialAccountId, IdKind, LedgerTransactionId, OrganizationId,
};
use crate::core::state::AppState;
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts,
    validate_record_business_transaction, validate_record_business_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money, helpers::format_money_cents,
};
use std::collections::BTreeMap;
use thiserror::Error;

/// Deterministic receiving allocation for both criminal treasuries and private
/// proprietors. Combine existing accounted-funds capacity before opening another
/// account; prefer the largest pocket to minimize ledger postings and version use.
/// The caller opens at most one new account for any unpaid remainder.
fn plan_accounted_receipts(
    state: &AppState,
    owner: FinancialOwner,
    total: Money,
) -> (Vec<(FinancialAccountId, Money)>, Money) {
    let mut accounts: Vec<_> = state
        .finance()
        .accounts_for(owner)
        .filter(|account| {
            account.kind() == AccountKind::AccountedFunds
                && account.can_receive_liquid_payment(Money::from_cents(1))
        })
        .map(|account| (i64::MAX - account.balance().cents(), account.id()))
        .collect();
    accounts.sort_unstable_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let mut remaining = total.cents();
    let mut allocations = Vec::new();
    for (capacity, account) in accounts {
        if remaining == 0 {
            break;
        }
        let amount = remaining.min(capacity);
        allocations.push((account, Money::from_cents(amount)));
        remaining -= amount;
    }
    (allocations, Money::from_cents(remaining))
}

/// Sources and net receipt for one eligible owner. Source selection and fees
/// remain the responsibility of the calling distribution policy.
#[derive(Default)]
pub(super) struct ProfitDistribution {
    pub(super) total: Money,
    pub(super) postings: Vec<LedgerPosting>,
}

/// Settle the complete automatic cohort or defer it on shared allocator exhaustion.
pub(super) fn settle_profit_distributions(
    state: &mut AppState,
    groups: impl IntoIterator<Item = (FinancialOwner, ProfitDistribution)>,
    memo: &str,
) -> Result<Option<LedgerTransactionId>, BusinessProfitSweepError> {
    match validate_profit_distributions(state, groups, memo) {
        Ok(Some(transaction)) => Ok(Some(transaction.commit(state)?)),
        Ok(None) | Err(BusinessProfitSweepError::Finance(FinanceError::IdExhaustion(_))) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Plan a balanced owner distribution, including any required new accounts.
/// Manual and recurring withdrawals share this receipt policy. The caller
/// supplies distinct owners in deterministic order.
fn validate_profit_distributions(
    state: &AppState,
    groups: impl IntoIterator<Item = (FinancialOwner, ProfitDistribution)>,
    memo: &str,
) -> Result<Option<ValidatedLedgerTransaction>, BusinessProfitSweepError> {
    let mut distributions = Vec::new();
    let mut drafts = Vec::new();
    for (owner, group) in groups {
        if group.total == Money::ZERO {
            continue;
        }
        let (receipts, remaining) = plan_accounted_receipts(state, owner, group.total);
        let new_account = if remaining > Money::ZERO {
            let index = drafts.len();
            drafts.push(FinancialAccountDraft {
                owner,
                kind: AccountKind::AccountedFunds,
            });
            Some((index, remaining))
        } else {
            None
        };
        distributions.push((group.postings, receipts, new_account));
    }
    if distributions.is_empty() {
        return Ok(None);
    }
    let openings_count = u32::try_from(drafts.len()).expect("owner count fits account ID width");
    state
        .ids
        .reserve_many(&[
            (IdKind::FinancialAccount, openings_count),
            (IdKind::LedgerTransaction, 1),
        ])
        .map_err(FinanceError::from)?;

    let openings = if drafts.is_empty() {
        None
    } else {
        Some(validate_open_accounts(state, drafts)?)
    };
    let mut postings = Vec::new();
    for (sources, receipts, new_account) in distributions {
        postings.extend(sources);
        postings.extend(
            receipts
                .into_iter()
                .map(|(account, amount)| LedgerPosting { account, amount }),
        );
        if let Some((index, amount)) = new_account {
            postings.push(LedgerPosting {
                account: openings
                    .as_ref()
                    .expect("a new receipt account has been prevalidated")
                    .account_id(index)
                    .expect("prevalidated opening has its projected account ID"),
                amount,
            });
        }
    }
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        // The shared memo must not disclose other owners' participation.
        memo: memo.to_owned(),
        postings,
        authorization: None,
    };
    let transaction = match openings {
        Some(openings) => {
            validate_record_business_transaction_with_openings(state, openings, draft)?
        }
        None => validate_record_business_transaction(state, draft)?,
    };
    Ok(Some(transaction))
}

/// An owner's draw on a business it owns: legitimate till cash becomes spendable organization
/// wealth. Laundering fees are excluded because they settle into the business's non-liquid
/// clearing account and are a real cost, not earned owner surplus.
#[derive(Clone, Debug)]
pub struct BusinessProfitSweepDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessProfitSweepError {
    #[error("profit sweep amount must be positive")]
    NonPositiveAmount,
    #[error("the requested transfer is entirely consumed by portfolio administration")]
    NonPositiveNetReceipt,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(BusinessId),
    #[error("business {0} has no operating economy to sweep till cash from")]
    MissingBusinessEconomy(BusinessId),
    #[error(
        "business {business} till account {account} is missing or not legitimate operating funds"
    )]
    CorruptTillAccount {
        business: BusinessId,
        account: FinancialAccountId,
    },
    #[error(
        "business {business} till holds {available_cents} cents and cannot sweep {requested_cents}"
    )]
    InsufficientTillCash {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error(
        "business {business} retained-capital basis belongs to ownership version {basis_version}, but current ownership is version {current_version}"
    )]
    StaleCapitalBasis {
        business: BusinessId,
        basis_version: u32,
        current_version: u32,
    },
    #[error(
        "business {business} has only {available_cents} cents of earned surplus above retained capital and cannot sweep {requested_cents}"
    )]
    InsufficientEarnedSurplus {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error("business {business} ownership changed after sweep validation")]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {business} economy changed after sweep validation")]
    StaleEconomy {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("the owner's commercial portfolio changed after the distribution was validated")]
    StalePortfolio,
    #[error(transparent)]
    Finance(#[from] FinanceError),
}

pub struct ValidatedBusinessProfitSweep {
    transaction: ValidatedLedgerTransaction,
    business: BusinessId,
    organization: OrganizationId,
    expected_business_version: u32,
    expected_economy_version: u32,
    expected_overhead_basis_points: u16,
    gross: Money,
    fee: Money,
}

/// Actual committed withdrawal, distinct from the gross amount requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusinessProfitSweepOutcome {
    pub transaction: LedgerTransactionId,
    pub gross: Money,
    pub administration: Money,
    pub received: Money,
}

impl std::fmt::Debug for ValidatedBusinessProfitSweep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedBusinessProfitSweep")
            .field("business", &self.business)
            .field("organization", &self.organization)
            .field("expected_business_version", &self.expected_business_version)
            .field("expected_economy_version", &self.expected_economy_version)
            .finish_non_exhaustive()
    }
}

impl ValidatedBusinessProfitSweep {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<BusinessProfitSweepOutcome, BusinessProfitSweepError> {
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessProfitSweepError::MissingBusiness(self.business))?;
        if business_record.version() != self.expected_business_version {
            return Err(BusinessProfitSweepError::StaleBusiness {
                business: self.business,
                expected: self.expected_business_version,
                found: business_record.version(),
            });
        }
        if business_record.owner() != crate::world::BusinessOwner::Organization(self.organization) {
            return Err(BusinessProfitSweepError::ForeignBusiness(self.business));
        }
        let economy = state.economy.get_business_economy(self.business).ok_or(
            BusinessProfitSweepError::MissingBusinessEconomy(self.business),
        )?;
        if economy.version() != self.expected_economy_version {
            return Err(BusinessProfitSweepError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        if portfolio_overhead_basis_points(state, self.organization)
            != self.expected_overhead_basis_points
        {
            return Err(BusinessProfitSweepError::StalePortfolio);
        }
        let transaction = self.transaction.commit(state)?;
        Ok(BusinessProfitSweepOutcome {
            transaction,
            gross: self.gross,
            administration: self.fee,
            received: self
                .gross
                .checked_sub(self.fee)
                .expect("fee does not exceed gross"),
        })
    }
}

pub fn validate_sweep_business_profits(
    state: &AppState,
    draft: BusinessProfitSweepDraft,
) -> Result<ValidatedBusinessProfitSweep, BusinessProfitSweepError> {
    if draft.amount <= Money::ZERO {
        return Err(BusinessProfitSweepError::NonPositiveAmount);
    }
    let context = resolve_business_profit_sweep_context(state, draft.organization, draft.business)?;
    validate_business_profit_sweep_capacity(
        &draft,
        context.operating_balance,
        context.capital_floor,
    )?;
    let overhead = portfolio_overhead_basis_points(state, draft.organization);
    let fee = portfolio_overhead_charge(draft.amount, overhead);
    let owner_receipt = draft.amount.checked_sub(fee).expect("fee is at most gross");
    if owner_receipt == Money::ZERO {
        return Err(BusinessProfitSweepError::NonPositiveNetReceipt);
    }
    let mut postings = vec![LedgerPosting {
        account: context.operating_account,
        amount: draft
            .amount
            .checked_neg()
            .expect("positive gross must negate"),
    }];
    if fee > Money::ZERO {
        postings.push(LedgerPosting {
            account: context.settlement_account,
            amount: fee,
        });
    }
    let memo = format!(
        "Owner withdrawal of {} from {} till",
        format_money_cents(draft.amount.cents()),
        context.business_name,
    );
    let transaction = validate_profit_distributions(
        state,
        [(
            FinancialOwner::Organization(draft.organization),
            ProfitDistribution {
                total: owner_receipt,
                postings,
            },
        )],
        &memo,
    )?
    .expect("a positive owner receipt creates a distribution");
    Ok(ValidatedBusinessProfitSweep {
        transaction,
        business: draft.business,
        organization: draft.organization,
        expected_business_version: context.business_version,
        expected_economy_version: context.economy_version,
        expected_overhead_basis_points: overhead,
        gross: draft.amount,
        fee,
    })
}

pub(super) struct BusinessProfitSweepContext<'a> {
    pub(super) operating_account: FinancialAccountId,
    pub(super) settlement_account: FinancialAccountId,
    operating_balance: Money,
    capital_floor: Money,
    business_name: &'a str,
    business_version: u32,
    economy_version: u32,
}

impl BusinessProfitSweepContext<'_> {
    /// Source-side capacity, before destination-account allocation. A fee must fit
    /// into the settlement account, and the resulting owner receipt must be positive.
    pub(super) fn withdrawable_gross(
        &self,
        state: &AppState,
        overhead_rate: u16,
        limit: Money,
    ) -> Money {
        let operating = state
            .finance()
            .get_account(self.operating_account)
            .expect("owner-sweep context proved its operating account");
        let mut amount =
            available_business_profit(operating.available_payment_balance(), self.capital_floor)
                .min(limit);
        if amount == Money::ZERO {
            return Money::ZERO;
        }
        if overhead_rate > 0 {
            let settlement = state
                .finance()
                .get_account(self.settlement_account)
                .expect("owner-sweep context proved its settlement account");
            if settlement.version() == u32::MAX {
                return Money::ZERO;
            }
            let fee_headroom = i128::from(i64::MAX) - i128::from(settlement.balance().cents());
            // ceil(gross * rate / 10_000) <= headroom exactly at this bound.
            let gross_capacity = fee_headroom * 10_000 / i128::from(overhead_rate);
            amount = amount.min(Money::from_cents(
                gross_capacity.min(i128::from(i64::MAX)) as i64
            ));
        }
        if amount <= portfolio_overhead_charge(amount, overhead_rate) {
            Money::ZERO
        } else {
            amount
        }
    }
}

pub(super) fn resolve_business_profit_sweep_context(
    state: &AppState,
    organization: OrganizationId,
    business: BusinessId,
) -> Result<BusinessProfitSweepContext<'_>, BusinessProfitSweepError> {
    if state.world.get_organization(organization).is_none() {
        return Err(BusinessProfitSweepError::MissingOrganization(organization));
    }
    let business_record = state
        .world
        .get_business(business)
        .ok_or(BusinessProfitSweepError::MissingBusiness(business))?;
    if business_record.owner() != crate::world::BusinessOwner::Organization(organization) {
        return Err(BusinessProfitSweepError::ForeignBusiness(business));
    }
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessProfitSweepError::MissingBusinessEconomy(business))?;
    let operating = state
        .finance
        .get_account(economy.operating_account())
        .ok_or(BusinessProfitSweepError::CorruptTillAccount {
            business,
            account: economy.operating_account(),
        })?;
    if operating.owner() != FinancialOwner::Business(business)
        || operating.kind() != AccountKind::LegitimateOperating
    {
        return Err(BusinessProfitSweepError::CorruptTillAccount {
            business,
            account: economy.operating_account(),
        });
    }
    if economy.capital_floor_business_version() != business_record.version() {
        return Err(BusinessProfitSweepError::StaleCapitalBasis {
            business,
            basis_version: economy.capital_floor_business_version(),
            current_version: business_record.version(),
        });
    }
    Ok(BusinessProfitSweepContext {
        operating_account: economy.operating_account(),
        settlement_account: economy.settlement_account(),
        operating_balance: operating.spendable_balance(),
        capital_floor: economy.operating_capital_floor(),
        business_name: business_record.name(),
        business_version: business_record.version(),
        economy_version: economy.version(),
    })
}

/// A title change may temporarily leave operating books on their previous owner's
/// capital basis. No distribution is eligible until the current basis is established.
pub(super) fn eligible_profit_source(
    state: &AppState,
    organization: OrganizationId,
    business: BusinessId,
) -> Result<Option<BusinessProfitSweepContext<'_>>, BusinessProfitSweepError> {
    match resolve_business_profit_sweep_context(state, organization, business) {
        Ok(context) => Ok(Some(context)),
        Err(BusinessProfitSweepError::StaleCapitalBasis { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn available_at_rate(
    state: &AppState,
    organization: OrganizationId,
    business: BusinessId,
    overhead_rate: u16,
) -> Result<Money, BusinessProfitSweepError> {
    Ok(
        eligible_profit_source(state, organization, business)?.map_or(Money::ZERO, |context| {
            context.withdrawable_gross(state, overhead_rate, Money::from_cents(i64::MAX))
        }),
    )
}

/// Read the source-side earned surplus currently eligible for withdrawal, before
/// allocating destination accounts or reserving ledger IDs.
pub fn resolve_available_business_profit(
    state: &AppState,
    organization: OrganizationId,
    business: BusinessId,
) -> Result<Money, BusinessProfitSweepError> {
    available_at_rate(
        state,
        organization,
        business,
        portfolio_overhead_basis_points(state, organization),
    )
}

/// Current portfolio fee and source-side withdrawal capacity of each operating book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationProfitAvailability {
    pub administration_basis_points: u16,
    pub gross_by_business: BTreeMap<BusinessId, Money>,
}

/// Compute the fee rate once for the entire owned portfolio.
pub fn organization_profit_availability(
    state: &AppState,
    organization: OrganizationId,
) -> Result<OrganizationProfitAvailability, BusinessProfitSweepError> {
    if state.world.get_organization(organization).is_none() {
        return Err(BusinessProfitSweepError::MissingOrganization(organization));
    }
    let administration_basis_points = portfolio_overhead_basis_points(state, organization);
    let gross_by_business = state
        .world()
        .businesses_owned_by_organization(organization)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some()
        })
        .map(|business| {
            let id = business.id();
            available_at_rate(state, organization, id, administration_basis_points)
                .map(|amount| (id, amount))
        })
        .collect::<Result<_, _>>()?;
    Ok(OrganizationProfitAvailability {
        administration_basis_points,
        gross_by_business,
    })
}

/// Shared ceiling for manual and automatic withdrawals. Negative or unavailable retained
/// capital never turns into a positive withdrawable balance.
pub(super) fn available_business_profit(operating_balance: Money, capital_floor: Money) -> Money {
    operating_balance
        .checked_sub(capital_floor)
        .unwrap_or(Money::ZERO)
        .max(Money::ZERO)
}

pub(super) fn validate_business_profit_sweep_capacity(
    draft: &BusinessProfitSweepDraft,
    operating_balance: Money,
    capital_floor: Money,
) -> Result<(), BusinessProfitSweepError> {
    if operating_balance < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientTillCash {
            business: draft.business,
            available_cents: operating_balance.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    let available_surplus = available_business_profit(operating_balance, capital_floor);
    if available_surplus < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientEarnedSurplus {
            business: draft.business,
            available_cents: available_surplus.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    Ok(())
}
