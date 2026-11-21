//! Seller-side portfolio decisions. A buyer can quote a public estimate, but may not
//! inspect the seller's private cash, roster, or infrastructure dependencies.

use super::{BusinessAcquisitionError, acquisition_offer};
use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::economy::BusinessOperatingStatus;
use crate::economy::business_economy_system::forecast_normal_business_net;
use crate::finance::{FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::OrganizationKind;

/// Public appraisal horizon. It is also the minimum period over which a staffed
/// outfit expects to exploit a newly acquired productive business before reselling.
const VALUATION_DAYS: i128 = 90;
/// A fully funded year of the post-sale wage shortfall is a viable alternative
/// to holding the same property indefinitely when the city has no listings.
pub(super) const SALE_RUNWAY_DAYS: i128 = 365;

pub(super) fn daily_net(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<i128, BusinessAcquisitionError> {
    let record = state
        .world()
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let cycle = registry
        .get_business(record.kind())
        .economics()
        .cycle()
        .as_minutes();
    let net = forecast_normal_business_net(registry, state, business)?;
    Ok(i128::from(net.cents()) * i128::from(DAY_MINUTES) / i128::from(cycle))
}

pub(super) fn negotiated_reference_price(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<i128, BusinessAcquisitionError> {
    Ok(super::appraise_business(registry, state, business)?.reference_offer_cents)
}

pub(super) fn appraisal_reference_offer(
    standard_price: Money,
    cycle_net: Money,
    cycle_minutes: u32,
) -> i128 {
    i128::from(standard_price.cents())
        + i128::from(cycle_net.cents().max(0)) * VALUATION_DAYS * i128::from(DAY_MINUTES)
            / i128::from(cycle_minutes)
}

/// Staffed criminal enterprises are ongoing employers, not passive sellers. Preserve a
/// productive earning base unless the remaining portfolio covers wages, a comparably
/// useful replacement is available, or the offer plus net liquid reserves finances an
/// entire year's post-sale wage gap. Cash-poor sellers do not liquidate their livelihood
/// just to cover a week's payroll. Idle or unprofitable properties can still be sold,
/// as can surplus holdings. During the initial valuation holding period a sale
/// requires a buyout above the normal appraisal, not merely the routine offer
/// that autonomous investors repeatedly submit. This prevents weekly churn
/// without locking every traded storefront for 90 of the next 105 days.
pub(super) fn seller_preserves_earning_base(
    registry: &Registry,
    state: &AppState,
    seller: OrganizationId,
    business: BusinessId,
    offer: Money,
) -> Result<bool, BusinessAcquisitionError> {
    let Some(organization) = state.world().get_organization(seller) else {
        return Ok(false);
    };
    let members = state.world().characters_in_organization(seller).count();
    if organization.kind() != OrganizationKind::Criminal || members == 0 {
        return Ok(false);
    }
    let record = state
        .world()
        .get_business(business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(business))?;
    let net = daily_net(registry, state, business)?;
    if net <= 0
        || state
            .economy()
            .get_business_economy(business)
            .is_none_or(|economy| economy.status() != BusinessOperatingStatus::Active)
    {
        return Ok(false);
    }
    if state
        .world()
        .get_business_ownership_change_for_version(business, record.version())
        .is_some_and(|change| {
            change.previous_owner().is_some()
                && i128::from(
                    state
                        .now()
                        .as_minutes()
                        .saturating_sub(change.changed_at().as_minutes()),
                ) < VALUATION_DAYS * i128::from(DAY_MINUTES)
        })
    {
        // A very large voluntary offer can rationally buy out a new investment.
        // Require the original standard capital plus *two* full appraisal periods
        // of expected earnings. Autonomous bidders quote only one period, so
        // letting an exceptional player offer through does not create a weekly
        // round-trip between otherwise identical rival investors. The later
        // payroll and replacement checks still protect a dependent employer.
        let standard_price = registry
            .get_business(record.kind())
            .economics()
            .acquisition_cost();
        let early_buyout = i128::from(standard_price.cents()) + 2 * VALUATION_DAYS * net;
        if i128::from(offer.cents()) < early_buyout {
            return Ok(true);
        }
    }
    let daily_payroll = i128::from(registry.upkeep().per_member_daily().cents()) * members as i128;
    let mut remaining = 0_i128;
    for asset in state.world().businesses_owned_by_organization(seller) {
        if asset.id() != business
            && state
                .economy()
                .get_business_economy(asset.id())
                .is_some_and(|e| e.status() == BusinessOperatingStatus::Active)
        {
            remaining += daily_net(registry, state, asset.id())?;
        }
    }
    // Do not protect an asset that is redundant for the seller's legitimate wage base.
    if remaining >= daily_payroll {
        return Ok(false);
    }
    // Sale proceeds are clean, but still need to leave one week of payroll uncommitted.
    // The seller's own other liquid funds can supply that reserve, not the price.
    let (mut accounted, mut other) = (i128::from(offer.cents()), 0_i128);
    let mut net_liquid_after_sale = i128::from(offer.cents());
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(seller))
    {
        // Available cash funds replacement purchases; negative liquid balances are
        // still genuine obligations when judging whether the sale leaves a secure runway.
        if account.kind().is_liquid() {
            net_liquid_after_sale += i128::from(account.balance().cents());
        }
        let balance = i128::from(account.available_payment_balance().cents());
        if account.kind() == crate::finance::AccountKind::AccountedFunds {
            accounted += balance;
        } else {
            other += balance;
        }
    }
    // A mature cash-rich organization can rationally accept a strong offer even
    // with no remaining independent listings. Without this exit, a fully-owned city
    // can freeze its property market forever. This is seller-private information;
    // the buyer still receives only the generic refusal on an unsuccessful offer.
    let post_sale_gap = daily_payroll - remaining;
    if net_liquid_after_sale >= post_sale_gap * SALE_RUNWAY_DAYS + daily_payroll * 7 {
        return Ok(false);
    }
    let available = accounted - (daily_payroll * 7 - other).max(0);
    let minimum_replacement = net.min(daily_payroll - remaining);
    for (id, _, _, _) in state.world().public_business_directory() {
        if state.economy().get_business_economy(id).is_none() {
            continue;
        }
        if let Some((price, _)) = acquisition_offer(registry, state, seller, id)
            && i128::from(price.cents()) <= available
            && daily_net(registry, state, id)? >= minimum_replacement
        {
            return Ok(false);
        }
    }
    Ok(true)
}
