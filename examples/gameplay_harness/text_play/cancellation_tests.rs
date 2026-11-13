//! Canonical authority cancellation through the player terminal.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn text_play_cancel_releases_future_crew_and_persists_abort_report() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session
        .execute("plan intimidation business-1 delay=2h police=ask")
        .unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    let pending = snapshot(&registry, &session);
    assert!(session.execute("cancel operation-999").is_err());
    assert!(session.execute("cancel business-1").is_err());
    assert_eq!(pending, snapshot(&registry, &session));
    let count = session.state.reports().reports_for(session.player).count();
    let receipt = session
        .execute(&format!("cancel operation-{}", operation.raw()))
        .unwrap()
        .0;
    assert!(receipt.contains("authoritative abort report"), "{receipt}");
    let record = session.state.operations().get_operation(operation).unwrap();
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert!(record.abort_record().is_some());
    assert_eq!(
        session.state.reports().reports_for(session.player).count(),
        count + 1
    );
    let aborted = snapshot(&registry, &session);
    assert!(
        session
            .execute(&format!("cancel {}", operation.raw()))
            .is_err()
    );
    assert_eq!(aborted, snapshot(&registry, &session));
    let replan = session
        .execute("plan intimidation business-1 delay=2h police=press")
        .unwrap()
        .0;
    assert!(replan.contains("Authorized Intimidation"), "{replan}");
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        snapshot(&registry, &session),
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}

#[test]
fn text_play_cancel_stops_in_progress_operation_through_owner() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session.execute("plan surveillance district-1").unwrap();
    let id = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    for _ in 0..5 {
        run_tick(&registry, &mut session.state).unwrap();
    }
    assert_eq!(
        session
            .state
            .operations()
            .get_operation(id)
            .unwrap()
            .status(),
        OperationStatus::InProgress
    );
    let receipt = session.execute(&format!("cancel {}", id.raw())).unwrap().0;
    assert!(receipt.contains("Withdrew"), "{receipt}");
    let record = session.state.operations().get_operation(id).unwrap();
    assert_eq!(record.status(), OperationStatus::Aborted);
    assert!(record.abort_record().is_some());
    let after = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        after,
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap()
    );
}
