//! Play a generated city through the same real terminal, owner validation and
//! campaign persistence paths as the authored openings.
use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn generated_start_flags_are_strict_and_keep_authored_openings_available() {
    let args = vec!["--generate", "--city-size", "large", "--seed", "0x2a"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let options = parse_options(&args).unwrap();
    assert!(options.generated);
    assert_eq!(options.city_size, Some(CityScale::Large));
    assert_eq!(options.seed, Some(42));
    for bad in [
        vec!["--city-size", "small"],
        vec!["--generate", "--city-size", "huge"],
        vec!["--generate", "--opening", "night-trap"],
        vec!["--generate", "--policy-seed", "12"],
        vec!["--generate", "--generate"],
        vec![
            "--generate",
            "--city-size",
            "small",
            "--city-size",
            "medium",
        ],
        vec!["--load", "slot", "--generate"],
        vec!["--load", "slot", "--city-size", "small"],
    ] {
        assert!(parse_options(&bad.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
    }
    assert!(
        !parse_options(&["--opening".to_owned(), "night-trap".to_owned()])
            .unwrap()
            .generated
    );
}

#[test]
fn generated_city_runs_real_operations_and_restores_its_identical_world_and_labels() {
    let registry = crimocracy::build_registry();
    let seeds = EvaluationSeeds::new(19, 19);
    let mut session = Session::new_generated(&registry, seeds, CityScale::Small).unwrap();
    let home_name = session
        .state
        .world()
        .neighborhoods()
        .next()
        .unwrap()
        .name()
        .to_owned();
    assert_eq!(session.district, home_name);
    assert_eq!(session.district_variant, "PROCEDURAL");
    assert_eq!(session.state.world().neighborhoods().count(), 4);
    assert_eq!(session.known_opportunities().len(), 2);
    assert_eq!(session.player_prospects().unwrap().len(), 2);
    assert!(
        session
            .execute("connections")
            .unwrap()
            .0
            .contains("Municipal Defense Office")
    );
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Gambling")
    );
    let beginning = snapshot(&registry, &session);
    assert!(session.execute("plan surveillance district-9999").is_err());
    assert_eq!(beginning, snapshot(&registry, &session));
    assert!(
        session
            .execute("plan surveillance district-1")
            .unwrap()
            .0
            .contains("Authorized Surveillance")
    );
    let progress = session.execute("advance 3h").unwrap().0;
    assert!(
        progress.contains("operation reached a terminal outcome"),
        "{progress}"
    );
    assert!(
        session
            .execute("intelligence")
            .unwrap()
            .0
            .contains(&home_name),
        "surveillance must produce a home district observation"
    );
    let saved = snapshot(&registry, &session);
    let slot = "procedural_city_test19";
    session.save(slot).unwrap();
    let mut resumed = Session::load(&registry, slot).unwrap();
    assert_eq!(saved, snapshot(&registry, &resumed));
    assert_eq!(resumed.district, home_name);
    assert_eq!(resumed.district_variant, "PROCEDURAL");
    assert_eq!(session.brief(), resumed.brief());
    assert!(
        resumed
            .execute("known-targets")
            .unwrap()
            .0
            .contains("operation-1")
    );
    let after = resumed.execute("advance 2h").unwrap().0;
    assert!(
        after.contains("Advanced ") || after.contains("Paused:"),
        "{after}"
    );
    std::fs::remove_file(save_path(slot).unwrap()).unwrap();
}

#[test]
fn generated_city_can_be_started_via_the_shipped_command_line_client() {
    let registry = crimocracy::build_registry();
    for (size, expected_districts) in [("small", 4), ("medium", 8), ("large", 16)] {
        let args = ["--generate", "--seed", "12", "--city-size", size, "--brief"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut commands = std::io::Cursor::new("districts\nbrief\nquit\n".as_bytes());
        let mut output = Vec::new();
        run_with_io(&registry, &args, &mut commands, &mut output).unwrap();
        let displayed = String::from_utf8(output).unwrap();
        assert!(displayed.contains(&format!("New procedurally generated {size} city")));
        assert!(displayed.contains("PROCEDURAL"));
        assert_eq!(displayed.matches("district-",).count(), expected_districts);
    }
}
