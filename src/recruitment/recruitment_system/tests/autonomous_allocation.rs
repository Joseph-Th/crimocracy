//! Mixed-policy daily recruitment when one autonomous route runs out of persistent IDs.

use super::*;

fn add_personnel_crew(
    fixture: &mut Fixture,
    policy: ApprovalPolicy,
) -> (OrganizationId, CharacterId, CharacterId) {
    let organization = insert_organization(
        &fixture.registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Independent Recruiting Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second recruiting organization should validate");
    let manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Independent Personnel Manager".to_owned(),
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
            name: "Independent Recruitment Prospect".to_owned(),
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
        add_personnel_crew(&mut fixture, ApprovalPolicy::RequireApproval);
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
fn exhausted_attempt_ids_do_not_cancel_independent_player_approval() {
    let mut fixture = fixture();
    designate_player_organization(&mut fixture.state, fixture.target)
        .expect("recruiting criminal organization should be player-eligible");
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let (_, _, delegated_candidate) = add_personnel_crew(&mut fixture, ApprovalPolicy::Delegated);
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
