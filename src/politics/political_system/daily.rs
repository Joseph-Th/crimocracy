//! Daily campaign maintenance, expenditure and rival competition.

use super::election::apply_due_mayoral_elections;
use super::{
    CampaignAction, controls_district, eligible_manager, specialist_output, validate_invest_for,
    validate_spend,
};
use crate::core::id::{NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, is_day_boundary};
use crate::finance::{AccountKind, FinancialOwner};
use crate::politics::{DistrictCampaign, INFLUENCE_CAP, INFLUENCE_COST_PER_POINT};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use crate::world::world_system::sole_available_executive;
use std::collections::{BTreeMap, BTreeSet};

/// Actual standing lost on the next daily decay pass, before paid specialist work.
/// The campaign reader uses the same arithmetic as the simulation, not a separate forecast.
pub fn daily_campaign_erosion(machine: u16, grassroots: u16, institutions: u16) -> (u16, u16, u16) {
    (
        machine.min((machine / 40).max(5)),
        grassroots.min((grassroots / 60).max(3)),
        institutions.min((institutions / 50).max(4)),
    )
}

fn apply_daily_decay(state: &mut AppState) {
    for campaign in state.politics.organizations.values_mut() {
        for district in campaign.districts.values_mut() {
            let (machine, grassroots, institutions) = daily_campaign_erosion(
                district.influence,
                district.grassroots,
                district.institutions,
            );
            district.influence -= machine;
            district.grassroots -= grassroots;
            district.institutions -= institutions;
        }
    }
}

fn pay_campaign_staff(state: &mut AppState) {
    let offices = state
        .politics()
        .organizations
        .iter()
        .flat_map(|(&owner, campaign)| {
            campaign
                .districts
                .iter()
                .map(move |(&district, office)| (owner, district, office.specialists.clone()))
        })
        .collect::<Vec<_>>();
    for (owner, district, specialists) in offices {
        let active = specialists
            .into_iter()
            .filter(|(_, manager)| eligible_manager(state, owner, district, *manager))
            .map(|(kind, manager)| (kind, specialist_output(state, manager, kind)))
            .collect::<Vec<_>>();
        let cents: i64 = active.iter().map(|(kind, _)| kind.daily_salary()).sum();
        if cents == 0 {
            continue;
        }
        // Wages are earned only if funded in full. A depleted campaign cannot
        // accrue free influence, and does not incur an unrecorded loan.
        if let Ok(order) = validate_spend(state, owner, district, cents, CampaignAction::Salary)
            && order.commit(state).is_ok()
            && let Some(office) = state
                .politics
                .organizations
                .get_mut(&owner)
                .and_then(|campaign| campaign.districts.get_mut(&district))
        {
            for (_, (machine, grassroots, institutions)) in active {
                office.influence = office.influence.saturating_add(machine).min(INFLUENCE_CAP);
                office.grassroots = office
                    .grassroots
                    .saturating_add(grassroots)
                    .min(INFLUENCE_CAP);
                office.institutions = office
                    .institutions
                    .saturating_add(institutions)
                    .min(INFLUENCE_CAP);
            }
        }
    }
}

/// Available clean investment after reserving one full week of current named
/// payroll across all liquid accounts. Rivals must not spend wages on elections,
/// and a cash-poor organization can buy a few real points rather than silently
/// forgoing political activity because it cannot afford the full weekly target.
pub(in crate::politics) fn affordable_rival_campaign_points(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
) -> u16 {
    let members = state.world().characters_in_organization(owner).count() as i128;
    let reserve = i128::from(registry.upkeep().per_member_daily().cents()) * members * 7;
    let mut clean = 0_i128;
    let mut other = 0_i128;
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
    {
        let available = i128::from(account.available_payment_balance().cents());
        if account.kind() == AccountKind::AccountedFunds {
            clean += available;
        } else if account.kind().is_liquid() {
            // A legitimate operating pocket owned by the gang can fund wages too.
            // Only accounted funds can buy electoral influence, but every other
            // spendable kind reduces how much clean liquidity must be reserved.
            other += available;
        }
    }
    let spendable = (clean - (reserve - other).max(0)).max(0);
    u16::try_from((spendable / i128::from(INFLUENCE_COST_PER_POINT)).min(220))
        .expect("campaign points are bounded at 220")
}

fn autonomous_rival_campaigns(registry: &Registry, state: &mut AppState) {
    // Weekly initiative: defend an *operating* commercial stake against an
    // actual player challenge. Do not burn scarce rival capital on empty wards;
    // a large public player portfolio can also induce a modest preemptive defense
    // in wards where that organization owns several active businesses.
    if !(state.now().as_minutes() / DAY_MINUTES).is_multiple_of(7) {
        return;
    }
    let Some(player) = state.player_organization() else {
        return;
    };
    let player_prominence =
        crate::world::organization_pressure::public_prominence_tier(state, player);
    let player_properties_by_district = state
        .world()
        .businesses_owned_by_organization(player)
        .filter(|business| {
            state
                .economy()
                .get_business_economy(business.id())
                .is_some_and(|economy| {
                    economy.status() == crate::economy::BusinessOperatingStatus::Active
                })
        })
        .fold(
            BTreeMap::<NeighborhoodId, usize>::new(),
            |mut counts, business| {
                *counts.entry(business.neighborhood()).or_default() += 1;
                counts
            },
        );
    // Take a small value snapshot before rival ledgers are mutated below.
    let player_pressure: BTreeMap<_, _> = state
        .politics()
        .organization(player)
        .map(|campaign| {
            campaign
                .districts
                .iter()
                .map(|(&district, office)| {
                    (
                        district,
                        (
                            office.influence(),
                            office.grassroots(),
                            office.institutions(),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let rivals = state
        .world()
        .organizations()
        .filter(|organization| {
            organization.kind() == OrganizationKind::Criminal && organization.id() != player
        })
        .map(|organization| organization.id())
        .collect::<Vec<_>>();
    for owner in rivals {
        // A surviving commercial portfolio or treasury does not exercise new
        // executive authority. Like autonomous acquisitions, counter-campaigns
        // require a sole available executive to commission spending.
        if sole_available_executive(state, owner).is_none() {
            continue;
        }
        let affordable = affordable_rival_campaign_points(registry, state, owner);
        if affordable == 0 {
            continue;
        }
        let districts = state
            .world()
            .businesses_owned_by_organization(owner)
            .filter(|business| {
                state
                    .economy()
                    .get_business_economy(business.id())
                    .is_some_and(|economy| {
                        economy.status() == crate::economy::BusinessOperatingStatus::Active
                    })
            })
            .map(|business| business.neighborhood())
            .collect::<BTreeSet<_>>();
        let choice = districts
            .into_iter()
            .filter_map(|district| {
                // A fully secured ward cannot accept more investment. Exclude it
                // before ranking, otherwise an intense but already capped player
                // challenge can indefinitely starve another operating ward.
                let own = state
                    .politics()
                    .organization(owner)
                    .and_then(|campaign| campaign.district(district))
                    .map_or(0, DistrictCampaign::influence);
                if own == INFLUENCE_CAP {
                    return None;
                }
                let (machine, grassroots, institutions) =
                    player_pressure.get(&district).copied().unwrap_or((0, 0, 0));
                let preempt = player_prominence >= 3
                    && player_properties_by_district
                        .get(&district)
                        .is_some_and(|&owned| owned >= 3);
                if machine == 0 && grassroots == 0 && institutions == 0 && !preempt {
                    return None;
                }
                // A rational adversary defends a lead instead of wasting
                // money indefinitely once its current position is sufficient.
                let desired = (u32::from(machine) * 3 / 4 + 150 + u32::from(player_prominence) * 30)
                    .min(u32::from(INFLUENCE_CAP)) as u16;
                let deficit = desired.checked_sub(own).filter(|&points| points > 0)?;
                Some((
                    // A player presence that our machine already exceeds
                    // is not an urgent challenge merely because its
                    // historical office has nonzero support.
                    machine > own,
                    deficit,
                    u32::from(grassroots) + u32::from(institutions),
                    std::cmp::Reverse(district),
                ))
            })
            .max();
        let Some((_, deficit, _, std::cmp::Reverse(district))) = choice else {
            continue;
        };
        // Spend against the same deficit used to select the ward. The desired
        // threshold was already bounded by the local influence cap.
        let weekly_budget_points = (100 + u16::from(player_prominence) * 20).min(220);
        let points = deficit.min(weekly_budget_points).min(affordable);
        if let Ok(order) = validate_invest_for(
            state,
            owner,
            district,
            i64::from(points) * INFLUENCE_COST_PER_POINT,
        ) {
            let _ = order.commit(state);
        }
    }
}

/// After same-day payroll and commerce, campaigns pay workers, suffer ordinary
/// attention decay, and face rival political spending. The last step records
/// continuously defended majority tenure; one losing day resets the clock.
pub fn apply_due_political_campaigns(registry: &Registry, state: &mut AppState) {
    if !is_day_boundary(state.now()) {
        return;
    }
    apply_daily_decay(state);
    pay_campaign_staff(state);
    autonomous_rival_campaigns(registry, state);
    let owners = state
        .politics()
        .organizations
        .iter()
        .flat_map(|(&owner, campaign)| {
            campaign
                .districts
                .keys()
                .copied()
                .map(move |district| (owner, district))
        })
        .collect::<Vec<_>>();
    for (owner, district) in owners {
        let leading = controls_district(state, owner, district);
        let now = state.now();
        if let Some(office) = state
            .politics
            .organizations
            .get_mut(&owner)
            .and_then(|campaign| campaign.districts.get_mut(&district))
        {
            if leading {
                office.leading_since.get_or_insert(now);
            } else {
                office.leading_since = None;
            }
        }
    }
    apply_due_mayoral_elections(registry, state);
}
