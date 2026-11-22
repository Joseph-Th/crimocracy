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
    assert_eq!(
        session.player_prospects().unwrap().len(),
        6,
        "two established opening prospects plus one real freelance contact in each ward"
    );
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
fn generated_rival_turf_agreement_requires_sighting_and_blocks_both_types_of_escalation() {
    use crimocracy::world::territory_influence::active_standstill;

    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let district = session.state.world().neighborhoods().nth(1).unwrap().id();
    let (rival, venue) = session
        .state
        .world()
        .public_business_directory()
        .filter(|(_, _, _, ward)| *ward == district)
        .find_map(|(business, _, _, _)| {
            session
                .state
                .enterprises()
                .enterprises_at(EnterpriseLocation::Business(business))
                .next()
                .map(|enterprise| (enterprise.organization(), business))
        })
        .expect("generated seed has a foreign racket in district two");
    let negotiation = format!("negotiate district-{} org-{}", district.raw(), rival.raw());
    let before = snapshot(&registry, &session);
    assert!(session.execute(&negotiation).is_err());
    assert_eq!(before, snapshot(&registry, &session));

    session
        .execute(&format!("plan surveillance district-{}", district.raw()))
        .unwrap();
    session.execute("advance 3h").unwrap();
    assert!(session.known_organization_ids().contains(&rival));
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .nth(2)
        .expect("opening field manager exists")
        .id();
    session
        .execute(&format!(
            "delegate {} district-{}",
            manager.raw(),
            district.raw()
        ))
        .expect("racket manager receives district mandate");
    session
        .execute(&format!("autonomy {} delegated", manager.raw()))
        .unwrap();
    let new_account = session.execute("open-account settlement").unwrap().0;
    let settlement = new_account
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .last()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let accounts = session.execute("accounts").unwrap().0;
    let cash = accounts
        .lines()
        .find(|line| line.contains("StreetCash"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let racket = format!(
        "start-racket protection district-{} account-{cash} account-{settlement} manager=char-{}",
        district.raw(),
        manager.raw(),
    );
    session
        .execute(&racket)
        .expect("player claims a rival district");

    // Establish a legitimate hostile plan before signing. Its later
    // authorization must not circumvent a negotiated peace when the due
    // start arrives on a subsequent canonical minute.
    session
        .execute(&format!("plan surveillance business-{}", venue.raw()))
        .expect("the business can be observed before peace");
    session.execute("advance 3h").unwrap();
    let sabotage = session
        .execute(&format!("plan sabotage business-{} delay=1d", venue.raw()))
        .expect("a planned future disruption should be actionable before peace")
        .0;
    assert!(sabotage.contains("Authorized Sabotage"), "{sabotage}");

    let answer = session
        .execute(&negotiation)
        .expect("balanced rival accepts")
        .0;
    assert!(
        answer.contains("Agreed a 30-day district standstill"),
        "{answer}"
    );
    assert!(
        session
            .execute("turf")
            .unwrap()
            .0
            .contains("Standstill with")
    );
    assert!(active_standstill(&session.state, rival, district).is_some());
    assert!(session.execute(&racket).unwrap_err().contains("standstill"));

    session
        .execute("advance 2d")
        .expect("due hostile plan is evaluated under the treaty");
    assert!(
        session.execute("operations").unwrap().0.contains("Aborted"),
        "the formerly authorized attack must be cancelled by its canonical prestart phase"
    );

    session
        .execute(&format!("plan surveillance business-{}", venue.raw()))
        .expect("reconnaissance remains allowed under standstill");
    session.execute("advance 3h").unwrap();
    assert!(
        session
            .execute(&format!("preview sabotage business-{}", venue.raw()))
            .unwrap_err()
            .contains("standstill"),
        "production sabotage authorization must honor the treaty"
    );

    let before_restore = snapshot(&registry, &session);
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(before_restore, snapshot(&registry, &session));
    assert!(
        session
            .execute("turf")
            .unwrap()
            .0
            .contains("Standstill with")
    );
}

#[test]
fn generated_rival_clash_then_postwar_negotiation_uses_actual_player_orders() {
    use crimocracy::world::territory_conflict::{district_of_racket, suppression_ends_at};
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let district = session.state.world().neighborhoods().nth(1).unwrap().id();
    let target = session
        .state
        .world()
        .public_business_directory()
        .filter(|(_, _, _, ward)| *ward == district)
        .flat_map(|(venue, _, _, _)| {
            session
                .state
                .enterprises()
                .enterprises_at(EnterpriseLocation::Business(venue))
        })
        .find(|racket| {
            racket.organization() != session.player
                && racket.status() == crimocracy::enterprises::EnterpriseStatus::Active
        })
        .unwrap()
        .id();
    let rival = session
        .state
        .enterprises()
        .get_enterprise(target)
        .unwrap()
        .organization();
    let strike = format!("contest enterprise-{}", target.raw());
    let original = snapshot(&registry, &session);
    assert!(
        session.execute(&strike).is_err(),
        "unscouted targets cannot be attacked"
    );
    assert_eq!(original, snapshot(&registry, &session));

    session
        .execute(&format!("plan surveillance district-{}", district.raw()))
        .unwrap();
    session.execute("advance 3h").unwrap();
    assert!(session.known_enterprise_ids().contains(&target));
    // The starting enforcer knows violence but not command. The existing
    // experienced lieutenant can handle the additional field associates when
    // given real authority over the neighboring district. Use the public
    // mandate-revision command rather than bypassing the supervision model.
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .nth(1)
        .expect("generated lieutenant exists")
        .id();
    let mandate = session
        .state
        .delegation()
        .active_for_manager(manager)
        .expect("the lieutenant already commands the home racket")
        .id();
    session
        .execute(&format!(
            "mandate {} add district-{}",
            mandate.raw(),
            district.raw()
        ))
        .unwrap();
    let account_output = session.execute("open-account settlement").unwrap().0;
    let settlement_id = account_output
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .last()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let street = session
        .state
        .finance()
        .accounts_for(crimocracy::finance::FinancialOwner::Organization(
            session.player,
        ))
        .find(|account| {
            account.kind() == crimocracy::finance::AccountKind::StreetCash
                && account.available_payment_balance() > crimocracy::finance::Money::ZERO
        })
        .unwrap()
        .id();
    session
        .execute(&format!(
            "start-racket protection district-{} {} account-{} manager=char-{}",
            district.raw(),
            street,
            settlement_id,
            manager.raw(),
        ))
        .expect("player can establish rival district racket");
    let own_racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|racket| {
            racket.status() == crimocracy::enterprises::EnterpriseStatus::Active
                && district_of_racket(&session.state, racket.location()) == Some(district)
        })
        .unwrap()
        .id();
    session
        .execute(&format!("staff-racket {} 8", own_racket.raw()))
        .expect("pay for a superior rank-and-file complement");
    let planned = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("preview-contest enterprise-{}", target.raw()))
            .unwrap()
            .0
            .contains("their current force is unverified")
    );
    assert_eq!(
        planned,
        snapshot(&registry, &session),
        "preview must not change state"
    );
    let outcome = session.execute(&strike).unwrap().0;
    assert!(outcome.contains("displaced a rival operation"), "{outcome}");
    assert!(suppression_ends_at(&session.state, target).is_some());
    assert!(
        session
            .execute("reports")
            .unwrap()
            .0
            .contains("Turf conflict")
    );
    let peace = session
        .execute(&format!(
            "negotiate district-{} org-{}",
            district.raw(),
            rival.raw(),
        ))
        .expect("recently defeated gang can still settle")
        .0;
    assert!(peace.contains("Agreed a 30-day"), "{peace}");
    assert!(
        session
            .execute("turf")
            .unwrap()
            .0
            .contains("Standstill with")
    );
    let before = snapshot(&registry, &session);
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(before, snapshot(&registry, &session));
    assert!(
        session
            .execute("turf")
            .unwrap()
            .0
            .contains("Standstill with")
    );
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
