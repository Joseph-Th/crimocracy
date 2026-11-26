//! Player-scoped governance and generic operation-planning regressions.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
}

#[test]
fn prospects_explain_exhausted_manager_pitches_until_relationship_changes() {
    use crimocracy::social::{
        RelationshipDimensions, relationship_system::validate_set_relationship,
    };
    use crimocracy::world::{
        AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind,
        world_system::{designate_player_organization, insert_character, insert_organization},
    };
    let registry = crimocracy::build_registry();
    let mut state = AppState::new(17);
    let mut create_org = |name: &str| {
        insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: name.to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap()
    };
    let player = create_org("Test recruiting crew");
    let rival = create_org("Test incumbent crew");
    designate_player_organization(&mut state, player).unwrap();
    let mut create_member = |name: &str, organization, supervisor, autonomy| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor,
                autonomy,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap()
    };
    let boss = create_member("Recruiting executive", player, None, AutonomyLevel::Tight);
    let manager = create_member(
        "Recruiting manager",
        player,
        Some(boss),
        AutonomyLevel::Delegated,
    );
    let incumbent = create_member("Incumbent executive", rival, None, AutonomyLevel::Tight);
    let candidate = create_member(
        "Committed associate",
        rival,
        Some(incumbent),
        AutonomyLevel::Guided,
    );
    let strong = RelationshipDimensions {
        trust: crate::campaign::level(100),
        respect: crate::campaign::level(100),
        affection: crate::campaign::level(100),
        dependence: crate::campaign::level(100),
        ..RelationshipDimensions::zero()
    };
    let weak = RelationshipDimensions {
        trust: crate::campaign::level(10),
        respect: crate::campaign::level(20),
        fear: crate::campaign::level(30),
        affection: crate::campaign::level(5),
        ..RelationshipDimensions::zero()
    };
    for (to, dimensions) in [(incumbent, strong), (manager, weak)] {
        validate_set_relationship(&state, candidate, to, dimensions)
            .unwrap()
            .commit(&mut state)
            .unwrap();
    }
    // Fund both crews so the refusal sequence is not disrupted by unrelated payroll departures.
    for organization in [player, rival] {
        let owner = FinancialOwner::Organization(organization);
        let cash = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::StreetCash,
            },
        )
        .unwrap();
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner,
                kind: AccountKind::Settlement,
            },
        )
        .unwrap();
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Recruitment test opening funds".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: cash,
                        amount: Money::from_cents(2_000_000),
                    },
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-2_000_000),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
    }
    let mut session = Session {
        registry: &registry,
        state,
        player,
        seeds: EvaluationSeeds::new(17, 17),
        district: "Test ward".to_owned(),
        district_variant: "TEST",
    };
    session
        .execute(&format!("delegate char-{} personnel", manager.raw()))
        .unwrap();
    session.execute("policy recruitment delegate").unwrap();
    let cadence = registry
        .recruitment()
        .autonomous_attempt_cadence()
        .as_minutes();
    let cooldown = registry.recruitment().cooldown().as_minutes();
    let step = cooldown.div_ceil(cadence) * cadence;
    let mut approaches = BTreeSet::new();
    let mut latest = None;
    for _ in 0..4 {
        session
            .execute(&format!("advance {step} uninterrupted"))
            .unwrap();
        let attempt = session
            .state
            .recruitment()
            .latest_attempt_for(candidate, player)
            .unwrap();
        assert_eq!(
            attempt.outcome(),
            crimocracy::recruitment::RecruitmentOutcome::Refused
        );
        assert!(approaches.insert(attempt.approach()));
        latest = Some(attempt.id());
    }
    session
        .execute(&format!("advance {step} uninterrupted"))
        .unwrap();
    assert_eq!(
        session
            .state
            .recruitment()
            .latest_attempt_for(candidate, player)
            .map(|attempt| attempt.id()),
        latest
    );
    let before = snapshot(&registry, &session);
    let listing = session.execute("prospects").unwrap().0;
    assert!(
        listing.contains("all recent pitches were refused"),
        "{listing}"
    );
    assert!(
        !listing.contains("may act at the daily recruitment pass"),
        "{listing}"
    );
    assert_eq!(before, snapshot(&registry, &session));
    session.state =
        restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(session.execute("prospects").unwrap().0, listing);
    validate_set_relationship(
        &session.state,
        candidate,
        manager,
        RelationshipDimensions {
            trust: crate::campaign::level(11),
            ..weak
        },
    )
    .unwrap()
    .commit(&mut session.state)
    .unwrap();
    let listing = session.execute("prospects").unwrap().0;
    assert!(
        listing.contains("may act at the daily recruitment pass"),
        "{listing}"
    );
    assert!(
        !listing.contains("all recent pitches were refused"),
        "{listing}"
    );
}

#[test]
fn generated_manager_can_develop_contacts_and_request_recruitment() {
    use crimocracy::world::{AutonomyLevel, CapabilityKind};
    let registry = crimocracy::build_registry();
    let mut session =
        Session::new_generated(&registry, EvaluationSeeds::new(12, 12), CityScale::Small).unwrap();
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| {
            member
                .capability(CapabilityKind::Surveillance)
                .is_some_and(|rating| rating.value() == 91)
        })
        .unwrap()
        .id();
    let member = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| {
            member.id() != manager
                && member
                    .capability(CapabilityKind::Driving)
                    .is_some_and(|rating| rating.value() == 91)
        })
        .unwrap()
        .id();
    session
        .execute(&format!("delegate char-{} personnel", manager.raw()))
        .unwrap();
    let candidates = session
        .manager_prospects()
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == manager)
        .unwrap()
        .1;
    assert!(!candidates.is_empty());
    assert!(
        session
            .execute("prospects")
            .unwrap()
            .0
            .contains("requires delegated or broad autonomy")
    );
    session
        .execute(&format!("autonomy char-{} delegated", manager.raw()))
        .unwrap();
    assert!(
        session
            .execute("prospects")
            .unwrap()
            .0
            .contains("may act at the daily recruitment pass")
    );
    session
        .execute(&format!(
            "supervise char-{} char-{}",
            member.raw(),
            manager.raw()
        ))
        .unwrap();
    assert_eq!(
        session
            .state
            .world()
            .get_character(member)
            .unwrap()
            .supervisor(),
        Some(manager)
    );
    let before = snapshot(&registry, &session);
    assert!(session.execute("autonomy char-999 broad").is_err());
    assert!(
        session
            .execute(&format!("autonomy char-{} delegated", manager.raw()))
            .is_err()
    );
    assert!(
        session
            .execute(&format!(
                "supervise char-{} char-{}",
                member.raw(),
                member.raw()
            ))
            .is_err()
    );
    assert_eq!(before, snapshot(&registry, &session));
    let restored = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        restored.world().get_character(manager).unwrap().autonomy(),
        AutonomyLevel::Delegated
    );
    assert_eq!(
        restored.world().get_character(member).unwrap().supervisor(),
        Some(manager)
    );
    session.execute("advance 1d").unwrap();
    assert!(
        session
            .execute("decisions")
            .unwrap()
            .0
            .to_lowercase()
            .contains("recruit"),
        "manager should raise a recruitment approval"
    );
}

#[test]
fn text_play_governance_policy_and_mandate_changes_are_canonical() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(
        session
            .execute("policies")
            .unwrap()
            .0
            .contains("RequireApproval")
    );
    session.execute("policy recruitment delegate").unwrap();
    assert_eq!(
        session
            .state
            .world()
            .get_organization(session.player)
            .unwrap()
            .policy(PolicyKind::IndependentRecruitment),
        Some(PolicySetting::IndependentRecruitment(
            ApprovalPolicy::Delegated
        )),
    );
    session.execute("delegate char-4 personnel").unwrap();
    let manager = session
        .state
        .world()
        .characters_in_organization(session.player)
        .find(|member| member.id().raw() == 4)
        .unwrap()
        .id();
    let mandate = session
        .state
        .delegation()
        .active_for_manager(manager)
        .unwrap()
        .id();
    session
        .execute(&format!(
            "mandate {} order recruitment approve",
            mandate.raw()
        ))
        .unwrap();
    session
        .execute(&format!(
            "mandate {} budget daily account-13 12000",
            mandate.raw()
        ))
        .unwrap();
    session
        .execute(&format!("mandate {} add district-2", mandate.raw()))
        .unwrap();
    let record = session.state.delegation().get_mandate(mandate).unwrap();
    assert_eq!(record.budget().unwrap().limit.cents(), 12_000);
    assert_eq!(record.scopes().len(), 2);
    assert_eq!(
        record.standing_order(PolicyKind::IndependentRecruitment),
        Some(PolicySetting::IndependentRecruitment(
            ApprovalPolicy::RequireApproval
        ))
    );
    let save = build_save(&registry, &session.state).unwrap();
    let mut restored = Session {
        registry: &registry,
        state: restore_save(&registry, save).unwrap(),
        player: session.player,
        seeds: session.seeds,
        district: session.district.clone(),
        district_variant: session.district_variant,
    };
    assert!(
        restored
            .execute("mandates")
            .unwrap()
            .0
            .contains("12000 cents")
    );
    restored
        .execute(&format!(
            "mandate {} order recruitment inherit",
            mandate.raw()
        ))
        .unwrap();
    restored
        .execute(&format!("mandate {} budget clear", mandate.raw()))
        .unwrap();
    restored
        .execute(&format!("mandate {} remove district-2", mandate.raw()))
        .unwrap();
    restored
        .execute(&format!("mandate {} revoke", mandate.raw()))
        .unwrap();
    assert!(
        restored
            .state
            .delegation()
            .active_for_manager(manager)
            .is_none()
    );
}

#[test]
fn text_play_governance_rejects_rival_authority_and_racket_dependency() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let original = snapshot(&registry, &session);
    for command in [
        "policy recruitment automatic",
        "delegate char-6 enterprise",
        "mandate mandate-2 revoke",
        "mandate mandate-3 remove district-1",
        "mandate mandate-3 revoke",
        "mandate mandate-3 budget daily account-11 1000",
        "mandate mandate-3 order recruitment delegate",
        "mandate mandate-3 order legal automatic",
    ] {
        assert!(session.execute(command).is_err(), "{command} must reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} changed authoritative state on rejection"
        );
    }
}

#[test]
fn text_play_planner_respects_knowledge_and_scarce_crew_skills() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    assert!(
        session
            .execute("operation-kinds")
            .unwrap()
            .0
            .contains("Robbery")
    );
    let original = snapshot(&registry, &session);
    for command in [
        "plan robbery 5",
        "plan burglary 1",
        "plan extraction 1",
        "plan witnesspressure 1",
        "plan robbery 1",
        "plan robbery 1 approach=covert",
        "plan robbery 1 muscle=char-6",
        "plan robbery 1 muscle=char-3 coordinator=char-3",
        "plan robbery 1 invalid=char-3",
        "plan fake 1",
    ] {
        assert!(session.execute(command).is_err(), "{command} should reject");
        assert_eq!(
            original,
            snapshot(&registry, &session),
            "{command} mutated on rejection"
        );
    }
    assert!(
        session
            .execute("plan intimidation business-1")
            .unwrap()
            .0
            .contains("Authorized Intimidation")
    );
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|op| op.kind() == OperationKind::Intimidation)
        .unwrap();
    assert_eq!(
        operation.roles()[&RoleKind::Coordinator].raw(),
        3,
        "the capable subordinate should coordinate by default"
    );
    let before = snapshot(&registry, &session);
    assert!(
        session
            .execute("plan intimidation 1 coordinator=char-3")
            .is_err(),
        "explicit overlapping assignments must still reject"
    );
    assert_eq!(before, snapshot(&registry, &session));
    session.execute("plan intimidation 1").unwrap();
    let crews = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .map(|op| op.participants())
        .collect::<Vec<_>>();
    assert_eq!(crews.len(), 2);
    assert!(
        crews[0].is_disjoint(&crews[1]),
        "automatic staffing must use available trained crew"
    );
}

#[test]
fn text_play_planner_supports_explicit_crew_and_persisted_outcomes() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let confirmation = session
        .execute(
            "plan robbery 1 approach=intimidating muscle=char-3 coordinator=char-2 leader=char-2",
        )
        .unwrap()
        .0;
    assert!(confirmation.contains("Authorized Robbery"));
    let dossier = session.execute("operation 1").unwrap().0;
    assert!(dossier.contains("Intimidating"));
    assert!(dossier.contains("Carlo Venn"));
    assert!(dossier.contains("Joseph Marrow"));
    session.execute("advance 3h").unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .next()
        .unwrap();
    assert!(matches!(
        operation.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ));
    let state = restore_save(&registry, build_save(&registry, &session.state).unwrap()).unwrap();
    assert_eq!(
        snapshot(&registry, &session),
        bincode::serialize(&build_save(&registry, &state).unwrap()).unwrap()
    );
}

#[test]
fn text_play_planner_prioritizes_trained_roles_and_requires_manual_risk_opt_in() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let before = snapshot(&registry, &session);
    let unsupported = session.execute("plan robbery 1").unwrap_err();
    assert!(unsupported.contains("Violence") && unsupported.contains("Muscle=char-N"));
    assert_eq!(before, snapshot(&registry, &session));
    let duplicated = session
        .execute("plan hijacking 1 approach=covert approach=violent")
        .unwrap_err();
    assert!(duplicated.contains("only be specified once"));
    assert_eq!(before, snapshot(&registry, &session));
    let kinds = session.execute("operation-kinds").unwrap().0;
    assert!(kinds.contains("Muscle (Violence)"));
    assert!(kinds.contains("Driver (Driving)"));
    session.execute("plan hijacking business-1").unwrap();
    let operation = session
        .state
        .operations()
        .operations_for_organization(session.player)
        .find(|op| op.kind() == OperationKind::Hijacking)
        .unwrap();
    assert_eq!(operation.roles()[&RoleKind::Driver].raw(), 4);
    assert_eq!(operation.roles()[&RoleKind::Coordinator].raw(), 3);
}

#[test]
fn text_play_district_ids_are_public_and_discoverable() {
    let registry = crimocracy::build_registry();
    let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
    let districts = session.execute("districts").unwrap().0;
    assert!(districts.contains("district-1 | South Ward"));
    assert!(districts.contains("district-2 | Harbor District"));
    let directory = session.execute("directory").unwrap().0;
    assert!(directory.contains("district-2 Harbor District"));
    assert!(
        !directory.contains("Rosetti"),
        "directory must not expose rival ownership"
    );
    session.execute("delegate char-4 district-2").unwrap();
    let mandates = session.execute("mandates").unwrap().0;
    assert!(mandates.contains("district-2 Harbor District"));
}
