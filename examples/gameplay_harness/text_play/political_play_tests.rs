use super::*;
use crimocracy::core::invariants::validate_invariants;
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};

#[test]
fn real_text_campaign_orders_expose_funded_civic_work_and_official_election_schedule() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(562_365, 562_365),
        CityScale::Small,
    )
    .unwrap();
    let saved = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let campaign = session.execute("campaign").unwrap().0;
    assert!(campaign.contains("grassroots 0/1000"), "{campaign}");
    assert!(campaign.contains("Next election day 180"), "{campaign}");
    assert!(
        session
            .execute("election")
            .unwrap()
            .0
            .contains("No mayoral ballot yet")
    );
    assert!(
        session
            .execute("political district-1")
            .unwrap()
            .0
            .contains("meetings: venue/contact ready")
    );
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        "political dossiers cannot mutate the world"
    );
    assert!(session.execute("nominate-mayor char-8").is_err());
    assert!(
        session
            .execute("campaign-event district-1 char-9 meetings")
            .is_err()
    );
    assert_eq!(
        saved,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        "rejected orders preserve the complete saved state"
    );
    session.execute("mandate 3 add political").unwrap();

    let funding = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(session.player),
            kind: AccountKind::AccountedFunds,
        },
    )
    .unwrap();
    let clearing = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(session.player),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Fixture campaign cash".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: funding,
                    amount: Money::from_cents(200_000),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-200_000),
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let event = session
        .execute("campaign-event district-1 char-9 meetings")
        .unwrap()
        .0;
    assert!(event.contains("PublicMeetings"), "{event}");
    assert!(event.contains("+70 grassroots"), "{event}");
    let district = session.execute("political district-1").unwrap().0;
    assert!(
        district.contains("machine 10/1000 | grassroots 70/1000"),
        "{district}"
    );
    assert!(district.contains("cooldown 7 day(s)"), "{district}");
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute("campaign-event district-1 char-9 meetings")
            .is_err()
    );
    assert!(session.execute("campaign-event district-1 char-9").is_err());
    assert!(session.execute("nominate-mayor char-8 extra").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session.execute("buy-influence district-1 5000").unwrap();
    let campaign = session.execute("campaign").unwrap().0;
    assert!(
        campaign.contains("machine 11/1000 | grassroots 70/1000"),
        "{campaign}"
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    validate_invariants(&restored);
    session.state = restored;
    assert_eq!(session.execute("campaign").unwrap().0, campaign);
}
