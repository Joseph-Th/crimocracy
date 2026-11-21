use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::{TickError, run_tick};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::world::world_system::{
    designate_player_organization, insert_character, insert_organization,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

fn retirement_fixture() -> (Registry, AppState, OrganizationId, CharacterId, CharacterId) {
    let registry = build_registry();
    let mut state = AppState::new(711);
    let owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Retiring outfit".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut state, owner).unwrap();
    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Boss".into(),
            organization: Some(owner),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Member".into(),
            organization: Some(owner),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    (registry, state, owner, boss, member)
}

fn fund(state: &mut AppState, owner: OrganizationId, cents: i64) {
    let clean = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let source = insert_account(
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
            memo: "Test seed capital".into(),
            postings: vec![
                LedgerPosting {
                    account: clean,
                    amount: Money::from_cents(cents),
                },
                LedgerPosting {
                    account: source,
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

/// Wage obligations increase with real personnel, even when profits do not.
fn hire_additional_staff(
    state: &mut AppState,
    organization: OrganizationId,
    boss: CharacterId,
    count: usize,
) {
    for index in 0..count {
        insert_character(
            state,
            CharacterDraft {
                name: format!("Additional employee {index}"),
                organization: Some(organization),
                supervisor: Some(boss),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
}

#[test]
fn retirement_rejects_unfunded_settlement_without_mutating_books() {
    let (registry, mut state, owner, _, _) = retirement_fixture();
    fund(&mut state, owner, RETIREMENT_NEST_EGG.cents());
    let before = bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).err(),
        Some(VictoryError::RetirementNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
}

#[test]
fn funded_retirement_pays_boss_and_staff_and_survives_reload_as_terminal() {
    let (registry, mut state, owner, boss, member) = retirement_fixture();
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .unwrap();
    let required = RETIREMENT_NEST_EGG.checked_add(severance).unwrap();
    fund(&mut state, owner, required.cents() + 200_000);
    let plan = validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).unwrap();
    let result = plan.commit(&registry, &mut state).unwrap();
    assert_eq!(result.path, VictoryPath::FundedRetirement);
    assert!(result.settlement.is_some());
    assert_eq!(
        state.history().campaign_victory(owner).unwrap().id(),
        result.history
    );
    assert_eq!(
        state.reports().get_report(result.report).unwrap().kind(),
        ReportKind::CampaignVictory
    );
    for (person, expected) in [(boss, RETIREMENT_NEST_EGG), (member, severance)] {
        let paid: i64 = state
            .finance()
            .accounts_for(FinancialOwner::Character(person))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| account.balance().cents())
            .sum();
        assert_eq!(paid, expected.cents());
    }
    let clean_left: i64 = state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.balance().cents())
        .sum();
    assert_eq!(clean_left, 200_000);
    validate_invariants(&state);
    validate_state_against_registry(&registry, &state).unwrap();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        restored.history().campaign_victory(owner).unwrap().id(),
        result.history
    );
    let before = bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap();
    assert_eq!(
        run_tick(&registry, &mut restored),
        Err(TickError::CampaignWon {
            path: VictoryPath::FundedRetirement
        })
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
    assert!(matches!(
        validate_claim_victory(&registry, &restored, VictoryPath::FundedRetirement).err(),
        Some(VictoryError::AlreadyWon)
    ));
}

#[test]
fn victory_token_rejects_clock_drift_without_paying_anyone() {
    let (registry, mut state, owner, _, _) = retirement_fixture();
    let required = RETIREMENT_NEST_EGG
        .checked_add(
            registry
                .upkeep()
                .per_member_daily()
                .checked_mul(RETIREMENT_SEVERANCE_DAYS)
                .unwrap(),
        )
        .unwrap();
    fund(&mut state, owner, required.cents() + 1_000);
    let token = validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).unwrap();
    state.advance_clock(crate::core::time::SimDuration::ONE_MINUTE);
    let before = bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        token.commit(&registry, &mut state).err(),
        Some(VictoryError::Stale)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
}

#[test]
fn political_victory_requires_contact_and_citywide_real_estate_then_ends_campaign() {
    use crate::city_generation::{CityScale, generate_city};
    use crate::contacts::contact_system::{InstitutionalContactDraft, validate_establish_contact};
    use crate::economy::business_acquisition::{
        BusinessAcquisitionDraft, validate_acquire_business,
    };
    use crate::world::BusinessOwner;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let council = city
        .state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::Political)
        .unwrap()
        .id();
    let official = city
        .state
        .world()
        .characters_in_organization(council)
        .next()
        .unwrap()
        .id();
    let standing = progress(&registry, &city.state).unwrap();
    assert_eq!(standing.active_political_contacts, 0);
    assert!(matches!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    ));
    validate_establish_contact(
        &city.state,
        InstitutionalContactDraft {
            sponsor: city.player,
            handler: city.boss,
            contact: official,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .active_political_contacts,
        1
    );

    // Purchase actual businesses in multiple districts. A cheap same-day
    // purchase plus the newly opened contact must not be a terminal win.
    fund(&mut city.state, city.player, 20_000_000);
    let funding = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect::<BTreeSet<_>>();
    let districts = city
        .state
        .world()
        .neighborhoods()
        .map(|district| district.id())
        .filter(|district| *district != city.home)
        .take(2)
        .collect::<Vec<_>>();
    for district in districts {
        let listings = city
            .state
            .world()
            .public_business_directory()
            .filter(|(id, _, _, location)| {
                *location == district
                    && city.state.world().get_business(*id).unwrap().owner()
                        == BusinessOwner::Independent
            })
            .take(5)
            .map(|(id, _, _, _)| id)
            .collect::<Vec<_>>();
        assert!(!listings.is_empty());
        for listing in listings {
            validate_acquire_business(
                &registry,
                &city.state,
                BusinessAcquisitionDraft {
                    organization: city.player,
                    business: listing,
                    funding_accounts: funding.clone(),
                },
            )
            .unwrap()
            .commit(&registry, &mut city.state)
            .unwrap();
        }
    }
    let early = progress(&registry, &city.state).unwrap();
    assert!(early.owned_operating_business_districts >= early.required_political_districts);
    assert_eq!(early.established_political_contacts, 0);
    assert_eq!(early.established_business_districts, 0);
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );

    // Isolate the victory requirement: settle the owned portfolio through the
    // economy owner at each real business cycle, without the unrelated city's
    // autonomous operations. The full minute pipeline is exercised separately.
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan,
    };
    use crate::opportunities::OpportunityStatus;
    use crate::opportunities::opportunity_system::validate_dismiss_opportunity;
    let open = city
        .state
        .opportunities()
        .opportunities_for_organization(city.player)
        .filter(|opportunity| opportunity.status() == OpportunityStatus::Open)
        .map(|opportunity| opportunity.id())
        .collect::<Vec<_>>();
    for opportunity in open {
        validate_dismiss_opportunity(&registry, &city.state, opportunity)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    for day in 1..=VICTORY_ESTABLISHMENT_DAYS {
        city.state
            .advance_clock(crate::core::time::SimDuration::from_minutes(
                DAY_MINUTES as u32,
            ));
        let businesses = city
            .state
            .world()
            .businesses_owned_by_organization(city.player)
            .map(|business| business.id())
            .collect::<Vec<_>>();
        for business in businesses {
            if let Ok(plan) = decide_business_cycle(&registry, &city.state, business, 0) {
                validate_business_cycle_plan(&city.state, plan)
                    .unwrap()
                    .commit(&mut city.state)
                    .unwrap();
            }
        }
        if day == POLITICAL_CONTACT_DAYS {
            assert_eq!(
                progress(&registry, &city.state)
                    .unwrap()
                    .established_political_contacts,
                1
            );
            assert_eq!(
                progress(&registry, &city.state)
                    .unwrap()
                    .established_business_districts,
                0
            );
        }
        if day == VICTORY_ESTABLISHMENT_DAYS - 1 {
            assert_eq!(
                progress(&registry, &city.state)
                    .unwrap()
                    .established_business_districts,
                0
            );
        }
    }
    let established = progress(&registry, &city.state).unwrap();
    assert!(
        established.established_business_districts >= established.required_political_districts,
        "qualifying productive business districts: {:?}",
        established
    );
    assert!(established.legitimate_self_sufficient);
    assert!(established.legitimate_daily_net >= established.daily_payroll);
    let restored_before_claim =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        progress(&registry, &restored_before_claim).unwrap(),
        established
    );
    let mut overstaffed =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    hire_additional_staff(&mut overstaffed, city.player, city.boss, 50);
    let underfunded = progress(&registry, &overstaffed).unwrap();
    assert!(underfunded.established_business_districts >= underfunded.required_political_districts);
    assert!(!underfunded.legitimate_self_sufficient);
    let before = bincode::serialize(&build_save(&registry, &overstaffed).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &overstaffed, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &overstaffed).unwrap()).unwrap()
    );

    // Historical successes cannot fund a permanent win if no new cycles have
    // been settled in the last month. This is a read-only progress probe.
    let mut stale = restored_before_claim;
    stale.set_now_for_test(SimTime::from_minutes(
        city.state.now().as_minutes() + (VICTORY_INCOME_WINDOW_DAYS + 1) * DAY_MINUTES,
    ));
    let stale_progress = progress(&registry, &stale).unwrap();
    assert_eq!(stale_progress.established_business_districts, 0);
    assert_eq!(stale_progress.legitimate_daily_net, Money::ZERO);
    assert_eq!(
        validate_claim_victory(&registry, &stale, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    let outcome = validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower)
        .unwrap()
        .commit(&registry, &mut city.state)
        .unwrap();
    assert_eq!(outcome.path, VictoryPath::PoliticalPower);
    assert!(outcome.settlement.is_none());
    assert!(matches!(
        run_tick(&registry, &mut city.state),
        Err(TickError::CampaignWon {
            path: VictoryPath::PoliticalPower
        })
    ));
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.campaign_victory().unwrap().id(), outcome.history);
}

#[test]
fn uncontested_territory_and_superior_enterprise_strength_claim_underboss() {
    use crate::city_generation::{CityGenerationConfig, generate_city};
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
    use crate::enterprises::enterprise_execution::validate_establish_enterprise;
    use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
    use crate::world::world_system::validate_set_character_autonomy;

    let registry = build_registry();
    let mut city = generate_city(
        &registry,
        CityGenerationConfig {
            seed: 405,
            districts: 4,
            businesses_per_district: 8,
            rivals: 1,
        },
    )
    .unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .required_underboss_districts,
        3
    );
    assert!(matches!(
        validate_claim_victory(&registry, &city.state, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    ));
    let home = city.home;
    let districts = city
        .state
        .world()
        .neighborhoods()
        .map(|district| district.id())
        .filter(|district| *district != home)
        .collect::<Vec<_>>();
    // Rival starts in the next district. Establish distinct player-led rackets in
    // two other districts, keeping the rival's uncontested home independent.
    let empty_districts = &districts[1..];
    let managers = city
        .state
        .world()
        .characters_in_organization(city.player)
        .filter(|member| {
            member.id() != city.boss
                && city
                    .state
                    .delegation()
                    .active_for_manager(member.id())
                    .is_none()
        })
        .take(2)
        .map(|member| member.id())
        .collect::<Vec<_>>();
    assert_eq!(managers.len(), 2);
    let cash = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && account.available_payment_balance() > Money::ZERO
        })
        .unwrap()
        .id();
    for (&manager, &district) in managers.iter().zip(empty_districts) {
        if city
            .state
            .world()
            .get_character(manager)
            .unwrap()
            .autonomy()
            != AutonomyLevel::Delegated
        {
            validate_set_character_autonomy(
                &city.state,
                city.player,
                manager,
                AutonomyLevel::Delegated,
            )
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        }
        let scope = ResponsibilityScope::Neighborhood(district);
        let mandate = validate_assign_mandate(
            &city.state,
            MandateDraft {
                organization: city.player,
                manager,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        let settlement = insert_account(
            &mut city.state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(city.player),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_establish_enterprise(
            &registry,
            &city.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization: city.player,
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope,
                },
                location: EnterpriseLocation::Neighborhood(district),
                supporting_businesses: BTreeSet::new(),
                cash_account: cash,
                settlement_account: settlement,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    let standing = progress(&registry, &city.state).unwrap();
    assert_eq!(standing.own_active_racket_districts, 3);
    assert_eq!(standing.established_racket_districts, 0);
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    validate_invariants(&city.state);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        progress(&registry, &restored)
            .unwrap()
            .established_racket_districts,
        0
    );
    // Elapsed clock time by itself is not operating proof; no settlements
    // have occurred for the newly placed rackets.
    let mut clock_only = restored;
    clock_only.set_now_for_test(SimTime::from_minutes(
        VICTORY_ESTABLISHMENT_DAYS * DAY_MINUTES,
    ));
    assert_eq!(
        progress(&registry, &clock_only)
            .unwrap()
            .established_racket_districts,
        0
    );

    use crate::enterprises::enterprise_execution::{
        EnterpriseCycleRandomness, decide_enterprise_cycle, validate_enterprise_cycle_plan,
    };
    use crate::opportunities::OpportunityStatus;
    use crate::opportunities::opportunity_system::validate_dismiss_opportunity;
    let open = city
        .state
        .opportunities()
        .opportunities_for_organization(city.player)
        .filter(|opportunity| opportunity.status() == OpportunityStatus::Open)
        .map(|opportunity| opportunity.id())
        .collect::<Vec<_>>();
    for opportunity in open {
        validate_dismiss_opportunity(&registry, &city.state, opportunity)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    for day in 1..=VICTORY_ESTABLISHMENT_DAYS {
        city.state
            .advance_clock(crate::core::time::SimDuration::from_minutes(
                DAY_MINUTES as u32,
            ));
        let rackets = city
            .state
            .enterprises()
            .enterprises_for_organization(city.player)
            .map(|enterprise| enterprise.id())
            .collect::<Vec<_>>();
        for racket in rackets {
            let plan = decide_enterprise_cycle(
                &registry,
                &city.state,
                racket,
                EnterpriseCycleRandomness::new(
                    0,
                    EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
                ),
            )
            .unwrap();
            validate_enterprise_cycle_plan(&city.state, plan)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
        if day == VICTORY_ESTABLISHMENT_DAYS - 1 {
            assert_eq!(
                progress(&registry, &city.state)
                    .unwrap()
                    .established_racket_districts,
                0
            );
        }
    }
    let established = progress(&registry, &city.state).unwrap();
    assert!(established.established_racket_districts >= established.required_underboss_districts);
    assert!(established.racket_self_sufficient);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), established);
    let mut overstaffed =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    hire_additional_staff(&mut overstaffed, city.player, city.boss, 50);
    let underfunded = progress(&registry, &overstaffed).unwrap();
    assert!(underfunded.established_racket_districts >= underfunded.required_underboss_districts);
    assert!(!underfunded.racket_self_sufficient);
    let before = bincode::serialize(&build_save(&registry, &overstaffed).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &overstaffed, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &overstaffed).unwrap()).unwrap()
    );
    let outcome = validate_claim_victory(&registry, &city.state, VictoryPath::CityUnderboss)
        .unwrap()
        .commit(&registry, &mut city.state)
        .unwrap();
    assert_eq!(outcome.path, VictoryPath::CityUnderboss);
    assert!(matches!(
        run_tick(&registry, &mut city.state),
        Err(TickError::CampaignWon {
            path: VictoryPath::CityUnderboss
        })
    ));
    validate_invariants(&city.state);
    assert_eq!(
        restore_save(&registry, build_save(&registry, &city.state).unwrap())
            .unwrap()
            .campaign_victory()
            .unwrap()
            .id(),
        outcome.history
    );
}

#[test]
fn established_crews_must_outnumber_combined_rivals_not_only_win_a_plurality() {
    use crate::world::territory_influence::{NeighborhoodInfluenceSummary, TerritoryStanding};

    let player = OrganizationId::from_raw(1);
    let rival_a = OrganizationId::from_raw(2);
    let rival_b = OrganizationId::from_raw(3);
    let district = NeighborhoodInfluenceSummary {
        standings: vec![
            TerritoryStanding {
                organization: player,
                active_enterprises: 4,
            },
            TerritoryStanding {
                organization: rival_a,
                active_enterprises: 1,
            },
            TerritoryStanding {
                organization: rival_b,
                active_enterprises: 1,
            },
        ],
    };
    assert_eq!(district.economic_leader(), Some(player));
    assert!(
        !mature_territorial_majority(&district, player, 2),
        "fresh rackets cannot turn an established tie into underboss dominance"
    );
    assert!(mature_territorial_majority(&district, player, 3));
    assert!(!mature_territorial_majority(&district, rival_a, 1));
}

#[test]
fn long_owned_low_margin_storefront_does_not_create_political_leverage() {
    use crate::city_generation::{CityScale, generate_city};
    use crate::economy::business_acquisition::{
        BusinessAcquisitionDraft, validate_acquire_business,
    };
    use crate::economy::business_economy_system::{
        decide_business_cycle, forecast_normal_business_net, validate_business_cycle_plan,
    };
    use crate::opportunities::OpportunityStatus;
    use crate::opportunities::opportunity_system::validate_dismiss_opportunity;
    use crate::world::{BusinessKind, BusinessOwner};

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(947)).unwrap();
    let storefront = city
        .state
        .world()
        .public_business_directory()
        .map(|(id, _, _, _)| id)
        .find(|id| {
            let business = city.state.world().get_business(*id).unwrap();
            business.kind() == BusinessKind::SoupKitchen
                && business.neighborhood() != city.home
                && business.owner() == BusinessOwner::Independent
        })
        .expect("the generated city must have a purchasable outlying kitchen");
    let one_wage = registry.upkeep().per_member_daily().cents();
    let forecast = forecast_normal_business_net(&registry, &city.state, storefront).unwrap();
    assert!(forecast > Money::ZERO);
    assert!(forecast.cents() < one_wage);
    fund(&mut city.state, city.player, 500_000);
    let funding = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect::<BTreeSet<_>>();
    validate_acquire_business(
        &registry,
        &city.state,
        BusinessAcquisitionDraft {
            organization: city.player,
            business: storefront,
            funding_accounts: funding,
        },
    )
    .unwrap()
    .commit(&registry, &mut city.state)
    .unwrap();

    let open = city
        .state
        .opportunities()
        .opportunities_for_organization(city.player)
        .filter(|opportunity| opportunity.status() == OpportunityStatus::Open)
        .map(|opportunity| opportunity.id())
        .collect::<Vec<_>>();
    for opportunity in open {
        validate_dismiss_opportunity(&registry, &city.state, opportunity)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    for _ in 0..VICTORY_ESTABLISHMENT_DAYS {
        city.state
            .advance_clock(crate::core::time::SimDuration::from_minutes(
                DAY_MINUTES as u32,
            ));
        validate_business_cycle_plan(
            &city.state,
            decide_business_cycle(&registry, &city.state, storefront, 0).unwrap(),
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    assert!(established_business(&city.state, storefront, city.player));
    let earned = recent_business_net(&city.state, storefront, city.player);
    assert!(earned > 0);
    assert!(earned < i128::from(one_wage) * i128::from(VICTORY_INCOME_WINDOW_DAYS));
    let standing = progress(&registry, &city.state).unwrap();
    assert_eq!(standing.owned_operating_business_districts, 2);
    assert_eq!(standing.established_business_districts, 0);
    assert!(!standing.legitimate_self_sufficient);
    assert_eq!(
        progress(
            &registry,
            &restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap()
        )
        .unwrap(),
        standing
    );
}

#[test]
fn political_and_underboss_claims_reject_unsupported_status() {
    let (registry, state, _, _, _) = retirement_fixture();
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
}

#[test]
fn victory_progress_counts_hosted_rackets_only_while_operating_after_restore() {
    use crate::city_generation::{CityScale, generate_city};
    use crate::enterprises::EnterpriseLocation;
    use crate::enterprises::enterprise_execution::validate_suspend_enterprise;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(947)).unwrap();
    let racket = city
        .state
        .enterprises()
        .enterprises_for_organization(city.player)
        .next()
        .expect("generated player has a hosted racket");
    let racket_id = racket.id();
    let EnterpriseLocation::Business(venue) = racket.location() else {
        panic!("the opening racket should be located at a real business");
    };
    assert_eq!(
        city.state
            .world()
            .get_business(venue)
            .unwrap()
            .neighborhood(),
        city.home
    );
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .own_active_racket_districts,
        1
    );

    validate_suspend_enterprise(&city.state, racket_id)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .own_active_racket_districts,
        0
    );
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        progress(&registry, &restored)
            .unwrap()
            .own_active_racket_districts,
        0
    );
    validate_invariants(&restored);
}
