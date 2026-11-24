use super::*;
use crimocracy::world::IndependentVocation;

#[test]
fn generated_city_has_hundreds_of_real_people_and_hires_a_freelancer_for_a_fixed_term() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(562365, 562365),
        CityScale::Small,
    )
    .unwrap();
    let census = session.execute("population").unwrap().0;
    assert!(session.state.world().population_size() >= 200, "{census}");
    assert!(
        census.contains("Persistently simulated named people:"),
        "{census}"
    );
    let listing = session.execute("freelancers").unwrap().0;
    assert!(listing.contains("contract char-"), "{listing}");
    let boss = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| person.supervisor().is_none())
        .unwrap()
        .id();
    let freelancer = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find_map(|member| {
            session
                .state
                .social()
                .relationships_to(member.id())
                .find(|relationship| {
                    session
                        .state
                        .world()
                        .get_character(relationship.from())
                        .is_some_and(|person| {
                            person.organization().is_none()
                                && person.life().vocation()
                                    == IndependentVocation::FreelanceCriminal
                        })
                })
                .map(|relationship| (relationship.from(), member.id()))
        })
        .unwrap();
    let (worker, recruiter) = freelancer;
    assert_eq!(recruiter, boss);
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute(&format!(
                "contract char-{} char-{} 0",
                worker.raw(),
                recruiter.raw()
            ))
            .is_err()
    );
    assert!(session.execute("contract char-999999 char-8 3").is_err());
    assert!(session.execute("freelancers extra").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let outcome = session
        .execute(&format!(
            "contract char-{} char-{} 2",
            worker.raw(),
            recruiter.raw()
        ))
        .unwrap()
        .0;
    assert!(outcome.contains("Contracted"), "{outcome}");
    assert_eq!(
        session
            .state
            .world()
            .get_character(worker)
            .unwrap()
            .organization(),
        Some(session.player)
    );
    assert!(
        session
            .state
            .world()
            .active_freelance_contract(worker)
            .is_some()
    );
    // Establish a trusting freelance acquaintance with the hired worker.
    // The directory must not advertise a route through a temporary supervisor.
    let home = session
        .state
        .world()
        .get_character(worker)
        .unwrap()
        .life()
        .home()
        .unwrap();
    let other = crimocracy::world::world_system::insert_character(
        &mut session.state,
        crimocracy::world::CharacterDraft {
            name: "Independent acquaintance".into(),
            organization: None,
            supervisor: None,
            autonomy: crimocracy::world::AutonomyLevel::Guided,
            capabilities: std::collections::BTreeMap::new(),
            traits: std::collections::BTreeSet::new(),
            drives: std::collections::BTreeMap::new(),
        },
    )
    .unwrap();
    crimocracy::world::character_life::validate_set_civic_occupation(
        &session.state,
        other,
        home,
        IndependentVocation::FreelanceCriminal,
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let mut dimensions = crimocracy::social::RelationshipDimensions::zero();
    dimensions.trust = crimocracy::social::RelationshipLevel::try_new(80).unwrap();
    crimocracy::social::relationship_system::validate_set_relationship(
        &session.state,
        other,
        worker,
        dimensions,
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let listing = session.execute("freelancers").unwrap().0;
    let temporary_manager = format!("char-{}", worker.raw());
    assert!(
        !listing
            .split_whitespace()
            .any(|token| token.trim_end_matches(',') == temporary_manager),
        "{listing}"
    );
    assert!(
        !crimocracy::world::freelance_contracts::freelance_contact_available(
            &session.state,
            session.player,
            other,
            worker,
        )
    );
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute(&format!(
                "contract char-{} char-{} 1",
                other.raw(),
                worker.raw()
            ))
            .is_err()
    );
    assert_eq!(
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        before
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        restored
            .world()
            .active_freelance_contract(worker)
            .unwrap()
            .fee_cents(),
        session
            .state
            .world()
            .active_freelance_contract(worker)
            .unwrap()
            .fee_cents()
    );
    let advance = session.execute("advance 2d uninterrupted").unwrap().0;
    assert!(advance.contains("1 freelance contract end(s)"), "{advance}");
    assert!(
        session
            .state
            .world()
            .active_freelance_contract(worker)
            .is_none()
    );
    assert_eq!(
        session
            .state
            .world()
            .get_character(worker)
            .unwrap()
            .organization(),
        None
    );
    assert!(
        session
            .execute("freelancers")
            .unwrap()
            .0
            .contains(&format!("char-{}", worker.raw()))
    );
    restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
}

#[test]
fn public_civic_people_are_real_and_private_cases_remain_hidden_after_work_and_restore() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(562_365, 562_365),
        CityScale::Small,
    )
    .unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let full = session.execute("civics").unwrap().0;
    assert!(full.contains("PoliceOfficer:"), "{full}");
    assert!(full.contains("Politician:"), "{full}");
    assert!(full.contains("BusinessOwner:"), "{full}");
    let district = session.execute("civics district-1").unwrap().0;
    assert!(district.contains("proprietor"), "{district}");
    assert!(district.contains("PoliceOfficer"), "{district}");
    assert!(district.contains("Politician"), "{district}");
    let merchant = session
        .state
        .world()
        .public_civic_people()
        .find(|person| {
            session.state.world().civic_profession(person.id())
                == Some(crimocracy::world::CivicProfession::BusinessOwner)
        })
        .unwrap()
        .id();
    let person = session
        .execute(&format!("civic-person char-{}", merchant.raw()))
        .unwrap()
        .0;
    assert!(person.contains("Registered owner: business-"), "{person}");
    assert!(
        person.contains("Private condition and personal finances are not disclosed"),
        "{person}"
    );
    assert!(!person.contains("fatigue"), "{person}");
    let boss = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| person.supervisor().is_none())
        .unwrap()
        .id();
    let boss_result = session
        .execute(&format!("civic-person char-{}", boss.raw()))
        .unwrap()
        .0;
    assert!(
        boss_result.contains("Own member condition"),
        "{boss_result}"
    );
    assert!(
        boss_result.contains("civilian workdays not applicable"),
        "{boss_result}"
    );
    assert!(
        boss_result.contains("Recorded crew activity:"),
        "{boss_result}"
    );
    assert!(
        !boss_result.contains("No recorded professional duty yet"),
        "{boss_result}"
    );
    assert!(session.execute("civic-person char-999999").is_err());
    assert!(session.execute("civics district-999999").is_err());
    assert!(session.execute("civics one two").is_err());
    assert!(session.execute("civic-person").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        "civic dossiers and rejected input must be read-only"
    );

    session.execute("advance 2d uninterrupted").unwrap();
    let after_work = session
        .execute(&format!("civic-person char-{}", merchant.raw()))
        .unwrap()
        .0;
    assert!(
        after_work.contains("Last recorded duty: personally managed business-"),
        "{after_work}"
    );
    let saved = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let existing = session.execute("civics district-1").unwrap().0;
    session.state = saved;
    assert_eq!(session.execute("civics district-1").unwrap().0, existing);
}
