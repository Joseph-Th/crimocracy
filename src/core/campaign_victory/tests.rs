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

    // Purchase actual businesses in additional city districts through ordinary acquisition.
    fund(&mut city.state, city.player, 20_000_000);
    let funding = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect::<BTreeSet<_>>();
    while progress(&registry, &city.state)
        .unwrap()
        .owned_operating_business_districts
        < progress(&registry, &city.state)
            .unwrap()
            .required_political_districts
    {
        let occupied = city
            .state
            .world()
            .businesses_owned_by_organization(city.player)
            .filter(|business| {
                city.state
                    .economy()
                    .get_business_economy(business.id())
                    .is_some_and(|books| books.status() == BusinessOperatingStatus::Active)
            })
            .map(|business| business.neighborhood())
            .collect::<BTreeSet<_>>();
        let listing = city
            .state
            .world()
            .public_business_directory()
            .find_map(|(id, _, _, district)| {
                (!occupied.contains(&district)
                    && city.state.world().get_business(id).unwrap().owner()
                        == BusinessOwner::Independent)
                    .then_some(id)
            })
            .expect("generated city should contain independent properties outside home district");
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
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert!(restored.campaign_victory().is_some());
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
