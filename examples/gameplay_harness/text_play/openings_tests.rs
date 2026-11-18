//! Authorial opening variants remain real campaign state, not scripted decisions.

use super::*;

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
        assert_eq!(session.known_opportunities().len(), 2);
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
