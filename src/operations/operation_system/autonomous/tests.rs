use super::*;
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_test_tick;
use crate::delegation::MandateDraft;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crate::world::{
    BusinessDraft, BusinessFunction, BusinessOwner, CapabilityKind, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, Rating,
};

struct Fixture {
    registry: Registry,
    state: AppState,
    rival: OrganizationId,
    player: OrganizationId,
    competitor: BusinessId,
    manager: CharacterId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).unwrap()
}

fn fixture() -> Fixture {
    let registry = crate::build_registry();
    let mut state = AppState::new(0x95AC);
    let mut organization = |name: &str| {
        insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: name.to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap()
    };
    let rival = organization("Harbor Crew");
    let player = organization("Finch Holdings");
    designate_player_organization(&mut state, player).unwrap();

    let mut district = |name: &str| {
        insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: name.to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(60),
                        commercial_activity: rating(60),
                        illicit_demand: rating(55),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(0),
                    },
                },
            },
        )
        .unwrap()
    };
    let harbor = district("Harbor");
    let elsewhere = district("Uptown");
    let mut venue = |name: &str, owner, neighborhood| {
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: name.to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::ProfessionalRecords,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(owner),
            },
        )
        .unwrap();
        let operating_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .unwrap();
        let settlement_account = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_establish_business_economy(
            &registry,
            &state,
            BusinessEconomyDraft {
                business,
                operating_account,
                settlement_account,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        business
    };
    // The lower-ID foreign property is out of territory and must not be selected.
    venue("Uptown Store", player, elsewhere);
    venue("Harbor Crew Store", rival, harbor);
    let competitor = venue("Finch Harbor Store", player, harbor);

    let mut member = |organization, name: &str, capabilities| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities,
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap()
    };
    let manager = member(
        rival,
        "Harbor Leader",
        BTreeMap::from([(CapabilityKind::Management, rating(100))]),
    );
    member(
        rival,
        "Harbor Scout",
        BTreeMap::from([
            (CapabilityKind::Surveillance, rating(100)),
            (CapabilityKind::Burglary, rating(100)),
            (CapabilityKind::Stealth, rating(100)),
        ]),
    );
    let player_manager = member(
        player,
        "Finch Leader",
        BTreeMap::from([
            (CapabilityKind::Management, rating(100)),
            (CapabilityKind::Surveillance, rating(100)),
        ]),
    );
    for (organization, manager) in [(rival, manager), (player, player_manager)] {
        validate_assign_mandate(
            &state,
            MandateDraft {
                organization,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Operations,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
    }
    Fixture {
        registry,
        state,
        rival,
        player,
        competitor,
        manager,
    }
}

fn next_day(registry: &Registry, state: &mut AppState) -> crate::core::simulation::TickOutcome {
    let remaining = DAY_MINUTES - state.now().as_minutes() % DAY_MINUTES;
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(remaining - 1).unwrap(),
    ));
    run_test_tick(registry, state)
}

fn finish_operation(registry: &Registry, state: &mut AppState, id: crate::core::id::OperationId) {
    // Execute the real tick, not a fabricated operation result or economy mutation.
    for _ in 0..400 {
        run_test_tick(registry, state);
        if !ACTIVE_ASSIGNMENT_STATUSES
            .contains(&state.operations().get_operation(id).unwrap().status())
        {
            return;
        }
    }
    panic!("operation {id} did not terminate");
}

#[test]
fn rival_scouts_local_competitor_then_disrupts_actual_player_business_after_restore() {
    let Fixture {
        registry,
        mut state,
        rival,
        player,
        competitor,
        ..
    } = fixture();
    let tick = next_day(&registry, &mut state);
    assert_eq!(
        tick.autonomous_operations.len(),
        1,
        "player mandates must not authorize NPC moves"
    );
    let scout = tick.autonomous_operations[0];
    let record = state.operations().get_operation(scout).unwrap();
    assert_eq!(record.kind(), OperationKind::Surveillance);
    assert_eq!(record.responsible_organization(), rival);
    assert_eq!(record.objective().business_target(), Some(competitor));
    assert!(
        state
            .operations()
            .operations_for_organization(player)
            .next()
            .is_none()
    );
    assert!(
        apply_due_rival_operations(&registry, &mut state).is_empty(),
        "no second plan with active work"
    );
    finish_operation(&registry, &mut state, scout);
    assert!(
        state
            .intelligence()
            .information_for_holder_subject(
                KnowledgeHolder::Organization(rival),
                EntityRef::Business(competitor)
            )
            .any(|info| info.topic() == InformationTopic::MarketAccess
                && info.source_kind() == InformationSourceKind::Surveillance)
    );

    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut state);
    let replay = next_day(&registry, &mut restored);
    assert_eq!(tick, replay);
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    assert_eq!(tick.autonomous_operations.len(), 1);
    let strike = tick.autonomous_operations[0];
    let record = state.operations().get_operation(strike).unwrap();
    assert_eq!(record.kind(), OperationKind::Sabotage);
    assert_eq!(record.objective().business_target(), Some(competitor));
    assert_eq!(record.responsible_organization(), rival);
    finish_operation(&registry, &mut state, strike);
    assert!(
        state
            .economy()
            .get_business_economy(competitor)
            .unwrap()
            .is_disrupted(state.now())
    );
    let settlement = next_day(&registry, &mut state);
    assert!(settlement.business_cycles.iter().any(|cycle| {
        let cycle = state.economy().get_cycle(*cycle).unwrap();
        cycle.business() == competitor
            && cycle.disrupted()
            && cycle.attention() == crate::core::attention::AttentionClass::Notable
    }));
    crate::core::invariants::validate_state(&state).unwrap();
    restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
}

#[test]
fn rival_without_operational_authority_cannot_launch_pressure() {
    let Fixture {
        registry,
        mut state,
        manager,
        ..
    } = fixture();
    let mandate = state.delegation().active_for_manager(manager).unwrap().id();
    crate::delegation::delegation_system::validate_revoke_mandate(&state, mandate)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let tick = next_day(&registry, &mut state);
    assert!(tick.autonomous_operations.is_empty());
}

#[test]
fn rival_stands_down_under_own_police_fear() {
    let Fixture {
        registry,
        mut state,
        rival,
        ..
    } = fixture();
    crate::reputation::reputation_system::apply_reputation_delta(
        &registry,
        &mut state,
        rival,
        AudienceKind::Police,
        100,
    )
    .unwrap();
    let tick = next_day(&registry, &mut state);
    assert!(tick.autonomous_operations.is_empty());
}
