//! Focused tests for deterministic operation resolution, proceeds, exposure, and dispositions.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::id::{BusinessId, FinancialAccountId, OrganizationId};
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::simulation::run_tick;
use crate::core::time::SimDuration;
use crate::decisions::decision_system::{
    validate_request_police_arrival_decision_on_arrival, validate_resolve_decision, DecisionError,
};
use crate::decisions::{DecisionContext, DecisionResponse};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner, Money};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{InformationDraft, InformationTopic};
use crate::legal::informant_system::RECRUITMENT_DECISION_OFFSET_MINUTES;
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::patrol_system::{
    validate_establish_patrol_deployment, validate_revise_patrol_deployment,
};
use crate::legal::{
    Admissibility, ArrestDraft, DayMinute, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, InvestigationDraft, JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow,
};
use crate::operations::operation_economics::RECENT_HIT_WINDOW_MINUTES;
use crate::operations::operation_system::{validate_authorize_operation, OperationError};
use crate::operations::property_disposition::{
    validate_deposit_operation_cash, validate_dispose_property, CashDispositionDraft,
    PropertyDispositionDraft, PropertyDispositionError,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationApproach, OperationContingency,
    OperationDraft, OperationKind, OperationObjective, OperationStatus, RoleKind,
};
use crate::reports::organization_financial_report::validate_organization_financial_report;
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization, validate_reassign_character,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CharacterDraft,
    DriveKind, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind,
};
use std::collections::{BTreeMap, BTreeSet};

fn insert_property_disposition_fixture(
    registry: &Registry,
    state: &mut AppState,
    neighborhood: NeighborhoodId,
    organization: OrganizationId,
) -> (BusinessId, FinancialAccountId, FinancialAccountId) {
    let resale_venue = insert_business(
        registry,
        state,
        BusinessDraft {
            name: "Fixture Pawn Exchange".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::ResaleMarket,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("resale venue should validate");
    let cash_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("liquidation cash account should validate");
    let settlement_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("liquidation settlement account should validate");
    (resale_venue, cash_account, settlement_account)
}

/// Compact cash-capable target for operation fixtures. When `owner` is set the business
/// belongs to that character so its owner can surface as an incident witness.
fn make_fixture_business_with_owner(
    registry: &Registry,
    state: &mut AppState,
    name: &str,
    owner: BusinessOwner,
) -> BusinessId {
    let neighborhood = insert_neighborhood(
        state,
        NeighborhoodDraft {
            name: format!("{name} ward"),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture wealth should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture commerce should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture demand should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(30)
                        .expect("fixture police presence should validate"),
                },
            },
        },
    )
    .expect("fixture neighborhood should validate");
    insert_business(
        registry,
        state,
        BusinessDraft {
            name: name.to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CashIntensive]),
            neighborhood,
            owner,
        },
    )
    .expect("fixture business should validate")
}

fn make_fixture_business(registry: &Registry, state: &mut AppState, name: &str) -> BusinessId {
    make_fixture_business_with_owner(registry, state, name, BusinessOwner::Independent)
}

fn make_operation_fixture() -> (Registry, AppState, OrganizationId, OperationId) {
    let registry = build_registry();
    let mut state = AppState::new(0x0A19_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Operation Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("operation organization fixture should validate");
    let target = make_fixture_business(&registry, &mut state, "Operation Test Target");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Operation Test Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(82).expect("fixture rating should be valid"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("operation leader fixture should validate");
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Operation execution fixture".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: vec![OperationContingency::RequestDecisionOnPoliceArrival],
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("operation fixture should validate")
    .commit(&mut state)
    .expect("validated operation fixture should commit");
    (registry, state, organization, operation)
}

fn make_intelligence_operation_fixture() -> (Registry, AppState, OperationId) {
    let registry = build_registry();
    let mut state = AppState::new(0x1A7E_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Intelligence Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("intelligence operation organization should validate");
    let target = make_fixture_business(&registry, &mut state, "Intelligence Test Target");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Prepared Crew Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(82).expect("fixture management should validate"),
                ),
                (
                    CapabilityKind::Stealth,
                    Rating::try_new(0).expect("fixture stealth should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("prepared leader should validate");
    let mut intelligence = BTreeSet::new();
    for topic in [
        InformationTopic::Personnel,
        InformationTopic::Relationship,
        InformationTopic::PoliceActivity,
    ] {
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic,
                source_entity: None,
                subject: EntityRef::Business(target),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: format!("Fresh precise planning information for {topic:?}."),
            },
        )
        .expect("planning information should validate")
        .commit(&mut state)
        .expect("planning information should commit");
        intelligence.insert(information);
    }
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Prepared intimidation".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence,
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("prepared operation should validate")
    .commit(&mut state)
    .expect("prepared operation should commit");
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(20));
    (registry, state, operation)
}

fn make_exposed_business_operation_fixture(
    assign_jurisdiction: bool,
) -> (
    Registry,
    AppState,
    OrganizationId,
    NeighborhoodId,
    OperationId,
) {
    make_exposed_business_operation_fixture_with_contingencies(assign_jurisdiction, Vec::new())
}

fn make_exposed_business_operation_fixture_with_contingencies(
    assign_jurisdiction: bool,
    contingencies: Vec<OperationContingency>,
) -> (
    Registry,
    AppState,
    OrganizationId,
    NeighborhoodId,
    OperationId,
) {
    make_exposed_operation_fixture(OperationKind::Burglary, assign_jurisdiction, contingencies)
}

/// Builds an observable operation of `kind` against an independent retail business, optionally
/// inside an assigned police jurisdiction so trace exposure can open a case through intake.
fn make_exposed_operation_fixture(
    kind: OperationKind,
    assign_jurisdiction: bool,
    contingencies: Vec<OperationContingency>,
) -> (
    Registry,
    AppState,
    OrganizationId,
    NeighborhoodId,
    OperationId,
) {
    let registry = build_registry();
    let mut state = AppState::new(0xE710_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Exposure Test Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("exposure crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Exposure Test Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("exposure precinct should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Observed Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture wealth should validate"),
                    commercial_activity: Rating::try_new(60)
                        .expect("fixture commerce should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture demand should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(90)
                        .expect("fixture police presence should validate"),
                },
            },
        },
    )
    .expect("exposure neighborhood should validate");
    if assign_jurisdiction {
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(80)
                    .expect("fixture case priority should validate"),
            },
        )
        .expect("precinct jurisdiction should validate")
        .commit(&mut state)
        .expect("precinct jurisdiction should commit");
    }
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Observed Retail Target".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("exposure business should validate");
    if kind == OperationKind::Sabotage {
        // DisruptBusiness authorization requires an operating economy on the target.
        let operating = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .expect("sabotage operating account should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .expect("sabotage settlement account should validate");
        crate::economy::business_economy_system::validate_establish_business_economy(
            &registry,
            &state,
            crate::economy::BusinessEconomyDraft {
                business,
                operating_account: operating,
                settlement_account: settlement,
            },
        )
        .expect("sabotage target economy should establish")
        .commit(&mut state)
        .expect("sabotage target economy should commit");
    }
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Exposure Crew Leader".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(80).expect("fixture management should validate"),
                ),
                (
                    CapabilityKind::Stealth,
                    Rating::try_new(0).expect("fixture stealth should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("exposure leader should validate");
    let specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Exposure Entry Specialist".to_owned(),
            organization: Some(organization),
            supervisor: Some(leader),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Burglary,
                    Rating::try_new(80).expect("fixture burglary should validate"),
                ),
                (
                    CapabilityKind::Stealth,
                    Rating::try_new(0).expect("fixture stealth should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("exposure specialist should validate");
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: match kind {
                OperationKind::Burglary => "Observed burglary".to_owned(),
                OperationKind::Sabotage => "Observed sabotage".to_owned(),
                _ => unreachable!("exposure fixture supports only scene-trace operation kinds"),
            },
            kind,
            responsible_organization: organization,
            leader,
            objective: match kind {
                OperationKind::Burglary => OperationObjective::AcquireProperty {
                    target: EntityRef::Business(business),
                },
                OperationKind::Sabotage => OperationObjective::DisruptBusiness {
                    target: EntityRef::Business(business),
                },
                _ => unreachable!("exposure fixture supports only scene-trace operation kinds"),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, leader),
                (RoleKind::EntrySpecialist, specialist),
            ]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies,
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("exposure operation should validate")
    .commit(&mut state)
    .expect("exposure operation should commit");
    (registry, state, police, neighborhood, operation)
}

#[test]
fn trace_exposing_sabotage_resolves_and_opens_a_case_through_canonical_intake() {
    // Regression: authored sabotage exposure once carried the ForensicAnalysis evidence kind,
    // which the legal intake gate rejects, so any trace-exposing sabotage panicked the tick
    // at plan validation. Sabotage now leaves physical-trace evidence like other scene work.
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture(OperationKind::Sabotage, true, Vec::new());
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let resolution_outcome = loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            break outcome;
        }
    };
    assert!(!resolution_outcome.resolved_operations.is_empty());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("resolved sabotage should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    validate_state(&state).expect("sabotage resolution state should validate");
    validate_state_against_registry(&registry, &state)
        .expect("sabotage intake evidence should match the authored exposure kind");
    validate_invariants(&state);
}

#[test]
fn scheduled_operation_resolves_into_persisted_after_action_report_information_and_history() {
    let (registry, mut state, organization, operation) = make_operation_fixture();
    for minute in 1..=20_u64 {
        let outcome = run_tick(&registry, &mut state);
        assert_eq!(outcome.now, SimTime::from_minutes(minute));
        if minute == 1 {
            assert_eq!(outcome.started_operations, vec![operation]);
        }
        assert!(outcome.resolved_operations.is_empty());
    }

    let outcome = run_tick(&registry, &mut state);
    assert_eq!(outcome.now, SimTime::from_minutes(21));
    assert_eq!(outcome.resolved_operations, vec![operation]);
    let record = state
        .operations()
        .get_operation(operation)
        .expect("resolved operation should remain recorded");
    assert_eq!(record.status(), OperationStatus::Completed);
    let resolution = record
        .resolution()
        .expect("completed operation should persist its resolution");
    let information = state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("operation resolution should create after-action information");
    assert_eq!(
        information.holder(),
        KnowledgeHolder::Organization(organization)
    );
    assert_eq!(
        information.source_kind(),
        InformationSourceKind::AfterAction
    );
    assert_eq!(information.subject(), EntityRef::Operation(operation));
    assert!(information.summary().contains("Objective"));
    let report = state
        .reports()
        .get_report(resolution.after_action_report())
        .expect("operation resolution should create an after-action report");
    assert_eq!(report.kind(), ReportKind::AfterAction);
    assert_eq!(report.recipient(), organization);
    assert_eq!(report.generated_at(), resolution.resolved_at());
    assert_eq!(report.entries().len(), 1);
    assert_eq!(report.entries()[0].attention, AttentionClass::Notable);
    assert_eq!(report.entries()[0].summary, information.summary());
    assert!(report.entries()[0].sources.is_empty());
    assert!(report.entries()[0].decision.is_none());
    assert!(report.entries()[0]
        .entities
        .contains(&EntityRef::Operation(operation)));
    let history = state
        .history()
        .get_event(resolution.history_event())
        .expect("operation resolution should create campaign history");
    assert_eq!(history.kind(), HistoryEventKind::Operation);
    assert!(history
        .entities()
        .contains(&EntityRef::Operation(operation)));
    validate_state(&state).expect("resolved operation state should validate");
    validate_invariants(&state);
}

#[test]
fn after_action_summary_contextualizes_adverse_variance() {
    let factors = OperationResolutionFactors {
        role_capability_average: Rating::try_new(80).expect("fixture rating should be valid"),
        leader_capability: Some(Rating::try_new(80).expect("fixture rating should be valid")),
        intelligence_quality: Rating::try_new(0).expect("fixture rating should be valid"),
        intelligence_adjustment: 0,
        intelligence_topics_covered: 0,
        intelligence_topics_relevant: 1,
        target_police_presence: None,
        police_response_arrived: false,
        approach_adjustment: 0,
        time_pressure: 0,
        variance: -1,
    };

    // On an achieved job the variance is already visible in the outcome; reciting luck
    // commentary would be noise in the executive brief.
    let achieved = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        factors,
        OperationExposureLevel::None,
    );
    assert!(!achieved.contains("unplanned circumstances"));
    assert!(!achieved.contains("crew overcame them"));

    let partial = build_after_action_summary(
        OperationObjectiveOutcome::Partial,
        factors,
        OperationExposureLevel::None,
    );
    assert!(partial.contains("reduced the result"));

    let failed = build_after_action_summary(
        OperationObjectiveOutcome::Failed,
        factors,
        OperationExposureLevel::None,
    );
    assert!(failed.contains("contributed to the failure"));
}

#[test]
fn after_action_summary_omits_neutral_lines_and_keeps_deviations() {
    let neutral = OperationResolutionFactors {
        role_capability_average: Rating::try_new(80).expect("fixture rating should be valid"),
        leader_capability: Some(Rating::try_new(80).expect("fixture rating should be valid")),
        intelligence_quality: Rating::try_new(0).expect("fixture rating should be valid"),
        intelligence_adjustment: 0,
        intelligence_topics_covered: 0,
        intelligence_topics_relevant: 4,
        target_police_presence: None,
        police_response_arrived: false,
        approach_adjustment: 0,
        time_pressure: 0,
        variance: 0,
    };

    // A routine clean job by a strong crew reports just the outcome and genuinely notable
    // context; strong-but-expected crew quality is not recited as a sentence.
    let routine = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        neutral,
        OperationExposureLevel::None,
    );
    assert!(routine.starts_with("Objective achieved."));
    assert!(!routine.contains("Assigned-role competence"));
    assert!(!routine.contains("Leadership coordination"));
    assert!(!routine.contains("normal execution window"));
    assert!(!routine.contains("no material execution advantage"));
    assert!(!routine.contains("neutral to execution difficulty"));
    assert!(!routine.contains("Unplanned circumstances were neutral"));
    assert!(!routine.contains("No material operational exposure"));
    assert!(!routine.contains("limited execution pressure"));
    assert!(routine.contains("No location-based police pressure could be established"));

    // Weak crew quality on a clean job is a risk factor the boss should see.
    let thin = OperationResolutionFactors {
        role_capability_average: Rating::try_new(30).expect("fixture rating should be valid"),
        ..neutral
    };
    let thin_crew = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        thin,
        OperationExposureLevel::None,
    );
    assert!(thin_crew.contains("Assigned-role competence was competent."));

    // Deviations stay: covered intelligence, compressed deadlines, adverse circumstances,
    // and real exposure each earn their sentence.
    let informed = OperationResolutionFactors {
        intelligence_topics_covered: 2,
        ..neutral
    };
    let planned = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        informed,
        OperationExposureLevel::None,
    );
    assert!(planned.contains("Planning intelligence covered 2 of 4 relevant areas"));
    assert!(planned.contains("reduced execution uncertainty"));

    // Coverage below half the relevant areas reads as an honest planning gap instead of
    // false reassurance.
    let gapped = OperationResolutionFactors {
        intelligence_topics_covered: 1,
        ..neutral
    };
    let gapped_plan = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        gapped,
        OperationExposureLevel::None,
    );
    assert!(gapped_plan.contains("Planning intelligence covered 1 of 4 relevant areas"));
    assert!(gapped_plan.contains("large gaps remained in the plan's information"));

    let pressured = OperationResolutionFactors {
        time_pressure: 3,
        ..neutral
    };
    let rushed = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        pressured,
        OperationExposureLevel::None,
    );
    assert!(rushed.contains("compressed the execution window"));

    let witnessed = build_after_action_summary(
        OperationObjectiveOutcome::Partial,
        neutral,
        OperationExposureLevel::Witnessed,
    );
    assert!(witnessed.contains("witnessed or otherwise clearly observed"));
}

#[test]
fn police_arrival_decision_pauses_and_shifts_operation_resolution_schedule() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .responsible_organization();
    let due_before_pause = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .resolution_due_at()
        .expect("in-progress operation should be scheduled for resolution");

    // The response arrives post-entry and raises the leadership decision automatically.
    let paused_at = loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.decision_requests.is_empty() {
            break outcome.now;
        }
        assert!(outcome.resolved_operations.is_empty());
    };
    let decision_id = state
        .decisions()
        .pending_for_operation(operation)
        .expect("arrival decision should be pending");
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .awaiting_decision_since(),
        Some(paused_at)
    );

    for _ in 0..10 {
        let outcome = run_tick(&registry, &mut state);
        assert!(outcome.resolved_operations.is_empty());
    }
    validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Continue,
    )
    .expect("continue response should validate")
    .commit(&mut state)
    .expect("validated continue response should commit");
    let resumed = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist after resume");
    assert_eq!(resumed.status(), OperationStatus::InProgress);
    assert_eq!(resumed.awaiting_decision_since(), None);
    // The pause shifts the resolution deadline by exactly its duration.
    assert_eq!(
        resumed.resolution_due_at(),
        Some(SimTime::from_minutes(due_before_pause.as_minutes() + 10))
    );

    let shifted_due = resumed.resolution_due_at().expect("shifted deadline");
    loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            assert_eq!(outcome.now, shifted_due);
            assert_eq!(outcome.resolved_operations, vec![operation]);
            break;
        }
    }
    validate_state(&state).expect("resumed operation state should validate");
    validate_invariants(&state);
}

#[test]
fn resume_rejects_participant_booked_into_the_pause_extension_window() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    run_tick(&registry, &mut state);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .responsible_organization();
    let original_due = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .resolution_due_at()
        .expect("in-progress operation should be scheduled for resolution");
    let leader = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .leader();
    let target = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .objective()
        .referenced_entities()
        .into_iter()
        .find_map(|entity| match entity {
            EntityRef::Business(business) => Some(business),
            _ => None,
        })
        .expect("fixture objective should reference its target business");

    // The response arrives post-entry and pauses the operation pending leadership.
    let paused_at = loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.decision_requests.is_empty() {
            break outcome.now;
        }
    };
    let decision_id = state
        .decisions()
        .pending_for_operation(operation)
        .expect("arrival decision should be pending");
    // Scheduled past the un-shifted deadline: authorization sees no conflict against the
    // paused operation's stale window.
    let follow_up_start = SimTime::from_minutes(original_due.as_minutes() + 3);
    let follow_up = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Follow-up assignment".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: follow_up_start,
        },
    )
    .expect("follow-up operation should validate")
    .commit(&mut state)
    .expect("follow-up operation should commit");

    // Pause long enough that the shifted deadline moves past the follow-up's start; the
    // extension then double-books the shared leader and the continue must be rejected.
    let pause_minutes_needed = follow_up_start.as_minutes() - original_due.as_minutes() + 2;
    while state.now().as_minutes() - paused_at.as_minutes() < pause_minutes_needed {
        run_tick(&registry, &mut state);
    }
    let error = match validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Continue,
    ) {
        Ok(_) => panic!("resume must reject a participant double-booked by the shift"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        DecisionError::Operation(OperationError::ParticipantBusy {
            character: leader,
            operation: follow_up,
        })
    );
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("paused operation should persist")
            .status(),
        OperationStatus::AwaitingDecision
    );
    validate_state(&state).expect("rejected resume state should validate");
    validate_invariants(&state);
}

#[test]
fn police_arrival_abort_persists_decision_provenance_and_after_action_artifacts() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    run_tick(&registry, &mut state);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("operation should exist")
        .responsible_organization();
    // The post-entry arrival pauses the operation pending leadership direction.
    loop {
        let outcome = run_tick(&registry, &mut state);
        assert!(outcome.resolved_operations.is_empty());
        if !outcome.decision_requests.is_empty() {
            break;
        }
    }
    let decision_id = state
        .decisions()
        .pending_for_operation(operation)
        .expect("arrival decision should be pending");
    let decision_summary = state
        .decisions()
        .get_decision(decision_id)
        .expect("decision should persist")
        .summary()
        .to_owned();
    assert!(decision_summary.contains("Leadership direction is required"));

    let outcome = validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Abort,
    )
    .expect("abort response should validate")
    .commit(&mut state)
    .expect("abort response should atomically terminate the operation");
    assert!(outcome.recruitment_attempt.is_none());

    let aborted_at = state.now();
    let record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should persist");
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert!(record.resolution().is_none());
    let abort = record
        .abort_record()
        .expect("decision abort should persist abort provenance");
    assert_eq!(abort.aborted_at(), aborted_at);
    assert_eq!(abort.phase(), OperationAbortPhase::AwaitingDecision);
    assert_eq!(abort.cause(), OperationAbortCause::Decision(decision_id));
    let decision_record = state
        .decisions()
        .get_decision(decision_id)
        .expect("resolved decision should persist");
    let resolution = decision_record
        .resolution()
        .expect("abort decision should be resolved");
    assert_eq!(resolution.response(), DecisionResponse::Abort);
    assert_eq!(resolution.resolved_at(), abort.aborted_at());

    let artifacts = abort
        .artifacts()
        .expect("abort after execution began should create after-action artifacts");
    let information = state
        .intelligence()
        .get_information(artifacts.information())
        .expect("abort information should persist");
    assert!(information.summary().contains(decision_summary.as_str()));
    let report = state
        .reports()
        .get_report(artifacts.report())
        .expect("abort report should persist");
    assert_eq!(report.entries()[0].summary, information.summary());
    assert!(report.entries()[0]
        .entities
        .contains(&EntityRef::DecisionRequest(decision_id)));
    let history = state
        .history()
        .get_event(artifacts.history_event())
        .expect("abort history should persist");
    assert_eq!(history.summary(), information.summary());
    assert!(history
        .entities()
        .contains(&EntityRef::DecisionRequest(decision_id)));

    for _ in 0..30 {
        let tick = run_tick(&registry, &mut state);
        assert!(!tick.resolved_operations.contains(&operation));
    }
    validate_state(&state).expect("decision-aborted operation state should validate");
    validate_invariants(&state);
}

#[test]
fn save_round_trip_preserves_deterministic_operation_resolution() {
    let (registry, mut original, _organization, operation) = make_operation_fixture();
    for _ in 0..20 {
        run_tick(&registry, &mut original);
    }
    assert_eq!(
        original
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .resolution_due_at(),
        Some(SimTime::from_minutes(21))
    );
    let envelope =
        build_save(&registry, &original).expect("pre-resolution operation state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored =
        restore_save(&registry, decoded).expect("pre-resolution operation save should restore");

    let original_tick = run_tick(&registry, &mut original);
    let restored_tick = run_tick(&registry, &mut restored);
    assert_eq!(original_tick, restored_tick);
    assert_eq!(original_tick.resolved_operations, vec![operation]);
    let original_resolution = original
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("original operation should resolve");
    let restored_resolution = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("restored operation should resolve");
    assert_eq!(
        original_resolution.objective_outcome(),
        restored_resolution.objective_outcome()
    );
    assert_eq!(
        original_resolution.execution_margin(),
        restored_resolution.execution_margin()
    );
    assert_eq!(original_resolution.factors(), restored_resolution.factors());
    assert_eq!(
        original_resolution.exposure().level(),
        restored_resolution.exposure().level()
    );
    assert_eq!(
        original_resolution.exposure().score(),
        restored_resolution.exposure().score()
    );
    assert_eq!(
        original_resolution.exposure().factors(),
        restored_resolution.exposure().factors()
    );
    assert_eq!(
        original_resolution.after_action_report(),
        restored_resolution.after_action_report()
    );
    let original_report = original
        .reports()
        .get_report(original_resolution.after_action_report())
        .expect("original after-action report should persist");
    let restored_report = restored
        .reports()
        .get_report(restored_resolution.after_action_report())
        .expect("restored after-action report should persist");
    assert_eq!(original_report.title(), restored_report.title());
    assert_eq!(
        original_report.entries()[0].summary,
        restored_report.entries()[0].summary
    );
    validate_state(&restored).expect("deterministically restored resolution should validate");
    validate_invariants(&restored);
}

#[test]
fn same_minute_operation_after_action_is_included_in_due_executive_brief() {
    let (registry, mut state, organization, operation) = make_operation_fixture();
    designate_player_organization(&mut state, organization)
        .expect("operation organization should be eligible as the player organization");

    state.advance_clock(SimDuration::from_minutes(1_419));
    let start_tick = run_tick(&registry, &mut state);
    assert_eq!(start_tick.now, SimTime::from_minutes(1_420));
    assert_eq!(start_tick.started_operations, vec![operation]);
    assert!(start_tick.executive_brief.is_none());
    for _ in 0..19 {
        let tick = run_tick(&registry, &mut state);
        assert!(tick.resolved_operations.is_empty());
        assert!(tick.executive_brief.is_none());
    }

    let boundary_tick = run_tick(&registry, &mut state);
    assert_eq!(boundary_tick.now, SimTime::from_minutes(1_440));
    assert_eq!(boundary_tick.resolved_operations, vec![operation]);
    let executive_brief = boundary_tick
        .executive_brief
        .expect("daily boundary should synthesize an executive brief");
    let resolution = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("operation should resolve at the daily boundary");
    assert!(resolution.after_action_report() < executive_brief);
    let after_action = state
        .reports()
        .get_report(resolution.after_action_report())
        .expect("same-minute after-action report should persist");
    let executive = state
        .reports()
        .get_report(executive_brief)
        .expect("same-minute executive brief should persist");
    assert!(executive.entries().iter().any(|entry| {
        entry.attention == AttentionClass::Notable
            && entry.summary == after_action.entries()[0].summary
            && entry.entities.contains(&EntityRef::Operation(operation))
    }));
    validate_state(&state).expect("same-minute synthesis state should validate");
    validate_invariants(&state);
}

#[test]
fn completed_operation_remains_valid_after_leader_leaves_organization() {
    let (registry, mut state, _organization, operation) = make_operation_fixture();
    for _ in 0..21 {
        run_tick(&registry, &mut state);
    }
    let leader = state
        .operations()
        .get_operation(operation)
        .expect("completed operation should persist")
        .leader();
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("completed operation should persist")
            .status(),
        OperationStatus::Completed
    );

    validate_reassign_character(&state, leader, None, None)
        .expect("completed operation should no longer bind leader membership")
        .commit(&mut state)
        .expect("leader reassignment should commit after operation completion");
    validate_state(&state).expect("historical operation should survive leader reassignment");
    validate_invariants(&state);

    let envelope = build_save(&registry, &state)
        .expect("historical operation with reassigned leader should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded)
        .expect("historical operation with reassigned leader should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .expect("restored historical operation should persist")
            .status(),
        OperationStatus::Completed
    );
    validate_invariants(&restored);
}

#[test]
fn fresh_complete_intelligence_improves_execution_and_reduces_exposure() {
    let (registry, mut state, operation) = make_intelligence_operation_fixture();
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due prepared operation should resolve deterministically");
    assert_eq!(plan.outcome.factors.intelligence_quality().value(), 99);
    assert_eq!(plan.outcome.factors.intelligence_adjustment(), -13);
    // The fixture business sits in a police-presence-30 ward; Intimidation's authored
    // pressure weight is 25, so difficulty carries 30 * 25 / 100 = 7 extra pressure.
    assert_eq!(plan.outcome.execution_margin, 50 - 7);
    assert_eq!(plan.outcome.exposure.factors.intelligence_mitigation(), 19);
    // Baseline score 33 plus the same ward's police-observation contribution
    // (35 weight * presence 30 / 100 = 10) that an organization target never had.
    assert_eq!(plan.outcome.exposure.score, 43);
    assert_eq!(plan.outcome.exposure.level, OperationExposureLevel::Trace);

    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("fresh causal resolution plan should validate")
        .commit(&mut state)
        .expect("prepared causal resolution should commit");
    validate_state(&state).expect("intelligence-backed operation state should validate");
    validate_invariants(&state);
}

#[test]
fn successful_cash_take_holds_proceeds_until_canonical_deposit() {
    let (registry, mut state, organization, operation) = make_operation_fixture();
    for minute in 1..=25_u64 {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            assert_eq!(outcome.resolved_operations, vec![operation]);
            assert_eq!(outcome.now, SimTime::from_minutes(minute));
            break;
        }
    }
    let record = state
        .operations()
        .get_operation(operation)
        .expect("resolved operation should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    let resolution = record.resolution().expect("completion should persist");
    let proceeds = resolution
        .cash_proceeds()
        .expect("an achieved intimidation racket must hold its protection take");
    assert!(proceeds.amount().cents() > 0);
    let after_action = state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("completion should persist after-action information");
    assert!(after_action.summary().contains("remains undeposited"));

    let cash_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .expect("street cash account should validate");
    let settlement_account = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("settlement account should validate");

    let deposit = validate_deposit_operation_cash(
        &state,
        CashDispositionDraft {
            operation,
            cash_account,
            settlement_account,
        },
    )
    .expect("held cash should be depositable into an organization account");
    let outcome = deposit
        .commit(&mut state)
        .expect("cash deposit should commit atomically");
    assert_eq!(outcome.deposited_value, proceeds.amount());
    assert_eq!(
        state
            .finance()
            .get_account(cash_account)
            .expect("cash account should persist")
            .balance(),
        proceeds.amount()
    );
    assert_eq!(
        state
            .finance()
            .get_account(settlement_account)
            .expect("settlement account should persist")
            .balance(),
        Money::from_cents(-proceeds.amount().cents())
    );

    assert!(matches!(
      validate_deposit_operation_cash(
        &state,
        CashDispositionDraft {
          operation,
          cash_account,
          settlement_account,
        },
      ),
      Err(PropertyDispositionError::AlreadyDeposited(found)) if found == operation
    ));
    validate_state(&state).expect("cash disposition state should remain valid");
    validate_invariants(&state);

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("cash disposition state should save"),
    )
    .expect("cash disposition state should restore");
    let restored_disposition = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.cash_disposition())
        .expect("restored state should preserve the cash disposition");
    assert_eq!(restored_disposition.realized_value(), proceeds.amount());
    assert_eq!(restored_disposition.transaction(), outcome.transaction);
    validate_invariants(&restored);
}

#[test]
fn successful_extraction_frees_detained_member_through_canonical_release() {
    let registry = build_registry();
    let mut state = AppState::new(0x0E77_1933);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Extraction Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Extraction Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let mut make_member = |name: &str, supervisor: Option<CharacterId>| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(crew),
                supervisor,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (
                        CapabilityKind::Management,
                        Rating::try_new(99).expect("fixture rating should be valid"),
                    ),
                    (
                        CapabilityKind::Driving,
                        Rating::try_new(99).expect("fixture rating should be valid"),
                    ),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("member should validate")
    };
    let leader = make_member("Extraction Leader", None);
    let driver = make_member("Extraction Driver", Some(leader));
    let detainee = make_member("Detained Member", Some(leader));

    // Put the member in custody through the canonical evidence-backed arrest path.
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Detention test case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(detainee)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("evidence should validate")
    .commit(&mut state)
    .expect("evidence should commit");
    let arrest = crate::legal::arrest_system::validate_arrest(
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("evidence-backed arrest should validate")
    .commit(&mut state)
    .expect("arrest should commit");

    // A free-detainee objective against someone not in custody must be rejected.
    let free_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Impossible extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: leader },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect_err("extraction requires a detained target");
    assert!(matches!(
      free_error,
      OperationError::TargetNotDetained(character) if character == leader
    ));

    let extraction = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Bust-out extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect("detained-target extraction should validate")
    .commit(&mut state)
    .expect("extraction should commit");

    // A second live extraction against the same custody is rejected: it could only
    // resolve after the first freed the target and would then be uncommittable.
    let duplicate_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Duplicate extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect_err("a detainee supports exactly one live extraction plan");
    assert!(matches!(
      duplicate_error,
      OperationError::DetaineeAlreadyTargeted {
        character,
        operation
      } if character == detainee && operation == extraction
    ));
    let operation_count = state.operations().operations().count();
    loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            break;
        }
    }
    let record = state
        .operations()
        .get_operation(extraction)
        .expect("extraction should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    assert_ne!(
        record
            .resolution()
            .map(|resolution| resolution.objective_outcome()),
        Some(OperationObjectiveOutcome::Failed),
        "a fully capable crew must not fail the extraction"
    );
    let released = state
        .legal()
        .get_arrest(arrest)
        .expect("arrest should persist");
    assert_eq!(
        released.status(),
        crate::legal::ArrestStatus::Released,
        "successful extraction must release the detainee through canonical custody"
    );
    assert!(released.released_at().is_some());
    assert_eq!(
        state.operations().operations().count(),
        operation_count,
        "the rejected duplicate extraction must not create a record"
    );
    validate_state(&state).expect("post-extraction state should remain valid");
    validate_invariants(&state);
}

#[test]
fn witnessed_exposure_registers_owner_witness_whose_interview_becomes_case_testimony() {
    let registry = build_registry();
    let mut state = AppState::new(0x0B1E_1933);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Pipeline Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Pipeline Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let owner = insert_character(
        &mut state,
        CharacterDraft {
            name: "Shopkeeper Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("owner witness should validate");
    let business = make_fixture_business_with_owner(
        &registry,
        &mut state,
        "Witnessed Emporium",
        BusinessOwner::Character(owner),
    );
    // Give the precinct jurisdiction over the target's ward so exposure opens a case.
    let neighborhood = state
        .world()
        .get_business(business)
        .expect("fixture business should exist")
        .neighborhood();
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(80)
                .expect("fixture case priority should validate"),
        },
    )
    .expect("precinct jurisdiction should validate")
    .commit(&mut state)
    .expect("precinct jurisdiction should commit");
    // The precinct needs a capable detective so the case can be staffed and interviews
    // can be conducted.
    let _detective = insert_character(
        &mut state,
        CharacterDraft {
            name: "Pipeline Detective".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(99).expect("fixture rating should be valid"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Pipeline Crew Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(99).expect("fixture rating should be valid"),
                ),
                (
                    CapabilityKind::Intimidation,
                    Rating::try_new(99).expect("fixture rating should be valid"),
                ),
            ]),
            // A maximal Safety drive makes the detained leader maximally susceptible to
            // the fear-of-prison informant flip.
            drives: BTreeMap::from([(
                DriveKind::Safety,
                Rating::try_new(99).expect("fixture rating should be valid"),
            )]),
            traits: BTreeSet::new(),
        },
    )
    .expect("leader should validate");

    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Loud protection shakedown".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("intimidation operation should validate")
    .commit(&mut state)
    .expect("intimidation operation should commit");
    loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            break;
        }
    }
    let record = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist");
    assert_eq!(record.status(), OperationStatus::Completed);
    let exposure = record
        .resolution()
        .expect("resolution should persist")
        .exposure();
    let suspect = exposure.identified_character();
    assert!(
        exposure.level() as i32 >= 2,
        "an intimidating shakedown at a quiet ward must at least be witnessed"
    );

    // The character-owned business's owner is the case's named witness.
    let investigation = exposure
        .investigation()
        .expect("a witnessed incident must open an investigation when jurisdiction exists");
    let witnesses: Vec<_> = state
        .legal()
        .case_witnesses_for_investigation(investigation)
        .map(|witness| witness.witness())
        .collect();
    assert_eq!(witnesses, vec![owner]);

    // Witness pressure against that same witness is now authorizable while the crew's
    // exposed leader is still free: authorizing it before testimony lands also books
    // him, which legally blocks the institution from arresting mid-operation.
    let _pressure = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Quiet the shopkeeper".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(owner),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::from_minutes(1),
        },
    )
    .expect("pressure against a named witness should validate")
    .commit(&mut state)
    .expect("pressure operation should commit");

    // Drive the pipeline: staffing schedules the interview whose success records real
    // testimony through the witness-statement path; the pressure operation resolves and
    // degrades cooperation one step; once the leader's crew work is terminal and his
    // case holds corroborated testimony, the precinct arrests him through the canonical
    // validated path.
    let suspect = suspect.expect("an identifying shakedown must expose a specific participant");
    let arrested_at = loop {
        let outcome = run_tick(&registry, &mut state);
        let has_statement = state
            .legal()
            .case_witness_for(investigation, owner)
            .is_some_and(|witness| !witness.statements().is_empty());
        if let Some(arrest) = state.legal().active_arrest_for_character(suspect) {
            assert!(
                has_statement,
                "custody must not precede the corroborating witness statement"
            );
            break arrest.id();
        }
        assert!(
            outcome.now.as_minutes() < 20_000,
            "the pipeline should reach custody well before this bound"
        );
    };
    let pressured = state
        .legal()
        .case_witness_for(investigation, owner)
        .expect("witness record should persist");
    assert_eq!(pressured.statements().len(), 1);
    assert_eq!(
        pressured.cooperation(),
        crate::legal::WitnessCooperation::Hostile,
        "successful witness pressure must have moved cooperation off reluctant"
    );
    let arrest_record = state
        .legal()
        .get_arrest(arrested_at)
        .expect("arrest persists");
    assert_eq!(arrest_record.character(), suspect);
    assert_eq!(arrest_record.authority(), police);
    assert!(
        arrest_record.evidence().len() >= 2,
        "custody requires corroboration beyond a single item"
    );

    // One authored cadence window into custody, the detained member faces their single
    // recruitment decision. With a maximal Safety drive the fear-of-prison chance is
    // high, and this seed's deterministic roll lands inside it. The flipped member
    // personally knows how their crew's job ended (every participant holds that
    // after-action knowledge), so the same pipeline pass discloses it into the
    // handler's case about that operation as InformantStatement evidence.
    let decision_minute =
        arrest_record.arrested_at().as_minutes() + RECRUITMENT_DECISION_OFFSET_MINUTES;
    loop {
        let outcome = run_tick(&registry, &mut state);
        let flipped = state
            .legal()
            .active_informant_for(suspect, police)
            .is_some();
        let disclosed = state
            .legal()
            .get_investigation(investigation)
            .expect("case should persist")
            .evidence()
            .iter()
            .filter_map(|id| state.legal().get_evidence(*id))
            .any(|evidence| evidence.kind() == EvidenceKind::InformantStatement);
        if flipped && disclosed {
            break;
        }
        assert!(
            !flipped || outcome.informant_disclosures.is_empty(),
            "a qualifying informant disclosure must record immediately"
        );
        assert!(
            outcome.now.as_minutes() < decision_minute + 10,
            "the recruitment draw must happen exactly at the cadence minute"
        );
    }
    let case_has_informant_evidence = state
        .legal()
        .get_investigation(investigation)
        .expect("case should persist")
        .evidence()
        .iter()
        .filter_map(|id| state.legal().get_evidence(*id))
        .any(|evidence| evidence.kind() == EvidenceKind::InformantStatement);
    assert!(case_has_informant_evidence);

    validate_state(&state).expect("witness pipeline state should remain valid");
    validate_invariants(&state);
}

#[test]
fn control_plane_surveillance_targets_proxy_to_their_owner_footprint() {
    let (_registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Open Case Watch".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(operation)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");

    // Surveillance of a control-plane record must attribute to the world footprint of its
    // owner — the case runs where its authority has jurisdiction — so exposure and police
    // response for such operations cannot silently vanish into "no neighborhood".
    let attributed = resolve_target_neighborhoods(
        &state,
        vec![
            EntityRef::Operation(operation),
            EntityRef::Investigation(investigation),
        ],
    );
    assert_eq!(
        attributed,
        BTreeSet::from([neighborhood]),
        "operation and investigation targets attribute via their owning organization"
    );
}

#[test]
fn neighborhood_exposure_opens_jurisdiction_case_and_survives_save_round_trip() {
    let (registry, mut original, police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    for _ in 0..45 {
        let tick = run_tick(&registry, &mut original);
        assert!(tick.resolved_operations.is_empty());
    }
    assert_eq!(original.now(), SimTime::from_minutes(45));
    let envelope = build_save(&registry, &original)
        .expect("pre-exposure-resolution operation state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&registry, decoded)
        .expect("pre-exposure-resolution operation save should restore");

    let original_tick = run_tick(&registry, &mut original);
    let restored_tick = run_tick(&registry, &mut restored);
    assert_eq!(original_tick, restored_tick);
    assert_eq!(original_tick.resolved_operations, vec![operation]);
    for state in [&original, &restored] {
        let resolution = state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("exposed operation should resolve");
        assert!(matches!(
            resolution.exposure().level(),
            OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
        ));
        let investigation_id = resolution
            .exposure()
            .investigation()
            .expect("jurisdictional exposure should open an investigation");
        let investigation = state
            .legal()
            .get_investigation(investigation_id)
            .expect("operation investigation should persist");
        assert_eq!(investigation.owner(), police);
        assert_eq!(resolution.exposure().evidence().len(), 1);
        let legal_activity_information = resolution
            .legal_activity_information()
            .expect("jurisdictional exposure should create player legal-activity knowledge");
        let legal_activity = state
            .intelligence()
            .get_information(legal_activity_information)
            .expect("player legal-activity information should persist");
        assert_eq!(legal_activity.topic(), InformationTopic::LegalActivity);
        assert_eq!(legal_activity.subject(), EntityRef::Operation(operation));
        assert!(legal_activity
            .summary()
            .contains("produced a police investigation"));
        let evidence_id = *resolution
            .exposure()
            .evidence()
            .iter()
            .next()
            .expect("operation exposure should persist one evidence record");
        let evidence = state
            .legal()
            .get_evidence(evidence_id)
            .expect("operation evidence should persist");
        assert_eq!(evidence.origin(), Some(EntityRef::Operation(operation)));
        assert_eq!(
            state
                .legal()
                .evidence_from_origin(EntityRef::Operation(operation))
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![evidence_id]
        );
        validate_state(state).expect("exposure-linked legal state should validate");
        validate_invariants(state);
    }
    let original_exposure = original
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("original exposure should resolve")
        .exposure();
    let restored_exposure = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("restored exposure should resolve")
        .exposure();
    assert_eq!(original_exposure.level(), restored_exposure.level());
    assert_eq!(original_exposure.score(), restored_exposure.score());
    assert_eq!(original_exposure.factors(), restored_exposure.factors());
    assert_eq!(
        original_exposure.investigation(),
        restored_exposure.investigation()
    );
    assert_eq!(original_exposure.evidence(), restored_exposure.evidence());
    assert_eq!(
        original
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information()),
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information())
    );
}

#[test]
fn exposed_operation_without_jurisdiction_creates_no_implicit_case() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    for _ in 0..46 {
        run_tick(&registry, &mut state);
    }
    let exposure = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("exposed operation should resolve")
        .exposure();
    assert!(matches!(
        exposure.level(),
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ));
    assert_eq!(exposure.investigation(), None);
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.legal_activity_information()),
        None
    );
    assert!(exposure.evidence().is_empty());
    assert_eq!(
        state
            .legal()
            .evidence_from_origin(EntityRef::Operation(operation))
            .count(),
        0
    );
    validate_state(&state).expect("unrouted exposure should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn patrol_presence_controls_persisted_police_response_delay() {
    let (low_registry, mut low_state, low_police, low_neighborhood, low_operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &low_state,
        PatrolDeploymentDraft {
            organization: low_police,
            neighborhood: low_neighborhood,
            windows: vec![PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(0).expect("zero patrol presence should validate"),
            )
            .expect("fixture patrol window should validate")],
        },
    )
    .expect("zero-presence patrol should validate")
    .commit(&mut low_state)
    .expect("zero-presence patrol should commit");
    let low_start = run_tick(&low_registry, &mut low_state);
    assert_eq!(low_start.started_operations, vec![low_operation]);
    let low_response_id = low_state
        .operations()
        .get_operation(low_operation)
        .and_then(|record| record.police_response())
        .expect("observable burglary should dispatch a response");
    let low_response = low_state
        .legal()
        .get_police_response(low_response_id)
        .expect("low-presence response should persist");
    assert_eq!(low_response.response_presence().value(), 0);
    assert_eq!(
        low_response.arrival_due_at().as_minutes() - low_response.dispatched_at().as_minutes(),
        12
    );

    let (high_registry, mut high_state, high_police, high_neighborhood, high_operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &high_state,
        PatrolDeploymentDraft {
            organization: high_police,
            neighborhood: high_neighborhood,
            windows: vec![PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(100).expect("full patrol presence should validate"),
            )
            .expect("fixture patrol window should validate")],
        },
    )
    .expect("full-presence patrol should validate")
    .commit(&mut high_state)
    .expect("full-presence patrol should commit");
    let high_start = run_tick(&high_registry, &mut high_state);
    assert_eq!(high_start.started_operations, vec![high_operation]);
    let high_response_id = high_state
        .operations()
        .get_operation(high_operation)
        .and_then(|record| record.police_response())
        .expect("observable burglary should dispatch a response");
    let high_response = high_state
        .legal()
        .get_police_response(high_response_id)
        .expect("high-presence response should persist");
    assert_eq!(high_response.response_presence().value(), 100);
    assert_eq!(
        high_response.arrival_due_at().as_minutes() - high_response.dispatched_at().as_minutes(),
        3
    );

    validate_state_against_registry(&low_registry, &low_state)
        .expect("low-presence response state should match authored content");
    validate_state_against_registry(&high_registry, &high_state)
        .expect("high-presence response state should match authored content");
    validate_invariants(&low_state);
    validate_invariants(&high_state);
}

#[test]
fn police_arrival_before_entry_executes_standing_abort_contingency() {
    let (registry, mut state, police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
        );
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let response_id = operation_record
        .police_response()
        .expect("high-observation burglary should dispatch police response");
    let entry_at = operation_record
        .entry_at()
        .expect("burglary should have an authored entry milestone");

    let mut arrival_tick = None;
    while state.now() < entry_at {
        let outcome = run_tick(&registry, &mut state);
        if outcome.arrived_police_responses.contains(&response_id) {
            arrival_tick = Some(outcome.now);
            break;
        }
    }
    let arrived_at = arrival_tick.expect("police response should arrive before burglary entry");
    assert!(arrived_at < entry_at);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should persist");
    assert_eq!(operation_record.status(), OperationStatus::Aborted);
    let abort = operation_record
        .abort_record()
        .expect("standing police contingency should create abort history");
    assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
    assert_eq!(
        abort.cause(),
        OperationAbortCause::PoliceArrival(response_id)
    );
    assert!(operation_record.resolution().is_none());
    assert_eq!(
        state
            .legal()
            .get_police_response(response_id)
            .and_then(|response| response.arrived_at()),
        Some(arrived_at)
    );
    let mut participants = operation_record
        .roles()
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    participants.insert(operation_record.leader());
    for participant in participants {
        let pressure: Vec<_> = state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Character(participant),
                InformationTopic::PoliceActivity,
            )
            .collect();
        assert_eq!(pressure.len(), 1);
        assert_eq!(
            pressure[0].source_kind(),
            InformationSourceKind::DirectObservation
        );
        assert_eq!(
            pressure[0].source_entity(),
            Some(EntityRef::Organization(police))
        );
        assert_eq!(pressure[0].subject(), EntityRef::Character(participant));
        assert_eq!(pressure[0].observed_at(), arrived_at);
        assert_eq!(pressure[0].reliability(), Reliability::DirectAccess);
        assert_eq!(pressure[0].specificity(), Specificity::Precise);
    }
    validate_state(&state).expect("police-contingency abort state should remain valid");
    validate_state_against_registry(&registry, &state)
        .expect("police-contingency abort should match authored content");
    validate_invariants(&state);
}

#[test]
fn post_entry_police_arrival_raises_provenance_backed_decision() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(0).expect("zero patrol presence should validate"),
            )
            .expect("fixture patrol window should validate")],
        },
    )
    .expect("zero-presence patrol should validate")
    .commit(&mut state)
    .expect("zero-presence patrol should commit");

    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let response_id = operation_record
        .police_response()
        .expect("observable burglary should dispatch police response");
    let entry_at = operation_record
        .entry_at()
        .expect("burglary should have an authored entry milestone");
    let response_due = state
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    assert!(response_due > entry_at);

    let arrival_outcome = loop {
        let outcome = run_tick(&registry, &mut state);
        if outcome.arrived_police_responses.contains(&response_id) {
            break outcome;
        }
    };
    assert_eq!(arrival_outcome.now, response_due);
    assert_eq!(arrival_outcome.arrived_police_responses, vec![response_id]);
    assert_eq!(arrival_outcome.decision_requests.len(), 1);
    assert!(arrival_outcome.resolved_operations.is_empty());

    let decision_id = arrival_outcome.decision_requests[0].decision;
    let decision = state
        .decisions()
        .get_decision(decision_id)
        .expect("response decision should persist");
    assert_eq!(decision.requested_at(), response_due);
    assert!(matches!(
      decision.context(),
      DecisionContext::OperationPoliceArrival {
        operation: decision_operation,
        response: decision_response,
      } if decision_operation == operation && decision_response == response_id
    ));
    assert!(decision.summary().contains("response reached"));
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("decision-blocked operation should persist");
    assert_eq!(operation_record.status(), OperationStatus::AwaitingDecision);
    assert_eq!(
        operation_record.awaiting_decision_since(),
        Some(response_due)
    );

    let organization = operation_record.responsible_organization();
    let envelope = build_save(&registry, &state)
        .expect("pending police-arrival decision should survive save validation");
    let bytes =
        bincode::serialize(&envelope).expect("police-arrival decision save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("police-arrival decision save should deserialize");
    state = restore_save(&registry, decoded)
        .expect("pending police-arrival decision should restore with provenance indexes");
    assert_eq!(
        state
            .decisions()
            .decisions_for_operation(operation)
            .filter(|candidate| candidate.id() == decision_id)
            .count(),
        1
    );
    validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Continue,
    )
    .expect("post-entry police response should allow leadership to continue")
    .commit(&mut state)
    .expect("post-entry continue should resume operation");
    let resumed = state
        .operations()
        .get_operation(operation)
        .expect("resumed operation should persist");
    assert_eq!(resumed.status(), OperationStatus::InProgress);
    assert_eq!(resumed.awaiting_decision_since(), None);
    assert_eq!(
        state
            .legal()
            .get_police_response(response_id)
            .and_then(|response| response.arrived_at()),
        Some(response_due)
    );
    // One police response must never produce two leadership decisions.
    // One police response must never produce two leadership decisions: once its arrival
    // decision exists the operation is decision-blocked, and after resolution the response
    // is no longer a freshly-due dispatch.
    let duplicate = validate_request_police_arrival_decision_on_arrival(&state, response_id)
        .expect_err("one police response must not create duplicate leadership decisions");
    assert_eq!(
        duplicate,
        DecisionError::InvalidPoliceResponseDecision {
            operation,
            response: response_id,
        }
    );
    validate_state(&state).expect("post-entry response decision state should validate");
    validate_state_against_registry(&registry, &state)
        .expect("post-entry response decision should match authored content");
    validate_invariants(&state);
}

#[test]
fn arrived_response_penalizes_continuing_operation_and_stales_prearrival_plan() {
    let (registry, mut response_state, _police, _neighborhood, response_operation) =
        make_exposed_business_operation_fixture(true);
    let (_, mut control_state, _control_police, _control_neighborhood, control_operation) =
        make_exposed_business_operation_fixture(false);
    run_tick(&registry, &mut response_state);
    run_tick(&registry, &mut control_state);

    let response_id = response_state
        .operations()
        .get_operation(response_operation)
        .and_then(|record| record.police_response())
        .expect("jurisdictional burglary should dispatch response");
    response_state.advance_clock(SimDuration::from_minutes(45));
    control_state.advance_clock(SimDuration::from_minutes(45));
    let stale_plan = decide_operation_resolution(
        &registry,
        &response_state,
        response_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should be plannable before response processing");
    assert!(!stale_plan.outcome.factors.police_response_arrived());
    let response_outcome =
        crate::operations::police_response_integration::apply_due_police_response_arrivals(
            &mut response_state,
        )
        .expect("due response should process");
    assert_eq!(response_outcome.arrived, vec![response_id]);
    assert!(response_outcome.decisions.is_empty());
    let stale_error =
        match validate_operation_resolution_plan(&registry, &response_state, stale_plan) {
            Ok(_) => panic!("response arrival must invalidate a pre-arrival resolution plan"),
            Err(error) => error,
        };
    assert_eq!(
        stale_error,
        OperationResolutionError::StalePoliceResponseContext {
            operation: response_operation,
        }
    );

    let response_plan = decide_operation_resolution(
        &registry,
        &response_state,
        response_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("arrived-response operation should re-plan");
    let control_plan = decide_operation_resolution(
        &registry,
        &control_state,
        control_operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("unrouted control operation should plan");
    assert!(response_plan.outcome.factors.police_response_arrived());
    assert!(!control_plan.outcome.factors.police_response_arrived());
    let execution = registry.get_operation(OperationKind::Burglary).execution();
    assert_eq!(
        control_plan.outcome.execution_margin - response_plan.outcome.execution_margin,
        i16::from(execution.police_arrival_difficulty_penalty())
    );
    assert_eq!(
        response_plan.outcome.exposure.score - control_plan.outcome.exposure.score,
        i16::from(execution.police_arrival_exposure_penalty())
    );
    validate_operation_resolution_plan(&registry, &response_state, response_plan)
        .expect("response-aware resolution should validate")
        .commit(&mut response_state)
        .expect("response-aware resolution should commit");
    validate_state_against_registry(&registry, &response_state)
        .expect("response-aware completion should validate against registry");
    validate_invariants(&response_state);
}

#[test]
fn police_response_arrival_is_deterministic_across_save_round_trip() {
    let (registry, mut original, _police, _neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    run_tick(&registry, &mut original);
    let response_id = original
        .operations()
        .get_operation(operation)
        .and_then(|record| record.police_response())
        .expect("jurisdictional burglary should dispatch response");
    let due_at = original
        .legal()
        .get_police_response(response_id)
        .expect("response should persist")
        .arrival_due_at();
    while original.now() + SimDuration::ONE_MINUTE < due_at {
        let outcome = run_tick(&registry, &mut original);
        assert!(outcome.arrived_police_responses.is_empty());
    }
    let envelope =
        build_save(&registry, &original).expect("pre-arrival police response state should save");
    let bytes = bincode::serialize(&envelope).expect("response save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("response save should deserialize");
    let mut restored = restore_save(&registry, decoded).expect("response save should restore");

    let original_tick = run_tick(&registry, &mut original);
    let restored_tick = run_tick(&registry, &mut restored);
    assert_eq!(original_tick.arrived_police_responses, vec![response_id]);
    assert_eq!(restored_tick.arrived_police_responses, vec![response_id]);
    assert_eq!(
        original
            .legal()
            .get_police_response(response_id)
            .and_then(|record| record.arrived_at()),
        restored
            .legal()
            .get_police_response(response_id)
            .and_then(|record| record.arrived_at())
    );
    validate_state(&restored).expect("restored police-response state should validate");
    validate_invariants(&restored);
}

#[test]
fn resolution_plan_snapshots_patrol_versions_and_uses_explicit_schedule_gaps() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let deployment = validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![PatrolWindow::try_new(
                DayMinute::try_new(0).expect("fixture minute should validate"),
                1_440,
                Rating::try_new(70).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate")],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");
    let stale_plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should resolve against active patrol state");
    assert_eq!(
        stale_plan
            .outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(70)
    );

    validate_revise_patrol_deployment(
        &state,
        deployment,
        vec![PatrolWindow::try_new(
            DayMinute::try_new(600).expect("fixture minute should validate"),
            120,
            Rating::try_new(80).expect("fixture patrol rating should validate"),
        )
        .expect("fixture patrol window should validate")],
    )
    .expect("patrol revision should validate")
    .commit(&mut state)
    .expect("patrol revision should commit");

    let error = validate_operation_resolution_plan(&registry, &state, stale_plan)
        .err()
        .expect("patrol revision must stale an operation resolution plan");
    assert_eq!(
        error,
        OperationResolutionError::StalePoliceDeploymentContext { operation }
    );

    let fresh_plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("operation should re-plan against revised patrol schedule");
    assert_eq!(
        fresh_plan
            .outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(0)
    );
    assert_eq!(
        fresh_plan
            .outcome
            .exposure
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(0)
    );
    validate_operation_resolution_plan(&registry, &state, fresh_plan)
        .expect("fresh patrol-aware resolution plan should validate")
        .commit(&mut state)
        .expect("fresh patrol-aware resolution should commit");
    validate_state(&state).expect("patrol-aware operation resolution should remain valid");
    validate_invariants(&state);
}

#[test]
fn operation_resolution_uses_time_weighted_patrol_presence_across_execution_window() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    validate_establish_patrol_deployment(
        &state,
        PatrolDeploymentDraft {
            organization: police,
            neighborhood,
            windows: vec![PatrolWindow::try_new(
                DayMinute::try_new(45).expect("fixture patrol minute should validate"),
                60,
                Rating::try_new(90).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate")],
        },
    )
    .expect("patrol deployment should validate")
    .commit(&mut state)
    .expect("patrol deployment should commit");

    let start = run_tick(&registry, &mut state);
    assert_eq!(start.now, SimTime::from_minutes(1));
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));

    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due operation should resolve across its whole execution window");
    assert_eq!(
        plan.outcome
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(2)
    );
    assert_eq!(
        plan.outcome
            .exposure
            .factors
            .target_police_presence()
            .map(Rating::value),
        Some(2)
    );
    assert!(!plan
        .narrative
        .summary
        .contains("High local police presence materially increased execution pressure."));
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("time-weighted patrol plan should validate")
        .commit(&mut state)
        .expect("time-weighted patrol resolution should commit");
    validate_state(&state).expect("time-weighted patrol state should validate");
    validate_invariants(&state);
}

#[test]
fn property_acquisition_persists_estimated_held_value_with_partial_recovery() {
    let (registry, mut achieved_state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let start = run_tick(&registry, &mut achieved_state);
    assert_eq!(start.started_operations, vec![operation]);
    achieved_state.advance_clock(SimDuration::from_minutes(45));
    let mut partial_state = achieved_state.clone();

    let achieved_plan = decide_operation_resolution(
        &registry,
        &achieved_state,
        operation,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("favorable property operation should resolve");
    assert_eq!(
        achieved_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    let achieved_proceeds = achieved_plan
        .outcome
        .property_proceeds_plan
        .proceeds
        .expect("achieved property acquisition should create held proceeds");
    assert_eq!(achieved_proceeds.estimated_value().cents(), 56_400);
    assert!(achieved_plan
        .narrative
        .summary
        .contains("estimated held value of $564.00"));
    assert!(achieved_plan
        .narrative
        .summary
        .contains("remains unliquidated"));
    validate_operation_resolution_plan(&registry, &achieved_state, achieved_plan)
        .expect("achieved property proceeds should validate")
        .commit(&mut achieved_state)
        .expect("achieved property proceeds should commit");
    assert_eq!(
        achieved_state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.property_proceeds())
            .map(|proceeds| proceeds.estimated_value().cents()),
        Some(56_400)
    );
    let organization = achieved_state
        .operations()
        .get_operation(operation)
        .expect("completed property operation should persist")
        .responsible_organization();
    let financial_report = validate_organization_financial_report(
        &achieved_state,
        organization,
        SimTime::ZERO,
        achieved_state.now(),
    )
    .expect("held property should integrate into organization financial reporting")
    .commit(&mut achieved_state)
    .expect("held property financial report should commit");
    let report = achieved_state
        .reports()
        .get_report(financial_report)
        .expect("organization financial report should persist");
    assert!(report.entries()[0].summary.contains(
        "Held operation property at period end: 1 operation(s), estimated value $564.00"
    ));
    assert!(report.entries().iter().any(|entry| {
        entry.entities.contains(&EntityRef::Operation(operation))
            && entry.summary.contains("estimated held value of $564.00")
    }));

    let (resale_venue, cash_account, settlement_account) = insert_property_disposition_fixture(
        &registry,
        &mut achieved_state,
        neighborhood,
        organization,
    );
    let disposition = validate_dispose_property(
        &registry,
        &achieved_state,
        PropertyDispositionDraft {
            operation,
            venue: resale_venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("held burglary property should be disposable through a resale venue");
    assert_eq!(disposition.realized_value().cents(), 32_148);
    let disposition_outcome = disposition
        .commit(&mut achieved_state)
        .expect("property disposition should commit atomically");
    assert_eq!(disposition_outcome.realized_value.cents(), 32_148);
    assert_eq!(
        achieved_state
            .finance()
            .get_account(cash_account)
            .expect("cash account should persist")
            .balance()
            .cents(),
        32_148
    );
    assert_eq!(
        achieved_state
            .finance()
            .get_account(settlement_account)
            .expect("settlement account should persist")
            .balance()
            .cents(),
        -32_148
    );
    assert!(matches!(
      validate_dispose_property(
        &registry,
        &achieved_state,
        PropertyDispositionDraft {
          operation,
          venue: resale_venue,
          cash_account,
          settlement_account,
        },
      ),
      Err(PropertyDispositionError::AlreadyDisposed(found)) if found == operation
    ));
    let liquidated_report = validate_organization_financial_report(
        &achieved_state,
        organization,
        SimTime::ZERO,
        achieved_state.now(),
    )
    .expect("liquidated property should integrate into organization financial reporting")
    .commit(&mut achieved_state)
    .expect("liquidated property financial report should commit");
    let liquidated_report = achieved_state
        .reports()
        .get_report(liquidated_report)
        .expect("liquidation financial report should persist");
    assert!(liquidated_report.entries()[0]
        .summary
        .contains("Held operation property at period end: 0 operation(s), estimated value $0.00"));
    assert!(liquidated_report.entries()[0].summary.contains(
        "Liquidated operation property during period: 1 disposition(s), realized cash $321.48"
    ));
    assert!(liquidated_report.entries().iter().any(|entry| {
        entry.entities.contains(&EntityRef::Operation(operation))
            && entry
                .summary
                .contains("liquidated through Fixture Pawn Exchange")
            && entry.summary.contains("$321.48")
    }));
    let restored = restore_save(
        &registry,
        build_save(&registry, &achieved_state).expect("property disposition state should save"),
    )
    .expect("property disposition state should restore");
    let restored_disposition = restored
        .operations()
        .get_operation(operation)
        .and_then(|record| record.property_disposition())
        .expect("property disposition should survive save restoration");
    assert_eq!(restored_disposition.realized_value().cents(), 32_148);
    assert_eq!(restored_disposition.venue(), resale_venue);
    validate_state_against_registry(&registry, &restored)
        .expect("restored property disposition should remain registry-valid");
    validate_invariants(&restored);

    let partial_plan = decide_operation_resolution(
        &registry,
        &partial_state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("neutral property operation should resolve");
    assert_eq!(
        partial_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Partial
    );
    assert_eq!(
        partial_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .expect("partial property acquisition should create reduced held proceeds")
            .estimated_value()
            .cents(),
        22_560
    );
    validate_operation_resolution_plan(&registry, &partial_state, partial_plan)
        .expect("partial property proceeds should validate")
        .commit(&mut partial_state)
        .expect("partial property proceeds should commit");
    validate_state_against_registry(&registry, &achieved_state)
        .expect("achieved property proceeds should remain registry-valid");
    validate_state_against_registry(&registry, &partial_state)
        .expect("partial property proceeds should remain registry-valid");
    validate_invariants(&achieved_state);
    validate_invariants(&partial_state);
}

#[test]
fn repeat_scores_on_one_target_deplete_and_recover_after_the_recency_window() {
    let (registry, mut state, _police, _neighborhood, first) =
        make_exposed_business_operation_fixture(false);
    let organization = state
        .operations()
        .get_operation(first)
        .expect("first operation should persist")
        .responsible_organization();
    let (business, leader, specialist) = {
        let record = state
            .operations()
            .get_operation(first)
            .expect("first operation should persist");
        let OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } = record.objective()
        else {
            panic!("fixture operation must target business property");
        };
        let specialist = *record
            .roles()
            .get(&RoleKind::EntrySpecialist)
            .expect("fixture entry specialist should persist");
        (*business, record.leader(), specialist)
    };

    let authorize_follow_up =
        |registry: &Registry, state: &mut AppState, title: &str| -> OperationId {
            validate_authorize_operation(
                registry,
                state,
                OperationDraft {
                    title: title.to_owned(),
                    kind: OperationKind::Burglary,
                    responsible_organization: organization,
                    leader,
                    objective: OperationObjective::AcquireProperty {
                        target: EntityRef::Business(business),
                    },
                    approach: OperationApproach::Covert,
                    roles: BTreeMap::from([
                        (RoleKind::Coordinator, leader),
                        (RoleKind::EntrySpecialist, specialist),
                    ]),
                    intelligence: BTreeSet::new(),
                    constraints: Vec::new(),
                    contingencies: Vec::new(),
                    scheduled_for: state.now() + SimDuration::ONE_MINUTE,
                },
            )
            .expect("follow-up burglary should validate")
            .commit(state)
            .expect("follow-up burglary should commit")
        };

    let resolve_achieved = |registry: &Registry,
                            state: &mut AppState,
                            operation: OperationId|
     -> OperationResolutionPlan {
        run_tick(registry, state);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            registry,
            state,
            operation,
            OperationResolutionRandomness::new(12, 0),
        )
        .expect("favorable property operation should resolve");
        assert_eq!(
            plan.outcome.objective_outcome,
            OperationObjectiveOutcome::Achieved
        );
        validate_operation_resolution_plan(registry, state, plan.clone())
            .expect("resolution should validate")
            .commit(state)
            .expect("resolution should commit");
        plan
    };

    // The first take yields full value with no depletion note.
    run_tick(&registry, &mut state);
    assert_eq!(
        state.operations().get_operation(first).map(|r| r.status()),
        Some(OperationStatus::InProgress)
    );
    state.advance_clock(SimDuration::from_minutes(45));
    let first_plan = decide_operation_resolution(
        &registry,
        &state,
        first,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("first take should resolve");
    assert_eq!(
        first_plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    assert_eq!(
        first_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .as_ref()
            .expect("first take should create proceeds")
            .estimated_value()
            .cents(),
        56_400
    );
    assert!(
        !first_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );
    assert!(!first_plan.narrative.summary.contains("lighter than usual"));
    validate_operation_resolution_plan(&registry, &state, first_plan)
        .expect("first take should validate")
        .commit(&mut state)
        .expect("first take should commit");

    // An immediate second score on the same target finds partially replaced stock.
    let second = authorize_follow_up(&registry, &mut state, "Repeat burglary");
    let second_plan = resolve_achieved(&registry, &mut state, second);
    assert_eq!(
        second_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .as_ref()
            .expect("second take should create reduced proceeds")
            .estimated_value()
            .cents(),
        28_200
    );
    assert!(
        second_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );
    assert!(second_plan.narrative.summary.contains("lighter than usual"));

    // After the recency window passes the target stocks back up to full value.
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(RECENT_HIT_WINDOW_MINUTES)
            .expect("recency window must fit simulation minutes"),
    ));
    let third = authorize_follow_up(&registry, &mut state, "Recovered burglary");
    let third_plan = resolve_achieved(&registry, &mut state, third);
    assert_eq!(
        third_plan
            .outcome
            .property_proceeds_plan
            .proceeds
            .as_ref()
            .expect("recovered take should create full proceeds")
            .estimated_value()
            .cents(),
        56_400
    );
    assert!(
        !third_plan
            .outcome
            .property_proceeds_plan
            .depleted_by_recent_take
    );

    validate_state_against_registry(&registry, &state)
        .expect("depleted-take history should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn property_disposition_reporting_respects_executive_brief_window() {
    let (registry, mut state, _police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("authorized operation should persist")
        .responsible_organization();
    designate_player_organization(&mut state, organization)
        .expect("test organization should be designatable as player");
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(12, 0),
    )
    .expect("favorable property operation should resolve");
    assert_eq!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Achieved
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("property acquisition should validate")
        .commit(&mut state)
        .expect("property acquisition should commit");

    let (venue, cash_account, settlement_account) =
        insert_property_disposition_fixture(&registry, &mut state, neighborhood, organization);
    let mut same_window = state.clone();
    let mut later_window = state;

    validate_dispose_property(
        &registry,
        &same_window,
        PropertyDispositionDraft {
            operation,
            venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("same-window property disposition should validate")
    .commit(&mut same_window)
    .expect("same-window property disposition should commit");
    let delta = 1_439_u64
        .checked_sub(same_window.now().as_minutes())
        .expect("fixture should resolve before first daily brief");
    same_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("first brief delta should fit SimDuration"),
    ));
    let same_window_tick = run_tick(&registry, &mut same_window);
    let same_window_brief = same_window_tick
        .executive_brief
        .expect("first daily brief should be generated");
    let same_window_report = same_window
        .reports()
        .get_report(same_window_brief)
        .expect("same-window executive brief should persist");
    let operation_entries = same_window_report
        .entries()
        .iter()
        .filter(|entry| entry.entities.contains(&EntityRef::Operation(operation)))
        .collect::<Vec<_>>();
    assert_eq!(operation_entries.len(), 1);
    assert!(operation_entries[0]
        .summary
        .contains("it was later liquidated through Fixture Pawn Exchange for $321.48"));
    assert!(!same_window_report
        .entries()
        .iter()
        .any(|entry| entry.summary.starts_with("Property from ")));
    assert!(!same_window_report
        .entries()
        .iter()
        .any(|entry| entry.summary.contains("remains unliquidated")));

    let delta = 1_439_u64
        .checked_sub(later_window.now().as_minutes())
        .expect("fixture should resolve before first daily brief");
    later_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("first brief delta should fit SimDuration"),
    ));
    let first_tick = run_tick(&registry, &mut later_window);
    let first_brief = first_tick
        .executive_brief
        .expect("first daily brief should be generated");
    let first_report = later_window
        .reports()
        .get_report(first_brief)
        .expect("first executive brief should persist");
    assert!(first_report
        .entries()
        .iter()
        .any(|entry| entry.summary.contains("remains unliquidated")));

    validate_dispose_property(
        &registry,
        &later_window,
        PropertyDispositionDraft {
            operation,
            venue,
            cash_account,
            settlement_account,
        },
    )
    .expect("later-window property disposition should validate")
    .commit(&mut later_window)
    .expect("later-window property disposition should commit");
    let delta = 2_879_u64
        .checked_sub(later_window.now().as_minutes())
        .expect("disposition should precede the second daily brief");
    later_window.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("second brief delta should fit SimDuration"),
    ));
    let second_tick = run_tick(&registry, &mut later_window);
    let second_brief = second_tick
        .executive_brief
        .expect("second daily brief should be generated");
    let second_report = later_window
        .reports()
        .get_report(second_brief)
        .expect("second executive brief should persist");
    assert!(second_report.entries().iter().any(|entry| {
        entry.summary.starts_with("Property from ")
            && entry
                .summary
                .contains("liquidated through Fixture Pawn Exchange for $321.48")
    }));
    assert!(!second_report
        .entries()
        .iter()
        .any(|entry| entry.summary.contains("remains unliquidated")));

    validate_state_against_registry(&registry, &same_window)
        .expect("same-window brief state should remain registry-valid");
    validate_state_against_registry(&registry, &later_window)
        .expect("later-window brief state should remain registry-valid");
    validate_invariants(&same_window);
    validate_invariants(&later_window);
}

#[test]
fn resolution_token_rejects_changed_incident_jurisdiction() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due exposure operation should resolve a plan");
    assert!(matches!(
        plan.outcome.exposure.level,
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ));
    let validated = validate_operation_resolution_plan(&registry, &state, plan)
        .expect("operation incident should validate against jurisdiction version one");

    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(90)
                .expect("updated case priority should validate"),
        },
    )
    .expect("jurisdiction update should validate")
    .commit(&mut state)
    .expect("jurisdiction update should commit");

    let error = validated
        .commit(&mut state)
        .expect_err("stale incident authority snapshot must reject commit");
    assert_eq!(
        error,
        OperationResolutionError::StaleIncidentJurisdictionVersion {
            neighborhood,
            organization: police,
            expected_version: 1,
            found_version: Some(2),
        }
    );
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("stale resolution must leave operation intact")
            .status(),
        OperationStatus::InProgress
    );
    assert_eq!(
        state
            .legal()
            .evidence_from_origin(EntityRef::Operation(operation))
            .count(),
        0
    );
    validate_state(&state).expect("stale resolution rejection should not corrupt state");
    validate_invariants(&state);
}

#[test]
fn resolution_token_rejects_new_jurisdiction_after_unrouted_validation() {
    let (registry, mut state, police, neighborhood, operation) =
        make_exposed_business_operation_fixture(false);
    let start = run_tick(&registry, &mut state);
    assert_eq!(start.started_operations, vec![operation]);
    state.advance_clock(SimDuration::from_minutes(45));
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due exposed operation should resolve a plan");
    assert!(matches!(
        plan.outcome.exposure.level,
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ));
    let validated = validate_operation_resolution_plan(&registry, &state, plan)
        .expect("unrouted exposure should validate against absence of jurisdiction");

    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(80)
                .expect("fixture case priority should validate"),
        },
    )
    .expect("new jurisdiction should validate")
    .commit(&mut state)
    .expect("new jurisdiction should commit");

    let error = validated
        .commit(&mut state)
        .expect_err("new incident authority must stale an unrouted resolution token");
    assert_eq!(
        error,
        OperationResolutionError::StaleIncidentRouting {
            neighborhood,
            expected: None,
            found: Some(police),
        }
    );
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("stale resolution must leave operation intact")
            .status(),
        OperationStatus::InProgress
    );
    assert_eq!(
        state
            .legal()
            .evidence_from_origin(EntityRef::Operation(operation))
            .count(),
        0
    );
    validate_state(&state).expect("stale unrouted resolution must leave valid state");
    validate_invariants(&state);
}
