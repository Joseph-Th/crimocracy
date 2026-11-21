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
fn infiltrated_business_books_quote_settled_money_in_player_readable_units() {
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::{
        decide_business_cycle, validate_business_cycle_plan, validate_establish_business_economy,
    };
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};

    let mut fixture = fixture(100, false);
    let owner = FinancialOwner::Business(fixture.business);
    let operating = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::LegitimateOperating,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_business_economy(
        &fixture.registry,
        &fixture.state,
        BusinessEconomyDraft {
            business: fixture.business,
            operating_account: operating,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    fixture.state.advance_clock(
        fixture
            .registry
            .get_business(BusinessKind::Hospitality)
            .economics()
            .cycle(),
    );
    let cycle = validate_business_cycle_plan(
        &fixture.state,
        decide_business_cycle(&fixture.registry, &fixture.state, fixture.business, 0).unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let record = fixture.state.economy().get_cycle(cycle).unwrap();
    let observations = super::super::infiltration::internal_findings(
        &fixture.state,
        EntityRef::Business(fixture.business),
    );
    let financials = observations
        .iter()
        .find(|finding| finding.topic == InformationTopic::FinancialPerformance)
        .expect("a settled inside book should expose an actual operating cycle");
    for (label, cents) in [
        ("gross", record.gross_revenue().cents()),
        ("costs", record.operating_cost().cents()),
        ("net", record.net_cash().cents()),
    ] {
        let amount = crate::finance::helpers::format_money_cents(cents);
        assert!(financials.summary.contains(&format!("{label} {amount}")));
    }
    assert!(!financials.summary.contains(" cents"));
    // Title can return to the exact same owner. Matching the owner alone must not revive
    // an earlier tenure's books as if the returned proprietor had already made a sale.
    for owner in [
        BusinessOwner::Organization(fixture.crew),
        BusinessOwner::Independent,
    ] {
        crate::world::world_system::validate_transfer_business_ownership(
            &fixture.state,
            fixture.business,
            owner,
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    }
    let after_return = super::super::infiltration::internal_findings(
        &fixture.state,
        EntityRef::Business(fixture.business),
    );
    let after_return_financials = after_return
        .iter()
        .find(|finding| finding.topic == InformationTopic::FinancialPerformance)
        .unwrap();
    assert!(
        after_return_financials
            .summary
            .contains("No completed operating cycle"),
        "the owner must not receive historical results from a previous tenure: {}",
        after_return_financials.summary
    );
    let target = EntityRef::Business(fixture.business);
    let operation = authorize_infiltration(&mut fixture, target);
    complete_infiltration(&mut fixture, operation);
    let resolution = fixture
        .state
        .operations()
        .get_operation(operation)
        .unwrap()
        .resolution()
        .unwrap();
    assert!(resolution.discovered_information().iter().any(|id| {
        fixture
            .state
            .intelligence()
            .get_information(*id)
            .unwrap()
            .summary()
            .contains("No completed operating cycle")
    }));
    validate_invariants(&fixture.state);
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
