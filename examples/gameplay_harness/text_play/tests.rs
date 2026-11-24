//! Text-play contract and end-to-end behavior checks.

use super::*;

#[test]
fn political_dossier_exposes_title_countdown_without_mutating_or_exposing_rivals() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(203768, 203768),
        CityScale::Small,
    )
    .unwrap();
    let district = session.state.world().neighborhoods().next().unwrap().id();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let dossier = session
        .execute(&format!("political district-{}", district.raw()))
        .unwrap()
        .0;
    assert!(dossier.contains("Political dossier:"), "{dossier}");
    assert!(dossier.contains("title age 0 full day(s)"), "{dossier}");
    assert!(
        dossier.contains("title matures in 1080 day(s)"),
        "{dossier}"
    );
    assert!(dossier.contains("profitable settlements"), "{dossier}");
    assert!(!dossier.contains("rival influence"), "{dossier}");
    assert!(session.execute("political district-99999").is_err());
    assert!(session.execute("political 1 ignored").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        dossier,
        session
            .execute(&format!("political district-{}", district.raw()))
            .unwrap()
            .0
    );
    assert!(dossier.contains("No local Political-function mandate"));
    let candidate = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|person| {
            person.supervisor().is_some()
                && session
                    .state
                    .delegation()
                    .active_for_manager(person.id())
                    .is_none()
        })
        .unwrap()
        .id();
    session
        .execute(&format!("autonomy {} delegated", candidate.raw()))
        .unwrap();
    session
        .execute(&format!(
            "delegate {} district-{}",
            candidate.raw(),
            district.raw()
        ))
        .unwrap();
    let mandate = session
        .state
        .delegation()
        .active_for_manager(candidate)
        .unwrap()
        .id();
    session
        .execute(&format!("mandate {} add political", mandate.raw()))
        .unwrap();
    let appointed = session
        .execute(&format!("political district-{}", district.raw()))
        .unwrap()
        .0;
    assert!(
        appointed.contains(&format!("Local Political-function mandate {}", mandate)),
        "{appointed}"
    );
    assert!(
        appointed.contains("counted only when local commerce qualifies"),
        "{appointed}"
    );
    assert!(
        appointed.contains("title matures in 1080 day(s)"),
        "{appointed}"
    );
}

#[test]
fn uninterrupted_advance_runs_real_ticks_and_skips_optional_pauses_only() {
    let registry = crimocracy::build_registry();
    let seeds = EvaluationSeeds::new(12, 12);
    let mut manual = Session::new(&registry, seeds).unwrap();
    let mut played = Session::new(&registry, seeds).unwrap();
    manual.execute("burgle 1 abort").unwrap();
    played.execute("burgle 1 abort").unwrap();
    let before = bincode::serialize(&build_save(&registry, &played.state).unwrap()).unwrap();
    assert!(played.execute("advance 2d mistaken").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &played.state).unwrap()).unwrap()
    );
    assert_eq!(parse_advance("30d").unwrap(), MAX_ADVANCE);
    assert!(parse_advance("31d").is_err());
    let result = played.execute("advance 3h uninterrupted").unwrap().0;
    assert!(result.contains("Advanced 180 minute(s)"), "{result}");
    assert!(result.contains("Uninterrupted review:"), "{result}");
    for _ in 0..180 {
        run_tick(&registry, &mut manual.state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&build_save(&registry, &played.state).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &manual.state).unwrap()).unwrap(),
        "skipping UI pauses may not skip or change simulation work"
    );
    let restored = restore_save(&registry, build_save(&registry, &played.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&build_save(&registry, &restored).unwrap()).unwrap(),
        bincode::serialize(&build_save(&registry, &played.state).unwrap()).unwrap(),
    );
}

#[test]
fn dissolved_campaign_rejects_mutation_but_preserves_read_only_history_after_restore() {
    use crimocracy::world::world_system::{
        designate_player_organization, insert_character, insert_organization,
    };
    use crimocracy::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

    let registry = crimocracy::build_registry();
    let mut state = AppState::new(41);
    let player = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Unfunded family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut state, player).unwrap();
    let boss = insert_character(
        &mut state,
        CharacterDraft {
            name: "Boss".to_owned(),
            organization: Some(player),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    insert_character(
        &mut state,
        CharacterDraft {
            name: "Associate".to_owned(),
            organization: Some(player),
            supervisor: Some(boss),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let mut session = Session {
        registry: &registry,
        state,
        player,
        seeds: EvaluationSeeds::new(41, 41),
        district: "Test ward".to_owned(),
        district_variant: "TEST",
    };
    for _ in 0..5 {
        session
            .execute("advance 1d")
            .expect("daily payroll must run");
    }
    assert!(session.state.player_organization_defeated());
    assert!(
        session
            .execute("brief")
            .unwrap()
            .0
            .contains("Campaign over")
    );
    for command in [
        "help", "people", "reports", "contacts", "ledger", "recovery",
    ] {
        session
            .execute(command)
            .expect("terminal history stays inspectable");
    }
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    for command in [
        "open-account street",
        "contact 1",
        "contact 1 ask",
        "attention exception off",
        "recruit 3",
        "claim-victory retirement",
        "advance 1m",
    ] {
        assert_eq!(
            session.execute(command),
            Err("Campaign over: your organization dissolved".to_owned()),
            "{command}"
        );
    }
    assert_eq!(
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        before
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.state.player_organization_defeated());
    assert_eq!(
        session.execute("open-account street"),
        Err("Campaign over: your organization dissolved".to_owned())
    );
}

#[test]
fn terminal_command_allowlist_excludes_contact_disclosures_and_all_other_actions() {
    assert!(super::commands::terminal_command_is_read_only("contacts"));
    assert!(super::commands::terminal_command_is_read_only("report"));
    assert!(!super::commands::terminal_command_is_read_only("contact"));
    assert!(!super::commands::terminal_command_is_read_only(
        "open-account"
    ));
    assert!(!super::commands::terminal_command_is_read_only(
        "claim-victory"
    ));
}

#[test]
fn recovery_and_brief_are_read_only_and_explain_dormant_rackets() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let racket = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|enterprise| enterprise.status() == EnterpriseStatus::Active)
        .expect("the opening must offer an operating racket")
        .id();
    session
        .execute(&format!("racket {} suspend", racket.raw()))
        .unwrap();
    let save = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let recovery = session.execute("recovery").unwrap().0;
    assert!(
        recovery.contains(&format!("enterprise-{}", racket.raw())),
        "{recovery}"
    );
    assert!(recovery.contains("Can resume manually:"), "{recovery}");
    let brief = session.execute("brief").unwrap().0;
    assert!(brief.contains("Realized operating net"), "{brief}");
    assert!(brief.contains("Detained staff: 0"), "{brief}");
    assert!(brief.contains("suspended rackets: 1"), "{brief}");
    assert_eq!(
        save,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        "diagnostic views must not change the campaign"
    );
    assert!(session.execute("recovery all").is_err());
    assert!(session.execute("counsel anything").is_err());
    assert_eq!(
        save,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    session.state = restored;
    assert_eq!(session.execute("recovery").unwrap().0, recovery);
}

#[test]
fn player_can_cover_suspended_rackets_atomically_without_automatic_manager_selection() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new_generated(
        &registry,
        EvaluationSeeds::new(582731, 582731),
        CityScale::Small,
    )
    .expect("seeded city should generate");
    let original = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|record| record.status() == EnterpriseStatus::Active)
        .unwrap()
        .id();
    assert!(
        session
            .execute("recruit char-21 advancement")
            .unwrap()
            .0
            .contains("Accepted")
    );
    session.execute("delegate char-21 district-1").unwrap();
    session
        .execute("start-racket protection district-1 98 103 manager=char-21")
        .expect("second racket should establish through the canonical owner");
    let second = session
        .state
        .enterprises()
        .enterprises_for_organization(session.player)
        .find(|record| record.id() != original)
        .unwrap()
        .id();
    session
        .execute(&format!("racket {} suspend", original.raw()))
        .unwrap();
    let snapshot = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let err = session
        .execute(&format!(
            "cover-rackets char-21 enterprise-{} enterprise-{}",
            original.raw(),
            second.raw()
        ))
        .expect_err("active second racket must reject the whole selected cohort");
    assert!(err.contains("enterprise"), "{err}");
    assert_eq!(
        snapshot,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap(),
        "a failed second resumption cannot commit the preceding management handoff"
    );
    assert!(
        session
            .execute(&format!(
                "cover-rackets char-21 enterprise-{} enterprise-{}",
                original.raw(),
                original.raw()
            ))
            .is_err(),
        "duplicate selection is not silently deduplicated"
    );
    assert_eq!(
        snapshot,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );

    let result = session
        .execute(&format!(
            "cover-rackets char-21 enterprise-{}",
            original.raw()
        ))
        .expect("one explicit cover order should reassign and resume");
    assert!(
        result.0.contains("All changes committed together"),
        "{result:?}"
    );
    let record = session
        .state
        .enterprises()
        .get_enterprise(original)
        .unwrap();
    assert_eq!(record.status(), EnterpriseStatus::Active);
    assert_eq!(record.manager().raw(), 21);
    let outlook = session
        .execute(&format!("racket-outlook {}", original.raw()))
        .unwrap()
        .0;
    assert!(
        outlook.contains("Net with one or more own-operation cases"),
        "{outlook}"
    );
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap())
        .expect("composed handoff must save and restore");
    crimocracy::core::invariants::validate_invariants(&restored);
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(original)
            .unwrap()
            .manager()
            .raw(),
        21
    );
}

#[test]
fn command_verbs_are_case_insensitive_across_handlers() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    for command in [
        "help",
        "ledger",
        "policies",
        "contacts",
        "counsel",
        "debriefs",
        "known-targets",
    ] {
        let expected = session.execute(command).unwrap();
        assert_eq!(
            session.execute(&command.to_ascii_uppercase()).unwrap(),
            expected,
            "{command}"
        );
    }
}

#[test]
fn discovery_uses_older_usable_information_when_newer_observation_is_too_weak() {
    use crimocracy::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
        intelligence_system::validate_record_information,
    };
    use crimocracy::world::world_system::{
        designate_player_organization, insert_business, insert_neighborhood, insert_organization,
    };
    use crimocracy::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, NeighborhoodDraft,
        NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile,
        OrganizationDraft, OrganizationKind, Rating,
    };

    let registry = crimocracy::build_registry();
    let mut state = AppState::new(12);
    let player = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Test crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    designate_player_organization(&mut state, player).unwrap();
    let rating = Rating::try_new(70).unwrap();
    let district = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Test ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating,
                    commercial_activity: rating,
                    illicit_demand: rating,
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating,
                },
            },
        },
    )
    .unwrap();
    let target = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Test shop".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood: district,
            owner: BusinessOwner::Independent,
        },
    )
    .unwrap();
    let record = |state: &mut AppState, reliability, specificity| {
        validate_record_information(
            state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(player),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::TargetSecurity,
                source_entity: None,
                subject: EntityRef::Business(target),
                observed_at: state.now(),
                reliability,
                specificity,
                summary: "Observed shop security.".to_owned(),
            },
        )
        .unwrap()
        .commit(state)
        .unwrap()
    };
    let usable = record(&mut state, Reliability::DirectAccess, Specificity::Precise);
    run_tick(&registry, &mut state).unwrap();
    let weak = record(&mut state, Reliability::Unreliable, Specificity::Vague);
    let age = u64::from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .max_intelligence_age()
            .as_minutes(),
    );
    while state.now().as_minutes() < age * 97 / 100 {
        run_tick(&registry, &mut state).unwrap();
    }
    let mut session = Session {
        registry: &registry,
        state,
        player,
        seeds: EvaluationSeeds::new(12, 12),
        district: "Test ward".to_owned(),
        district_variant: "TEST",
    };
    session
        .execute(&format!("discover {}", target.raw()))
        .unwrap();
    let lead = session
        .state
        .opportunities()
        .opportunities_for_organization(player)
        .next()
        .unwrap();
    assert_eq!(lead.source_information(), &BTreeSet::from([usable]));
    assert!(!lead.source_information().contains(&weak));
}

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
    assert!(
        session
            .brief()
            .contains("Rackets: 1 (1 active, 0 suspended)")
    );
    session
        .execute(&format!("racket {} suspend", racket.raw()))
        .unwrap();
    assert!(
        session
            .brief()
            .contains("Rackets: 1 (0 active, 1 suspended)")
    );
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
fn treasury_standing_order_is_explicit_and_persistent() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(session.execute("treasury").unwrap().0.contains("manual"));
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.execute("treasury invalid").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    session.execute("treasury daily").unwrap();
    assert!(session.brief().contains("distribution: daily"));
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.state.automatic_profit_sweeps());
    session.execute("treasury manual").unwrap();
    assert!(!session.state.automatic_profit_sweeps());
}

#[test]
fn owner_surplus_projection_rounds_portfolio_fee_per_storefront() {
    let surpluses = [Money::from_cents(101), Money::from_cents(101)];
    assert_eq!(estimate_owner_distribution(surpluses, 300), (202, 194));
    assert_eq!(estimate_owner_distribution(surpluses, 0), (202, 202));
}

#[test]
fn leadership_runway_counts_organization_operating_cash_eligible_for_payroll() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let before = session.brief();
    let original_runway = before
        .lines()
        .find(|line| line.starts_with("Gross payroll runway:"))
        .expect("opening brief must show the available wage runway");

    let owner = FinancialOwner::Organization(session.player);
    let wages = registry.upkeep().per_member_daily().cents()
        * session
            .state
            .world()
            .characters_in_organization(session.player)
            .count() as i64;
    let source = session
        .state
        .finance()
        .accounts_for(owner)
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && account.available_payment_balance().cents() >= wages
        })
        .expect("generated organization starts with an adequately funded street account")
        .id();
    let operating = insert_account(
        &mut session.state,
        FinancialAccountDraft {
            owner,
            kind: AccountKind::LegitimateOperating,
        },
    )
    .expect("organization-owned operating account must open");
    let amount = Money::from_cents(wages);
    validate_record_transaction(
        &session.state,
        LedgerTransactionDraft {
            occurred_at: session.state.now(),
            memo: "Reallocate an existing wage reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: amount.checked_neg().unwrap(),
                },
                LedgerPosting {
                    account: operating,
                    amount,
                },
            ],
            authorization: None,
        },
    )
    .expect("balanced transfer between organization accounts must validate")
    .commit(&mut session.state)
    .expect("reallocation must settle");

    let after = session.brief();
    assert!(
        after.contains(&format!("operating {wages} cents")),
        "{after}"
    );
    assert_eq!(
        after
            .lines()
            .find(|line| line.starts_with("Gross payroll runway:")),
        Some(original_runway),
        "moving existing liquid wages between eligible account kinds cannot shorten runway"
    );
    assert!(
        after.contains(&format!("covered from current treasury: {wages} cents")),
        "{after}"
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        after,
        session.brief(),
        "a restored brief must retain the exact projection"
    );
}

#[test]
fn depleted_manual_treasury_warns_about_earned_but_unwithdrawn_payroll_cash() {
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    session.execute("advance 10d uninterrupted").unwrap();
    let brief = session.brief();
    assert!(
        brief.contains("covered from current treasury: 0 cents"),
        "{brief}"
    );
    assert!(
        brief.contains("Earned storefront cash cannot pay wages until withdrawn."),
        "{brief}"
    );
    assert!(brief.contains("distribution: manual"), "{brief}");
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(session.brief(), brief);

    session.execute("treasury daily").unwrap();
    let updated = session.brief();
    assert!(updated.contains("distribution: daily"), "{updated}");
    assert!(
        !updated.contains("Earned storefront cash cannot pay wages until withdrawn."),
        "{updated}"
    );
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
fn text_play_save_does_not_overwrite_an_unrelated_temporary_file() {
    let registry = crimocracy::build_registry();
    let session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let slot = format!("text_play_distinct_temporary_{}", std::process::id());
    let path = save_path(&slot).unwrap();
    let unrelated = path.with_extension("sav.tmp");
    std::fs::create_dir_all(SAVE_DIR).unwrap();
    std::fs::write(&unrelated, b"another writer's unfinished save").unwrap();

    session.save(&slot).unwrap();
    let restored = Session::load(&registry, &slot).unwrap();
    let temporary_contents = std::fs::read(&unrelated);
    let _ = std::fs::remove_file(&unrelated);
    let _ = std::fs::remove_file(&path);

    assert_eq!(restored.brief(), session.brief());
    assert_eq!(
        temporary_contents.unwrap(),
        b"another writer's unfinished save",
        "publishing a save must never overwrite or consume another writer's staging file"
    );
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
    let candidate = *prospects.keys().next().unwrap();
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
fn underboss_requires_a_command_network_and_recruitment_can_expand_the_authored_crew() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let progress = session.execute("victory").unwrap().0;
    assert!(
        progress.contains("Named command staff: 4 / 14 required"),
        "{progress}"
    );
    assert!(progress.contains("independently captained mature districts"));
    let initial = session.execute("prospects").unwrap().0;
    assert!(initial.contains("Danny Ferro"));
    assert!(
        !initial.contains("Grace Mercer"),
        "unconnected outsiders must stay hidden"
    );
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(session.execute("claim-victory underboss").is_err());
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
    );
    let accepted = session.execute("recruit 7 advancement").unwrap().0;
    assert!(accepted.contains("Accepted"), "{accepted}");
    session.execute("autonomy 7 delegated").unwrap();
    session.execute("delegate 7 personnel").unwrap();
    let next_circle = session.execute("prospects").unwrap().0;
    for candidate in ["Grace Mercer", "Walter Kane", "Evelyn Cross", "Daniel Ward"] {
        assert!(
            next_circle.contains(candidate),
            "{candidate} absent: {next_circle}"
        );
    }
    assert!(
        session
            .execute("victory")
            .unwrap()
            .0
            .contains("Named command staff: 5 / 14 required")
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert!(
        session
            .execute("prospects")
            .unwrap()
            .0
            .contains("Grace Mercer")
    );
    let personnel_manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| member.name() == "Danny Ferro")
        .unwrap()
        .id();
    session.execute("policy recruitment delegate").unwrap();
    session.execute("advance 1d").unwrap();
    let recruited = session
        .state
        .world()
        .characters_in_organization(session.player)
        .filter(|member| member.supervisor() == Some(personnel_manager))
        .count();
    assert!(
        recruited >= 1,
        "delegated personnel authority must grow the real roster"
    );
    assert!(
        session
            .execute("victory")
            .unwrap()
            .0
            .contains("Named command staff: 6 / 14 required")
    );
}

#[test]
fn text_play_staffing_is_authoritative_financially_visible_and_persistent() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap();
    let brief = session.execute("brief").unwrap().0;
    assert!(
        brief.contains("People: 4 named | Associates: 5 active"),
        "{brief}"
    );
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Rank-and-file associates: 5")
    );
    for command in [
        "staff-racket 1 0",
        "staff-racket 1 4",
        "staff-racket 1 14",
        "staff-racket 1 5",
        "staff-racket 999 6",
        "staff-racket 1 -1",
    ] {
        assert!(
            session.execute(command).is_err(),
            "{command} must be rejected"
        );
        assert_eq!(
            before,
            bincode::serialize(&build_save(&registry, &session.state).unwrap()).unwrap()
        );
    }
    let baseline = session.execute("racket-outlook 1").unwrap().0;
    assert!(
        session
            .execute("staff-racket 1 9")
            .unwrap()
            .0
            .contains("9 rank-and-file associates assigned")
    );
    assert_ne!(baseline, session.execute("racket-outlook 1").unwrap().0);
    assert!(
        session
            .execute("enterprises")
            .unwrap()
            .0
            .contains("Rank-and-file associates: 9")
    );
    assert!(
        session
            .execute("brief")
            .unwrap()
            .0
            .contains("Associates: 9 active")
    );
    assert!(
        session
            .execute("brief")
            .unwrap()
            .0
            .contains("Additional associate wages: 6000 cents/day")
    );
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    session.execute("advance 1d").unwrap();
    let racket = session
        .state
        .enterprises()
        .get_enterprise(
            session
                .state
                .enterprises()
                .enterprises_for_organization(session.player)
                .next()
                .unwrap()
                .id(),
        )
        .unwrap();
    assert_eq!(racket.associates(), 9);
    assert!(
        session
            .state
            .enterprises()
            .latest_cycle(racket.id())
            .is_some()
    );
    session.execute("staff-racket 1 5").unwrap();
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    let first = restored
        .enterprises()
        .enterprises_for_organization(session.player)
        .next()
        .unwrap();
    let cycle = restored.enterprises().latest_cycle(first.id()).unwrap();
    assert_eq!(first.associates(), 5);
    assert_eq!(first.associates_at(cycle.occurred_at()), 9);
    assert!(
        session
            .execute("victory")
            .unwrap()
            .0
            .contains("Proven rank-and-file associates: 0")
    );
}

#[test]
fn text_play_recruits_through_available_executive_connection() {
    use crimocracy::social::{
        RelationshipDimensions, relationship_system::validate_set_relationship,
    };
    use crimocracy::world::{AutonomyLevel, CharacterDraft, world_system::insert_character};

    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let leader = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Second Executive".to_owned(),
            organization: Some(session.player),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let candidate = insert_character(
        &mut session.state,
        CharacterDraft {
            name: "Executive Connection".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    validate_set_relationship(
        &session.state,
        candidate,
        leader,
        RelationshipDimensions {
            trust: crate::campaign::level(70),
            respect: crate::campaign::level(60),
            ..RelationshipDimensions::zero()
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();

    assert!(
        find_recruitment_candidates(&registry, &session.state, session.player, leader)
            .unwrap()
            .contains(&candidate)
    );
    assert!(
        session
            .execute("prospects")
            .unwrap()
            .0
            .contains("Executive Connection")
    );
    assert!(
        session
            .execute("known-people")
            .unwrap()
            .0
            .contains("Executive Connection")
    );
    session
        .execute(&format!("recruit {} appeal", candidate.raw()))
        .unwrap();
    assert_eq!(
        session
            .state
            .recruitment()
            .latest_attempt_for(candidate, session.player)
            .unwrap()
            .recruiter(),
        leader
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
fn burglary_police_modes_have_distinct_arrival_outcomes() {
    let registry = crimocracy::build_registry();
    for (mode, expected, status, pending) in [
        (
            "abort",
            vec![
                OperationContingency::AbortOnPoliceArrivalBeforeEntry,
                OperationContingency::RequestDecisionOnPoliceArrival,
            ],
            OperationStatus::Aborted,
            0,
        ),
        (
            "ask",
            vec![OperationContingency::RequestDecisionOnPoliceArrival],
            OperationStatus::AwaitingDecision,
            1,
        ),
        ("press", vec![], OperationStatus::InProgress, 0),
    ] {
        let mut session =
            Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small)
                .unwrap();
        session.execute(&format!("burgle 1 {mode}")).unwrap();
        session.execute("advance 10m").unwrap();
        let operation = session
            .state
            .operations()
            .operations_for_organization(session.player)
            .find(|record| record.kind() == OperationKind::Burglary)
            .unwrap();
        assert_eq!(operation.contingencies(), expected, "{mode}");
        assert_eq!(operation.status(), status, "{mode}");
        assert_eq!(
            session
                .state
                .decisions()
                .pending_for_recipient(session.player)
                .count(),
            pending,
            "{mode}"
        );
    }
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
