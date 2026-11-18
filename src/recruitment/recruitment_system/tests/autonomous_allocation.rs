//! Mixed-policy daily recruitment when one autonomous route runs out of persistent IDs.

use super::*;

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
