//! Private proprietors are actual characters with personal books. Their daily
//! dividends draw only realized surplus above each shop's working capital,
//! through the same balanced business-ledger access as organization sweeps.
//! One fully preflighted transaction covers the viable cohort without using
//! shop ID order to select which individual gets paid.

use super::profit_sweep::{
    BusinessProfitSweepError, available_business_profit, plan_accounted_receipts,
};
use crate::core::id::{BusinessId, CharacterId, FinancialAccountId, IdKind, LedgerTransactionId};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::finance::finance_system::{
    validate_open_accounts, validate_record_business_transaction,
    validate_record_business_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::world::BusinessOwner;
use std::collections::BTreeMap;

struct OwnerPayment {
    total: Money,
    sources: Vec<(BusinessId, FinancialAccountId, Money)>,
}

pub(crate) fn apply_due_personal_owner_dividends(
    state: &mut AppState,
) -> Result<Option<LedgerTransactionId>, BusinessProfitSweepError> {
    if !is_day_boundary(state.now()) {
        return Ok(None);
    }
    let mut owners: BTreeMap<CharacterId, OwnerPayment> = BTreeMap::new();
    for business in state.world().businesses() {
        let BusinessOwner::Character(character) = business.owner() else {
            continue;
        };
        let Some(economy) = state.economy().get_business_economy(business.id()) else {
            continue;
        };
        if economy.capital_floor_business_version() != business.version() {
            continue;
        }
        let Some(operating) = state.finance().get_account(economy.operating_account()) else {
            continue;
        };
        if operating.version() == u32::MAX
            || operating.owner() != FinancialOwner::Business(business.id())
            || operating.kind() != AccountKind::LegitimateOperating
        {
            continue;
        }
        let surplus = available_business_profit(
            operating.spendable_balance(),
            economy.operating_capital_floor(),
        );
        if surplus <= Money::ZERO {
            continue;
        }
        let group = owners.entry(character).or_insert_with(|| OwnerPayment {
            total: Money::ZERO,
            sources: Vec::new(),
        });
        let amount = surplus.min(Money::from_cents(i64::MAX - group.total.cents()));
        if amount == Money::ZERO {
            continue;
        }
        group.total = group
            .total
            .checked_add(amount)
            .expect("group payment is capped by representable money");
        group.sources.push((business.id(), operating.id(), amount));
    }
    if owners.is_empty() {
        return Ok(None);
    }

    let mut destinations = BTreeMap::new();
    let mut missing = Vec::new();
    for (&character, group) in &owners {
        let (allocations, remaining) =
            plan_accounted_receipts(state, FinancialOwner::Character(character), group.total);
        destinations.insert(character, allocations);
        if remaining > Money::ZERO {
            missing.push((character, remaining));
        }
    }
    if state
        .ids
        .reserve_many(&[
            (
                IdKind::FinancialAccount,
                u32::try_from(missing.len()).expect("private proprietor cohort fits id space"),
            ),
            (IdKind::LedgerTransaction, 1),
        ])
        .is_err()
    {
        return Ok(None);
    }
    let openings = if missing.is_empty() {
        None
    } else {
        Some(validate_open_accounts(
            state,
            missing
                .iter()
                .map(|(character, _)| FinancialAccountDraft {
                    owner: FinancialOwner::Character(*character),
                    kind: AccountKind::AccountedFunds,
                })
                .collect(),
        )?)
    };
    if let Some(openings) = &openings {
        for (index, (owner, remaining)) in missing.iter().enumerate() {
            destinations
                .get_mut(owner)
                .expect("private proprietor was included in the allocation")
                .push((
                    openings
                        .account_id(index)
                        .expect("prevalidated personal bank account has an ID"),
                    *remaining,
                ));
        }
    }
    let mut postings = Vec::new();
    for (&character, group) in &owners {
        for &(_business, account, amount) in &group.sources {
            postings.push(LedgerPosting {
                account,
                amount: amount.checked_neg().expect("private dividend is positive"),
            });
        }
        postings.extend(
            destinations[&character]
                .iter()
                .map(|&(account, amount)| LedgerPosting { account, amount }),
        );
    }
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: "Daily personal proprietor dividends".to_owned(),
        postings,
        authorization: None,
    };
    let transaction = match openings {
        Some(openings) => {
            validate_record_business_transaction_with_openings(state, openings, draft)?
        }
        None => validate_record_business_transaction(state, draft)?,
    };
    Ok(Some(transaction.commit(state)?))
}
