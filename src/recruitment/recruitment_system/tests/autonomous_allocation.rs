//! Mixed-policy daily recruitment when one autonomous route runs out of persistent IDs.

use super::*;

#[test]
fn institutional_personnel_mandates_do_not_enter_criminal_recruitment() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    let precinct = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Precinct Personnel Office".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .unwrap();
    let officer = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Precinct Personnel Manager".to_owned(),
            organization: Some(precinct),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: precinct,
            manager: officer,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    fixture.state.advance_clock(SimDuration::from_minutes(
        fixture
            .registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes()
            - 1,
    ));
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        let tick = run_tick(&fixture.registry, state);
        assert_eq!(tick.recruitment_attempts.len(), 1);
        let attempt = state
            .recruitment()
            .get_attempt(tick.recruitment_attempts[0])
            .unwrap();
        assert_eq!(attempt.target_organization(), fixture.target);
        assert_eq!(attempt.candidate(), fixture.candidate);
        assert_eq!(
            state.delegation().active_for_manager(officer).unwrap().id(),
            mandate
        );
        assert_eq!(
            state.world().get_character(officer).unwrap().organization(),
            Some(precinct)
        );
        validate_state(state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn rival_recruitment_approval_requires_sole_available_executive() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let other_root = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Rival executive".to_owned(),
            organization: Some(fixture.target),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    fixture
        .state
        .advance_clock(fixture.registry.recruitment().autonomous_attempt_cadence());
    let before = bincode::serialize(&fixture.state).unwrap();
    let mut restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).unwrap(),
    )
    .unwrap();
    for state in [&mut fixture.state, &mut restored] {
        let outcome = apply_due_autonomous_recruitment(&fixture.registry, state).unwrap();
        assert!(outcome.attempts.is_empty());
        assert!(outcome.approval_requests.is_empty());
        assert_eq!(bincode::serialize(state).unwrap(), before);
        crate::world::world_system::validate_set_character_supervisor(
            state,
            fixture.recruiter,
            Some(other_root),
        )
        .unwrap()
        .commit(state)
        .unwrap();
        state.advance_clock(fixture.registry.recruitment().autonomous_attempt_cadence());
        let outcome = apply_due_autonomous_recruitment(&fixture.registry, state).unwrap();
        assert_eq!(outcome.attempts.len(), 1);
        assert_eq!(outcome.approval_requests.len(), 1);
        assert_eq!(
            state
                .decisions()
                .get_decision(outcome.approval_requests[0].decision)
                .unwrap()
                .status(),
            DecisionStatus::Resolved
        );
        validate_invariants(state);
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn detained_executive_blocks_only_approval_required_rival_recruitment() {
    for policy in [ApprovalPolicy::RequireApproval, ApprovalPolicy::Delegated] {
        let mut fixture = fixture();
        let executive = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "Rival executive".to_owned(),
                organization: Some(fixture.target),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        crate::world::world_system::validate_set_character_supervisor(
            &fixture.state,
            fixture.recruiter,
            Some(executive),
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
        assign_personnel_mandate(&mut fixture, Some(policy));
        let arrest = detain_recruitment_character(&mut fixture, executive, "Executive custody");
        fixture
            .state
            .advance_clock(fixture.registry.recruitment().autonomous_attempt_cadence());
        let before = bincode::serialize(&fixture.state).unwrap();
        let outcome =
            apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state).unwrap();
        assert!(outcome.approval_requests.is_empty());
        if policy == ApprovalPolicy::RequireApproval {
            assert!(outcome.attempts.is_empty());
            assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
            crate::legal::arrest_system::validate_release_arrest(&fixture.state, arrest)
                .unwrap()
                .commit(&mut fixture.state)
                .unwrap();
            fixture
                .state
                .advance_clock(fixture.registry.recruitment().autonomous_attempt_cadence());
            let outcome =
                apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state).unwrap();
            assert_eq!(outcome.attempts.len(), 1);
            assert_eq!(outcome.approval_requests.len(), 1);
        } else {
            assert_eq!(outcome.attempts.len(), 1);
        }
        validate_invariants(&fixture.state);
        restore_save(
            &fixture.registry,
            build_save(&fixture.registry, &fixture.state).unwrap(),
        )
        .unwrap();
    }
}

fn add_personnel_crew(
    fixture: &mut Fixture,
    policy: ApprovalPolicy,
    label: &str,
) -> (OrganizationId, CharacterId, CharacterId) {
    let organization = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: format!("{label} Recruiting Crew"),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second recruiting organization should validate");
    let manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: format!("{label} Personnel Manager"),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second recruiting manager should validate");
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(policy),
            )]),
            budget: None,
        },
    )
    .expect("second personnel mandate should validate")
    .commit(&mut fixture.state)
    .expect("second personnel mandate should commit");
    let candidate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: format!("{label} Recruitment Prospect"),
            organization: Some(fixture.source),
            supervisor: Some(fixture.incumbent),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second prospect should validate");
    validate_set_relationship(
        &fixture.state,
        candidate,
        manager,
        relationship(90, 90, 0, 80, 0, 0, 60),
    )
    .expect("second prospect's relationship should validate")
    .commit(&mut fixture.state)
    .expect("second prospect's relationship should commit");
    (organization, manager, candidate)
}

#[test]
fn blocked_incumbent_report_does_not_discard_viable_recruitment_fallback() {
    for policy in [ApprovalPolicy::Delegated, ApprovalPolicy::RequireApproval] {
        let mut fixture = fixture();
        assign_personnel_mandate(&mut fixture, Some(policy));
        validate_set_relationship(
            &fixture.state,
            fixture.candidate,
            fixture.recruiter,
            relationship(100, 100, 0, 100, 100, 0, 100),
        )
        .expect("preferred prospect relationship should validate")
        .commit(&mut fixture.state)
        .expect("preferred prospect relationship should commit");
        let fallback = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "Unaffiliated fallback prospect".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("fallback prospect should validate");
        validate_set_relationship(
            &fixture.state,
            fallback,
            fixture.recruiter,
            relationship(55, 65, 10, 35, 15, 5, 10),
        )
        .expect("fallback relationship should validate")
        .commit(&mut fixture.state)
        .expect("fallback relationship should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        fixture
            .state
            .ids
            .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX);

        let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
            .expect("unavailable incumbent report must not prevent an unaffiliated pitch");
        assert_eq!(outcome.attempts.len(), 1, "{policy:?}");
        assert_eq!(
            outcome.approval_requests.len(),
            usize::from(policy == ApprovalPolicy::RequireApproval)
        );
        let attempt = fixture
            .state
            .recruitment()
            .get_attempt(outcome.attempts[0])
            .expect("fallback attempt should persist");
        assert_eq!(attempt.candidate(), fallback);
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.candidate)
                .expect("blocked prospect should persist")
                .organization(),
            Some(fixture.source)
        );
        assert_eq!(
            fixture.state.ids.next_raw(crate::core::id::IdKind::Report),
            u32::MAX
        );
        validate_state_against_registry(&fixture.registry, &fixture.state)
            .expect("fallback recruitment must preserve valid state");
        let restored = restore_save(
            &fixture.registry,
            build_save(&fixture.registry, &fixture.state).expect("fallback should save"),
        )
        .expect("fallback should restore");
        assert_eq!(
            restored
                .recruitment()
                .get_attempt(outcome.attempts[0])
                .expect("restored fallback attempt should persist")
                .candidate(),
            fallback
        );
        validate_invariants(&restored);
    }
}

#[test]
fn blocked_npc_pitch_does_not_contest_the_last_viable_approval_id() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let (_, blocked_manager, blocked_candidate) =
        add_personnel_crew(&mut fixture, ApprovalPolicy::RequireApproval, "Blocked");
    let viable_candidate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Independent approval prospect".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("independent prospect should validate");
    validate_set_relationship(
        &fixture.state,
        viable_candidate,
        fixture.recruiter,
        relationship(100, 100, 0, 100, 100, 0, 100),
    )
    .expect("first approval prospect relationship should validate")
    .commit(&mut fixture.state)
    .expect("first approval prospect relationship should commit");
    let support = |candidate, manager| {
        recruitment_relationship_support(
            fixture.registry.recruitment(),
            fixture
                .state
                .social()
                .get_relationship(candidate, manager)
                .expect("prospect relationship must exist")
                .dimensions(),
        )
    };
    assert!(
        support(viable_candidate, fixture.recruiter) > support(blocked_candidate, blocked_manager),
        "the viable pitch must reserve its decision before the blocked pitch is considered"
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::Report, u32::MAX);
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::DecisionRequest, u32::MAX - 1);

    let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
        .expect("an individually blocked second pitch must not contest the final approval ID");
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.approval_requests.len(), 1);
    assert_eq!(outcome.approval_requests[0].decision.raw(), u32::MAX - 1);
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("the viable recruitment attempt should persist");
    assert_eq!(attempt.candidate(), viable_candidate);
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(blocked_candidate)
            .expect("blocked prospect must persist")
            .organization(),
        Some(fixture.source)
    );
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("isolated last approval must preserve valid state");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("last approval should save"),
    )
    .expect("last approval should restore");
    validate_invariants(&restored);
}

#[test]
fn exhausted_approval_ids_do_not_cancel_independent_delegated_recruitment() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    let (_, _, approval_candidate) =
        add_personnel_crew(&mut fixture, ApprovalPolicy::RequireApproval, "Approval");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::DecisionRequest, u32::MAX);

    let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
        .expect("blocked approvals must not suppress independent delegated attempts");
    assert_eq!(outcome.attempts.len(), 1);
    assert!(outcome.approval_requests.is_empty());
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("independent delegated attempt should persist");
    assert_eq!(attempt.recruiter(), fixture.recruiter);
    assert_eq!(attempt.candidate(), fixture.candidate);
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(approval_candidate)
            .expect("unapproachable candidate should persist")
            .organization(),
        Some(fixture.source)
    );
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("isolated daily recruitment must preserve cross-domain state");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("recruitment should save"),
    )
    .expect("isolated recruitment should restore");
    validate_invariants(&restored);
}

#[test]
fn competing_approval_ids_do_not_suppress_independent_delegated_recruitment() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    let (_, first_manager, first_prospect) = add_personnel_crew(
        &mut fixture,
        ApprovalPolicy::RequireApproval,
        "First Approval",
    );
    let (_, _, second_prospect) = add_personnel_crew(
        &mut fixture,
        ApprovalPolicy::RequireApproval,
        "Second Approval",
    );
    // The first approval manager would claim the delegated manager's prospect when all
    // routes are eligible. Once the competing approval cohort is deferred, that claim
    // must be discarded and the delegated manager must be allowed to pitch the prospect.
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        first_manager,
        relationship(100, 100, 100, 100, 0, 0, 0),
    )
    .expect("competing recruitment relationship should validate")
    .commit(&mut fixture.state)
    .expect("competing recruitment relationship should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    // Each approval fits independently, but there is only one decision ID left for two
    // pending/resolved requests. Selecting the first by manager ID would be arbitrary.
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::DecisionRequest, u32::MAX - 1);

    let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
        .expect("shared approval scarcity should leave delegated recruitment actionable");
    assert_eq!(outcome.attempts.len(), 1);
    assert!(outcome.approval_requests.is_empty());
    assert_eq!(
        fixture
            .state
            .ids
            .next_raw(crate::core::id::IdKind::DecisionRequest),
        u32::MAX - 1,
        "no contested approval should consume the last available decision ID"
    );
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("delegated pitch should be recorded");
    assert_eq!(attempt.recruiter(), fixture.recruiter);
    assert_eq!(attempt.candidate(), fixture.candidate);
    for candidate in [first_prospect, second_prospect] {
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(candidate)
                .expect("unapproached candidate must still exist")
                .organization(),
            Some(fixture.source)
        );
    }
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("deferred approval cohort must preserve valid state");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("delegated pitch should save"),
    )
    .expect("delegated pitch should restore");
    let restored_attempt = restored
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("restored recruitment attempt must exist");
    assert_eq!(restored_attempt.recruiter(), fixture.recruiter);
    assert_eq!(restored_attempt.candidate(), fixture.candidate);
    validate_invariants(&restored);
}

#[test]
fn contested_attempt_and_information_ids_do_not_withhold_independent_player_approvals() {
    for scarce_kind in [
        crate::core::id::IdKind::RecruitmentAttempt,
        crate::core::id::IdKind::Information,
    ] {
        let mut fixture = fixture();
        designate_player_organization(&mut fixture.state, fixture.target)
            .expect("the recruiting criminal organization should be player-eligible");
        assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
        let (_, _, first_candidate) =
            add_personnel_crew(&mut fixture, ApprovalPolicy::Delegated, "First Delegated");
        let (_, _, second_candidate) =
            add_personnel_crew(&mut fixture, ApprovalPolicy::Delegated, "Second Delegated");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        // Two independent delegated attempts can each fit the last ID in isolation, but
        // allowing only the first would grant a processing-order advantage. The player's
        // decision request requires neither of the contested ID kinds.
        fixture
            .state
            .ids
            .set_next_raw_for_test(scarce_kind, u32::MAX - 1);

        let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
            .expect("contested attempt artifacts must not suppress independent approvals");
        assert!(outcome.attempts.is_empty(), "{scarce_kind:?}");
        assert_eq!(outcome.approval_requests.len(), 1, "{scarce_kind:?}");
        assert_eq!(
            fixture.state.ids.next_raw(scarce_kind),
            u32::MAX - 1,
            "the contested kind must not allocate to the first delegated manager"
        );
        let decision = outcome.approval_requests[0].decision;
        assert_eq!(
            fixture
                .state
                .decisions()
                .pending_for_recruitment_approval(fixture.target, fixture.candidate),
            Some(decision),
        );
        for candidate in [first_candidate, second_candidate] {
            assert_eq!(
                fixture
                    .state
                    .world()
                    .get_character(candidate)
                    .expect("unapproached candidate should persist")
                    .organization(),
                Some(fixture.source),
            );
        }
        validate_state_against_registry(&fixture.registry, &fixture.state)
            .expect("mixed recruitment pass must preserve structural invariants");
        let restored = restore_save(
            &fixture.registry,
            build_save(&fixture.registry, &fixture.state).expect("approval must save"),
        )
        .expect("approval must restore");
        assert_eq!(
            restored
                .decisions()
                .pending_for_recruitment_approval(fixture.target, fixture.candidate),
            Some(decision),
        );
        validate_invariants(&restored);
    }
}

#[test]
fn exhausted_attempt_ids_do_not_cancel_independent_player_approval() {
    let mut fixture = fixture();
    designate_player_organization(&mut fixture.state, fixture.target)
        .expect("recruiting criminal organization should be player-eligible");
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let (_, _, delegated_candidate) =
        add_personnel_crew(&mut fixture, ApprovalPolicy::Delegated, "Delegated");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::RecruitmentAttempt, u32::MAX);

    let outcome = apply_due_autonomous_recruitment(&fixture.registry, &mut fixture.state)
        .expect("blocked autonomous pitches must not suppress player approval requests");
    assert!(outcome.attempts.is_empty());
    assert_eq!(outcome.approval_requests.len(), 1);
    assert_eq!(
        fixture
            .state
            .decisions()
            .pending_for_recruitment_approval(fixture.target, fixture.candidate),
        Some(outcome.approval_requests[0].decision)
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(delegated_candidate)
            .expect("unapproached prospect should persist")
            .organization(),
        Some(fixture.source)
    );
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("player approval must preserve cross-domain state");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state).expect("approval should save"),
    )
    .expect("player approval must survive restore");
    assert_eq!(
        restored
            .decisions()
            .pending_for_recruitment_approval(fixture.target, fixture.candidate),
        Some(outcome.approval_requests[0].decision)
    );
    validate_invariants(&restored);
}
