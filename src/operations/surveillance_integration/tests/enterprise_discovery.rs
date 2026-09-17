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
            kind: EnterpriseKind::Protection,
            organization: rival,
            authority,
            location: EnterpriseLocation::Neighborhood(neighborhood),
            supporting_businesses: BTreeSet::new(),
            cash_account: cash,
            settlement_account: settlement,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap()
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
            format!("Activity at {location} appears active under Rival Manager for Visible Rival.")
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
        "Surveillance produced 4 usable target observations: personnel around Visible Rival; activity at Zulu Ward; activity at Yarrow Ward; activity at Xenia Ward."
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
    assert_eq!(
        information.summary(),
        "Activity at Zulu Ward appears active under Rival Manager for Visible Rival."
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
