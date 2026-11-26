//! Seller-side portfolio decisions. A buyer can quote a public estimate, but may not
//! inspect the seller's private cash, roster, or infrastructure dependencies.

use super::portfolio_forecast::PortfolioForecast;
use super::{BusinessAcquisitionError, acquisition_offer, appraise_business, buyer_adjusted_price};
use crate::core::id::{BusinessId, CharacterId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::economy::business_economy_system::{
    daily_owner_net, forecast_live_business_net_after_sale, forecast_normal_business_net,
};
use crate::finance::helpers::TreasuryLiquidity;
use crate::finance::{FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::{BusinessRecord, OrganizationKind, TraitKind};

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

/// A private proprietor knows the productive condition of their own holdings.
/// Closed, terminal and loss-making storefronts cannot replace a livelihood.
fn proprietor_daily_income(
    registry: &Registry,
    state: &AppState,
    business: &BusinessRecord,
    sold: Option<BusinessId>,
) -> Result<i128, BusinessAcquisitionError> {
    if sold == Some(business.id())
        || state
            .economy()
            .get_business_economy(business.id())
            .is_none_or(|economy| !economy.is_operating())
    {
        return Ok(0);
    }
    let cycle = registry
        .get_business(business.kind())
        .economics()
        .cycle()
        .as_minutes();
    let net = forecast_live_business_net_after_sale(registry, state, business.id(), sold)?;
    Ok(daily_owner_net(net, cycle, 0).max(0))
}

/// A registered person's choice to part with an actual livelihood. An owner
/// losing their last productive storefront through a sale requires another
/// appraisal period of their current forecast earnings. The retained alternative
/// must remain profitable after the transferred storefront reopens. Public quotes
/// remain unchanged; a buyer cannot inspect this private reservation price.
pub(super) fn personal_seller_minimum(
    registry: &Registry,
    state: &AppState,
    seller: CharacterId,
    business: BusinessId,
) -> Result<i128, BusinessAcquisitionError> {
    let person = state
        .world()
        .get_character(seller)
        .ok_or(BusinessAcquisitionError::OfferDeclined)?;
    if state.legal().active_arrest_for_character(seller).is_some() {
        return Err(BusinessAcquisitionError::OfferDeclined);
    }
    let base = negotiated_reference_price(registry, state, business)?;
    let mut current_income = 0_i128;
    let mut retains_livelihood = false;
    for shop in state.world().businesses_owned_by_character(seller) {
        if proprietor_daily_income(registry, state, shop, Some(business))? > 0 {
            retains_livelihood = true;
            break;
        }
        current_income += proprietor_daily_income(registry, state, shop, None)?;
    }
    let extra_income = if retains_livelihood {
        0
    } else {
        current_income * VALUATION_DAYS
    };
    let standing = i128::from(person.life().professional_standing());
    let fatigue = i128::from(person.life().fatigue());
    let stubbornness = if person.has_trait(TraitKind::Greedy) {
        18
    } else if person.has_trait(TraitKind::Proud) {
        12
    } else if person.has_trait(TraitKind::Cautious) {
        8
    } else if person.has_trait(TraitKind::Impulsive) {
        -8
    } else {
        0
    };
    let multiplier = (90 + standing / 4 - fatigue / 3 + stubbornness).clamp(65, 150);
    Ok(base.saturating_add(extra_income).saturating_mul(multiplier) / 100)
}

/// Staffed criminal enterprises are ongoing employers, not passive sellers. Preserve a
/// productive earning base unless the remaining portfolio covers wages, a comparably
/// useful replacement is available, or the offer plus net liquid reserves finances an
/// entire year's post-sale wage gap. Cash-poor sellers do not liquidate their livelihood
/// just to cover a week's payroll. Idle or unprofitable properties can still be sold
/// when reopening them under a new owner will not compromise the retained wage base,
/// as can surplus holdings. During the initial valuation holding period a productive sale
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
    let operating = state
        .economy()
        .get_business_economy(business)
        .is_some_and(|economy| economy.is_operating());
    let current = PortfolioForecast::new(registry, state, seller, None)?;
    let retained = PortfolioForecast::new(registry, state, seller, Some(business))?;
    let remaining = retained.daily_net();
    let lost_income = current.daily_net() - remaining;
    // A sale can reduce administration on the retained holdings enough to improve income.
    // Even a closed property can protect the seller's wage base by remaining
    // out of the market. Transfer restarts it under the new owner.
    if lost_income <= 0 {
        return Ok(false);
    }
    if operating
        && state
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
        let net = daily_net(registry, state, business)?.max(0);
        let early_buyout = i128::from(standard_price.cents()) + 2 * VALUATION_DAYS * net;
        if i128::from(offer.cents()) < early_buyout {
            return Ok(true);
        }
    }
    let daily_payroll = i128::from(registry.upkeep().per_member_daily().cents()) * members as i128;
    if remaining >= daily_payroll {
        return Ok(false);
    }
    // Sale proceeds are clean, but still need to leave one week of payroll uncommitted.
    // The seller's own other liquid funds can supply that reserve, not the price.
    let treasury = TreasuryLiquidity::from_accounts(
        state
            .finance()
            .accounts_for(FinancialOwner::Organization(seller)),
    )
    .with_accounted_receipt(offer);
    // A mature cash-rich organization can rationally accept a strong offer even
    // with no remaining independent listings. Without this exit, a fully-owned city
    // can freeze its property market forever. This is seller-private information;
    // the buyer still receives only the generic refusal on an unsuccessful offer.
    let post_sale_gap = daily_payroll - remaining;
    if treasury.net_liquid() >= post_sale_gap * SALE_RUNWAY_DAYS + daily_payroll * 7 {
        return Ok(false);
    }
    let available = treasury.accounted_after_reserve(daily_payroll * 7);
    let minimum_replacement = lost_income.min(daily_payroll - remaining);
    for (id, _, _, district) in state.world().public_business_directory() {
        if state
            .economy()
            .get_business_economy(id)
            .is_none_or(|economy| economy.version() >= u32::MAX - 1)
            || acquisition_offer(registry, state, seller, id).is_none()
        {
            continue;
        }
        let appraisal = appraise_business(registry, state, id)?;
        let price = buyer_adjusted_price(
            i128::from(appraisal.standard_price.cents()),
            retained.administration.property_count(),
        )?;
        if i128::from(price.cents()) <= available
            && retained.purchase_gain(registry, state, id, district, &appraisal)?
                >= minimum_replacement
        {
            return Ok(false);
        }
    }
    Ok(true)
}
