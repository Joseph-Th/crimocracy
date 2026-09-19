use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::legal::JurisdictionDraft;
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationConstraint, OperationContingency, OperationDraft, OperationKind,
    OperationObjective, OperationStatus, RoleKind,
};
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn test_rating(value: u8) -> Rating {
    Rating::try_new(value).expect("simulation test rating must be valid")
}

#[test]
fn terminal_clock_rejects_tick_without_mutating_state() {
    let registry = build_registry();
    let mut state = AppState::new(0xC10C_EA11);
    state.set_now_for_test(SimTime::from_minutes(u64::MAX));
    let before = bincode::serialize(&state).expect("terminal state should serialize");

    assert_eq!(
        run_tick(&registry, &mut state),
        Err(TickError::ClockExhausted {
            now: SimTime::from_minutes(u64::MAX),
        })
    );
    assert_eq!(
        bincode::serialize(&state).expect("rejected terminal state should still serialize"),
        before,
        "clock exhaustion must reject before any tick phase mutates state"
    );
}

#[test]
fn domain_random_streams_do_not_cross_contaminate_unrelated_simulation_work() {
    let mut baseline = AppState::new(0x1933_0814);
    let mut operation_heavy = baseline.clone();

    for _ in 0..64 {
        draw_signed_variance(operation_heavy.operation_rng_mut(), 12);
    }

    for _ in 0..32 {
        assert_eq!(
            draw_basis_point_variance(baseline.business_rng_mut(), 2_500),
            draw_basis_point_variance(operation_heavy.business_rng_mut(), 2_500)
        );
        assert_eq!(
            draw_basis_point_variance(baseline.enterprise_rng_mut(), 2_500),
            draw_basis_point_variance(operation_heavy.enterprise_rng_mut(), 2_500)
        );
        assert_eq!(
            draw_signed_variance(baseline.investigation_rng_mut(), 12),
            draw_signed_variance(operation_heavy.investigation_rng_mut(), 12)
        );
    }
}

#[test]
fn same_minute_police_arrival_blocks_back_to_back_participant_start() {
    let registry = build_registry();
    let mut state = AppState::new(0xB0A0_DA7A);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Boundary Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    designate_player_organization(&mut state, crew)
        .expect("boundary crew should be the player organization");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Boundary Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police organization should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Boundary Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(50),
                    commercial_activity: test_rating(50),
                    illicit_demand: test_rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(100),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: test_rating(80),
        },
    )
    .expect("jurisdiction should validate")
    .commit(&mut state)
    .expect("jurisdiction should commit");

    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Boundary Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Management, test_rating(75)),
                (CapabilityKind::Intimidation, test_rating(75)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader should validate");
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Boundary Store".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("target business should validate");

    let first = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Boundary collection".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: vec![OperationConstraint::CompleteBy(SimTime::from_minutes(4))],
            contingencies: vec![OperationContingency::RequestDecisionOnPoliceArrival],
            scheduled_for: SimTime::ZERO,
        },
    )
    .expect("first boundary operation should validate")
    .commit(&mut state)
    .expect("first boundary operation should commit");
    let follow_up = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Boundary follow-up".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::from_minutes(4),
        },
    )
    .expect("exact back-to-back follow-up should authorize")
    .commit(&mut state)
    .expect("exact back-to-back follow-up should commit");

    let first_tick = run_test_tick(&registry, &mut state);
    assert_eq!(first_tick.now, SimTime::from_minutes(1));
    assert_eq!(first_tick.started_operations, vec![first]);
    let first_record = state
        .operations()
        .get_operation(first)
        .expect("first operation should persist");
    assert_eq!(
        first_record.resolution_due_at(),
        Some(SimTime::from_minutes(4))
    );
    let response = first_record
        .police_response()
        .expect("high ambient police presence should dispatch a response");
    assert_eq!(
        state
            .legal()
            .get_police_response(response)
            .expect("response should persist")
            .arrival_due_at(),
        SimTime::from_minutes(4)
    );

    for expected_minute in 2..=3 {
        let tick = run_test_tick(&registry, &mut state);
        assert_eq!(tick.now, SimTime::from_minutes(expected_minute));
        assert!(tick.started_operations.is_empty());
        assert!(tick.arrived_police_responses.is_empty());
    }

    let boundary = run_test_tick(&registry, &mut state);
    assert_eq!(boundary.now, SimTime::from_minutes(4));
    assert_eq!(boundary.arrived_police_responses, vec![response]);
    assert_eq!(boundary.decision_requests.len(), 1);
    assert!(
        boundary.started_operations.is_empty(),
        "the unresolved arrival decision must retain the leader before the back-to-back follow-up begins"
    );
    assert_eq!(
        state
            .operations()
            .get_operation(first)
            .expect("first operation should persist")
            .status(),
        OperationStatus::AwaitingDecision
    );
    assert_eq!(
        state
            .operations()
            .get_operation(follow_up)
            .expect("deferred follow-up should persist")
            .status(),
        OperationStatus::Authorized
    );
    validate_state(&state).expect("same-minute arrival boundary state should validate");
    validate_invariants(&state);
}
