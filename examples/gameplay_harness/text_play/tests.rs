//! Text-play contract and end-to-end behavior checks.

use super::*;

#[test]
fn retired_rackets_are_history_not_current_leadership_assets() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .expect("opening has an operating racket")
        .id();
    assert!(session.brief().contains("Rackets: 1"));
    session
        .execute(&format!("racket {} suspend", racket.raw()))
        .unwrap();
    assert!(session.brief().contains("Rackets: 1"));
    session
        .execute(&format!("racket {} retire", racket.raw()))
        .unwrap();
    assert!(session.brief().contains("Rackets: 0"));
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Retired")
    );
    let mut restored = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    restored.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(restored.brief(), session.brief());
}

#[test]
fn obsolete_wait_alias_is_not_a_second_time_mutation_path() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.execute("wait 1m").is_err());
    let after = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn latest_reports_use_ordered_recipient_index_after_restore() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session.execute("advance 1d").unwrap();
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let chronological: Vec<_> = restored
        .reports()
        .reports_for(session.player)
        .map(|report| report.id().raw())
        .collect();
    let recent: Vec<_> = restored
        .reports()
        .reports_for(session.player)
        .rev()
        .take(3)
        .map(|report| report.id().raw())
        .collect();
    assert_eq!(
        recent,
        chronological.into_iter().rev().take(3).collect::<Vec<_>>()
    );
    let display = session.execute("reports 3").unwrap().0;
    let displayed: Vec<_> = display
        .lines()
        .map(|line| {
            line.split_whitespace()
                .next()
                .unwrap()
                .parse::<u32>()
                .unwrap()
        })
        .collect();
    assert_eq!(displayed, recent);
}

#[test]
fn leadership_leads_are_single_counted_across_discovery_and_expiry_reports() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert_eq!(session.known_opportunities().len(), 2);
    assert!(session.brief().contains("Open leads: 2"));
    session.execute("advance 1d").unwrap();
    let all_leads = session.known_opportunities();
    assert_eq!(all_leads.len(), 2);
    assert_eq!(
        all_leads
            .iter()
            .filter(|lead| lead.status() == OpportunityStatus::Expired)
            .count(),
        1
    );
    assert!(session.brief().contains("Open leads: 1"));
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        restored
            .opportunities()
            .opportunities_for_organization(session.player)
            .count(),
        2
    );
}

#[test]
fn text_play_script_runs_canonical_scout_and_reports() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(
        session
            .execute("opportunities")
            .unwrap()
            .0
            .contains("Burglary")
    );
    assert!(session.execute("scout 99999").is_err());
    let business = match session.known_opportunities()[0]
        .context()
        .targets()
        .iter()
        .next()
        .unwrap()
    {
        EntityRef::Business(id) => id.raw(),
        _ => panic!("opening opportunity must target a business"),
    };
    assert!(
        session
            .execute(&format!("scout {business}"))
            .unwrap()
            .0
            .contains("authorized")
    );
    assert!(
        session
            .execute("advance 6h")
            .unwrap()
            .0
            .contains("Advanced")
    );
    assert!(!session.execute("reports").unwrap().0.is_empty());
}

#[test]
fn text_play_ids_and_advancement_are_strict() {
    assert!(parse_number("0", "decision").is_err());
    assert_eq!(parse_number("decision-12", "decision").unwrap(), 12);
    assert!(parse_number("business-12", "decision").is_err());
    assert!(parse_number("foo-12", "decision").is_err());
    assert_eq!(parse_advance("2h").unwrap(), 120);
    assert!(parse_advance("999d").is_err());
    assert!(save_path("../../escape").is_err());
    assert!(parse_options(&["--load".into(), "slot".into(), "--seed".into(), "1".into()]).is_err());
}

#[test]
fn text_play_script_reads_only_player_owned_reports() {
    let registry = crimocracy::build_registry();
    let mut input = std::io::Cursor::new(Vec::<u8>::new());
    let mut output = Vec::new();
    let path = Path::new(SAVE_DIR).join("text-play-test-script.txt");
    std::fs::create_dir_all(SAVE_DIR).unwrap();
    std::fs::write(&path, "brief\nreports\npeople\nquit\n").unwrap();
    let result = run_with_io(
        &registry,
        &[
            "--seed".into(),
            "12".into(),
            "--script".into(),
            path.to_string_lossy().into_owned(),
        ],
        &mut input,
        &mut output,
    );
    let _ = std::fs::remove_file(&path);
    result.unwrap();
    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.contains("Pending decisions"));
    assert!(rendered.contains("Session ended"));
}

#[test]
fn text_play_save_restore_continues_exact_simulation() {
    let registry = crimocracy::build_registry();
    let mut original = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    original.execute("burgle 1 abort").unwrap();
    original.execute("advance 3h").unwrap();
    let slot = format!("text_play_roundtrip_{}", std::process::id());
    let path = save_path(&slot).unwrap();
    original.save(&slot).unwrap();
    original.save(&slot).unwrap(); // Updating an existing slot must work on Windows.
    let mut restored = Session::load(&registry, &slot).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        original.brief(),
        restored.brief(),
        "resuming must preserve opening display labels"
    );
    assert_eq!(restored.state.now(), original.state.now());
    for _ in 0..45 {
        run_tick(&registry, &mut original.state).unwrap();
        run_tick(&registry, &mut restored.state).unwrap();
    }
    let before = bincode::serialize(&build_save(&registry, &original.state).unwrap()).unwrap();
    let after = bincode::serialize(&build_save(&registry, &restored.state).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn text_play_rejects_expiring_lead_before_authorization() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let deadline = session.known_opportunities()[0]
        .valid_until()
        .unwrap()
        .as_minutes();
    while session.state.now().as_minutes() < deadline - 1 {
        run_tick(&registry, &mut session.state).unwrap();
    }
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute("burgle 1 abort")
            .unwrap_err()
            .contains("expires before")
    );
    let after = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn text_play_reconnaissance_can_inform_a_later_burglary() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let target = match session.known_opportunities()[0]
        .context()
        .targets()
        .iter()
        .next()
        .unwrap()
    {
        EntityRef::Business(id) => id.raw(),
        _ => panic!("opening opportunity must target a business"),
    };
    session.execute(&format!("scout {target}")).unwrap();
    session.execute("advance 6h").unwrap();
    let confirmation = session.execute("burgle 1 abort").unwrap().0;
    assert!(confirmation.contains("information item(s)"));
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|operation| operation.kind() == OperationKind::Burglary)
        .unwrap();
    assert!(
        operation.intelligence().len() > 1,
        "new scouting intelligence should contribute to the subsequent plan"
    );
}

#[test]
fn text_play_prospects_and_recruitment_are_organization_scoped() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let prospects = session.player_prospects().unwrap();
    assert_eq!(prospects.len(), 1);
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.execute("recruit 999 appeal").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let candidate = prospects[0];
    let reply = session
        .execute(&format!("recruit {} appeal", candidate.raw()))
        .unwrap()
        .0;
    assert!(reply.contains("Recruitment attempt"));
    assert!(
        session
            .state
            .recruitment()
            .latest_attempt_for(candidate, session.player)
            .is_some()
    );
    assert!(
        session
            .execute(&format!("recruit {} appeal", candidate.raw()))
            .is_err()
    );
}

#[test]
fn text_play_recon_can_discover_new_leads_with_explicit_crew() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(
        session.execute("discover 5").is_err(),
        "public names are not target intelligence"
    );
    assert!(session.execute("scout 5 char-999").is_err());
    session.execute("scout 5 char-5").unwrap();
    session.execute("advance 3h").unwrap();
    let opportunity = session.execute("discover 5").unwrap().0;
    assert!(opportunity.contains("Burglary lead 3"));
    assert!(
        session.execute("discover 5").is_err(),
        "duplicate lead must be rejected canonically"
    );
    session.execute("burgle 3 char-4 char-3 press").unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|record| record.kind() == OperationKind::Burglary)
        .unwrap();
    assert_eq!(
        operation
            .roles()
            .get(&RoleKind::EntrySpecialist)
            .unwrap()
            .raw(),
        4
    );
    assert_eq!(
        operation.roles().get(&RoleKind::Coordinator).unwrap().raw(),
        3
    );
}

#[test]
fn text_play_laundering_and_purchase_use_canonical_finance() {
    use crimocracy::finance::finance_system::validate_record_transaction;
    use crimocracy::finance::{LedgerPosting, LedgerTransactionDraft};
    use crimocracy::world::BusinessOwner;
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let directory = session.execute("directory").unwrap().0;
    assert!(directory.contains("Pier Nine Social Club"));
    assert!(
        !directory.contains("Rosetti"),
        "public directory must not reveal rival ownership"
    );
    let player = session.player;
    let front = session
        .state
        .world()
        .businesses_owned_by_organization(player)
        .find(|business| {
            business
                .functions()
                .contains(&crimocracy::world::BusinessFunction::CashIntensive)
        })
        .unwrap()
        .id();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session.execute("buy 7").is_err(),
        "clean funding is mandatory"
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let laundering = session
        .execute(&format!("launder {} 5000", front.raw()))
        .unwrap()
        .0;
    assert!(laundering.contains("transaction"));
    assert!(
        session
            .execute(&format!("launder {} 9999999", front.raw()))
            .is_err()
    );

    // A canonical external-capital posting supplies clean acquisition funding for
    // this focused adapter test. It does not bypass the acquisition validator.
    let target = session
        .state
        .world()
        .public_business_directory()
        .find(|(_, name, _, _)| *name == "Pier Nine Social Club")
        .unwrap()
        .0;
    let price = registry
        .get_business(session.state.world().get_business(target).unwrap().kind())
        .economics()
        .acquisition_cost();
    let destination = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(player))
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let clearing = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(player))
        .find(|account| account.kind() == AccountKind::Settlement)
        .unwrap()
        .id();
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Additional legitimate capital".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: clearing,
                    amount: Money::from_cents(-price.cents()),
                },
                LedgerPosting {
                    account: destination,
                    amount: price,
                },
            ],
            authorization: None,
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let purchase = session.execute(&format!("buy {}", target.raw())).unwrap().0;
    assert!(purchase.contains("Acquired business"));
    assert_eq!(
        session.state.world().get_business(target).unwrap().owner(),
        BusinessOwner::Organization(player)
    );
    assert!(
        session
            .state
            .economy()
            .get_business_economy(target)
            .is_some()
    );
}

#[test]
fn text_play_racket_lifecycle_uses_owned_enterprise() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(session.execute("racket 999 suspend").is_err());
    let id = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap()
        .id();
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Gambling")
    );
    assert!(
        session
            .execute(&format!("racket {} suspend", id.raw()))
            .unwrap()
            .0
            .contains("Suspended")
    );
    assert!(
        session
            .execute(&format!("racket {} resume", id.raw()))
            .unwrap()
            .0
            .contains("Active")
    );
    assert!(
        session
            .execute(&format!("racket {} suspend", id.raw()))
            .unwrap()
            .0
            .contains("Suspended")
    );
    assert!(
        session
            .execute(&format!("racket {} retire", id.raw()))
            .unwrap()
            .0
            .contains("Retired")
    );
    assert!(
        session
            .execute(&format!("racket {} resume", id.raw()))
            .is_err()
    );
}

#[test]
fn text_play_script_errors_include_line_and_do_not_create_saves() {
    let registry = crimocracy::build_registry();
    let path = Path::new(SAVE_DIR).join("text-play-invalid-script.txt");
    std::fs::create_dir_all(SAVE_DIR).unwrap();
    std::fs::write(&path, "# comment\nbrief\nbuy 9999\nquit\n").unwrap();
    let mut input = std::io::Cursor::new(Vec::<u8>::new());
    let mut output = Vec::new();
    let result = run_with_io(
        &registry,
        &[
            "--seed".into(),
            "12".into(),
            "--script".into(),
            path.to_string_lossy().into_owned(),
        ],
        &mut input,
        &mut output,
    );
    let _ = std::fs::remove_file(path);
    assert!(result.unwrap_err().contains("script line 3"));
}

#[test]
fn text_play_rejects_extra_arguments_without_mutating() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    for command in [
        "help ignored",
        "quit ignored",
        "brief ignored",
        "people ignored",
        "reports 3 ignored",
        "report 1 ignored",
        "save accidental_slot ignored",
        "move 11 9 5000 ignored",
        "decide 1 approve ignored",
        "advance 1h ignored",
        "open-account street ignored",
        "start-racket loan-sharking 3 9 10 4 ignored",
    ] {
        assert!(
            session.execute(command).is_err(),
            "{command} must reject extra arguments"
        );
    }
    let after = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn text_play_treasury_allocation_and_new_racket_survive_a_full_day_and_restore() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let player = session.player;
    let accounts: Vec<_> = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(player))
        .collect();
    let source = accounts
        .iter()
        .find(|account| {
            account.kind() == AccountKind::StreetCash && account.balance().cents() >= 10_000
        })
        .unwrap()
        .id();
    let destination = accounts
        .iter()
        .find(|account| account.kind() == AccountKind::StreetCash && account.balance().cents() == 0)
        .unwrap()
        .id();
    let accounted = accounts
        .iter()
        .find(|account| account.kind() == AccountKind::AccountedFunds)
        .unwrap()
        .id();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute(&format!("move {} {} 10000", source.raw(), accounted.raw()))
            .is_err(),
        "a generic move must not launder street cash into accounted funds"
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session
        .execute(&format!(
            "move {} {} 10000",
            source.raw(),
            destination.raw()
        ))
        .unwrap();
    assert_eq!(
        session
            .state
            .finance()
            .get_account(destination)
            .unwrap()
            .balance()
            .cents(),
        10_000
    );

    let first_enterprise = session
        .state
        .enterprises()
        .enterprises_for_organization(player)
        .next()
        .unwrap();
    let reserved_settlement = first_enterprise.settlement_account();
    let venue = match first_enterprise.location() {
        EnterpriseLocation::Business(id) => id,
        EnterpriseLocation::Neighborhood(_) => {
            panic!("fixture enterprise should be at a storefront")
        }
    };
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute(&format!(
                "start-racket loan-sharking {} {} {}",
                venue.raw(),
                destination.raw(),
                reserved_settlement.raw()
            ))
            .is_err()
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );

    session.execute("open-account settlement").unwrap();
    let fresh_settlement = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(player))
        .filter(|account| account.kind() == AccountKind::Settlement)
        .max_by_key(|account| account.id())
        .unwrap()
        .id();
    let started = session
        .execute(&format!(
            "start-racket loan-sharking {} {} {}",
            venue.raw(),
            destination.raw(),
            fresh_settlement.raw()
        ))
        .unwrap()
        .0;
    assert!(started.contains("Established LoanSharking enterprise"));
    assert_eq!(
        session
            .state
            .enterprises()
            .enterprises_for_organization(player)
            .count(),
        2
    );
    for _ in 0..DAY_MINUTES {
        run_tick(&registry, &mut session.state).unwrap();
    }
    let new_enterprise = session
        .state
        .enterprises()
        .enterprises_for_organization(player)
        .find(|enterprise| enterprise.kind() == EnterpriseKind::LoanSharking)
        .unwrap();
    assert!(
        session
            .state
            .enterprises()
            .latest_cycle(new_enterprise.id())
            .is_some()
    );
    let slot = format!("text_play_economy_{}", std::process::id());
    let path = save_path(&slot).unwrap();
    session.save(&slot).unwrap();
    let restored = Session::load(&registry, &slot).unwrap();
    let _ = std::fs::remove_file(path);
    assert_eq!(
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &restored.state).unwrap()).unwrap(),
    );
}

#[test]
fn text_play_held_property_can_be_liquidated_only_once_through_owned_books() {
    use crimocracy::world::BusinessFunction;
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    session.execute("burgle 1 abort").unwrap();
    session.execute("advance 3h").unwrap();
    assert!(
        session
            .execute("operation 1")
            .unwrap()
            .0
            .contains("Held property estimate")
    );
    let venue = session
        .state
        .world()
        .businesses_owned_by_organization(session.player)
        .find(|business| business.has_function(BusinessFunction::ResaleMarket))
        .unwrap()
        .id();
    let accounts: Vec<_> = session
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(session.player))
        .collect();
    let cash = accounts
        .iter()
        .find(|account| account.kind() == AccountKind::StreetCash && account.balance().cents() > 0)
        .unwrap()
        .id();
    let clearing = accounts
        .iter()
        .find(|account| account.kind() == AccountKind::Settlement && account.balance().cents() < 0)
        .unwrap()
        .id();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute(&format!("deposit 1 {} {}", cash.raw(), clearing.raw()))
            .is_err(),
        "a property haul must not be deposited as cash"
    );
    assert!(
        session
            .execute(&format!("dispose 1 5 {} {}", cash.raw(), clearing.raw()))
            .is_err(),
        "an independent or rival venue is not a player-owned fencing route"
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let disposed = session
        .execute(&format!(
            "dispose 1 {} {} {}",
            venue.raw(),
            cash.raw(),
            clearing.raw()
        ))
        .unwrap()
        .0;
    assert!(disposed.contains("Liquidated operation"));
    assert!(
        session
            .execute("operation 1")
            .unwrap()
            .0
            .contains("Held property: liquidated")
    );
    assert!(
        session
            .execute(&format!(
                "dispose 1 {} {} {}",
                venue.raw(),
                cash.raw(),
                clearing.raw()
            ))
            .is_err()
    );
}
