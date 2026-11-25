use super::*;
use crimocracy::core::invariants::validate_invariants;
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};

#[test]
fn political_dossier_reports_real_directors_before_commerce_qualifies() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(562_365, 562_365),
        CityScale::Small,
    )
    .unwrap();
    session.execute("mandate 3 add political").unwrap();
    session.execute("mandate 3 add finance").unwrap();
    session.execute("mandate 3 add district-2").unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    for district in [1, 2] {
        let dossier = session
            .execute(&format!("political district-{district}"))
            .unwrap()
            .0;
        assert!(
            dossier.contains("Current readiness: incomplete"),
            "{dossier}"
        );
        assert!(
            dossier.contains("Local eligible directors: 1 political, 1 financial"),
            "{dossier}"
        );
        assert!(
            dossier.contains(
                "citywide counted organizer: not assigned | financial director: not assigned"
            ),
            "{dossier}"
        );
        assert!(dossier.contains("Electoral contest:"), "{dossier}");
        assert!(dossier.contains("meetings:"), "{dossier}");
        if district == 2 {
            assert!(dossier.contains("No owned storefronts here"), "{dossier}");
        }
    }
    use crimocracy::core::campaign_victory::{
        political_district_progress, progress, progress_and_political_districts,
    };
    let (standing, rows) = progress_and_political_districts(&registry, &session.state).unwrap();
    assert_eq!(standing, progress(&registry, &session.state).unwrap());
    assert_eq!(
        rows,
        political_district_progress(&registry, &session.state).unwrap()
    );
    assert_eq!(rows.len(), session.state.world().neighborhoods().count());
    assert_eq!(standing.staffed_political_districts, 0);
    assert_eq!(standing.mayoral_majority_districts, 0);
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute("political district-2")
            .unwrap()
            .0
            .contains("Local eligible directors: 1 political, 1 financial")
    );
    session.execute("autonomy 9 tight").unwrap();
    assert!(
        session
            .execute("political district-2")
            .unwrap()
            .0
            .contains("Local eligible directors: 0 political, 0 financial")
    );
}

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
    assert!(
        campaign.contains(
            "Next election: day 181 00:00 | earliest ballot for a new filing: day 181 00:00"
        ),
        "{campaign}"
    );
    let election = session.execute("election").unwrap().0;
    assert!(
        election.contains("No mayoral ballot yet. Next election: day 181 00:00."),
        "{election}"
    );
    assert!(
        election.contains("Earliest ballot for a new filing: day 181 00:00."),
        "{election}"
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
                    amount: Money::from_cents(500_000),
                },
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-500_000),
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
    let campaign_after_meetings = session.execute("campaign").unwrap().0;
    assert!(
        campaign_after_meetings.contains("Next daily erosion before specialist output: -5 machine, -3 grassroots, -0 institutions"),
        "{campaign_after_meetings}"
    );
    assert!(campaign_after_meetings.contains("meetings primarily raise grassroots support"));
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
    session.execute("mandate 3 add district-2").unwrap();
    session
        .execute("hire-specialist district-1 char-9 organizer")
        .unwrap();
    let active = session.execute("campaign").unwrap().0;
    assert!(
        active.contains(
            "Campaign contracts: 3000 cents/day total | 3000 cents/day currently payable"
        ),
        "{active}"
    );
    assert!(active.contains("funded daily"), "{active}");
    let before_conflict =
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let rejection = session
        .execute("hire-specialist district-2 char-9 organizer")
        .err()
        .unwrap();
    assert!(
        rejection.contains("already supervises specialists in another district"),
        "{rejection}"
    );
    assert_eq!(
        before_conflict,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session.execute("autonomy 9 tight").unwrap();
    let paused = session.execute("campaign").unwrap().0;
    assert!(
        paused.contains("Campaign contracts: 3000 cents/day total | 0 cents/day currently payable"),
        "{paused}"
    );
    assert!(paused.contains("paused: manager unavailable"), "{paused}");
    let victory = session.execute("victory").unwrap().0;
    assert!(
        victory.contains("campaign contracts: 3000 cents/day, 270000 cents reserved"),
        "{victory}"
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    validate_invariants(&restored);
    session.state = restored;
    assert_eq!(session.execute("campaign").unwrap().0, paused);
}
