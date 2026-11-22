//! Deterministic daily distribution of earned business profits to criminal owners.
//!
//! One balanced ledger batch reuses the canonical owner-sweep capital ceiling and the finance
//! owner's atomic account-opening path. No withdrawal can spend a front's retained working
//! capital; player-owned fronts participate only under an explicit standing instruction.

use super::profit_sweep::{
    BusinessProfitSweepDraft, BusinessProfitSweepError, available_business_profit,
    resolve_business_profit_sweep_context, validate_business_profit_sweep_capacity,
};
use crate::core::id::{
    BusinessId, FinancialAccountId, IdKind, LedgerTransactionId, OrganizationId,
};
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
use crate::world::{BusinessOwner, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct PlannedProfitSource {
    business: BusinessId,
    operating_account: FinancialAccountId,
    amount: Money,
    business_version: u32,
    economy_version: u32,
}

struct OrganizationProfitPlan {
    total: Money,
    sources: Vec<PlannedProfitSource>,
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
    if groups.is_empty() {
        return Ok(None);
    }

    let mut destinations = BTreeMap::new();
    let mut opening_owners = Vec::new();
    for (organization, group) in &groups {
        let mut remaining = group.total.cents();
        let mut allocations = Vec::new();
        let mut receiving_accounts: Vec<_> = state
            .finance
            .accounts_for(FinancialOwner::Organization(*organization))
            .filter(|account| {
                account.kind() == AccountKind::AccountedFunds
                    && account.can_receive_liquid_payment(Money::from_cents(1))
            })
            .map(|account| (i64::MAX - account.balance().cents(), account.id()))
            .collect();
        // Prefer the largest usable pocket. This preserves the previous single-account
        // destination when one can take the entire draw, minimizing ledger postings and
        // version advances; ID decides only equal-capacity ties.
        receiving_accounts
            .sort_unstable_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        for (capacity, account) in receiving_accounts {
            if remaining == 0 {
                break;
            }
            let amount = remaining.min(capacity);
            allocations.push((account, Money::from_cents(amount)));
            remaining -= amount;
        }
        if remaining > 0 {
            opening_owners.push((*organization, Money::from_cents(remaining)));
        }
        destinations.insert(*organization, allocations);
    }

    let openings_count =
        u32::try_from(opening_owners.len()).expect("organization count fits account ID width");
    if state
        .ids
        .reserve_many(&[
            (IdKind::FinancialAccount, openings_count),
            (IdKind::LedgerTransaction, 1),
        ])
        .is_err()
    {
        return Ok(None);
    }

    let openings = if opening_owners.is_empty() {
        None
    } else {
        let drafts = opening_owners
            .iter()
            .map(|(organization, _)| FinancialAccountDraft {
                owner: FinancialOwner::Organization(*organization),
                kind: AccountKind::AccountedFunds,
            })
            .collect();
        Some(validate_open_accounts(state, drafts)?)
    };
    if let Some(openings) = &openings {
        for (index, (organization, remaining)) in opening_owners.iter().enumerate() {
            destinations
                .get_mut(organization)
                .expect("an opening belongs to a planned profitable owner")
                .push((
                    openings
                        .account_id(index)
                        .expect("prevalidated opening has its projected account ID"),
                    *remaining,
                ));
        }
    }

    let mut postings = Vec::new();

    for (organization, group) in &groups {
        let allocations = &destinations[organization];
        let destination = allocations
            .first()
            .expect("each profitable owner has at least one receiving account")
            .0;
        for source in &group.sources {
            let context =
                resolve_business_profit_sweep_context(state, *organization, source.business)?;
            if context.business_version != source.business_version {
                return Err(BusinessProfitSweepError::StaleBusiness {
                    business: source.business,
                    expected: source.business_version,
                    found: context.business_version,
                });
            }
            if context.economy_version != source.economy_version {
                return Err(BusinessProfitSweepError::StaleEconomy {
                    business: source.business,
                    expected: source.economy_version,
                    found: context.economy_version,
                });
            }
            validate_business_profit_sweep_capacity(
                &BusinessProfitSweepDraft {
                    organization: *organization,
                    business: source.business,
                    destination,
                    amount: source.amount,
                },
                context.operating_balance,
                context.capital_floor,
            )?;
            postings.push(LedgerPosting {
                account: source.operating_account,
                amount: source.amount.checked_neg().expect("earned surplus negates"),
            });
        }
        postings.extend(allocations.iter().map(|(account, amount)| LedgerPosting {
            account: *account,
            amount: *amount,
        }));
    }

    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        // The shared ledger memo must not disclose other owners' participation.
        memo: "Daily owner profit distribution".to_owned(),
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
                total: Money::ZERO,
                sources: Vec::new(),
            });
        // A destination can hold at most i64::MAX cents. A wealthy owner with several tills
        // withdraws the first representable portion today and keeps the remainder invested
        // rather than overflowing the money model or bricking every later daily tick.
        let remaining_headroom = Money::from_cents(i64::MAX - group.total.cents());
        let amount = surplus.min(remaining_headroom);
        if amount == Money::ZERO {
            continue;
        }
        group.total = group
            .total
            .checked_add(amount)
            .expect("owner distribution is capped by representable money");
        group.sources.push(PlannedProfitSource {
            business: business.id(),
            operating_account: context.operating_account,
            amount,
            business_version: context.business_version,
            economy_version: context.economy_version,
        });
    }
    Ok(groups)
}
