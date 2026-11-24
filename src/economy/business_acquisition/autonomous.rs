//! Non-player commercial investment. Executives buy from the same market as the player.
//! Purchases are separate transactions, not one city-wide atomic batch.

use super::portfolio_forecast::PortfolioForecast;
use super::{
    BusinessAcquisitionDraft, acquisition_offer, appraise_business, buyer_adjusted_price,
    validate_acquire_business, validate_offer_business,
};
use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimDuration, is_recurring_boundary};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::world_system::sole_available_executive;
use crate::world::{BusinessOwner, OrganizationKind};
use std::collections::BTreeSet;

const REVIEW_DAYS: u32 = 7;

/// Every week, each non-player gang with a sole available executive may make one investment.
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
    let mut organizations: Vec<_> = state
        .world
        .organizations()
        .filter(|organization| {
            organization.kind() == OrganizationKind::Criminal
                && Some(organization.id()) != state.player_organization()
        })
        .map(|organization| organization.id())
        .collect();
    // Rival investors compete for the same public listings. Always letting the lowest
    // organization ID offer first would grant the earliest-created rival a permanent
    // market advantage, even when equally funded executives want the same property.
    // Rotate the stable ID order each review week; this uses only the campaign clock and
    // current roster, and never consumes a simulation RNG stream or persists a cursor.
    if !organizations.is_empty() {
        organizations.sort_unstable();
        let review_number = state.now().as_minutes() / (u64::from(REVIEW_DAYS) * DAY_MINUTES);
        let offset = (review_number - 1)
            % u64::try_from(organizations.len()).expect("organization count fits u64");
        organizations.rotate_left(usize::try_from(offset).expect("offset fits roster length"));
    }
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
    sole_available_executive(state, organization)?;
    let portfolio = PortfolioForecast::new(registry, state, organization, None).ok()?;
    let expected_daily_net = portfolio.daily_net();
    let member_count = state
        .world()
        .characters_in_organization(organization)
        .count();
    let daily_payroll =
        i128::from(registry.upkeep().per_member_daily().cents()) * member_count as i128;
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
    for (id, _, _, district) in state.world.public_business_directory() {
        if state
            .world()
            .get_business(id)
            .is_some_and(|business| business.owner() == BusinessOwner::Organization(organization))
        {
            continue;
        }
        // Rival investors buy operating businesses, not empty premises. Both listings
        // and negotiated offers need existing books: filter before appraisal and bidding
        // rather than sending impossible offers through the canonical seller validator.
        let Some(economy) = state.economy.get_business_economy(id) else {
            continue;
        };
        // A purchased property is restarted under the buyer. At the final economy version,
        // that restart cannot schedule another operating cycle: autonomous investors must
        // not spend capital on a permanently inert asset merely because its public appraisal
        // still looks profitable.
        if economy.version() >= u32::MAX - 1 {
            continue;
        }
        // Market listing availability and standard appraisals are public. In the
        // absence of a listing, submit a voluntary offer; the seller alone decides
        // using its private portfolio. No foreign balance or owner enters the score.
        let listing = acquisition_offer(registry, state, organization, id);
        let listed = listing.is_some();
        let Ok(appraisal) = appraise_business(registry, state, id) else {
            continue;
        };
        let price = if let Some((price, _)) = listing {
            price
        } else {
            let Ok(price) = buyer_adjusted_price(
                appraisal.reference_offer_cents,
                portfolio.administration.property_count(),
            ) else {
                continue;
            };
            price
        };
        if price <= Money::ZERO || i128::from(price.cents()) > budget {
            continue;
        }
        // Assess both the incoming earnings and the new administration rate on all
        // existing holdings; reopening a dormant listing can also displace their sales.
        let Ok(incremental_daily_net) =
            portfolio.purchase_gain(registry, state, id, district, &appraisal)
        else {
            continue;
        };
        if incremental_daily_net <= 0 {
            continue;
        }
        let return_score = incremental_daily_net * 1_000_000_000_i128 / i128::from(price.cents());
        // Prefer extending an existing foothold, but do not strand capital after
        // local inventory is exhausted. Offers rank behind independent listings.
        candidates.push((
            !listed,
            std::cmp::Reverse(incremental_daily_net.min(wage_gap)),
            !portfolio.administration.contains_district(district),
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
