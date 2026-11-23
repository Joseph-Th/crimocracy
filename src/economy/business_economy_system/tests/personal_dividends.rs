//! Private proprietor distributions use the same account capacity rules as criminal treasuries.

use super::*;
use crate::city_generation::{CityScale, generate_city};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::economy::business_economy_system::profit_sweep::available_business_profit;
use crate::economy::business_economy_system::{
    apply_due_personal_owner_dividends, decide_business_cycle, validate_business_cycle_plan,
};

#[test]
fn private_dividends_fill_existing_combined_account_headroom_before_opening_another() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(562_365)).unwrap();
    let (person, property) = city
        .state
        .world()
        .businesses()
        .find_map(|business| {
            let BusinessOwner::Character(person) = business.owner() else {
                return None;
            };
            let forecast =
                super::super::forecast_normal_business_net(&registry, &city.state, business.id())
                    .ok()?;
            (forecast > Money::ZERO).then_some((person, business.id()))
        })
        .expect("a city contains an earning private proprietor");
    let operating = city
        .state
        .economy()
        .get_business_economy(property)
        .unwrap()
        .operating_account();

    // Let actual shop settlements create a withdrawable surplus instead of
    // depositing fictitious income into an unearned business till.
    let surplus = loop {
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
        let economy = city.state.economy().get_business_economy(property).unwrap();
        let amount = available_business_profit(
            city.state
                .finance()
                .get_account(operating)
                .unwrap()
                .spendable_balance(),
            economy.operating_capital_floor(),
        );
        if amount.cents() >= 4 {
            break amount.cents();
        }
        assert!(city.state.now().as_minutes() < 45 * DAY_MINUTES);
    };
    let headrooms = [surplus / 2, surplus - surplus / 2];
    let owner = FinancialOwner::Character(person);
    let mut accounts = Vec::new();
    for headroom in headrooms {
        let destination = insert_account(
            &mut city.state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::AccountedFunds,
            },
        )
        .unwrap();
        let settlement = insert_account(
            &mut city.state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        let funding = Money::from_cents(i64::MAX - headroom);
        validate_record_transaction(
            &city.state,
            LedgerTransactionDraft {
                occurred_at: city.state.now(),
                memo: "Proprietor account capacity fixture".into(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: funding.checked_neg().unwrap(),
                    },
                    LedgerPosting {
                        account: destination,
                        amount: funding,
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        accounts.push(destination);
    }
    let before_till = city
        .state
        .finance()
        .get_account(operating)
        .unwrap()
        .balance();
    let before_account_count = city.state.finance().accounts_for(owner).count();
    assert!(headrooms.iter().all(|&headroom| headroom < surplus));
    apply_due_personal_owner_dividends(&mut city.state)
        .unwrap()
        .expect("earned profits distribute on the day boundary");

    for account in accounts {
        assert_eq!(
            city.state
                .finance()
                .get_account(account)
                .unwrap()
                .balance()
                .cents(),
            i64::MAX,
            "both existing bank accounts receive the amount they can hold"
        );
    }
    assert_eq!(
        before_till.cents()
            - city
                .state
                .finance()
                .get_account(operating)
                .unwrap()
                .balance()
                .cents(),
        surplus,
    );
    assert_eq!(
        city.state.finance().accounts_for(owner).count(),
        before_account_count,
        "a third account is unnecessary when combined headroom covers the dividend"
    );
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap(),
    );
}
