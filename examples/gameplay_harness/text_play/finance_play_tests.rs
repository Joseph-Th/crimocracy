//! Finance-owner projection and ledger presentation integrity.

use super::*;
use crimocracy::world::BusinessOwner;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn text_play_ledger_is_owner_scoped_and_internal_transfers_are_unique() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let owner = FinancialOwner::Organization(session.player);
    let rival = session
        .state
        .world()
        .public_business_directory()
        .filter_map(|(id, ..)| session.state.world().get_business(id))
        .find_map(|business| match business.owner() {
            BusinessOwner::Organization(id) if id != session.player => Some(id),
            _ => None,
        })
        .expect("fixture must have a rival-owned venue");
    let rival_transaction = session
        .state
        .finance()
        .recent_transactions_for_owner(FinancialOwner::Organization(rival), 10)
        .into_iter()
        .next()
        .expect("rival fixture has a treasury seed")
        .id();
    assert!(
        session
            .state
            .finance()
            .get_transaction(rival_transaction)
            .is_some()
    );
    assert!(
        session
            .state
            .finance()
            .transaction_for_owner(owner, rival_transaction)
            .is_none()
    );
    assert!(
        session
            .state
            .finance()
            .transaction_by_number_for_owner(owner, rival_transaction.raw())
            .is_none()
    );
    assert!(
        session
            .state
            .finance()
            .recent_transactions_for_owner(owner, 0)
            .is_empty()
    );
    let original = snapshot(&registry, &session);
    for command in [
        format!("transaction {}", rival_transaction.raw()),
        "transaction 999999".to_owned(),
        "transaction report-1".to_owned(),
        "transaction".to_owned(),
        "ledger 0".to_owned(),
        "ledger nonsense".to_owned(),
        "ledger 1 extra".to_owned(),
    ] {
        assert!(session.execute(&command).is_err(), "{command} must reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} must not change campaign state"
        );
    }
    let first = session
        .state
        .finance()
        .recent_transactions_for_owner(owner, 10)
        .first()
        .expect("family opening treasury seed")
        .id();
    let details = session
        .execute(&format!("transaction {}", first.raw()))
        .unwrap()
        .0;
    assert!(details.contains("StreetCash"));
    assert!(details.contains("Settlement"));
    assert!(!details.contains("Rosetti"));

    let moved = session.execute("move 11 9 10000").unwrap().0;
    assert!(moved.contains("Allocated 10000 cents"));
    let newest = session
        .state
        .finance()
        .recent_transactions_for_owner(owner, 10);
    assert_eq!(
        newest.len(),
        2,
        "an internal transfer must appear once despite two postings"
    );
    assert!(newest[0].id().raw() > newest[1].id().raw());
    let transfer_id = newest[0].id();
    let rows = session.execute("ledger 2").unwrap().0;
    assert_eq!(
        rows.matches(&format!("transaction-{} |", transfer_id.raw()))
            .count(),
        1
    );
    assert!(rows.contains("debits 10000 cents, credits 10000 cents"));
    let split = session
        .execute(&format!("transaction transaction-{}", transfer_id.raw()))
        .unwrap()
        .0;
    assert!(split.contains("account-11 StreetCash: -10000 cents"));
    assert!(split.contains("account-9 StreetCash: +10000 cents"));
    assert!(split.contains("Net change across your accounts: +0 cents"));
    let after = snapshot(&registry, &session);
    assert!(
        session
            .execute("ledger 30")
            .unwrap()
            .0
            .contains("Seed the family treasury")
    );
    assert_eq!(after, snapshot(&registry, &session));
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let mut resumed = Session {
        registry: &registry,
        state: restored,
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert_eq!(
        session.execute("ledger 10").unwrap().0,
        resumed.execute("ledger 10").unwrap().0
    );
    assert_eq!(after, snapshot(&registry, &resumed));
}

#[test]
fn text_play_ledger_shows_genuine_conversion_cost_and_is_read_only() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session.execute("launder business-3 5000").unwrap();
    let owner = FinancialOwner::Organization(session.player);
    let latest = session
        .state
        .finance()
        .recent_transactions_for_owner(owner, 1);
    let receipt = latest[0];
    assert!(receipt.memo().contains("Launder"));
    let detail = session
        .execute(&format!("transaction {}", receipt.id().raw()))
        .unwrap()
        .0;
    assert!(
        detail.contains("account-11 StreetCash: -5000 cents"),
        "{detail}"
    );
    assert!(detail.contains("AccountedFunds"), "{detail}");
    assert!(
        detail.contains("Net change across your accounts: -750 cents"),
        "{detail}"
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute("ledger 1")
            .unwrap()
            .0
            .contains("debits 5000 cents, credits 4250 cents")
    );
    assert_eq!(before, snapshot(&registry, &session));
}
