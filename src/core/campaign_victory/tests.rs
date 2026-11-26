use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{validate_invariants, validate_state_against_registry};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::{TickError, run_tick};
use crate::core::time::SimDuration;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseCycleRandomness, decide_enterprise_cycle, validate_enterprise_cycle_plan,
    validate_establish_enterprise, validate_retire_enterprise, validate_suspend_enterprise,
};
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{FinancialAccountDraft, LedgerPosting, LedgerTransactionDraft};
use crate::reports::ReportKind;
use crate::world::BusinessOwner;
use crate::world::world_system::{
    designate_player_organization, insert_character, insert_neighborhood, insert_organization,
};
use crate::world::{
    AutonomyLevel, CapabilityKind, CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    Rating,
};
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
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(88).unwrap(),
            )]),
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
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(80).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    (registry, state, owner, boss, member)
}

#[test]
fn underboss_reserve_includes_organization_operating_liquidity_and_deficits() {
    let (registry, mut state, owner, _, _) = retirement_fixture();
    let baseline = progress(&registry, &state)
        .unwrap()
        .underboss_liquid_reserve;
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let debt = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Operating liquidity and separate debt".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: operating,
                    amount: Money::from_cents(100_000),
                },
                LedgerPosting {
                    account: debt,
                    amount: Money::from_cents(-25_000),
                },
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-75_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    assert_eq!(
        progress(&registry, &state)
            .unwrap()
            .underboss_liquid_reserve,
        baseline.checked_add(Money::from_cents(75_000)).unwrap(),
        "organization-owned operating accounts fund wages; their deficits also reduce reserves"
    );
    validate_invariants(&state);
}

/// A qualifying boss actually runs a local racket for 90 profitable days before
/// collecting the clean nest egg. These are canonical enterprise settlements,
/// not constructed revenue records or a victory-validator bypass.
fn retirement_ready_fixture() -> (Registry, AppState, OrganizationId, CharacterId, CharacterId) {
    let (registry, mut state, owner, boss, member) = retirement_fixture();
    hire_additional_staff(&mut state, owner, boss, RETIREMENT_MIN_KEY_PERSONNEL - 2);
    let rating = |value| Rating::try_new(value).unwrap();
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Criminal retirement proving ground".into(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(60),
                    commercial_activity: rating(70),
                    illicit_demand: rating(60),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(30),
                },
            },
        },
    )
    .unwrap();
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: owner,
            manager: member,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let racket = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: owner,
            authority: MandateAuthority {
                mandate,
                manager: member,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    for _ in 0..RETIREMENT_MIN_PROFITABLE_RACKET_CYCLES {
        state.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(
            &registry,
            &state,
            racket,
            EnterpriseCycleRandomness::new(
                0,
                EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
            ),
        )
        .unwrap();
        validate_enterprise_cycle_plan(&state, plan)
            .unwrap()
            .commit(&mut state)
            .unwrap();
    }
    let standing = progress(&registry, &state).unwrap();
    assert_eq!(
        standing.retirement_profitable_racket_cycles,
        RETIREMENT_MIN_PROFITABLE_RACKET_CYCLES
    );
    assert!(standing.retirement_racket_net >= RETIREMENT_MIN_RACKET_NET);
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
fn passive_millionaire_with_a_named_crew_cannot_retire_without_a_criminal_career() {
    let (registry, mut state, owner, boss, _) = retirement_fixture();
    hire_additional_staff(&mut state, owner, boss, RETIREMENT_MIN_KEY_PERSONNEL - 2);
    let required = progress(&registry, &state).unwrap().retirement_total;
    fund(&mut state, owner, required.cents() + 100_000);
    let standing = progress(&registry, &state).unwrap();
    assert_eq!(standing.key_personnel, RETIREMENT_MIN_KEY_PERSONNEL);
    assert_eq!(standing.retirement_profitable_racket_cycles, 0);
    assert_eq!(standing.retirement_racket_net, Money::ZERO);
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
fn retirement_recognizes_retired_racket_results_but_not_ancient_illicit_earnings() {
    let (registry, mut state, owner, _, _) = retirement_ready_fixture();
    let racket = state
        .enterprises()
        .enterprises_for_organization(owner)
        .next()
        .unwrap()
        .id();
    validate_suspend_enterprise(&state, racket)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    validate_retire_enterprise(&state, racket)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let required = progress(&registry, &state).unwrap().retirement_total;
    fund(&mut state, owner, required.cents());
    validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement)
        .expect("a recently retired criminal operation should count");

    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(RETIREMENT_CRIME_WINDOW_DAYS * DAY_MINUTES + 1).unwrap(),
    ));
    let standing = progress(&registry, &state).unwrap();
    assert_eq!(standing.retirement_profitable_racket_cycles, 0);
    assert_eq!(standing.retirement_racket_net, Money::ZERO);
    assert_eq!(
        validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).err(),
        Some(VictoryError::RetirementNotReady)
    );
}

#[test]
fn retirement_requires_a_million_for_the_leader_plus_staff_severance() {
    let (registry, mut state, owner, boss, _) = retirement_ready_fixture();
    hire_additional_staff(&mut state, owner, boss, 4);
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
    let (registry, mut state, owner, boss, member) = retirement_ready_fixture();
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .unwrap();
    let required = RETIREMENT_NEST_EGG
        .checked_add(severance.checked_mul(3).unwrap())
        .unwrap();
    fund(&mut state, owner, required.cents() + 200_000);
    // Open unrelated accounts before planning, then commit an independent receipt.
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let source = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let plan = validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).unwrap();
    let unrelated = validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Unrelated cash receipt".into(),
            postings: vec![
                LedgerPosting {
                    account: cash,
                    amount: Money::from_cents(100),
                },
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-100),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let result = plan.commit(&registry, &mut state).unwrap();
    assert_ne!(result.settlement, Some(unrelated));
    assert!(matches!(
        state.history().campaign_victory(owner).unwrap().kind(),
        crate::history::HistoryEventKind::CampaignVictory { settlement, .. }
            if settlement == result.settlement
    ));
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
fn retirement_rechecks_the_complete_payee_roster_before_settlement() {
    let (registry, mut state, owner, boss, _) = retirement_ready_fixture();
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .unwrap();
    let original_required = progress(&registry, &state).unwrap().retirement_total;
    // Enough for another subordinate even before planning; eligibility and all
    // existing payment accounts stay viable after the hire.
    fund(
        &mut state,
        owner,
        original_required.cents() + severance.cents(),
    );
    let token = validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement).unwrap();
    let recruit = insert_character(
        &mut state,
        CharacterDraft {
            name: "Last-minute subordinate".into(),
            organization: Some(owner),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let before = bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        token.commit(&registry, &mut state).err(),
        Some(VictoryError::Stale),
        "the held settlement omits the new subordinate despite sufficient reserves"
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
    // Leadership can also change without changing headcount, account versions
    // or the aggregate settlement total. The nest egg must follow the new boss.
    let mut succession = state.clone();
    let old_leadership =
        validate_claim_victory(&registry, &succession, VictoryPath::FundedRetirement).unwrap();
    use crate::world::world_system::validate_set_character_supervisor;
    validate_set_character_supervisor(&succession, recruit, None)
        .unwrap()
        .commit(&mut succession)
        .unwrap();
    validate_set_character_supervisor(&succession, boss, Some(recruit))
        .unwrap()
        .commit(&mut succession)
        .unwrap();
    let before_succession =
        bincode::serialize(&build_save(&registry, &succession).unwrap()).unwrap();
    assert_eq!(
        old_leadership.commit(&registry, &mut succession).err(),
        Some(VictoryError::Stale)
    );
    assert_eq!(
        before_succession,
        bincode::serialize(&build_save(&registry, &succession).unwrap()).unwrap()
    );
    validate_claim_victory(&registry, &succession, VictoryPath::FundedRetirement)
        .unwrap()
        .commit(&registry, &mut succession)
        .unwrap();
    for (person, expected) in [(recruit, RETIREMENT_NEST_EGG), (boss, severance)] {
        let paid = succession
            .finance()
            .accounts_for(FinancialOwner::Character(person))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| account.balance().cents())
            .sum::<i64>();
        assert_eq!(paid, expected.cents());
    }
    restore_save(&registry, build_save(&registry, &succession).unwrap()).unwrap();
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    for state in [&mut state, &mut restored] {
        validate_claim_victory(&registry, state, VictoryPath::FundedRetirement)
            .unwrap()
            .commit(&registry, state)
            .unwrap();
        let paid = state
            .finance()
            .accounts_for(FinancialOwner::Character(recruit))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .map(|account| account.balance().cents())
            .sum::<i64>();
        assert_eq!(paid, severance.cents());
        validate_invariants(state);
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn retirement_save_rejects_incomplete_or_underpaid_settlement_reference() {
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::LoadError;
    use crate::history::HistoryEventKind;

    let (registry, mut state, owner, boss, member) = retirement_ready_fixture();
    let (payees, required) = retirement_members(&registry, &state, owner).unwrap();
    fund(&mut state, owner, required.cents() * 4);
    let funding = state
        .finance()
        .accounts_for(FinancialOwner::Organization(owner))
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let destinations: Vec<_> = payees
        .iter()
        .map(|&(person, amount)| {
            let account = insert_account(
                &mut state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Character(person),
                    kind: AccountKind::AccountedFunds,
                },
            )
            .unwrap();
            (person, account, amount)
        })
        .collect();
    let mut alternatives = Vec::new();
    // Each alternate transfer has valid balanced ledger provenance and the
    // exact settlement memo and timestamp. Only the payee contract is false.
    // The third pays all subordinates uniformly but not at the authored wage.
    for mode in 0..3 {
        let mut total = 0;
        let mut postings = Vec::new();
        for &(person, account, amount) in &destinations {
            if mode == 0 && person == member {
                continue;
            }
            let cents = amount.cents()
                - i64::from((mode == 1 && person == member) || (mode == 2 && person != boss));
            total += cents;
            postings.push(LedgerPosting {
                account,
                amount: Money::from_cents(cents),
            });
        }
        postings.push(LedgerPosting {
            account: funding,
            amount: Money::from_cents(-total),
        });
        let payment = validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Final campaign retirement settlement: leader nest egg and crew severance"
                    .into(),
                postings,
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        alternatives.push(payment);
    }
    let outcome = validate_claim_victory(&registry, &state, VictoryPath::FundedRetirement)
        .unwrap()
        .commit(&registry, &mut state)
        .unwrap();
    let event = state.history().get_event(outcome.history).unwrap();
    let envelope = build_save(&registry, &state).unwrap();
    let bytes = bincode::serialize(&envelope).unwrap();
    let original = bincode::serialize(event).unwrap();
    let positions: Vec<_> = bytes
        .windows(original.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original.as_slice()).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    let start = positions[0];
    for alternative in alternatives {
        let replacement = bincode::serialize(&(
            event.id(),
            event.occurred_at(),
            HistoryEventKind::CampaignVictory {
                organization: owner,
                path: VictoryPath::FundedRetirement,
                settlement: Some(alternative),
            },
            event.summary(),
            event.entities(),
        ))
        .unwrap();
        assert_eq!(replacement.len(), original.len());
        let mut corrupt = bytes.clone();
        corrupt[start..start + original.len()].copy_from_slice(&replacement);
        let malformed = bincode::deserialize(&corrupt).unwrap();
        assert_eq!(
            restore_save(&registry, malformed).err(),
            Some(LoadError::InvalidState(
                StateValidationError::InvalidCampaignVictory {
                    event: outcome.history
                }
            ))
        );
    }
    assert_eq!(
        state.world().get_character(boss).unwrap().supervisor(),
        None
    );
    restore_save(&registry, envelope).unwrap();
}

#[test]
fn victory_token_rejects_clock_drift_without_paying_anyone() {
    let (registry, mut state, owner, _, _) = retirement_ready_fixture();
    let required = RETIREMENT_NEST_EGG
        .checked_add(
            registry
                .upkeep()
                .per_member_daily()
                .checked_mul(RETIREMENT_SEVERANCE_DAYS * 3)
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
    // This criminal outfit opens with a racket. Stopping it is mandatory
    // before it can claim a credible transition into legitimate politics.
    assert!(
        !progress(&registry, &city.state)
            .unwrap()
            .legitimate_transition_ready
    );
    let rackets = city
        .state
        .enterprises()
        .enterprises_for_organization(city.player)
        .filter(|racket| racket.status() == crate::enterprises::EnterpriseStatus::Active)
        .map(|racket| racket.id())
        .collect::<Vec<_>>();
    for racket in rackets {
        crate::enterprises::enterprise_execution::validate_suspend_enterprise(&city.state, racket)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    assert!(
        progress(&registry, &city.state)
            .unwrap()
            .legitimate_transition_ready
    );
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
    fund(&mut city.state, city.player, 100_000_000);
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
    // The original opening owns only two home-ward storefronts. A credible
    // political constituency now needs a third distinct commercial employer.
    let home_listing = city
        .state
        .world()
        .public_business_directory()
        .map(|(id, _, _, district)| (id, district))
        .filter(|(id, district)| {
            *district == city.home
                && city.state.world().get_business(*id).unwrap().owner()
                    == BusinessOwner::Independent
        })
        .max_by_key(|(id, _)| {
            crate::economy::business_economy_system::forecast_normal_business_net(
                &registry,
                &city.state,
                *id,
            )
            .unwrap()
            .cents()
        })
        .expect("generated home district has an independent listing")
        .0;
    validate_acquire_business(
        &registry,
        &city.state,
        BusinessAcquisitionDraft {
            organization: city.player,
            business: home_listing,
            funding_accounts: funding.clone(),
        },
    )
    .unwrap()
    .commit(&registry, &mut city.state)
    .unwrap();
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
    for district in &early_districts {
        let titles = political_business_progress(&city.state, district.district).unwrap();
        assert_eq!(
            titles.len(),
            district.active_owned + district.suspended_owned
        );
        for title in titles {
            assert_eq!(title.owned_days, 0);
            assert_eq!(
                title.days_until_mature,
                POLITICAL_BUSINESS_ESTABLISHMENT_DAYS
            );
            assert!(!title.established);
        }
    }
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
    for day in 1..=POLITICAL_BUSINESS_ESTABLISHMENT_DAYS {
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
        if day == POLITICAL_BUSINESS_ESTABLISHMENT_DAYS - 1 {
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
        "qualifying productive business districts: {:?}; districts: {:?}",
        established,
        political_district_progress(&registry, &city.state).unwrap()
    );
    assert!(established.legitimate_self_sufficient);
    assert!(established.legitimate_daily_net >= established.daily_payroll);
    let mature_districts = political_district_progress(&registry, &city.state).unwrap();
    for district in &mature_districts {
        let titles = political_business_progress(&city.state, district.district).unwrap();
        assert_eq!(
            titles.iter().filter(|title| title.established).count(),
            district.established_active
        );
        assert!(titles.iter().all(|title| title.days_until_mature == 0));
        for title in &titles {
            let business = city.state.world().get_business(title.business).unwrap();
            let expected = city
                .state
                .economy()
                .cycles_for(title.business)
                .filter(|cycle| {
                    cycle.occurred_at().as_minutes() > recent_income_start(city.state.now())
                        && cycle.occurred_at() <= city.state.now()
                        && cycle.business_version() == business.version()
                        && cycle.owner() == BusinessOwner::Organization(city.player)
                        && cycle.net_cash() > Money::ZERO
                })
                .count();
            assert_eq!(
                title.profitable_cycles, expected,
                "title progress must report the complete window, not cap it at the qualification threshold"
            );
        }
        assert!(
            titles
                .iter()
                .filter(|title| title.established)
                .all(|title| title.profitable_cycles >= VICTORY_PROFITABLE_CYCLES
                    && title.last_owned_settlement.is_some())
        );
    }
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
    hire_additional_staff(
        &mut city.state,
        city.player,
        city.boss,
        established
            .required_political_personnel
            .saturating_sub(established.key_personnel),
    );
    let managers = city
        .state
        .world()
        .characters_in_organization(city.player)
        .filter(|member| member.supervisor().is_some())
        .map(|member| member.id())
        .take(districts.len())
        .collect::<Vec<_>>();
    assert_eq!(managers.len(), districts.len());
    let finance_managers = city
        .state
        .world()
        .characters_in_organization(city.player)
        .filter(|member| member.supervisor().is_some())
        .map(|member| member.id())
        .skip(managers.len())
        .take(districts.len())
        .collect::<Vec<_>>();
    assert_eq!(finance_managers.len(), districts.len());
    assert!(established.key_personnel < established.required_political_personnel);
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
        0
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
    for (&manager, &district) in finance_managers.iter().zip(districts.iter()) {
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
        let desired = BTreeSet::from([
            ResponsibilityScope::Neighborhood(district),
            ResponsibilityScope::Function(ResponsibilityFunction::Finance),
        ]);
        if let Some(mandate) = city.state.delegation().active_for_manager(manager) {
            validate_revise_mandate(
                &city.state,
                mandate.id(),
                MandateRevisionDraft {
                    scopes: mandate.scopes().union(&desired).copied().collect(),
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
                    scopes: desired,
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

    // Joint staffing must find the valid team when the earliest organizer is
    // also the only finance director for the first ward. All actual mandates
    // are revised through the production owner, rather than editing the graph.
    let mut dual_role = city.state.clone();
    for (manager, remove_finance) in [(managers[0], false), (finance_managers[0], true)] {
        let mandate = dual_role.delegation().active_for_manager(manager).unwrap();
        let mut scopes = mandate.scopes().clone();
        if remove_finance {
            scopes.remove(&ResponsibilityScope::Function(
                ResponsibilityFunction::Finance,
            ));
            scopes.insert(ResponsibilityScope::Function(
                ResponsibilityFunction::Political,
            ));
        } else {
            scopes.insert(ResponsibilityScope::Function(
                ResponsibilityFunction::Finance,
            ));
        }
        validate_revise_mandate(
            &dual_role,
            mandate.id(),
            MandateRevisionDraft {
                scopes,
                standing_orders: mandate.standing_orders().clone(),
                budget: mandate.budget(),
            },
        )
        .unwrap()
        .commit(&mut dual_role)
        .unwrap();
    }
    let dual_rows = political_district_progress(&registry, &dual_role).unwrap();
    let first_ward = dual_rows
        .iter()
        .find(|row| row.district == districts[0])
        .unwrap();
    assert_eq!(first_ward.steward, Some(finance_managers[0]));
    assert_eq!(first_ward.financial_steward, Some(managers[0]));
    assert_eq!(first_ward.available_financial_stewards, 1);
    assert_eq!(
        progress(&registry, &dual_role)
            .unwrap()
            .staffed_political_districts,
        districts.len()
    );
    let restored_dual =
        restore_save(&registry, build_save(&registry, &dual_role).unwrap()).unwrap();
    assert_eq!(
        political_district_progress(&registry, &restored_dual).unwrap(),
        dual_rows
    );
    validate_state_against_registry(&registry, &dual_role).unwrap();

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
    // Commercial districts and delegates alone cannot elect a mayor. The
    // candidate must buy real electoral support, retain paid specialists,
    // defeat rival pressure, and hold the majority for a sustained period.
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    use crate::politics::SpecialistKind;
    use crate::politics::political_system::{
        apply_due_mayoral_elections, apply_due_political_campaigns, validate_hire_specialist,
        validate_invest_influence, validate_nominate_mayor,
    };
    fund(&mut city.state, city.player, 120_000_000);
    for (&district, &manager) in districts.iter().zip(managers.iter()) {
        validate_hire_specialist(&city.state, district, manager, SpecialistKind::Strategist)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        validate_hire_specialist(&city.state, district, manager, SpecialistKind::Organizer)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        validate_invest_influence(
            &city.state,
            district,
            810 * crate::politics::INFLUENCE_COST_PER_POINT,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    validate_nominate_mayor(&city.state, city.boss)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    // Even a funded nomination does not elect anyone before the next actual ballot.
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower).err(),
        Some(VictoryError::PoliticalNotReady)
    );
    let election_day =
        crate::politics::political_system::next_municipal_election_day(city.state.now())
            .expect("fixture has another municipal election");
    for day in POLITICAL_BUSINESS_ESTABLISHMENT_DAYS..election_day {
        city.state
            .advance_clock(crate::core::time::SimDuration::from_minutes(
                DAY_MINUTES as u32,
            ));
        for business in city
            .state
            .world()
            .businesses_owned_by_organization(city.player)
            .map(|business| business.id())
            .collect::<Vec<_>>()
        {
            if let Ok(plan) = decide_business_cycle(&registry, &city.state, business, 0) {
                validate_business_cycle_plan(&city.state, plan)
                    .unwrap()
                    .commit(&mut city.state)
                    .unwrap();
            }
        }
        if day + 1 == election_day - 40 {
            // Both wards have mature storefronts and sustained electoral support.
            // One shared finance director can serve either, but not both. Let the
            // earlier ward's public support erode without breaking its majority.
            let mut contested = city.state.clone();
            let weak = districts[0];
            let strong = districts[1];
            for kind in [SpecialistKind::Organizer, SpecialistKind::Strategist] {
                crate::politics::political_system::dismiss_specialist(&mut contested, weak, kind)
                    .unwrap();
            }
            let mandate = contested
                .delegation()
                .active_for_manager(finance_managers[0])
                .unwrap();
            let mut scopes = mandate.scopes().clone();
            scopes.insert(ResponsibilityScope::Neighborhood(strong));
            validate_revise_mandate(
                &contested,
                mandate.id(),
                MandateRevisionDraft {
                    scopes,
                    standing_orders: mandate.standing_orders().clone(),
                    budget: mandate.budget(),
                },
            )
            .unwrap()
            .commit(&mut contested)
            .unwrap();
            validate_set_character_autonomy(
                &contested,
                city.player,
                finance_managers[1],
                AutonomyLevel::Guided,
            )
            .unwrap()
            .commit(&mut contested)
            .unwrap();
            for offset in 0..=40 {
                if offset > 0 {
                    contested.advance_clock(crate::core::time::DAY_DURATION);
                    let businesses = contested
                        .world()
                        .businesses_owned_by_organization(city.player)
                        .map(|business| business.id())
                        .collect::<Vec<_>>();
                    for business in businesses {
                        if let Ok(plan) = decide_business_cycle(&registry, &contested, business, 0)
                        {
                            validate_business_cycle_plan(&contested, plan)
                                .unwrap()
                                .commit(&mut contested)
                                .unwrap();
                        }
                    }
                }
                let influence = contested
                    .politics()
                    .organization(city.player)
                    .unwrap()
                    .district(weak)
                    .unwrap()
                    .influence();
                if influence < 825 {
                    validate_invest_influence(
                        &contested,
                        weak,
                        i64::from(825 - influence) * crate::politics::INFLUENCE_COST_PER_POINT,
                    )
                    .unwrap()
                    .commit(&mut contested)
                    .unwrap();
                }
                apply_due_political_campaigns(&registry, &mut contested);
            }
            crate::reputation::reputation_system::apply_reputation_delta(
                &registry,
                &mut contested,
                city.player,
                crate::reputation::AudienceKind::Residents,
                100,
            )
            .unwrap();
            for district in [weak, strong] {
                assert!(
                    crate::politics::political_system::held_electoral_majority(
                        &contested,
                        city.player,
                        district,
                    ),
                    "the fixture must retain the sustained majority in both wards"
                );
            }
            apply_due_mayoral_elections(&registry, &mut contested);
            let ballot = contested
                .politics()
                .organization(city.player)
                .unwrap()
                .latest_election()
                .unwrap();
            let weak_ballot = ballot
                .ballots
                .iter()
                .find(|ward| ward.district == weak)
                .unwrap();
            let strong_ballot = ballot
                .ballots
                .iter()
                .find(|ward| ward.district == strong)
                .unwrap();
            assert!(
                weak_ballot.candidate_votes < weak_ballot.incumbent_votes,
                "{weak_ballot:?}"
            );
            assert!(
                strong_ballot.candidate_votes
                    > strong_ballot.incumbent_votes.max(strong_ballot.rival_votes),
                "{strong_ballot:?}"
            );
            assert!(
                strong_ballot.ward_won,
                "a losing ward cannot consume the only director available to a winning ward: {ballot:?}"
            );
            validate_state_against_registry(&registry, &contested).unwrap();
            let restored =
                restore_save(&registry, build_save(&registry, &contested).unwrap()).unwrap();
            assert_eq!(
                bincode::serialize(&contested).unwrap(),
                bincode::serialize(&restored).unwrap()
            );
        }
        if day + 1 == election_day {
            // Run the same fully organized ballot with a depleted clean treasury.
            // The final day's specialists still get paid and local support remains
            // intact, but the citywide reserve requirement is independently unmet.
            let mut depleted = city.state.clone();
            let reserve = progress(&registry, &depleted)
                .unwrap()
                .accounted_net
                .cents();
            let mut remaining = reserve - (MAYORAL_LIQUID_RESERVE_REQUIRED - 1);
            assert!(remaining > 0);
            let mut postings = Vec::new();
            for account in depleted
                .finance()
                .accounts_for(FinancialOwner::Organization(city.player))
            {
                if account.kind() == AccountKind::AccountedFunds {
                    let debit = remaining.min(account.available_payment_balance().cents());
                    if debit > 0 {
                        postings.push(LedgerPosting {
                            account: account.id(),
                            amount: Money::from_cents(-debit),
                        });
                        remaining -= debit;
                    }
                }
            }
            assert_eq!(remaining, 0);
            let spent = -postings
                .iter()
                .map(|posting| posting.amount.cents())
                .sum::<i64>();
            let destination = insert_account(
                &mut depleted,
                FinancialAccountDraft {
                    owner: FinancialOwner::Organization(city.player),
                    kind: AccountKind::Settlement,
                },
            )
            .unwrap();
            postings.push(LedgerPosting {
                account: destination,
                amount: Money::from_cents(spent),
            });
            validate_record_transaction(
                &depleted,
                LedgerTransactionDraft {
                    occurred_at: depleted.now(),
                    memo: "Final pre-election capital expenditure".into(),
                    postings,
                    authorization: None,
                },
            )
            .unwrap()
            .commit(&mut depleted)
            .unwrap();
            apply_due_political_campaigns(&registry, &mut depleted);
            apply_due_mayoral_elections(&registry, &mut depleted);
            let result = depleted
                .politics()
                .organization(city.player)
                .unwrap()
                .latest_election()
                .unwrap();
            assert!(result.wards_won >= result.wards_needed);
            assert!(!result.eligible);
            assert!(!result.elected);
            let report = depleted
                .reports()
                .reports_for_after(city.player, None)
                .find(|report| report.title().starts_with("Municipal election"))
                .unwrap();
            let summary = &report.entries()[0].summary;
            assert!(
                summary.contains("remaining clean reserve"),
                "The report must explain the failed citywide qualification despite enough winning wards: {summary}"
            );
            let restored =
                restore_save(&registry, build_save(&registry, &depleted).unwrap()).unwrap();
            assert_eq!(
                bincode::serialize(&restored).unwrap(),
                bincode::serialize(&depleted).unwrap()
            );
        }
        apply_due_political_campaigns(&registry, &mut city.state);
        apply_due_mayoral_elections(&registry, &mut city.state);
        if day % 7 == 0 {
            for (&district, &manager) in districts.iter().zip(managers.iter()) {
                if let Ok(order) = crate::politics::political_system::validate_civic_initiative(
                    &city.state,
                    district,
                    manager,
                    crate::politics::CivicInitiative::PublicMeetings,
                ) {
                    order.commit(&mut city.state).unwrap();
                }
            }
        }
    }
    let elected = progress(&registry, &city.state).unwrap();
    assert_eq!(
        elected.mayoral_majority_districts,
        elected.required_political_districts
    );
    assert!(elected.campaign_spent_cents >= MAYORAL_CAMPAIGN_SPEND_REQUIRED);
    assert!(elected.accounted_net.cents() >= MAYORAL_LIQUID_RESERVE_REQUIRED);
    assert_eq!(elected.latest_election_day, Some(election_day));
    assert!(
        elected.latest_election_won,
        "actual municipal ballot failed: {elected:?}"
    );
    // A winning ballot is durable even when the current coalition changes
    // before leadership certifies the result. A held report must not confuse
    // current standings or a former political manager with the elected record.
    let ballot = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .latest_election()
        .unwrap()
        .clone();
    let mut changed = city.state.clone();
    let held = validate_claim_victory(&registry, &changed, VictoryPath::PoliticalPower).unwrap();
    for &manager in &managers {
        validate_set_character_autonomy(&changed, city.player, manager, AutonomyLevel::Guided)
            .unwrap()
            .commit(&mut changed)
            .unwrap();
    }
    assert!(
        progress(&registry, &changed)
            .unwrap()
            .mayoral_majority_districts
            < ballot.wards_won
    );
    let mut fresh = restore_save(&registry, build_save(&registry, &changed).unwrap()).unwrap();
    let certified = validate_claim_victory(&registry, &fresh, VictoryPath::PoliticalPower)
        .unwrap()
        .commit(&registry, &mut fresh)
        .unwrap();
    let report = fresh.reports().get_report(certified.report).unwrap();
    assert!(
        report.entries()[0].summary.contains(&format!(
            "{} independently organized districts",
            ballot.wards_won
        )),
        "the certification must describe the recorded ballot, not the current coalition: {}",
        report.entries()[0].summary
    );
    for ward in ballot.ballots.iter().filter(|ward| ward.ward_won) {
        let name = fresh
            .world()
            .get_neighborhood(ward.district)
            .unwrap()
            .name();
        assert!(
            report
                .entries()
                .iter()
                .any(|entry| entry.summary.contains(name)
                    && entry
                        .entities
                        .contains(&EntityRef::Neighborhood(ward.district))),
            "a winning ward remains part of the ballot even after it loses its current delegate"
        );
    }
    let held_result = held.commit(&registry, &mut changed).unwrap();
    assert_eq!(
        bincode::serialize(fresh.reports().get_report(certified.report).unwrap()).unwrap(),
        bincode::serialize(changed.reports().get_report(held_result.report).unwrap()).unwrap(),
        "same-minute changes must be reflected at certification in both held and fresh orders"
    );
    validate_invariants(&changed);
    restore_save(&registry, build_save(&registry, &changed).unwrap()).unwrap();

    let outcome = validate_claim_victory(&registry, &city.state, VictoryPath::PoliticalPower)
        .unwrap()
        .commit(&registry, &mut city.state)
        .unwrap();
    assert_eq!(outcome.path, VictoryPath::PoliticalPower);
    assert!(outcome.settlement.is_none());
    let victory_report = city.state.reports().get_report(outcome.report).unwrap();
    assert_eq!(victory_report.kind(), ReportKind::CampaignVictory);
    assert!(victory_report.entries().len() >= districts.len() + 2);
    let summary = victory_report
        .entries()
        .iter()
        .map(|entry| entry.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(summary.contains("was elected mayor"), "{summary}");
    assert!(summary.contains("Political sponsor:"), "{summary}");
    for district in &districts {
        let district_name = city
            .state
            .world()
            .get_neighborhood(*district)
            .unwrap()
            .name();
        assert!(summary.contains(district_name), "{summary}");
    }
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
    use crate::contacts::contact_system::{
        InstitutionalContactDraft, validate_establish_contact, validate_terminate_contact,
    };
    use crate::enterprises::enterprise_execution::validate_staff_enterprise;
    use crate::world::world_system::validate_set_character_supervisor;

    let lieutenant = city
        .state
        .enterprises()
        .get_enterprise(city.starter_racket)
        .unwrap()
        .manager();
    validate_staff_enterprise(&city.state, city.starter_racket, 6)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    // Actual established civic relationships, with more than one current handler.
    let mut contacts = Vec::new();
    for (kind, handler) in [
        (OrganizationKind::LegalServices, lieutenant),
        (OrganizationKind::Political, city.boss),
        (OrganizationKind::Labor, city.boss),
    ] {
        let institution = city
            .state
            .world()
            .organizations()
            .find(|org| org.kind() == kind)
            .unwrap()
            .id();
        let establishment = city
            .state
            .world()
            .characters_in_organization(institution)
            .find_map(|official| {
                validate_establish_contact(
                    &city.state,
                    InstitutionalContactDraft {
                        sponsor: city.player,
                        handler,
                        contact: official.id(),
                    },
                )
                .ok()
            })
            .expect("generated crew has a real connection to this institution");
        contacts.push(establishment.commit(&mut city.state).unwrap());
    }
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .underboss_contact_kinds,
        0
    );
    let senior = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Senior district coordinator".into(),
            organization: Some(city.player),
            supervisor: Some(city.boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(90).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
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
                supervisor: Some(senior),
                autonomy: AutonomyLevel::Delegated,
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
                organization: city.player,
                manager: captain,
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
        for slot in 0..2 {
            let venue = insert_business(
                &registry,
                &mut city.state,
                BusinessDraft {
                    name: format!("Independent district front {index}-{slot}"),
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
            let (kind, workers) = if index == 0 {
                if slot == 0 {
                    (EnterpriseKind::Gambling, 12)
                } else {
                    (EnterpriseKind::Protection, 12)
                }
            } else {
                (EnterpriseKind::Gambling, 13)
            };
            let racket = validate_establish_enterprise(
                &registry,
                &city.state,
                EnterpriseDraft {
                    kind,
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
            validate_staff_enterprise(&city.state, racket, workers)
                .unwrap()
                .commit(&mut city.state)
                .unwrap();
        }
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
    assert!(established.mature_associates >= 90);
    assert_eq!(established.defended_underboss_districts, 0);
    assert_eq!(established.staffed_underboss_districts, 0);
    assert_eq!(established.required_underboss_personnel, 24);
    assert_eq!(established.underboss_contact_kinds, 3);
    assert_eq!(established.underboss_contact_handlers, 2);
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
    // The same successful operations cannot claim command without named
    // defenders. Add the actual capabilities consumed by local_defensive_force.
    for index in 0..9 {
        insert_character(
            &mut city.state,
            CharacterDraft {
                name: format!("District defensive specialist {index}"),
                organization: Some(city.player),
                supervisor: Some(city.boss),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Violence,
                    Rating::try_new(75).unwrap(),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
    let staffed = progress(&registry, &city.state).unwrap();
    assert_eq!(staffed.defended_underboss_districts, 3);
    assert_eq!(staffed.staffed_underboss_districts, 3);
    assert_eq!(
        staffed.senior_underboss_commands, 0,
        "senior qualification needs a fully backed coalition"
    );
    hire_additional_staff(
        &mut city.state,
        city.player,
        city.boss,
        staffed.required_underboss_personnel - staffed.key_personnel,
    );
    let ready = progress(&registry, &city.state).unwrap();
    assert_eq!(ready.key_personnel, ready.required_underboss_personnel);
    assert_eq!(
        ready.staffed_underboss_districts,
        ready.required_underboss_districts
    );
    assert!(ready.racket_self_sufficient);
    // New hires need operating history, but expanding an established field crew
    // must not erase the strength its existing workers have already demonstrated.
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
        13,
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
    assert_eq!(instant.mature_associates, ready.mature_associates);
    assert_eq!(
        instant.workforce_ready_districts,
        ready.workforce_ready_districts
    );
    validate_state_against_registry(&registry, &instant_hiring).unwrap();
    let mut instant_hiring =
        restore_save(&registry, build_save(&registry, &instant_hiring).unwrap()).unwrap();
    assert_eq!(progress(&registry, &instant_hiring).unwrap(), instant);
    for earned in 1..=VICTORY_PROFITABLE_CYCLES {
        instant_hiring.advance_clock(crate::core::time::DAY_DURATION);
        let plan = decide_enterprise_cycle(
            &registry,
            &instant_hiring,
            home_expansion,
            EnterpriseCycleRandomness::new(
                0,
                EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
            ),
        )
        .unwrap();
        let cycle = validate_enterprise_cycle_plan(&instant_hiring, plan)
            .unwrap()
            .commit(&mut instant_hiring)
            .unwrap();
        assert!(
            instant_hiring
                .enterprises()
                .get_cycle(cycle)
                .unwrap()
                .net_cash()
                > Money::ZERO
        );
        let racket = instant_hiring
            .enterprises()
            .get_enterprise(home_expansion)
            .unwrap();
        assert_eq!(
            racket_standing(&instant_hiring, racket).proven_associates,
            if earned < VICTORY_PROFITABLE_CYCLES {
                12
            } else {
                13
            },
            "the additional worker earns credit only after twenty profitable cycles"
        );
    }
    validate_staff_enterprise(&instant_hiring, home_expansion, 11)
        .unwrap()
        .commit(&mut instant_hiring)
        .unwrap();
    let racket = instant_hiring
        .enterprises()
        .get_enterprise(home_expansion)
        .unwrap();
    assert_eq!(
        racket_standing(&instant_hiring, racket).proven_associates,
        11
    );
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
    let cash = progress(&registry, &city.state).unwrap();
    assert_eq!(
        cash.required_underboss_liquid_reserve.cents(),
        cash.daily_payroll.cents() * UNDERBOSS_LIQUID_RUNWAY_DAYS as i64
            + cash.underboss_operating_buffer.cents(),
    );
    assert!(cash.underboss_operating_buffer > Money::ZERO);
    // Capitalize the scaled command through the ledger, then isolate the
    // additional authority/relationship/reserve gates on the same viable state.
    use crate::politics::political_system::{validate_civic_initiative, validate_invest_influence};
    use crate::politics::{CivicInitiative, INFLUENCE_COST_PER_POINT};
    let influence_cost = i64::from(UNDERBOSS_LOCAL_MACHINE_INFLUENCE) * INFLUENCE_COST_PER_POINT;
    let civic_cost = CivicInitiative::LaborCoalition.cost_cents();
    let shortfall = (cash.required_underboss_liquid_reserve.cents()
        - cash.underboss_liquid_reserve.cents())
    .max(0);
    fund(
        &mut city.state,
        city.player,
        shortfall + 3 * (influence_cost + civic_cost),
    );
    let without_leverage = progress(&registry, &city.state).unwrap();
    assert_eq!(without_leverage.staffed_underboss_districts, 3);
    assert_eq!(without_leverage.influential_underboss_districts, 0);
    assert!(
        without_leverage.underboss_liquid_reserve
            >= without_leverage.required_underboss_liquid_reserve
    );
    assert_eq!(
        validate_claim_victory(&registry, &city.state, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    for district in [home, empty_districts[0], empty_districts[1]] {
        validate_invest_influence(&city.state, district, influence_cost)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    assert_eq!(
        progress(&registry, &city.state)
            .unwrap()
            .influential_underboss_districts,
        0,
        "machine influence alone does not constitute institutional leverage"
    );
    for district in [home, empty_districts[0], empty_districts[1]] {
        let manager = city
            .state
            .delegation()
            .active_for_scope(ResponsibilityScope::Neighborhood(district))
            .find(|mandate| {
                mandate.organization() == city.player
                    && mandate.scopes().contains(&ResponsibilityScope::Function(
                        ResponsibilityFunction::Political,
                    ))
            })
            .unwrap()
            .manager();
        validate_civic_initiative(
            &city.state,
            district,
            manager,
            CivicInitiative::LaborCoalition,
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    }
    let cash = progress(&registry, &city.state).unwrap();
    assert_eq!(cash.influential_underboss_districts, 3);
    assert!(cash.senior_underboss_commands > 0);
    assert!(cash.underboss_ready);

    // A proven crew still operates under Guided autonomy, but its captain no
    // longer commands independently. Do not let a same-minute change in
    // discretion bypass the hierarchy requirement of the underboss ending.
    let mut micromanaged = city.state.clone();
    let captain = managers[0];
    let held =
        validate_claim_victory(&registry, &micromanaged, VictoryPath::CityUnderboss).unwrap();
    validate_set_character_autonomy(&micromanaged, city.player, captain, AutonomyLevel::Guided)
        .unwrap()
        .commit(&mut micromanaged)
        .unwrap();
    let reduced = progress(&registry, &micromanaged).unwrap();
    assert_eq!(reduced.racket_daily_net, cash.racket_daily_net);
    assert_eq!(reduced.mature_associates, cash.mature_associates);
    assert_eq!(reduced.defended_underboss_districts, 3);
    assert_eq!(
        reduced.staffed_underboss_districts, 2,
        "a micromanaged captain is not independent"
    );
    assert!(!reduced.underboss_ready);
    let before = bincode::serialize(&build_save(&registry, &micromanaged).unwrap()).unwrap();
    assert_eq!(
        held.commit(&registry, &mut micromanaged).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &micromanaged).unwrap()).unwrap()
    );
    let mut restored =
        restore_save(&registry, build_save(&registry, &micromanaged).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), reduced);
    for branch in [&mut micromanaged, &mut restored] {
        validate_set_character_autonomy(branch, city.player, captain, AutonomyLevel::Delegated)
            .unwrap()
            .commit(branch)
            .unwrap();
        assert!(progress(&registry, branch).unwrap().underboss_ready);
        validate_state_against_registry(&registry, branch).unwrap();
    }
    assert_eq!(
        bincode::serialize(&micromanaged).unwrap(),
        bincode::serialize(&restored).unwrap()
    );

    // A fourth defended, politically backed ward has enough proven workers,
    // but only one captain. A senior commander whose two direct reports cover
    // that ward and one complete ward cannot supervise two complete teams.
    let mut fragmented = city.state.clone();
    let incomplete_district = fragmented
        .world()
        .neighborhoods()
        .map(|district| district.id())
        .find(|district| ![home, empty_districts[0], empty_districts[1]].contains(district))
        .unwrap();
    let cash_account = fragmented
        .enterprises()
        .get_enterprise(city.starter_racket)
        .unwrap()
        .cash_account();
    let original_reports = fragmented
        .world()
        .characters_in_organization(city.player)
        .filter(|person| person.supervisor() == Some(senior))
        .map(|person| person.id())
        .collect::<Vec<_>>();
    assert_eq!(original_reports.len(), 3);
    let other_captains = fragmented
        .enterprises()
        .active_for_organization(city.player)
        .map(|racket| racket.manager())
        .filter(|&manager| manager != original_reports[0])
        .collect::<BTreeSet<_>>();
    for report in other_captains {
        if fragmented
            .world()
            .get_character(report)
            .unwrap()
            .supervisor()
            != Some(city.boss)
        {
            validate_set_character_supervisor(&fragmented, report, Some(city.boss))
                .unwrap()
                .commit(&mut fragmented)
                .unwrap();
        }
    }
    let isolated_captain = insert_character(
        &mut fragmented,
        CharacterDraft {
            name: "Isolated fourth-ward captain".into(),
            organization: Some(city.player),
            supervisor: Some(senior),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(100).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Neighborhood(incomplete_district);
    let mandate = validate_assign_mandate(
        &fragmented,
        MandateDraft {
            organization: city.player,
            manager: isolated_captain,
            scopes: BTreeSet::from([
                scope,
                ResponsibilityScope::Function(ResponsibilityFunction::Political),
            ]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fragmented)
    .unwrap();
    for (slot, kind) in [
        EnterpriseKind::Gambling,
        EnterpriseKind::Protection,
        EnterpriseKind::Gambling,
    ]
    .into_iter()
    .enumerate()
    {
        let venue = insert_business(
            &registry,
            &mut fragmented,
            BusinessDraft {
                name: format!("Fourth ward front {slot}"),
                kind: BusinessKind::Hospitality,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::MeetingSpace,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood: incomplete_district,
                owner: BusinessOwner::Organization(city.player),
            },
        )
        .unwrap();
        let settlement = insert_account(
            &mut fragmented,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(city.player),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        let racket = validate_establish_enterprise(
            &registry,
            &fragmented,
            EnterpriseDraft {
                kind,
                organization: city.player,
                authority: MandateAuthority {
                    mandate,
                    manager: isolated_captain,
                    scope,
                },
                location: EnterpriseLocation::Business(venue),
                supporting_businesses: BTreeSet::new(),
                cash_account,
                settlement_account: settlement,
            },
        )
        .unwrap()
        .commit(&mut fragmented)
        .unwrap();
        validate_staff_enterprise(&fragmented, racket, 10)
            .unwrap()
            .commit(&mut fragmented)
            .unwrap();
    }
    for index in 0..3 {
        insert_character(
            &mut fragmented,
            CharacterDraft {
                name: format!("Fourth ward defender {index}"),
                organization: Some(city.player),
                supervisor: Some(city.boss),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Violence,
                    Rating::try_new(75).unwrap(),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
    }
    fund(&mut fragmented, city.player, 100_000_000);
    for _ in 0..VICTORY_ESTABLISHMENT_DAYS {
        fragmented.advance_clock(crate::core::time::DAY_DURATION);
        let rackets = fragmented
            .enterprises()
            .active_for_organization(city.player)
            .map(|racket| racket.id())
            .collect::<Vec<_>>();
        for racket in rackets {
            let plan = decide_enterprise_cycle(
                &registry,
                &fragmented,
                racket,
                EnterpriseCycleRandomness::new(
                    0,
                    EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
                ),
            )
            .unwrap();
            validate_enterprise_cycle_plan(&fragmented, plan)
                .unwrap()
                .commit(&mut fragmented)
                .unwrap();
        }
    }
    validate_invest_influence(&fragmented, incomplete_district, influence_cost)
        .unwrap()
        .commit(&mut fragmented)
        .unwrap();
    validate_civic_initiative(
        &fragmented,
        incomplete_district,
        isolated_captain,
        CivicInitiative::LaborCoalition,
    )
    .unwrap()
    .commit(&mut fragmented)
    .unwrap();
    let fragmented_standing = progress(&registry, &fragmented).unwrap();
    assert_eq!(
        fragmented_standing.defended_underboss_districts, 4,
        "{fragmented_standing:?}"
    );
    assert_eq!(fragmented_standing.influential_underboss_districts, 3);
    assert!(fragmented_standing.racket_self_sufficient);
    assert!(
        fragmented_standing.underboss_liquid_reserve
            >= fragmented_standing.required_underboss_liquid_reserve
    );
    assert_eq!(
        fragmented_standing.senior_underboss_commands, 0,
        "the senior cannot rely on an unmatchable ward"
    );
    assert_eq!(
        validate_claim_victory(&registry, &fragmented, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    let mut restored =
        restore_save(&registry, build_save(&registry, &fragmented).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), fragmented_standing);
    for branch in [&mut fragmented, &mut restored] {
        validate_set_character_supervisor(branch, original_reports[1], Some(senior))
            .unwrap()
            .commit(branch)
            .unwrap();
        assert!(progress(&registry, branch).unwrap().underboss_ready);
        validate_state_against_registry(&registry, branch).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fragmented).unwrap(),
        bincode::serialize(&restored).unwrap()
    );

    let mut isolated = city.state.clone();
    validate_terminate_contact(&isolated, contacts[0])
        .unwrap()
        .commit(&mut isolated)
        .unwrap();
    let no_reach = progress(&registry, &isolated).unwrap();
    assert_eq!(no_reach.underboss_contact_kinds, 2);
    assert_eq!(no_reach.staffed_underboss_districts, 3);
    assert_eq!(
        validate_claim_victory(&registry, &isolated, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );

    let mut flat = city.state.clone();
    let captains = flat
        .enterprises()
        .active_for_organization(city.player)
        .map(|racket| racket.manager())
        .collect::<BTreeSet<_>>();
    for captain in captains {
        if flat.world().get_character(captain).unwrap().supervisor() != Some(city.boss) {
            validate_set_character_supervisor(&flat, captain, Some(city.boss))
                .unwrap()
                .commit(&mut flat)
                .unwrap();
        }
    }
    let flat_progress = progress(&registry, &flat).unwrap();
    assert_eq!(flat_progress.staffed_underboss_districts, 3);
    assert_eq!(flat_progress.senior_underboss_commands, 0);
    assert_eq!(
        validate_claim_victory(&registry, &flat, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady)
    );
    // Holding only the former six-month payroll reserve is insufficient when
    // the current network also has real wages and operating expenses.
    let payroll_only = cash.daily_payroll.cents() * UNDERBOSS_LIQUID_RUNWAY_DAYS as i64;
    let mut unfunded = city.state.clone();
    let sources = unfunded
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash | AccountKind::AccountedFunds
            )
        })
        .filter_map(|account| {
            let amount = account.available_payment_balance();
            (amount > Money::ZERO).then_some((account.id(), amount))
        })
        .collect::<Vec<_>>();
    let total = Money::from_cents(cash.underboss_liquid_reserve.cents() - payroll_only);
    let sink = insert_account(
        &mut unfunded,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(city.player),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let mut remaining = total;
    let mut postings = Vec::new();
    for (account, balance) in sources {
        let amount = remaining.min(balance);
        if amount > Money::ZERO {
            postings.push(LedgerPosting {
                account,
                amount: amount.checked_neg().unwrap(),
            });
            remaining = remaining.checked_sub(amount).unwrap();
        }
    }
    assert_eq!(remaining, Money::ZERO);
    postings.push(LedgerPosting {
        account: sink,
        amount: total,
    });
    validate_record_transaction(
        &unfunded,
        LedgerTransactionDraft {
            occurred_at: unfunded.now(),
            memo: "District reserve committed to unavailable clearing".into(),
            postings,
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut unfunded)
    .unwrap();
    let depleted = progress(&registry, &unfunded).unwrap();
    assert_eq!(
        depleted.staffed_underboss_districts,
        cash.staffed_underboss_districts
    );
    assert_eq!(depleted.racket_daily_net, cash.racket_daily_net);
    assert_eq!(depleted.underboss_liquid_reserve.cents(), payroll_only);
    assert!(depleted.underboss_liquid_reserve < depleted.required_underboss_liquid_reserve);
    let before = bincode::serialize(&build_save(&registry, &unfunded).unwrap()).unwrap();
    assert_eq!(
        validate_claim_victory(&registry, &unfunded, VictoryPath::CityUnderboss).err(),
        Some(VictoryError::UnderbossNotReady),
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &unfunded).unwrap()).unwrap()
    );
    validate_state_against_registry(&registry, &unfunded).unwrap();
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
    assert_eq!(required_underboss_personnel(2), 24);
    assert_eq!(required_underboss_personnel(3), 24);
    assert_eq!(required_underboss_personnel(6), 42);
    assert_eq!(required_underboss_personnel(12), 78);
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
fn recent_enterprise_losses_survive_suspension_and_retirement() {
    let (registry, mut state, owner, boss, _) = retirement_fixture();
    let rating = |value| Rating::try_new(value).unwrap();
    let district = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Low-demand ward".into(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(0),
                    commercial_activity: rating(0),
                    illicit_demand: rating(2),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(44),
                },
            },
        },
    )
    .unwrap();
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Inexperienced manager".into(),
            organization: Some(owner),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Neighborhood(district);
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization: owner,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let enterprise = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization: owner,
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
    .commit(&mut state)
    .unwrap();
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    let plan = decide_enterprise_cycle(
        &registry,
        &state,
        enterprise,
        EnterpriseCycleRandomness::new(
            -800,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .unwrap();
    validate_enterprise_cycle_plan(&state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let loss = state
        .enterprises()
        .latest_cycle(enterprise)
        .unwrap()
        .net_cash();
    assert!(loss.cents() < -30);
    let daily_loss = Money::from_cents(loss.cents() / VICTORY_INCOME_WINDOW_DAYS as i64);
    assert_eq!(
        progress(&registry, &state).unwrap().racket_daily_net,
        daily_loss
    );
    validate_suspend_enterprise(&state, enterprise)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    assert_eq!(
        progress(&registry, &state).unwrap().racket_daily_net,
        daily_loss
    );
    validate_retire_enterprise(&state, enterprise)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let standing = progress(&registry, &state).unwrap();
    assert_eq!(standing.racket_daily_net, daily_loss);
    assert_eq!(standing.established_racket_districts, 0);
    let restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), standing);
    state.advance_clock(SimDuration::from_minutes(
        (VICTORY_INCOME_WINDOW_DAYS * DAY_MINUTES) as u32,
    ));
    assert_eq!(
        progress(&registry, &state).unwrap().racket_daily_net,
        Money::ZERO
    );
}

#[test]
fn recent_storefront_losses_survive_suspension_and_title_transfer() {
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan, validate_establish_business_economy,
        validate_suspend_business_economy,
    };
    use crate::world::world_system::{insert_business, validate_transfer_business_ownership};
    use crate::world::{BusinessDraft, BusinessFunction};

    let (registry, mut state, owner, _, _) = retirement_fixture();
    let rating = |value| Rating::try_new(value).unwrap();
    let district = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Depressed Ward".into(),
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
    let shop = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Loss-making warehouse".into(),
            kind: BusinessKind::Warehouse,
            functions: BTreeSet::from([BusinessFunction::Warehousing]),
            neighborhood: district,
            owner: BusinessOwner::Organization(owner),
        },
    )
    .unwrap();
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(shop),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(shop),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business: shop,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    let plan = decide_business_cycle(&registry, &state, shop, -500).unwrap();
    validate_business_cycle_plan(&state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let loss = state.economy().latest_cycle(shop).unwrap().net_cash();
    assert!(loss.cents() < -30);
    let daily_loss = Money::from_cents(loss.cents() / VICTORY_INCOME_WINDOW_DAYS as i64);
    assert_eq!(
        progress(&registry, &state).unwrap().legitimate_daily_net,
        daily_loss
    );
    validate_suspend_business_economy(&state, shop)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    assert_eq!(
        progress(&registry, &state).unwrap().legitimate_daily_net,
        daily_loss
    );
    validate_transfer_business_ownership(&state, shop, BusinessOwner::Independent)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let standing = progress(&registry, &state).unwrap();
    assert_eq!(standing.legitimate_daily_net, daily_loss);
    assert_eq!(standing.owned_operating_business_districts, 0);
    assert_eq!(standing.established_business_districts, 0);
    let restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), standing);
    state.advance_clock(SimDuration::from_minutes(
        (VICTORY_INCOME_WINDOW_DAYS * DAY_MINUTES) as u32,
    ));
    assert_eq!(
        progress(&registry, &state).unwrap().legitimate_daily_net,
        Money::ZERO
    );
}

#[test]
fn political_income_requires_proven_recent_storefront_operation() {
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan, validate_establish_business_economy,
        validate_resume_business_economy, validate_suspend_business_economy,
    };
    use crate::world::BusinessDraft;
    use crate::world::world_system::insert_business;

    let (registry, mut state, owner, _, _) = retirement_fixture();
    let rating = Rating::try_new(90).unwrap();
    let district = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Prosperous ward".into(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating,
                    commercial_activity: rating,
                    illicit_demand: rating,
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(10).unwrap(),
                },
            },
        },
    )
    .unwrap();
    let shop = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Long-held idle storefront".into(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::new(),
            neighborhood: district,
            owner: BusinessOwner::Organization(owner),
        },
    )
    .unwrap();
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(shop),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(shop),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &state,
        BusinessEconomyDraft {
            business: shop,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();
    validate_suspend_business_economy(&state, shop)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    state.advance_clock(SimDuration::from_minutes(
        (POLITICAL_BUSINESS_ESTABLISHMENT_DAYS * DAY_MINUTES) as u32,
    ));
    validate_resume_business_economy(&registry, &state, shop)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let settle = |state: &mut AppState| {
        state.advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
        validate_business_cycle_plan(
            state,
            decide_business_cycle(&registry, state, shop, 0).unwrap(),
        )
        .unwrap()
        .commit(state)
        .unwrap();
    };
    for _ in 1..VICTORY_PROFITABLE_CYCLES {
        settle(&mut state);
    }
    let before = political_business_progress(&state, district).unwrap()[0];
    assert_eq!(before.profitable_cycles, VICTORY_PROFITABLE_CYCLES - 1);
    assert!(before.recent_net > Money::ZERO);
    assert!(!before.established);
    let row = political_district_progress(&registry, &state).unwrap()[0];
    assert_eq!(row.mature_active, 1);
    assert_eq!(row.established_active, 0);
    assert_eq!(
        row.mature_window_net,
        Money::ZERO,
        "idle ownership alone must not turn unproven earnings into political income"
    );
    assert_eq!(
        progress(&registry, &state).unwrap().legitimate_daily_net,
        Money::ZERO
    );

    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    for branch in [&mut state, &mut restored] {
        settle(branch);
        let title = political_business_progress(branch, district).unwrap()[0];
        assert!(title.established);
        assert_eq!(title.profitable_cycles, VICTORY_PROFITABLE_CYCLES);
        assert_eq!(
            political_district_progress(&registry, branch).unwrap()[0].mature_window_net,
            title.recent_net,
        );
        assert_eq!(
            progress(&registry, branch)
                .unwrap()
                .legitimate_daily_net
                .cents(),
            title.recent_net.cents() / VICTORY_INCOME_WINDOW_DAYS as i64,
        );
        validate_state_against_registry(&registry, branch).unwrap();
    }
    assert_eq!(
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
    );
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
    for _ in 0..POLITICAL_BUSINESS_ESTABLISHMENT_DAYS {
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
    let district = city
        .state
        .world()
        .get_business(storefront)
        .unwrap()
        .neighborhood();
    let title = political_business_progress(&city.state, district)
        .unwrap()
        .into_iter()
        .find(|title| title.business == storefront)
        .expect("acquired storefront appears in its district's title evidence");
    assert!(title.established);
    assert!(title.recent_net > Money::ZERO);
    assert!(
        i128::from(title.recent_net.cents())
            < i128::from(one_wage) * i128::from(VICTORY_INCOME_WINDOW_DAYS)
    );
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
fn political_legitimacy_requires_a_year_after_criminal_operating_income() {
    use crate::city_generation::{CityScale, generate_city};
    use crate::enterprises::enterprise_execution::{
        EnterpriseCycleRandomness, decide_enterprise_cycle, validate_enterprise_cycle_plan,
        validate_suspend_enterprise,
    };
    use crate::opportunities::OpportunityStatus;
    use crate::opportunities::opportunity_system::validate_dismiss_opportunity;
    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(104)).unwrap();
    let racket = city
        .state
        .enterprises()
        .enterprises_for_organization(city.player)
        .next()
        .unwrap()
        .id();
    let open = city
        .state
        .opportunities()
        .opportunities_for_organization(city.player)
        .filter(|lead| lead.status() == OpportunityStatus::Open)
        .map(|lead| lead.id())
        .collect::<Vec<_>>();
    for lead in open {
        validate_dismiss_opportunity(&registry, &city.state, lead)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    city.state
        .advance_clock(crate::core::time::SimDuration::from_minutes(
            DAY_MINUTES as u32,
        ));
    let cycle = decide_enterprise_cycle(
        &registry,
        &city.state,
        racket,
        EnterpriseCycleRandomness::new(
            0,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .unwrap();
    validate_enterprise_cycle_plan(&city.state, cycle)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    validate_suspend_enterprise(&city.state, racket)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let recent = progress(&registry, &city.state).unwrap();
    assert_eq!(recent.days_since_last_racket_cycle, Some(0));
    assert!(!recent.legitimate_transition_ready);
    city.state
        .advance_clock(crate::core::time::SimDuration::from_minutes(
            ((POLITICAL_CLEAN_OPERATION_DAYS - 1) * DAY_MINUTES) as u32,
        ));
    let almost = progress(&registry, &city.state).unwrap();
    assert_eq!(almost.days_since_last_racket_cycle, Some(364));
    assert!(!almost.legitimate_transition_ready);
    city.state
        .advance_clock(crate::core::time::SimDuration::from_minutes(
            DAY_MINUTES as u32,
        ));
    let clean = progress(&registry, &city.state).unwrap();
    assert_eq!(clean.days_since_last_racket_cycle, Some(365));
    assert!(clean.legitimate_transition_ready);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(progress(&registry, &restored).unwrap(), clean);
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
