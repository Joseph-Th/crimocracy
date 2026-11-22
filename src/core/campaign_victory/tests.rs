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
fn political_business_coalition_requires_distinct_profitable_industries() {
    let mut standing = BusinessDistrictStanding {
        active: 2,
        mature: 2,
        established: 2,
        window_net: 250_000,
        ..BusinessDistrictStanding::default()
    };
    standing.established_kinds.insert(BusinessKind::Pawnshop);
    assert!(
        !standing.qualifies(120_000),
        "two pawnshops are still a single local commercial constituency"
    );
    standing.established_kinds.insert(BusinessKind::Hospitality);
    assert!(standing.qualifies(120_000));
    standing.window_net = 119_999;
    assert!(
        !standing.qualifies(120_000),
        "diversity cannot replace income"
    );
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
fn retirement_requires_a_million_for_the_leader_plus_staff_severance() {
    let (registry, mut state, owner, boss, _) = retirement_fixture();
    hire_additional_staff(&mut state, owner, boss, 6);
    let old_requirement = 10_040_000;
    let expected_requirement = 105_040_000;
    assert_eq!(RETIREMENT_NEST_EGG.cents(), 100_000_000);
    assert_eq!(
        progress(&registry, &state)
            .unwrap()
            .retirement_total
            .cents(),
        expected_requirement
    );

    fund(&mut state, owner, old_requirement);
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).err(),
        Some(VictoryError::RetirementNotReady)
    );
    fund(
        &mut state,
        owner,
        expected_requirement - old_requirement - 1,
    );
    let before = bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).err(),
        Some(VictoryError::RetirementNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );

    fund(&mut state, owner, 1);
    validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement)
        .unwrap()
        .commit(&registry, &mut state)
        .unwrap();
    assert_eq!(
        state
            .finance()
            .accounts_for(FinancialOwner::Organization(owner))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| account.balance().cents())
            .sum::<i64>(),
        0
    );
    validate_state_against_registry(&registry, &state).unwrap();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert!(restored.campaign_victory().is_some());
    assert_eq!(
        run_tick(&registry, &mut restored),
        Err(TickError::CampaignWon {
            path: VictoryPath::FundedRetirement
        })
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
    use crate::delegation::delegation_system::{
        MandateRevisionDraft, validate_assign_mandate, validate_revise_mandate,
    };
    use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
    use crate::economy::business_acquisition::{
        BusinessAcquisitionDraft, validate_acquire_business,
    };
    use crate::world::BusinessOwner;
    use crate::world::world_system::validate_set_character_autonomy;

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
    let early_districts = political_district_progress(&registry, &city.state).unwrap();
    assert_eq!(
        early_districts
            .iter()
            .filter(|row| row.active_owned > 0)
            .count(),
        early.owned_operating_business_districts
    );
    assert!(early_districts.iter().all(|row| !row.qualifies));
    assert!(early_districts.iter().all(|row| row.mature_active == 0));
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
    let mature_districts = political_district_progress(&registry, &city.state).unwrap();
    assert_eq!(established.staffed_political_districts, 0);
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady),
        "passive real estate must not create a citywide political coalition"
    );
    assert_eq!(
        mature_districts
            .iter()
            .filter(|row| row.economically_qualified)
            .count(),
        established.established_business_districts,
        "district-level financial view must agree with the victory validator"
    );
    assert!(
        mature_districts
            .iter()
            .filter(|row| row.economically_qualified)
            .all(
                |row| row.established_kinds >= POLITICAL_BUSINESS_KINDS_PER_DISTRICT
                    && row.mature_window_net > Money::ZERO
            )
    );
    assert!(mature_districts.iter().all(|row| !row.qualifies));

    // Even a broad political mandate covering every profitable district is
    // just one person. It can staff only one district, not a civic coalition.
    let districts = mature_districts
        .iter()
        .filter(|row| row.economically_qualified)
        .map(|row| row.district)
        .collect::<Vec<_>>();
    let managers = city
        .state
        .world()
        .characters_in_organization(city.player)
        .filter(|member| member.supervisor().is_some())
        .map(|member| member.id())
        .take(districts.len())
        .collect::<Vec<_>>();
    assert_eq!(managers.len(), districts.len());
    assert!(established.key_personnel >= established.required_political_personnel);
    for &manager in &managers {
        if !matches!(
            city.state
                .world()
                .get_character(manager)
                .unwrap()
                .autonomy(),
            AutonomyLevel::Delegated | AutonomyLevel::Broad
        ) {
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
    }
    let first = managers[0];
    let wide_scopes = districts
        .iter()
        .map(|district| ResponsibilityScope::Neighborhood(*district))
        .chain(std::iter::once(ResponsibilityScope::Function(
            ResponsibilityFunction::Political,
        )))
        .collect::<BTreeSet<_>>();
    if let Some(mandate) = city.state.delegation().active_for_manager(first) {
        validate_revise_mandate(
            &city.state,
            mandate.id(),
            MandateRevisionDraft {
                scopes: mandate.scopes().union(&wide_scopes).copied().collect(),
                standing_orders: mandate.standing_orders().clone(),
                budget: mandate.budget(),
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    } else {
        validate_assign_mandate(
            &city.state,
            MandateDraft {
                organization: city.player,
                manager: first,
                scopes: wide_scopes,
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .staffed_political_districts,
        1
    );
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    for (&manager, &district) in managers.iter().skip(1).zip(districts.iter().skip(1)) {
        let scope = ResponsibilityScope::Neighborhood(district);
        if let Some(mandate) = city.state.delegation().active_for_manager(manager) {
            let mut scopes = mandate.scopes().clone();
            scopes.insert(scope);
            scopes.insert(ResponsibilityScope::Function(
                ResponsibilityFunction::Political,
            ));
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
        } else {
            validate_assign_mandate(
                &city.state,
                MandateDraft {
                    organization: city.player,
                    manager,
                    scopes: BTreeSet::from([
                        scope,
                        ResponsibilityScope::Function(ResponsibilityFunction::Political),
                    ]),
                    standing_orders: BTreeMap::new(),
                    budget: None,
                },
            )
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        }
    }
    let staffed = progress(&registry, &city.state).unwrap();
    assert_eq!(staffed.staffed_political_districts, districts.len());
    assert_eq!(
        political_district_progress(&registry, &city.state)
            .unwrap()
            .iter()
            .filter(|row| row.qualifies)
            .count(),
        staffed.staffed_political_districts
    );
    assert!(staffed.legitimate_self_sufficient);
    assert!(staffed.key_personnel >= staffed.required_political_personnel);
    // A political delegate needs current independent authority, not a name
    // in a historical mandate or a dormant member's payroll record.
    validate_set_character_autonomy(&city.state, city.player, managers[1], AutonomyLevel::Tight)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .staffed_political_districts,
        districts.len() - 1
    );
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    validate_set_character_autonomy(
        &city.state,
        city.player,
        managers[1],
        AutonomyLevel::Delegated,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .staffed_political_districts,
        districts.len()
    );
    let established = staffed;
    let mature_districts = political_district_progress(&registry, &city.state).unwrap();
    let restored_before_claim =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        progress(&registry, &restored_before_claim).unwrap(),
        established
    );
    assert_eq!(
        political_district_progress(&registry, &restored_before_claim).unwrap(),
        mature_districts
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
    assert!(
        political_district_progress(&registry, &stale)
            .unwrap()
            .iter()
            .all(|row| !row.qualifies)
    );
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
    use crate::world::world_system::insert_business;
    use crate::world::world_system::validate_set_character_autonomy;
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind, Rating,
    };

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
    // Independent profitable captains, rather than one omnipresent manager,
    // must be answerable for each long-established district foothold.
    for (index, district) in [home, empty_districts[0], empty_districts[1]]
        .into_iter()
        .enumerate()
    {
        let captain = insert_character(
            &mut city.state,
            CharacterDraft {
                name: format!("Independent district captain {index}"),
                organization: Some(city.player),
                supervisor: Some(city.boss),
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Management,
                    Rating::try_new(95).unwrap(),
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
                organization: city.player,
                manager: captain,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        let venue = insert_business(
            &registry,
            &mut city.state,
            BusinessDraft {
                name: format!("Independent district front {index}"),
                kind: BusinessKind::Hospitality,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::MeetingSpace,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood: district,
                owner: BusinessOwner::Organization(city.player),
            },
        )
        .unwrap();
        let settlement = insert_account(
            &mut city.state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(city.player),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        let second_racket = validate_establish_enterprise(
            &registry,
            &city.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization: city.player,
                authority: MandateAuthority {
                    mandate,
                    manager: captain,
                    scope,
                },
                location: EnterpriseLocation::Business(venue),
                supporting_businesses: BTreeSet::new(),
                cash_account: cash,
                settlement_account: settlement,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        // Field strength must actually earn its way into territorial standing.
        crate::enterprises::enterprise_execution::validate_staff_enterprise(
            &city.state,
            second_racket,
            if index == 0 { 7 } else { 8 },
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
    assert_eq!(established.workforce_ready_districts, 3);
    assert!(established.mature_associates >= 36);
    assert_eq!(established.staffed_underboss_districts, 3);
    assert_eq!(established.required_underboss_personnel, 10);
    assert!(established.key_personnel < established.required_underboss_personnel);
    assert!(established.racket_self_sufficient);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), established);
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady),
        "territorial strength cannot replace a staffed command roster"
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    hire_additional_staff(
        &mut city.state,
        city.player,
        city.boss,
        established.required_underboss_personnel - established.key_personnel,
    );
    let ready = progress(&registry, &city.state).unwrap();
    assert_eq!(ready.key_personnel, ready.required_underboss_personnel);
    assert_eq!(
        ready.staffed_underboss_districts,
        ready.required_underboss_districts
    );
    assert!(ready.racket_self_sufficient);
    // New hires need operating history; buying a field crew after day 90
    // cannot instantly transform the same mature income into territorial force.
    let mut instant_hiring =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let home_expansion = instant_hiring
        .enterprises()
        .enterprises_for_organization(city.player)
        .find(|racket| {
            racket.kind() == EnterpriseKind::Gambling
                && racket.id() != city.starter_racket
                && matches!(racket.location(), EnterpriseLocation::Business(venue)
                    if instant_hiring.world().get_business(venue).unwrap().neighborhood() == home)
        })
        .unwrap()
        .id();
    crate::enterprises::enterprise_execution::validate_staff_enterprise(
        &instant_hiring,
        home_expansion,
        8,
    )
    .unwrap()
    .commit(&mut instant_hiring)
    .unwrap();
    let instant = progress(&registry, &instant_hiring).unwrap();
    assert_eq!(instant.key_personnel, ready.key_personnel);
    assert_eq!(
        instant.established_racket_districts,
        ready.established_racket_districts
    );
    assert_eq!(instant.workforce_ready_districts, 2);
    assert_eq!(
        validate_claim_victory(&registry, &instant_hiring, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    validate_state_against_registry(&registry, &instant_hiring).unwrap();
    // Reassigning one established district's second racket to its first
    // captain leaves the same profits, payroll, membership and territory,
    // but destroys the independent local command structure.
    let mut concentrated =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let second_venue_racket = concentrated
        .enterprises()
        .enterprises_for_organization(city.player)
        .find(|racket| match racket.location() {
            EnterpriseLocation::Business(business) => {
                concentrated
                    .world()
                    .get_business(business)
                    .unwrap()
                    .neighborhood()
                    == empty_districts[0]
            }
            EnterpriseLocation::Neighborhood(_) => false,
        })
        .unwrap()
        .id();
    let first_captain = concentrated
        .enterprises()
        .get_enterprise(second_venue_racket)
        .unwrap()
        .manager();
    let district_protection = concentrated
        .enterprises()
        .enterprises_for_organization(city.player)
        .find(|racket| racket.location() == EnterpriseLocation::Neighborhood(empty_districts[0]))
        .unwrap()
        .id();
    let captain_mandate = concentrated
        .delegation()
        .active_for_manager(first_captain)
        .unwrap()
        .id();
    crate::enterprises::enterprise_execution::validate_reassign_enterprise(
        &concentrated,
        district_protection,
        MandateAuthority {
            mandate: captain_mandate,
            manager: first_captain,
            scope: ResponsibilityScope::Neighborhood(empty_districts[0]),
        },
    )
    .unwrap()
    .commit(&mut concentrated)
    .unwrap();
    let concentrated_progress = progress(&registry, &concentrated).unwrap();
    assert_eq!(concentrated_progress.key_personnel, ready.key_personnel);
    assert_eq!(
        concentrated_progress.racket_daily_net,
        ready.racket_daily_net
    );
    assert_eq!(concentrated_progress.staffed_underboss_districts, 2);
    assert_eq!(
        validate_claim_victory(&registry, &concentrated, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    validate_invariants(&concentrated);
    // A newly hired manager can take over the same mature racket, but cannot
    // retroactively claim its predecessor's 20 profitable settlements.
    let mut unproven =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let replacement = insert_character(
        &mut unproven,
        CharacterDraft {
            name: "Unproven replacement captain".to_owned(),
            organization: Some(city.player),
            supervisor: Some(city.boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(45).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let replacement_scope = ResponsibilityScope::Neighborhood(empty_districts[0]);
    let replacement_mandate = validate_assign_mandate(
        &unproven,
        MandateDraft {
            organization: city.player,
            manager: replacement,
            scopes: BTreeSet::from([replacement_scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut unproven)
    .unwrap();
    crate::enterprises::enterprise_execution::validate_reassign_enterprise(
        &unproven,
        second_venue_racket,
        MandateAuthority {
            mandate: replacement_mandate,
            manager: replacement,
            scope: replacement_scope,
        },
    )
    .unwrap()
    .commit(&mut unproven)
    .unwrap();
    let unproven_progress = progress(&registry, &unproven).unwrap();
    assert!(unproven_progress.key_personnel >= unproven_progress.required_underboss_personnel);
    assert!(unproven_progress.racket_self_sufficient);
    assert_eq!(unproven_progress.staffed_underboss_districts, 2);
    assert_eq!(
        validate_claim_victory(&registry, &unproven, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    validate_invariants(&unproven);
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
fn city_command_posts_cannot_be_filled_twice_by_the_same_manager() {
    assert_eq!(required_underboss_personnel(2), 10);
    assert_eq!(required_underboss_personnel(3), 10);
    assert_eq!(required_underboss_personnel(6), 16);
    assert_eq!(required_underboss_personnel(12), 28);
    let first = NeighborhoodId::from_raw(1);
    let second = NeighborhoodId::from_raw(2);
    let third = NeighborhoodId::from_raw(3);
    let a = CharacterId::from_raw(1);
    let b = CharacterId::from_raw(2);
    let c = CharacterId::from_raw(3);
    let d = CharacterId::from_raw(4);
    let e = CharacterId::from_raw(5);
    let districts = BTreeSet::from([first, second]);
    let mut captains = BTreeMap::from([
        (first, BTreeSet::from([a, b])),
        (second, BTreeSet::from([a, b])),
    ]);
    assert_eq!(staffed_racket_districts(&districts, &captains), 1);
    captains.insert(second, BTreeSet::from([a, b, c]));
    assert_eq!(staffed_racket_districts(&districts, &captains), 1);
    captains.insert(second, BTreeSet::from([c, d]));
    assert_eq!(staffed_racket_districts(&districts, &captains), 2);
    let districts = BTreeSet::from([first, second, third]);
    captains.insert(third, BTreeSet::from([e]));
    assert_eq!(staffed_racket_districts(&districts, &captains), 2);
    // Overlapping candidate lists still permit matching when enough distinct
    // managers exist, regardless of which district the matcher visits first.
    captains.insert(first, BTreeSet::from([a, b, c]));
    captains.insert(second, BTreeSet::from([a, d]));
    captains.insert(third, BTreeSet::from([b, e]));
    assert_eq!(staffed_racket_districts(&districts, &captains), 2);

    // Four different managers can fully command the second and third districts.
    // A maximum individual-post matching that fills the first district greedily
    // leaves one post in each other district and incorrectly counts only one.
    captains.insert(first, BTreeSet::from([a, b]));
    captains.insert(second, BTreeSet::from([a, c]));
    captains.insert(third, BTreeSet::from([b, d]));
    assert_eq!(staffed_racket_districts(&districts, &captains), 2);
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
