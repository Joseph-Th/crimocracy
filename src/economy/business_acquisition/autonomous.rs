//! Non-player commercial investment. Executives buy from the same market as the player.
//! Purchases are separate transactions, not one city-wide atomic batch.

use super::{BusinessAcquisitionDraft, acquisition_offer, validate_acquire_business};
use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimDuration, is_recurring_boundary};
use crate::economy::business_economy_system::forecast_normal_business_net;
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

const REVIEW_DAYS: u32 = 7;

/// Every week, each free non-player criminal executive may make one investment in a
/// neighborhood where the organization already owns property. Rank by estimated return
/// per unit of capital and time, not seller-side private balances. Keep at least one
/// week's payroll in accounted funds after purchase. Other organizations see only the
/// remaining listings. The canonical purchase owns all transfer, ledger and report effects.
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
    let districts: BTreeSet<_> = state
        .world
        .businesses_owned_by_organization(organization)
        .map(|business| business.neighborhood())
        .collect();
    let reserve = i128::from(registry.upkeep().per_member_daily().cents())
        * members.len() as i128
        * i128::from(REVIEW_DAYS);
    let accounts: Vec<_> = state
        .finance
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| {
            account.kind() == AccountKind::AccountedFunds
                && account.spendable_balance() > Money::ZERO
                && account.version() < u32::MAX
        })
        .collect();
    let available = accounts
        .iter()
        .map(|account| i128::from(account.spendable_balance().cents()))
        .sum::<i128>();
    let budget = available - reserve;
    if budget <= 0 {
        return None;
    }
    let funding_accounts: BTreeSet<_> = accounts.iter().map(|account| account.id()).collect();
    let mut candidates = Vec::new();
    for district in districts {
        for business in state.world.businesses_in_neighborhood(district) {
            let id = business.id();
            // Invest in functioning market inventory, not unmodeled empty premises.
            if state.economy.get_business_economy(id).is_none() {
                continue;
            }
            let Some((price, _)) = acquisition_offer(registry, state, organization, id) else {
                continue;
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
            let cycle = registry
                .get_business(business.kind())
                .economics()
                .cycle()
                .as_minutes();
            // Scaled return per minute; bounded i128 arithmetic even at extreme registry values.
            let return_score = i128::from(net.cents()) * 1_000_000_000_000_i128
                / i128::from(price.cents())
                / i128::from(cycle);
            candidates.push((std::cmp::Reverse(return_score), price, id));
        }
    }
    candidates.sort_unstable();
    for (_, _, business) in candidates {
        // Inability to complete a purchase (including exhausted IDs or account capacity) is
        // a normal market constraint. Canonical validation and commit leave state unchanged
        // on rejection; try the next affordable listing instead of panicking in a tick.
        let draft = BusinessAcquisitionDraft {
            organization,
            business,
            funding_accounts: funding_accounts.clone(),
        };
        if let Ok(purchase) = validate_acquire_business(registry, state, draft)
            && purchase.commit(registry, state).is_ok()
        {
            return Some(business);
        }
    }
    None
}
