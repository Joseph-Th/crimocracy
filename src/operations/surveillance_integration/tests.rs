//! Focused tests for surveillance intelligence discovery and police-visibility boundaries.

use super::*;
use crate::build_registry;
use crate::core::id::EvidenceId;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::SimDuration;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    CaseActivitySignal, InformationDraft, InformationSignal, InformationSourceKind,
    KnowledgeHolder, PatrolIntervalSignal, Reliability, Specificity,
};
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{
    InvestigationTransition, apply_cold_case_decay, validate_add_evidence,
    validate_incident_intake, validate_open_investigation, validate_transition_investigation,
};
use crate::legal::jurisdiction_system::validate_set_jurisdiction;
use crate::legal::patrol_system::validate_establish_patrol_deployment;
use crate::legal::{
    Admissibility, ArrestDraft, DayMinute, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft,
    JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow,
};
use crate::operations::operation_execution::{
    OperationResolutionError, OperationResolutionRandomness, decide_operation_resolution,
    resolve_intelligence_factors, validate_operation_resolution_plan,
};
use crate::operations::operation_system::{OperationError, validate_authorize_operation};
use crate::operations::{
    OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
};
use crate::registry::Registry;
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
    validate_reassign_character,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind,
};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    crew: OrganizationId,
    observer: CharacterId,
    entry_specialist: CharacterId,
    police: OrganizationId,
    neighborhood: NeighborhoodId,
    business: BusinessId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating should validate")
}

fn fixture(observer_skill: u8, with_patrol: bool) -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0x5A11_1933);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Northside Observation Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Northside Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Northside Market".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(55),
                    commercial_activity: rating(75),
                    illicit_demand: rating(60),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(55),
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
            case_intake_priority: rating(80),
        },
    )
    .expect("jurisdiction should validate")
    .commit(&mut state)
    .expect("jurisdiction should commit");
    if with_patrol {
        validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![
                    PatrolWindow::try_new(
                        DayMinute::try_new(120).expect("fixture minute should validate"),
                        120,
                        rating(80),
                    )
                    .expect("fixture patrol window should validate"),
                    PatrolWindow::try_new(
                        DayMinute::try_new(1_320).expect("fixture minute should validate"),
                        120,
                        rating(60),
                    )
                    .expect("fixture patrol window should validate"),
                ],
            },
        )
        .expect("patrol should validate")
        .commit(&mut state)
        .expect("patrol should commit");
    }
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Market Social Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
                BusinessFunction::Warehousing,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business should validate");
    let observer = insert_character(
        &mut state,
        CharacterDraft {
            name: "Mara Vale".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, rating(observer_skill)),
                (CapabilityKind::Management, rating(observer_skill)),
                (CapabilityKind::Stealth, rating(observer_skill)),
                (CapabilityKind::Burglary, rating(observer_skill)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("observer should validate");
    let entry_specialist = insert_character(
        &mut state,
        CharacterDraft {
            name: "Nora Quill".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Burglary, rating(observer_skill))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("entry specialist should validate");
    Fixture {
        registry,
        state,
        crew,
        observer,
        entry_specialist,
        police,
        neighborhood,
        business,
    }
}

fn authorize_surveillance(fixture: &mut Fixture, target: EntityRef) -> OperationId {
    authorize_surveillance_for(fixture, fixture.crew, fixture.observer, target)
}

fn authorize_surveillance_for(
    fixture: &mut Fixture,
    organization: OrganizationId,
    observer: CharacterId,
    target: EntityRef,
) -> OperationId {
    validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Observe target".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: organization,
            leader: observer,
            objective: OperationObjective::GatherInformation { target },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, observer)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("surveillance should validate")
    .commit(&mut fixture.state)
    .expect("surveillance should commit")
}

fn resolve_with_zero_variance(fixture: &mut Fixture, operation: OperationId) {
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![operation]);
    fixture.state.advance_clock(SimDuration::from_minutes(120));
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("due surveillance should produce a resolution plan");
    validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .expect("fresh surveillance plan should validate")
        .commit(&mut fixture.state)
        .expect("validated surveillance should commit");
}

fn detain_character_for_surveillance_test(
    fixture: &mut Fixture,
    character: CharacterId,
    title: &str,
) {
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: title.to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(character)]),
        },
    )
    .expect("custody investigation should validate")
    .commit(&mut fixture.state)
    .expect("custody investigation should commit");
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("custody evidence should validate")
    .commit(&mut fixture.state)
    .expect("custody evidence should commit");
    let corroborating = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("corroborating custody evidence should validate")
    .commit(&mut fixture.state)
    .expect("corroborating custody evidence should commit");
    validate_arrest(
        &fixture.registry,
        &fixture.state,
        ArrestDraft {
            character,
            investigation,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("evidence-backed detention should validate")
    .commit(&mut fixture.state)
    .expect("evidence-backed detention should commit");
}

#[test]
fn achieved_business_surveillance_creates_actionable_patrol_and_access_intelligence() {
    let mut fixture = fixture(100, true);
    let business = fixture.business;
    let surveillance = authorize_surveillance(&mut fixture, EntityRef::Business(business));
    resolve_with_zero_variance(&mut fixture, surveillance);

    let resolution = fixture
        .state
        .operations()
        .get_operation(surveillance)
        .and_then(|record| record.resolution())
        .expect("surveillance should resolve");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    assert_eq!(resolution.discovered_information().len(), 2);

    let discovered = resolution
        .discovered_information()
        .iter()
        .map(|information| {
            fixture
                .state
                .intelligence()
                .get_information(*information)
                .expect("discovered information should persist")
        })
        .collect::<Vec<_>>();
    for information in &discovered {
        assert_eq!(
            information.holder(),
            KnowledgeHolder::Organization(fixture.crew)
        );
        assert_eq!(
            information.source_kind(),
            InformationSourceKind::Surveillance
        );
        assert_eq!(
            information.source_entity(),
            Some(EntityRef::Operation(surveillance))
        );
        assert_eq!(information.reliability(), Reliability::GenerallyReliable);
        assert_eq!(information.specificity(), Specificity::Specific);
        assert_eq!(
            fixture
                .state
                .operations()
                .operation_for_discovered_information(information.id())
                .map(|record| record.id()),
            Some(surveillance)
        );
    }
    let police = discovered
        .iter()
        .find(|information| information.topic() == InformationTopic::PoliceActivity)
        .expect("business surveillance should discover police activity");
    assert_eq!(
        police.subject(),
        EntityRef::Neighborhood(fixture.neighborhood)
    );
    assert_eq!(
        police.signal(),
        Some(&InformationSignal::PatrolPattern {
            intervals: BTreeSet::from([
                PatrolIntervalSignal::try_new(120, 240).expect("fixture interval must validate"),
                PatrolIntervalSignal::try_new(1_320, 1_440)
                    .expect("fixture interval must validate"),
            ]),
        })
    );
    assert!(police.summary().contains("recurring pattern"));
    assert!(police.summary().contains("roughly 02:00-04:00"));
    assert!(!police.summary().contains("patrol-deployment"));

    let access = discovered
        .iter()
        .find(|information| information.topic() == InformationTopic::MarketAccess)
        .expect("achieved business surveillance should discover venue access");
    assert_eq!(access.subject(), EntityRef::Business(fixture.business));
    assert!(access.summary().contains("regular customer access"));
    assert!(access.summary().contains("private meeting space"));
    assert!(access.summary().contains("storage space"));

    let after_action = fixture
        .state
        .intelligence()
        .get_information(resolution.after_action_information())
        .expect("after-action information should persist");
    assert!(after_action.summary().contains(
    "Surveillance produced 2 usable target observations: police activity around Northside Market; access intelligence at Market Social Club."
  ));

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("surveillance discoveries should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored =
        restore_save(&fixture.registry, decoded).expect("surveillance discoveries should restore");
    for information in resolution.discovered_information() {
        assert_eq!(
            restored
                .operations()
                .operation_for_discovered_information(*information)
                .map(|record| record.id()),
            Some(surveillance)
        );
    }

    let police_information = police.id();
    let access_information = access.id();
    let burglary = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Use surveillance for entry planning".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: fixture.crew,
            leader: fixture.observer,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(fixture.business),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, fixture.observer),
                (RoleKind::EntrySpecialist, fixture.entry_specialist),
            ]),
            intelligence: BTreeSet::from([police_information, access_information]),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("fresh surveillance intelligence should be valid burglary planning input")
    .commit(&mut fixture.state)
    .expect("intelligence-backed burglary should commit");
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert!(start.started_operations.contains(&burglary));
    let (quality, adjustment, covered, relevant) =
        resolve_intelligence_factors(&fixture.registry, &fixture.state, burglary);
    assert!(quality.value() > 0);
    assert!(adjustment < 0);
    assert!(covered >= 2);
    assert!(covered < relevant);
    validate_state(&fixture.state).expect("surveillance-backed planning state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn foreign_operation_surveillance_requires_prior_organization_knowledge() {
    let mut fixture = fixture(100, false);
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Observed Rival Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival organization should validate");
    let rival_observer = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival Observer".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(100))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("rival observer should validate");
    let business = fixture.business;
    let rival_operation = authorize_surveillance_for(
        &mut fixture,
        rival,
        rival_observer,
        EntityRef::Business(business),
    );

    let unknown = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Probe unknown rival operation".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.crew,
            leader: fixture.observer,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Operation(rival_operation),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("a raw rival operation id must not reveal hidden operation existence");
    assert_eq!(
        unknown,
        OperationError::MissingEntity(EntityRef::Operation(rival_operation))
    );

    validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.crew),
            source_kind: InformationSourceKind::StreetRumor,
            topic: InformationTopic::OperationalOutcome,
            source_entity: None,
            subject: EntityRef::Operation(rival_operation),
            observed_at: fixture.state.now(),
            reliability: Reliability::Unreliable,
            specificity: Specificity::General,
            summary: "Street sources identified a rival field operation.".to_owned(),
        },
    )
    .expect("organization-held rival-operation knowledge should validate")
    .commit(&mut fixture.state)
    .expect("organization-held rival-operation knowledge should commit");
    authorize_surveillance(&mut fixture, EntityRef::Operation(rival_operation));
    validate_state(&fixture.state).expect("known rival-operation surveillance should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn achieved_surveillance_carries_every_observed_patrol_window_in_typed_signal() {
    let mut fixture = fixture(100, false);
    let windows = [60_u16, 180, 300, 420, 540]
        .into_iter()
        .map(|start| {
            PatrolWindow::try_new(
                DayMinute::try_new(start).expect("fixture minute should validate"),
                30,
                rating(80),
            )
            .expect("fixture patrol window should validate")
        })
        .collect::<Vec<_>>();
    validate_establish_patrol_deployment(
        &fixture.state,
        PatrolDeploymentDraft {
            organization: fixture.police,
            neighborhood: fixture.neighborhood,
            windows,
        },
    )
    .expect("five-window patrol should validate")
    .commit(&mut fixture.state)
    .expect("five-window patrol should commit");

    let neighborhood = fixture.neighborhood;
    let surveillance = authorize_surveillance(&mut fixture, EntityRef::Neighborhood(neighborhood));
    resolve_with_zero_variance(&mut fixture, surveillance);
    let resolution = fixture
        .state
        .operations()
        .get_operation(surveillance)
        .and_then(|record| record.resolution())
        .expect("neighborhood surveillance should resolve");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    let information = fixture
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("patrol observation should persist");
    let expected = [60_u16, 180, 300, 420, 540]
        .into_iter()
        .map(|start| {
            PatrolIntervalSignal::try_new(start, start + 30)
                .expect("expected patrol interval should validate")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        information.signal(),
        Some(&InformationSignal::PatrolPattern {
            intervals: expected
        })
    );
    assert!(information.summary().contains("roughly 09:00-09:30"));
    assert!(
        !information
            .summary()
            .contains("additional recurring window")
    );
    validate_state(&fixture.state).expect("complete patrol signal state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn achieved_surveillance_keeps_short_and_wrapped_patrol_windows_bounded() {
    let mut fixture = fixture(100, false);
    validate_establish_patrol_deployment(
        &fixture.state,
        PatrolDeploymentDraft {
            organization: fixture.police,
            neighborhood: fixture.neighborhood,
            windows: vec![
                PatrolWindow::try_new(
                    DayMinute::try_new(721).expect("short-window start should validate"),
                    5,
                    rating(80),
                )
                .expect("short patrol window should validate"),
                PatrolWindow::try_new(
                    DayMinute::try_new(1_430).expect("wrapped-window start should validate"),
                    40,
                    rating(60),
                )
                .expect("wrapped patrol window should validate"),
            ],
        },
    )
    .expect("short and wrapped patrol schedule should validate")
    .commit(&mut fixture.state)
    .expect("short and wrapped patrol schedule should commit");

    let neighborhood = fixture.neighborhood;
    let surveillance = authorize_surveillance(&mut fixture, EntityRef::Neighborhood(neighborhood));
    resolve_with_zero_variance(&mut fixture, surveillance);
    let resolution = fixture
        .state
        .operations()
        .get_operation(surveillance)
        .and_then(|record| record.resolution())
        .expect("neighborhood surveillance should resolve");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    let information = fixture
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("patrol observation should persist");
    assert_eq!(
        information.signal(),
        Some(&InformationSignal::PatrolPattern {
            intervals: BTreeSet::from([
                PatrolIntervalSignal::try_new(0, 30).expect("wrapped patrol head should validate"),
                PatrolIntervalSignal::try_new(720, 750)
                    .expect("short patrol approximation should validate"),
                PatrolIntervalSignal::try_new(1_410, 1_440)
                    .expect("wrapped patrol tail should validate"),
            ]),
        })
    );
    assert!(information.summary().contains("roughly 12:00-12:30"));
    assert!(information.summary().contains("roughly 23:30-00:30"));
    assert!(
        !information.summary().contains("all day"),
        "a short rounded patrol window must never inflate into all-day coverage"
    );
    validate_state(&fixture.state).expect("bounded patrol approximation should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn organization_surveillance_resolution_stales_when_visible_member_is_detained_after_planning() {
    let mut fixture = fixture(100, false);
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Custody Snapshot Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival should validate");
    let member = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Visible Before Custody".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("rival member should validate");
    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![operation]);
    fixture.state.advance_clock(SimDuration::from_minutes(120));
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("surveillance plan should capture the currently visible member");

    detain_character_for_surveillance_test(
        &mut fixture,
        member,
        "Custody snapshot staleness inquiry",
    );
    let error = validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .err()
        .expect("detention must stale the organization personnel snapshot");
    assert_eq!(
        error,
        OperationResolutionError::Surveillance(SurveillanceError::StaleTarget(
            EntityRef::Organization(rival)
        ))
    );
    assert_eq!(
        fixture
            .state
            .operations()
            .get_operation(operation)
            .expect("stale surveillance should remain in progress")
            .status(),
        OperationStatus::InProgress
    );
    validate_state(&fixture.state).expect("stale detention rejection should preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn partial_and_failed_surveillance_degrade_or_withhold_target_knowledge() {
    let rival_registry = build_registry();
    let mut partial = fixture(35, false);
    let rival = insert_organization(
        &rival_registry,
        &mut partial.state,
        OrganizationDraft {
            name: "Dock Rival".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival should validate");
    let target = insert_character(
        &mut partial.state,
        CharacterDraft {
            name: "Nico Hart".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("target should validate");
    let operation = authorize_surveillance(&mut partial, EntityRef::Character(target));
    resolve_with_zero_variance(&mut partial, operation);
    let resolution = partial
        .state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("partial surveillance should resolve");
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Partial
    );
    assert_eq!(resolution.discovered_information().len(), 1);
    let information = partial
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("partial surveillance information should persist");
    assert_eq!(information.reliability(), Reliability::Mixed);
    assert_eq!(information.specificity(), Specificity::General);

    let mut failed = fixture(0, false);
    let failed_target = insert_character(
        &mut failed.state,
        CharacterDraft {
            name: "Unresolved Target".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("failed target should validate");
    let failed_operation = authorize_surveillance(&mut failed, EntityRef::Character(failed_target));
    resolve_with_zero_variance(&mut failed, failed_operation);
    let failed_resolution = failed
        .state
        .operations()
        .get_operation(failed_operation)
        .and_then(|record| record.resolution())
        .expect("failed surveillance should resolve");
    assert_eq!(
        failed_resolution.objective_outcome(),
        OperationObjectiveOutcome::Failed
    );
    assert!(failed_resolution.discovered_information().is_empty());
    let after_action = failed
        .state
        .intelligence()
        .get_information(failed_resolution.after_action_information())
        .expect("failed surveillance should still produce after-action information");
    assert!(
        after_action
            .summary()
            .contains("no target observation reliable enough for planning")
    );
    validate_state(&partial.state).expect("partial surveillance state should validate");
    validate_state(&failed.state).expect("failed surveillance state should validate");
    validate_invariants(&partial.state);
    validate_invariants(&failed.state);
}

#[test]
fn surveillance_resolution_rejects_target_change_after_planning() {
    let mut fixture = fixture(100, false);
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Moving Target Group".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival should validate");
    let target = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Changing Subject".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("target should validate");
    let operation = authorize_surveillance(&mut fixture, EntityRef::Character(target));
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![operation]);
    fixture.state.advance_clock(SimDuration::from_minutes(120));
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .expect("surveillance plan should resolve against current target state");

    validate_reassign_character(&fixture.state, target, None, None)
        .expect("target reassignment should validate")
        .commit(&mut fixture.state)
        .expect("target reassignment should commit");
    let error = validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .err()
        .expect("target change must stale surveillance resolution");
    assert_eq!(
        error,
        OperationResolutionError::Surveillance(SurveillanceError::StaleTarget(
            EntityRef::Character(target)
        ))
    );
    assert_eq!(
        fixture
            .state
            .operations()
            .get_operation(operation)
            .expect("stale surveillance should remain present")
            .status(),
        OperationStatus::InProgress
    );
    validate_state(&fixture.state).expect("stale surveillance rejection should preserve state");
    validate_invariants(&fixture.state);
}

#[test]
fn investigation_surveillance_reports_visible_case_activity_without_evidence_graph_leakage() {
    let mut fixture = fixture(100, false);
    let suspect = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Hidden Case Subject".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("suspect should validate");
    let investigation = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Harbor Ledger Inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(suspect)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut fixture.state)
    .expect("investigation should commit");
    validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("hidden case evidence should validate")
    .commit(&mut fixture.state)
    .expect("hidden case evidence should commit");
    let hidden_target = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Probe unknown case".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.crew,
            leader: fixture.observer,
            objective: OperationObjective::GatherInformation {
                target: EntityRef::Investigation(investigation),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("a raw hidden investigation id must not be a surveillance existence oracle");
    assert_eq!(
        hidden_target,
        OperationError::MissingEntity(EntityRef::Investigation(investigation))
    );
    validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.crew),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Organization(fixture.police)),
            subject: EntityRef::Investigation(investigation),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::General,
            summary: "Observers identified the Harbor Ledger Inquiry as an active police file."
                .to_owned(),
        },
    )
    .expect("organization-held case knowledge should validate")
    .commit(&mut fixture.state)
    .expect("organization-held case knowledge should commit");
    let operation = authorize_surveillance(&mut fixture, EntityRef::Investigation(investigation));
    resolve_with_zero_variance(&mut fixture, operation);
    let resolution = fixture
        .state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("investigation surveillance should resolve");
    assert_eq!(resolution.discovered_information().len(), 1);
    let information = fixture
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("legal-activity observation should persist");
    assert_eq!(information.topic(), InformationTopic::LegalActivity);
    assert_eq!(
        information.signal(),
        Some(&InformationSignal::CaseActivity(CaseActivitySignal::Active))
    );
    assert_eq!(
        information.subject(),
        EntityRef::Investigation(investigation)
    );
    assert!(information.summary().contains("Harbor Ledger Inquiry"));
    assert!(information.summary().contains("Northside Precinct"));
    assert!(!information.summary().contains("Hidden Case Subject"));
    assert!(!information.summary().contains("Document"));
    validate_state(&fixture.state).expect("investigation surveillance state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn law_enforcement_org_surveillance_reports_case_heat_and_shelved_close_without_leaks() {
    let mut fixture = fixture(100, false);
    let business = fixture.business;
    let police = fixture.police;
    let incident = authorize_surveillance(&mut fixture, EntityRef::Business(business));
    // Resolve the originating surveillance to terminal state so it does not also start when
    // the later re-check surveillance run_tick fires; the fixture has no patrol deployment, so
    // its resolution creates no exposure case.
    resolve_with_zero_variance(&mut fixture, incident);
    let case = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: fixture.police,
            title: "Crew Incident Inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(incident)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Operation(incident),
                origin: Some(EntityRef::Operation(incident)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(incident)),
            witness: None,
        },
    )
    .expect("incident intake should validate")
    .commit(&mut fixture.state)
    .expect("incident intake should commit")
    .investigation;

    // While the case is active, police-organization surveillance reports the case heat
    // without revealing the evidence graph or internal case details.
    let hot_surveillance = authorize_surveillance(&mut fixture, EntityRef::Organization(police));
    resolve_with_zero_variance(&mut fixture, hot_surveillance);
    let hot_resolution = fixture
        .state
        .operations()
        .get_operation(hot_surveillance)
        .and_then(|record| record.resolution())
        .expect("hot surveillance should resolve");
    assert_eq!(hot_resolution.discovered_information().len(), 1);
    let hot_observation = fixture
        .state
        .intelligence()
        .get_information(
            *hot_resolution
                .discovered_information()
                .iter()
                .next()
                .unwrap(),
        )
        .expect("case-heat observation should persist");
    assert_eq!(hot_observation.topic(), InformationTopic::LegalActivity);
    assert_eq!(hot_observation.subject(), EntityRef::Organization(police));
    assert_eq!(
        hot_observation.signal(),
        Some(&InformationSignal::CaseActivity(CaseActivitySignal::Active))
    );
    assert!(
        hot_observation
            .summary()
            .contains("actively developing the case")
    );
    assert!(!hot_observation.summary().contains("Crew Incident Inquiry"));
    assert!(!hot_observation.summary().contains("Surveillance"));

    // A passing of the authored cold window deterministically shelves the case, and a fresh
    // police-organization surveillance then reports the matter has gone quiet.
    fixture.state.advance_clock(SimDuration::from_minutes(121));
    let suspended = apply_cold_case_decay(&mut fixture.state, SimDuration::from_minutes(120))
        .expect("cold-case decay should resolve");
    assert_eq!(suspended.suspended, vec![case]);
    assert!(suspended.closed.is_empty());
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(case)
            .expect("cold case should persist")
            .status(),
        InvestigationStatus::Suspended
    );
    validate_state(&fixture.state).expect("cold-case decay state should validate");
    validate_invariants(&fixture.state);

    let cold_surveillance = authorize_surveillance(&mut fixture, EntityRef::Organization(police));
    resolve_with_zero_variance(&mut fixture, cold_surveillance);
    let cold_resolution = fixture
        .state
        .operations()
        .get_operation(cold_surveillance)
        .and_then(|record| record.resolution())
        .expect("recheck surveillance should resolve");
    let cold_observation = fixture
        .state
        .intelligence()
        .get_information(
            *cold_resolution
                .discovered_information()
                .iter()
                .next()
                .unwrap(),
        )
        .expect("shelved observation should persist");
    assert_eq!(
        cold_observation.signal(),
        Some(&InformationSignal::CaseActivity(
            CaseActivitySignal::Shelved
        ))
    );
    assert!(cold_observation.summary().contains("shelved"));
    assert!(
        !cold_observation
            .summary()
            .contains("actively developing the case")
    );
    validate_state(&fixture.state).expect("shelved recheck state should validate");
    validate_invariants(&fixture.state);

    validate_transition_investigation(&fixture.state, case, InvestigationTransition::Close)
        .expect("shelved known case should be closable")
        .commit(&mut fixture.state)
        .expect("known case closure should commit");
    let closed_surveillance = authorize_surveillance(&mut fixture, EntityRef::Organization(police));
    resolve_with_zero_variance(&mut fixture, closed_surveillance);
    let closed_resolution = fixture
        .state
        .operations()
        .get_operation(closed_surveillance)
        .and_then(|record| record.resolution())
        .expect("closed-case recheck should resolve");
    let closed_observation = fixture
        .state
        .intelligence()
        .get_information(
            *closed_resolution
                .discovered_information()
                .iter()
                .next()
                .expect("closed-case recheck should discover one observation"),
        )
        .expect("closed-case observation should persist");
    assert_eq!(
        closed_observation.signal(),
        Some(&InformationSignal::CaseActivity(CaseActivitySignal::Closed))
    );
    assert!(closed_observation.summary().contains("closed"));
    validate_state(&fixture.state).expect("closed-case recheck state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn resumed_case_does_not_grant_unrelated_organization_authority_sightline() {
    let mut fixture = fixture(100, false);
    let police = fixture.police;
    let business = fixture.business;
    let incident = authorize_surveillance(&mut fixture, EntityRef::Business(business));
    resolve_with_zero_variance(&mut fixture, incident);

    let case = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: police,
            title: "Initial crew incident".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(incident)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Operation(incident),
                origin: Some(EntityRef::Operation(incident)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(incident)),
            witness: None,
        },
    )
    .expect("initial incident intake should validate")
    .commit(&mut fixture.state)
    .expect("initial incident intake should commit")
    .investigation;

    fixture.state.advance_clock(SimDuration::from_minutes(121));
    let decay = apply_cold_case_decay(&mut fixture.state, SimDuration::from_minutes(120))
        .expect("cold-case decay should suspend the inactive case");
    assert_eq!(decay.suspended, vec![case]);

    let second_crew = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Southside Observation Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second crew should validate");
    let second_observer = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Iris Venn".to_owned(),
            organization: Some(second_crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, rating(100)),
                (CapabilityKind::Management, rating(100)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second observer should validate");

    let resumed = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: police,
            title: "Continued crew incident".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(incident)]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Operation(incident),
                origin: Some(EntityRef::Operation(incident)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(incident)),
            witness: None,
        },
    )
    .expect("continued incident should find and validate the suspended shelf")
    .commit(&mut fixture.state)
    .expect("continued incident should resume the shelf");
    assert_eq!(resumed.investigation, case);
    assert!(resumed.resumed_shelf);

    let recheck = authorize_surveillance_for(
        &mut fixture,
        second_crew,
        second_observer,
        EntityRef::Organization(police),
    );
    resolve_with_zero_variance(&mut fixture, recheck);
    let resolution = fixture
        .state
        .operations()
        .get_operation(recheck)
        .and_then(|record| record.resolution())
        .expect("second crew authority surveillance should resolve");
    assert_eq!(resolution.discovered_information().len(), 1);
    let observation = fixture
        .state
        .intelligence()
        .get_information(
            *resolution
                .discovered_information()
                .iter()
                .next()
                .expect("authority recheck should discover case activity"),
        )
        .expect("authority case-activity observation should persist");
    assert_eq!(
        observation.holder(),
        KnowledgeHolder::Organization(second_crew)
    );
    assert_eq!(observation.topic(), InformationTopic::Personnel);
    assert_eq!(observation.subject(), EntityRef::Organization(police));
    assert!(
        !matches!(
            observation.signal(),
            Some(InformationSignal::CaseActivity(_))
        ),
        "another organization cannot inherit case visibility merely because a shelf was resumed"
    );

    validate_state(&fixture.state)
        .expect("resumed unrelated-case sightline should remain structurally valid");
    validate_invariants(&fixture.state);
}

#[test]
fn partial_authority_sightline_carries_no_definitive_case_activity_signal() {
    assert_eq!(
        authority_sightline_signal(
            CaseActivitySignal::Active,
            OperationObjectiveOutcome::Partial
        ),
        None
    );
    assert_eq!(
        authority_sightline_signal(
            CaseActivitySignal::Shelved,
            OperationObjectiveOutcome::Partial
        ),
        None
    );
    assert_eq!(
        authority_sightline_signal(
            CaseActivitySignal::Active,
            OperationObjectiveOutcome::Achieved
        ),
        Some(CaseActivitySignal::Active)
    );
    assert_eq!(
        authority_sightline_signal(
            CaseActivitySignal::Shelved,
            OperationObjectiveOutcome::Achieved
        ),
        Some(CaseActivitySignal::Shelved)
    );
    assert_eq!(
        authority_sightline_signal(
            CaseActivitySignal::Closed,
            OperationObjectiveOutcome::Achieved
        ),
        Some(CaseActivitySignal::Closed)
    );
}

#[test]
fn surveillance_authorization_rejects_semantically_invalid_objectives_and_targets() {
    let fixture = fixture(80, false);
    let invalid_objective = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Not actually surveillance".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.crew,
            leader: fixture.observer,
            objective: OperationObjective::Frighten {
                target: EntityRef::Business(fixture.business),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("surveillance must require a gather-information objective");
    assert_eq!(
        invalid_objective,
        OperationError::InvalidSurveillanceObjective
    );

    let evidence = EntityRef::Evidence(EvidenceId::from_raw(9_999));
    let unsupported = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Observe evidence record".to_owned(),
            kind: OperationKind::Surveillance,
            responsible_organization: fixture.crew,
            leader: fixture.observer,
            objective: OperationObjective::GatherInformation { target: evidence },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect_err("evidence records are not directly observable operation targets");
    assert_eq!(
        unsupported,
        OperationError::UnsupportedSurveillanceTarget(evidence)
    );
    assert_eq!(
        fixture
            .state
            .operations()
            .operations_for_organization(fixture.crew)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn police_org_surveillance_without_notified_case_produces_personnel_and_survives_later_notification()
 {
    let mut fixture = fixture(90, false);
    let police = fixture.police;

    // No investigation has ever been notified to the crew, so there is no sightline to
    // re-read: the observation must fall back to ordinary personnel intelligence.
    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(police));
    resolve_with_zero_variance(&mut fixture, operation);
    let resolution = fixture
        .state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("police-org surveillance should resolve");
    assert_eq!(resolution.discovered_information().len(), 1);
    let observation = fixture
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("personnel observation should persist");
    assert_eq!(observation.topic(), InformationTopic::Personnel);
    assert_eq!(observation.subject(), EntityRef::Organization(police));
    assert_eq!(observation.signal(), None);
    assert!(
        observation
            .summary()
            .contains("did not identify a recurring active affiliate")
    );
    assert!(resolution.surveillance_signatures().contains(&(
        observation.topic(),
        observation.subject(),
        observation.signal().cloned(),
    )));
    let observation_id = observation.id();
    let observation_signal = observation.signal().cloned();
    validate_state(&fixture.state)
        .expect("no-sightline police-org surveillance state should validate");
    validate_invariants(&fixture.state);

    // A case originating from the crew only after the resolution must not retroactively
    // invalidate the honestly-produced observation: the signature set was frozen on resolution.
    let intake = crate::legal::investigation_system::validate_incident_intake(
        &fixture.state,
        crate::legal::IncidentIntakeDraft {
            owner: police,
            title: "Later Notified Case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(operation)]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Operation(operation),
                origin: Some(EntityRef::Operation(operation)),
                kind: crate::legal::EvidenceKind::Surveillance,
                strength: crate::legal::EvidenceStrength::Weak,
                reliability: crate::legal::EvidenceReliability::Questionable,
                admissibility: crate::legal::Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(operation)),
            witness: None,
        },
    )
    .expect("later incident intake should validate")
    .commit(&mut fixture.state)
    .expect("later incident intake should commit");
    let _ = intake.investigation;
    validate_state(&fixture.state)
        .expect("later case creation must not invalidate persisted surveillance signatures");
    validate_invariants(&fixture.state);

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("typed personnel surveillance should save after later case creation");
    let bytes = bincode::serialize(&envelope).expect("typed personnel save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("typed personnel save should deserialize");
    let restored = restore_save(&fixture.registry, decoded)
        .expect("typed personnel surveillance should restore with frozen semantics");
    let restored_observation = restored
        .intelligence()
        .get_information(observation_id)
        .expect("restored personnel observation should persist");
    assert_eq!(restored_observation.signal(), observation_signal.as_ref());
    validate_invariants(&restored);
}

#[test]
fn organization_surveillance_carries_visible_member_ids_and_excludes_detainees() {
    let mut fixture = fixture(100, false);
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Typed Personnel Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("rival organization should validate");
    let member_names = ["Ada Pike", "Bram Vale", "Cleo North", "Dane Moss"];
    let members = member_names.map(|name| {
        insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(rival),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("rival member should validate")
    });

    let detained = members[1];
    detain_character_for_surveillance_test(
        &mut fixture,
        detained,
        "Typed personnel custody inquiry",
    );

    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    resolve_with_zero_variance(&mut fixture, operation);
    let resolution = fixture
        .state
        .operations()
        .get_operation(operation)
        .and_then(|record| record.resolution())
        .expect("organization surveillance should resolve");
    assert_eq!(resolution.discovered_information().len(), 1);
    let observation = fixture
        .state
        .intelligence()
        .get_information(*resolution.discovered_information().iter().next().unwrap())
        .expect("typed personnel observation should persist");
    assert_eq!(observation.topic(), InformationTopic::Personnel);
    assert_eq!(observation.subject(), EntityRef::Organization(rival));
    let expected = BTreeSet::from([members[0], members[2], members[3]]);
    assert_eq!(
        observation.signal(),
        Some(&InformationSignal::PersonnelPresence {
            characters: expected.clone(),
        })
    );
    assert!(observation.summary().contains(member_names[0]));
    assert!(!observation.summary().contains(member_names[1]));
    assert!(observation.summary().contains(member_names[2]));
    assert!(observation.summary().contains(member_names[3]));
    assert!(resolution.surveillance_signatures().contains(&(
        observation.topic(),
        observation.subject(),
        observation.signal().cloned(),
    )));
    let observation_id = observation.id();
    let observation_signal = observation.signal().cloned();
    validate_state(&fixture.state).expect("typed personnel surveillance state should validate");
    validate_invariants(&fixture.state);

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("typed personnel surveillance should save");
    let bytes = bincode::serialize(&envelope).expect("typed personnel save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("typed personnel save should deserialize");
    let restored = restore_save(&fixture.registry, decoded)
        .expect("typed personnel surveillance should restore with frozen semantics");
    let restored_observation = restored
        .intelligence()
        .get_information(observation_id)
        .expect("restored typed personnel observation should persist");
    assert_eq!(restored_observation.signal(), observation_signal.as_ref());
    validate_invariants(&restored);
}
