//! Tick-ordering, due-work atomicity, deterministic RNG, and cross-domain simulation tests.

use super::*;
use crate::build_registry;
use crate::core::entity::EntityRef;
use crate::core::id::OrganizationId;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
use crate::economy::BusinessEconomyDraft;
use crate::economy::business_economy_system::validate_establish_business_economy;
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::legal::JurisdictionDraft;
use crate::legal::investigation_system::{
    validate_add_evidence, validate_assign_investigator, validate_open_investigation,
};
use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::{
    Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft, InvestigationWorkDraft, InvestigationWorkFocus, InvestigationWorkKind,
};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationConstraint, OperationContingency, OperationDraft, OperationKind,
    OperationObjective, OperationStatus, RoleKind,
};
use crate::reputation::AudienceKind;
use crate::reputation::reputation_system::{apply_reputation_delta, resolve_score};
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
fn executive_brief_report_exhaustion_is_terminal_noop() {
    let registry = build_registry();
    let mut state = AppState::new(0xB12E_FFFF);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Terminal Brief Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("brief organization should validate");
    designate_player_organization(&mut state, organization)
        .expect("brief organization should become the player");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(crate::core::time::DAY_MINUTES)
            .expect("authored day length must fit SimDuration"),
    ));
    assert!(is_executive_brief_due(&registry, state.now()));
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX);
    let before = bincode::serialize(&state).expect("pre-brief state should serialize");

    assert_eq!(synthesize_executive_brief(&registry, &mut state), None);
    assert_eq!(
        bincode::serialize(&state).expect("post-brief state should serialize"),
        before,
        "terminal report capacity must not mutate executive-brief state"
    );
    assert_eq!(state.reports().reports().count(), 0);
    validate_state(&state).expect("terminal brief no-op should remain valid");
    validate_invariants(&state);
}

fn make_resolved_player_reputation_pair() -> (Registry, AppState, OrganizationId, Vec<OperationId>)
{
    let registry = build_registry();
    let mut state = AppState::new(0x57A0_D1A6);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Standing Cohort Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("standing cohort organization should validate");
    designate_player_organization(&mut state, organization)
        .expect("standing cohort organization should become the player");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Standing Cohort Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(60),
                    commercial_activity: test_rating(60),
                    illicit_demand: test_rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(0),
                },
            },
        },
    )
    .expect("standing cohort neighborhood should validate");
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Standing Cohort Shop".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("standing cohort target should validate");

    let mut operations = Vec::new();
    for index in 0..2 {
        let leader = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Standing Cohort Leader {index}"),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Management, test_rating(100)),
                    (CapabilityKind::Stealth, test_rating(100)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("standing cohort leader should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: format!("Standing cohort collection {index}"),
                kind: OperationKind::Intimidation,
                responsible_organization: organization,
                leader,
                objective: OperationObjective::ObtainCash {
                    target: EntityRef::Business(target),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: SimTime::ZERO,
            },
        )
        .expect("standing cohort operation should validate")
        .commit(&mut state)
        .expect("standing cohort operation should commit");
        operations.push(operation);
    }
    state.advance_clock(SimDuration::ONE_MINUTE);
    for operation in &operations {
        apply_transition(
            &registry,
            &mut state,
            *operation,
            OperationTransition::Begin,
        )
        .expect("standing cohort operation should begin");
    }
    let due_at = state
        .operations()
        .get_operation(operations[0])
        .and_then(|record| record.resolution_due_at())
        .expect("standing cohort operation must have a due time");
    assert_eq!(
        state
            .operations()
            .get_operation(operations[1])
            .and_then(|record| record.resolution_due_at()),
        Some(due_at)
    );
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("standing cohort duration must fit SimDuration"),
    ));
    let execution = registry
        .get_operation(OperationKind::Intimidation)
        .execution();
    let execution_variance =
        i8::try_from(execution.variance_limit()).expect("authored operation variance fits i8");
    let exposure_variance = -i8::try_from(execution.exposure_variance_limit())
        .expect("authored exposure variance fits i8");
    for operation in &operations {
        let plan = decide_operation_resolution(
            &registry,
            &state,
            *operation,
            OperationResolutionRandomness::new(execution_variance, exposure_variance),
        )
        .expect("standing cohort operation should decide");
        validate_operation_resolution_plan(&registry, &state, plan)
            .expect("standing cohort resolution should validate")
            .commit(&mut state)
            .expect("standing cohort resolution should commit");
        let resolution = state
            .operations()
            .get_operation(*operation)
            .and_then(|record| record.resolution())
            .expect("standing cohort operation should be resolved");
        assert_ne!(
            resolution.objective_outcome(),
            crate::operations::OperationObjectiveOutcome::Failed,
            "maximal execution variance should produce a reputation-moving result"
        );
        assert!(matches!(
            resolution.exposure().level(),
            crate::operations::OperationExposureLevel::None
                | crate::operations::OperationExposureLevel::Trace
        ));
    }
    (registry, state, organization, operations)
}

#[test]
fn reputation_phase_treats_standing_report_exhaustion_as_terminal_before_mutation() {
    let (registry, mut state, organization, operations) = make_resolved_player_reputation_pair();
    let baseline = registry.reputation().baseline();
    assert_eq!(
        resolve_score(
            &registry,
            state.reputation(),
            organization,
            AudienceKind::Underworld,
        ),
        baseline
    );
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX - 1);
    let before = bincode::serialize(&state).expect("pre-reputation state should serialize");

    let changed = apply_reputation_phase(&registry, &mut state, &operations, &[])
        .expect("Standing-report exhaustion is a terminal autonomous no-op");
    assert_eq!(changed, 0);
    assert_eq!(
        bincode::serialize(&state).expect("rejected reputation state should serialize"),
        before,
        "Standing-report exhaustion must reject before decay or the first consequence mutation"
    );
    assert_eq!(
        resolve_score(
            &registry,
            state.reputation(),
            organization,
            AudienceKind::Underworld,
        ),
        baseline
    );
    validate_invariants(&state);
}

#[test]
fn reputation_phase_preflight_counts_only_consequence_groups_that_still_move() {
    let (registry, mut state, organization, operations) = make_resolved_player_reputation_pair();
    let first_outcome = state
        .operations()
        .get_operation(operations[0])
        .and_then(|record| record.resolution())
        .expect("first standing operation should be resolved")
        .objective_outcome();
    let competence_delta = match first_outcome {
        crate::operations::OperationObjectiveOutcome::Achieved => {
            registry.reputation().achieved_underworld_competence()
        }
        crate::operations::OperationObjectiveOutcome::Partial => {
            registry.reputation().partial_underworld_competence()
        }
        crate::operations::OperationObjectiveOutcome::Failed => unreachable!(),
    };
    assert!(competence_delta > 0);
    let competence_delta = u8::try_from(competence_delta)
        .expect("authored positive competence consequence must fit u8");
    let baseline = registry.reputation().baseline();
    let starting_score = 100_u8
        .checked_sub(competence_delta)
        .expect("authored competence consequence cannot exceed the score range");
    assert!(starting_score >= baseline);
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Underworld,
        i8::try_from(starting_score - baseline).expect("fixture reputation movement must fit i8"),
    )
    .expect("fixture reputation should move to one step below the rail");
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX - 1);

    let changed = apply_reputation_phase(&registry, &mut state, &operations, &[])
        .expect("only the first consequence group still needs a Standing report");

    assert_eq!(changed, 1);
    assert_eq!(
        resolve_score(
            &registry,
            state.reputation(),
            organization,
            AudienceKind::Underworld,
        ),
        100,
    );
    assert_eq!(
        state
            .reports()
            .reports_for(organization)
            .filter(|report| report.kind() == crate::reports::ReportKind::Standing)
            .count(),
        1,
        "the saturated second consequence must not reserve or emit a phantom report"
    );
    assert_eq!(
        state.ids.next_raw(crate::core::id::IdKind::Report),
        u32::MAX,
        "exactly one final report ID should have been consumed"
    );
    validate_invariants(&state);
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
fn due_business_cycle_cohort_treats_allocator_exhaustion_as_terminal_without_progress() {
    let registry = build_registry();
    let mut state = AppState::new(0xB051_BA7C);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Batch Business Holdings".to_owned(),
            kind: OrganizationKind::Commercial,
        },
    )
    .expect("business owner fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Batch Commerce Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(60),
                    commercial_activity: test_rating(70),
                    illicit_demand: test_rating(30),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(55),
                },
            },
        },
    )
    .expect("business neighborhood should validate");

    let mut businesses = Vec::new();
    for name in ["First Batch Shop", "Second Batch Shop"] {
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: name.to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(organization),
            },
        )
        .expect("batch business should validate");
        let operating = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .expect("batch operating account should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .expect("batch settlement account should validate");
        validate_establish_business_economy(
            &registry,
            &state,
            BusinessEconomyDraft {
                business,
                operating_account: operating,
                settlement_account: settlement,
            },
        )
        .expect("batch business economy should validate")
        .commit(&mut state)
        .expect("batch business economy should commit");
        businesses.push(business);
    }

    state.advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(find_due_businesses(&state), businesses);
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::BusinessCycle, u32::MAX - 1);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion business state should serialize");

    let cycles = run_business_cycle_phase(&registry, &mut state)
        .expect("business-cycle ID exhaustion is a terminal autonomous no-op");
    assert!(cycles.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected business state should serialize"),
        before,
        "batch rejection must preserve both business state and the serialized business RNG"
    );
    assert_eq!(find_due_businesses(&state), businesses);
    validate_state(&state).expect("rejected business-cycle cohort should remain valid");
    validate_invariants(&state);
}

#[test]
fn authorized_prestart_abort_cohort_exhaustion_is_terminal_noop_before_first_abort() {
    let registry = build_registry();
    let mut state = AppState::new(0xA071_BA7C);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Prestart Abort Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("prestart abort crew should validate");

    let mut operations = Vec::new();
    for index in 0..2 {
        let leader = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Late Starter {index}"),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(CapabilityKind::Surveillance, test_rating(60))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("late starter should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: format!("Missed prestart window {index}"),
                kind: OperationKind::Surveillance,
                responsible_organization: crew,
                leader,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Organization(crew),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: vec![OperationConstraint::CompleteBy(SimTime::from_minutes(60))],
                contingencies: Vec::new(),
                scheduled_for: SimTime::ZERO,
            },
        )
        .expect("deadline-constrained authorized operation should validate")
        .commit(&mut state)
        .expect("deadline-constrained authorized operation should commit");
        operations.push(operation);
    }

    state.advance_clock(SimDuration::from_minutes(61));
    assert_eq!(find_due_authorized_operations(&state), operations);
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Information, u32::MAX - 1);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion authorized state should serialize");

    let planned = prepare_authorized_prestart_aborts(&registry, &state, &operations)
        .expect("prestart artifact exhaustion is a terminal autonomous no-op");
    assert!(planned.is_none());
    assert_eq!(
        bincode::serialize(&state).expect("rejected authorized state should serialize"),
        before,
        "artifact exhaustion must not abort only the earliest authorized operation"
    );
    for operation in operations {
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("rejected authorized operation should persist")
                .status(),
            OperationStatus::Authorized
        );
    }
    validate_state(&state).expect("rejected prestart abort cohort should remain valid");
    validate_invariants(&state);
}

#[test]
fn overdue_operation_cleanup_exhaustion_is_terminal_noop_before_mutation() {
    let registry = build_registry();
    let mut state = AppState::new(0x0A0E_D34D);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Overdue Cleanup Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("overdue cleanup crew should validate");

    let mut operations = Vec::new();
    for index in 0..2 {
        let leader = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Overdue Scout {index}"),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(CapabilityKind::Surveillance, test_rating(60))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("overdue scout should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: format!("Overdue surveillance {index}"),
                kind: OperationKind::Surveillance,
                responsible_organization: crew,
                leader,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Organization(crew),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: vec![OperationConstraint::CompleteBy(SimTime::from_minutes(60))],
                contingencies: Vec::new(),
                scheduled_for: SimTime::ZERO,
            },
        )
        .expect("deadline-constrained surveillance should validate")
        .commit(&mut state)
        .expect("deadline-constrained surveillance should commit");
        operations.push(operation);
    }

    state.advance_clock(SimDuration::ONE_MINUTE);
    for operation in &operations {
        apply_transition(
            &registry,
            &mut state,
            *operation,
            OperationTransition::Begin,
        )
        .expect("overdue fixture operation should begin");
    }
    state.advance_clock(SimDuration::from_minutes(60));
    assert_eq!(
        find_due_operations_with_missed_deadlines(&state),
        operations,
        "both independent operations must be overdue in canonical deadline order"
    );

    // Each deadline abort emits one primary information record. Leave room for exactly one:
    // individual aborts fit, but the complete same-minute cohort does not.
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Information, u32::MAX - 1);
    let before = bincode::serialize(&state).expect("pre-exhaustion overdue state should serialize");

    let aborted = apply_overdue_operation_cleanup(&registry, &mut state)
        .expect("overdue abort artifact exhaustion is a terminal autonomous no-op");
    assert!(aborted.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected overdue state should serialize"),
        before,
        "artifact exhaustion must not abort only the earliest overdue operation"
    );
    for operation in operations {
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("rejected overdue operation should persist")
                .status(),
            OperationStatus::InProgress
        );
    }
    validate_state(&state).expect("rejected overdue cohort should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn due_operation_resolution_treats_allocator_exhaustion_as_terminal_without_rng_progress() {
    let registry = build_registry();
    let mut state = AppState::new(0x0A0E_BA7C);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Transactional Resolution Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("resolution crew should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Quiet Resolution Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(50),
                    commercial_activity: test_rating(50),
                    illicit_demand: test_rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(0),
                },
            },
        },
    )
    .expect("resolution neighborhood should validate");
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Observed Corner Shop".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("resolution target should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Transactional Observer".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, test_rating(100)),
                (CapabilityKind::Stealth, test_rating(100)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("resolution leader should validate");
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Transactional surveillance".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::ZERO,
        },
    )
    .expect("resolution operation should validate")
    .commit(&mut state)
    .expect("resolution operation should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
        .expect("resolution operation should begin at its canonical earliest start");
    let due_at = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution_due_at())
        .expect("in-progress operation must have a due time");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture execution duration must fit SimDuration"),
    ));
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion operation state should serialize");

    let resolved = run_operation_resolution_phase(&registry, &mut state)
        .expect("resolution artifact exhaustion is a terminal autonomous no-op");
    assert!(resolved.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected resolution state should serialize"),
        before,
        "failed resolution must preserve authoritative state and operation RNG exactly"
    );
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("rejected operation should persist")
            .status(),
        OperationStatus::InProgress
    );
    validate_state(&state).expect("rejected operation resolution should remain valid");
    validate_invariants(&state);
}

#[test]
fn due_operation_resolution_cohort_preflights_mandatory_reports() {
    let registry = build_registry();
    let mut state = AppState::new(0x0A0E_C0A0);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cohort Resolution Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("resolution crew should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Cohort Resolution Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(50),
                    commercial_activity: test_rating(50),
                    illicit_demand: test_rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(0),
                },
            },
        },
    )
    .expect("resolution neighborhood should validate");
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Cohort Observed Shop".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("resolution target should validate");
    let mut operations = Vec::new();
    for index in 0..2 {
        let leader = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Cohort Observer {index}"),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Surveillance, test_rating(100)),
                    (CapabilityKind::Stealth, test_rating(100)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("resolution leader should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: format!("Cohort surveillance {index}"),
                kind: OperationKind::Surveillance,
                responsible_organization: crew,
                leader,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Business(target),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: SimTime::ZERO,
            },
        )
        .expect("resolution operation should validate")
        .commit(&mut state)
        .expect("resolution operation should commit");
        operations.push(operation);
    }
    state.advance_clock(SimDuration::ONE_MINUTE);
    for operation in &operations {
        apply_transition(
            &registry,
            &mut state,
            *operation,
            OperationTransition::Begin,
        )
        .expect("cohort resolution operation should begin");
    }
    let due_at = state
        .operations()
        .get_operation(operations[0])
        .and_then(|record| record.resolution_due_at())
        .expect("in-progress operation must have a due time");
    assert_eq!(
        state
            .operations()
            .get_operation(operations[1])
            .and_then(|record| record.resolution_due_at()),
        Some(due_at),
        "matched surveillance operations should resolve in one cohort"
    );
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture execution duration must fit SimDuration"),
    ));
    assert_eq!(find_due_in_progress_operations(&state), operations);
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX - 1);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion operation cohort should serialize");
    let mut untouched_rng = state.operation_rng_mut().clone();

    let resolved = run_operation_resolution_phase(&registry, &mut state)
        .expect("mandatory report exhaustion is a terminal autonomous no-op");
    assert!(resolved.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected operation cohort should serialize"),
        before,
        "mandatory report exhaustion must reject before the first same-minute resolution"
    );
    let post_failure_draw =
        draw_index(state.operation_rng_mut(), 100).expect("operation RNG draw should succeed");
    let untouched_draw =
        draw_index(&mut untouched_rng, 100).expect("control operation RNG draw should succeed");
    assert_eq!(post_failure_draw, untouched_draw);
    for operation in operations {
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("rejected operation should persist")
                .status(),
            OperationStatus::InProgress
        );
    }
    validate_state(&state).expect("rejected operation-resolution cohort should remain valid");
    validate_invariants(&state);
}

#[test]
fn due_investigation_work_cohort_treats_late_artifact_exhaustion_as_terminal_noop() {
    let registry = build_registry();
    let mut state = AppState::new(0x1A7E_BA7C);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Batch Investigation Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("investigation authority fixture should validate");
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Batch Investigation Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("investigation subject organization should validate");

    let mut works = Vec::new();
    for index in 0..2 {
        let investigator = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Batch Detective {index}"),
                organization: Some(police),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Investigation, test_rating(100))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("batch investigator should validate");
        let subject = insert_character(
            &mut state,
            CharacterDraft {
                name: format!("Batch Subject {index}"),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("batch case subject should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: format!("Batch forensic inquiry {index}"),
                subjects: BTreeSet::from([EntityRef::Character(subject)]),
            },
        )
        .expect("batch investigation should validate")
        .commit(&mut state)
        .expect("batch investigation should commit");
        validate_assign_investigator(&state, investigation, investigator)
            .expect("batch lead assignment should validate")
            .commit(&mut state)
            .expect("batch lead assignment should commit");
        let evidence = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(subject),
                origin: None,
                kind: EvidenceKind::Fingerprint,
                strength: EvidenceStrength::Direct,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("batch source evidence should validate")
        .commit(&mut state)
        .expect("batch source evidence should commit");
        let work = validate_schedule_investigation_work(
            &registry,
            &state,
            InvestigationWorkDraft {
                investigation,
                investigator,
                kind: InvestigationWorkKind::EvidenceReview,
                focus: InvestigationWorkFocus::evidence(evidence),
            },
        )
        .expect("batch forensic work should validate")
        .commit(&mut state)
        .expect("batch forensic work should commit");
        works.push(work);
    }

    let duration = registry
        .get_investigation_work(InvestigationWorkKind::EvidenceReview)
        .duration();
    state.advance_clock(duration);
    assert_eq!(find_due_scheduled_investigation_work(&state), works);

    // Preview exactly the draws the production phase will freeze without publishing them.
    // The maximally capable, direct-evidence fixture must make both reviews produce evidence,
    // otherwise it would not exercise later-item allocator exhaustion.
    let mut preview_rng = state.investigation_rng_mut().clone();
    let mut evidence_budget = 0_u32;
    for work in &works {
        let variance_limit = registry
            .get_investigation_work(InvestigationWorkKind::EvidenceReview)
            .variance_limit();
        let variance = draw_signed_variance(&mut preview_rng, variance_limit);
        let plan = decide_investigation_work_resolution(
            &registry,
            &state,
            *work,
            InvestigationWorkRandomness::new(variance),
        )
        .expect("preview work resolution should decide");
        let validated = validate_investigation_work_resolution_plan(&registry, &state, plan)
            .expect("preview work resolution should validate");
        evidence_budget += validated
            .id_budget()
            .into_iter()
            .filter(|(kind, _)| *kind == crate::core::id::IdKind::Evidence)
            .map(|(_, count)| count)
            .sum::<u32>();
    }
    assert_eq!(
        evidence_budget, 2,
        "fixed cohort must make both forensic reviews consume evidence IDs"
    );
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Evidence, u32::MAX - 1);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion investigation state should serialize");
    let mut untouched_rng = state.investigation_rng_mut().clone();

    let resolved = run_investigation_work_phase(&registry, &mut state)
        .expect("evidence-ID exhaustion is a terminal autonomous no-op");
    assert!(resolved.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected investigation state should serialize"),
        before,
        "batch rejection must preserve all due work and the serialized investigation RNG"
    );
    let post_failure_draw = draw_index(state.investigation_rng_mut(), 100)
        .expect("post-failure investigation draw should succeed");
    let untouched_draw =
        draw_index(&mut untouched_rng, 100).expect("control investigation draw should succeed");
    assert_eq!(
        post_failure_draw, untouched_draw,
        "failed cohort must not publish any speculative investigation-work draw"
    );
    for work in works {
        assert_eq!(
            state
                .legal()
                .get_investigation_work(work)
                .expect("rejected batch work should persist")
                .status(),
            crate::legal::InvestigationWorkStatus::Scheduled
        );
    }
    validate_state(&state).expect("rejected investigation-work cohort should remain valid");
    validate_invariants(&state);
}

#[test]
fn due_enterprise_cycle_treats_allocator_exhaustion_as_terminal_without_rng_progress() {
    let registry = build_registry();
    let mut state = AppState::new(0xE17E_BA7C);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Transactional Racket Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("enterprise organization should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Transactional Racket Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(60),
                    commercial_activity: test_rating(70),
                    illicit_demand: test_rating(55),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(40),
                },
            },
        },
    )
    .expect("enterprise neighborhood should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Transactional Racket Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, test_rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("enterprise manager should validate");
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("enterprise mandate should validate")
    .commit(&mut state)
    .expect("enterprise mandate should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("enterprise cash account should validate");
    let settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("enterprise settlement account should validate");
    let enterprise = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization,
            authority: MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .expect("protection enterprise should validate")
    .commit(&mut state)
    .expect("protection enterprise should commit");
    let cycle_duration = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .cycle();
    state.advance_clock(cycle_duration);
    assert_eq!(find_due_enterprises(&state), vec![enterprise]);
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::EnterpriseCycle, u32::MAX);
    let before =
        bincode::serialize(&state).expect("pre-exhaustion enterprise state should serialize");
    let mut untouched_rng = state.enterprise_rng_mut().clone();

    let cycles = run_enterprise_cycle_phase(&registry, &mut state)
        .expect("cycle-ID exhaustion is a terminal autonomous no-op");
    assert!(cycles.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected enterprise state should serialize"),
        before,
        "failed enterprise settlement must not publish either speculative RNG draw"
    );
    let post_failure_draw =
        draw_index(state.enterprise_rng_mut(), 100).expect("enterprise RNG draw should succeed");
    let untouched_draw =
        draw_index(&mut untouched_rng, 100).expect("control enterprise RNG draw should succeed");
    assert_eq!(post_failure_draw, untouched_draw);
    assert!(
        state.enterprises().cycles_for(enterprise).next().is_none(),
        "rejected enterprise settlement must not persist a cycle"
    );
    validate_state(&state).expect("rejected enterprise-cycle state should remain valid");
    validate_invariants(&state);
}

#[test]
fn due_enterprise_cycle_cohort_preflights_mandatory_cycle_ids() {
    let registry = build_registry();
    let mut state = AppState::new(0xE17E_C0A0);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cohort Racket Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("enterprise organization should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Cohort Racket Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: test_rating(60),
                    commercial_activity: test_rating(70),
                    illicit_demand: test_rating(55),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: test_rating(40),
                },
            },
        },
    )
    .expect("enterprise neighborhood should validate");
    let manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Cohort Racket Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, test_rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("enterprise manager should validate");
    let scope = ResponsibilityScope::Neighborhood(neighborhood);
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("enterprise mandate should validate")
    .commit(&mut state)
    .expect("enterprise mandate should commit");
    let cash = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("enterprise cash account should validate");
    let protection_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("protection settlement account should validate");
    let gambling_settlement = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("gambling settlement account should validate");
    let authority = MandateAuthority {
        mandate,
        manager,
        scope,
    };
    let protection = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Protection,
            organization,
            authority,
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: protection_settlement,
        },
    )
    .expect("protection enterprise should validate")
    .commit(&mut state)
    .expect("protection enterprise should commit");
    let gambling_venue = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Cohort Card Room".to_owned(),
            kind: BusinessKind::Hospitality,
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
    let gambling = validate_establish_enterprise(
        &registry,
        &state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority,
            location: EnterpriseLocation::Business(gambling_venue),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: gambling_settlement,
        },
    )
    .expect("gambling enterprise should validate")
    .commit(&mut state)
    .expect("gambling enterprise should commit");
    state.advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(find_due_enterprises(&state), vec![protection, gambling]);
    state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::EnterpriseCycle, u32::MAX - 1);
    let before = bincode::serialize(&state).expect("pre-cohort state should serialize");
    let mut untouched_rng = state.enterprise_rng_mut().clone();

    let cycles = run_enterprise_cycle_phase(&registry, &mut state)
        .expect("incomplete cycle-ID cohort capacity is a terminal autonomous no-op");
    assert!(cycles.is_empty());
    assert_eq!(
        bincode::serialize(&state).expect("rejected cohort state should serialize"),
        before,
        "mandatory cycle-ID exhaustion must reject before the first same-minute settlement"
    );
    let post_failure_draw =
        draw_index(state.enterprise_rng_mut(), 100).expect("enterprise RNG draw should succeed");
    let untouched_draw =
        draw_index(&mut untouched_rng, 100).expect("control enterprise RNG draw should succeed");
    assert_eq!(post_failure_draw, untouched_draw);
    assert_eq!(state.enterprises().cycles_for(protection).count(), 0);
    assert_eq!(state.enterprises().cycles_for(gambling).count(), 0);
    validate_state(&state).expect("rejected enterprise-cycle cohort should remain valid");
    validate_invariants(&state);
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
