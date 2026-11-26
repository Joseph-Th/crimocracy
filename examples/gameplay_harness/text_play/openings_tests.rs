//! Authorial opening variants remain real campaign state, not scripted decisions.

use super::*;

#[test]
fn text_play_converts_a_single_storefront_lead_with_contextual_entities() {
    use crimocracy::intelligence::intelligence_system::validate_record_information;
    use crimocracy::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
    };
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let original = session.known_opportunities().next().unwrap();
    let id = original.id();
    let target = *original.context().targets().iter().next().unwrap();
    let mut sources = original.source_information().clone();
    session.execute(&format!("dismiss {}", id.raw())).unwrap();
    let leader = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| person.supervisor().is_none())
        .unwrap()
        .id();
    let contextual = EntityRef::Character(leader);
    let source = validate_record_information(
        &session.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(session.player),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::TargetSecurity,
            source_entity: None,
            subject: contextual,
            observed_at: session.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "A known participant supplied the contextual observation.".to_owned(),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    sources.insert(source);
    let lead = validate_discover_operation_opportunity(
        &registry,
        &session.state,
        OperationOpportunityDraft {
            organization: session.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([target, contextual]),
            source_information: sources,
            summary: "One storefront with additional contextual information.".to_owned(),
            valid_until: Some(SimTime::from_minutes(120)),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let before = build_save(&registry, &session.state).unwrap();
    let command = format!("burgle {} abort", lead.raw());
    let output = session.execute(&command).unwrap().0;
    assert!(
        output.contains(&format!("lead {} converted", lead.raw())),
        "{output}"
    );
    let record = session.state.opportunities().get_opportunity(lead).unwrap();
    let operation = record.resolution().unwrap().operation().unwrap();
    assert_eq!(
        session
            .state
            .operations()
            .get_operation(operation)
            .unwrap()
            .objective(),
        &OperationObjective::AcquireProperty { target }
    );
    let after = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    session.state = restore_save(&registry, before).unwrap();
    assert_eq!(session.execute(&command).unwrap().0, output);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        after
    );
}

#[test]
fn text_play_alternate_openings_are_saved_campaigns_with_distinct_crews() {
    let registry = crimocracy::build_registry();
    let seeds = EvaluationSeeds::new(12, 12);
    let mut strength = Vec::new();
    for opening in [
        ScenarioProfile::NightTrap,
        ScenarioProfile::LatePatrol,
        ScenarioProfile::FleetingWindow,
        ScenarioProfile::VeteranCrew,
        ScenarioProfile::ThinCrew,
        ScenarioProfile::GreenScout,
    ] {
        let mut session = Session::new_with_opening(&registry, seeds, opening).unwrap();
        let leads: Vec<_> = session.known_opportunities().collect();
        assert_eq!(leads.len(), 2);
        let burglary_duration = u64::from(
            registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration()
                .as_minutes(),
        );
        let expected_deadline = if opening == ScenarioProfile::FleetingWindow {
            burglary_duration + 25
        } else {
            (DAY_MINUTES / 2).max(burglary_duration + 60)
        };
        assert_eq!(
            leads[0].valid_until().map(|at| at.as_minutes()),
            Some(expected_deadline),
        );
        assert_eq!(leads[1].valid_until(), None);
        let saved = build_save(&registry, &session.state).unwrap();
        let restored = restore_save(&registry, saved.clone()).unwrap();
        assert_eq!(
            bincode::serialize(&saved).unwrap(),
            bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
        );
        let member = session
            .state
            .world()
            .characters_in_organization(session.player)
            .find(|member| member.id().raw() == 4)
            .unwrap();
        strength.push(
            member
                .capability(crimocracy::world::CapabilityKind::Burglary)
                .unwrap()
                .value(),
        );
        session.execute("advance 1m").unwrap();
        assert_eq!(session.state.now().as_minutes(), 1);
    }
    assert_ne!(
        strength[3], strength[4],
        "veteran and thin-crew openings must create different member capabilities"
    );
}

#[test]
fn text_play_opening_options_are_validated_before_building_campaigns() {
    let args = vec![
        "--opening".to_owned(),
        "green-scout".to_owned(),
        "--seed".to_owned(),
        "12".to_owned(),
    ];
    assert_eq!(
        parse_options(&args).unwrap().opening,
        Some(ScenarioProfile::GreenScout)
    );
    for args in [
        vec!["--opening", "unknown"],
        vec!["--opening", "night-trap", "--opening", "thin-crew"],
        vec!["--seed", "12", "--seed", "12"],
        vec!["--load", "session", "--opening", "thin-crew"],
        vec!["--load", "session", "--seed", "12"],
    ] {
        assert!(parse_options(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
    }
}
