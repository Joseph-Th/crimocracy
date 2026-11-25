use super::*;
use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::core::invariants::validate_invariants;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::SimDuration;
use crate::economy::business_economy_system::{
    decide_business_cycle, validate_business_cycle_plan, validate_suspend_business_economy,
};
use crate::finance::Money;
use crate::world::BusinessOwner;
use crate::world::world_system::validate_transfer_business_ownership;

#[test]
fn proprietor_and_employee_record_the_day_the_business_closes() {
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::validate_establish_business_economy;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::world::world_system::{insert_business, insert_character, insert_neighborhood};
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, CharacterDraft,
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    let registry = build_registry();
    let mut state = AppState::new(562_365);
    let rating = |value| Rating::try_new(value).unwrap();
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Depressed Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(5),
                    commercial_activity: rating(5),
                    illicit_demand: rating(5),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(95),
                },
            },
        },
    )
    .unwrap();
    let owner = insert_character(
        &mut state,
        CharacterDraft {
            name: "Warehouse Proprietor".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Bleeding Warehouse".to_owned(),
            kind: BusinessKind::Warehouse,
            functions: BTreeSet::from([BusinessFunction::Warehousing]),
            neighborhood,
            owner: BusinessOwner::Character(owner),
        },
    )
    .unwrap();
    let employee = insert_character(
        &mut state,
        CharacterDraft {
            name: "Warehouse Employee".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    validate_set_civic_occupation(
        &state,
        employee,
        neighborhood,
        IndependentVocation::BusinessEmployee(business),
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let operating_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business,
            operating_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();

    let threshold = registry
        .get_business(BusinessKind::Warehouse)
        .economics()
        .losing_cycles_before_suspension();
    for _ in 0..threshold {
        state.advance_clock(crate::core::time::DAY_DURATION);
        let before = state.world().get_character(owner).unwrap().life();
        let employee_before = state.world().get_character(employee).unwrap().life();
        let plan = decide_business_cycle(&registry, &state, business, -500).unwrap();
        let cycle = validate_business_cycle_plan(&state, plan)
            .unwrap()
            .commit(&mut state)
            .unwrap();
        assert!(state.economy().get_cycle(cycle).unwrap().net_cash() < Money::ZERO);
        assert!(
            !(state.now().as_minutes() / DAY_MINUTES + u64::from(owner.raw())).is_multiple_of(7)
        );
        apply_due_character_lives(&mut state);
        let after = state.world().get_character(owner).unwrap().life();
        assert_eq!(after.career_days(), before.career_days() + 1);
        assert_eq!(
            after.professional_standing(),
            before.professional_standing() - 2
        );
        assert_eq!(after.fatigue(), before.fatigue() + 5);
        assert_eq!(
            after.last_activity(),
            Some((state.now(), CivicActivity::BusinessManagement(business)))
        );
        assert!(
            !(state.now().as_minutes() / DAY_MINUTES + u64::from(employee.raw())).is_multiple_of(7)
        );
        let employee_after = state.world().get_character(employee).unwrap().life();
        assert_eq!(
            employee_after.career_days(),
            employee_before.career_days() + 1
        );
        assert_eq!(
            employee_after.professional_standing(),
            employee_before.professional_standing() + 1
        );
        assert_eq!(employee_after.fatigue(), employee_before.fatigue() + 2);
        assert_eq!(
            employee_after.last_activity(),
            Some((state.now(), CivicActivity::BusinessShift(business)))
        );
    }
    assert_eq!(
        state
            .economy()
            .get_business_economy(business)
            .unwrap()
            .status(),
        BusinessOperatingStatus::Suspended
    );
    let closed_life = state.world().get_character(owner).unwrap().life();
    let employee_closed = state.world().get_character(employee).unwrap().life();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    for state in [&mut state, &mut restored] {
        state.advance_clock(crate::core::time::DAY_DURATION);
        apply_due_character_lives(state);
        let after = state.world().get_character(owner).unwrap().life();
        assert_eq!(after.career_days(), closed_life.career_days());
        assert_eq!(
            after.professional_standing(),
            closed_life.professional_standing()
        );
        assert_eq!(after.last_activity(), closed_life.last_activity());
        assert_eq!(after.fatigue(), 0);
        let employee_after = state.world().get_character(employee).unwrap().life();
        assert_eq!(employee_after.career_days(), employee_closed.career_days());
        assert_eq!(
            employee_after.professional_standing(),
            employee_closed.professional_standing()
        );
        assert_eq!(
            employee_after.last_activity(),
            employee_closed.last_activity()
        );
        assert_eq!(employee_after.fatigue(), 0);
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn suspended_proprietor_does_not_resume_an_old_vocation() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let business = city
        .state
        .world()
        .businesses()
        .find(|shop| {
            matches!(shop.owner(), BusinessOwner::Character(_))
                && city
                    .state
                    .economy()
                    .get_business_economy(shop.id())
                    .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
        })
        .unwrap()
        .id();
    let worker = city
        .state
        .world()
        .characters()
        .find(|person| {
            let IndependentVocation::BusinessEmployee(workplace) = person.life().vocation() else {
                return false;
            };
            workplace != business
                && city.state.world().civic_profession(person.id())
                    == Some(CivicProfession::BusinessEmployee)
                && city
                    .state
                    .economy()
                    .get_business_economy(workplace)
                    .is_some_and(|economy| economy.status() == BusinessOperatingStatus::Active)
        })
        .unwrap()
        .id();
    validate_transfer_business_ownership(&city.state, business, BusinessOwner::Character(worker))
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_suspend_business_economy(&city.state, business)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        city.state.world().civic_profession(worker),
        Some(CivicProfession::BusinessOwner)
    );
    let day = (1..=7)
        .find(|day| !(*day + u64::from(worker.raw())).is_multiple_of(7))
        .unwrap();
    let life = city.state.world().get_character(worker).unwrap().life();
    city.state
        .advance_clock(SimDuration::from_minutes((day * DAY_MINUTES) as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    apply_due_character_lives(&mut city.state);
    let after = city.state.world().get_character(worker).unwrap().life();
    assert_eq!(after.career_days(), life.career_days());
    assert_eq!(after.professional_standing(), life.professional_standing());
    assert_eq!(
        after.last_activity(),
        life.last_activity(),
        "a closed proprietor must not work at a previous employer"
    );
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        restored.world().get_character(worker).unwrap().life(),
        after
    );
}

#[test]
fn reopened_proprietor_does_not_reuse_pre_suspension_earnings() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let (owner, business) = city
        .state
        .world()
        .businesses()
        .find_map(|shop| {
            let BusinessOwner::Character(owner) = shop.owner() else {
                return None;
            };
            if city
                .state
                .world()
                .businesses_owned_by_character(owner)
                .count()
                != 1
            {
                return None;
            }
            city.state
                .economy()
                .get_business_economy(shop.id())
                .filter(|economy| economy.status() == BusinessOperatingStatus::Active)
                .map(|_| (owner, shop.id()))
        })
        .unwrap();
    let due = city
        .state
        .economy()
        .get_business_economy(business)
        .unwrap()
        .next_cycle_at()
        .unwrap();
    city.state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due.as_minutes()).unwrap(),
    ));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    let plan = decide_business_cycle(&registry, &city.state, business, 0).unwrap();
    let cycle = validate_business_cycle_plan(&city.state, plan)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_ne!(
        city.state.economy().get_cycle(cycle).unwrap().net_cash(),
        Money::ZERO
    );
    validate_suspend_business_economy(&city.state, business)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    crate::economy::business_economy_system::validate_resume_business_economy(
        &registry,
        &city.state,
        business,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();

    let day = city.state.now().as_minutes() / DAY_MINUTES;
    let working_day = (day..day + 7)
        .find(|&day| !(day + u64::from(owner.raw())).is_multiple_of(7))
        .unwrap();
    let work = choose_work(&city.state, owner, working_day);
    assert_eq!(
        work.action,
        Some(CivicActivity::BusinessManagement(business))
    );
    assert_eq!(work.business_profit, 0);
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        choose_work(&restored, owner, working_day).business_profit,
        0
    );

    for state in [&mut city.state, &mut restored] {
        let due = state
            .economy()
            .get_business_economy(business)
            .unwrap()
            .next_cycle_at()
            .unwrap();
        state.advance_clock(SimDuration::from_minutes(
            u32::try_from(due.as_minutes() - state.now().as_minutes()).unwrap(),
        ));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, state)
            .unwrap();
        let plan = decide_business_cycle(&registry, state, business, 0).unwrap();
        let cycle = validate_business_cycle_plan(state, plan)
            .unwrap()
            .commit(state)
            .unwrap();
        let net = state.economy().get_cycle(cycle).unwrap().net_cash();
        assert_ne!(net, Money::ZERO);
        let work = choose_work(state, owner, working_day);
        assert_eq!(work.business_profit, if net > Money::ZERO { 1 } else { -1 });
        validate_invariants(state);

        // An operating shop can miss its next settlement (for example,
        // when a ledger account is temporarily unable to post). The owner
        // still works, but the old result cannot earn another day's standing
        // or impose another day's loss penalty.
        state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        let work = choose_work(state, owner, working_day);
        assert_eq!(
            work.action,
            Some(CivicActivity::BusinessManagement(business))
        );
        assert_eq!(work.business_profit, 0);
    }
}

#[test]
fn new_proprietor_does_not_inherit_previous_owners_earnings_record() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let (previous_owner, business) = city
        .state
        .world()
        .businesses()
        .find_map(|shop| {
            let BusinessOwner::Character(owner) = shop.owner() else {
                return None;
            };
            if city
                .state
                .world()
                .businesses_owned_by_character(owner)
                .count()
                != 1
            {
                return None;
            }
            city.state
                .economy()
                .get_business_economy(shop.id())
                .filter(|economy| economy.status() == BusinessOperatingStatus::Active)
                .map(|_| (owner, shop.id()))
        })
        .expect("generated city has an active private proprietor");
    let successor = city
        .state
        .world()
        .characters()
        .find(|person| {
            person.id() != previous_owner
                && person.organization().is_none()
                && city
                    .state
                    .world()
                    .businesses_owned_by_character(person.id())
                    .next()
                    .is_none()
        })
        .expect("generated city has an independent non-owner")
        .id();

    // Settle a real prior owner's business cycle; its signed net is the
    // historical signal that would otherwise be incorrectly inherited.
    for _ in 0..45 {
        city.state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        if let Ok(plan) = decide_business_cycle(&registry, &city.state, business, 0) {
            validate_business_cycle_plan(&city.state, plan)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
        if city
            .state
            .economy()
            .cycles_for(business)
            .next_back()
            .is_some_and(|cycle| cycle.net_cash() != Money::ZERO)
        {
            break;
        }
    }
    let historical_cycle = city
        .state
        .economy()
        .cycles_for(business)
        .next_back()
        .expect("private business should settle during the review horizon");
    assert_ne!(historical_cycle.net_cash(), Money::ZERO);

    validate_transfer_business_ownership(
        &city.state,
        business,
        BusinessOwner::Character(successor),
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();

    let day = city.state.now().as_minutes() / DAY_MINUTES;
    let working_day = (day..day + 7)
        .find(|&day| !(day + u64::from(successor.raw())).is_multiple_of(7))
        .unwrap();
    let work = choose_work(&city.state, successor, working_day);
    assert_eq!(
        work.action,
        Some(CivicActivity::BusinessManagement(business))
    );
    assert_eq!(work.business_profit, 0);

    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let restored_work = choose_work(&restored, successor, working_day);
    assert_eq!(restored_work.action, work.action);
    assert_eq!(restored_work.business_profit, work.business_profit);
}
