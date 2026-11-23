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
use crate::core::time::{DAY_MINUTES, is_day_boundary};
use crate::decisions::{
    DecisionContext, DecisionRequestDraft, DecisionResponse, DecisionStatus,
    ProtectionDemandContext,
};
use crate::economy::BusinessOperatingStatus;
use crate::economy::business_economy_system::resolve_business_gross_potential;
use crate::finance::finance_system::{ValidatedLedgerTransaction, validate_record_transaction};
use crate::finance::helpers::format_money_cents;
use crate::finance::{AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money};
use crate::registry::Registry;
use crate::world::territory_conflict::{
    DEFENSIVE_ADVANTAGE, active_district_force, district_of_racket, local_defensive_force,
};
use crate::world::territory_influence::active_standstill;
use crate::world::{BusinessOwner, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const PROTECTION_DAYS: u64 = 30;
const REFUSAL_COOLDOWN_DAYS: u64 = 7;
const NEGOTIATION_GRACE_DAYS: u64 = 1;

/// Per-ward cooldowns avoid overlapping demands. Per-gang histories preserve
/// prior retaliation and paid agreements when another rival demands money.
fn latest_demand(
    state: &AppState,
    district: NeighborhoodId,
    attacker: Option<OrganizationId>,
) -> Option<&crate::decisions::DecisionRequestRecord> {
    state
        .decisions()
        .decisions()
        .filter(|record| {
            matches!(record.context(), DecisionContext::ProtectionDemand(context)
                if context.district() == district
                    && attacker.is_none_or(|gang| context.attacker() == gang))
        })
        .max_by_key(|record| (record.requested_at(), record.id()))
}

fn within_days(now: u64, at: u64, days: u64) -> bool {
    now.saturating_sub(at) < days * DAY_MINUTES
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
    latest_demand(state, district, Some(attacker)).is_some_and(|record| {
        let DecisionContext::ProtectionDemand(context) = record.context() else {
            return false;
        };
        context.attacker() == attacker
            && ((record.status() == DecisionStatus::Pending
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
                }))
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
    latest_demand(state, district, Some(attacker)).is_some_and(|record| {
        matches!(record.context(), DecisionContext::ProtectionDemand(context)
            if context.attacker() == attacker)
            && record.resolution().is_some_and(|resolution| {
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
            .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
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

struct Candidate {
    district: NeighborhoodId,
    attacker: OrganizationId,
    requester: crate::core::id::CharacterId,
    business: BusinessId,
    amount: Money,
    force: u32,
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
    let shops = active_shops(state, defender);
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
        if let Some(previous) = latest_demand(state, district, None) {
            let (since, interval) = match previous.resolution() {
                Some(resolution) if resolution.response() == DecisionResponse::Approve => {
                    (resolution.resolved_at().as_minutes(), PROTECTION_DAYS)
                }
                Some(resolution) => (resolution.resolved_at().as_minutes(), REFUSAL_COOLDOWN_DAYS),
                None => (previous.requested_at().as_minutes(), PROTECTION_DAYS),
            };
            if within_days(state.now().as_minutes(), since, interval) {
                continue;
            }
        }
        let defense = local_defensive_force(state, defender, district);
        let amount = Money::from_cents(offer_cents(registry, state, &holdings));
        let Some(business) = holdings.first().copied() else {
            continue;
        };
        for rival in state.world().organizations() {
            if rival.id() == defender || rival.kind() != OrganizationKind::Criminal {
                continue;
            }
            if active_standstill(state, rival.id(), district)
                .is_some_and(|treaty| treaty.counterparty == defender)
            {
                continue;
            }
            let force = active_district_force(state, rival.id(), district);
            if force <= defense.saturating_add(DEFENSIVE_ADVANTAGE) {
                continue;
            }
            let requester = state
                .enterprises()
                .active_for_organization(rival.id())
                .filter(|racket| district_of_racket(state, racket.location()) == Some(district))
                .filter(|racket| {
                    state
                        .legal()
                        .active_arrest_for_character(racket.manager())
                        .is_none()
                })
                .max_by_key(|racket| (racket.associates(), std::cmp::Reverse(racket.id())))
                .map(|racket| racket.manager());
            if let Some(requester) = requester {
                candidates.push(Candidate {
                    district,
                    attacker: rival.id(),
                    requester,
                    business,
                    amount,
                    force,
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
            "{} has established a local street presence and demands {} for 30 days of protection over your businesses in {}. You can currently muster {} local defenders across rackets and available muscle. Approve to pay from accounted funds, or reject and face attempts to displace your commercial foothold.",
            rival.name(),
            format_money_cents(offer.amount.cents()),
            ward.name(),
            local_defensive_force(state, defender, offer.district),
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

/// A protection payment is an actual organizational transfer. It cannot draw
/// from protected tills or clearing balances and it cannot mint funds if short.
pub(super) fn validate_protection_payment(
    state: &AppState,
    decision: DecisionRequestId,
    defender: OrganizationId,
    context: ProtectionDemandContext,
) -> Result<ValidatedLedgerTransaction, DecisionError> {
    let shop = state
        .world()
        .get_business(context.business())
        .ok_or(DecisionError::MissingProtectionBusiness(context.business()))?;
    if shop.owner() != BusinessOwner::Organization(defender)
        || shop.neighborhood() != context.district()
    {
        return Err(DecisionError::ProtectionNoLongerOwned(context.business()));
    }
    let mut remaining = context.amount().cents();
    let mut postings = Vec::new();
    for account in state
        .finance()
        .accounts_for(FinancialOwner::Organization(defender))
    {
        if account.kind() != AccountKind::AccountedFunds {
            continue;
        }
        let taken = remaining.min(account.available_payment_balance().cents());
        if taken > 0 {
            postings.push(LedgerPosting {
                account: account.id(),
                amount: Money::from_cents(-taken),
            });
            remaining -= taken;
        }
        if remaining == 0 {
            break;
        }
    }
    if remaining > 0 {
        return Err(DecisionError::InsufficientProtectionFunds {
            needed: context.amount().cents(),
            missing: remaining,
        });
    }
    let recipient = state
        .finance()
        .accounts_for(FinancialOwner::Organization(context.attacker()))
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && account.version() < u32::MAX
                && account.balance().checked_add(context.amount()).is_some()
        })
        .ok_or(DecisionError::MissingProtectionRecipient(
            context.attacker(),
        ))?;
    postings.push(LedgerPosting {
        account: recipient.id(),
        amount: context.amount(),
    });
    Ok(validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!("Protection payment for decision {}", decision.raw()),
            postings,
            authorization: None,
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city_generation::{CityScale, GeneratedCity, generate_city};
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::simulation::run_test_tick;
    use crate::core::time::SimDuration;
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::validate_establish_business_economy;
    use crate::finance::finance_system::insert_account;
    use crate::world::world_system::{insert_business, insert_character};
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, CapabilityKind,
        CharacterDraft, Rating,
    };

    fn exposed_city() -> (Registry, GeneratedCity, NeighborhoodId) {
        let registry = crate::build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(82_734_113)).unwrap();
        let rival = city
            .rivals
            .iter()
            .copied()
            .find(|organization| {
                city.state
                    .enterprises()
                    .active_for_organization(*organization)
                    .any(|racket| u32::from(racket.associates()) > 4)
            })
            .expect("generated city should have an established rival");
        let district = city
            .state
            .enterprises()
            .active_for_organization(rival)
            .find_map(|racket| {
                (u32::from(racket.associates()) > 4)
                    .then(|| district_of_racket(&city.state, racket.location()))
                    .flatten()
            })
            .unwrap();
        // Active, genuinely owned commerce, not just bare venue records. Its
        // operating settlement and protected till go through the production owner.
        for index in 0..3 {
            let business = insert_business(
                &registry,
                &mut city.state,
                BusinessDraft {
                    name: format!("Exposed Commercial Shop {index}"),
                    kind: BusinessKind::Retail,
                    functions: BTreeSet::from([
                        BusinessFunction::CustomerAccess,
                        BusinessFunction::ProfessionalRecords,
                    ]),
                    neighborhood: district,
                    owner: BusinessOwner::Organization(city.player),
                },
            )
            .unwrap();
            let operating_account = insert_account(
                &mut city.state,
                crate::finance::FinancialAccountDraft {
                    owner: FinancialOwner::Business(business),
                    kind: AccountKind::LegitimateOperating,
                },
            )
            .unwrap();
            let settlement_account = insert_account(
                &mut city.state,
                crate::finance::FinancialAccountDraft {
                    owner: FinancialOwner::Business(business),
                    kind: AccountKind::Settlement,
                },
            )
            .unwrap();
            validate_establish_business_economy(
                &registry,
                &city.state,
                BusinessEconomyDraft {
                    business,
                    operating_account,
                    settlement_account,
                },
            )
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        }
        (registry, city, district)
    }

    fn first_demand(
        registry: &Registry,
        city: &mut GeneratedCity,
    ) -> (DecisionRequestId, ProtectionDemandContext) {
        city.state.advance_clock(SimDuration::from_minutes(
            u32::try_from(DAY_MINUTES - 1).unwrap(),
        ));
        let tick = run_test_tick(registry, &mut city.state);
        let request = city
            .state
            .decisions()
            .pending_for_recipient(city.player)
            .find(|record| matches!(record.context(), DecisionContext::ProtectionDemand(_)))
            .expect("exposed business portfolio must receive a rival protection demand");
        assert!(
            tick.decision_requests
                .iter()
                .any(|outcome| outcome.decision == request.id())
        );
        let DecisionContext::ProtectionDemand(context) = request.context() else {
            unreachable!()
        };
        (request.id(), context)
    }

    #[test]
    fn unanswered_protection_demand_only_suspends_rival_for_one_day() {
        let (registry, mut city, district) = exposed_city();
        let (request, terms) = first_demand(&registry, &mut city);
        assert_eq!(terms.district(), district);
        assert!(rival_pressure_blocked(
            &city.state,
            terms.attacker(),
            district
        ));
        city.state.advance_clock(SimDuration::from_minutes(
            u32::try_from(DAY_MINUTES - 1).unwrap(),
        ));
        assert!(rival_pressure_blocked(
            &city.state,
            terms.attacker(),
            district
        ));
        city.state.advance_clock(SimDuration::ONE_MINUTE);
        assert!(
            !rival_pressure_blocked(&city.state, terms.attacker(), district),
            "leaving a demand unanswered must not buy indefinite protection"
        );
        assert_eq!(
            city.state
                .decisions()
                .get_decision(request)
                .unwrap()
                .status(),
            DecisionStatus::Pending,
            "the payment decision remains available, but the rival no longer waits"
        );
    }

    #[test]
    fn named_muscle_is_distributed_not_multiplied_across_legitimate_wards() {
        let (_registry, city, exposed) = exposed_city();
        let named = city
            .state
            .world()
            .characters_in_organization(city.player)
            .filter(|member| {
                city.state
                    .legal()
                    .active_arrest_for_character(member.id())
                    .is_none()
            })
            .filter(|member| {
                [CapabilityKind::Violence, CapabilityKind::Intimidation]
                    .into_iter()
                    .any(|skill| {
                        member
                            .capability(skill)
                            .is_some_and(|rating| rating.value() >= 60)
                    })
            })
            .count() as u32;
        let home_associates = active_district_force(&city.state, city.player, city.home);
        let exposed_associates = active_district_force(&city.state, city.player, exposed);
        assert_eq!(exposed_associates, 0);
        let total_local = local_defensive_force(&city.state, city.player, city.home)
            + local_defensive_force(&city.state, city.player, exposed);
        assert_eq!(
            total_local,
            named + home_associates + exposed_associates,
            "one violent member cannot defend multiple districts at full strength"
        );
        assert!(local_defensive_force(&city.state, city.player, exposed) < named);
    }

    #[test]
    fn protection_quotes_scale_to_large_portfolios_without_a_token_fee_cap() {
        let (registry, city, district) = exposed_city();
        let shops = active_shops(&city.state, city.player);
        let first = shops[&district][0];
        assert!(offer_cents(&registry, &city.state, &shops[&district]) > 5_000);
        assert_eq!(
            offer_cents(&registry, &city.state, &vec![first; 1_000]),
            1_500_000,
            "a large commercial portfolio should not get a trivial flat-price protection contract"
        );
    }

    #[test]
    fn an_existing_rival_operation_cannot_run_through_a_new_protection_demand() {
        use crate::operations::{OperationAbortCause, OperationObjectiveBlocker, OperationStatus};

        let (registry, mut city, district) = exposed_city();
        city.state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .expect("expired generated leads must close before a save");
        let already_authorized = crate::operations::operation_system::apply_due_rival_operations(
            &registry,
            &mut city.state,
        );
        let created = apply_due_protection_demands(&registry, &mut city.state);
        let request = city
            .state
            .decisions()
            .get_decision(
                created
                    .first()
                    .expect("the rival must demand protection")
                    .decision,
            )
            .unwrap();
        let DecisionContext::ProtectionDemand(terms) = request.context() else {
            unreachable!()
        };
        assert_eq!(terms.district(), district);
        let target = already_authorized
            .iter()
            .copied()
            .find(|id| {
                let op = city.state.operations().get_operation(*id).unwrap();
                op.responsible_organization() == terms.attacker()
                    && op.objective().business_target().is_some_and(|business| {
                        city.state
                            .world()
                            .get_business(business)
                            .is_some_and(|shop| {
                                shop.owner() == BusinessOwner::Organization(city.player)
                                    && shop.neighborhood() == district
                            })
                    })
            })
            .expect("the same credible rival must have planned a coercive operation");
        let operation = city.state.operations().get_operation(target).unwrap();
        assert_eq!(operation.status(), OperationStatus::Authorized);
        assert_eq!(
            crate::operations::operation_objective::resolve_objective_blocker(
                &city.state,
                operation,
            ),
            Some(OperationObjectiveBlocker::ProtectionAgreement),
            "already authorized operations must not bypass the protection decision"
        );
        let mut restored =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
        let tick = run_test_tick(&registry, &mut city.state);
        let replay = run_test_tick(&registry, &mut restored);
        assert_eq!(tick, replay);
        assert!(tick.aborted_operations.contains(&target));
        let aborted = city.state.operations().get_operation(target).unwrap();
        assert_eq!(aborted.status(), OperationStatus::Aborted);
        assert_eq!(
            aborted.abort_record().unwrap().cause(),
            OperationAbortCause::ObjectiveUnavailable(
                OperationObjectiveBlocker::ProtectionAgreement
            )
        );
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
    }

    #[test]
    fn rival_demands_actual_money_and_respects_paid_30_day_protection_after_restore() {
        let (registry, mut city, district) = exposed_city();
        let (decision, context) = first_demand(&registry, &mut city);
        assert_eq!(context.district(), district);
        assert!(rival_pressure_blocked(
            &city.state,
            context.attacker(),
            district
        ));
        let snapshot = bincode::serialize(&city.state).unwrap();
        assert!(matches!(
            super::super::validate_resolve_decision(
                &registry,
                &city.state,
                decision,
                city.player,
                DecisionResponse::Approve
            ),
            Err(DecisionError::InsufficientProtectionFunds { .. }),
        ));
        assert_eq!(snapshot, bincode::serialize(&city.state).unwrap());

        // A real balanced transfer funds the clean payment. The same ledger
        // later records the rival's actual receipt, not a virtual toll meter.
        let payer = city
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(city.player))
            .find(|account| account.kind() == AccountKind::AccountedFunds)
            .unwrap()
            .id();
        let clearing = city
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(city.player))
            .find(|account| account.kind() == AccountKind::Settlement)
            .unwrap()
            .id();
        validate_record_transaction(
            &city.state,
            LedgerTransactionDraft {
                occurred_at: city.state.now(),
                memo: "Test capital raised from existing settlement".into(),
                postings: vec![
                    LedgerPosting {
                        account: clearing,
                        amount: Money::from_cents(-300_000),
                    },
                    LedgerPosting {
                        account: payer,
                        amount: Money::from_cents(300_000),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        let victim_before = city
            .state
            .finance()
            .get_account(payer)
            .unwrap()
            .balance()
            .cents();
        let rival_cash = city
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(context.attacker()))
            .find(|account| account.kind() == AccountKind::StreetCash)
            .unwrap()
            .id();
        let rival_before = city
            .state
            .finance()
            .get_account(rival_cash)
            .unwrap()
            .balance()
            .cents();
        super::super::validate_resolve_decision(
            &registry,
            &city.state,
            decision,
            city.player,
            DecisionResponse::Approve,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        assert_eq!(
            city.state
                .finance()
                .get_account(payer)
                .unwrap()
                .balance()
                .cents(),
            victim_before - context.amount().cents()
        );
        assert_eq!(
            city.state
                .finance()
                .get_account(rival_cash)
                .unwrap()
                .balance()
                .cents(),
            rival_before + context.amount().cents()
        );
        assert!(rival_pressure_blocked(
            &city.state,
            context.attacker(),
            district
        ));
        crate::core::invariants::validate_state(&city.state).unwrap();
        let restored =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
        assert!(rival_pressure_blocked(
            &restored,
            context.attacker(),
            district
        ));
        let mut renewal = restored.clone();
        renewal.advance_clock(SimDuration::from_minutes(
            u32::try_from(PROTECTION_DAYS * DAY_MINUTES - 1).unwrap(),
        ));
        assert!(rival_pressure_blocked(
            &renewal,
            context.attacker(),
            district
        ));
        renewal.advance_clock(SimDuration::ONE_MINUTE);
        assert!(
            !rival_pressure_blocked(&renewal, context.attacker(), district),
            "the protected gang cannot promise an indefinite ceasefire for one payment"
        );
        let next_offer = apply_due_protection_demands(&registry, &mut renewal);
        assert_eq!(
            next_offer.len(),
            1,
            "an exposed commercial holding must face renewed demands when coverage expires"
        );
        let mut next = restored.clone();
        city.state.advance_clock(SimDuration::from_minutes(1));
        next.advance_clock(SimDuration::from_minutes(1));
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&next).unwrap()
        );
    }

    #[test]
    fn refusing_protection_opens_rival_retaliation_and_armed_defense_deters_demand() {
        let (registry, mut city, district) = exposed_city();
        let (decision, context) = first_demand(&registry, &mut city);
        super::super::validate_resolve_decision(
            &registry,
            &city.state,
            decision,
            city.player,
            DecisionResponse::Reject,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        assert!(!rival_pressure_blocked(
            &city.state,
            context.attacker(),
            district
        ));
        assert!(recently_refused_protection(
            &city.state,
            context.attacker(),
            district
        ));
        assert!(apply_due_protection_demands(&registry, &mut city.state).is_empty());
        crate::core::invariants::validate_state(&city.state).unwrap();

        let (registry, mut protected, district) = exposed_city();
        let attackers = protected
            .state
            .world()
            .organizations()
            .filter(|organization| {
                organization.kind() == OrganizationKind::Criminal
                    && organization.id() != protected.player
            })
            .map(|organization| {
                active_district_force(&protected.state, organization.id(), district)
            })
            .max()
            .unwrap_or(0);
        // A mobile crew has to cover both the headquarters ward and the
        // exposed commercial ward. Hiring just one person per threat point
        // cannot magically defend every district at the same strength.
        for index in 0..attackers.saturating_mul(2).saturating_add(8) {
            insert_character(
                &mut protected.state,
                CharacterDraft {
                    name: format!("Local Defender {index}"),
                    organization: Some(protected.player),
                    supervisor: None,
                    autonomy: AutonomyLevel::Broad,
                    capabilities: BTreeMap::from([(
                        CapabilityKind::Violence,
                        Rating::try_new(90).unwrap(),
                    )]),
                    traits: BTreeSet::new(),
                    drives: BTreeMap::new(),
                },
            )
            .unwrap();
        }
        assert!(local_defensive_force(&protected.state, protected.player, district) >= attackers);
        protected.state.advance_clock(SimDuration::from_minutes(
            u32::try_from(DAY_MINUTES - 1).unwrap(),
        ));
        let tick = run_test_tick(&registry, &mut protected.state);
        assert!(tick.decision_requests.iter().all(|outcome| {
            !matches!(
                protected
                    .state
                    .decisions()
                    .get_decision(outcome.decision)
                    .unwrap()
                    .context(),
                DecisionContext::ProtectionDemand(_)
            )
        }));
        crate::core::invariants::validate_state(&protected.state).unwrap();
    }
}
