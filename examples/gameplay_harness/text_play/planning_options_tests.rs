//! Player-driven operation planning constraints, timing, and police reactions.

use super::*;
use crimocracy::intelligence::InformationTopic;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn preview_exposes_usable_coverage_without_authorizing_and_respects_scheduled_freshness() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = snapshot(&registry, &session);
    let fresh = session
        .execute("PREVIEW documenttheft business-1")
        .unwrap()
        .0;
    assert!(fresh.contains("Preview only"));
    assert!(fresh.contains("Planning intelligence: 1/"), "{fresh}");
    assert!(fresh.contains("TargetSecurity: information-"));
    assert!(fresh.contains("Personnel: missing usable information"));
    assert_eq!(before, snapshot(&registry, &session));
    let stale = session
        .execute("preview documenttheft business-1 delay=7d")
        .unwrap()
        .0;
    assert!(
        stale.contains("TargetSecurity: missing usable information"),
        "{stale}"
    );
    assert_eq!(before, snapshot(&registry, &session));
    let authorized = session
        .execute("plan documenttheft business-1 delay=7d")
        .unwrap()
        .0;
    assert!(authorized.contains("Planning intelligence: 0/"));
    let op = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap();
    assert!(op.intelligence().is_empty());
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn text_play_plan_controls_are_strict_and_never_mutate_on_rejection() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = snapshot(&registry, &session);
    for command in [
        "plan intimidation business-1 police=abort", // No authored entry window
        "plan intimidation business-1 police=notreal",
        "plan intimidation business-1 police=ask police=press",
        "plan intimidation business-1 delay=0",
        "plan intimidation business-1 delay=8d",
        "plan intimidation business-1 delay=garbage",
        "plan intimidation business-1 delay=1h delay=2h",
        "plan intimidation business-1 deadline=0",
        "plan intimidation business-1 deadline=1h deadline=2h",
        "plan intimidation business-1 delay=3h deadline=2h",
        "plan intimidation business-1 need=targetsecurity", // Not an authored relevant topic
        "plan intimidation business-1 need=policeactivity", // No player-held police activity yet
        "plan documenttheft business-1 need=targetsecurity need=targetsecurity",
        "plan intimidation business-1 unknown=somevalue",
        "plan intimidation business-1 deadline=10000000",
    ] {
        assert!(session.execute(command).is_err(), "{command} should reject");
        assert_eq!(
            before,
            snapshot(&registry, &session),
            "{command} must not change authoritative campaign state"
        );
    }
}

#[test]
fn text_play_scheduled_plan_persists_deadline_and_waits_for_start() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let confirmation = session
        .execute("plan intimidation business-1 approach=covert delay=2h deadline=3h police=ask")
        .unwrap()
        .0;
    assert!(
        confirmation.contains("Authorized Intimidation"),
        "{confirmation}"
    );
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap();
    assert_eq!(operation.scheduled_for().as_minutes(), 120);
    assert_eq!(
        operation.contingencies(),
        &[OperationContingency::RequestDecisionOnPoliceArrival]
    );
    assert_eq!(
        operation.constraints(),
        &[OperationConstraint::CompleteBy(
            crimocracy::core::time::SimTime::from_minutes(180)
        )]
    );
    let id = operation.id();
    let dossier = session
        .execute(&format!("operation {}", id.raw()))
        .unwrap()
        .0;
    assert!(dossier.contains("day 1 02:00"));
    assert!(dossier.contains("CompleteBy"));
    assert!(dossier.contains("RequestDecisionOnPoliceArrival"));
    for _ in 0..119 {
        run_tick(&registry, &mut session.state).unwrap();
    }
    assert_eq!(
        session
            .state
            .operations()
            .get_operation(id)
            .unwrap()
            .status(),
        OperationStatus::Authorized
    );
    run_tick(&registry, &mut session.state).unwrap();
    assert_ne!(
        session
            .state
            .operations()
            .get_operation(id)
            .unwrap()
            .status(),
        OperationStatus::Authorized,
        "scheduled operation must start at its authorized window"
    );
    let saved = snapshot(&registry, &session);
    let state = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
}

#[test]
fn text_play_document_theft_uses_known_security_and_real_pre_entry_abort_rule() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let text = session
        .execute("plan documenttheft business-1 need=targetsecurity police=abort")
        .unwrap()
        .0;
    assert!(text.contains("Authorized DocumentTheft"), "{text}");
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap();
    assert_eq!(
        operation.contingencies(),
        &[
            OperationContingency::AbortOnPoliceArrivalBeforeEntry,
            OperationContingency::RequestDecisionOnPoliceArrival,
        ]
    );
    assert_eq!(
        operation.constraints(),
        &[OperationConstraint::RequireIntelligenceTopic(
            InformationTopic::TargetSecurity
        )]
    );
    assert_eq!(
        operation.intelligence().len(),
        1,
        "the held opening target-security observation must accompany the operation"
    );
    let saved = snapshot(&registry, &session);
    let state = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
}

#[test]
fn text_play_intelligence_prerequisite_can_be_earned_by_precinct_reconnaissance() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let precinct = session
        .state
        .contacts()
        .contacts_for_sponsor(session.player)
        .find(|record| record.kind() == ContactKind::Police)
        .unwrap()
        .institution();
    let target = format!("org-{}", precinct.raw());
    let requested =
        format!("plan infiltration {target} need=personnel police=ask delay=2h deadline=1d");
    let before = snapshot(&registry, &session);
    assert!(
        session.execute(&requested).is_err(),
        "information cannot be conjured merely by adding a knowledge gate"
    );
    assert_eq!(before, snapshot(&registry, &session));
    session
        .execute(&format!("plan surveillance {target}"))
        .unwrap();
    session.execute("advance 3h").unwrap();
    let held = session
        .organization_information()
        .find(|info| {
            info.subject() == EntityRef::Organization(precinct)
                && info.topic() == InformationTopic::Personnel
        })
        .expect("a real surveillance result must supply the new personnel observation");
    let source = held.id();
    let accepted = session.execute(&requested).unwrap().0;
    assert!(accepted.contains("Authorized Infiltration"), "{accepted}");
    let plan = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|op| op.kind() == OperationKind::Infiltration)
        .unwrap();
    assert!(plan.intelligence().contains(&source));
    assert_eq!(
        plan.contingencies(),
        &[OperationContingency::RequestDecisionOnPoliceArrival]
    );
    assert!(
        plan.constraints()
            .contains(&OperationConstraint::RequireIntelligenceTopic(
                InformationTopic::Personnel
            ))
    );
    assert_eq!(
        plan.scheduled_for().as_minutes(),
        session.state.now().as_minutes() + 120
    );
    let saved = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
