//! Deterministic daily distribution of earned business profits to criminal owners.
//!
//! One balanced ledger batch reuses the canonical owner-sweep capital ceiling and the finance
//! owner's atomic account-opening path. No withdrawal can spend a front's retained working
//! capital; player-owned fronts participate only under an explicit standing instruction.

use super::profit_sweep::{
    BusinessProfitSweepError, ProfitDistribution, available_business_profit,
    resolve_business_profit_sweep_context, settle_profit_distributions,
};
use super::{portfolio_overhead_basis_points, portfolio_overhead_charge};
use crate::core::id::{LedgerTransactionId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::finance::{FinancialOwner, LedgerPosting, Money};
use crate::world::{BusinessOwner, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct OrganizationProfitPlan {
    distribution: ProfitDistribution,
    overhead_rate: u16,
}

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
            .map(|(owner, group)| (FinancialOwner::Organization(owner), group.distribution)),
        "Daily owner profit distribution",
    )
}

fn plan_available_profit_sources(
    state: &AppState,
) -> Result<BTreeMap<OrganizationId, OrganizationProfitPlan>, BusinessProfitSweepError> {
    let eligible_organizations: BTreeSet<_> = state
        .world
        .organizations()
        .filter(|organization| {
            organization.kind() == OrganizationKind::Criminal
                && (Some(organization.id()) != state.player_organization()
                    || state.automatic_profit_sweeps())
        })
        .map(|organization| organization.id())
        .collect();
    if eligible_organizations.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut groups = BTreeMap::new();
    for business in state.world.businesses() {
        let BusinessOwner::Organization(organization) = business.owner() else {
            continue;
        };
        if !eligible_organizations.contains(&organization)
            || state.economy.get_business_economy(business.id()).is_none()
        {
            continue;
        }
        let context = resolve_business_profit_sweep_context(state, organization, business.id())?;
        let operating = state
            .finance
            .get_account(context.operating_account)
            .expect("owner-sweep context proved its operating account");
        // At a finite version rail this till cannot be posted again. Preserve its historical
        // balance and continue to distribute independently withdrawable earnings.
        if operating.version() == u32::MAX {
            continue;
        }
        let surplus = available_business_profit(context.operating_balance, context.capital_floor);
        if surplus == Money::ZERO {
            continue;
        }

        let group = groups
            .entry(organization)
            .or_insert_with(|| OrganizationProfitPlan {
                distribution: ProfitDistribution::default(),
                overhead_rate: portfolio_overhead_basis_points(state, organization),
            });
        // A destination can hold at most i64::MAX cents. A wealthy owner with several tills
        // withdraws the first representable portion today and keeps the remainder invested
        // rather than overflowing the money model or bricking every later daily tick.
        let remaining_headroom = Money::from_cents(i64::MAX - group.distribution.total.cents());
        let mut amount = surplus.min(remaining_headroom);
        // Administration is credited to the source's settlement account. A full or
        // terminal settlement account must not block otherwise independent withdrawals.
        if group.overhead_rate > 0 {
            let settlement = state
                .finance
                .get_account(context.settlement_account)
                .expect("owner-sweep context proved its settlement account");
            if settlement.version() == u32::MAX {
                continue;
            }
            let fee_headroom = i128::from(i64::MAX) - i128::from(settlement.balance().cents());
            // ceil(gross * rate / 10_000) <= headroom exactly when gross <= this bound.
            let gross_capacity = fee_headroom * 10_000 / i128::from(group.overhead_rate);
            amount = amount.min(Money::from_cents(
                gross_capacity.min(i128::from(i64::MAX)) as i64
            ));
        }
        if amount == Money::ZERO {
            continue;
        }
        let overhead = portfolio_overhead_charge(amount, group.overhead_rate);
        let receipt = amount
            .checked_sub(overhead)
            .expect("owner fee cannot exceed gross");
        if receipt == Money::ZERO {
            continue;
        }
        group.distribution.total = group
            .distribution
            .total
            .checked_add(receipt)
            .expect("owner distribution is capped by representable money");
        group.distribution.postings.push(LedgerPosting {
            account: context.operating_account,
            amount: amount.checked_neg().expect("earned surplus negates"),
        });
        if overhead > Money::ZERO {
            group.distribution.postings.push(LedgerPosting {
                account: context.settlement_account,
                amount: overhead,
            });
        }
    }
    Ok(groups)
}
