use super::*;

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
