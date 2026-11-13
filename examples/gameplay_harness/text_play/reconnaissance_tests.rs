//! Player knowledge and completed operation checks for non-business surveillance.

use super::*;
use crimocracy::intelligence::intelligence_system::validate_record_information;
use crimocracy::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
};

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

fn precinct(session: &Session<'_>) -> OrganizationId {
    session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|record| record.kind() == ContactKind::Police)
        .expect("opening has a genuine police contact")
        .institution()
}

#[test]
fn text_play_reconnaissance_directory_guards_unknown_world_and_admin_ids() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let precinct = precinct(&session);
    let directory = session.execute("known-targets").unwrap().0;
    assert!(directory.contains(&format!("org-{} | Central Precinct", precinct.raw())));
    assert!(directory.contains("org-1 | Marrow Organization"));
    assert!(directory.contains("enterprise-1 | your Gambling"));
    assert!(
        !directory.contains("Rosetti"),
        "the player cannot browse rival organizations without acquiring a lead"
    );
    assert!(!directory.contains("investigation-"));
    assert!(
        session
            .execute("prospects")
            .unwrap()
            .0
            .contains("Danny Ferro")
    );
    assert!(
        session
            .execute("known-people")
            .unwrap()
            .0
            .contains("Danny Ferro"),
        "a prospect presented by the recruitment owner is also a known person"
    );
    let before = snapshot(&registry, &session);
    for command in [
        "plan surveillance org-2",
        "plan infiltration org-2",
        "plan surveillance org-999",
        "plan surveillance char-999",
        "plan surveillance operation-999",
        "plan infiltration enterprise-999",
        "plan surveillance investigation-999",
        "plan surveillance district-999",
        "plan surveillance business-999",
        "plan surveillance account-11",
        "plan surveillance evidence-1",
        "plan robbery org-4",
        "plan intimidation district-1",
        "known-targets extraneous",
    ] {
        assert!(session.execute(command).is_err(), "{command} should reject");
        assert_eq!(
            before,
            snapshot(&registry, &session),
            "{command} changed authoritative campaign data on rejection"
        );
    }
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut resumed = Session {
        registry: &registry,
        state: restored,
        player: session.player,
        seeds: session.seeds,
        district: session.district,
        district_variant: session.district_variant,
    };
    assert_eq!(directory, resumed.execute("known-targets").unwrap().0);
    assert_eq!(before, snapshot(&registry, &resumed));
}

#[test]
fn text_play_public_district_surveillance_generates_real_patrol_intelligence() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let confirmation = session.execute("plan surveillance district-1").unwrap().0;
    assert!(
        confirmation.contains("Authorized Surveillance"),
        "{confirmation}"
    );
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap();
    assert_eq!(
        operation.objective(),
        &OperationObjective::GatherInformation {
            target: EntityRef::Neighborhood(
                session.state.world().neighborhoods().next().unwrap().id()
            )
        }
    );
    let report = session.execute("advance 3h").unwrap().0;
    assert!(
        report.contains("operation reached a terminal outcome"),
        "{report}"
    );
    let patrol = session
        .organization_information()
        .find(|info| {
            info.topic() == InformationTopic::PoliceActivity
                && matches!(info.signal(), Some(InformationSignal::PatrolPattern { .. }))
        })
        .expect("completed surveillance must actually observe a patrol rhythm");
    assert!(patrol.summary().contains("South Ward"));
    assert_eq!(patrol.source_kind(), InformationSourceKind::Surveillance);
    let after = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        after,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn text_play_scouts_known_person_and_watches_own_operations_and_rackets() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let prospect = session
        .player_prospects()
        .unwrap()
        .into_iter()
        .find(|id| {
            session
                .state
                .world()
                .get_character(*id)
                .is_some_and(|person| person.name() == "Danny Ferro")
        })
        .unwrap();
    let authorized = session
        .execute(&format!("plan surveillance char-{}", prospect.raw()))
        .unwrap()
        .0;
    assert!(authorized.contains("Danny Ferro"), "{authorized}");
    session.execute("advance 3h").unwrap();
    assert!(
        session
            .organization_information()
            .any(|info| info.subject() == EntityRef::Character(prospect)
                && info.source_kind() == InformationSourceKind::Surveillance)
    );
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    let known = session.execute("known-targets").unwrap().0;
    assert!(known.contains(&format!("operation-{} | your operation", operation.raw())));
    assert!(
        session
            .execute(&format!("plan infiltration operation-{}", operation.raw()))
            .unwrap()
            .0
            .contains("Authorized Infiltration")
    );
    let dossier = session.execute("operation 2").unwrap().0;
    assert!(dossier.contains("Operation(OperationId(1))"));
    session.execute("advance 6h").unwrap();
    assert!(
        session
            .organization_information()
            .any(|info| info.subject() == EntityRef::Operation(operation)
                && info.source_kind() == InformationSourceKind::Surveillance)
    );
    session.execute("plan infiltration enterprise-1").unwrap();
    session.execute("advance 6h").unwrap();
    assert!(session.organization_information().any(|info| matches!(
        info.subject(),
        EntityRef::Enterprise(_)
    ) && info.source_kind()
        == InformationSourceKind::Surveillance));
}

#[test]
fn text_play_precinct_surveillance_discovers_casework_without_exposing_case_ids() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    session.execute("advance 1d").unwrap();
    let police = precinct(&session);
    assert!(
        session
            .state
            .legal()
            .investigations_for_owner(police)
            .next()
            .is_some(),
        "fixed seed robbery must actually initiate official casework"
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute("known-targets")
            .unwrap()
            .0
            .contains("Central Precinct")
    );
    assert!(
        !session
            .execute("known-targets")
            .unwrap()
            .0
            .contains("investigation-"),
        "knowing an institution must not reveal its hidden investigation IDs"
    );
    assert!(
        session
            .execute("plan surveillance investigation-1")
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));

    let confirmation = session
        .execute(&format!("plan surveillance org-{}", police.raw()))
        .unwrap()
        .0;
    assert!(confirmation.contains("Central Precinct"));
    session.execute("advance 3h").unwrap();
    let information = session
        .organization_information()
        .find(|info| {
            info.subject() == EntityRef::Organization(police)
                && matches!(info.signal(), Some(InformationSignal::CaseActivity(_)))
        })
        .expect("surveillance must reveal the case activity attributable to our actual operation");
    assert!(information.summary().contains("actively developing"));
    assert!(
        !session
            .execute("known-targets")
            .unwrap()
            .0
            .contains("investigation-"),
        "an authority-level sightline gives status, not a free case dossier"
    );
}

#[test]
fn text_play_learned_case_can_be_surveilled_without_private_dossier_leak() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan robbery business-1 approach=violent muscle=char-3 coordinator=char-2")
        .unwrap();
    session.execute("advance 1d").unwrap();
    let police = precinct(&session);
    let case = session
        .state
        .legal()
        .investigations_for_owner(police)
        .next()
        .expect("fixed seed robbery opens a police case")
        .id();
    assert!(
        session
            .execute(&format!("plan surveillance investigation-{}", case.raw()))
            .is_err()
    );
    let before = snapshot(&registry, &session);
    validate_record_information(
        &session.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(session.player),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: None,
            subject: EntityRef::Investigation(case),
            observed_at: session.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Specific,
            summary: "A direct observer identified the specific record, but not its current state."
                .to_owned(),
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    assert_ne!(before, snapshot(&registry, &session));
    let directory = session.execute("known-targets").unwrap().0;
    assert!(directory.contains(&format!(
        "investigation-{} | learned case; status unverified",
        case.raw()
    )));
    assert!(
        !directory.contains("Actively developing"),
        "the case's private current status must not be displayed before surveillance"
    );
    let watch = session
        .execute(&format!("plan surveillance investigation-{}", case.raw()))
        .unwrap()
        .0;
    assert!(watch.contains("Authorized Surveillance"), "{watch}");
    session.execute("advance 3h").unwrap();
    assert!(
        session
            .organization_information()
            .any(|info| info.subject() == EntityRef::Investigation(case)
                && matches!(info.signal(), Some(InformationSignal::CaseActivity(_))))
    );
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
