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
    fixture_with_scout_rating(100)
}

fn fixture_with_scout_rating(scout_rating: u8) -> Fixture {
    fixture_with_competitors(scout_rating, None)
}

fn fixture_with_competitors(scout_rating: u8, second_competitor: Option<BusinessKind>) -> Fixture {
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
    let mut venue = |name: &str, owner, neighborhood, kind| {
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: name.to_owned(),
                kind,
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
    venue("Uptown Store", player, elsewhere, BusinessKind::Retail);
    venue("Harbor Crew Store", rival, harbor, BusinessKind::Retail);
    let competitor = venue("Finch Harbor Store", player, harbor, BusinessKind::Retail);
    if let Some(kind) = second_competitor {
        venue("Second Finch Harbor Store", player, harbor, kind);
    }

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
            (CapabilityKind::Surveillance, rating(scout_rating)),
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

    let mut control = state.clone();
    let manager = control
        .world()
        .characters_in_organization(rival)
        .find(|person| {
            control
                .delegation()
                .active_for_manager(person.id())
                .is_some()
        })
        .unwrap()
        .id();
    let mandate = control
        .delegation()
        .active_for_manager(manager)
        .unwrap()
        .id();
    crate::delegation::delegation_system::validate_revoke_mandate(&control, mandate)
        .unwrap()
        .commit(&mut control)
        .unwrap();
    let rival_shop = state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .id();
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
    next_day(&registry, &mut control);
    next_day(&registry, &mut control);
    let rival_cycle = state.economy().latest_cycle(rival_shop).unwrap();
    let control_cycle = control.economy().latest_cycle(rival_shop).unwrap();
    assert_eq!(rival_cycle.occurred_at(), control_cycle.occurred_at());
    assert_eq!(
        rival_cycle.variance_basis_points(),
        control_cycle.variance_basis_points()
    );
    assert!(
        rival_cycle.gross_revenue() > control_cycle.gross_revenue(),
        "successful pressure must improve the attacker's own operating revenue"
    );
    assert!(
        rival_cycle.market_gross_basis_points(&registry)
            > control_cycle.market_gross_basis_points(&registry)
    );
    assert!(
        state
            .economy()
            .latest_cycle(competitor)
            .unwrap()
            .gross_revenue()
            < control
                .economy()
                .latest_cycle(competitor)
                .unwrap()
                .gross_revenue()
    );
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
fn rival_reassesses_market_after_a_concerted_campaign() {
    let Fixture {
        registry,
        mut state,
        rival,
        competitor,
        ..
    } = fixture_with_competitors(100, Some(BusinessKind::Pharmacy));
    let second = state
        .world()
        .public_business_directory()
        .find(|(_, name, _, _)| *name == "Second Finch Harbor Store")
        .unwrap()
        .0;
    for kind in [
        OperationKind::Surveillance,
        OperationKind::Sabotage,
        OperationKind::Sabotage,
    ] {
        let tick = next_day(&registry, &mut state);
        assert_eq!(tick.autonomous_operations.len(), 1);
        let id = tick.autonomous_operations[0];
        let record = state.operations().get_operation(id).unwrap();
        assert_eq!(record.responsible_organization(), rival);
        assert_eq!(record.kind(), kind);
        assert_eq!(record.objective().business_target(), Some(competitor));
        finish_operation(&registry, &mut state, id);
        assert!(
            state
                .operations()
                .get_operation(id)
                .unwrap()
                .resolution()
                .is_some_and(|outcome| outcome.objective_outcome()
                    != crate::operations::OperationObjectiveOutcome::Failed)
        );
    }
    let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
    let tick = next_day(&registry, &mut state);
    assert_eq!(tick, next_day(&registry, &mut restored));
    let record = state
        .operations()
        .get_operation(tick.autonomous_operations[0])
        .unwrap();
    assert_eq!(record.kind(), OperationKind::Surveillance);
    assert_eq!(record.objective().business_target(), Some(second));
    assert_eq!(
        bincode::serialize(&state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn held_business_settlement_stales_when_local_competitor_is_damaged() {
    use crate::economy::business_economy_system::{
        BusinessEconomyError, decide_business_cycle, validate_business_cycle_plan,
        validate_disrupt_business_economy,
    };
    let Fixture {
        registry,
        mut state,
        rival,
        competitor,
        ..
    } = fixture();
    let shop = state
        .world()
        .businesses_owned_by_organization(rival)
        .next()
        .unwrap()
        .id();
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    let plan = decide_business_cycle(&registry, &state, shop, 0).unwrap();
    let token = validate_business_cycle_plan(&state, plan.clone()).unwrap();
    validate_disrupt_business_economy(&registry, &state, competitor)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    let before = bincode::serialize(&state).unwrap();
    assert!(
        matches!(validate_business_cycle_plan(&state, plan), Err(BusinessEconomyError::StaleMarket(id)) if id == shop)
    );
    assert_eq!(
        token.commit(&mut state),
        Err(BusinessEconomyError::StaleMarket(shop))
    );
    assert_eq!(before, bincode::serialize(&state).unwrap());
    let plan = decide_business_cycle(&registry, &state, shop, 0).unwrap();
    let id = validate_business_cycle_plan(&state, plan)
        .unwrap()
        .commit(&mut state)
        .unwrap();
    assert!(
        state
            .economy()
            .get_cycle(id)
            .unwrap()
            .market_gross_basis_points(&registry)
            == 8_857
    );
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
fn rival_planner_rejects_untrained_scout() {
    let Fixture {
        registry,
        mut state,
        ..
    } = fixture_with_scout_rating(0);
    assert!(
        next_day(&registry, &mut state)
            .autonomous_operations
            .is_empty()
    );
}

#[test]
fn rival_planner_cooldown_starts_at_abort_not_authorization() {
    let Fixture {
        registry,
        mut state,
        ..
    } = fixture();
    let operation = next_day(&registry, &mut state).autonomous_operations[0];
    run_test_tick(&registry, &mut state);
    super::super::apply_transition(
        &registry,
        &mut state,
        operation,
        super::super::OperationTransition::Abort,
    )
    .unwrap();
    let retry_minutes = u32::try_from(UNSUCCESSFUL_RETRY_MINUTES - 1).unwrap();
    state.advance_clock(SimDuration::from_minutes(retry_minutes));
    assert!(apply_due_rival_operations(&registry, &mut state).is_empty());
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(DAY_MINUTES).unwrap(),
    ));
    assert_eq!(apply_due_rival_operations(&registry, &mut state).len(), 1);
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
