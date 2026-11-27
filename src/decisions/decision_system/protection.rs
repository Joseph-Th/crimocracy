//! Rival protection demands are durable player decisions, not free passive damage.
//! A local gang with more operating rank-and-file force than a shop owner's muscle
//! can demand a 30-day payment. Rejection leaves the normal scout/sabotage campaign
//! available; payment is a balanced transfer and suspends that gang's coercion.
//! Only one unresolved demand per ward exists, and all historical cooldowns are
//! derived from the canonical decision ledger rather than separate mutable state.

use super::{DecisionError, DecisionRequestOutcome, insert_pending_decision_request};
use crate::core::attention::AttentionClass;
use crate::core::id::{BusinessId, DecisionRequestId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, is_day_boundary};
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionRequestDraft, DecisionResponse,
    DecisionStatus, ProtectionDemandContext, build_cancellation,
};
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::helpers::format_money_cents;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::registry::Registry;
use crate::world::territory_conflict::{
    DEFENSIVE_ADVANTAGE, active_district_force, local_defensive_forces,
};
use crate::world::territory_influence::active_standstill;
use crate::world::{BusinessOwner, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const PROTECTION_DAYS: u64 = 30;
const OFFER_VALIDITY_DAYS: u64 = 30;
const REFUSAL_COOLDOWN_DAYS: u64 = 7;
const NEGOTIATION_GRACE_DAYS: u64 = 1;

fn within_days(now: u64, at: u64, days: u64) -> bool {
    now.saturating_sub(at) < days * DAY_MINUTES
}

pub(crate) fn protection_offer_expired(requested_at: SimTime, now: SimTime) -> bool {
    !within_days(
        now.as_minutes(),
        requested_at.as_minutes(),
        OFFER_VALIDITY_DAYS,
    )
}

fn expire_unanswered_offers(state: &mut AppState, defender: OrganizationId) {
    let now = state.now();
    let expired: Vec<_> = state
        .decisions()
        .pending_for_recipient(defender)
        .filter(|record| {
            matches!(record.context(), DecisionContext::ProtectionDemand(_))
                && protection_offer_expired(record.requested_at(), now)
        })
        .map(|record| record.id())
        .collect();
    for decision in expired {
        state.decisions.cancel(
            decision,
            build_cancellation(now, DecisionCancellationReason::ProtectionOfferExpired),
        );
    }
}

/// The demanding gang holds its crews for one day while leadership considers
/// the offer, not indefinitely. Ignoring an unpaid demand must not become free
/// protection in adapters that can advance time with pending decisions.
/// Once paid, that gang respects the negotiated period; other rivals do not.
pub(crate) fn rival_pressure_blocked(
    state: &AppState,
    attacker: OrganizationId,
    district: NeighborhoodId,
) -> bool {
    state
        .decisions()
        .latest_protection_demand(district, attacker)
        .is_some_and(|record| {
            (record.status() == DecisionStatus::Pending
                && within_days(
                    state.now().as_minutes(),
                    record.requested_at().as_minutes(),
                    NEGOTIATION_GRACE_DAYS,
                ))
                || record.resolution().is_some_and(|resolution| {
                    resolution.response() == DecisionResponse::Approve
                        && within_days(
                            state.now().as_minutes(),
                            resolution.resolved_at().as_minutes(),
                            PROTECTION_DAYS,
                        )
                })
        })
}

/// When the target rejects an offer, the same gang has a concrete reason
/// to prioritize its real reconnaissance and sabotage over other prospects.
/// This does not create a strike without trained personnel or a valid plan.
pub(crate) fn recently_refused_protection(
    state: &AppState,
    attacker: OrganizationId,
    district: NeighborhoodId,
) -> bool {
    state
        .decisions()
        .latest_protection_demand(district, attacker)
        .is_some_and(|record| {
            record.resolution().is_some_and(|resolution| {
                resolution.response() == DecisionResponse::Reject
                    && within_days(
                        state.now().as_minutes(),
                        resolution.resolved_at().as_minutes(),
                        PROTECTION_DAYS,
                    )
            })
        })
}

/// An owned commercial ward with no believable defense is a claimable
/// protection market. The estimate uses public ordinary commerce, not private
/// customer books, and excludes suspended stores.
fn active_shops(
    state: &AppState,
    defender: OrganizationId,
) -> BTreeMap<NeighborhoodId, Vec<BusinessId>> {
    let mut by_district = BTreeMap::new();
    for business in state.world().businesses_owned_by_organization(defender) {
        if state
            .economy()
            .get_business_economy(business.id())
            .is_some_and(|economy| economy.is_operating())
        {
            by_district
                .entry(business.neighborhood())
                .or_insert_with(Vec::new)
                .push(business.id());
        }
    }
    by_district
}

fn offer_cents(registry: &Registry, state: &AppState, shops: &[BusinessId]) -> i64 {
    let daily_gross: i128 = shops
        .iter()
        .map(|business| {
            resolve_business_gross_potential(registry, state, *business)
                .map_or(0, |gross| i128::from(gross.cents().max(0)))
        })
        .sum();
    // Five percent of a public 30-day gross, not five percent of a
    // business's protected working capital or its private operating till.
    let monthly = daily_gross.saturating_mul(30) / 20;
    i64::try_from(monthly.clamp(5_000, 1_500_000)).expect("bounded demand")
}

/// Share the live coercive basis between offer creation and acceptance.
/// An old request cannot authorize a payment after the rival loses local force,
/// the defender becomes strong enough, or the parties agree to a standstill.
fn coercive_requester(
    state: &AppState,
    attacker: OrganizationId,
    defender: OrganizationId,
    district: NeighborhoodId,
    defense: u32,
) -> Option<(crate::core::id::CharacterId, u32)> {
    if active_standstill(state, attacker, district)
        .is_some_and(|treaty| treaty.counterparty == defender)
    {
        return None;
    }
    let force = active_district_force(state, attacker, district);
    if force <= defense.saturating_add(DEFENSIVE_ADVANTAGE) {
        return None;
    }
    let requester = state
        .enterprises()
        .active_for_organization(attacker)
        .filter(|racket| racket.location().neighborhood(state.world()) == Some(district))
        .filter(|racket| {
            state
                .legal()
                .active_arrest_for_character(racket.manager())
                .is_none()
        })
        .max_by_key(|racket| (racket.associates(), std::cmp::Reverse(racket.id())))
        .map(|racket| racket.manager())?;
    Some((requester, force))
}

struct Candidate {
    district: NeighborhoodId,
    attacker: OrganizationId,
    requester: crate::core::id::CharacterId,
    business: BusinessId,
    amount: Money,
    force: u32,
    defense: u32,
}

/// Deterministic local coercion: strongest actual rival foot soldiers in the ward
/// make one demand at a time. A rival must still have an available named manager
/// who owns the racket, so ownerless phantom gangs cannot charge fees.
pub(crate) fn apply_due_protection_demands(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<DecisionRequestOutcome> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let Some(defender) = state.player_organization() else {
        return Vec::new();
    };
    expire_unanswered_offers(state, defender);
    let shops = active_shops(state, defender);
    if shops.is_empty() {
        return Vec::new();
    }
    let defenses = local_defensive_forces(state, defender);
    // Even the original two storefronts become vulnerable if their criminal
    // workforce leaves and a rival actually takes a stronger local position.
    // Legitimate ownership alone is never proof of defensible territory.
    let mut candidates = Vec::new();
    for (district, holdings) in shops {
        if state
            .decisions()
            .pending_protection_demand(district)
            .is_some()
        {
            continue;
        }
        let defense = defenses.get(&district).copied().unwrap_or_default().total();
        let amount = Money::from_cents(offer_cents(registry, state, &holdings));
        let Some(business) = holdings.first().copied() else {
            continue;
        };
        for rival in state.world().organizations() {
            if rival.id() == defender || rival.kind() != OrganizationKind::Criminal {
                continue;
            }
            if let Some(previous) = state
                .decisions()
                .latest_protection_demand(district, rival.id())
            {
                let (since, interval) = match previous.resolution() {
                    Some(resolution) if resolution.response() == DecisionResponse::Approve => {
                        (resolution.resolved_at().as_minutes(), PROTECTION_DAYS)
                    }
                    Some(resolution) => {
                        (resolution.resolved_at().as_minutes(), REFUSAL_COOLDOWN_DAYS)
                    }
                    None => (previous.requested_at().as_minutes(), OFFER_VALIDITY_DAYS),
                };
                if within_days(state.now().as_minutes(), since, interval) {
                    continue;
                }
            }
            if let Some((requester, force)) =
                coercive_requester(state, rival.id(), defender, district, defense)
            {
                candidates.push(Candidate {
                    district,
                    attacker: rival.id(),
                    requester,
                    business,
                    amount,
                    force,
                    defense,
                });
            }
        }
    }
    // One demand citywide per day avoids turning an exposed commercial portfolio
    // into a simultaneous screenful of unrelated modal choices.
    candidates.sort_unstable_by_key(|offer| {
        (
            std::cmp::Reverse(offer.force),
            offer.district,
            offer.attacker,
        )
    });
    let Some(offer) = candidates.into_iter().next() else {
        return Vec::new();
    };
    let rival = state
        .world()
        .get_organization(offer.attacker)
        .expect("known gang");
    let ward = state
        .world()
        .get_neighborhood(offer.district)
        .expect("known district");
    let draft = DecisionRequestDraft {
        requester: offer.requester,
        context: DecisionContext::ProtectionDemand(ProtectionDemandContext::new(
            offer.attacker,
            offer.district,
            offer.business,
            offer.amount,
        )),
        attention: AttentionClass::Exception,
        summary: format!(
            "{} has established a local street presence and demands {} for 30 days of protection over your businesses in {}. You can currently muster {} local defenders across rackets and available muscle. Approve to pay from accounted funds, or reject and face attempts to displace your commercial foothold. The rival waits one day; the unpaid offer expires after 30 days.",
            rival.name(),
            format_money_cents(offer.amount.cents()),
            ward.name(),
            offer.defense,
        ),
    };
    if state
        .ids
        .reserve_many(&[(crate::core::id::IdKind::DecisionRequest, 1)])
        .is_err()
    {
        return Vec::new();
    }
    let outcome = insert_pending_decision_request(
        state,
        defender,
        draft,
        BTreeSet::from([DecisionResponse::Approve, DecisionResponse::Reject]),
    )
    .expect("preflighted protection demand must have a decision ID");
    vec![outcome]
}

/// Live territorial and commercial facts can change without advancing the
/// decision's own version. Refusal remains possible even when an offer is stale.
pub(super) fn validate_protection_terms(
    state: &AppState,
    defender: OrganizationId,
    context: ProtectionDemandContext,
) -> Result<(), DecisionError> {
    let shop = state
        .world()
        .get_business(context.business())
        .ok_or(DecisionError::MissingProtectionBusiness(context.business()))?;
    if shop.owner() != BusinessOwner::Organization(defender)
        || shop.neighborhood() != context.district()
    {
        return Err(DecisionError::ProtectionNoLongerOwned(context.business()));
    }
    let defense = local_defensive_forces(state, defender)
        .get(&context.district())
        .copied()
        .unwrap_or_default();
    if defense.operating_shops == 0
        || coercive_requester(
            state,
            context.attacker(),
            defender,
            context.district(),
            defense.total(),
        )
        .is_none()
    {
        return Err(DecisionError::ProtectionThreatEnded {
            district: context.district(),
            attacker: context.attacker(),
        });
    }
    Ok(())
}

/// A protection payment is an actual organizational transfer. It cannot draw
/// from protected tills or clearing balances and it cannot mint funds if short.
pub(super) fn validate_protection_payment(
    state: &AppState,
    decision: DecisionRequestId,
    defender: OrganizationId,
    context: ProtectionDemandContext,
) -> Result<ValidatedLedgerTransaction, DecisionError> {
    validate_protection_terms(state, defender, context)?;
    let mut postings = crate::finance::helpers::payment_debits(
        state
            .finance()
            .accounts_for(FinancialOwner::Organization(defender))
            .filter(|account| account.kind() == AccountKind::AccountedFunds),
        context.amount(),
    )
    .map_err(|missing| DecisionError::InsufficientProtectionFunds {
        needed: context.amount().cents(),
        missing: missing.cents(),
    })?;
    let owner = FinancialOwner::Organization(context.attacker());
    let recipient = state
        .finance()
        .accounts_for(owner)
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && account.can_receive_liquid_payment(context.amount())
        })
        .map(|account| account.id());
    let draft = |destination| {
        postings.push(LedgerPosting {
            account: destination,
            amount: context.amount(),
        });
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!("Protection payment for decision {}", decision.raw()),
            postings,
            authorization: None,
        }
    };
    if let Some(destination) = recipient {
        Ok(validate_record_transaction(state, draft(destination))?)
    } else {
        let openings = validate_open_accounts(
            state,
            vec![FinancialAccountDraft {
                owner,
                kind: AccountKind::StreetCash,
            }],
        )?;
        let destination = openings
            .account_id(0)
            .expect("one protection receipt account");
        Ok(validate_record_transaction_with_openings(
            state,
            openings,
            draft(destination),
        )?)
    }
}

#[cfg(test)]
mod tests;
