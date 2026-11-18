//! Player-scoped governance and generic operation-planning regressions.

use super::*;

fn snapshot(registry: &Registry, session: &Session<'_>) -> Vec<u8> {
    bincode::serialize(&build_save(registry, &session.state).unwrap()).unwrap()
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
