//! Player-facing capitalization through the real finance owner and saved campaign.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn clean_reserves_can_refund_a_street_account_without_hiding_ledger_postings() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session.execute("launder business-3 5000").unwrap();
    let owner = FinancialOwner::Organization(session.player);
    let accounted = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let street = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| {
            account.kind() == AccountKind::StreetCash && account.balance() >= Money::ZERO
        })
        .unwrap()
        .id();
    let clean_before = session
        .state
        .finance()
        .get_account(accounted)
        .unwrap()
        .balance();
    let street_before = session
        .state
        .finance()
        .get_account(street)
        .unwrap()
        .balance();
    let original = snapshot(&registry, &session);
    for command in [
        format!("fund-street {} {} 0", accounted.raw(), street.raw()),
        format!("fund-street {} {} 10000000", accounted.raw(), street.raw()),
        format!("fund-street {} {} 100", street.raw(), accounted.raw()),
        format!("fund-street {} {} 100", accounted.raw(), accounted.raw()),
        format!("fund-street {} {} 100", accounted.raw(), 999999),
        format!("move {} {} 100", accounted.raw(), street.raw()),
    ] {
        assert!(session.execute(&command).is_err(), "{command} must reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} mutated state"
        );
    }
    let response = session
        .execute(&format!(
            "fund-street {} {} 1000",
            accounted.raw(),
            street.raw()
        ))
        .unwrap()
        .0;
    assert!(
        response.contains("Allocated 1000 cents of working cash"),
        "{response}"
    );
    assert_eq!(
        session
            .state
            .finance()
            .get_account(accounted)
            .unwrap()
            .balance()
            .cents(),
        clean_before.cents() - 1000
    );
    assert_eq!(
        session
            .state
            .finance()
            .get_account(street)
            .unwrap()
            .balance()
            .cents(),
        street_before.cents() + 1000
    );
    let brief = session.brief();
    assert!(brief.contains("Spendable now: street"), "{brief}");
    assert!(brief.contains("separate account deficits"), "{brief}");
    assert!(session.execute("accounts").unwrap().0.contains("spendable"));
    let receipt = session
        .state
        .finance()
        .recent_transactions_for_owner(owner, 1)[0]
        .id();
    let detail = session
        .execute(&format!("transaction {}", receipt.raw()))
        .unwrap()
        .0;
    assert!(detail.contains("Treasury capitalization:"), "{detail}");
    assert!(detail.contains("Net change across your accounts: +0 cents"));
    let before_save = snapshot(&registry, &session);
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let resumed = Session {
        registry: &registry,
        state: restored,
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert_eq!(before_save, snapshot(&registry, &resumed));
    assert_eq!(session.brief(), resumed.brief());
}
