//! Protection decisions, local defense, and balanced-payment regressions.

use super::*;
use crate::city_generation::{CityScale, GeneratedCity, generate_city};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_test_tick;
use crate::core::time::SimDuration;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::finance::finance_system::insert_account;
use crate::world::territory_conflict::active_district_force;
use crate::world::world_system::{insert_business, insert_character};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, CapabilityKind, CharacterDraft,
    Rating,
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
                .then(|| racket.location().neighborhood(city.state.world()))
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
fn unanswered_protection_offer_expires_and_can_be_replaced() {
    let (registry, mut city, district) = exposed_city();
    let (request, terms) = first_demand(&registry, &mut city);
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for state in [&mut city.state, &mut restored] {
        state.advance_clock(SimDuration::from_minutes((30 * DAY_MINUTES) as u32));
        assert!(!rival_pressure_blocked(state, terms.attacker(), district));
        let renewed = apply_due_protection_demands(&registry, state);
        let expired = state.decisions().get_decision(request).unwrap();
        assert_eq!(expired.status(), DecisionStatus::Cancelled);
        assert_eq!(
            expired.cancellation().unwrap().reason(),
            DecisionCancellationReason::ProtectionOfferExpired
        );
        assert!(expired.resolution().is_none());
        assert!(!recently_refused_protection(
            state,
            terms.attacker(),
            district
        ));
        let replacement = state
            .decisions()
            .pending_protection_demand(district)
            .unwrap();
        assert_ne!(replacement, request);
        assert!(
            renewed
                .iter()
                .any(|outcome| outcome.decision == replacement)
        );
    }
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
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
    let defenses = local_defensive_forces(&city.state, city.player);
    assert_eq!(defenses.len(), 2);
    let home = defenses[&city.home];
    let exposed = defenses[&exposed];
    assert_eq!(home.associates, home_associates);
    assert_eq!(home.active_rackets, 1);
    assert_eq!(home.operating_shops, 2);
    assert_eq!(exposed.associates, 0);
    assert_eq!(exposed.active_rackets, 0);
    assert_eq!(exposed.operating_shops, 3);
    assert_eq!(
        home.total() + exposed.total(),
        named + home_associates + exposed_associates,
        "one violent member cannot defend multiple districts at full strength"
    );
    assert_eq!(home.named_enforcers + exposed.named_enforcers, named);
    assert!(exposed.total() < named);
}

#[test]
fn executing_operation_temporarily_removes_enforcer_from_local_defense() {
    use crate::core::entity::EntityRef;
    use crate::operations::operation_system::{
        OperationTransition, apply_transition, validate_authorize_operation,
    };
    use crate::operations::{
        OperationApproach, OperationDraft, OperationKind, OperationObjective, OperationStatus,
        RoleKind,
    };
    let (registry, mut city, exposed) = exposed_city();
    let defender = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Mobile defender".into(),
            organization: Some(city.player),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([
                (CapabilityKind::Violence, Rating::try_new(90).unwrap()),
                (CapabilityKind::Surveillance, Rating::try_new(90).unwrap()),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let force = |state: &AppState| {
        let defenses = local_defensive_forces(state, city.player);
        defenses[&city.home].total() + defenses[&exposed].total()
    };
    let before = force(&city.state);
    let op = validate_authorize_operation(
        &registry,
        &city.state,
        OperationDraft {
            title: "Off-site reconnaissance".into(),
            kind: OperationKind::Surveillance,
            responsible_organization: city.player,
            leader: defender,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Neighborhood(exposed),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, defender)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: city.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert_eq!(
        force(&city.state),
        before,
        "a future order is not current field work"
    );
    assert!(
        run_test_tick(&registry, &mut city.state)
            .started_operations
            .contains(&op)
    );
    assert_eq!(
        city.state.operations().get_operation(op).unwrap().status(),
        OperationStatus::InProgress
    );
    assert_eq!(
        force(&city.state),
        before - 1,
        "one committed enforcer cannot simultaneously defend the holdings"
    );
    let mut replay = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(force(&replay), before - 1);
    for state in [&mut city.state, &mut replay] {
        apply_transition(&registry, state, op, OperationTransition::Abort).unwrap();
        assert_eq!(
            force(state),
            before,
            "terminal operations free the enforcer"
        );
    }
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&replay).unwrap()
    );
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
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .expect("expired generated leads must close before a save");
    let already_authorized =
        crate::operations::operation_system::apply_due_rival_operations(&registry, &mut city.state);
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
        crate::operations::operation_objective::resolve_objective_blocker(&city.state, operation,),
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
        OperationAbortCause::ObjectiveUnavailable(OperationObjectiveBlocker::ProtectionAgreement)
    );
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

fn fund_protection_payment(
    city: &mut GeneratedCity,
    cents: i64,
) -> crate::core::id::FinancialAccountId {
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
                    amount: Money::from_cents(-cents),
                },
                LedgerPosting {
                    account: payer,
                    amount: Money::from_cents(cents),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    payer
}

#[test]
fn protection_approval_rejects_same_minute_business_transfer() {
    let (registry, mut city, _) = exposed_city();
    let (decision, context) = first_demand(&registry, &mut city);
    fund_protection_payment(&mut city, context.amount().cents());
    let held = super::super::validate_resolve_decision(
        &registry,
        &city.state,
        decision,
        city.player,
        DecisionResponse::Approve,
    )
    .unwrap();

    crate::world::world_system::validate_transfer_business_ownership(
        &city.state,
        context.business(),
        BusinessOwner::Independent,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();

    let before = bincode::serialize(&city.state).unwrap();
    assert_eq!(
        held.commit(&mut city.state),
        Err(DecisionError::ProtectionNoLongerOwned(context.business()))
    );
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);
    assert_eq!(
        city.state
            .decisions()
            .get_decision(decision)
            .unwrap()
            .status(),
        DecisionStatus::Pending
    );
}

#[test]
fn protection_approval_rejects_disappeared_local_threat_without_payment() {
    let (registry, mut city, _) = exposed_city();
    let (decision, context) = first_demand(&registry, &mut city);
    fund_protection_payment(&mut city, context.amount().cents());
    let approval = super::super::validate_resolve_decision(
        &registry,
        &city.state,
        decision,
        city.player,
        DecisionResponse::Approve,
    )
    .unwrap();
    let rackets: Vec<_> = city
        .state
        .enterprises()
        .active_for_organization(context.attacker())
        .filter(|racket| {
            racket.location().neighborhood(city.state.world()) == Some(context.district())
        })
        .map(|racket| racket.id())
        .collect();
    assert!(!rackets.is_empty());
    for racket in rackets {
        crate::enterprises::enterprise_execution::validate_suspend_enterprise(&city.state, racket)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    let before = bincode::serialize(&city.state).unwrap();
    let expected = DecisionError::ProtectionThreatEnded {
        district: context.district(),
        attacker: context.attacker(),
    };
    assert!(matches!(
        approval.commit(&mut city.state),
        Err(error) if error == expected
    ));
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(matches!(
        super::super::validate_resolve_decision(
            &registry,
            &restored,
            decision,
            city.player,
            DecisionResponse::Approve,
        ),
        Err(error) if error == expected
    ));
    assert_eq!(bincode::serialize(&restored).unwrap(), before);
    super::super::validate_resolve_decision(
        &registry,
        &restored,
        decision,
        city.player,
        DecisionResponse::Reject,
    )
    .unwrap()
    .commit(&mut restored)
    .unwrap();
}

#[test]
fn protection_approval_opens_usable_cash_when_existing_receipts_would_cover_debt() {
    let (registry, mut city, _) = exposed_city();
    let (decision, context) = first_demand(&registry, &mut city);
    fund_protection_payment(&mut city, context.amount().cents());
    let owner = FinancialOwner::Organization(context.attacker());
    let cash: Vec<_> = city
        .state
        .finance()
        .accounts_for(owner)
        .filter(|account| account.kind() == AccountKind::StreetCash)
        .map(|account| (account.id(), account.balance().cents()))
        .collect();
    assert!(!cash.is_empty());
    let clearing = insert_account(
        &mut city.state,
        crate::finance::FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let mut postings: Vec<_> = cash
        .iter()
        .filter_map(|&(account, balance)| {
            let amount = -1 - balance;
            (amount != 0).then_some(LedgerPosting {
                account,
                amount: Money::from_cents(amount),
            })
        })
        .collect();
    let total: i64 = postings.iter().map(|posting| posting.amount.cents()).sum();
    postings.push(LedgerPosting {
        account: clearing,
        amount: Money::from_cents(-total),
    });
    validate_record_transaction(
        &city.state,
        LedgerTransactionDraft {
            occurred_at: city.state.now(),
            memo: "Existing rival cash deficits".into(),
            postings,
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
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
    for (id, _) in &cash {
        assert_eq!(
            city.state.finance().get_account(*id).unwrap().balance(),
            Money::from_cents(-1)
        );
    }
    let new_cash: Vec<_> = city
        .state
        .finance()
        .accounts_for(owner)
        .filter(|account| account.kind() == AccountKind::StreetCash)
        .filter(|account| !cash.iter().any(|&(id, _)| id == account.id()))
        .collect();
    assert_eq!(new_cash.len(), 1);
    assert_eq!(new_cash[0].balance(), context.amount());
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&restored).unwrap(),
        bincode::serialize(&city.state).unwrap()
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

    // Fund the clean payment through a real balanced transfer.
    let payer = fund_protection_payment(&mut city, 300_000);
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
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
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
fn latest_protection_history_preserves_each_gangs_terms_across_renewal_and_restore() {
    independent_protection_terms(DecisionResponse::Reject);
    independent_protection_terms(DecisionResponse::Approve);
}

fn independent_protection_terms(first_response: DecisionResponse) {
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
    use crate::enterprises::enterprise_execution::{
        validate_establish_enterprise, validate_retire_enterprise, validate_suspend_enterprise,
    };
    use crate::enterprises::{
        EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseStatus,
    };

    let (registry, mut city, district) = exposed_city();
    // Isolate this daily owner from concurrent expansion by the other outfits.
    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    let first = apply_due_protection_demands(&registry, &mut city.state)[0].decision;
    let DecisionContext::ProtectionDemand(terms) = city
        .state
        .decisions()
        .get_decision(first)
        .unwrap()
        .context()
    else {
        panic!("expected a protection demand");
    };
    assert_eq!(terms.district(), district);
    let first_gang = terms.attacker();
    let first_paid = first_response == DecisionResponse::Approve;
    if first_paid {
        fund_protection_payment(&mut city, terms.amount().cents());
    }
    super::super::validate_resolve_decision(
        &registry,
        &city.state,
        first,
        city.player,
        first_response,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();

    // Replace the local coercive footprint through ordinary enterprise operations.
    // The old gang's terms remain meaningful even after a different gang demands money.
    let local: Vec<_> = city
        .state
        .enterprises()
        .enterprises_for_organization(first_gang)
        .filter(|racket| racket.location().neighborhood(city.state.world()) == Some(district))
        .filter(|racket| racket.status() != EnterpriseStatus::Retired)
        .map(|racket| (racket.id(), racket.status()))
        .collect();
    for (racket, status) in local {
        if status == EnterpriseStatus::Active {
            validate_suspend_enterprise(&city.state, racket)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
        validate_retire_enterprise(&city.state, racket)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    let second_gang = city
        .rivals
        .iter()
        .copied()
        .find(|rival| *rival != first_gang)
        .unwrap();
    let captain = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Second gang local captain".into(),
            organization: Some(second_gang),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(100).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Neighborhood(district);
    let mandate = validate_assign_mandate(
        &city.state,
        MandateDraft {
            organization: second_gang,
            manager: captain,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let owner = FinancialOwner::Organization(second_gang);
    let cash_account = city
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::StreetCash)
        .unwrap()
        .id();
    let settlement_account = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_enterprise(
        &registry,
        &city.state,
        EnterpriseDraft {
            organization: second_gang,
            kind: EnterpriseKind::Protection,
            authority: MandateAuthority {
                mandate,
                manager: captain,
                scope,
            },
            location: EnterpriseLocation::Neighborhood(district),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    // The unrelated gang must not inherit the first gang's cooldown.
    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    let new_offer = apply_due_protection_demands(&registry, &mut city.state);
    assert_eq!(new_offer.len(), 1);
    let second = new_offer[0].decision;
    let second_terms = match city
        .state
        .decisions()
        .get_decision(second)
        .unwrap()
        .context()
    {
        DecisionContext::ProtectionDemand(terms) => terms,
        DecisionContext::OperationPoliceArrival { .. }
        | DecisionContext::RecruitmentApproval(_) => {
            panic!("expected a new protection demand")
        }
    };
    assert_eq!(second_terms.attacker(), second_gang);
    assert_eq!(second_terms.district(), district);
    let latest = |state: &AppState, attacker| {
        state
            .decisions()
            .latest_protection_demand(district, attacker)
            .map(|record| record.id())
    };
    assert_eq!(latest(&city.state, first_gang), Some(first));
    assert_eq!(latest(&city.state, second_gang), Some(second));
    assert_eq!(
        recently_refused_protection(&city.state, first_gang, district),
        !first_paid
    );
    assert_eq!(
        rival_pressure_blocked(&city.state, first_gang, district),
        first_paid
    );
    assert!(rival_pressure_blocked(&city.state, second_gang, district));

    fund_protection_payment(&mut city, second_terms.amount().cents());
    super::super::validate_resolve_decision(
        &registry,
        &city.state,
        second,
        city.player,
        DecisionResponse::Approve,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(latest(&restored, first_gang), Some(first));
    assert_eq!(latest(&restored, second_gang), Some(second));
    assert_eq!(
        recently_refused_protection(&restored, first_gang, district),
        !first_paid
    );
    assert!(rival_pressure_blocked(&restored, second_gang, district));
    assert_eq!(
        rival_pressure_blocked(&restored, first_gang, district),
        first_paid
    );
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
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
        .map(|organization| active_district_force(&protected.state, organization.id(), district))
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
    assert!(
        local_defensive_forces(&protected.state, protected.player)[&district].total() >= attackers
    );
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
