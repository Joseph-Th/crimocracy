//! Non-player commercial investment. Executives buy from the same market as the player.
//! Purchases are separate transactions, not one city-wide atomic batch.

use super::seller_strategy::daily_net;
use super::{
    BusinessAcquisitionDraft, acquisition_offer, negotiated_reference_price,
    validate_acquire_business, validate_offer_business,
};
use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimDuration, is_recurring_boundary};
use crate::economy::{
    BusinessOperatingStatus, business_economy_system::forecast_normal_business_net,
};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

const REVIEW_DAYS: u32 = 7;

/// Every week, each free non-player criminal executive may make one investment.
/// Prefer listed property over voluntary offers. First improve sustainable payroll
/// coverage, then prefer existing districts and estimated return per unit of capital
/// and time. A new foothold remains possible even after local inventory sells. Keep at least one
/// week's payroll in spendable reserves; only accounted funds can pay the price.
/// Seller consent and all transfer, ledger and report effects remain canonical.
pub(crate) fn apply_due_autonomous_acquisitions(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<BusinessId> {
    let cadence = SimDuration::from_minutes(REVIEW_DAYS * DAY_MINUTES as u32);
    if !is_recurring_boundary(state.now(), cadence) {
        return Vec::new();
    }
    let organizations: Vec<_> = state
        .world
        .organizations()
        .filter(|organization| {
            organization.kind() == OrganizationKind::Criminal
                && Some(organization.id()) != state.player_organization()
        })
        .map(|organization| organization.id())
        .collect();
    let mut acquired = Vec::new();
    for organization in organizations {
        if let Some(business) = invest(registry, state, organization) {
            acquired.push(business);
        }
    }
    acquired
}

fn invest(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
) -> Option<BusinessId> {
    let members: Vec<_> = state
        .world
        .characters_in_organization(organization)
        .collect();
    if !members.iter().any(|member| {
        member.supervisor().is_none()
            && state
                .legal()
                .active_arrest_for_character(member.id())
                .is_none()
    }) {
        return None;
    }
    let mut districts = BTreeSet::new();
    let mut owned = BTreeSet::new();
    let mut expected_daily_net = 0_i128;
    for business in state.world.businesses_owned_by_organization(organization) {
        districts.insert(business.neighborhood());
        owned.insert(business.id());
        if state
            .economy()
            .get_business_economy(business.id())
            .is_some_and(|e| e.status() == BusinessOperatingStatus::Active)
        {
            expected_daily_net += daily_net(registry, state, business.id()).ok()?;
        }
    }
    let daily_payroll =
        i128::from(registry.upkeep().per_member_daily().cents()) * members.len() as i128;
    let wage_gap = (daily_payroll - expected_daily_net).max(0);
    let reserve = daily_payroll * i128::from(REVIEW_DAYS);
    let mut funding_accounts = BTreeSet::new();
    let mut accounted = 0_i128;
    let mut other_liquidity = 0_i128;
    for account in state
        .finance
        .accounts_for(FinancialOwner::Organization(organization))
    {
        let spendable = account.available_payment_balance();
        if spendable == Money::ZERO {
            continue;
        }
        if account.kind() == AccountKind::AccountedFunds {
            accounted += i128::from(spendable.cents());
            funding_accounts.insert(account.id());
        } else {
            other_liquidity += i128::from(spendable.cents());
        }
    }
    // Never count dirty cash toward the price; it only reduces the clean wage reserve.
    let budget = accounted - (reserve - other_liquidity).max(0);
    if budget <= 0 {
        return None;
    }
    let mut candidates = Vec::new();
    for (id, _, kind, district) in state.world.public_business_directory() {
        if owned.contains(&id) {
            continue;
        }
        // Market listing availability and standard appraisals are public. In the
        // absence of a listing, submit a voluntary offer; the seller alone decides
        // using its private portfolio. No foreign balance or owner enters the score.
        let listing = acquisition_offer(registry, state, organization, id);
        // Do not create operating books from unmodeled premises in the autonomous market.
        if listing.is_some() && state.economy.get_business_economy(id).is_none() {
            continue;
        }
        let listed = listing.is_some();
        let price = if let Some((price, _)) = listing {
            price
        } else {
            let Ok(cents) = negotiated_reference_price(registry, state, id) else {
                continue;
            };
            let Ok(cents) = i64::try_from(cents) else {
                continue;
            };
            Money::from_cents(cents)
        };
        if price <= Money::ZERO || i128::from(price.cents()) > budget {
            continue;
        }
        let Ok(net) = forecast_normal_business_net(registry, state, id) else {
            continue;
        };
        if net <= Money::ZERO {
            continue;
        }
        let cycle = registry.get_business(kind).economics().cycle().as_minutes();
        let return_score = i128::from(net.cents()) * 1_000_000_000_000_i128
            / i128::from(price.cents())
            / i128::from(cycle);
        // Prefer extending an existing foothold, but do not strand capital after
        // local inventory is exhausted. Offers rank behind independent listings.
        let daily_contribution =
            i128::from(net.cents()) * i128::from(DAY_MINUTES) / i128::from(cycle);
        candidates.push((
            !listed,
            std::cmp::Reverse(daily_contribution.min(wage_gap)),
            !districts.contains(&district),
            std::cmp::Reverse(return_score),
            price,
            id,
        ));
    }
    candidates.sort_unstable();
    for (negotiated, _, _, _, price, business) in candidates {
        // Inability to complete a purchase (including exhausted IDs or account capacity) is
        // a normal market constraint. Canonical validation and commit leave state unchanged
        // on rejection; try the next affordable listing instead of panicking in a tick.
        let draft = BusinessAcquisitionDraft {
            organization,
            business,
            funding_accounts: funding_accounts.clone(),
        };
        let purchase = if negotiated {
            validate_offer_business(registry, state, draft, price)
        } else {
            validate_acquire_business(registry, state, draft)
        };
        if let Ok(purchase) = purchase
            && purchase.commit(registry, state).is_ok()
        {
            return Some(business);
        }
    }
    None
}
