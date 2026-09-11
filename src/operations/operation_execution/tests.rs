//! Focused tests for deterministic operation resolution, proceeds, exposure, and dispositions.

use super::resolution_factors::resolve_target_neighborhoods;
use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::id::{BusinessId, FinancialAccountId, OrganizationId};
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::decision_system::{
    DecisionError, validate_request_police_arrival_decision_on_arrival, validate_resolve_decision,
};
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionResponse, DecisionStatus,
};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner, Money};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{InformationDraft, InformationTopic};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::patrol_system::{
    validate_establish_patrol_deployment, validate_revise_patrol_deployment,
};
use crate::legal::{
    Admissibility, ArrestDraft, CaseWitnessDraft, DayMinute, EvidenceDraft, EvidenceKind,
    EvidenceReliability, EvidenceStrength, InvestigationDraft, JurisdictionDraft,
    PatrolDeploymentDraft, PatrolWindow, PoliceResponsePatrolSnapshot, PoliceResponseRecord,
    PoliceResponseStatus, WitnessCooperation, WitnessStatementDraft,
};
use crate::operations::operation_system::{
    OperationError, OperationTransition, apply_transition, validate_authorize_operation,
};
use crate::operations::property_disposition::{
    CashDispositionDraft, PropertyDispositionDraft, PropertyDispositionError,
    validate_deposit_operation_cash, validate_dispose_property,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationApproach, OperationConstraint,
    OperationContingency, OperationDraft, OperationKind, OperationObjective,
    OperationObjectiveBlocker, OperationObjectiveOutcome, OperationStatus, RoleKind,
};
use crate::reports::organization_financial_report::validate_organization_financial_report;
use crate::world::world_system::{
    designate_player_organization, insert_business, insert_character, insert_neighborhood,
    insert_organization, validate_reassign_character, validate_transfer_business_ownership,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, DriveKind, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

mod police_response;
mod property_economics;

#[derive(Clone, Copy, Serialize)]
struct OperationCashProceedsRecordWire {
    target: EntityRef,
    amount: Money,
}

fn cash_proceeds_wire(
    record: crate::operations::OperationCashProceedsRecord,
) -> OperationCashProceedsRecordWire {
    OperationCashProceedsRecordWire {
        target: record.target(),
        amount: record.amount(),
    }
}

fn replace_serialized_cash_proceeds(
    envelope: SaveEnvelope,
    original: crate::operations::OperationCashProceedsRecord,
    replacement: &OperationCashProceedsRecordWire,
) -> SaveEnvelope {
    let original_bytes =
        bincode::serialize(&original).expect("operation cash proceeds should serialize");
    assert_eq!(
        bincode::serialize(&cash_proceeds_wire(original))
            .expect("operation cash proceeds mirror should serialize"),
        original_bytes,
        "wire mirror must match the production persistence layout exactly"
    );
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement cash proceeds should serialize");
    assert_eq!(replacement_bytes.len(), original_bytes.len());
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized operation cash proceeds must occur exactly once"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-layout cash-proceeds corruption must remain decodable")
}

#[test]
fn restore_rejects_cash_proceeds_not_derived_from_operation_economics() {
    let (registry, mut state, _organization, operation) = make_operation_fixture();
    loop {
        let outcome = run_tick(&registry, &mut state);
        if outcome.resolved_operations.contains(&operation) {
            break;
        }
        assert!(
            state.now().as_minutes() < 100,
            "cash-take fixture should resolve well before this guard"
        );
    }
    let proceeds = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .and_then(|resolution| resolution.cash_proceeds())
        .expect("completed intimidation fixture must retain held cash proceeds");
    let mut corrupted = cash_proceeds_wire(proceeds);
    corrupted.amount = Money::from_cents(
        proceeds
            .amount()
            .cents()
            .checked_add(1)
            .expect("fixture cash amount must leave room for one-cent corruption"),
    );

    let error = restore_save(
        &registry,
        replace_serialized_cash_proceeds(
            build_save(&registry, &state)
                .expect("valid held-cash operation should save before corruption"),
            proceeds,
            &corrupted,
        ),
    )
    .expect_err("cash proceeds must be exactly derivable from authored operation economics");
    assert!(matches!(
        error,
        crate::core::persistence::LoadError::InvalidState(
            crate::core::invariants::StateValidationError::InvalidOperationCashProceeds {
                operation: invalid
            }
        ) if invalid == operation
    ));
}

#[test]
fn detention_cancels_pending_operation_decision_and_aborts_operation() {
    let (registry, mut state, police, _neighborhood, operation) =
        make_exposed_business_operation_fixture_with_contingencies(
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
        );
    run_tick(&registry, &mut state);
    loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.decision_requests.is_empty() {
            break;
        }
    }
    let decision_id = state
        .decisions()
        .pending_for_operation(operation)
        .expect("police-arrival decision should be pending");
    let leader = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist")
        .leader();
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Operation leader detention case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(leader)]),
        },
    )
    .expect("detention investigation should validate")
    .commit(&mut state)
    .expect("detention investigation should commit");
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(leader),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("detention evidence should validate")
    .commit(&mut state)
    .expect("detention evidence should commit");
    let corroborating = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(leader),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating detention evidence should validate")
    .commit(&mut state)
    .expect("corroborating detention evidence should commit");
    let detained_at = state.now();
    let arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: leader,
            investigation,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("custody should validate while leadership decision is pending")
    .commit(&mut state)
    .expect("custody should atomically cancel the decision and abort the operation");

    assert_eq!(
        state
            .legal()
            .active_arrest_for_character(leader)
            .map(|record| record.id()),
        Some(arrest)
    );
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("aborted operation should remain historical");
    assert_eq!(operation_record.status(), OperationStatus::Aborted);
    let abort = operation_record
        .abort_record()
        .expect("detention abort provenance should persist");
    assert_eq!(abort.phase(), OperationAbortPhase::AwaitingDecision);
    assert_eq!(
        abort.cause(),
        OperationAbortCause::ParticipantDetained(leader)
    );
    assert_eq!(abort.aborted_at(), detained_at);
    let decision = state
        .decisions()
        .get_decision(decision_id)
        .expect("cancelled decision should remain historical");
    assert_eq!(decision.status(), DecisionStatus::Cancelled);
    assert!(decision.resolution().is_none());
    let cancellation = decision
        .cancellation()
        .expect("cancelled decision should retain provenance");
    assert_eq!(cancellation.cancelled_at(), detained_at);
    assert_eq!(
        cancellation.reason(),
        DecisionCancellationReason::OperationParticipantDetained(leader)
    );
    assert!(state.decisions().pending_for_operation(operation).is_none());
    validate_state(&state).expect("detention-preempted decision state should validate");
    validate_invariants(&state);
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
    make_exposed_operation_fixture_with_constraints(
        kind,
        assign_jurisdiction,
        contingencies,
        Vec::new(),
    )
}

fn make_exposed_operation_fixture_with_constraints(
    kind: OperationKind,
    assign_jurisdiction: bool,
    contingencies: Vec<OperationContingency>,
    constraints: Vec<OperationConstraint>,
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
    let (title, objective, approach, roles) = match kind {
        OperationKind::Burglary => (
            "Observed burglary".to_owned(),
            OperationObjective::AcquireProperty {
                target: EntityRef::Business(business),
            },
            OperationApproach::Covert,
            BTreeMap::from([
                (RoleKind::Coordinator, leader),
                (RoleKind::EntrySpecialist, specialist),
            ]),
        ),
        OperationKind::Sabotage => (
            "Observed sabotage".to_owned(),
            OperationObjective::DisruptBusiness {
                target: EntityRef::Business(business),
            },
            OperationApproach::Covert,
            BTreeMap::from([
                (RoleKind::Coordinator, leader),
                (RoleKind::EntrySpecialist, specialist),
            ]),
        ),
        OperationKind::Intimidation => (
            "Observed intimidation".to_owned(),
            OperationObjective::ObtainCash {
                target: EntityRef::Business(business),
            },
            OperationApproach::Intimidating,
            BTreeMap::from([(RoleKind::Coordinator, leader)]),
        ),
        OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Surveillance
        | OperationKind::WitnessPressure
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Extraction
        | OperationKind::Arson => panic!("exposure fixture does not support {kind:?}"),
    };
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title,
            kind,
            responsible_organization: organization,
            leader,
            objective,
            approach,
            roles,
            intelligence: BTreeSet::new(),
            constraints,
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
fn sabotage_of_a_suspended_target_records_practical_objective_failure() {
    // Regression: a target whose economy went suspended between authorization and resolution
    // has nothing operating to disrupt, yet the after-action narrative unconditionally
    // claimed disruption. The summary and the committed effect must agree.
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture(OperationKind::Sabotage, false, Vec::new());
    let record = state
        .operations()
        .get_operation(operation)
        .expect("authorized sabotage should persist");
    let OperationObjective::DisruptBusiness {
        target: EntityRef::Business(business),
    } = record.objective()
    else {
        panic!(
            "sabotage fixture must target a business: {:?}",
            record.objective()
        );
    };
    let business = *business;
    crate::economy::business_economy_system::validate_suspend_business_economy(&state, business)
        .expect("an active target economy should suspend")
        .commit(&mut state)
        .expect("target economy suspension should commit");
    run_tick(&registry, &mut state);
    loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.resolved_operations.is_empty() {
            break;
        }
    }
    let record = state
        .operations()
        .get_operation(operation)
        .expect("resolved sabotage should persist");
    let resolution = record
        .resolution()
        .expect("completed operation should persist its resolution");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed,
        "a crew cannot achieve a disruption objective against a business that is not operating"
    );
    assert_eq!(
        resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::TargetEconomyInactive)
    );
    let summary = state
        .reports()
        .get_report(resolution.after_action_report())
        .expect("after-action report should persist")
        .entries()[0]
        .summary
        .clone();
    assert!(
        !summary.contains(crate::operations::operation_economics::SABOTAGE_DISRUPTION_CLAUSE),
        "a suspended target cannot be disrupted, so the after-action must not claim it: {summary}"
    );
    assert!(summary.contains("no active business to disrupt"));
    let economy = state
        .economy()
        .get_business_economy(business)
        .expect("target economy should persist");
    assert!(
        economy.disrupted_through().is_none(),
        "no disruption may be committed against a suspended economy"
    );
    validate_state(&state).expect("suspended-target sabotage state should validate");
    validate_invariants(&state);
}

#[test]
fn operation_does_not_take_from_business_acquired_by_its_sponsor_mid_execution() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture(OperationKind::Burglary, false, Vec::new());
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let (organization, business, due_at) = {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("started burglary should persist");
        let OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } = record.objective()
        else {
            panic!("burglary fixture must target a business")
        };
        (
            record.responsible_organization(),
            *business,
            record
                .resolution_due_at()
                .expect("started burglary must have a resolution deadline"),
        )
    };

    validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Organization(organization),
    )
    .expect("independent target should be transferable to the sponsoring organization")
    .commit(&mut state)
    .expect("target ownership transfer should commit");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture duration must fit SimDuration"),
    ));
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due burglary should decide even after target ownership changes");
    assert_eq!(
        plan.outcome.objective_blocker,
        Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    );
    assert_eq!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(plan.outcome.property_proceeds_plan.proceeds, None);
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("self-owned-target practical failure should validate")
        .commit(&mut state)
        .expect("self-owned-target practical failure should commit");

    let resolution = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("blocked burglary should persist its resolution");
    assert_eq!(
        resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    );
    assert!(resolution.property_proceeds().is_none());
    let after_action = state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("blocked burglary should persist after-action information");
    assert!(after_action.summary().contains("hitting its own assets"));

    // Cross-domain records share minute timestamps. Selling the business again after the
    // operation resolves in the same minute must not make restore pretend the blocker was
    // impossible merely because the later ownership change sorts last within world history.
    validate_transfer_business_ownership(&state, business, BusinessOwner::Independent)
        .expect("the sponsor should be able to sell the blocked target after resolution")
        .commit(&mut state)
        .expect("same-minute post-resolution sale should commit");
    validate_state_against_registry(&registry, &state)
        .expect("same-minute ownership churn must preserve the earlier blocker snapshot");
    validate_invariants(&state);

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("ownership-blocked operation should save"),
    )
    .expect("ownership-blocked operation should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.objective_blocker()),
        Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    );
    validate_invariants(&restored);
}

#[test]
fn same_minute_post_resolution_acquisition_does_not_rewrite_prior_operation_success() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture(OperationKind::Burglary, false, Vec::new());
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let (organization, business, due_at) = {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("started burglary should persist");
        let OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } = record.objective()
        else {
            panic!("burglary fixture must target a business")
        };
        (
            record.responsible_organization(),
            *business,
            record
                .resolution_due_at()
                .expect("started burglary must have a resolution deadline"),
        )
    };
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture duration must fit SimDuration"),
    ));
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due burglary should decide");
    assert_eq!(plan.outcome.objective_blocker, None);
    assert_ne!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed,
        "maximal favorable variance must exercise a successful take"
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("fresh successful burglary should validate")
        .commit(&mut state)
        .expect("fresh successful burglary should commit");

    // The organization acquires the target only after the operation has already resolved, but
    // within the same simulation minute. Minute-level persistence cannot order these cross-domain
    // facts, so the validated resolution snapshot remains authoritative for that ambiguity.
    validate_transfer_business_ownership(
        &state,
        business,
        BusinessOwner::Organization(organization),
    )
    .expect("independent target should remain transferable after the operation")
    .commit(&mut state)
    .expect("same-minute post-resolution acquisition should commit");
    let resolution = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("successful burglary should retain its resolution");
    assert_eq!(resolution.objective_blocker(), None);
    assert_ne!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    validate_state_against_registry(&registry, &state)
        .expect("later same-minute acquisition must not rewrite prior operation success");
    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("same-minute acquisition state should save"),
    )
    .expect("same-minute acquisition state should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.objective_blocker()),
        None
    );
    validate_invariants(&restored);
}

fn detain_pressure_test_character(
    registry: &Registry,
    state: &mut AppState,
    police: OrganizationId,
    character: CharacterId,
    title: &str,
) -> ArrestId {
    let investigation = validate_open_investigation(
        state,
        InvestigationDraft {
            owner: police,
            title: title.to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(character)]),
        },
    )
    .expect("custody investigation should validate")
    .commit(state)
    .expect("custody investigation should commit");
    let strong = validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("strong custody evidence should validate")
    .commit(state)
    .expect("strong custody evidence should commit");
    let corroborating = validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating custody evidence should validate")
    .commit(state)
    .expect("corroborating custody evidence should commit");
    crate::legal::arrest_system::validate_arrest(
        registry,
        state,
        ArrestDraft {
            character,
            investigation,
            evidence: BTreeSet::from([strong, corroborating]),
        },
    )
    .expect("custody evidence should meet the authored arrest bar")
    .commit(state)
    .expect("custody should commit")
}

#[test]
fn detained_witness_cannot_be_pressured_and_in_flight_detention_blocks_the_effect() {
    let registry = build_registry();
    let mut state = AppState::new(0x517A_7E10);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Pressure Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal organization should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Witness Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("law-enforcement organization should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Custody Pressure Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
                (
                    CapabilityKind::Intimidation,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("pressure leader should validate");
    let already_detained = insert_character(
        &mut state,
        CharacterDraft {
            name: "Already Detained Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("first witness should validate");
    let later_detained = insert_character(
        &mut state,
        CharacterDraft {
            name: "Later Detained Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second witness should validate");
    for (witness, title) in [
        (already_detained, "Already-detained witness case"),
        (later_detained, "Later-detained witness case"),
    ] {
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: title.to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(leader)]),
            },
        )
        .expect("pressure-target investigation should validate")
        .commit(&mut state)
        .expect("pressure-target investigation should commit");
        crate::legal::witness_system::validate_register_case_witness(
            &state,
            CaseWitnessDraft {
                investigation,
                witness,
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect("pressure target should register as a witness")
        .commit(&mut state)
        .expect("witness registration should commit");
    }

    detain_pressure_test_character(
        &registry,
        &mut state,
        police,
        already_detained,
        "Custody of first pressure target",
    );
    let pressure_draft = |target, scheduled_for| OperationDraft {
        title: "Pressure custody-sensitive witness".to_owned(),
        kind: OperationKind::WitnessPressure,
        responsible_organization: crew,
        leader,
        objective: OperationObjective::Frighten {
            target: EntityRef::Character(target),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for,
    };
    let error = validate_authorize_operation(
        &registry,
        &state,
        pressure_draft(already_detained, SimTime::from_minutes(1)),
    )
    .expect_err("a detained witness is not an available field-intimidation target");
    assert_eq!(
        error,
        OperationError::InactiveObjectiveTarget(EntityRef::Character(already_detained))
    );

    let pressure = validate_authorize_operation(
        &registry,
        &state,
        pressure_draft(later_detained, SimTime::from_minutes(1)),
    )
    .expect("a free statementless witness should remain pressureable")
    .commit(&mut state)
    .expect("pressure operation should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, pressure, OperationTransition::Begin)
        .expect("pressure operation should begin while its target is free");
    detain_pressure_test_character(
        &registry,
        &mut state,
        police,
        later_detained,
        "Custody during witness pressure",
    );
    let case_witness = state
        .legal()
        .case_witnesses_for_character(later_detained)
        .find(|witness| {
            state
                .legal()
                .get_investigation(witness.investigation())
                .is_some_and(|investigation| {
                    investigation
                        .subjects()
                        .contains(&EntityRef::Character(leader))
                })
        })
        .expect("pressure-target witness registration should persist")
        .id();
    let cooperation_before = state
        .legal()
        .get_case_witness(case_witness)
        .expect("pressure-target witness should persist")
        .cooperation();
    let duration = registry
        .get_operation(OperationKind::WitnessPressure)
        .execution()
        .duration();
    state.advance_clock(duration);
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::WitnessPressure)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        pressure,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due pressure operation should still resolve");
    assert_eq!(
        plan.outcome.objective_blocker,
        Some(OperationObjectiveBlocker::NoPressureableWitnessCase)
    );
    assert_eq!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("custody-blocked pressure result should validate")
        .commit(&mut state)
        .expect("custody-blocked pressure result should commit");
    assert_eq!(
        state
            .legal()
            .get_case_witness(case_witness)
            .expect("witness should persist after blocked pressure")
            .cooperation(),
        cooperation_before,
        "a field operation must not mutate witness cooperation through police custody"
    );
    validate_state_against_registry(&registry, &state)
        .expect("custody-blocked witness pressure should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn statemented_witness_blocks_late_pressure_without_retroactively_weakening_testimony() {
    let registry = build_registry();
    let mut state = AppState::new(0x517A_7E11);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Pressure Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal organization should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("law-enforcement organization should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Pressure Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
                (
                    CapabilityKind::Intimidation,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("pressure leader should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Case Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Pressure target case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(leader)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let case_witness = crate::legal::witness_system::validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation,
            witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("statementless witness should be registerable")
    .commit(&mut state)
    .expect("witness registration should commit");
    let pressure_draft = || OperationDraft {
        title: "Pressure the witness".to_owned(),
        kind: OperationKind::WitnessPressure,
        responsible_organization: crew,
        leader,
        objective: OperationObjective::Frighten {
            target: EntityRef::Character(witness),
        },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: SimTime::from_minutes(1),
    };
    let pressure = validate_authorize_operation(&registry, &state, pressure_draft())
        .expect("statementless witness should be a meaningful pressure target")
        .commit(&mut state)
        .expect("pressure operation should commit");
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![pressure]);

    let statement = crate::legal::witness_system::validate_record_witness_statement(
        &registry,
        &state,
        WitnessStatementDraft {
            case_witness,
            subject: EntityRef::Character(leader),
            origin: None,
            confidence: Rating::try_new(80).expect("fixture confidence should validate"),
            summary: "The witness has already given the case a usable account.".to_owned(),
        },
    )
    .expect("statement should validate while the pressure operation is in flight")
    .commit(&mut state)
    .expect("statement should commit before pressure resolves");
    let statement_record = state
        .legal()
        .get_witness_statement(statement.statement)
        .expect("statement should persist");
    assert_eq!(
        statement_record.cooperation(),
        WitnessCooperation::Reluctant
    );

    let redundant_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
            ..pressure_draft()
        },
    )
    .expect_err("completed testimony leaves no modeled cooperation effect for a new pressure job");
    assert_eq!(
        redundant_error,
        OperationError::TargetNotPressureableWitness(witness)
    );

    let due_at = state
        .operations()
        .get_operation(pressure)
        .and_then(|record| record.resolution_due_at())
        .expect("in-progress pressure should retain its resolution deadline");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(due_at.as_minutes() - state.now().as_minutes())
            .expect("fixture duration must fit SimDuration"),
    ));
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::WitnessPressure)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        pressure,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due pressure operation should still resolve canonically");
    assert_eq!(
        plan.outcome.objective_blocker,
        Some(OperationObjectiveBlocker::NoPressureableWitnessCase)
    );
    assert_eq!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed
    );
    validate_operation_resolution_plan(&registry, &state, plan)
        .expect("statemented-witness practical failure should validate")
        .commit(&mut state)
        .expect("statemented-witness practical failure should commit");

    let witness_record = state
        .legal()
        .get_case_witness(case_witness)
        .expect("witness should persist");
    assert_eq!(
        witness_record.cooperation(),
        WitnessCooperation::Reluctant,
        "later pressure must not rewrite the cooperation snapshot behind completed testimony"
    );
    assert_eq!(
        state
            .legal()
            .get_witness_statement(statement.statement)
            .expect("statement should persist")
            .cooperation(),
        WitnessCooperation::Reluctant
    );
    let resolution = state
        .operations()
        .get_operation(pressure)
        .and_then(|record| record.resolution())
        .expect("blocked pressure should persist its resolution");
    assert_eq!(
        resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::NoPressureableWitnessCase)
    );
    let after_action = state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("blocked pressure should persist after-action information");
    assert!(
        after_action
            .summary()
            .contains("no witness cooperation the crew could still affect")
    );
    validate_state_against_registry(&registry, &state)
        .expect("statemented-witness pressure state should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn validated_witness_pressure_rejects_when_pressureable_case_set_changes() {
    let registry = build_registry();
    let mut state = AppState::new(0x517A_7E12);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Snapshot Pressure Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal organization should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Snapshot Witness Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("law-enforcement organization should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Snapshot Pressure Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (
                    CapabilityKind::Management,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
                (
                    CapabilityKind::Intimidation,
                    Rating::try_new(99).expect("fixture rating should validate"),
                ),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("pressure leader should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Snapshot Case Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("witness should validate");
    let first_investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "First pressureable case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(leader)]),
        },
    )
    .expect("first investigation should validate")
    .commit(&mut state)
    .expect("first investigation should commit");
    let first_case_witness = crate::legal::witness_system::validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation: first_investigation,
            witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("first witness registration should validate")
    .commit(&mut state)
    .expect("first witness registration should commit");
    let pressure = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Freeze witness targets".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(witness),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("pressure operation should validate")
    .commit(&mut state)
    .expect("pressure operation should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(&registry, &mut state, pressure, OperationTransition::Begin)
        .expect("pressure operation should begin");
    let duration = registry
        .get_operation(OperationKind::WitnessPressure)
        .execution()
        .duration();
    state.advance_clock(duration);
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::WitnessPressure)
            .execution()
            .variance_limit(),
    )
    .expect("authored variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        pressure,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due pressure operation should decide");
    assert_eq!(
        plan.outcome.witness_pressure_targets,
        vec![(first_case_witness, WitnessCooperation::Reluctant)],
        "resolution planning must freeze the exact cooperation targets it intends to mutate"
    );
    assert_ne!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed,
        "maximal favorable variance must exercise the pressure tail"
    );
    let validated = validate_operation_resolution_plan(&registry, &state, plan)
        .expect("fresh pressure resolution should validate");

    // Add a second live case for the same witness after validation. The old resolution token
    // must not silently intimidate only the case it happened to observe earlier.
    let second_investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Late pressureable case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(leader)]),
        },
    )
    .expect("second investigation should validate")
    .commit(&mut state)
    .expect("second investigation should commit");
    let second_case_witness = crate::legal::witness_system::validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation: second_investigation,
            witness,
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("late witness registration should validate")
    .commit(&mut state)
    .expect("late witness registration should commit");
    let id_counters_before = (
        state.ids.next_raw(IdKind::Information),
        state.ids.next_raw(IdKind::HistoryEvent),
        state.ids.next_raw(IdKind::Report),
    );
    let error = validated
        .commit(&mut state)
        .expect_err("changed witness-target set must stale the whole operation resolution");
    assert_eq!(
        error,
        OperationResolutionError::StaleObjectiveContext {
            operation: pressure,
        }
    );
    assert_eq!(
        (
            state.ids.next_raw(IdKind::Information),
            state.ids.next_raw(IdKind::HistoryEvent),
            state.ids.next_raw(IdKind::Report),
        ),
        id_counters_before,
        "stale witness-target rejection must not consume resolution artifact IDs"
    );
    assert_eq!(
        state
            .operations()
            .get_operation(pressure)
            .expect("rejected resolution must leave the operation present")
            .status(),
        OperationStatus::InProgress
    );
    assert!(
        state
            .operations()
            .get_operation(pressure)
            .expect("rejected resolution must leave the operation present")
            .resolution()
            .is_none()
    );
    assert_eq!(
        state
            .legal()
            .get_case_witness(first_case_witness)
            .expect("first witness registration should persist")
            .cooperation(),
        WitnessCooperation::Reluctant
    );
    assert_eq!(
        state
            .legal()
            .get_case_witness(second_case_witness)
            .expect("second witness registration should persist")
            .cooperation(),
        WitnessCooperation::Cooperative
    );
    validate_invariants(&state);
}

#[test]
fn stale_sabotage_tail_rejects_before_resolution_mutates_any_operation_artifact() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture(OperationKind::Sabotage, false, Vec::new());
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let record = state
        .operations()
        .get_operation(operation)
        .expect("started sabotage should persist");
    let OperationObjective::DisruptBusiness {
        target: EntityRef::Business(business),
    } = record.objective()
    else {
        panic!(
            "sabotage fixture must target a business: {:?}",
            record.objective()
        );
    };
    let business = *business;
    let due_at = record
        .resolution_due_at()
        .expect("started sabotage must have a resolution deadline");
    let minutes_until_due = u32::try_from(due_at.as_minutes() - state.now().as_minutes())
        .expect("fixture resolution delay must fit SimDuration");
    state.advance_clock(SimDuration::from_minutes(minutes_until_due));
    let variance_limit = i8::try_from(
        registry
            .get_operation(OperationKind::Sabotage)
            .execution()
            .variance_limit(),
    )
    .expect("authored operation variance limit must fit i8");
    let plan = decide_operation_resolution(
        &registry,
        &state,
        operation,
        OperationResolutionRandomness::new(variance_limit, 0),
    )
    .expect("due sabotage should decide");
    assert_ne!(
        plan.outcome.objective_outcome,
        OperationObjectiveOutcome::Failed,
        "maximal favorable execution variance must exercise the disruption tail"
    );
    let validated = validate_operation_resolution_plan(&registry, &state, plan)
        .expect("fresh sabotage resolution should validate");

    // The economy can change independently after resolution validation. Practical objective
    // context is part of the resolution snapshot, so this must stale before any artifact or tail
    // mutation rather than carrying a now-impossible success into commit.
    crate::economy::business_economy_system::validate_suspend_business_economy(&state, business)
        .expect("target economy should suspend")
        .commit(&mut state)
        .expect("target suspension should commit");
    let id_counters_before = (
        state.ids.next_raw(IdKind::Information),
        state.ids.next_raw(IdKind::HistoryEvent),
        state.ids.next_raw(IdKind::Report),
        state.ids.next_raw(IdKind::Investigation),
        state.ids.next_raw(IdKind::Evidence),
    );
    let error = validated
        .commit(&mut state)
        .expect_err("stale disruption must reject before resolution mutation");
    assert_eq!(
        error,
        OperationResolutionError::StaleObjectiveContext { operation }
    );
    let record = state
        .operations()
        .get_operation(operation)
        .expect("rejected resolution must leave the operation live");
    assert_eq!(record.status(), OperationStatus::InProgress);
    assert!(record.resolution().is_none());
    assert_eq!(
        (
            state.ids.next_raw(IdKind::Information),
            state.ids.next_raw(IdKind::HistoryEvent),
            state.ids.next_raw(IdKind::Report),
            state.ids.next_raw(IdKind::Investigation),
            state.ids.next_raw(IdKind::Evidence),
        ),
        id_counters_before,
        "rejected tail staleness must not consume any resolution artifact IDs"
    );
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
    assert!(
        report.entries()[0]
            .entities
            .contains(&EntityRef::Operation(operation))
    );
    let history = state
        .history()
        .get_event(resolution.history_event())
        .expect("operation resolution should create campaign history");
    assert_eq!(history.kind(), HistoryEventKind::Operation);
    assert!(
        history
            .entities()
            .contains(&EntityRef::Operation(operation))
    );
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
        OperationObjectiveOutcome::Achieved,
        factors,
        OperationExposureLevel::None,
        65,
    );
    assert!(!achieved.contains("unplanned circumstances"));
    assert!(!achieved.contains("crew overcame them"));

    let partial = build_after_action_summary(
        OperationObjectiveOutcome::Partial,
        OperationObjectiveOutcome::Partial,
        factors,
        OperationExposureLevel::None,
        65,
    );
    assert!(partial.contains("reduced the result"));

    let failed = build_after_action_summary(
        OperationObjectiveOutcome::Failed,
        OperationObjectiveOutcome::Failed,
        factors,
        OperationExposureLevel::None,
        65,
    );
    assert!(failed.contains("contributed to the failure"));
}

#[test]
fn practical_objective_failure_does_not_misattribute_tactical_success() {
    let factors = OperationResolutionFactors {
        role_capability_average: Rating::try_new(90).expect("fixture rating should be valid"),
        leader_capability: Some(Rating::try_new(90).expect("fixture rating should be valid")),
        intelligence_quality: Rating::try_new(0).expect("fixture rating should be valid"),
        intelligence_adjustment: 0,
        intelligence_topics_covered: 0,
        intelligence_topics_relevant: 1,
        target_police_presence: Some(
            Rating::try_new(20).expect("fixture police presence should be valid"),
        ),
        police_response_arrived: false,
        approach_adjustment: 0,
        time_pressure: 0,
        variance: -12,
    };

    let summary = build_after_action_summary(
        OperationObjectiveOutcome::Failed,
        OperationObjectiveOutcome::Achieved,
        factors,
        OperationExposureLevel::None,
        65,
    );
    assert!(summary.starts_with("Objective failed."));
    assert!(!summary.contains("Assigned-role competence"));
    assert!(!summary.contains("Leadership coordination"));
    assert!(!summary.contains("contributed to the failure"));
    assert!(!summary.contains("reduced the result"));
    assert!(!summary.contains("Favorable unplanned circumstances"));
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
        OperationObjectiveOutcome::Achieved,
        neutral,
        OperationExposureLevel::None,
        65,
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
        OperationObjectiveOutcome::Achieved,
        thin,
        OperationExposureLevel::None,
        65,
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
        OperationObjectiveOutcome::Achieved,
        informed,
        OperationExposureLevel::None,
        65,
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
        OperationObjectiveOutcome::Achieved,
        gapped,
        OperationExposureLevel::None,
        65,
    );
    assert!(gapped_plan.contains("Planning intelligence covered 1 of 4 relevant areas"));
    assert!(gapped_plan.contains("large gaps remained in the plan's information"));

    let pressured = OperationResolutionFactors {
        time_pressure: 3,
        ..neutral
    };
    let rushed = build_after_action_summary(
        OperationObjectiveOutcome::Achieved,
        OperationObjectiveOutcome::Achieved,
        pressured,
        OperationExposureLevel::None,
        65,
    );
    assert!(rushed.contains("compressed the execution window"));

    let witnessed = build_after_action_summary(
        OperationObjectiveOutcome::Partial,
        OperationObjectiveOutcome::Partial,
        neutral,
        OperationExposureLevel::Witnessed,
        65,
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
fn resume_allows_current_minute_follow_up_when_next_tick_start_equals_shifted_end() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture_with_constraints(
            OperationKind::Intimidation,
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
            vec![OperationConstraint::CompleteBy(SimTime::from_minutes(6))],
        );
    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started intimidation should persist");
    let organization = operation_record.responsible_organization();
    let leader = operation_record.leader();
    let target = operation_record
        .objective()
        .referenced_entities()
        .into_iter()
        .find_map(|entity| match entity {
            EntityRef::Business(business) => Some(business),
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Neighborhood(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => None,
        })
        .expect("intimidation fixture must target its business");
    let due_at = operation_record
        .resolution_due_at()
        .expect("started intimidation must have a due time");

    let paused_at = loop {
        let outcome = run_tick(&registry, &mut state);
        if !outcome.decision_requests.is_empty() {
            break outcome.now;
        }
        assert!(outcome.resolved_operations.is_empty());
    };
    assert_eq!(
        paused_at + SimDuration::ONE_MINUTE,
        due_at,
        "fixture must pause with exactly one minute of execution remaining"
    );
    let decision_id = state
        .decisions()
        .pending_for_operation(operation)
        .expect("police-arrival decision should be pending");

    let follow_up = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Boundary follow-up".to_owned(),
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
            scheduled_for: state.now(),
        },
    )
    .expect("current-minute follow-up actually begins next tick at the paused job's end")
    .commit(&mut state)
    .expect("boundary follow-up should commit");

    validate_resolve_decision(
        &registry,
        &state,
        decision_id,
        organization,
        DecisionResponse::Continue,
    )
    .expect("resume should not treat the follow-up's authored minute as occupied")
    .commit(&mut state)
    .expect("boundary resume should commit through the canonical decision path");
    let resumed = state
        .operations()
        .get_operation(operation)
        .expect("resumed operation should persist");
    assert_eq!(resumed.status(), OperationStatus::InProgress);
    assert_eq!(resumed.resolution_due_at(), Some(due_at));
    assert_eq!(
        state
            .operations()
            .get_operation(follow_up)
            .expect("follow-up should persist")
            .scheduled_for(),
        paused_at
    );
    validate_state(&state).expect("boundary resume state should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn due_follow_up_waits_when_prior_operation_remains_paused_past_boundary() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture_with_constraints(
            OperationKind::Intimidation,
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
            vec![OperationConstraint::CompleteBy(SimTime::from_minutes(6))],
        );
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let organization = operation_record.responsible_organization();
    let leader = operation_record.leader();
    let target = operation_record
        .objective()
        .referenced_entities()
        .into_iter()
        .find_map(|entity| match entity {
            EntityRef::Business(business) => Some(business),
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Neighborhood(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => None,
        })
        .expect("fixture operation must target a business");

    while state
        .operations()
        .get_operation(operation)
        .expect("operation should persist")
        .status()
        != OperationStatus::AwaitingDecision
    {
        run_tick(&registry, &mut state);
    }
    assert_eq!(state.now(), SimTime::from_minutes(5));

    let follow_up = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Deferred boundary follow-up".to_owned(),
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
            scheduled_for: state.now(),
        },
    )
    .expect("follow-up is valid if the pending decision resolves immediately")
    .commit(&mut state)
    .expect("boundary follow-up should commit");

    let boundary = run_tick(&registry, &mut state);
    assert_eq!(boundary.now, SimTime::from_minutes(6));
    assert!(boundary.started_operations.is_empty());
    assert_eq!(
        state
            .operations()
            .get_operation(follow_up)
            .expect("blocked follow-up should persist")
            .status(),
        OperationStatus::Authorized
    );
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("paused operation should persist")
            .status(),
        OperationStatus::AwaitingDecision
    );
    validate_state_against_registry(&registry, &state)
        .expect("pause growth after authorization is a valid temporary booking overlap");

    let deadline_cleanup = run_tick(&registry, &mut state);
    assert_eq!(deadline_cleanup.now, SimTime::from_minutes(7));
    assert!(deadline_cleanup.started_operations.is_empty());
    assert_eq!(
        state
            .operations()
            .get_operation(operation)
            .expect("deadline-aborted operation should persist")
            .status(),
        OperationStatus::Aborted
    );
    assert_eq!(
        state
            .operations()
            .get_operation(follow_up)
            .expect("follow-up should remain queued until the next tick")
            .status(),
        OperationStatus::Authorized
    );

    let retry = run_tick(&registry, &mut state);
    assert_eq!(retry.now, SimTime::from_minutes(8));
    assert_eq!(retry.started_operations, vec![follow_up]);
    validate_state(&state).expect("deferred follow-up state should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn delayed_authorized_operation_aborts_once_entry_window_becomes_impossible() {
    let (registry, mut state, _police, _neighborhood, operation) =
        make_exposed_operation_fixture_with_constraints(
            OperationKind::Intimidation,
            true,
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
            vec![OperationConstraint::CompleteBy(SimTime::from_minutes(6))],
        );
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("started operation should persist");
    let organization = operation_record.responsible_organization();
    let leader = operation_record.leader();
    let target = operation_record
        .objective()
        .referenced_entities()
        .into_iter()
        .find_map(|entity| match entity {
            EntityRef::Business(business) => Some(business),
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Neighborhood(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => None,
        })
        .expect("fixture operation must target a business");

    while state
        .operations()
        .get_operation(operation)
        .expect("operation should persist")
        .status()
        != OperationStatus::AwaitingDecision
    {
        run_tick(&registry, &mut state);
    }
    assert_eq!(state.now(), SimTime::from_minutes(5));

    let entry_specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Deadline Entry Specialist".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("entry specialist fixture should validate");
    let follow_up = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Deadline-compressed follow-up".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, leader),
                (RoleKind::EntrySpecialist, entry_specialist),
            ]),
            intelligence: BTreeSet::new(),
            constraints: vec![OperationConstraint::CompleteBy(SimTime::from_minutes(17))],
            contingencies: Vec::new(),
            scheduled_for: state.now(),
        },
    )
    .expect("one executable minute remains if the follow-up begins on the next tick")
    .commit(&mut state)
    .expect("deadline-compressed follow-up should commit");

    let first_due = run_tick(&registry, &mut state);
    assert_eq!(first_due.now, SimTime::from_minutes(6));
    assert!(first_due.started_operations.is_empty());
    assert_eq!(
        state
            .operations()
            .get_operation(follow_up)
            .expect("participant-blocked follow-up should persist")
            .status(),
        OperationStatus::Authorized
    );

    let impossible = run_tick(&registry, &mut state);
    assert_eq!(impossible.now, SimTime::from_minutes(7));
    assert!(impossible.started_operations.is_empty());
    let follow_up_record = state
        .operations()
        .get_operation(follow_up)
        .expect("deadline-aborted follow-up should persist");
    assert_eq!(follow_up_record.status(), OperationStatus::Aborted);
    assert_eq!(
        follow_up_record.abort_record().map(|abort| abort.phase()),
        Some(OperationAbortPhase::BeforeStart)
    );
    assert_eq!(
        follow_up_record.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    let artifacts = follow_up_record
        .abort_record()
        .and_then(|abort| abort.artifacts())
        .expect("pre-start deadline miss should surface after-action artifacts");
    assert!(
        state
            .intelligence()
            .get_information(artifacts.information())
            .expect("deadline-miss information should persist")
            .summary()
            .contains("could no longer meet its completion deadline"),
        "an early infeasibility abort must not claim the literal deadline already passed"
    );
    validate_state(&state)
        .expect("immediate deadline-abort state should remain structurally valid");
    validate_invariants(&state);

    let envelope = build_save(&registry, &state)
        .expect("early deadline-abort state should pass registry-aware save validation");
    let bytes = bincode::serialize(&envelope).expect("deadline-abort save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("deadline-abort save should deserialize");
    let restored = restore_save(&registry, decoded)
        .expect("registry-aware restore should accept canonical early deadline abort");
    let restored_follow_up = restored
        .operations()
        .get_operation(follow_up)
        .expect("early deadline-aborted operation should survive restore");
    assert_eq!(restored_follow_up.status(), OperationStatus::Aborted);
    assert_eq!(
        restored_follow_up.abort_record().map(|abort| abort.cause()),
        Some(OperationAbortCause::DeadlineMissed)
    );
    validate_state(&restored).expect("restored early deadline-abort state should validate");
    validate_state_against_registry(&registry, &restored)
        .expect("restored early deadline-abort state should match authored operation timing");
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
        .find_map(|entity| {
            if let EntityRef::Business(business) = entity {
                Some(business)
            } else {
                None
            }
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
    assert!(
        report.entries()[0]
            .entities
            .contains(&EntityRef::DecisionRequest(decision_id))
    );
    let history = state
        .history()
        .get_event(artifacts.history_event())
        .expect("abort history should persist");
    assert_eq!(history.summary(), information.summary());
    assert!(
        history
            .entities()
            .contains(&EntityRef::DecisionRequest(decision_id))
    );

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
    designate_player_organization(&mut state, organization)
        .expect("operation organization should be eligible as the player organization");
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
    assert!(after_action.summary().contains("held it for later deposit"));

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

    // A disposition settlement account is a one-off external balancing counterparty, not a
    // permanent reservation. Dedicating it to a new enterprise later in the same simulation
    // minute is a canonical sequence and must remain valid across save/restore.
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("completed operation should persist before enterprise establishment");
    let manager = operation_record.leader();
    let OperationObjective::ObtainCash {
        target: EntityRef::Business(target_business),
    } = operation_record.objective()
    else {
        panic!("cash fixture changed objective")
    };
    let target_business = *target_business;
    let neighborhood = state
        .world()
        .get_business(target_business)
        .expect("cash target business should persist")
        .neighborhood();
    let scope = crate::delegation::ResponsibilityScope::Neighborhood(neighborhood);
    let mandate = crate::delegation::delegation_system::validate_assign_mandate(
        &state,
        crate::delegation::MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("completed operation leader should accept a district mandate")
    .commit(&mut state)
    .expect("district mandate should commit");
    let enterprise = crate::enterprises::enterprise_execution::validate_establish_enterprise(
        &registry,
        &state,
        crate::enterprises::EnterpriseDraft {
            kind: crate::enterprises::EnterpriseKind::Protection,
            organization,
            authority: crate::delegation::MandateAuthority {
                mandate,
                manager,
                scope,
            },
            location: crate::enterprises::EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .expect("a historical disposition must not permanently reserve its settlement account")
    .commit(&mut state)
    .expect("later enterprise account dedication should commit");
    assert_eq!(
        state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should persist")
            .settlement_account(),
        settlement_account
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
    let delta = 1_439_u64
        .checked_sub(state.now().as_minutes())
        .expect("cash deposit should occur before the first daily brief");
    state.advance_clock(SimDuration::from_minutes(
        u32::try_from(delta).expect("daily brief delta should fit SimDuration"),
    ));
    let brief = run_tick(&registry, &mut state)
        .executive_brief
        .expect("daily brief should include both cash events");
    let brief = state
        .reports()
        .get_report(brief)
        .expect("cash-event executive brief should persist");
    let operation_entries: Vec<_> = brief
        .entries()
        .iter()
        .filter(|entry| entry.entities.contains(&EntityRef::Operation(operation)))
        .collect();
    assert_eq!(operation_entries.len(), 2);
    assert!(
        operation_entries
            .iter()
            .any(|entry| entry.summary.contains("held it for later deposit"))
    );
    assert!(operation_entries.iter().any(|entry| {
        entry.summary.starts_with("Cash from ") && entry.summary.contains("was deposited for")
    }));
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
    let corroborating = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating evidence should validate")
    .commit(&mut state)
    .expect("corroborating evidence should commit");
    let arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, corroborating]),
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

    // A plan that cannot finish before the current custody window ends is knowingly obsolete
    // before it starts and must be rejected rather than waiting for a due-tick failure.
    let maximum_detention = registry.legal().maximum_detention();
    let extraction_duration = registry
        .get_operation(OperationKind::Extraction)
        .execution()
        .duration();
    let impossible_start = SimTime::from_minutes(
        state.now().as_minutes()
            + u64::from(maximum_detention.as_minutes() - extraction_duration.as_minutes() + 1),
    );
    let impossible_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Too-late extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: impossible_start,
        },
    )
    .expect_err("an extraction cannot be planned beyond the target's current custody");
    assert!(matches!(
        impossible_error,
        OperationError::ExtractionMissesCustodyWindow {
            character,
            planned_end,
            custody_ends_at,
        } if character == detainee
            && planned_end == impossible_start + extraction_duration
            && custody_ends_at == state.now() + maximum_detention
    ));

    // Completion at the release instant is also too late: the custody relationship ends at that
    // instant, so the operation must finish strictly before it rather than racing the release pass.
    let boundary_start = SimTime::from_minutes(
        state.now().as_minutes()
            + u64::from(maximum_detention.as_minutes() - extraction_duration.as_minutes()),
    );
    let boundary_error = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Boundary extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: boundary_start,
        },
    )
    .expect_err("an extraction must finish before the custody release instant");
    assert!(matches!(
        boundary_error,
        OperationError::ExtractionMissesCustodyWindow {
            character,
            planned_end,
            custody_ends_at,
        } if character == detainee
            && planned_end == boundary_start + extraction_duration
            && custody_ends_at == state.now() + maximum_detention
    ));

    // If an already-running extraction is delayed until the custody cap, mandatory release is the
    // first same-minute lifecycle action. The operation then records a practical failure instead
    // of receiving credit for breaking custody that legally ended at the same instant.
    let mut expiry_state = state.clone();
    let expiry_extraction = validate_authorize_operation(
        &registry,
        &expiry_state,
        OperationDraft {
            title: "Expired-custody extraction".to_owned(),
            kind: OperationKind::Extraction,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::FreeDetainee { target: detainee },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Driver, driver)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: expiry_state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("early extraction should validate")
    .commit(&mut expiry_state)
    .expect("early extraction should commit");
    let start_outcome = run_tick(&registry, &mut expiry_state);
    assert_eq!(start_outcome.started_operations, vec![expiry_extraction]);
    let release_at = expiry_state
        .legal()
        .get_arrest(arrest)
        .expect("fixture arrest should persist")
        .arrested_at()
        + maximum_detention;
    let minutes_until_pre_release = release_at
        .as_minutes()
        .checked_sub(expiry_state.now().as_minutes() + 1)
        .expect("fresh extraction starts before custody release");
    expiry_state.advance_clock(SimDuration::from_minutes(
        u32::try_from(minutes_until_pre_release).expect("fixture detention window must fit u32"),
    ));
    let expiry_outcome = run_tick(&registry, &mut expiry_state);
    assert_eq!(expiry_outcome.now, release_at);
    assert_eq!(expiry_outcome.custody_releases, vec![arrest]);
    assert_eq!(expiry_outcome.resolved_operations, vec![expiry_extraction]);
    let expiry_resolution = expiry_state
        .operations()
        .get_operation(expiry_extraction)
        .and_then(|record| record.resolution())
        .expect("expired-custody extraction should still resolve");
    assert_eq!(
        expiry_resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(
        expiry_resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::ExtractionCustodyEnded)
    );
    assert_eq!(expiry_resolution.extraction_arrest(), None);
    assert_eq!(
        expiry_state
            .legal()
            .get_arrest(arrest)
            .and_then(|record| record.released_at()),
        Some(release_at)
    );
    validate_state(&expiry_state).expect("custody-boundary extraction state should validate");
    validate_invariants(&expiry_state);

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
    assert_eq!(
        record
            .resolution()
            .and_then(|resolution| resolution.extraction_arrest()),
        Some(arrest),
        "successful extraction history must identify the custody relationship it broke"
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

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("successful extraction state should save"),
    )
    .expect("successful extraction state should restore");
    assert_eq!(
        restored
            .operations()
            .get_operation(extraction)
            .and_then(|record| record.resolution())
            .and_then(|resolution| resolution.extraction_arrest()),
        Some(arrest)
    );
    validate_invariants(&restored);

    // Custody can still end after a valid authorization. If it ends after execution starts, the
    // due resolution must become a practical objective failure, not panic because the release
    // transaction no longer has an arrest to consume. Renewed custody in this same case must be
    // based on evidence discovered after the extraction release.
    state.advance_clock(SimDuration::ONE_MINUTE);
    let renewed_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::CommunicationRecord,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("post-release evidence should validate")
    .commit(&mut state)
    .expect("post-release evidence should commit");
    let rearrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, renewed_evidence]),
        },
    )
    .expect("evidence-backed renewed arrest should validate")
    .commit(&mut state)
    .expect("evidence-backed renewed arrest should commit");
    let stale_authorization = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Stale custody authorization".to_owned(),
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
    .expect("fresh custody should produce an extraction authorization token");
    crate::legal::arrest_system::validate_release_arrest(&state, rearrest)
        .expect("first renewed arrest should be releasable before authorization commit")
        .commit(&mut state)
        .expect("first renewed arrest release should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    let replacement_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::FinancialRecord,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("replacement-custody evidence should validate")
    .commit(&mut state)
    .expect("replacement-custody evidence should commit");
    let replacement_arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, replacement_evidence]),
        },
    )
    .expect("replacement custody should validate from newly developed evidence")
    .commit(&mut state)
    .expect("replacement custody should commit");
    let stale_error = stale_authorization
        .commit(&mut state)
        .expect_err("authorization must not retarget itself to replacement custody");
    assert_eq!(
        stale_error,
        OperationError::StaleExtractionCustody {
            character: detainee,
            expected: rearrest,
            found: Some(replacement_arrest),
        }
    );

    let stale_target_extraction = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Custody-ended extraction".to_owned(),
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
    .expect("replacement custody should support a fresh extraction")
    .commit(&mut state)
    .expect("fresh extraction should commit");
    assert_eq!(
        state
            .operations()
            .get_operation(stale_target_extraction)
            .and_then(|record| record.extraction_arrest()),
        Some(replacement_arrest),
        "the operation command must freeze the exact custody relationship it targets"
    );
    state.advance_clock(SimDuration::ONE_MINUTE);
    apply_transition(
        &registry,
        &mut state,
        stale_target_extraction,
        OperationTransition::Begin,
    )
    .expect("extraction should begin while custody is current");
    crate::legal::arrest_system::validate_release_arrest(&state, replacement_arrest)
        .expect("custody should still be releasable after operation start")
        .commit(&mut state)
        .expect("external custody release should commit");
    state.advance_clock(SimDuration::ONE_MINUTE);
    let later_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("later-custody evidence should validate")
    .commit(&mut state)
    .expect("later-custody evidence should commit");
    let later_arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: detainee,
            investigation,
            evidence: BTreeSet::from([evidence, later_evidence]),
        },
    )
    .expect("new evidence makes the later re-arrest a distinct legal event")
    .commit(&mut state)
    .expect("later re-arrest should commit without retargeting the active operation");
    state.advance_clock(SimDuration::from_minutes(
        extraction_duration.as_minutes() - 1,
    ));
    let outcome = run_tick(&registry, &mut state);
    assert_eq!(
        outcome.resolved_operations,
        vec![stale_target_extraction],
        "custody expiry must remain a resolvable canonical tick outcome"
    );
    let stale_resolution = state
        .operations()
        .get_operation(stale_target_extraction)
        .and_then(|record| record.resolution())
        .expect("custody-ended extraction should still persist a resolution");
    assert_eq!(
        stale_resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(stale_resolution.extraction_arrest(), None);
    assert_eq!(
        stale_resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::ExtractionCustodyEnded)
    );
    assert_eq!(
        state
            .legal()
            .active_arrest_for_character(detainee)
            .map(|arrest| arrest.id()),
        Some(later_arrest),
        "the old extraction must not release a newer custody event it was never authorized against"
    );
    let after_action = state
        .intelligence()
        .get_information(stale_resolution.after_action_information())
        .expect("failed extraction should persist an after-action record");
    assert!(after_action.summary().contains("no longer detained"));
    validate_state(&state).expect("custody-ended extraction state should remain valid");
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
    let decision_minute = arrest_record.arrested_at().as_minutes()
        + u64::from(registry.legal().informant_decision_delay().as_minutes());
    loop {
        let outcome = run_tick(&registry, &mut state);
        let flipped = state.legal().informant_for(suspect, police).is_some();
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
fn operation_case_geography_does_not_follow_later_organization_assets() {
    let (registry, mut state, _police, incident_neighborhood, operation) =
        make_exposed_business_operation_fixture(true);
    let organization = state
        .operations()
        .get_operation(operation)
        .expect("origin operation should persist")
        .responsible_organization();

    let started = run_tick(&registry, &mut state);
    assert_eq!(started.started_operations, vec![operation]);
    loop {
        let outcome = run_tick(&registry, &mut state);
        if outcome.resolved_operations.contains(&operation) {
            break;
        }
    }
    let investigation = state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .and_then(|resolution| resolution.exposure().investigation())
        .expect("high-exposure operation should open an incident case");
    let case = state
        .legal()
        .get_investigation(investigation)
        .expect("incident case should persist");
    assert_eq!(
        resolve_investigation_target_neighborhoods(&state, case),
        BTreeSet::from([incident_neighborhood])
    );

    let later_neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Later Expansion Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(55).expect("fixture wealth should validate"),
                    commercial_activity: Rating::try_new(55)
                        .expect("fixture commerce should validate"),
                    illicit_demand: Rating::try_new(55).expect("fixture demand should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(20)
                        .expect("fixture police presence should validate"),
                },
            },
        },
    )
    .expect("later expansion neighborhood should validate");
    insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Later Organization Asset".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CashIntensive]),
            neighborhood: later_neighborhood,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .expect("later organization asset should validate");

    let case = state
        .legal()
        .get_investigation(investigation)
        .expect("incident case should still persist");
    assert_eq!(
        resolve_investigation_target_neighborhoods(&state, case),
        BTreeSet::from([incident_neighborhood]),
        "an old operation case must remain anchored to its incident district after expansion"
    );
    validate_state(&state).expect("later unrelated asset should leave the case coherent");
    validate_invariants(&state);
}

#[test]
fn witness_pressure_prefers_case_geography_over_character_organization_footprint() {
    let registry = build_registry();
    let mut state = AppState::new(0x517A_7E13);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Civilian Pressure Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("pressure crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Civilian Witness Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("witness precinct should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Civilian Witness Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture wealth should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture commerce should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture demand should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(85)
                        .expect("fixture police presence should validate"),
                },
            },
        },
    )
    .expect("witness neighborhood should validate");
    let newer_neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Later Civilian Witness Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture wealth should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture commerce should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture demand should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(20)
                        .expect("fixture police presence should validate"),
                },
            },
        },
    )
    .expect("later witness neighborhood should validate");
    validate_set_jurisdiction(
        &state,
        JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([neighborhood, newer_neighborhood]),
            case_intake_priority: Rating::try_new(80)
                .expect("fixture case priority should validate"),
        },
    )
    .expect("witness jurisdiction should validate")
    .commit(&mut state)
    .expect("witness jurisdiction should commit");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Civilian Pressure Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Intimidation,
                Rating::try_new(80).expect("fixture intimidation should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("pressure leader should validate");
    let witness = insert_character(
        &mut state,
        CharacterDraft {
            name: "Independent Civilian Witness".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("civilian witness should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Civilian witness case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Neighborhood(neighborhood)]),
        },
    )
    .expect("witness case should validate")
    .commit(&mut state)
    .expect("witness case should commit");
    crate::legal::witness_system::validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation,
            witness,
            cooperation: WitnessCooperation::Reluctant,
        },
    )
    .expect("civilian witness should register on the case")
    .commit(&mut state)
    .expect("civilian witness registration should commit");
    let newer_investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Later civilian witness case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Neighborhood(newer_neighborhood)]),
        },
    )
    .expect("later witness case should validate")
    .commit(&mut state)
    .expect("later witness case should commit");
    crate::legal::witness_system::validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation: newer_investigation,
            witness,
            cooperation: WitnessCooperation::Cooperative,
        },
    )
    .expect("civilian witness should register on the later case")
    .commit(&mut state)
    .expect("later civilian witness registration should commit");
    let pressure = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Pressure independent civilian".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(witness),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::from_minutes(1),
        },
    )
    .expect("civilian witness pressure should authorize")
    .commit(&mut state)
    .expect("civilian witness pressure should commit");

    let alert = resolve_operation_police_alert_context(
        &registry,
        &state,
        pressure,
        SimTime::from_minutes(1),
    );
    assert_eq!(
        alert.neighborhood(),
        Some(newer_neighborhood),
        "one civilian encounter must use the latest pressureable case geography instead of inheriting the strongest police presence across every live witness case"
    );

    // Joining an organization with an unrelated asset in the older, higher-police district must
    // not teleport the already-authored witness encounter to that organization's strongest-risk
    // premises. The active witness case is the causal reason for the pressure operation and
    // remains the venue proxy for members and civilians alike.
    let rival = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Unrelated Rival Outfit".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival organization should validate");
    insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Unrelated Rival Premises".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CashIntensive]),
            neighborhood,
            owner: BusinessOwner::Organization(rival),
        },
    )
    .expect("rival premises should validate");
    validate_reassign_character(&state, witness, Some(rival), None)
        .expect("witness should be able to join the rival organization")
        .commit(&mut state)
        .expect("witness reassignment should commit");

    let member_alert = resolve_operation_police_alert_context(
        &registry,
        &state,
        pressure,
        SimTime::from_minutes(1),
    );
    assert_eq!(
        member_alert.neighborhood(),
        Some(newer_neighborhood),
        "witness membership must not replace case geography with an unrelated organization-wide footprint"
    );
    validate_state(&state).expect("civilian witness venue attribution should stay valid");
    validate_invariants(&state);
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
            .all_evidence()
            .filter(|record| record.origin() == Some(EntityRef::Operation(operation)))
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
            .all_evidence()
            .filter(|record| record.origin() == Some(EntityRef::Operation(operation)))
            .count(),
        0
    );
    validate_state(&state).expect("stale unrouted resolution must leave valid state");
    validate_invariants(&state);
}
