//! Private proprietors are actual characters with personal books. Their daily
//! dividends draw only realized surplus above each shop's working capital,
//! through the same balanced business-ledger access as organization sweeps.
//! One fully preflighted transaction covers the viable cohort without using
//! shop ID order to select which individual gets paid.

use super::profit_sweep::{
    BusinessProfitSweepError, ProfitDistribution, available_business_profit,
    settle_profit_distributions,
};
use crate::core::id::{CharacterId, LedgerTransactionId};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::finance::{AccountKind, FinancialOwner, LedgerPosting, Money};
use crate::world::BusinessOwner;
use std::collections::BTreeMap;

pub(crate) fn apply_due_personal_owner_dividends(
    state: &mut AppState,
) -> Result<Option<LedgerTransactionId>, BusinessProfitSweepError> {
    if !is_day_boundary(state.now()) {
        return Ok(None);
    }
    let mut owners: BTreeMap<CharacterId, ProfitDistribution> = BTreeMap::new();
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
        let group = owners.entry(character).or_default();
        let amount = surplus.min(Money::from_cents(i64::MAX - group.total.cents()));
        if amount == Money::ZERO {
            continue;
        }
        group.total = group
            .total
            .checked_add(amount)
            .expect("group payment is capped by representable money");
        group.postings.push(LedgerPosting {
            account: operating.id(),
            amount: amount.checked_neg().expect("private dividend is positive"),
        });
    }
    settle_profit_distributions(
        state,
        owners
            .into_iter()
            .map(|(owner, group)| (FinancialOwner::Character(owner), group)),
        "Daily personal proprietor dividends",
    )
}
