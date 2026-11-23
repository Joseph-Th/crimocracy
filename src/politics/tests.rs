use super::*;
use crate::build_registry;
use crate::city_generation::{CityScale, generate_city};
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::{DAY_MINUTES, SimDuration};
use crate::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::politics::political_system::{
    PoliticalError, apply_due_political_campaigns, controls_district, dismiss_specialist,
    rival_pressure, validate_hire_specialist, validate_invest_influence,
};
use crate::world::world_system::insert_business;
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, OrganizationKind,
};

fn fund(state: &mut crate::AppState, owner: OrganizationId, cents: i64) {
    let clean = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let clearing = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Political fixture capitalization".into(),
            postings: vec![
                LedgerPosting {
                    account: clean,
                    amount: Money::from_cents(cents),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-cents),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
}

fn advance_day(registry: &crate::Registry, state: &mut crate::AppState) {
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(registry, state).unwrap();
    apply_due_political_campaigns(registry, state);
}

fn operating_rival_wards(state: &crate::AppState, owner: OrganizationId) -> Vec<NeighborhoodId> {
    state
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
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[test]
fn political_investment_requires_clean_funds_and_loses_influence_without_staff() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(3_411)).unwrap();
    let home = city.home;
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(matches!(
        validate_invest_influence(&city.state, home, 500_000),
        Err(PoliticalError::InsufficientFunds)
    ));
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    fund(&mut city.state, city.player, 2_000_000);
    assert_eq!(
        validate_invest_influence(&city.state, home, 500_001).err(),
        Some(PoliticalError::InvalidInvestment)
    );
    validate_invest_influence(&city.state, home, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let campaign = city.state.politics().organization(city.player).unwrap();
    assert_eq!(campaign.spent_cents(), 500_000);
    assert_eq!(campaign.district(home).unwrap().influence(), 100);
    advance_day(&registry, &mut city.state);
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .district(home)
            .unwrap()
            .influence(),
        95
    );
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn specialist_requires_delegated_political_manager_and_paid_salary_to_grow() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(5_611)).unwrap();
    let district = city.home;
    fund(&mut city.state, city.player, 1_116_000);
    let manager = city
        .state
        .world()
        .characters_in_organization(city.player)
        .find(|person| {
            person.supervisor().is_some()
                && city
                    .state
                    .delegation()
                    .active_for_manager(person.id())
                    .is_some()
        })
        .unwrap()
        .id();
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_hire_specialist(&city.state, district, manager, SpecialistKind::Strategist).err(),
        Some(PoliticalError::InvalidManager)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    let mandate = city.state.delegation().active_for_manager(manager).unwrap();
    let mut scopes = mandate.scopes().clone();
    scopes.insert(ResponsibilityScope::Function(
        ResponsibilityFunction::Political,
    ));
    scopes.insert(ResponsibilityScope::Neighborhood(district));
    validate_revise_mandate(
        &city.state,
        mandate.id(),
        MandateRevisionDraft {
            scopes,
            standing_orders: mandate.standing_orders().clone(),
            budget: mandate.budget(),
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    if city
        .state
        .world()
        .get_character(manager)
        .unwrap()
        .autonomy()
        != AutonomyLevel::Delegated
    {
        crate::world::world_system::validate_set_character_autonomy(
            &city.state,
            city.player,
            manager,
            AutonomyLevel::Delegated,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    validate_invest_influence(&city.state, district, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_hire_specialist(&city.state, district, manager, SpecialistKind::Strategist)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        validate_hire_specialist(&city.state, district, manager, SpecialistKind::Strategist).err(),
        Some(PoliticalError::SpecialistExists)
    );
    advance_day(&registry, &mut city.state);
    let campaign = city.state.politics().organization(city.player).unwrap();
    let output =
        political_system::specialist_output(&city.state, manager, SpecialistKind::Strategist);
    assert_eq!(
        campaign.district(district).unwrap().influence(),
        95 + output.0
    );
    assert_eq!(campaign.district(district).unwrap().grassroots(), output.1);
    assert_eq!(
        campaign.district(district).unwrap().institutions(),
        output.2
    );
    assert_eq!(campaign.spent_cents(), 1_116_000);
    let saved = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(saved.politics(), city.state.politics());
    // Payroll is exhausted. A retained specialist cannot manufacture free
    // daily support; influence still erodes without its salary.
    advance_day(&registry, &mut city.state);
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .district(district)
            .unwrap()
            .influence(),
        90 + output.0
    );
    dismiss_specialist(&mut city.state, district, SpecialistKind::Strategist).unwrap();
    advance_day(&registry, &mut city.state);
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .district(district)
            .unwrap()
            .influence(),
        85 + output.0
    );
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .spent_cents(),
        1_116_000
    );
    validate_invariants(&city.state);
}

#[test]
fn purchased_machine_influence_alone_cannot_win_a_ward_and_rivals_can_contest_it() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_217)).unwrap();
    let rival = city
        .state
        .world()
        .organizations()
        .find(|organization| {
            organization.kind() == OrganizationKind::Criminal && organization.id() != city.player
        })
        .unwrap()
        .id();
    let district = city
        .state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .neighborhood();
    fund(&mut city.state, city.player, 12_000_000);
    fund(&mut city.state, rival, 12_000_000);
    validate_invest_influence(&city.state, district, 4_500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert!(
        !controls_district(&city.state, city.player, district),
        "purchased machine influence cannot replace grassroots and institutional constituencies"
    );
    political_system::validate_invest_for(&city.state, rival, district, 5_000_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(rival_pressure(&city.state, city.player, district), 1_000);
    assert!(!controls_district(&city.state, city.player, district));
    advance_day(&registry, &mut city.state);
    assert!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .district(district)
            .unwrap()
            .leading_since()
            .is_none()
    );
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn autonomous_rivals_spend_clean_money_weekly_in_the_players_contested_ward() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_218)).unwrap();
    let rival = city
        .state
        .world()
        .organizations()
        .find(|organization| {
            organization.kind() == OrganizationKind::Criminal && organization.id() != city.player
        })
        .unwrap()
        .id();
    let district = city
        .state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .neighborhood();
    fund(&mut city.state, city.player, 500_000);
    validate_invest_influence(&city.state, district, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    for _ in 0..6 {
        advance_day(&registry, &mut city.state);
    }
    assert_eq!(rival_pressure(&city.state, city.player, district), 0);
    fund(&mut city.state, rival, 500_000);
    advance_day(&registry, &mut city.state);
    assert_eq!(rival_pressure(&city.state, city.player, district), 100);
    assert_eq!(
        city.state
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents(),
        500_000
    );
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    assert!(
        rival_pressure(&city.state, city.player, district) < 100,
        "an unfunded rival's influence must also decay"
    );
    assert_eq!(
        city.state
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents(),
        500_000
    );
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn financed_rival_counter_campaign_scales_with_visible_player_reach() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_218)).unwrap();
    let rival = city
        .state
        .world()
        .organizations()
        .find(|organization| {
            organization.kind() == OrganizationKind::Criminal && organization.id() != city.player
        })
        .unwrap()
        .id();
    let contested = city
        .state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .neighborhood();
    fund(&mut city.state, rival, 3_000_000);
    fund(&mut city.state, city.player, 1_000_000);
    validate_invest_influence(&city.state, contested, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let mut quiet = city.state.clone();
    let mut prominent = city.state;
    assert_eq!(
        crate::world::organization_pressure::public_prominence_tier(&quiet, city.player),
        0
    );
    for index in 0..12 {
        insert_business(
            &registry,
            &mut prominent,
            BusinessDraft {
                name: format!("Visible Public Holding {index}"),
                kind: BusinessKind::Retail,
                functions: std::collections::BTreeSet::from([
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::ProfessionalRecords,
                ]),
                neighborhood: city.home,
                owner: BusinessOwner::Organization(city.player),
            },
        )
        .unwrap();
    }
    let prominence =
        crate::world::organization_pressure::public_prominence_tier(&prominent, city.player);
    assert!(prominence >= 3);
    for _ in 0..7 {
        advance_day(&registry, &mut quiet);
        advance_day(&registry, &mut prominent);
    }
    let influence = |state: &crate::AppState| {
        state
            .politics()
            .organization(rival)
            .and_then(|campaign| campaign.district(contested))
            .map_or(0, DistrictCampaign::influence)
    };
    assert_eq!(influence(&quiet), 100);
    assert_eq!(influence(&prominent), 100 + u16::from(prominence) * 20);
    assert!(
        prominent
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents()
            > quiet.politics().organization(rival).unwrap().spent_cents(),
        "escalation must be paid from rival funds, not generated influence"
    );
    validate_state_against_registry(&registry, &prominent).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &prominent).unwrap()).unwrap();
    assert_eq!(restored.politics(), prominent.politics());
}

#[test]
fn autonomous_rival_campaign_moves_between_actual_competing_wards() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_219)).unwrap();
    let rival = city.rivals[0];
    let districts = operating_rival_wards(&city.state, rival);
    assert!(
        districts.len() >= 2,
        "rival must have two operating commercial wards"
    );
    let (first, second) = (districts[0], districts[1]);
    fund(&mut city.state, city.player, 2_500_000);
    fund(&mut city.state, rival, 5_500_000);
    validate_invest_influence(&city.state, first, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_invest_influence(&city.state, second, 2_000_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    let influence = |state: &crate::AppState, district| {
        state
            .politics()
            .organization(rival)
            .and_then(|campaign| campaign.district(district))
            .map_or(0, DistrictCampaign::influence)
    };
    assert_eq!(influence(&city.state, first), 0);
    assert_eq!(influence(&city.state, second), 100);
    // After the strongest challenge is defended, redirect the finite weekly
    // budget toward the next exposed ward, not an arbitrary lowest-ID ward.
    political_system::validate_invest_for(&city.state, rival, second, 4_500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
        advance_day(&registry, &mut restored);
    }
    assert_eq!(influence(&city.state, first), 100);
    assert_eq!(influence(&restored, first), 100);
    assert_eq!(influence(&restored, second), influence(&city.state, second));
    assert_eq!(
        city.state
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents(),
        5_500_000
    );
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
}

#[test]
fn rival_with_a_well_defended_weak_challenge_invests_in_another_ward() {
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_219)).unwrap();
    let rival = city.rivals[0];
    let districts = operating_rival_wards(&city.state, rival);
    assert!(districts.len() >= 2);
    let (other, contested) = (districts[0], districts[1]);
    fund(&mut city.state, city.player, 500_000);
    fund(&mut city.state, rival, 5_500_000);
    validate_invest_influence(&city.state, contested, 500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    political_system::validate_invest_for(&city.state, rival, contested, 4_500_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    let campaign = city.state.politics().organization(rival).unwrap();
    assert_eq!(campaign.district(other).unwrap().influence(), 100);
    assert!(campaign.district(contested).unwrap().influence() > 600);
    assert_eq!(campaign.spent_cents(), 5_000_000);
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
}
