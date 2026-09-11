//! Focused tests for operation authorization, transitions, aborts, and indexes.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::id::MandateId;
use crate::core::invariants::{
    StateValidationError, validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::time::SimTime;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::operation_abort::validate_authority_abort_operation;
use crate::operations::operation_execution::{
    OperationResolutionRandomness, decide_operation_resolution, validate_operation_resolution_plan,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationApproach, OperationDraft, OperationKind,
    OperationObjective, OperationObjectiveBlocker, OperationObjectiveKind, RoleKind,
};
use crate::reports::ReportKind;
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization, validate_reassign_character, validate_transfer_business_ownership,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn make_test_operation_state() -> (Registry, AppState, OrganizationId, CharacterId, EntityRef) {
    let registry = build_registry();
    let mut state = AppState::new(19);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police organization fixture should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Test Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood fixture should validate");
    crate::legal::jurisdiction_system::validate_set_jurisdiction(
        &state,
        crate::legal::JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(80).expect("fixture priority should validate"),
        },
    )
    .expect("precinct jurisdiction fixture should validate")
    .commit(&mut state)
    .expect("precinct jurisdiction fixture should commit");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Test Business".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business fixture should validate");
    (
        registry,
        state,
        organization,
        leader,
        EntityRef::Business(business),
    )
}

fn make_test_draft(
    organization: OrganizationId,
    leader: CharacterId,
    target: EntityRef,
) -> OperationDraft {
    OperationDraft {
        title: "Test intimidation racket".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash { target },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    }
}

#[test]
fn operation_rejects_character_objective_target_as_crew_participant() {
    let (registry, state, organization, leader, _) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Self-surveillance".to_owned(),
        kind: OperationKind::Surveillance,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::GatherInformation {
            target: EntityRef::Character(leader),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("a character cannot carry out an operation targeting themself");
    assert_eq!(
        error,
        OperationError::ObjectiveTargetIsParticipant { character: leader }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_state_against_registry(&registry, &state)
        .expect("rejected self-targeting must preserve canonical state");
    validate_invariants(&state);
}

#[test]
fn operation_rejects_non_criminal_responsible_organization() {
    let (registry, mut state, _, _, target) = make_test_operation_state();
    let authority = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Operation-ineligible authority".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("authority fixture should validate");
    let officer = insert_character(
        &mut state,
        CharacterDraft {
            name: "Operation-ineligible officer".to_owned(),
            organization: Some(authority),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("officer fixture should validate");
    let draft = OperationDraft {
        title: "Authority surveillance".to_owned(),
        kind: OperationKind::Surveillance,
        responsible_organization: authority,
        leader: officer,
        objective: OperationObjective::GatherInformation { target },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Surveillance, officer)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("criminal operations require a criminal sponsoring organization");
    assert_eq!(error, OperationError::InvalidOrganizationKind(authority));
    assert_eq!(state.operations().operations().count(), 0);
    validate_state_against_registry(&registry, &state)
        .expect("rejected non-criminal sponsorship must preserve canonical state");
    validate_invariants(&state);
}

#[test]
fn due_operation_aborts_before_start_when_objective_became_unavailable() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let EntityRef::Business(business) = target else {
        panic!("fixture target should be a business");
    };
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(1);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("foreign business should be actionable at authorization")
        .commit(&mut state)
        .expect("future operation should commit");

    validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Organization(organization),
    )
    .expect("authorized work must not freeze target ownership")
    .commit(&mut state)
    .expect("business acquisition should commit before the operation begins");

    let outcome = crate::core::simulation::run_tick(&registry, &mut state);
    assert!(outcome.started_operations.is_empty());
    assert!(outcome.resolved_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("cancelled operation should remain as history");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert!(record.started_at().is_none());
    assert!(record.resolution_due_at().is_none());
    let abort = record
        .abort_record()
        .expect("objective loss should persist explicit abort causality");
    assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
    assert_eq!(
        abort.cause(),
        OperationAbortCause::ObjectiveUnavailable(
            OperationObjectiveBlocker::TargetBusinessOwnershipMismatch,
        )
    );
    let artifacts = abort
        .artifacts()
        .expect("objective-loss cancellation should be visible to leadership");
    assert!(
        state
            .intelligence()
            .get_information(artifacts.information())
            .expect("objective-loss information should persist")
            .summary()
            .contains("had come under the sponsoring organization's ownership")
    );
    assert!(
        state
            .operations()
            .find_active_operation_booking(leader)
            .is_none()
    );
    validate_state_against_registry(&registry, &state)
        .expect("objective-loss abort should remain registry-valid");
    validate_invariants(&state);

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("objective-loss abort should save"),
    )
    .expect("objective-loss abort should restore under the current schema");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.abort_record())
            .map(|abort| abort.cause()),
        Some(OperationAbortCause::ObjectiveUnavailable(
            OperationObjectiveBlocker::TargetBusinessOwnershipMismatch,
        ))
    );
}

fn insert_test_operation_leader(
    state: &mut AppState,
    organization: OrganizationId,
    name: &str,
) -> CharacterId {
    insert_character(
        state,
        CharacterDraft {
            name: name.to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("additional operation leader should validate")
}

#[test]
fn due_authorized_operations_preserve_start_chronology_before_id_order() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let second_leader =
        insert_test_operation_leader(&mut state, organization, "Earlier Scheduled Leader");

    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = SimTime::from_minutes(20);
    let later_due_lower_id = validate_authorize_operation(&registry, &state, later)
        .expect("later lower-ID operation should validate")
        .commit(&mut state)
        .expect("later lower-ID operation should commit");
    let mut earlier = make_test_draft(organization, second_leader, target);
    earlier.scheduled_for = SimTime::from_minutes(10);
    let earlier_due_higher_id = validate_authorize_operation(&registry, &state, earlier)
        .expect("earlier higher-ID operation should validate")
        .commit(&mut state)
        .expect("earlier higher-ID operation should commit");
    assert!(later_due_lower_id < earlier_due_higher_id);

    state.advance_clock(SimDuration::from_minutes(20));
    assert_eq!(
        find_due_authorized_operations(&state),
        vec![earlier_due_higher_id, later_due_lower_id],
        "an older scheduled start must run before a later-due lower operation ID"
    );
    validate_invariants(&state);
}

#[test]
fn gambling_event_requires_sponsor_control_and_a_real_gambling_venue() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let EntityRef::Business(foreign_business) = target else {
        panic!("unexpected fixture target {target:?}");
    };
    let scheduled_for = state.now();
    let draft_for = |business| OperationDraft {
        title: "Back-room card night".to_owned(),
        kind: OperationKind::GamblingEvent,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash {
            target: EntityRef::Business(business),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for,
    };

    assert_eq!(
        validate_authorize_operation(&registry, &state, draft_for(foreign_business))
            .expect_err("a gambling event cannot claim house proceeds from a foreign venue"),
        OperationError::TargetBusinessNotSponsorOwned {
            business: foreign_business,
        }
    );

    let neighborhood = state
        .world()
        .get_business(foreign_business)
        .expect("fixture business should exist")
        .neighborhood();
    let inadequate = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Bare Cash Office".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("owned non-venue business should persist");
    assert_eq!(
        validate_authorize_operation(&registry, &state, draft_for(inadequate))
            .expect_err("cash handling and customers without meeting space are not a venue"),
        OperationError::TargetBusinessMissingFunction {
            business: inadequate,
            function: BusinessFunction::MeetingSpace,
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn gambling_event_fails_if_the_sponsor_loses_the_venue_before_payout() {
    let (registry, mut state, organization, _leader, target) = make_test_operation_state();
    let EntityRef::Business(fixture_business) = target else {
        panic!("unexpected fixture target {target:?}");
    };
    let neighborhood = state
        .world()
        .get_business(fixture_business)
        .expect("fixture business should exist")
        .neighborhood();
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Gambling Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                crate::world::CapabilityKind::Management,
                Rating::try_new(100).expect("fixture rating should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("gambling manager should validate");
    let venue = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Transient Gambling Venue".to_owned(),
            kind: BusinessKind::Nightclub,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("gambling venue should validate");
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Venue control regression".to_owned(),
            kind: OperationKind::GamblingEvent,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(venue),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now(),
        },
    )
    .expect("controlled gambling venue should authorize")
    .commit(&mut state)
    .expect("controlled gambling venue should commit");

    let tick = crate::core::simulation::run_tick(&registry, &mut state);
    assert_eq!(tick.started_operations, vec![operation]);
    validate_transfer_business_ownership(&state, venue, BusinessOwner::Independent)
        .expect("an operation target does not freeze world ownership")
        .commit(&mut state)
        .expect("venue transfer should commit");
    let due_at = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution_due_at())
        .expect("started gambling event should have a resolution time");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture duration must fit SimDuration"),
    ));
    let variance = i8::try_from(
        registry
            .get_operation(OperationKind::GamblingEvent)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(variance, 0),
    )
    .expect("due gambling event should decide despite venue ownership drift");
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("venue-loss failure should validate")
        .commit(&mut state)
        .expect("venue-loss failure should commit");

    let resolution = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("failed gambling event should persist its resolution");
    assert_eq!(
        resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    );
    assert!(resolution.cash_proceeds().is_none());
    let after_action = state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("failed gambling event should report its cause");
    assert!(
        after_action
            .summary()
            .contains("left the sponsoring organization's control")
    );
    validate_state_against_registry(&registry, &state)
        .expect("venue-loss operation should remain registry-valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("venue-loss operation should save"),
    )
    .expect("venue-loss operation should restore");
    validate_invariants(&restored);
}

#[test]
fn gambling_event_allows_an_organization_owned_venue() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let EntityRef::Business(target_business) = target else {
        panic!("unexpected fixture target {target:?}");
    };
    let neighborhood = state
        .world()
        .get_business(target_business)
        .expect("fixture business should exist")
        .neighborhood();
    let venue = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Organization Gambling Venue".to_owned(),
            kind: BusinessKind::Nightclub,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("owned gambling venue should validate");

    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Back-room card night".to_owned(),
            kind: OperationKind::GamblingEvent,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(venue),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now(),
        },
    )
    .expect("a gambling event may use the sponsor's own venue")
    .commit(&mut state)
    .expect("owned-venue gambling event should commit");

    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("gambling operation should persist")
            .objective(),
        &OperationObjective::ObtainCash {
            target: EntityRef::Business(venue),
        }
    );
    let record = state
        .operations()
        .get_operation(operation)
        .expect("gambling operation should persist");
    assert_eq!(
        crate::operations::operation_objective::resolve_objective_blocker(&state, record),
        None,
        "owning the gambling venue must not become an execution-time objective blocker"
    );
    validate_state(&state).expect("owned-venue gambling operation must remain valid");
    validate_invariants(&state);
}

#[test]
fn due_in_progress_operations_preserve_resolution_chronology_before_id_order() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let second_leader =
        insert_test_operation_leader(&mut state, organization, "Earlier Resolution Leader");

    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = SimTime::from_minutes(10);
    let later_due_lower_id = validate_authorize_operation(&registry, &state, later)
        .expect("later lower-ID operation should validate")
        .commit(&mut state)
        .expect("later lower-ID operation should commit");
    let earlier_due_higher_id = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, second_leader, target),
    )
    .expect("earlier higher-ID operation should validate")
    .commit(&mut state)
    .expect("earlier higher-ID operation should commit");
    assert!(later_due_lower_id < earlier_due_higher_id);

    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(
        &registry,
        &mut state,
        earlier_due_higher_id,
        OperationTransition::Begin,
    )
    .expect("higher-ID operation should begin first");
    state.advance_clock(SimDuration::from_minutes(9));
    apply_transition(
        &registry,
        &mut state,
        later_due_lower_id,
        OperationTransition::Begin,
    )
    .expect("lower-ID operation should begin later");

    let earlier_due_at = state
        .operations()
        .get_operation(earlier_due_higher_id)
        .and_then(|record| record.resolution_due_at())
        .expect("earlier operation should have a resolution time");
    let later_due_at = state
        .operations()
        .get_operation(later_due_lower_id)
        .and_then(|record| record.resolution_due_at())
        .expect("later operation should have a resolution time");
    assert!(earlier_due_at < later_due_at);
    let catch_up_minutes = u32::try_from(later_due_at.as_minutes() - state.now().as_minutes())
        .expect("fixture catch-up duration must fit SimDuration");
    state.advance_clock(SimDuration::from_minutes(catch_up_minutes));

    assert_eq!(
        crate::operations::operation_execution::find_due_in_progress_operations(&state),
        vec![earlier_due_higher_id, later_due_lower_id],
        "an earlier resolution must consume RNG before a later-due lower operation ID"
    );
    validate_invariants(&state);
}

#[test]
fn authorization_rejects_a_deadline_that_cannot_accommodate_a_next_tick_begin() {
    // A current-minute plan begins on the next canonical tick. Burglary's entry milestone is
    // ten minutes after that begin, so minute 11 leaves no usable execution time before the
    // milestone while minute 12 leaves one minute and is admissible.
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let crew = insert_character(
        &mut state,
        CharacterDraft {
            name: "Crew Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("crew fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    // Burglary carries an authored 10-minute entry offset and requires an entry specialist.
    draft.kind = OperationKind::Burglary;
    draft.objective = OperationObjective::AcquireProperty { target };
    draft.approach = OperationApproach::Covert;
    draft.roles = BTreeMap::from([
        (RoleKind::Coordinator, leader),
        (RoleKind::EntrySpecialist, crew),
    ]);
    draft.constraints = vec![crate::operations::OperationConstraint::CompleteBy(
        SimTime::from_minutes(11),
    )];
    let error = validate_authorize_operation(&registry, &state, draft.clone())
        .expect_err("a deadline that only fits a begin at the schedule minute must be rejected");
    assert_eq!(error, OperationError::DeadlineLeavesNoExecutionWindow);

    draft.constraints = vec![crate::operations::OperationConstraint::CompleteBy(
        SimTime::from_minutes(12),
    )];
    validate_authorize_operation(&registry, &state, draft)
        .expect("one spare minute above entry offset admits the guaranteed next-tick begin");
}

#[test]
fn authorization_rejects_future_operation_whose_duration_exceeds_simulation_clock() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let duration = u64::from(
        registry
            .get_operation(OperationKind::Intimidation)
            .execution()
            .duration()
            .as_minutes(),
    );
    assert!(duration > 0);
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(u64::MAX - duration + 1);

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("an operation whose authored duration crosses the clock horizon must reject");
    assert_eq!(error, OperationError::SimulationTimeOverflow);
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_take_targets_owned_by_the_sponsoring_organization() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut state = state;
    let EntityRef::Business(id) = target else {
        panic!("unexpected fixture target {target:?}");
    };
    let neighborhood = state
        .world()
        .get_business(id)
        .expect("fixture business should exist")
        .neighborhood();
    let front = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Family Front".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("owned front fixture should validate");

    // A take against the organization's own premises would enrich it out of itself;
    // authorization rejects the self-target before anything is created.
    let error = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, EntityRef::Business(front)),
    )
    .expect_err("self-targeted operation must be rejected");
    assert!(matches!(error, OperationError::SelfTargetedBusiness { .. }));
    validate_invariants(&state);
}

#[test]
fn delayed_begin_rejects_resolution_beyond_clock_horizon_without_starting_operation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("ordinary operation should authorize")
    .commit(&mut state)
    .expect("ordinary operation should persist authorized");
    let duration = registry
        .get_operation(OperationKind::Intimidation)
        .execution()
        .duration();
    state.set_now_for_test(SimTime::from_minutes(
        u64::MAX - u64::from(duration.as_minutes()) + 1,
    ));

    let error = match validate_begin_operation(&registry, &state, operation) {
        Ok(_) => panic!("late begin must reject when its resolution cannot be represented"),
        Err(error) => error,
    };
    assert_eq!(error, OperationError::SimulationTimeOverflow);
    let record = state
        .operations()
        .get_operation(operation)
        .expect("rejected begin must preserve authorized operation");
    assert_eq!(record.status(), OperationStatus::Authorized);
    assert!(record.started_at().is_none());
    assert!(record.resolution_due_at().is_none());
}

#[test]
fn invalid_terminal_transition_leaves_operation_unchanged() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("validated operation should remain current");

    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("authorized operation should begin");
    apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
        .expect("in-progress operation should abort");
    let before = state
        .operations
        .get_operation(operation)
        .expect("operation should exist")
        .version();

    let error = apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
        .expect_err("terminal operation must reject further transitions");
    assert_eq!(
        error,
        OperationError::InvalidTransition {
            status: OperationStatus::Aborted,
            transition: OperationTransition::Abort,
        }
    );
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation should still exist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(record.version(), before);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_foreign_crew_members_before_authorization() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut state = state;
    let foreign_organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Foreign Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("foreign organization fixture should validate");
    let foreign_member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Foreign Member".to_owned(),
            organization: Some(foreign_organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("foreign member fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    draft.roles.insert(RoleKind::Coordinator, foreign_member);

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("foreign crew members must not be authorized");
    assert_eq!(
        error,
        OperationError::ForeignParticipant {
            character: foreign_member,
            expected: organization,
            actual: Some(foreign_organization),
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_one_character_filling_multiple_roles() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Impossible double assignment".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash { target },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Muscle, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("one character must not fill multiple simultaneous roles");
    assert_eq!(
        error,
        OperationError::DuplicateRoleParticipant {
            character: leader,
            first_role: RoleKind::Muscle,
            second_role: RoleKind::Coordinator,
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_administrative_records_as_field_objective_targets() {
    let (registry, state, organization, leader, _) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Invalid mandate coercion".to_owned(),
        kind: OperationKind::Intimidation,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::ObtainCash {
            target: EntityRef::Mandate(MandateId::from_raw(1)),
        },
        approach: OperationApproach::Intimidating,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("field operations must not target internal mandate records");
    assert_eq!(
        error,
        OperationError::InvalidObjectiveTarget {
            objective: OperationObjectiveKind::ObtainCash,
            target: EntityRef::Mandate(MandateId::from_raw(1)),
        }
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_overlapping_participant_assignment_until_prior_operation_is_terminal() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");

    let error = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect_err("one person must not be scheduled for overlapping operations");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first,
        }
    );
    assert_eq!(state.operations().operations().count(), 1);

    validate_authority_abort_operation(&state, first)
        .expect("the first operation should be cancellable before start")
        .commit(&mut state)
        .expect("the cancellation should commit");
    validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("terminal operations must release their participants");
    validate_invariants(&state);
}

#[test]
fn operation_allows_non_overlapping_future_assignment() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");
    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = SimTime::from_minutes(1)
        + registry
            .get_operation(OperationKind::Intimidation)
            .execution()
            .duration();

    validate_authorize_operation(&registry, &state, later)
        .expect("a future operation at the prior operation's real end should validate");
    validate_invariants(&state);
}

#[test]
fn delayed_earlier_operation_keeps_priority_over_later_authorized_reservation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");
    let duration = registry
        .get_operation(OperationKind::Intimidation)
        .execution()
        .duration();
    let first_projected_end = SimTime::from_minutes(1) + duration;
    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = first_projected_end;
    let later = validate_authorize_operation(&registry, &state, later)
        .expect("originally back-to-back later operation should validate")
        .commit(&mut state)
        .expect("later reservation should commit");

    state.advance_clock(SimDuration::from_minutes(2));
    validate_begin_operation(&registry, &state, first)
        .expect("a later reservation must not leapfrog overdue earlier work")
        .commit(&mut state)
        .expect("delayed earlier operation should begin");
    validate_state_against_registry(&registry, &state)
        .expect("canonical delay-created overlap must remain restore-valid");

    let later_due_minutes = first_projected_end.as_minutes() - state.now().as_minutes();
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(later_due_minutes).expect("fixture delay must fit SimDuration"),
    ));
    let error = validate_begin_operation(&registry, &state, later)
        .err()
        .expect("later reservation must wait for the earlier live commitment");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first,
        }
    );
    assert_eq!(
        state
            .operations()
            .get_operation(later)
            .expect("later operation should persist")
            .status(),
        OperationStatus::Authorized
    );
    validate_state_against_registry(&registry, &state)
        .expect("queued later reservation must remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn restore_rejects_overlapping_active_operation_bookings() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");
    let duration = registry
        .get_operation(OperationKind::Intimidation)
        .execution()
        .duration();
    let first_end = SimTime::from_minutes(1) + duration;
    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = first_end;
    let second = validate_authorize_operation(&registry, &state, later)
        .expect("back-to-back operation should validate")
        .commit(&mut state)
        .expect("back-to-back operation should commit");
    validate_state_against_registry(&registry, &state)
        .expect("canonical back-to-back bookings should remain registry-valid");

    let original = state
        .operations()
        .get_operation(second)
        .expect("second operation should persist")
        .clone();
    let mut replacement = original.clone();
    replacement.command.scheduled_for = SimTime::from_minutes(u64::from(duration.as_minutes()));
    let original_bytes = bincode::serialize(&original).expect("operation record should serialize");
    let replacement_bytes =
        bincode::serialize(&replacement).expect("replacement operation should serialize");
    assert_eq!(
        replacement_bytes.len(),
        original_bytes.len(),
        "fixed-width schedule corruption must preserve the record wire size"
    );
    let envelope = build_save(&registry, &state).expect("canonical fixture should save");
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "the second operation record must occur exactly once in the save"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    let corrupted: SaveEnvelope =
        bincode::deserialize(&envelope_bytes).expect("same-layout corruption should decode");

    let error = restore_save(&registry, corrupted)
        .expect_err("restore must reject an overlapping active booking");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::ActiveOperationParticipantOverlap {
            participant: leader,
            first,
            second,
        }),
        "restore must reject an overlapping active booking that canonical authorization cannot create"
    );
}

#[test]
fn current_minute_authorization_reserves_crew_through_its_real_next_tick_window() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate")
    .commit(&mut state)
    .expect("first operation should commit");
    let duration = registry
        .get_operation(OperationKind::Intimidation)
        .execution()
        .duration();
    let mut overlapping = make_test_draft(organization, leader, target);
    overlapping.scheduled_for = SimTime::ZERO + duration;

    let error = validate_authorize_operation(&registry, &state, overlapping)
        .expect_err("the first operation actually starts next tick and still occupies this minute");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first,
        }
    );
    validate_invariants(&state);
}

#[test]
fn deadline_constrained_authorization_releases_crew_at_the_deadline() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut first = make_test_draft(organization, leader, target);
    first.scheduled_for = SimTime::from_minutes(10);
    first.constraints = vec![crate::operations::OperationConstraint::CompleteBy(
        SimTime::from_minutes(20),
    )];
    validate_authorize_operation(&registry, &state, first)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");
    let mut later = make_test_draft(organization, leader, target);
    later.scheduled_for = SimTime::from_minutes(20);

    validate_authorize_operation(&registry, &state, later)
        .expect("the binding deadline should end the earlier crew reservation");
    validate_invariants(&state);
}

#[test]
fn operation_commit_rechecks_participant_availability_after_validation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let first = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("first operation should validate");
    let second = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("the second plan can validate against the same initial snapshot");
    let first_id = first
        .commit(&mut state)
        .expect("the first plan should commit");

    let error = second
        .commit(&mut state)
        .expect_err("commit must recheck a participant reservation created after validation");
    assert_eq!(
        error,
        OperationError::ParticipantBusy {
            character: leader,
            operation: first_id,
        }
    );
    assert_eq!(state.operations().operations().count(), 1);
    validate_invariants(&state);
}

#[test]
fn property_acquisition_requires_a_business_target() {
    let (registry, state, organization, leader, _) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Invalid property seizure".to_owned(),
        kind: OperationKind::Burglary,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::AcquireProperty {
            target: EntityRef::Character(leader),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::EntrySpecialist, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::ZERO,
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("property acquisition must identify a business target");
    assert_eq!(
        error,
        OperationError::InvalidPropertyTarget(EntityRef::Character(leader))
    );
}

#[test]
fn expired_planning_information_is_not_reported_as_covered() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: target,
            observed_at: SimTime::ZERO,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "Expired personnel observation".to_owned(),
        },
    )
    .expect("information fixture should validate")
    .commit(&mut state)
    .expect("information fixture should commit");
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(10_081);
    draft.intelligence.insert(information);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("operation with stale but structurally valid information should authorize")
        .commit(&mut state)
        .expect("operation should commit");

    let (quality, adjustment, covered, relevant) =
        crate::operations::operation_execution::resolve_intelligence_factors(
            &registry, &state, operation,
        );
    assert_eq!(quality.value(), 0);
    assert_eq!(adjustment, 0);
    assert_eq!(covered, 0);
    assert_eq!(relevant, 3);
}

#[test]
fn pre_start_cancellation_records_cause_without_fabricating_execution_artifacts() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate")
        .commit(&mut state)
        .expect("future operation should commit");

    // A pre-start cancellation creates no information, report, or history record. Its
    // transaction must therefore remain usable even when an unrelated optional ID stream is
    // exhausted.
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Information, u32::MAX);

    validate_authority_abort_operation(&state, operation)
        .expect("authorized operation should accept a leadership cancellation")
        .commit(&mut state)
        .expect("validated pre-start cancellation should commit");

    let record = state
        .operations()
        .get_operation(operation)
        .expect("cancelled operation should persist");
    let abort = record
        .abort_record()
        .expect("cancelled operation should persist its abort record");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(abort.aborted_at(), SimTime::ZERO);
    assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
    assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
    assert!(abort.artifacts().is_none());
    assert!(record.started_at().is_none());
    assert!(record.resolution_due_at().is_none());
    assert_eq!(state.reports().reports_for(organization).count(), 0);
    validate_state(&state).expect("pre-start cancellation should be structurally valid");
    validate_invariants(&state);
}

#[test]
fn in_progress_authority_abort_records_causal_artifacts_and_survives_save_round_trip() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("operation fixture should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(5));

    validate_authority_abort_operation(&state, operation)
        .expect("in-progress operation should accept a leadership abort")
        .commit(&mut state)
        .expect("validated in-progress abort should commit");

    let record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should persist");
    let abort = record
        .abort_record()
        .expect("started abort should persist its provenance");
    assert_eq!(abort.aborted_at(), SimTime::from_minutes(6));
    assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
    assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
    assert!(record.resolution().is_none());
    let artifacts = abort
        .artifacts()
        .expect("started abort should produce after-action artifacts");
    let information = state
        .intelligence()
        .get_information(artifacts.information())
        .expect("abort information should persist");
    assert_eq!(
        information.holder(),
        KnowledgeHolder::Organization(organization)
    );
    assert_eq!(information.topic(), InformationTopic::OperationalOutcome);
    assert_eq!(information.subject(), EntityRef::Operation(operation));
    assert!(information.summary().contains("aborted by leadership"));
    let report = state
        .reports()
        .get_report(artifacts.report())
        .expect("abort report should persist");
    assert_eq!(report.kind(), ReportKind::AfterAction);
    assert_eq!(report.recipient(), organization);
    assert_eq!(report.entries().len(), 1);
    assert_eq!(report.entries()[0].summary, information.summary());
    let history = state
        .history()
        .get_event(artifacts.history_event())
        .expect("abort history should persist");
    assert_eq!(history.summary(), information.summary());

    let envelope = build_save(&registry, &state).expect("aborted operation should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("aborted operation should restore");
    let restored_abort = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.abort_record())
        .expect("restored abort provenance should persist");
    assert_eq!(restored_abort, abort);
    validate_state(&restored).expect("restored abort state should validate");
    validate_invariants(&restored);
}

#[test]
fn abort_token_rejects_time_staleness_without_partial_mutation() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let operation = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("operation fixture should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin");
    let abort =
        validate_authority_abort_operation(&state, operation).expect("fresh abort should validate");
    state.advance_clock(crate::core::time::SimDuration::ONE_MINUTE);

    let error = abort
        .commit(&mut state)
        .expect_err("abort token must expire when simulation time advances");
    assert_eq!(
        error,
        OperationError::StaleAbortTime {
            operation,
            expected: SimTime::from_minutes(1),
            found: SimTime::from_minutes(2),
        }
    );
    let record = state
        .operations()
        .get_operation(operation)
        .expect("stale abort must leave operation present");
    assert_eq!(record.status(), OperationStatus::InProgress);
    assert!(record.abort_record().is_none());
    assert_eq!(state.reports().reports_for(organization).count(), 0);
    assert_eq!(state.history().events().count(), 0);
    validate_state(&state).expect("stale abort rejection must leave valid state");
    validate_invariants(&state);
}

#[test]
fn missing_required_role_is_rejected_before_id_allocation() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.roles.clear();

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("missing required role must fail validation");
    assert_eq!(
        error,
        OperationError::MissingRequiredRole(RoleKind::Coordinator)
    );
    assert_eq!(
        state
            .operations
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn stock_registry_rejects_incoherent_surveillance_approach() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let draft = OperationDraft {
        title: "Overt surveillance".to_owned(),
        kind: OperationKind::Surveillance,
        responsible_organization: organization,
        leader,
        objective: OperationObjective::GatherInformation { target },
        approach: OperationApproach::Violent,
        roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: state.now(),
    };

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("violent surveillance is a different criminal act, not an observation plan");
    assert_eq!(error, OperationError::UnsupportedApproach);
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_rejects_irrelevant_specialist_role() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Safe Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("specialist fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    draft.roles.insert(RoleKind::SafeSpecialist, specialist);

    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("a safe specialist has no execution function in intimidation");
    assert_eq!(
        error,
        OperationError::UnsupportedRole(RoleKind::SafeSpecialist)
    );
    assert_eq!(state.operations().operations().count(), 0);
    validate_invariants(&state);
}

#[test]
fn operation_cannot_begin_before_scheduled_time() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate")
        .commit(&mut state)
        .expect("validated operation should remain current");
    let version = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .version();

    let error = apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect_err("operation must not begin early");
    assert_eq!(
        error,
        OperationError::StartBeforeEarliestStart {
            operation,
            earliest_start: SimTime::from_minutes(30),
        }
    );
    let record = state
        .operations()
        .get_operation(operation)
        .expect("operation should still exist");
    assert_eq!(record.status(), OperationStatus::Authorized);
    assert_eq!(record.version(), version);
    validate_invariants(&state);
}

#[test]
fn missed_completion_deadline_aborts_before_start_with_visible_provenance() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(30);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(40),
        ));
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");

    state.advance_clock(crate::core::time::SimDuration::from_minutes(41));
    let outcome = crate::core::simulation::run_tick(&registry, &mut state);

    assert!(outcome.started_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-missed operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    let abort = record
        .abort_record()
        .expect("deadline miss should persist an abort record");
    assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
    assert_eq!(abort.cause(), OperationAbortCause::DeadlineMissed);
    let artifacts = abort
        .artifacts()
        .expect("deadline miss should produce visible provenance");
    let information = state
        .intelligence()
        .get_information(artifacts.information())
        .expect("deadline information should persist");
    assert!(
        information
            .summary()
            .contains("missed its completion deadline")
    );
    assert_eq!(state.reports().reports_for(organization).count(), 1);
    validate_state(&state).expect("deadline-missed operation should remain valid");
    validate_invariants(&state);

    let envelope = build_save(&registry, &state).expect("deadline miss should be saveable");
    let bytes = bincode::serialize(&envelope).expect("deadline save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("deadline save should deserialize");
    let restored = restore_save(&registry, decoded).expect("deadline save should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.abort_record())
            .map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
}

#[test]
fn in_progress_operation_aborts_when_its_deadline_passes_without_resolution() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(10),
        ));
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin before its deadline");

    state.advance_clock(crate::core::time::SimDuration::from_minutes(9));
    let outcome = crate::core::simulation::run_tick(&registry, &mut state);

    assert!(outcome.resolved_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-aborted operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(
        record.abort_record().map(|abort| abort.phase()),
        Some(OperationAbortPhase::InProgress)
    );
    assert_eq!(
        record.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    let artifacts = record
        .abort_record()
        .and_then(|abort| abort.artifacts())
        .expect("an in-progress deadline miss should be visible");
    assert!(
        state
            .intelligence()
            .get_information(artifacts.information())
            .expect("deadline information should persist")
            .summary()
            .contains("before execution could complete")
    );
    validate_state(&state).expect("deadline abort should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn missed_deadline_scan_preserves_deadline_chronology_before_operation_id() {
    let (registry, mut state, organization, first_leader, target) = make_test_operation_state();
    let second_leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Second Deadline Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second leader fixture should validate");

    let mut later_deadline = make_test_draft(organization, first_leader, target);
    later_deadline.title = "Later deadline lower id".to_owned();
    later_deadline
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(20),
        ));
    let lower_id = validate_authorize_operation(&registry, &state, later_deadline)
        .expect("later-deadline operation should validate")
        .commit(&mut state)
        .expect("later-deadline operation should commit");

    let mut earlier_deadline = make_test_draft(organization, second_leader, target);
    earlier_deadline.title = "Earlier deadline higher id".to_owned();
    earlier_deadline
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(10),
        ));
    let higher_id = validate_authorize_operation(&registry, &state, earlier_deadline)
        .expect("earlier-deadline operation should validate")
        .commit(&mut state)
        .expect("earlier-deadline operation should commit");
    assert!(lower_id < higher_id);

    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, lower_id, OperationTransition::Begin)
        .expect("lower-ID operation should begin");
    apply_transition(&registry, &mut state, higher_id, OperationTransition::Begin)
        .expect("higher-ID operation should begin");
    state.advance_clock(crate::core::time::SimDuration::from_minutes(20));

    assert_eq!(
        find_due_operations_with_missed_deadlines(&state),
        vec![higher_id, lower_id],
        "the older missed deadline must be handled before a newer deadline even when its operation ID is higher"
    );
    validate_state(&state).expect("overdue operation fixture should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn deadline_constrained_operation_resolves_on_its_clamped_deadline_minute() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(10),
        ));
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline-constrained operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained operation should commit");

    // One canonical minute per tick: the clamped window must resolve exactly on the deadline.
    let mut resolved = None;
    for _ in 0..24 {
        let tick = crate::core::simulation::run_tick(&registry, &mut state);
        if tick.resolved_operations.contains(&operation) {
            resolved = Some(tick);
            break;
        }
    }
    let tick = resolved.expect("a deadline-constrained operation must resolve on its window");
    assert_eq!(tick.now, SimTime::from_minutes(10));
    let record = state
        .operations()
        .get_operation(operation)
        .expect("resolved operation should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    // The compressed execution window is visible in the organization's after-action knowledge.
    assert!(
        state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(organization),
                InformationTopic::OperationalOutcome,
            )
            .any(|information| {
                information.subject() == EntityRef::Operation(operation)
                    && information
                        .summary()
                        .contains("completion deadline compressed the execution window")
            })
    );
    validate_state(&state).expect("deadline resolution should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn decision_paused_operation_auto_aborts_when_deadline_expires() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    designate_player_organization(&mut state, organization)
        .expect("decision fixture organization should be eligible as the player organization");
    let mut draft = make_test_draft(organization, leader, target);
    draft
        .constraints
        .push(crate::operations::OperationConstraint::CompleteBy(
            SimTime::from_minutes(10),
        ));
    draft
        .contingencies
        .push(crate::operations::OperationContingency::RequestDecisionOnPoliceArrival);
    let operation = validate_authorize_operation(&registry, &state, draft)
        .expect("decision-capable operation should validate")
        .commit(&mut state)
        .expect("decision-capable operation should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("operation should begin before its deadline");
    // The dispatched response arrives and pauses the operation pending leadership.
    let mut decision = None;
    for _ in 0..9 {
        let outcome = crate::core::simulation::run_tick(&registry, &mut state);
        if let Some(request) = outcome.decision_requests.first() {
            decision = Some(request.decision);
            break;
        }
    }
    let decision = decision.expect("police response must request leadership before the deadline");
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("operation should persist")
            .status(),
        OperationStatus::AwaitingDecision
    );

    while state.now() < SimTime::from_minutes(12) {
        crate::core::simulation::run_tick(&registry, &mut state);
    }

    let record = state
        .operations()
        .get_operation(operation)
        .expect("deadline-aborted operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert_eq!(
        record.abort_record().map(|abort| abort.phase()),
        Some(OperationAbortPhase::AwaitingDecision)
    );
    assert_eq!(
        record.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    let request = state
        .decisions()
        .get_decision(decision)
        .expect("deadline decision should remain historical");
    assert_eq!(request.status(), crate::decisions::DecisionStatus::Resolved);
    assert_eq!(
        request.resolution().map(|resolution| resolution.response()),
        Some(crate::decisions::DecisionResponse::Abort)
    );
    assert!(state.decisions().pending_for_operation(operation).is_none());
    validate_state(&state).expect("deadline decision abort should remain valid");
    validate_invariants(&state);
}

#[test]
fn stale_authorization_cannot_commit_after_leader_reassignment() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let validated = validate_authorize_operation(
        &registry,
        &state,
        make_test_draft(organization, leader, target),
    )
    .expect("operation should validate against the original hierarchy");

    validate_reassign_character(&state, leader, None, None)
        .expect("leader should be reassignable before the operation is committed")
        .commit(&mut state)
        .expect("reassignment should commit");

    let error = validated
        .commit(&mut state)
        .expect_err("stale authorization must not create an invalid operation");
    assert_eq!(
        error,
        OperationError::StaleParticipant {
            character: leader,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn authorization_expires_if_scheduled_time_passes_before_commit() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(2);
    let validated = validate_authorize_operation(&registry, &state, draft)
        .expect("future operation should validate");

    for _ in 0..3 {
        crate::core::simulation::run_tick(&registry, &mut state);
    }

    let error = validated
        .commit(&mut state)
        .expect_err("authorization must expire once its scheduled time is in the past");
    assert_eq!(
        error,
        OperationError::AuthorizationExpired {
            scheduled_for: 2,
            now: 3,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn authorization_commit_rechecks_deadline_against_actual_authorization_minute() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.scheduled_for = SimTime::from_minutes(10);
    draft.constraints = vec![crate::operations::OperationConstraint::CompleteBy(
        SimTime::from_minutes(11),
    )];
    let validated = validate_authorize_operation(&registry, &state, draft)
        .expect("deadline leaves one minute when authorization is still prospective");

    state.advance_clock(SimDuration::from_minutes(10));
    let error = validated
        .commit(&mut state)
        .expect_err("committing on the schedule minute shifts begin to the next tick");
    assert_eq!(error, OperationError::DeadlineLeavesNoExecutionWindow);
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn operation_intelligence_must_be_owned_and_relevant() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    let irrelevant = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary:
                "Detailed security information that does not answer an intimidation planning need."
                    .to_owned(),
        },
    )
    .expect("irrelevant information fixture should still be valid information")
    .commit(&mut state)
    .expect("irrelevant information fixture should commit");
    let unavailable = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(leader),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "The leader knows the target's personnel pattern personally.".to_owned(),
        },
    )
    .expect("character information fixture should validate")
    .commit(&mut state)
    .expect("character information fixture should commit");

    let mut draft = make_test_draft(organization, leader, target);
    draft.intelligence = BTreeSet::from([irrelevant]);
    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("authored operation should reject an irrelevant intelligence topic");
    assert_eq!(error, OperationError::IrrelevantInformation(irrelevant));

    let mut draft = make_test_draft(organization, leader, target);
    draft.intelligence = BTreeSet::from([unavailable]);
    let error = validate_authorize_operation(&registry, &state, draft).expect_err(
        "operation plan should reject intelligence not yet reported to the organization",
    );
    assert_eq!(
        error,
        OperationError::InformationUnavailable {
            information: unavailable,
            organization,
        }
    );
    assert_eq!(
        state
            .operations()
            .operations_for_organization(organization)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn require_intelligence_topic_constraint_gates_authorization() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    // Sabotage requires a target with an active operating economy; establish one so the
    // intelligence gate is what this test exercises.
    let EntityRef::Business(business_id) = target else {
        panic!("fixture target should be a business");
    };
    let operating = crate::finance::finance_system::insert_account(
        &mut state,
        crate::finance::FinancialAccountDraft {
            owner: crate::finance::FinancialOwner::Business(business_id),
            kind: crate::finance::AccountKind::LegitimateOperating,
        },
    )
    .expect("operating account should validate");
    let settlement = crate::finance::finance_system::insert_account(
        &mut state,
        crate::finance::FinancialAccountDraft {
            owner: crate::finance::FinancialOwner::Business(business_id),
            kind: crate::finance::AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");
    crate::economy::business_economy_system::validate_establish_business_economy(
        &registry,
        &state,
        crate::economy::BusinessEconomyDraft {
            business: business_id,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .expect("target economy fixture should validate")
    .commit(&mut state)
    .expect("target economy fixture should commit");
    let specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Entry Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("specialist fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    draft.kind = OperationKind::Sabotage;
    draft.title = "Test sabotage".to_owned();
    draft.objective = OperationObjective::DisruptBusiness { target };
    draft.approach = OperationApproach::Covert;
    draft.roles.insert(RoleKind::EntrySpecialist, specialist);
    draft.constraints.push(
        crate::operations::OperationConstraint::RequireIntelligenceTopic(
            InformationTopic::TargetSecurity,
        ),
    );

    // Without any intelligence the recon-prerequisite gate must reject authorization.
    let error = validate_authorize_operation(&registry, &state, draft.clone())
        .expect_err("authorization without required intelligence must be rejected");
    assert!(
        matches!(
            error,
            OperationError::MissingRequiredIntelligenceTopic(InformationTopic::TargetSecurity)
        ),
        "unexpected error: {error}"
    );

    // Irrelevant-topic intelligence does not satisfy the gate.
    let wrong_topic = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::Surveillance,
            topic: InformationTopic::Route,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Delivery route observed.".to_owned(),
        },
    )
    .expect("route information should record")
    .commit(&mut state)
    .expect("route information should commit");
    let mut wrong_draft = draft.clone();
    wrong_draft.intelligence.insert(wrong_topic);
    let error = validate_authorize_operation(&registry, &state, wrong_draft)
        .expect_err("authorization with only irrelevant intelligence must be rejected");
    assert!(
        matches!(error, OperationError::IrrelevantInformation(_))
            || matches!(
                error,
                OperationError::MissingRequiredIntelligenceTopic(InformationTopic::TargetSecurity)
            ),
        "unexpected error: {error}"
    );

    // With the required topic covered the same plan authorizes.
    let covering = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::Surveillance,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: target,
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Rear door has no alarm sensor.".to_owned(),
        },
    )
    .expect("target-security information should record")
    .commit(&mut state)
    .expect("target-security information should commit");

    // Information that will have decayed to zero canonical planning value by the planned start
    // cannot satisfy a reconnaissance prerequisite merely because the record still exists.
    let mut stale_draft = draft.clone();
    stale_draft.intelligence.insert(covering);
    stale_draft.scheduled_for = state
        .now()
        .checked_add(
            registry
                .get_operation(OperationKind::Sabotage)
                .execution()
                .max_intelligence_age(),
        )
        .expect("test schedule should fit simulation time");
    let error = validate_authorize_operation(&registry, &state, stale_draft)
        .expect_err("zero-value intelligence at the planned start must not satisfy the constraint");
    assert_eq!(
        error,
        OperationError::MissingRequiredIntelligenceTopic(InformationTopic::TargetSecurity)
    );

    draft.intelligence.insert(covering);
    let validated = validate_authorize_operation(&registry, &state, draft)
        .expect("covered plan should authorize");
    validated
        .commit(&mut state)
        .expect("covered plan should commit");
    validate_invariants(&state);
}

#[test]
fn sabotage_objective_is_rejected_for_non_sabotage_kinds() {
    let (registry, state, organization, leader, target) = make_test_operation_state();
    let mut draft = make_test_draft(organization, leader, target);
    draft.objective = OperationObjective::DisruptBusiness { target };
    let error = validate_authorize_operation(&registry, &state, draft)
        .expect_err("intimidation cannot carry a sabotage objective");
    assert!(matches!(
        error,
        OperationError::InvalidObjectiveForKind {
            kind: OperationKind::Intimidation,
            objective: OperationObjectiveKind::DisruptBusiness,
        }
    ));
}

#[test]
fn sabotage_of_a_business_without_an_operating_economy_is_rejected_atomically() {
    let (registry, mut state, organization, leader, target) = make_test_operation_state();
    // The fixture business has no operating economy record: there is nothing to disrupt.
    let specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Sabotage Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("specialist fixture should validate");
    let mut draft = make_test_draft(organization, leader, target);
    draft.kind = OperationKind::Sabotage;
    draft.title = "Economy-less sabotage".to_owned();
    draft.objective = OperationObjective::DisruptBusiness { target };
    draft.approach = OperationApproach::Covert;
    draft.roles.insert(RoleKind::EntrySpecialist, specialist);

    let EntityRef::Business(business) = target else {
        panic!("fixture target should be a business");
    };
    assert!(state.economy().get_business_economy(business).is_none());

    let error = match validate_authorize_operation(&registry, &state, draft) {
        Err(error) => error,
        Ok(_) => panic!("sabotage of an economy-less business must be rejected"),
    };
    assert!(
        matches!(
          error,
          OperationError::TargetWithoutOperatingEconomy(economyless) if economyless == business
        ),
        "unexpected error: {error}"
    );
}
