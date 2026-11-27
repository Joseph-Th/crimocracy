//! Deterministic daily distribution of earned business profits to criminal owners.
//!
//! One balanced ledger batch reuses the canonical owner-sweep capital ceiling and the finance
//! owner's atomic account-opening path. No withdrawal can spend a front's retained working
//! capital; player-owned fronts participate only under an explicit standing instruction.

use super::profit_sweep::{
    BusinessProfitSweepError, ProfitDistribution, eligible_profit_source,
    settle_profit_distributions,
};
use super::{portfolio_overhead_basis_points, portfolio_overhead_charge};
use crate::core::id::{LedgerTransactionId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::finance::{FinancialOwner, LedgerPosting, Money};
use crate::world::OrganizationKind;
use std::collections::BTreeMap;

/// Distributes existing legitimate earnings before the daily payroll debit. Reuse the combined
/// receiving capacity of existing accounted-funds accounts before opening a new account; no
/// single nearly-full treasury pocket can strand otherwise withdrawable profits.
/// Every participating business is validated through the direct owner's retained-capital rule;
/// the entire cross-owner ledger and all destination openings commit as one transaction.
/// Scarce shared ledger/account IDs defer the complete batch to another day, without giving the
/// earliest organization a priority claim on the last representable ID.
pub(crate) fn apply_due_autonomous_profit_sweeps(
    state: &mut AppState,
) -> Result<Option<LedgerTransactionId>, BusinessProfitSweepError> {
    if !is_day_boundary(state.now()) {
        return Ok(None);
    }
    let groups = plan_available_profit_sources(state)?;
    settle_profit_distributions(
        state,
        groups
            .into_iter()
            .map(|(owner, group)| (FinancialOwner::Organization(owner), group)),
        "Daily owner profit distribution",
    )
}

fn plan_available_profit_sources(
    state: &AppState,
) -> Result<BTreeMap<OrganizationId, ProfitDistribution>, BusinessProfitSweepError> {
    let mut groups = BTreeMap::new();
    for owner in state.world.organizations().filter(|organization| {
        organization.kind() == OrganizationKind::Criminal
            && (Some(organization.id()) != state.player_organization()
                || state.automatic_profit_sweeps())
    }) {
        let organization = owner.id();
        let overhead_rate = portfolio_overhead_basis_points(state, organization);
        let mut group = ProfitDistribution::default();
        for business in state.world().businesses_owned_by_organization(organization) {
            if state
                .economy()
                .get_business_economy(business.id())
                .is_none()
            {
                continue;
            }
            let Some(context) = eligible_profit_source(state, organization, business.id())? else {
                continue;
            };
            // Bound each owner's daily aggregate without overflowing representable money.
            let remaining_headroom = Money::from_cents(i64::MAX - group.total.cents());
            let amount = context.withdrawable_gross(state, overhead_rate, remaining_headroom);
            if amount == Money::ZERO {
                continue;
            }
            let overhead = portfolio_overhead_charge(amount, overhead_rate);
            let receipt = amount
                .checked_sub(overhead)
                .expect("owner fee cannot exceed gross");
            group.total = group
                .total
                .checked_add(receipt)
                .expect("owner distribution is capped by representable money");
            group.postings.push(LedgerPosting {
                account: context.operating_account,
                amount: amount.checked_neg().expect("earned surplus negates"),
            });
            if overhead > Money::ZERO {
                group.postings.push(LedgerPosting {
                    account: context.settlement_account,
                    amount: overhead,
                });
            }
        }
        if group.total > Money::ZERO {
            groups.insert(organization, group);
        }
    }
    Ok(groups)
}
