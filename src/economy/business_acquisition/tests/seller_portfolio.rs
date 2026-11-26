//! Seller-side commercial portfolio forecasts and private livelihood consent.

use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::DAY_MINUTES;
use crate::economy::business_economy_system::validate_suspend_business_economy;
use crate::world::world_system::{designate_player_organization, insert_character};
use crate::world::{AutonomyLevel, CharacterDraft};
use std::collections::BTreeMap;

#[test]
fn selling_closed_shop_accounts_for_reopened_competition_in_retained_wage_base() {
    let mut fixture = make_independent_fixture();
    let buyer = fixture.organization;
    designate_player_organization(&mut fixture.state, buyer).unwrap();
    let seller = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Local employer".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let mut executive = None;
    for index in 0..4 {
        let member = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: format!("Local employee {index}"),
                organization: Some(seller),
                supervisor: executive,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        executive.get_or_insert(member);
    }
    let district = ward(&mut fixture, "Shared commercial market", 100, 100);
    let retained = established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Organization(seller),
        district,
    );
    let target = established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Organization(seller),
        district,
    );
    validate_suspend_business_economy(&fixture.state, target)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let before_income = business_economy_system::forecast_live_business_net(
        &fixture.registry,
        &fixture.state,
        retained,
    )
    .unwrap()
    .cents() as i128;
    let mut reopened = fixture.state.clone();
    business_economy_system::validate_resume_business_economy(&fixture.registry, &reopened, target)
        .unwrap()
        .commit(&mut reopened)
        .unwrap();
    let after_income =
        business_economy_system::forecast_live_business_net(&fixture.registry, &reopened, retained)
            .unwrap()
            .cents() as i128;
    let payroll = i128::from(fixture.registry.upkeep().per_member_daily().cents()) * 4;
    assert!(before_income >= payroll && after_income < payroll);
    let reference = appraise_business(&fixture.registry, &fixture.state, target)
        .unwrap()
        .reference_offer_cents;
    let runway =
        (payroll - after_income) * super::super::seller_strategy::SALE_RUNWAY_DAYS + payroll * 7;
    assert!(reference < runway);
    fund_accounted_from_street(&mut fixture, i64::try_from(runway).unwrap());
    let draft = BusinessAcquisitionDraft {
        organization: buyer,
        business: target,
        funding_accounts: BTreeSet::from([fixture.accounted]),
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(
        matches!(
            validate_offer_business(
                &fixture.registry,
                &fixture.state,
                draft.clone(),
                Money::from_cents(i64::try_from(reference).unwrap())
            ),
            Err(BusinessAcquisitionError::OfferDeclined)
        ),
        "a reopened competitor would reduce the seller's wage base below payroll"
    );
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    let forecast = super::super::portfolio_forecast::PortfolioForecast::new(
        &fixture.registry,
        &fixture.state,
        seller,
        Some(target),
    )
    .unwrap();
    assert_eq!(forecast.daily_net(), after_income);
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_offer_business(
            &fixture.registry,
            state,
            draft.clone(),
            Money::from_cents(i64::try_from(runway).unwrap()),
        )
        .unwrap()
        .commit(&fixture.registry, state)
        .unwrap();
        assert_eq!(
            state.world().get_business(target).unwrap().owner(),
            BusinessOwner::Organization(buyer)
        );
        assert!(
            state
                .economy()
                .get_business_economy(target)
                .unwrap()
                .is_operating()
        );
        assert_eq!(
            business_economy_system::forecast_live_business_net(&fixture.registry, state, retained)
                .unwrap()
                .cents() as i128,
            after_income
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    build_save(&fixture.registry, &fixture.state).unwrap();
}

#[test]
fn proprietor_values_livelihood_lost_when_sold_closed_shop_reopens() {
    let mut fixture = make_independent_fixture();
    let seller = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Small shop proprietor".into(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let district = ward(&mut fixture, "Narrow retail market", 10, 10);
    let retained = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Character(seller),
        district,
    );
    let target = established_shop(
        &mut fixture,
        BusinessKind::Retail,
        BusinessOwner::Character(seller),
        district,
    );
    validate_suspend_business_economy(&fixture.state, target)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(
        business_economy_system::forecast_live_business_net(
            &fixture.registry,
            &fixture.state,
            retained
        )
        .unwrap()
            > Money::ZERO
    );
    let mut reopened = fixture.state.clone();
    business_economy_system::validate_resume_business_economy(&fixture.registry, &reopened, target)
        .unwrap()
        .commit(&mut reopened)
        .unwrap();
    assert!(
        business_economy_system::forecast_live_business_net(&fixture.registry, &reopened, retained)
            .unwrap()
            < Money::ZERO
    );
    let public = appraise_business(&fixture.registry, &fixture.state, target).unwrap();
    let mut no_livelihood = fixture.state.clone();
    validate_suspend_business_economy(&no_livelihood, retained)
        .unwrap()
        .commit(&mut no_livelihood)
        .unwrap();
    let minimum = |state: &AppState| {
        personal_seller_minimum(&fixture.registry, state, seller, target).unwrap()
    };
    let ordinary = minimum(&no_livelihood);
    let protected = minimum(&fixture.state);
    assert!(
        protected > ordinary,
        "sale would destroy the sole productive alternative"
    );
    assert_eq!(
        appraise_business(&fixture.registry, &no_livelihood, target).unwrap(),
        public
    );
    fund_accounted_from_street(&mut fixture, i64::try_from(protected).unwrap());
    let draft = BusinessAcquisitionDraft {
        organization: fixture.organization,
        business: target,
        funding_accounts: BTreeSet::from([fixture.accounted]),
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        validate_offer_business(
            &fixture.registry,
            &fixture.state,
            draft.clone(),
            Money::from_cents(i64::try_from(ordinary).unwrap())
        ),
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        validate_offer_business(
            &fixture.registry,
            state,
            draft.clone(),
            Money::from_cents(i64::try_from(protected).unwrap()),
        )
        .unwrap()
        .commit(&fixture.registry, state)
        .unwrap();
        assert_eq!(
            state.world().get_business(target).unwrap().owner(),
            BusinessOwner::Organization(fixture.organization)
        );
        assert!(
            state
                .economy()
                .get_business_economy(target)
                .unwrap()
                .is_operating()
        );
        assert!(
            business_economy_system::forecast_live_business_net(&fixture.registry, state, retained)
                .unwrap()
                < Money::ZERO
        );
        let paid: i128 = state
            .finance()
            .accounts_for(FinancialOwner::Character(seller))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| i128::from(account.balance().cents()))
            .sum();
        assert_eq!(paid, protected);
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn replacement_investment_uses_post_sale_market_and_matches_actual_conveyances() {
    use super::super::portfolio_forecast::PortfolioForecast;

    let mut fixture = make_independent_fixture();
    let buyer = fixture.organization;
    let seller = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Local vendor".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let district = ward(&mut fixture, "Three competing shops", 100, 100);
    established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Organization(seller),
        district,
    );
    let sold = established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Organization(seller),
        district,
    );
    let replacement = established_shop(
        &mut fixture,
        BusinessKind::Hospitality,
        BusinessOwner::Independent,
        district,
    );
    for business in [sold, replacement] {
        validate_suspend_business_economy(&fixture.state, business)
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap();
    }
    let appraisal = appraise_business(&fixture.registry, &fixture.state, replacement).unwrap();
    let before = bincode::serialize(&fixture.state).unwrap();
    let retained =
        PortfolioForecast::new(&fixture.registry, &fixture.state, seller, Some(sold)).unwrap();
    let predicted_gain = retained
        .purchase_gain(
            &fixture.registry,
            &fixture.state,
            replacement,
            district,
            &appraisal,
        )
        .unwrap();
    assert!(predicted_gain > 0);
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    let offer = Money::from_cents(
        i64::try_from(
            appraise_business(&fixture.registry, &fixture.state, sold)
                .unwrap()
                .reference_offer_cents,
        )
        .unwrap(),
    );
    fund_accounted_from_street(&mut fixture, offer.cents());
    validate_offer_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: buyer,
            business: sold,
            funding_accounts: BTreeSet::from([fixture.accounted]),
        },
        offer,
    )
    .unwrap()
    .commit(&fixture.registry, &mut fixture.state)
    .unwrap();
    let after_sale =
        PortfolioForecast::new(&fixture.registry, &fixture.state, seller, None).unwrap();
    assert_eq!(retained.daily_net(), after_sale.daily_net());
    assert_eq!(
        predicted_gain,
        after_sale
            .purchase_gain(
                &fixture.registry,
                &fixture.state,
                replacement,
                district,
                &appraisal
            )
            .unwrap(),
        "replacement forecast must be based on the market after the sale"
    );
    let funds = fixture
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(seller))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect();
    validate_acquire_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: seller,
            business: replacement,
            funding_accounts: funds,
        },
    )
    .unwrap()
    .commit(&fixture.registry, &mut fixture.state)
    .unwrap();
    let actual = PortfolioForecast::new(&fixture.registry, &fixture.state, seller, None).unwrap();
    assert_eq!(actual.daily_net() - after_sale.daily_net(), predicted_gain);
    validate_invariants(&fixture.state);
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn criminal_seller_accounts_for_administration_before_selling_wage_base() {
    let mut fixture = make_independent_fixture();
    let buyer = fixture.organization;
    designate_player_organization(&mut fixture.state, buyer).unwrap();
    let seller = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Large employer".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let mut executive = None;
    for index in 0..17 {
        let member = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: format!("Seller member {index}"),
                organization: Some(seller),
                supervisor: executive,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        executive.get_or_insert(member);
    }
    let mut shops = Vec::new();
    for index in 1..=5 {
        let district = ward(&mut fixture, &format!("Seller ward {index}"), 100, 100);
        shops.push(established_shop(
            &mut fixture,
            BusinessKind::Hospitality,
            BusinessOwner::Organization(seller),
            district,
        ));
    }
    let target = shops.pop().unwrap();
    let payroll = i128::from(fixture.registry.upkeep().per_member_daily().cents()) * 17;
    let mut gross = 0_i128;
    let mut net = 0_i128;
    for shop in shops {
        let quote = appraise_business(&fixture.registry, &fixture.state, shop).unwrap();
        let profit = quote.expected_cycle_net;
        let fee = business_economy_system::portfolio_overhead_charge(profit, 900);
        gross +=
            i128::from(profit.cents()) * i128::from(DAY_MINUTES) / i128::from(quote.cycle_minutes);
        net += i128::from(profit.checked_sub(fee).unwrap().cents()) * i128::from(DAY_MINUTES)
            / i128::from(quote.cycle_minutes);
    }
    assert!(gross >= payroll && net < payroll);
    let offer = appraise_business(&fixture.registry, &fixture.state, target)
        .unwrap()
        .reference_offer_cents;
    assert!(offer < (payroll - net) * 365 + payroll * 7);
    fund_accounted_from_street(&mut fixture, i64::try_from(offer).unwrap());
    let before = bincode::serialize(&fixture.state).unwrap();
    let result = validate_offer_business(
        &fixture.registry,
        &fixture.state,
        BusinessAcquisitionDraft {
            organization: buyer,
            business: target,
            funding_accounts: BTreeSet::from([fixture.accounted]),
        },
        Money::from_cents(i64::try_from(offer).unwrap()),
    );
    assert!(matches!(
        result,
        Err(BusinessAcquisitionError::OfferDeclined)
    ));
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    validate_invariants(&fixture.state);
}
