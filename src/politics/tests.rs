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
use crate::world::world_system::{insert_business, insert_character, insert_organization};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    OrganizationDraft, OrganizationKind,
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
fn public_machine_influence_counts_without_property_titles() {
    use crate::world::organization_pressure::public_prominence_tier;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(3_411)).unwrap();
    let owner = city.player;
    let title_districts: std::collections::BTreeSet<_> = city
        .state
        .world()
        .businesses_owned_by_organization(owner)
        .map(|business| business.neighborhood())
        .collect();
    let wards: Vec<_> = city
        .state
        .world()
        .neighborhoods()
        .map(|ward| ward.id())
        .filter(|ward| !title_districts.contains(ward))
        .take(2)
        .collect();
    assert_eq!(wards.len(), 2);
    let baseline = public_prominence_tier(&city.state, owner);
    let investment = i64::from(INFLUENCE_CAP) * INFLUENCE_COST_PER_POINT;
    fund(&mut city.state, owner, 2 * investment);
    for ward in wards {
        validate_invest_influence(&city.state, ward, investment)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    assert!(public_prominence_tier(&city.state, owner) > baseline);
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        public_prominence_tier(&restored, owner),
        public_prominence_tier(&city.state, owner)
    );
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
fn political_spending_is_bound_to_the_paying_organization_and_actual_ledger() {
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::LoadError;
    use crate::finance::finance_system::FinanceError;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(8_731)).unwrap();
    let rival = city.rivals[0];
    let home = city.home;
    for owner in [city.player, rival] {
        fund(&mut city.state, owner, 1_000_000);
        political_system::validate_invest_for(&city.state, owner, home, 500_000)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    let original = city.state.politics().organization(city.player).unwrap();
    let clearing = original.clearing_account.unwrap();
    let rival_clearing = city
        .state
        .politics()
        .organization(rival)
        .unwrap()
        .clearing_account
        .unwrap();
    assert_ne!(clearing, rival_clearing);
    let envelope = build_save(&registry, &city.state).unwrap();
    let bytes = bincode::serialize(&envelope).unwrap();
    let campaign_bytes = bincode::serialize(&(city.player, original)).unwrap();
    let offsets: Vec<_> = bytes
        .windows(campaign_bytes.len())
        .enumerate()
        .filter_map(|(i, window)| (window == campaign_bytes.as_slice()).then_some(i))
        .collect();
    assert_eq!(offsets.len(), 1);
    for mode in 0..3 {
        // Corrupt only campaign metadata in a serialized envelope, leaving
        // both real organizations' balanced ledger histories untouched.
        let mut malformed = original.clone();
        match mode {
            0 => malformed.spent_cents += MAYORAL_CAMPAIGN_SPEND_REQUIRED,
            1 => malformed.spent_cents = 0,
            2 => malformed.clearing_account = Some(rival_clearing),
            _ => unreachable!(),
        }
        let replacement = bincode::serialize(&(city.player, malformed)).unwrap();
        let mut damaged = bytes.clone();
        damaged[offsets[0]..offsets[0] + campaign_bytes.len()].copy_from_slice(&replacement);
        assert_eq!(
            restore_save(&registry, bincode::deserialize(&damaged).unwrap()).err(),
            Some(LoadError::InvalidState(
                StateValidationError::InvalidPoliticalCampaign
            )),
            "campaign corruption mode {mode} must not survive restoration",
        );
    }
    let restored = restore_save(&registry, envelope).unwrap();
    assert_eq!(restored.politics(), city.state.politics());

    // A lower-level balanced transfer must not credit or drain an already
    // attached clearing account outside the political spending owner.
    let funding = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(city.player))
        .find(|account| {
            account.kind() == AccountKind::AccountedFunds
                && account.available_payment_balance() >= Money::from_cents(5000)
        })
        .unwrap()
        .id();
    let before = bincode::serialize(&city.state).unwrap();
    for cents in [-5000, 5000] {
        let draft = LedgerTransactionDraft {
            occurred_at: city.state.now(),
            memo: "Unrelated treasury transfer".into(),
            postings: vec![
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(cents),
                },
                LedgerPosting {
                    account: funding,
                    amount: Money::from_cents(-cents),
                },
            ],
            authorization: None,
        };
        assert_eq!(
            validate_record_transaction(&city.state, draft).err(),
            Some(FinanceError::ProtectedCampaignClearingAccount(clearing))
        );
        assert_eq!(before, bincode::serialize(&city.state).unwrap());
    }
    // Non-campaign municipal settlement accounts are still ordinary ledger
    // counterparties. Protection is specific to the assigned account.
    let institution = city.state.finance().get_account(clearing).unwrap().owner();
    let unrelated = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner: institution,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &city.state,
        LedgerTransactionDraft {
            occurred_at: city.state.now(),
            memo: "Unrelated municipal settlement".into(),
            postings: vec![
                LedgerPosting {
                    account: unrelated,
                    amount: Money::from_cents(5000),
                },
                LedgerPosting {
                    account: funding,
                    amount: Money::from_cents(-5000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for state in [&mut city.state, &mut restored] {
        validate_invest_influence(state, home, 5000)
            .unwrap()
            .commit(state)
            .unwrap();
        assert_eq!(
            state
                .politics()
                .organization(city.player)
                .unwrap()
                .spent_cents(),
            505_000
        );
        assert_eq!(
            state
                .finance()
                .get_account(clearing)
                .unwrap()
                .balance()
                .cents(),
            505_000
        );
        validate_invariants(state);
        build_save(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
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
fn current_save_rejects_one_manager_assigned_to_two_campaign_offices() {
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::LoadError;
    use crate::delegation::MandateDraft;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use std::collections::BTreeSet;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(5_611)).unwrap();
    let first = city.home;
    let second = city
        .state
        .world()
        .neighborhoods()
        .find(|ward| ward.id() != first)
        .unwrap()
        .id();
    fund(&mut city.state, city.player, 1_000_000);
    let mut managers = Vec::new();
    for district in [first, second] {
        let manager = insert_character(
            &mut city.state,
            CharacterDraft {
                name: format!("Office manager {district}"),
                organization: Some(city.player),
                supervisor: Some(city.boss),
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        validate_assign_mandate(
            &city.state,
            MandateDraft {
                organization: city.player,
                manager,
                scopes: BTreeSet::from([
                    ResponsibilityScope::Neighborhood(district),
                    ResponsibilityScope::Function(ResponsibilityFunction::Political),
                ]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
        validate_hire_specialist(&city.state, district, manager, SpecialistKind::Organizer)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        managers.push(manager);
    }
    let envelope = build_save(&registry, &city.state).unwrap();
    restore_save(&registry, envelope.clone()).unwrap();
    let office = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .district(second)
        .unwrap();
    // Corrupt only the second office's serialized assignment. Both identities
    // still exist, so reference checks alone would accept this invalid save.
    let office_bytes = bincode::serialize(&(second, office)).unwrap();
    let assignment = bincode::serialize(&(SpecialistKind::Organizer, managers[1])).unwrap();
    let replacement = bincode::serialize(&(SpecialistKind::Organizer, managers[0])).unwrap();
    let mut bytes = bincode::serialize(&envelope).unwrap();
    let unique_offset = |haystack: &[u8], needle: &[u8]| {
        let offsets: Vec<_> = haystack
            .windows(needle.len())
            .enumerate()
            .filter_map(|(i, window)| (window == needle).then_some(i))
            .collect();
        assert_eq!(offsets.len(), 1);
        offsets[0]
    };
    let offset = unique_offset(&bytes, &office_bytes) + unique_offset(&office_bytes, &assignment);
    bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
    let malformed = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(
        restore_save(&registry, malformed),
        Err(LoadError::InvalidState(
            StateValidationError::InvalidPoliticalCampaign
        ))
    ));
}

#[test]
fn permanent_campaign_offices_need_distinct_supervisors_and_reserve_real_contract_payroll() {
    use crate::core::campaign_victory::{UNDERBOSS_OPERATING_RUNWAY_DAYS, progress};
    use crate::politics::political_system::campaign_salary_commitment;
    use crate::world::world_system::validate_set_character_autonomy;

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(5_611)).unwrap();
    let district = city.home;
    let other = city
        .state
        .world()
        .neighborhoods()
        .find(|ward| ward.id() != district)
        .unwrap()
        .id();
    fund(&mut city.state, city.player, 3_000_000);
    let manager = city
        .state
        .enterprises()
        .get_enterprise(city.starter_racket)
        .unwrap()
        .manager();
    let mandate = city.state.delegation().active_for_manager(manager).unwrap();
    let mut scopes = mandate.scopes().clone();
    scopes.insert(ResponsibilityScope::Function(
        ResponsibilityFunction::Political,
    ));
    scopes.insert(ResponsibilityScope::Neighborhood(other));
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
    validate_invest_influence(&city.state, district, INFLUENCE_COST_PER_POINT)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let baseline = progress(&registry, &city.state).unwrap();
    let held =
        validate_hire_specialist(&city.state, other, manager, SpecialistKind::Organizer).unwrap();
    validate_hire_specialist(&city.state, district, manager, SpecialistKind::Strategist)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let before = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(
        held.commit(&mut city.state).err(),
        Some(PoliticalError::Stale)
    );
    assert_eq!(
        validate_hire_specialist(&city.state, other, manager, SpecialistKind::Organizer).err(),
        Some(PoliticalError::ManagerOfficeConflict)
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap()
    );
    validate_hire_specialist(&city.state, district, manager, SpecialistKind::Organizer)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let salary = Money::from_cents(
        SpecialistKind::Strategist.daily_salary() + SpecialistKind::Organizer.daily_salary(),
    );
    let commitment = campaign_salary_commitment(&city.state, city.player);
    assert_eq!(commitment.contracted, salary);
    assert_eq!(commitment.payable, salary);
    let expanded = progress(&registry, &city.state).unwrap();
    assert_eq!(expanded.campaign_daily_payroll, salary);
    assert_eq!(
        expanded.underboss_campaign_buffer.cents(),
        salary.cents() * UNDERBOSS_OPERATING_RUNWAY_DAYS as i64
    );
    assert_eq!(
        expanded.required_underboss_liquid_reserve.cents(),
        baseline.required_underboss_liquid_reserve.cents()
            + expanded.underboss_campaign_buffer.cents()
    );

    // Dormant offices earn no salary or influence, but their contractual
    // reserve remains until the organization actually releases the staff.
    validate_set_character_autonomy(&city.state, city.player, manager, AutonomyLevel::Tight)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let inactive = campaign_salary_commitment(&city.state, city.player);
    assert_eq!(inactive.contracted, salary);
    assert_eq!(inactive.payable, Money::ZERO);
    let spent = city
        .state
        .politics()
        .organization(city.player)
        .unwrap()
        .spent_cents();
    advance_day(&registry, &mut city.state);
    assert_eq!(
        city.state
            .politics()
            .organization(city.player)
            .unwrap()
            .spent_cents(),
        spent
    );
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(campaign_salary_commitment(&restored, city.player), inactive);
    assert_eq!(
        progress(&registry, &restored)
            .unwrap()
            .underboss_campaign_buffer,
        expanded.underboss_campaign_buffer
    );
    validate_set_character_autonomy(&city.state, city.player, manager, AutonomyLevel::Delegated)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    dismiss_specialist(&mut city.state, district, SpecialistKind::Strategist).unwrap();
    dismiss_specialist(&mut city.state, district, SpecialistKind::Organizer).unwrap();
    validate_hire_specialist(&city.state, other, manager, SpecialistKind::Organizer)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        campaign_salary_commitment(&city.state, city.player).contracted,
        Money::from_cents(SpecialistKind::Organizer.daily_salary())
    );
    validate_state_against_registry(&registry, &city.state).unwrap();
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
fn paid_orders_reconcile_majority_tenure_without_waiting_for_daily_maintenance() {
    use crate::core::campaign_victory::political_district_progress;
    use crate::delegation::MandateDraft;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::world::world_system::insert_neighborhood;
    use crate::world::{
        CapabilityKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
    };

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_217)).unwrap();
    let rating = |value| Rating::try_new(value).unwrap();
    // No rival storefronts here: the only opposition is the deliberate paid order below.
    let district = insert_neighborhood(
        &mut city.state,
        NeighborhoodDraft {
            name: "New civic ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(50),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(50),
                },
            },
        },
    )
    .unwrap();
    let manager = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Local organizer".to_owned(),
            organization: Some(city.player),
            supervisor: Some(city.boss),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::SocialAccess, rating(100)),
                (CapabilityKind::Negotiation, rating(100)),
            ]),
            traits: Default::default(),
            drives: Default::default(),
        },
    )
    .unwrap();
    validate_assign_mandate(
        &city.state,
        MandateDraft {
            organization: city.player,
            manager,
            scopes: [
                ResponsibilityScope::Neighborhood(district),
                ResponsibilityScope::Function(ResponsibilityFunction::Political),
            ]
            .into_iter()
            .collect(),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let rival = city.rivals[0];
    fund(&mut city.state, city.player, 15_000_000);
    fund(&mut city.state, rival, 5_000_000);
    for kind in [SpecialistKind::Organizer, SpecialistKind::Publicist] {
        validate_hire_specialist(&city.state, district, manager, kind)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
    }
    let office = |state: &crate::AppState| {
        state
            .politics()
            .organization(city.player)
            .unwrap()
            .district(district)
            .unwrap()
            .clone()
    };
    for _ in 0..30 {
        let local = office(&city.state);
        if local.grassroots() >= GRASSROOTS_REQUIRED
            && local.institutions() >= INSTITUTIONAL_REQUIRED
        {
            break;
        }
        advance_day(&registry, &mut city.state);
    }
    let local = office(&city.state);
    assert!(
        local.grassroots() >= GRASSROOTS_REQUIRED && local.institutions() >= INSTITUTIONAL_REQUIRED
    );
    assert!(local.influence() < MAYORAL_INFLUENCE_REQUIRED);
    assert!(local.leading_since().is_none());

    // An ordinary paid investment, not a daily tick, crosses the majority threshold.
    city.state.advance_clock(SimDuration::ONE_MINUTE);
    let won_at = city.state.now();
    validate_invest_influence(
        &city.state,
        district,
        i64::from(MAYORAL_INFLUENCE_REQUIRED - local.influence()) * INFLUENCE_COST_PER_POINT,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert!(controls_district(&city.state, city.player, district));
    assert_eq!(office(&city.state).leading_since(), Some(won_at));

    city.state.advance_clock(SimDuration::ONE_MINUTE);
    political_system::validate_invest_for(
        &city.state,
        rival,
        district,
        550 * INFLUENCE_COST_PER_POINT,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    assert!(!controls_district(&city.state, city.player, district));
    assert!(office(&city.state).leading_since().is_none());
    let row = political_district_progress(&registry, &city.state)
        .unwrap()
        .into_iter()
        .find(|row| row.district == district)
        .unwrap();
    assert_eq!(row.majority_held_days, 0);

    // A later payment restores the lead before the next daily pass, but cannot inherit
    // the earlier streak. The new start is preserved exactly in the save.
    city.state.advance_clock(SimDuration::ONE_MINUTE);
    let regained_at = city.state.now();
    validate_invest_influence(&city.state, district, 200 * INFLUENCE_COST_PER_POINT)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert!(controls_district(&city.state, city.player, district));
    assert_eq!(office(&city.state).leading_since(), Some(regained_at));
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(office(&restored).leading_since(), Some(regained_at));
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn rival_campaign_reserve_counts_operating_cash_owned_by_the_organization() {
    let registry = build_registry();
    let mut state = crate::AppState::new(31);
    let owner = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cash-backed opposition".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    insert_character(
        &mut state,
        CharacterDraft {
            name: "Opposition executive".to_owned(),
            organization: Some(owner),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: std::collections::BTreeMap::new(),
            traits: std::collections::BTreeSet::new(),
            drives: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();
    let reserve = registry.upkeep().per_member_daily().cents() * 7;
    let campaign_cash = reserve + INFLUENCE_COST_PER_POINT * 5;
    fund(&mut state, owner, campaign_cash);
    assert_eq!(
        political_system::affordable_rival_campaign_points(&registry, &state, owner),
        5,
        "clean cash alone must preserve seven days of payroll"
    );

    let account_owner = FinancialOwner::Organization(owner);
    let operating = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: account_owner,
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: account_owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Fund the organization's operating wage reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: operating,
                    amount: Money::from_cents(reserve),
                },
                LedgerPosting {
                    account: settlement,
                    amount: Money::from_cents(-reserve),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut state)
    .unwrap();

    assert_eq!(
        political_system::affordable_rival_campaign_points(&registry, &state, owner),
        10,
        "a separate real wage reserve frees clean cash for competitive political spending"
    );
    assert_eq!(
        crate::world::payroll_execution::payroll_funding_snapshot(&registry, &state, owner)
            .unwrap()
            .1
            .cents(),
        registry.upkeep().per_member_daily().cents(),
        "payroll itself recognizes that operating balance"
    );
    validate_state_against_registry(&registry, &state).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    assert_eq!(
        political_system::affordable_rival_campaign_points(&registry, &restored, owner),
        10
    );
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
    let affordable =
        political_system::affordable_rival_campaign_points(&registry, &city.state, rival);
    assert!(
        (1..100).contains(&affordable),
        "funding cannot consume reserved wages"
    );
    advance_day(&registry, &mut city.state);
    assert_eq!(
        rival_pressure(&city.state, city.player, district),
        affordable
    );
    assert_eq!(
        city.state
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents(),
        i64::from(affordable) * INFLUENCE_COST_PER_POINT
    );
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    let spent = city
        .state
        .politics()
        .organization(rival)
        .unwrap()
        .spent_cents();
    assert!(
        spent >= i64::from(affordable) * INFLUENCE_COST_PER_POINT,
        "later spending must be supported by real new funds"
    );
    let restored = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    assert_eq!(restored.politics(), city.state.politics());
}

#[test]
fn rival_campaign_requires_available_executive() {
    use crate::core::entity::EntityRef;
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::validate_establish_business_economy;
    use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft,
    };

    let registry = build_registry();
    let mut city = generate_city(&registry, CityScale::Small.config(6_218)).unwrap();
    let district = city.home;
    let rival = insert_organization(
        &registry,
        &mut city.state,
        OrganizationDraft {
            name: "Leaderless Commercial Rival".into(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let business = insert_business(
        &registry,
        &mut city.state,
        BusinessDraft {
            name: "Leaderless Retail Holding".into(),
            kind: BusinessKind::Retail,
            functions: std::collections::BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            neighborhood: district,
            owner: BusinessOwner::Organization(rival),
        },
    )
    .unwrap();
    let operating = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut city.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(business),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &registry,
        &city.state,
        BusinessEconomyDraft {
            business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    fund(&mut city.state, rival, 2_000_000);
    fund(&mut city.state, city.player, 1_000_000);
    validate_invest_influence(&city.state, district, 1_000_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert!(political_system::affordable_rival_campaign_points(&registry, &city.state, rival) > 0);
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    assert!(
        city.state.politics().organization(rival).is_none(),
        "a funded organization without any named executive cannot commission political spending"
    );
    let cash = city
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(rival))
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap();
    assert_eq!(cash.balance(), Money::from_cents(2_000_000));

    let executive = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "New Executive".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: Default::default(),
            traits: Default::default(),
            drives: Default::default(),
        },
    )
    .unwrap();
    insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Commercial Subordinate".into(),
            organization: Some(rival),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: Default::default(),
            traits: Default::default(),
            drives: Default::default(),
        },
    )
    .unwrap();
    let mut ambiguous = city.state.clone();
    insert_character(
        &mut ambiguous,
        CharacterDraft {
            name: "Competing Executive".into(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: Default::default(),
            traits: Default::default(),
            drives: Default::default(),
        },
    )
    .unwrap();
    for _ in 0..7 {
        advance_day(&registry, &mut ambiguous);
    }
    assert!(
        ambiguous.politics().organization(rival).is_none(),
        "ambiguous executive authority cannot commission a campaign"
    );
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
    }
    let spent = city
        .state
        .politics()
        .organization(rival)
        .unwrap()
        .spent_cents();
    assert!(
        spent > 0,
        "available leadership can commission a funded campaign"
    );
    for _ in 0..6 {
        advance_day(&registry, &mut city.state);
    }
    let prior_influence = rival_pressure(&city.state, city.player, district);
    assert!(prior_influence > 0);
    city.state
        .advance_clock(SimDuration::from_minutes(DAY_MINUTES as u32));
    crate::opportunities::opportunity_system::apply_opportunity_expiry(&registry, &mut city.state)
        .unwrap();
    let police = city
        .state
        .world()
        .organizations()
        .find(|org| org.kind() == OrganizationKind::LawEnforcement)
        .unwrap()
        .id();
    let investigation = validate_open_investigation(
        &city.state,
        InvestigationDraft {
            owner: police,
            title: "Executive custody".into(),
            subjects: std::collections::BTreeSet::from([EntityRef::Character(executive)]),
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let mut evidence = std::collections::BTreeSet::new();
    for kind in [EvidenceKind::Document, EvidenceKind::KnownAssociation] {
        evidence.insert(
            validate_add_evidence(
                &city.state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject: EntityRef::Character(executive),
                    origin: None,
                    kind,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: city.state.now(),
                },
            )
            .unwrap()
            .commit(&mut city.state)
            .unwrap(),
        );
    }
    let arrest = validate_arrest(
        &registry,
        &city.state,
        ArrestDraft {
            character: executive,
            investigation,
            evidence,
        },
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for state in [&mut city.state, &mut restored] {
        apply_due_political_campaigns(&registry, state);
        let campaign = state.politics().organization(rival).unwrap();
        assert_eq!(
            campaign.spent_cents(),
            spent,
            "detained leadership must not authorize a new campaign through an ordinary subordinate"
        );
        assert!(
            campaign.district(district).unwrap().influence() < prior_influence,
            "existing influence still decays"
        );
        validate_release_arrest(state, arrest)
            .unwrap()
            .commit(state)
            .unwrap();
        for _ in 0..7 {
            advance_day(&registry, state);
        }
        assert!(state.politics().organization(rival).unwrap().spent_cents() > spent);
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&city.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
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
    let first_spent = city
        .state
        .politics()
        .organization(rival)
        .unwrap()
        .spent_cents();
    // The player's visible campaign lifts the opening footprint to tier one,
    // increasing the rival's weekly capacity from 100 to 120 points.
    assert_eq!(influence(&city.state, second), 120);
    // After the strongest challenge is defended, redirect the finite weekly
    // budget toward the next exposed ward, not an arbitrary lowest-ID ward.
    political_system::validate_invest_for(&city.state, rival, second, 4_400_000)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    let second_week_affordable =
        political_system::affordable_rival_campaign_points(&registry, &city.state, rival);
    assert!((1..=100).contains(&second_week_affordable));
    let mut restored =
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    for _ in 0..7 {
        advance_day(&registry, &mut city.state);
        advance_day(&registry, &mut restored);
    }
    assert_eq!(influence(&city.state, first), second_week_affordable);
    assert_eq!(influence(&restored, first), second_week_affordable);
    assert_eq!(influence(&restored, second), influence(&city.state, second));
    assert!(
        city.state
            .politics()
            .organization(rival)
            .unwrap()
            .spent_cents()
            > first_spent
    );
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
}

#[test]
fn rival_with_a_well_defended_weak_challenge_preserves_funds_in_an_uncontested_ward() {
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
    assert!(campaign.district(other).is_none());
    assert!(campaign.district(contested).unwrap().influence() > 600);
    assert_eq!(campaign.spent_cents(), 4_500_000);
    validate_invariants(&city.state);
    validate_state_against_registry(&registry, &city.state).unwrap();
}
