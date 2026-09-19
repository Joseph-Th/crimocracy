//! Criminal-organization surveillance discovers bounded, provenance-backed enterprise identities.

use super::*;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crate::enterprises::enterprise_execution::{
    validate_establish_enterprise, validate_resume_enterprise, validate_retire_enterprise,
    validate_suspend_enterprise,
};
use crate::enterprises::{EnterpriseDraft, EnterpriseKind};
use crate::finance::finance_system::insert_account;
use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crate::operations::operation_execution::resolve_operation_police_alert_context;

fn make_test_rival(fixture: &mut Fixture) -> (OrganizationId, MandateAuthority) {
    let rival = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Visible Rival".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival Manager".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: rival,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    (
        rival,
        MandateAuthority {
            mandate,
            manager,
            scope,
        },
    )
}

fn make_test_enterprise(
    fixture: &mut Fixture,
    rival: OrganizationId,
    authority: MandateAuthority,
    name: &str,
) -> EnterpriseId {
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: name.to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(50),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(0),
                },
            },
        },
    )
    .unwrap();
    make_test_enterprise_at(
        fixture,
        rival,
        authority,
        EnterpriseLocation::Neighborhood(neighborhood),
    )
}

fn make_test_enterprise_at(
    fixture: &mut Fixture,
    rival: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
) -> EnterpriseId {
    make_test_enterprise_kind_at(
        fixture,
        rival,
        authority,
        location,
        EnterpriseKind::Protection,
    )
}

fn make_test_enterprise_kind_at(
    fixture: &mut Fixture,
    rival: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
    kind: EnterpriseKind,
) -> EnterpriseId {
    let cash = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(rival),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_enterprise(
        &fixture.registry,
        &fixture.state,
        EnterpriseDraft {
            kind,
            organization: rival,
            authority,
            location,
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap()
}

fn make_test_hosted_enterprise(
    fixture: &mut Fixture,
    rival: OrganizationId,
    authority: MandateAuthority,
) -> EnterpriseId {
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Rival Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([BusinessFunction::MeetingSpace]),
            neighborhood: fixture.neighborhood,
            owner: BusinessOwner::Organization(rival),
        },
    )
    .unwrap();
    make_test_enterprise_at(
        fixture,
        rival,
        authority,
        EnterpriseLocation::Business(business),
    )
}

fn surveillance_draft(fixture: &Fixture, target: EntityRef) -> OperationDraft {
    OperationDraft {
        title: "Follow learned enterprise".to_owned(),
        kind: OperationKind::Surveillance,
        responsible_organization: fixture.crew,
        leader: fixture.observer,
        objective: OperationObjective::GatherInformation { target },
        approach: OperationApproach::Covert,
        roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
        intelligence: BTreeSet::new(),
        constraints: Vec::new(),
        contingencies: Vec::new(),
        scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
    }
}

fn assert_unknown_enterprise(fixture: &Fixture, enterprise: EnterpriseId) {
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            surveillance_draft(fixture, EntityRef::Enterprise(enterprise)),
        )
        .expect_err("raw foreign enterprise identity must not grant surveillance access"),
        OperationError::MissingEntity(EntityRef::Enterprise(enterprise)),
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
}

#[test]
fn organization_surveillance_uses_active_enterprise_footprint_for_police_geography() {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    let enterprise = make_test_enterprise(&mut fixture, rival, authority, "Racket Footprint Ward");
    let EnterpriseLocation::Neighborhood(neighborhood) = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise should persist")
        .location()
    else {
        panic!("fixture enterprise should be neighborhood-scoped");
    };
    assert_eq!(
        fixture
            .state
            .world()
            .businesses_owned_by_organization(rival)
            .count(),
        0,
        "the test must prove geography comes from the racket rather than owned real estate"
    );

    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    let alert = resolve_operation_police_alert_context(
        &fixture.registry,
        &fixture.state,
        operation,
        fixture.state.now() + SimDuration::ONE_MINUTE,
    );
    assert_eq!(
        alert.neighborhood(),
        Some(neighborhood),
        "an active neighborhood racket must keep organization-target surveillance geographically attributable"
    );

    validate_suspend_enterprise(&fixture.state, enterprise)
        .expect("active enterprise should suspend")
        .commit(&mut fixture.state)
        .expect("enterprise suspension should commit");
    let suspended_alert = resolve_operation_police_alert_context(
        &fixture.registry,
        &fixture.state,
        operation,
        fixture.state.now() + SimDuration::ONE_MINUTE,
    );
    assert_eq!(
        suspended_alert.neighborhood(),
        None,
        "a suspended racket is historical context, not a current operating venue"
    );
    validate_state(&fixture.state)
        .expect("enterprise-backed surveillance geography should stay valid");
    validate_invariants(&fixture.state);
}

#[test]
fn achieved_organization_surveillance_discovers_first_three_active_enterprises_and_unlocks_followup()
 {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    // Names deliberately disagree with identity order. Inactive low IDs must not consume slots.
    let names = [
        "Suspended Ward",
        "Zulu Ward",
        "Retired Ward",
        "Yarrow Ward",
        "Xenia Ward",
        "Alpha Ward",
    ];
    let enterprises = names.map(|name| make_test_enterprise(&mut fixture, rival, authority, name));
    for enterprise in [enterprises[0], enterprises[2]] {
        validate_suspend_enterprise(&fixture.state, enterprise)
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap();
    }
    validate_retire_enterprise(&fixture.state, enterprises[2])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    for enterprise in enterprises {
        assert_unknown_enterprise(&fixture, enterprise);
    }

    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    resolve_with_zero_variance(&mut fixture, operation);
    let record = fixture.state.operations().get_operation(operation).unwrap();
    let resolution = record.resolution().unwrap();
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    assert_eq!(resolution.discovered_information().len(), 4);
    let discovered = resolution
        .discovered_information()
        .iter()
        .map(|id| fixture.state.intelligence().get_information(*id).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        discovered
            .iter()
            .map(|information| information.subject())
            .collect::<Vec<_>>(),
        vec![
            EntityRef::Organization(rival),
            EntityRef::Enterprise(enterprises[1]),
            EntityRef::Enterprise(enterprises[3]),
            EntityRef::Enterprise(enterprises[4])
        ],
    );
    assert_eq!(
        discovered[0].signal(),
        Some(&InformationSignal::PersonnelPresence {
            characters: BTreeSet::from([authority.manager]),
        })
    );
    for (information, location) in discovered[1..].iter().zip([names[1], names[3], names[4]]) {
        assert_eq!(information.topic(), InformationTopic::Personnel);
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
            Some(EntityRef::Operation(operation))
        );
        assert_eq!(information.reliability(), Reliability::GenerallyReliable);
        assert_eq!(information.specificity(), Specificity::Specific);
        assert_eq!(information.signal(), None);
        assert_eq!(information.observed_at(), resolution.resolved_at());
        assert_eq!(information.recorded_at(), resolution.resolved_at());
        assert!(information.derived_from().is_empty());
        // Exact public-face summary excludes accounts, profits, economic cycles, and cases.
        assert_eq!(
            information.summary(),
            format!(
                "Observed protection activity at {location} appears active under Rival Manager for Visible Rival."
            )
        );
        assert!(is_valid_persisted_surveillance_information(
            record,
            information
        ));
        assert_eq!(
            fixture
                .state
                .operations()
                .operation_for_discovered_information(information.id())
                .unwrap()
                .id(),
            operation
        );
    }
    let report = fixture
        .state
        .reports()
        .get_report(resolution.after_action_report())
        .unwrap();
    assert!(
        report.entries()[0]
            .entities
            .contains(&EntityRef::Organization(rival))
    );
    assert!(report.entries()[0].summary.contains(
        "Surveillance produced 4 usable target observations: personnel around Visible Rival; protection activity at Zulu Ward; protection activity at Yarrow Ward; protection activity at Xenia Ward."
    ));
    // Existing after-action reports carry findings and target links, not source citations.
    assert_eq!(
        report.entries()[0].summary,
        fixture
            .state
            .intelligence()
            .get_information(resolution.after_action_information())
            .unwrap()
            .summary()
    );
    for enterprise in [enterprises[0], enterprises[2], enterprises[5]] {
        assert_unknown_enterprise(&fixture, enterprise);
    }

    let followup = authorize_surveillance(&mut fixture, EntityRef::Enterprise(enterprises[1]));
    let envelope = build_save(&fixture.registry, &fixture.state).unwrap();
    let decoded: SaveEnvelope =
        bincode::deserialize(&bincode::serialize(&envelope).unwrap()).unwrap();
    let mut restored = restore_save(&fixture.registry, decoded).unwrap();
    assert_eq!(
        bincode::serialize(&restored).unwrap(),
        bincode::serialize(&fixture.state).unwrap()
    );
    let duration = fixture
        .registry
        .get_operation(OperationKind::Surveillance)
        .execution()
        .duration();
    for _ in 0..=duration.as_minutes() {
        run_tick(&fixture.registry, &mut fixture.state);
        run_tick(&fixture.registry, &mut restored);
    }
    assert_eq!(
        bincode::serialize(&build_save(&fixture.registry, &fixture.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&fixture.registry, &restored).unwrap()).unwrap(),
    );
    let followup_resolution = restored
        .operations()
        .get_operation(followup)
        .unwrap()
        .resolution()
        .unwrap();
    assert_eq!(
        followup_resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    let information = restored
        .intelligence()
        .get_information(
            *followup_resolution
                .discovered_information()
                .iter()
                .next()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(information.subject(), EntityRef::Enterprise(enterprises[1]));
    assert_eq!(followup_resolution.discovered_information().len(), 2);
    let police = followup_resolution
        .discovered_information()
        .iter()
        .map(|id| restored.intelligence().get_information(*id).unwrap())
        .find(|information| information.topic() == InformationTopic::PoliceActivity)
        .unwrap();
    assert_eq!(police.signal(), None);
    assert!(
        police
            .summary()
            .contains("No stable daily patrol deployment pattern was confirmed around Zulu Ward")
    );
    assert_eq!(
        information.summary(),
        "Observed protection activity at Zulu Ward appears active under Rival Manager for Visible Rival."
    );
    // Later inactivity cannot retroactively erase the frozen discovery/provenance.
    validate_suspend_enterprise(&fixture.state, enterprises[1])
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    validate_state(&fixture.state).unwrap();
}

#[test]
fn direct_enterprise_surveillance_adds_neighborhood_patrol_intelligence() {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    let enterprise = make_test_enterprise(&mut fixture, rival, authority, "Watched Ward");
    let neighborhood = crate::enterprises::enterprise_execution::resolve_location_neighborhood(
        &fixture.state,
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .unwrap()
            .location(),
    )
    .unwrap();
    validate_set_jurisdiction(
        &fixture.state,
        JurisdictionDraft {
            organization: fixture.police,
            neighborhoods: BTreeSet::from([fixture.neighborhood, neighborhood]),
            case_intake_priority: rating(80),
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    validate_establish_patrol_deployment(
        &fixture.state,
        PatrolDeploymentDraft {
            organization: fixture.police,
            neighborhood,
            windows: vec![
                PatrolWindow::try_new(DayMinute::try_new(600).unwrap(), 120, rating(80)).unwrap(),
            ],
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let discovery = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    resolve_with_zero_variance(&mut fixture, discovery);
    let operation = authorize_surveillance(&mut fixture, EntityRef::Enterprise(enterprise));
    resolve_with_zero_variance(&mut fixture, operation);
    let record = fixture.state.operations().get_operation(operation).unwrap();
    let resolution = record.resolution().unwrap();
    assert_eq!(
        resolution.objective_outcome(),
        OperationObjectiveOutcome::Achieved
    );
    let observations = resolution
        .discovered_information()
        .iter()
        .map(|id| fixture.state.intelligence().get_information(*id).unwrap())
        .collect::<Vec<_>>();
    let police = observations
        .iter()
        .find(|information| information.topic() == InformationTopic::PoliceActivity)
        .expect("direct enterprise surveillance must add neighborhood police activity");
    assert_eq!(observations.len(), 2);
    assert_eq!(police.subject(), EntityRef::Neighborhood(neighborhood));
    assert_eq!(
        police.signal(),
        Some(&InformationSignal::PatrolPattern {
            intervals: BTreeSet::from([PatrolIntervalSignal::try_new(600, 720).unwrap()]),
        })
    );
    assert!(police.summary().contains("roughly 10:00-12:00"));
    assert!(is_valid_persisted_surveillance_information(record, police));
    assert!(
        fixture
            .state
            .intelligence()
            .get_information(resolution.after_action_information())
            .unwrap()
            .summary()
            .contains("police activity around Watched Ward")
    );
    let envelope = build_save(&fixture.registry, &fixture.state).unwrap();
    let decoded = bincode::deserialize(&bincode::serialize(&envelope).unwrap()).unwrap();
    let restored = restore_save(&fixture.registry, decoded).unwrap();
    assert_eq!(
        bincode::serialize(&restored).unwrap(),
        bincode::serialize(&fixture.state).unwrap()
    );
}

#[test]
fn colocated_rackets_remain_distinguishable_in_surveillance_and_after_action() {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    let neighborhood = fixture.neighborhood;
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Shared Club".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Organization(rival),
        },
    )
    .unwrap();
    let kinds = [
        EnterpriseKind::Protection,
        EnterpriseKind::Bookmaking,
        EnterpriseKind::LoanSharking,
    ];
    let enterprises = kinds.map(|kind| {
        make_test_enterprise_kind_at(
            &mut fixture,
            rival,
            authority,
            EnterpriseLocation::Business(business),
            kind,
        )
    });
    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    resolve_with_zero_variance(&mut fixture, operation);
    let result = fixture
        .state
        .operations()
        .get_operation(operation)
        .unwrap()
        .resolution()
        .unwrap();
    let observations: Vec<_> = result
        .discovered_information()
        .iter()
        .map(|id| fixture.state.intelligence().get_information(*id).unwrap())
        .filter(|item| matches!(item.subject(), EntityRef::Enterprise(_)))
        .collect();
    assert_eq!(observations.len(), 3);
    let after_action = fixture
        .state
        .intelligence()
        .get_information(result.after_action_information())
        .unwrap();
    for ((observation, enterprise), label) in
        observations
            .iter()
            .zip(enterprises)
            .zip(["protection", "bookmaking", "loan-sharking"])
    {
        assert_eq!(observation.subject(), EntityRef::Enterprise(enterprise));
        assert_eq!(
            observation.summary(),
            format!(
                "Observed {label} activity at Shared Club appears active under Rival Manager for Visible Rival."
            )
        );
        assert!(
            after_action
                .summary()
                .contains(&format!("{label} activity at Shared Club"))
        );
    }
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for observation in observations {
        assert_eq!(
            restored
                .intelligence()
                .get_information(observation.id())
                .unwrap()
                .summary(),
            observation.summary()
        );
    }
}

#[test]
fn hosted_enterprise_watch_bounds_patrol_knowledge_by_outcome() {
    for (skill, expected_outcome, count) in [
        (100, OperationObjectiveOutcome::Achieved, 2),
        (35, OperationObjectiveOutcome::Partial, 2),
        (0, OperationObjectiveOutcome::Failed, 0),
    ] {
        let mut fixture = fixture(skill, true);
        let (rival, authority) = make_test_rival(&mut fixture);
        let enterprise = make_test_hosted_enterprise(&mut fixture, rival, authority);
        validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(fixture.crew),
                source_kind: InformationSourceKind::Surveillance,
                topic: InformationTopic::Personnel,
                source_entity: Some(EntityRef::Character(fixture.observer)),
                subject: EntityRef::Enterprise(enterprise),
                observed_at: fixture.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "Known enterprise at the club.".to_owned(),
            },
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
        let operation = authorize_surveillance(&mut fixture, EntityRef::Enterprise(enterprise));
        resolve_with_zero_variance(&mut fixture, operation);
        let record = fixture.state.operations().get_operation(operation).unwrap();
        let resolution = record.resolution().unwrap();
        assert_eq!(resolution.objective_outcome(), expected_outcome);
        assert_eq!(resolution.discovered_information().len(), count);
        if expected_outcome != OperationObjectiveOutcome::Failed {
            let police = resolution
                .discovered_information()
                .iter()
                .map(|id| fixture.state.intelligence().get_information(*id).unwrap())
                .find(|information| information.topic() == InformationTopic::PoliceActivity)
                .unwrap();
            assert_eq!(
                police.subject(),
                EntityRef::Neighborhood(fixture.neighborhood)
            );
            if expected_outcome == OperationObjectiveOutcome::Achieved {
                assert_eq!(
                    police.signal(),
                    Some(&InformationSignal::PatrolPattern {
                        intervals: BTreeSet::from([
                            PatrolIntervalSignal::try_new(120, 240).unwrap(),
                            PatrolIntervalSignal::try_new(1320, 1440).unwrap(),
                        ]),
                    })
                );
            } else {
                assert_eq!(police.signal(), None);
                assert_eq!(police.reliability(), Reliability::Mixed);
                assert_eq!(police.specificity(), Specificity::General);
                assert!(
                    police
                        .summary()
                        .contains("a dependable daily patrol pattern was not established")
                );
                assert!(!police.summary().contains("roughly"));
            }
            assert!(is_valid_persisted_surveillance_information(record, police));
        } else {
            assert!(resolution.surveillance_signatures().is_empty());
        }
        restore_save(
            &fixture.registry,
            build_save(&fixture.registry, &fixture.state).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn direct_enterprise_watch_rejects_changed_patrol_snapshot() {
    let mut fixture = fixture(100, true);
    let (rival, authority) = make_test_rival(&mut fixture);
    let enterprise = make_test_hosted_enterprise(&mut fixture, rival, authority);
    let discovery = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    resolve_with_zero_variance(&mut fixture, discovery);
    let discovery_record = fixture.state.operations().get_operation(discovery).unwrap();
    assert!(
        discovery_record
            .resolution()
            .unwrap()
            .discovered_information()
            .iter()
            .all(|id| fixture
                .state
                .intelligence()
                .get_information(*id)
                .unwrap()
                .topic()
                == InformationTopic::Personnel)
    );
    let operation = authorize_surveillance(&mut fixture, EntityRef::Enterprise(enterprise));
    run_tick(&fixture.registry, &mut fixture.state);
    fixture.state.advance_clock(SimDuration::from_minutes(120));
    let record = fixture.state.operations().get_operation(operation).unwrap();
    let snapshot = decide_surveillance_intelligence(
        &fixture.registry,
        &fixture.state,
        record,
        OperationObjectiveOutcome::Achieved,
    )
    .unwrap()
    .unwrap();
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .unwrap();
    let validated =
        validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan.clone())
            .unwrap();
    let deployment = fixture
        .state
        .legal()
        .active_patrol_deployments_for_neighborhood(fixture.neighborhood)
        .next()
        .unwrap()
        .id();
    // Change only a future window, not presence during this watch. The frozen direct snapshot
    // must still reject the now-different recurring pattern that it was going to publish.
    crate::legal::patrol_system::validate_revise_patrol_deployment(
        &fixture.state,
        deployment,
        vec![
            PatrolWindow::try_new(DayMinute::try_new(120).unwrap(), 120, rating(80)).unwrap(),
            PatrolWindow::try_new(DayMinute::try_new(1200).unwrap(), 120, rating(60)).unwrap(),
        ],
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_surveillance_plan_snapshot(&fixture.state, &snapshot),
        Err(SurveillanceError::StaleTarget(EntityRef::Enterprise(
            enterprise
        )))
    );
    // The execution-level historical police guard may reject before surveillance's own guard.
    let error = validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
        .err()
        .unwrap();
    assert!(
        matches!(error,
            OperationResolutionError::StalePoliceDeploymentContext { operation: id } if id == operation
        ) || error
            == OperationResolutionError::Surveillance(SurveillanceError::StaleTarget(
                EntityRef::Enterprise(enterprise)
            ))
    );
    assert_eq!(validated.commit(&mut fixture.state).unwrap_err(), error);
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
}

#[test]
fn partial_and_failed_organization_surveillance_do_not_discover_enterprises() {
    for (skill, expected_outcome, expected_count) in [
        (35, OperationObjectiveOutcome::Partial, 1),
        (0, OperationObjectiveOutcome::Failed, 0),
    ] {
        let mut fixture = fixture(skill, false);
        let (rival, authority) = make_test_rival(&mut fixture);
        let enterprise = make_test_enterprise(&mut fixture, rival, authority, "Unlearned Ward");
        let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
        resolve_with_zero_variance(&mut fixture, operation);
        let resolution = fixture
            .state
            .operations()
            .get_operation(operation)
            .unwrap()
            .resolution()
            .unwrap();
        assert_eq!(resolution.objective_outcome(), expected_outcome);
        assert_eq!(resolution.discovered_information().len(), expected_count);
        for id in resolution.discovered_information() {
            let information = fixture.state.intelligence().get_information(*id).unwrap();
            assert_eq!(information.subject(), EntityRef::Organization(rival));
            assert_eq!(information.topic(), InformationTopic::Personnel);
            assert_eq!(
                information.signal(),
                Some(&InformationSignal::PersonnelPresence {
                    characters: BTreeSet::from([authority.manager]),
                })
            );
        }
        assert_unknown_enterprise(&fixture, enterprise);
        assert!(
            !fixture
                .state
                .intelligence()
                .get_information(resolution.after_action_information())
                .unwrap()
                .summary()
                .contains("Unlearned Ward")
        );
        validate_state(&fixture.state).unwrap();
    }
}

#[test]
fn organization_surveillance_stales_when_enterprise_selection_changes() {
    for resume in [false, true] {
        let mut fixture = fixture(100, false);
        let (rival, authority) = make_test_rival(&mut fixture);
        let first = make_test_enterprise(&mut fixture, rival, authority, "First Ward");
        for name in ["Second Ward", "Third Ward", "Fourth Ward"] {
            make_test_enterprise(&mut fixture, rival, authority, name);
        }
        if resume {
            validate_suspend_enterprise(&fixture.state, first)
                .unwrap()
                .commit(&mut fixture.state)
                .unwrap();
        }
        let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
        run_tick(&fixture.registry, &mut fixture.state);
        fixture.state.advance_clock(SimDuration::from_minutes(120));
        let plan = decide_operation_resolution(
            &fixture.registry,
            &fixture.state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .unwrap();
        // Also prove the commit-time gate, not just the initial validation gate.
        let validated =
            validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan.clone())
                .unwrap();
        if resume {
            validate_resume_enterprise(&fixture.registry, &fixture.state, first)
                .unwrap()
                .commit(&mut fixture.state)
                .unwrap();
        } else {
            validate_suspend_enterprise(&fixture.state, first)
                .unwrap()
                .commit(&mut fixture.state)
                .unwrap();
        }
        let before = bincode::serialize(&fixture.state).unwrap();
        let expected = OperationResolutionError::Surveillance(SurveillanceError::StaleTarget(
            EntityRef::Organization(rival),
        ));
        assert_eq!(
            validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan).err(),
            Some(expected.clone())
        );
        assert_eq!(validated.commit(&mut fixture.state).err(), Some(expected));
        assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
        validate_state(&fixture.state).unwrap();
    }
}

#[test]
fn noncriminal_organization_surveillance_never_discovers_rival_portfolios() {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    let enterprise = make_test_enterprise(&mut fixture, rival, authority, "Hidden Rival Ward");
    // Noncriminal organizations cannot canonically establish enterprises. A real rival
    // portfolio alongside every other organization kind proves there is no global scan leak.
    for kind in [
        OrganizationKind::LawEnforcement,
        OrganizationKind::LegalAuthority,
        OrganizationKind::LegalServices,
        OrganizationKind::Prosecutor,
        OrganizationKind::Political,
        OrganizationKind::Press,
        OrganizationKind::Labor,
        OrganizationKind::Civic,
        OrganizationKind::Commercial,
    ] {
        let target = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: format!("Observed {kind:?}"),
                kind,
            },
        )
        .unwrap();
        let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(target));
        resolve_with_zero_variance(&mut fixture, operation);
        let resolution = fixture
            .state
            .operations()
            .get_operation(operation)
            .unwrap()
            .resolution()
            .unwrap();
        assert_eq!(
            resolution.objective_outcome(),
            OperationObjectiveOutcome::Achieved
        );
        assert_eq!(resolution.discovered_information().len(), 1);
        let information = fixture
            .state
            .intelligence()
            .get_information(*resolution.discovered_information().iter().next().unwrap())
            .unwrap();
        assert_eq!(information.topic(), InformationTopic::Personnel);
        assert_eq!(information.subject(), EntityRef::Organization(target));
        assert_unknown_enterprise(&fixture, enterprise);
    }
    validate_state(&fixture.state).unwrap();
}

#[test]
fn organization_surveillance_stales_when_first_enterprise_is_established_after_planning() {
    let mut fixture = fixture(100, false);
    let (rival, authority) = make_test_rival(&mut fixture);
    let operation = authorize_surveillance(&mut fixture, EntityRef::Organization(rival));
    run_tick(&fixture.registry, &mut fixture.state);
    fixture.state.advance_clock(SimDuration::from_minutes(120));
    let plan = decide_operation_resolution(
        &fixture.registry,
        &fixture.state,
        operation,
        OperationResolutionRandomness::new(0, 0),
    )
    .unwrap();
    make_test_enterprise(&mut fixture, rival, authority, "New Ward");
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan).err(),
        Some(OperationResolutionError::Surveillance(
            SurveillanceError::StaleTarget(EntityRef::Organization(rival))
        ))
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    validate_state(&fixture.state).unwrap();
}
