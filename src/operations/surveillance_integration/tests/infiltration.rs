//! Inside records are a distinct reward for successful infiltration, not passive observation.
use super::*;

fn authorize_infiltration(fixture: &mut Fixture, target: EntityRef) -> OperationId {
    let actor = fixture.observer;
    validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Inside records".to_owned(),
            kind: OperationKind::Infiltration,
            responsible_organization: fixture.crew,
            leader: actor,
            objective: OperationObjective::GatherInformation { target },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, actor)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap()
}

fn complete_infiltration(fixture: &mut Fixture, id: OperationId) {
    let start = run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(start.started_operations, vec![id]);
    fixture.state.advance_clock(
        fixture
            .registry
            .get_operation(OperationKind::Infiltration)
            .execution()
            .duration(),
    );
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        id,
        OperationResolutionRandomness::new(0, 0),
    )
    .unwrap();
    validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
}

#[test]
fn infiltration_business_reveals_internal_records_and_preserves_report_on_restore() {
    let mut fixture = fixture(100, false);
    let business = EntityRef::Business(fixture.business);
    let id = authorize_infiltration(&mut fixture, business);
    complete_infiltration(&mut fixture, id);
    let operation = fixture.state.operations().get_operation(id).unwrap();
    let resolution = operation.resolution().unwrap();
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    let records: Vec<_> = resolution
        .discovered_information()
        .iter()
        .map(|id| fixture.state.intelligence().get_information(*id).unwrap())
        .collect();
    assert!(records.iter().any(|record| record.subject() == business
        && record.topic() == InformationTopic::FinancialPerformance
        && record.summary().contains("independent proprietor")));
    let history = persisted_surveillance_after_action_clause(&fixture.state, operation)
        .unwrap()
        .unwrap();
    assert!(history.contains("Infiltration produced"));
    assert!(history.contains("internal books of Market Social Club"));
    validate_invariants(&fixture.state);
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted_surveillance_after_action_clause(
            &restored,
            restored.operations().get_operation(id).unwrap()
        )
        .unwrap()
        .unwrap(),
        history
    );
}

#[test]
fn infiltration_organization_identifies_internal_leadership_without_police_file_contents() {
    let mut fixture = fixture(100, false);
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Rival Concern".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let leader = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival Executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let target = EntityRef::Organization(rival);
    let id = authorize_infiltration(&mut fixture, target);
    complete_infiltration(&mut fixture, id);
    let operation = fixture.state.operations().get_operation(id).unwrap();
    let resolution = operation.resolution().unwrap();
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    assert!(resolution.discovered_information().iter().any(|id| {
        let info = fixture.state.intelligence().get_information(*id).unwrap();
        info.subject() == EntityRef::Character(leader)
            && info.topic() == InformationTopic::Personnel
            && info.summary().contains("Inside personnel records")
    }));
    assert!(
        super::super::infiltration::internal_findings(
            &fixture.state,
            EntityRef::Organization(fixture.police)
        )
        .is_empty()
    );
    validate_invariants(&fixture.state);
}
