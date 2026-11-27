//! Rival purchase planning must account for the real economics of reopening a closed shop.

use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::DAY_MINUTES;
use crate::economy::BusinessOperatingStatus;
use crate::economy::business_economy_system::validate_suspend_business_economy;
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

#[test]
fn rival_investment_accounts_for_its_own_closed_competitor_without_changing_public_appraisal() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = ward(&mut fixture, "Marginal local market", 10, 10);
    let closed = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Organization(rival),
        neighborhood,
    );
    let candidate = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    let public = appraise_business(&fixture.registry, &fixture.state, candidate).unwrap();
    assert!(public.expected_cycle_net < Money::ZERO);
    validate_suspend_business_economy(&fixture.state, closed)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        appraise_business(&fixture.registry, &fixture.state, candidate).unwrap(),
        public
    );
    let expected_net = business_economy_system::forecast_live_business_net(
        &fixture.registry,
        &fixture.state,
        candidate,
    )
    .unwrap();
    assert!(expected_net > Money::ZERO);
    let price = acquisition_offer(&fixture.registry, &fixture.state, rival, candidate)
        .unwrap()
        .0;
    let reserve = 7 * fixture.registry.upkeep().per_member_daily().cents();
    fund_accounted_from_street(&mut fixture, price.cents() + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert_eq!(
            apply_due_autonomous_acquisitions(&fixture.registry, state),
            vec![candidate]
        );
        assert_eq!(
            state.world().get_business(candidate).unwrap().owner(),
            BusinessOwner::Organization(rival)
        );
        state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        let plan =
            business_economy_system::decide_business_cycle(&fixture.registry, state, candidate, 0)
                .unwrap();
        business_economy_system::validate_business_cycle_plan(state, plan)
            .unwrap()
            .commit(state)
            .unwrap();
        assert_eq!(
            state
                .economy()
                .cycles_for(candidate)
                .next_back()
                .unwrap()
                .net_cash(),
            expected_net
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn rival_avoids_reopening_positive_margin_shop_that_destroys_its_other_earnings() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    let player = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Player organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut fixture.state, player).unwrap();
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Marginal retail ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(20),
                    commercial_activity: rating(21),
                    illicit_demand: rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(100),
                },
            },
        },
    )
    .unwrap();

    let dormant = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    let pharmacy = established_shop(
        &mut fixture,
        BusinessKind::Pharmacy,
        BusinessOwner::Organization(rival),
        neighborhood,
    );
    validate_suspend_business_economy(&fixture.state, dormant)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let appraisal = appraise_business(&fixture.registry, &fixture.state, dormant).unwrap();
    assert!(appraisal.expected_cycle_net > Money::ZERO);
    let lost_daily_net = business_economy_system::acquisition_owner_daily_displacement(
        &fixture.registry,
        &fixture.state,
        dormant,
        rival,
        0,
        None,
    )
    .unwrap();
    let incoming_daily_net = i128::from(appraisal.expected_cycle_net.cents())
        * i128::from(DAY_MINUTES)
        / i128::from(appraisal.cycle_minutes);
    assert!(lost_daily_net > incoming_daily_net);

    let price = fixture
        .registry
        .get_business(BusinessKind::Retail)
        .economics()
        .acquisition_cost();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price.cents() * 2 + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert!(apply_due_autonomous_acquisitions(&fixture.registry, state).is_empty());
        assert_eq!(
            state.world().get_business(dormant).unwrap().owner(),
            BusinessOwner::Independent
        );
        assert_eq!(
            state.world().get_business(pharmacy).unwrap().owner(),
            BusinessOwner::Organization(rival)
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );

    // This is an autonomous economic choice, not a new player-acquisition restriction.
    let purchase = validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: rival,
            business: dormant,
            funding_accounts: BTreeSet::from([fixture.accounted]),
        },
    )
    .unwrap();
    purchase
        .commit(&fixture.registry, &mut fixture.state)
        .unwrap();
    validate_invariants(&fixture.state);
}

#[test]
fn rival_rejects_damaged_dormant_purchase_that_loses_income_after_repair() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Marginal retail ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(35),
                    commercial_activity: rating(30),
                    illicit_demand: rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(100),
                },
            },
        },
    )
    .unwrap();
    let dormant = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    let pharmacy = established_shop(
        &mut fixture,
        BusinessKind::Pharmacy,
        BusinessOwner::Organization(rival),
        neighborhood,
    );
    let price = acquisition_offer(&fixture.registry, &fixture.state, rival, dormant)
        .unwrap()
        .0;
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price.cents() + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    business_economy_system::validate_disrupt_business_economy(
        &fixture.registry,
        &fixture.state,
        dormant,
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    validate_suspend_business_economy(&fixture.state, dormant)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();

    let appraisal = appraise_business(&fixture.registry, &fixture.state, dormant).unwrap();
    assert!(appraisal.expected_cycle_net > Money::ZERO);
    assert!(
        fixture
            .state
            .economy()
            .get_business_economy(dormant)
            .unwrap()
            .is_disrupted(fixture.state.now())
    );
    let mut purchased = fixture.state.clone();
    validate_acquire_business(
        &fixture.registry,
        &purchased,
        BusinessAcquisitionDraft {
            organization: rival,
            business: dormant,
            funding_accounts: BTreeSet::from([fixture.accounted]),
        },
    )
    .unwrap()
    .commit(&fixture.registry, &mut purchased)
    .unwrap();
    // Inspect the actual settled books after damage expires, rather than comparing
    // the planner's two formulas against each other. Neither branch changes title
    // or closes another shop during the matched settlement.
    let mut retained = fixture.state.clone();
    for state in [&mut purchased, &mut retained] {
        state.advance_clock(fixture.registry.business_disruption().duration());
        assert!(
            !state
                .economy()
                .get_business_economy(dormant)
                .unwrap()
                .is_disrupted(state.now())
        );
        for business in [pharmacy, dormant] {
            if !state
                .economy()
                .get_business_economy(business)
                .unwrap()
                .is_operating()
            {
                continue;
            }
            let plan = business_economy_system::decide_business_cycle(
                &fixture.registry,
                state,
                business,
                0,
            )
            .unwrap();
            business_economy_system::validate_business_cycle_plan(state, plan)
                .unwrap()
                .commit(state)
                .unwrap();
        }
        restore_save(
            &fixture.registry,
            build_save(&fixture.registry, state).unwrap(),
        )
        .unwrap();
    }
    let settled_net = |state: &AppState, business| {
        state
            .economy()
            .cycles_for(business)
            .next_back()
            .unwrap()
            .net_cash()
            .cents()
    };
    assert!(settled_net(&purchased, dormant) > 0);
    assert!(
        settled_net(&purchased, pharmacy) + settled_net(&purchased, dormant)
            < settled_net(&retained, pharmacy)
    );

    let before = bincode::serialize(&fixture.state).unwrap();
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert!(
            apply_due_autonomous_acquisitions(&fixture.registry, state).is_empty(),
            "the buyer must not price healthy revenue against permanently damaged competition"
        );
        assert_eq!(bincode::serialize(state).unwrap(), before);
    }
}

#[test]
fn rival_reopens_viable_dormant_listing_without_harming_its_portfolio() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Available executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let neighborhood = fixture
        .state
        .world()
        .get_business(fixture.business)
        .unwrap()
        .neighborhood();
    let dormant = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        neighborhood,
    );
    validate_suspend_business_economy(&fixture.state, dormant)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        business_economy_system::acquisition_owner_daily_displacement(
            &fixture.registry,
            &fixture.state,
            dormant,
            rival,
            0,
            None,
        )
        .unwrap(),
        0,
    );
    let price = fixture
        .registry
        .get_business(BusinessKind::Retail)
        .economics()
        .acquisition_cost();
    assert!(
        appraise_business(&fixture.registry, &fixture.state, dormant)
            .unwrap()
            .expected_cycle_net
            > Money::ZERO
    );
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price.cents() + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    assert_eq!(
        apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state),
        vec![dormant],
    );
    assert_eq!(
        fixture
            .state
            .economy()
            .get_business_economy(dormant)
            .unwrap()
            .status(),
        BusinessOperatingStatus::Active,
    );
    validate_invariants(&fixture.state);
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn rival_declines_positive_margin_purchase_that_reduces_distributable_income() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let mut shops = Vec::new();
    for index in 1..=6 {
        let district = ward(&mut fixture, &format!("Prosperous ward {index}"), 100, 100);
        shops.push(established_shop(
            &mut fixture,
            BusinessKind::Hospitality,
            BusinessOwner::Organization(rival),
            district,
        ));
    }
    let third = ward(&mut fixture, "Marginal expansion ward", 20, 21);
    let candidate = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Independent,
        third,
    );
    let incoming = appraise_business(&fixture.registry, &fixture.state, candidate).unwrap();
    let daily_receipt = |business, rate| {
        let quote = appraise_business(&fixture.registry, &fixture.state, business).unwrap();
        let profit = quote.expected_cycle_net;
        let receipt = profit
            .checked_sub(business_economy_system::portfolio_overhead_charge(
                profit, rate,
            ))
            .unwrap();
        i128::from(receipt.cents()) * i128::from(DAY_MINUTES) / i128::from(quote.cycle_minutes)
    };
    assert!(incoming.expected_cycle_net > Money::ZERO);
    assert_eq!(
        business_economy_system::portfolio_overhead_basis_points(&fixture.state, rival),
        1700
    );
    let before: i128 = shops.iter().map(|&shop| daily_receipt(shop, 1700)).sum();
    let after = shops
        .iter()
        .map(|&shop| daily_receipt(shop, 2100))
        .sum::<i128>()
        + daily_receipt(candidate, 2100);
    assert!(
        after < before,
        "fixture needs a destructive purchase: {before} -> {after}"
    );
    let price = acquisition_offer(&fixture.registry, &fixture.state, rival, candidate)
        .unwrap()
        .0
        .cents();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(apply_due_autonomous_acquisitions(&fixture.registry, &mut fixture.state).is_empty());
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert!(apply_due_autonomous_acquisitions(&fixture.registry, &mut restored).is_empty());
    assert_eq!(bincode::serialize(&restored).unwrap(), before);
}

#[test]
fn rival_negotiated_purchase_includes_consolidation_premium() {
    let mut fixture = make_independent_fixture();
    let rival = fixture.organization;
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    for index in 1..=3 {
        let district = ward(&mut fixture, &format!("Existing ward {index}"), 100, 100);
        established_shop(
            &mut fixture,
            BusinessKind::Hospitality,
            BusinessOwner::Organization(rival),
            district,
        );
    }
    let merchant = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Merchant company".into(),
            kind: OrganizationKind::Commercial,
        },
    )
    .unwrap();
    let district = ward(&mut fixture, "New ward", 100, 100);
    let target = established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Organization(merchant),
        district,
    );
    let quote = appraise_business(&fixture.registry, &fixture.state, target).unwrap();
    let price = i64::try_from(quote.reference_offer_cents * 150 / 100).unwrap();
    let reserve = fixture.registry.upkeep().per_member_daily().cents() * 7;
    fund_accounted_from_street(&mut fixture, price + reserve);
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(7 * DAY_MINUTES as u32));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        assert_eq!(
            apply_due_autonomous_acquisitions(&fixture.registry, state),
            vec![target],
        );
        assert_eq!(
            state.world().get_business(target).unwrap().owner(),
            BusinessOwner::Organization(rival),
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap(),
    );
}
