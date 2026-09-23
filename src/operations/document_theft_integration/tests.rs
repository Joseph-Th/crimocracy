//! Focused executable contract for document theft as information acquisition.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_test_tick as run_tick;
use crate::core::state::AppState;
use crate::core::time::SimDuration;
use crate::intelligence::{InformationSourceKind, KnowledgeHolder};
use crate::operations::information_acquisition::InformationAcquisitionError;
use crate::operations::operation_execution::{
    OperationResolutionError, OperationResolutionRandomness, decide_operation_resolution,
    validate_operation_resolution_plan,
};
use crate::operations::operation_system::{OperationError, validate_authorize_operation};
use crate::operations::{
    OperationApproach, OperationDraft, OperationObjectiveBlocker, OperationObjectiveKind, RoleKind,
};
use crate::registry::Registry;
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
    validate_transfer_business_ownership,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    crew: OrganizationId,
    leader: crate::core::id::CharacterId,
    entry: crate::core::id::CharacterId,
    target: BusinessId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating should validate")
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0xD0C0_7E57);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Records Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Records District".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(60),
                    illicit_demand: rating(40),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(0),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Ledger Office".to_owned(),
            kind: BusinessKind::ProfessionalServices,
            functions: BTreeSet::from([
                BusinessFunction::ProfessionalRecords,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("target business should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Records Lead".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Management, rating(100)),
                (CapabilityKind::Burglary, rating(100)),
                (CapabilityKind::Stealth, rating(100)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader should validate");
    let entry = insert_character(
        &mut state,
        CharacterDraft {
            name: "Records Entry".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Burglary, rating(100)),
                (CapabilityKind::Stealth, rating(100)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("entry specialist should validate");
    Fixture {
        registry,
        state,
        crew,
        leader,
        entry,
        target,
    }
}

fn draft(fixture: &Fixture, objective: OperationObjective) -> OperationDraft {
    OperationDraft {
        title: "Acquire target records".to_owned(),
        kind: OperationKind::DocumentTheft,
        responsible_organization: fixture.crew,
        leader: fixture.leader,
        objective,
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([
            (RoleKind::Coordinator, fixture.leader),
            (RoleKind::EntrySpecialist, fixture.entry),
        ]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
    }
}

fn authorize(fixture: &mut Fixture) -> OperationId {
    validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        draft(
            fixture,
            OperationObjective::GatherInformation {
                target: EntityRef::Business(fixture.target),
            },
        ),
    )
    .expect("document theft information objective should validate")
    .commit(&mut fixture.state)
    .expect("validated document theft should commit")
}

fn decide_due_resolution(
    fixture: &mut Fixture,
    operation: OperationId,
) -> crate::operations::operation_execution::OperationResolutionPlan {
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![operation]);
    fixture.state.advance_clock(SimDuration::from_minutes(30));
    decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due document theft should produce a resolution plan")
}

#[test]
fn document_theft_requires_gather_information_instead_of_property_acquisition() {
    let fixture = fixture();
    assert_eq!(
        OperationKind::DocumentTheft.objective_kind(),
        OperationObjectiveKind::GatherInformation
    );
    assert!(
        fixture
            .registry
            .get_operation(OperationKind::DocumentTheft)
            .execution()
            .property_proceeds()
            .is_none()
    );
    let objective = OperationObjective::AcquireProperty {
        target: EntityRef::Business(fixture.target),
    };
    assert_eq!(
        validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            draft(&fixture, objective),
        )
        .expect_err("property-acquisition document theft must be rejected"),
        OperationError::InvalidObjectiveForKind {
            kind: OperationKind::DocumentTheft,
            objective: OperationObjectiveKind::AcquireProperty,
        }
    );
}

#[test]
fn successful_document_theft_persists_acquired_records_without_fenceable_property() {
    let mut fixture = fixture();
    let operation = authorize(&mut fixture);
    let plan = decide_due_resolution(&mut fixture, operation);
    validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .expect("fresh document theft resolution should validate")
        .commit(&mut fixture.state)
        .expect("validated document theft resolution should commit");

    let record = fixture
        .state
        .operations()
        .get_operation(operation)
        .expect("document theft should persist");
    let resolution = record
        .resolution()
        .expect("document theft should have a resolution");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    assert!(resolution.property_proceeds().is_none());
    assert!(record.property_disposition().is_none());
    assert_eq!(resolution.discovered_information().len(), 1);
    let information_id = *resolution
        .discovered_information()
        .iter()
        .next()
        .expect("successful document theft should recover records");
    let information = fixture
        .state
        .intelligence()
        .get_information(information_id)
        .expect("acquired records should persist");
    assert_eq!(
        information.holder(),
        KnowledgeHolder::Organization(fixture.crew)
    );
    assert_eq!(
        information.source_kind(),
        InformationSourceKind::AcquiredRecords
    );
    assert_eq!(information.topic(), InformationTopic::MarketAccess);
    assert_eq!(information.subject(), EntityRef::Business(fixture.target));
    assert!(
        information
            .summary()
            .contains("professional record handling")
    );
    assert!(resolution.discovery_signatures().contains(&(
        InformationTopic::MarketAccess,
        EntityRef::Business(fixture.target),
        None,
    )));
    let after_action = fixture
        .state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("after-action information should persist");
    assert!(
        after_action
            .summary()
            .contains("Document theft recovered 1 usable record set")
    );
    validate_state(&fixture.state).expect("document theft state should validate");
    validate_invariants(&fixture.state);
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("document theft state should save"),
    )
    .expect("document theft state should restore");
}

#[test]
fn document_theft_resolution_rejects_changed_target_business() {
    let mut fixture = fixture();
    let operation = authorize(&mut fixture);
    let plan = decide_due_resolution(&mut fixture, operation);
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.target,
        BusinessOwner::Organization(fixture.crew),
    )
    .expect("ownership transfer should validate")
    .commit(&mut fixture.state)
    .expect("ownership transfer should commit");

    assert_eq!(
        validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
            .err()
            .expect("changed target must stale the document-theft plan"),
        OperationResolutionError::InformationAcquisition(
            InformationAcquisitionError::DocumentTheft(DocumentTheftError::StaleTarget(
                fixture.target
            ))
        )
    );
    assert!(
        fixture
            .state
            .operations()
            .get_operation(operation)
            .expect("stale operation should remain present")
            .resolution()
            .is_none()
    );
    validate_state(&fixture.state).expect("stale-plan rejection should preserve valid state");
}

#[test]
fn document_theft_does_not_steal_records_from_a_business_acquired_mid_execution() {
    let mut fixture = fixture();
    let operation = authorize(&mut fixture);
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![operation]);
    validate_transfer_business_ownership(
        &fixture.state,
        fixture.target,
        BusinessOwner::Organization(fixture.crew),
    )
    .expect("independent target should be transferable to the crew")
    .commit(&mut fixture.state)
    .expect("target ownership transfer should commit");
    fixture.state.advance_clock(SimDuration::from_minutes(30));
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due document theft should still produce a practical-failure plan");
    validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .expect("ownership-blocked document theft should validate")
        .commit(&mut fixture.state)
        .expect("ownership-blocked document theft should commit");

    let resolution = fixture
        .state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("started document theft should persist its practical failure");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert_eq!(
        resolution.objective_blocker(),
        Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
    );
    assert!(resolution.discovered_information().is_empty());
    assert!(resolution.property_proceeds().is_none());
    let after_action = fixture
        .state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("after-action information should persist");
    assert!(
        after_action
            .summary()
            .contains("stealing its own files would no longer acquire outside information")
    );
    validate_state(&fixture.state).expect("ownership-blocked state should validate");
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("ownership-blocked document theft should save"),
    )
    .expect("ownership-blocked document theft should restore");
}

#[test]
fn achieved_records_include_available_books_while_partial_records_do_not_overclaim_them() {
    let snapshot = DocumentTargetSnapshot {
        business_version: 1,
        name: "Books Office".to_owned(),
        functions: BTreeSet::from([BusinessFunction::ProfessionalRecords]),
        latest_financials: Some(DocumentFinancialSnapshot {
            occurred_at: SimTime::from_minutes(30),
            gross_revenue: Money::from_cents(125_000),
            operating_cost: Money::from_cents(75_000),
            net_cash: Money::from_cents(50_000),
        }),
    };
    let achieved = build_observations(&snapshot, OperationObjectiveOutcome::Achieved);
    assert_eq!(achieved.len(), 2);
    assert_eq!(achieved[0].topic, InformationTopic::MarketAccess);
    assert_eq!(achieved[1].topic, InformationTopic::FinancialPerformance);
    assert_eq!(achieved[1].reliability, Reliability::DirectAccess);
    assert_eq!(achieved[1].specificity, Specificity::Precise);
    assert!(achieved[1].summary.contains("$1,250.00 gross revenue"));
    assert!(achieved[1].summary.contains("$500.00 net cash"));

    let partial = build_observations(&snapshot, OperationObjectiveOutcome::Partial);
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].topic, InformationTopic::MarketAccess);
    assert_eq!(partial[0].reliability, Reliability::GenerallyReliable);
    assert_eq!(partial[0].specificity, Specificity::Specific);
    assert!(build_observations(&snapshot, OperationObjectiveOutcome::Failed).is_empty());
}
