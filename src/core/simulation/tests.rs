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
use crate::reputation::reputation_system::{apply_reputation_delta, resolve_score};
use crate::reputation::{AudienceKind, ReputationDimension};
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

struct SequenceRng {
    draws: std::collections::VecDeque<u64>,
}

impl SequenceRng {
    fn new(draws: impl IntoIterator<Item = u64>) -> Self {
        Self {
            draws: draws.into_iter().collect(),
        }
    }
}

impl rand_core::RngCore for SequenceRng {
    fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() & u64::from(u32::MAX))
            .expect("masked deterministic draw must fit u32")
    }

    fn next_u64(&mut self) -> u64 {
        self.draws
            .pop_front()
            .expect("deterministic RNG fixture exhausted")
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rand_core::impls::fill_bytes_via_next(self, dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

fn test_rating(value: u8) -> Rating {
    Rating::try_new(value).expect("simulation test rating must be valid")
}

#[test]
fn draw_index_maps_the_accepted_domain_in_stable_modulo_order() {
    let mut rng = SequenceRng::new([0, 1, 2, 3, 4, 5]);
    let actual = (0..6)
        .map(|_| draw_index(&mut rng, 3).expect("nonempty choice set should draw"))
        .collect::<Vec<_>>();
    assert_eq!(actual, vec![0, 1, 2, 0, 1, 2]);
}

#[test]
fn draw_index_accepts_high_power_of_two_domain_values_without_redraw() {
    // A two-choice range uses a rejection zone of u64::MAX - 1. This high odd value is still
    // inside the accepted domain and therefore maps to choice one without consuming the next
    // RNG value. It directly constrains the rejection-zone subtraction instead of relying on
    // unrelated simulation behavior to notice arithmetic drift.
    let mut rng = SequenceRng::new([u64::MAX - 2, 0]);
    assert_eq!(
        draw_index(&mut rng, 2).expect("high two-choice draw should be accepted"),
        1
    );
    assert_eq!(
        rng.next_u64(),
        0,
        "accepted power-of-two draw must not consume the following RNG value"
    );
}

#[test]
fn draw_index_rejects_the_exclusive_boundary_before_mapping() {
    // For three choices, u64::MAX is the first rejected value because the accepted domain
    // contains exactly u64::MAX values, which is divisible by three. A non-strict comparison
    // would incorrectly map that boundary to choice zero instead of consuming the next draw.
    let mut rng = SequenceRng::new([u64::MAX, 2]);
    assert_eq!(
        draw_index(&mut rng, 3).expect("second deterministic draw should be accepted"),
        2
    );
}

#[test]
fn draw_index_rejects_empty_choice_set_without_consuming_rng() {
    let mut rng = SequenceRng::new([7]);
    assert_eq!(
        draw_index(&mut rng, 0),
        Err(RandomDecisionError::EmptyChoiceSet)
    );
    assert_eq!(
        rng.next_u64(),
        7,
        "rejected draw must not consume RNG state"
    );
}

#[test]
fn tick_outcome_surfaces_reputation_only_decay_mutation() {
    let registry = build_registry();
    let mut state = AppState::new(0xDEC4_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Quiet Reputation Fixture".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("fixture organization should validate");
    let baseline = registry.reputation().baseline();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        10,
    )
    .expect("fixture reputation movement should apply");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(crate::core::time::DAY_MINUTES - 1)
            .expect("one campaign day minus one minute must fit SimDuration"),
    ));

    let outcome = run_tick(&registry, &mut state).expect("day-boundary tick should succeed");

    assert_eq!(outcome.now.as_minutes(), crate::core::time::DAY_MINUTES);
    assert_eq!(outcome.reputation_changes, 1);
    assert_eq!(
        resolve_score(
            &registry,
            state.reputation(),
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear,
        ),
        baseline + 10 - registry.reputation().daily_decay_step(),
    );
    assert!(outcome.payrolls.is_empty());
    assert!(outcome.business_cycles.is_empty());
    assert!(outcome.enterprise_cycles.is_empty());
    assert!(outcome.recruitment_attempts.is_empty());
    assert!(outcome.autonomous_enterprises.is_empty());
    assert!(outcome.executive_brief.is_none());
    validate_state(&state).expect("reputation-only tick should remain structurally valid");
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
