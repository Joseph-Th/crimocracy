use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::economy::business_acquisition::{
    BusinessAcquisitionDraft, BusinessAcquisitionError, appraise_business, validate_offer_business,
};
use crate::economy::business_economy_system::{
    apply_due_personal_owner_dividends, decide_business_cycle, validate_business_cycle_plan,
};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::world::character_life::apply_due_character_lives;
use crate::world::{BusinessOwner, CivicActivity, CivicProfession, OrganizationKind};
use std::collections::BTreeSet;

#[test]
fn actual_named_professions_have_daily_duties_and_private_career_histories() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let (shopkeeper, business) = city
        .state
        .world()
        .businesses()
        .find_map(|business| match business.owner() {
            BusinessOwner::Character(owner)
                if city
                    .state
                    .world()
                    .businesses_owned_by_character(owner)
                    .count()
                    == 1 =>
            {
                Some((owner, business.id()))
            }
            _ => None,
        })
        .expect("a generated city's private proprietors are named");
    let council = city
        .state
        .world()
        .organizations()
        .find(|record| record.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let politician = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    let officer = city
        .state
        .world()
        .characters_in_organization(city.police[0])
        .next()
        .unwrap()
        .id();
    assert_eq!(
        city.state.world().civic_profession(shopkeeper),
        Some(CivicProfession::BusinessOwner)
    );
    assert_eq!(
        city.state.world().civic_profession(officer),
        Some(CivicProfession::PoliceOfficer)
    );
    assert_eq!(
        city.state.world().civic_profession(politician),
        Some(CivicProfession::Politician)
    );
    for _ in 0..7 {
        city.state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        apply_due_character_lives(&mut city.state);
    }
    let shopkeeper_life = city.state.world().get_character(shopkeeper).unwrap().life();
    let politician_life = city.state.world().get_character(politician).unwrap().life();
    let officer_life = city.state.world().get_character(officer).unwrap().life();
    assert_eq!(
        shopkeeper_life.career_days(),
        6,
        "every seventh day is time off"
    );
    assert_eq!(politician_life.career_days(), 6);
    assert_eq!(officer_life.career_days(), 6);
    assert_eq!(
        shopkeeper_life.last_activity().unwrap().1,
        CivicActivity::BusinessManagement(business)
    );
    assert_eq!(
        politician_life.last_activity().unwrap().1,
        CivicActivity::Politics(council)
    );
    assert!(matches!(
        officer_life.last_activity().unwrap().1,
        CivicActivity::PolicePatrol(_) | CivicActivity::DetectiveWork(_)
    ));
    assert!(
        politician_life.professional_standing() > 50,
        "real public service develops an incumbent's political credibility"
    );
    assert!(
        officer_life.fatigue() < 12,
        "weekly recovery prevents linear exhaustion"
    );
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        restored.world().get_character(shopkeeper).unwrap().life(),
        shopkeeper_life
    );
    assert_eq!(
        restored.world().get_character(officer).unwrap().life(),
        officer_life
    );
}

#[test]
fn proprietor_receives_real_business_dividends_and_personally_decides_a_sale() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let (shopkeeper, property) = city
        .state
        .world()
        .businesses()
        .find_map(|business| {
            let BusinessOwner::Character(owner) = business.owner() else {
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
            let forecast = crate::economy::business_economy_system::forecast_normal_business_net(
                &registry,
                &city.state,
                business.id(),
            )
            .ok()?;
            (forecast.cents() > 0).then_some((owner, business.id()))
        })
        .expect("a private proprietor operates a forecast-profitable shop");
    // The default generated owner starts without an empty bank account. This
    // explicit account lets the test follow exact daily debit/credit changes
    // while a separate proprietor cohort exercises automatic openings.
    assert!(
        city.state
            .finance()
            .accounts_for(FinancialOwner::Character(shopkeeper))
            .next()
            .is_none()
    );
    let private_cash = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Character(shopkeeper),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let operating = city
        .state
        .economy()
        .get_business_economy(property)
        .unwrap()
        .operating_account();
    let mut distributed = false;
    for _ in 0..45 {
        city.state
            .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        crate::opportunities::opportunity_system::apply_opportunity_expiry(
            &registry,
            &mut city.state,
        )
        .unwrap();
        if let Ok(plan) = decide_business_cycle(&registry, &city.state, property, 0) {
            validate_business_cycle_plan(&city.state, plan)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
        let before_shop = city
            .state
            .finance()
            .get_account(operating)
            .unwrap()
            .balance()
            .cents();
        let before_person = city
            .state
            .finance()
            .get_account(private_cash)
            .unwrap()
            .balance()
            .cents();
        apply_due_personal_owner_dividends(&mut city.state).unwrap();
        let after_shop = city
            .state
            .finance()
            .get_account(operating)
            .unwrap()
            .balance()
            .cents();
        let after_person = city
            .state
            .finance()
            .get_account(private_cash)
            .unwrap()
            .balance()
            .cents();
        assert_eq!(
            before_shop - after_shop,
            after_person - before_person,
            "a proprietor's earnings must be balanced against the real store's till"
        );
        if after_person > before_person {
            distributed = true;
        }
        apply_due_character_lives(&mut city.state);
    }
    assert!(
        distributed,
        "the actual proprietor must eventually draw earned positive surplus"
    );
    let saved = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        saved.world().get_character(shopkeeper).unwrap().life(),
        city.state.world().get_character(shopkeeper).unwrap().life()
    );

    let reference = appraise_business(&registry, &city.state, property)
        .unwrap()
        .reference_offer_cents;
    let accepted = i64::try_from(reference.max(1).saturating_mul(6))
        .expect("small-city appraisal fits in real accounted funds");
    let initial_seller_money = city
        .state
        .finance()
        .get_account(private_cash)
        .unwrap()
        .balance()
        .cents();
    let buyer_account = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let settlement = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(city.player),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &city.state,
        LedgerTransactionDraft {
            occurred_at: city.state.now(),
            memo: "Capital for a private shop acquisition".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-accepted),
                },
                LedgerPosting {
                    account: buyer_account,
                    amount: Money::from_cents(accepted),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let draft = || BusinessAcquisitionDraft {
        organization: city.player,
        business: property,
        funding_accounts: BTreeSet::from([buyer_account]),
    };
    let before_rejection =
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_offer_business(&registry, &city.state, draft(), Money::from_cents(1)).err(),
        Some(BusinessAcquisitionError::OfferDeclined)
    );
    assert_eq!(
        before_rejection,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    let sale =
        validate_offer_business(&registry, &city.state, draft(), Money::from_cents(accepted))
            .unwrap()
            .commit(&registry, &mut city.state)
            .unwrap();
    assert_eq!(sale.price.cents(), accepted);
    assert_eq!(
        city.state.world().get_business(property).unwrap().owner(),
        BusinessOwner::Organization(city.player)
    );
    assert_eq!(
        city.state
            .finance()
            .get_account(private_cash)
            .unwrap()
            .balance()
            .cents(),
        initial_seller_money + accepted
    );
    assert_eq!(
        city.state.world().civic_profession(shopkeeper),
        Some(CivicProfession::Resident),
        "changing an actual property title changes the person's profession"
    );
    validate_invariants(&city.state);
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    restored.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    apply_due_character_lives(&mut city.state);
    apply_due_character_lives(&mut restored);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
