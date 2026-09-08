//! Focused tests for relationship-gated recruitment, autonomy, cooldowns, and approvals.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{LoadError, SaveEnvelope, build_save, restore_save};
use crate::core::time::SimDuration;
use crate::decisions::decision_system::{
    DecisionError, validate_request_recruitment_approval, validate_resolve_decision,
};
use crate::decisions::{
    DecisionContext, DecisionResponse, DecisionStatus, RecruitmentApprovalRequestDraft,
};
use crate::delegation::delegation_system::{validate_assign_mandate, validate_revoke_mandate};
use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{InformationDraft, InformationSourceKind, Reliability, Specificity};
use crate::recruitment::autonomous_recruitment::{
    AutonomousRecruitmentError, apply_due_autonomous_recruitment,
};
use crate::reports::ReportKind;
use crate::reputation::reputation_system::{apply_reputation_delta, resolve_score};
use crate::reputation::{AudienceKind, ReputationDimension};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{
    insert_character, insert_organization, set_policy, validate_reassign_character,
};
use crate::world::{
    ApprovalPolicy, AutonomyLevel, CapabilityKind, CharacterDraft, DriveKind, OrganizationDraft,
    PolicyKind, PolicySetting, Rating, TraitKind,
};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    source: OrganizationId,
    target: OrganizationId,
    incumbent: CharacterId,
    recruiter: CharacterId,
    candidate: CharacterId,
}

#[test]
fn candidate_discovery_skips_temporarily_bound_prospects() {
    let mut fixture = fixture();
    let subordinate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Candidate's Soldier".to_owned(),
            organization: Some(fixture.source),
            supervisor: Some(fixture.candidate),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("direct-report fixture should validate");

    let candidates = find_recruitment_candidates(
        &fixture.registry,
        &fixture.state,
        fixture.target,
        fixture.recruiter,
    )
    .expect("ordinary personnel obligations should filter rather than fail discovery");
    assert!(candidates.is_empty());
    assert_eq!(
        validate_reassign_character(
            &fixture.state,
            fixture.candidate,
            Some(fixture.target),
            Some(fixture.recruiter),
        )
        .expect_err("candidate should remain blocked by their direct report"),
        WorldError::DirectReportAssignment {
            character: fixture.candidate,
            direct_report: subordinate,
        }
    );
    validate_invariants(&fixture.state);
}

fn tamper_serialized_summary(
    envelope: SaveEnvelope,
    summary: &str,
    expected_occurrences: usize,
) -> SaveEnvelope {
    assert!(!summary.is_empty());
    assert!(
        summary.is_ascii(),
        "fixture summaries must be byte-stable ASCII"
    );
    let needle = summary.as_bytes();
    let mut bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let mut cursor = 0;
    let mut replaced = 0;
    while cursor + needle.len() <= bytes.len() {
        let Some(relative) = bytes[cursor..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            break;
        };
        let start = cursor + relative;
        bytes[start..start + needle.len()].fill(b'X');
        cursor = start + needle.len();
        replaced += 1;
    }
    assert_eq!(
        replaced, expected_occurrences,
        "test must corrupt exactly the intended persisted summary copies"
    );
    bincode::deserialize(&bytes).expect("equal-length summary corruption must remain decodable")
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
}

fn relationship(
    trust: u8,
    respect: u8,
    fear: u8,
    affection: u8,
    dependence: u8,
    resentment: u8,
    debt: u8,
) -> RelationshipDimensions {
    RelationshipDimensions {
        trust: level(trust),
        respect: level(respect),
        fear: level(fear),
        affection: level(affection),
        dependence: level(dependence),
        resentment: level(resentment),
        debt: level(debt),
    }
}

fn assign_personnel_mandate(
    fixture: &mut Fixture,
    recruitment_policy: Option<ApprovalPolicy>,
) -> crate::core::id::MandateId {
    let standing_orders = recruitment_policy
        .map(|policy| {
            BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(policy),
            )])
        })
        .unwrap_or_default();
    validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.target,
            manager: fixture.recruiter,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders,
            budget: None,
        },
    )
    .expect("personnel mandate should validate")
    .commit(&mut fixture.state)
    .expect("personnel mandate should commit")
}

fn personnel_authority(fixture: &Fixture, mandate: crate::core::id::MandateId) -> MandateAuthority {
    MandateAuthority {
        mandate,
        manager: fixture.recruiter,
        scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
    }
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0x5EC2_1933);
    let source = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "North Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("source organization should validate");
    let target = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "South Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("target organization should validate");
    let incumbent = insert_character(
        &mut state,
        CharacterDraft {
            name: "Incumbent Lieutenant".to_owned(),
            organization: Some(source),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("incumbent should validate");
    let recruiter = insert_character(
        &mut state,
        CharacterDraft {
            name: "Rival Recruiter".to_owned(),
            organization: Some(target),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(90))]),
            traits: BTreeSet::from([TraitKind::Charismatic]),
            drives: BTreeMap::new(),
        },
    )
    .expect("recruiter should validate");
    let candidate = insert_character(
        &mut state,
        CharacterDraft {
            name: "Frightened Associate".to_owned(),
            organization: Some(source),
            supervisor: Some(incumbent),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::from([TraitKind::EasilyFrightened]),
            drives: BTreeMap::from([(DriveKind::Safety, rating(90))]),
        },
    )
    .expect("candidate should validate");
    validate_set_relationship(
        &state,
        candidate,
        recruiter,
        relationship(55, 65, 10, 35, 15, 5, 10),
    )
    .expect("candidate-recruiter relationship should validate")
    .commit(&mut state);
    Fixture {
        registry,
        state,
        source,
        target,
        incumbent,
        recruiter,
        candidate,
    }
}

fn protection_draft(fixture: &Fixture) -> RecruitmentDraft {
    RecruitmentDraft {
        target_organization: fixture.target,
        recruiter: fixture.recruiter,
        candidate: fixture.candidate,
        approach: RecruitmentApproach::Protection,
    }
}

#[test]
fn executive_channel_rejects_supervised_recruiters() {
    let mut fixture = fixture();
    let supervised = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Supervised Soldier".to_owned(),
            organization: Some(fixture.target),
            supervisor: Some(fixture.recruiter),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("supervised member should validate");
    let error = match validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        RecruitmentDraft {
            target_organization: fixture.target,
            recruiter: supervised,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("supervised members must not bypass recruitment policy"),
    };
    assert_eq!(
        error,
        RecruitmentError::ExecutiveRecruiterSupervised {
            recruiter: supervised,
            supervisor: fixture.recruiter,
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .organization(),
        Some(fixture.source)
    );
    validate_state(&fixture.state).expect("rejected recruitment state should be unchanged");
}

#[test]
fn candidate_discovery_follows_incoming_relationships_not_global_roster() {
    let mut fixture = fixture();
    let unrelated = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unrelated Associate".to_owned(),
            organization: Some(fixture.source),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unrelated character should validate");
    validate_set_relationship(
        &fixture.state,
        fixture.recruiter,
        unrelated,
        relationship(90, 90, 0, 50, 0, 0, 0),
    )
    .expect("reverse-direction relationship should validate")
    .commit(&mut fixture.state);

    let candidates = find_recruitment_candidates(
        &fixture.registry,
        &fixture.state,
        fixture.target,
        fixture.recruiter,
    )
    .expect("candidate discovery should validate");
    assert_eq!(candidates, vec![fixture.candidate]);
    validate_invariants(&fixture.state);
}

#[test]
fn validated_recruitment_stales_when_target_competence_changes_before_commit() {
    let mut fixture = fixture();
    let draft = protection_draft(&fixture);
    let expected = resolve_score(
        &fixture.registry,
        fixture.state.reputation(),
        fixture.target,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
    );
    let validated = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("fresh recruitment should validate");

    let found = apply_reputation_delta(
        &fixture.registry,
        &mut fixture.state,
        fixture.target,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        1,
    )
    .expect("canonical reputation change should commit");
    assert_ne!(found, expected);

    let error = validated
        .commit(&mut fixture.state)
        .expect_err("recruitment must not commit a willingness result from stale competence");
    assert_eq!(
        error,
        RecruitmentError::StaleOrganizationCompetence {
            organization: fixture.target,
            expected,
            found,
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .organization(),
        Some(fixture.source),
        "stale recruitment rejection must not move the candidate"
    );
    assert!(
        fixture.state.recruitment().attempts().next().is_none(),
        "stale recruitment rejection must not record an attempt"
    );
    validate_state(&fixture.state).expect("reputation movement plus rejected attempt stays valid");
    validate_invariants(&fixture.state);
}

#[test]
fn day_boundary_recruitment_uses_current_decayed_reputation() {
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    let baseline = fixture.registry.reputation().baseline();
    crate::reputation::reputation_system::apply_reputation_delta(
        &fixture.registry,
        &mut fixture.state,
        fixture.target,
        crate::reputation::AudienceKind::Underworld,
        crate::reputation::ReputationDimension::Competence,
        3,
    )
    .expect("fixture competence reputation should apply");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));

    let outcome = crate::core::simulation::run_tick(&fixture.registry, &mut fixture.state);
    assert_eq!(outcome.recruitment_attempts.len(), 1);
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.recruitment_attempts[0])
        .expect("day-boundary autonomous attempt should persist");
    assert_eq!(
        attempt.factors().organization_competence(),
        baseline + 2,
        "daily reputation decay must settle before the same-minute recruitment decision"
    );
    assert_eq!(
        crate::reputation::reputation_system::resolve_score(
            &fixture.registry,
            fixture.state.reputation(),
            fixture.target,
            crate::reputation::AudienceKind::Underworld,
            crate::reputation::ReputationDimension::Competence,
        ),
        baseline + 2,
        "the persisted attempt must quote the same current reputation the world now holds"
    );
    validate_state(&fixture.state).expect("current-reputation recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_broad_manager_attempts_recruitment_on_authored_cadence() {
    let registry = build_registry();
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    assert!(
        apply_due_autonomous_recruitment(&registry, &mut fixture.state)
            .expect("non-due autonomous recruitment should still validate")
            .attempts
            .is_empty()
    );
    fixture.state.advance_clock(SimDuration::ONE_MINUTE);
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("due autonomous recruitment should validate");
    let attempts = outcome.attempts;
    assert_eq!(attempts.len(), 1);
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(attempts[0])
        .expect("autonomous attempt should persist");
    assert_eq!(attempt.recruiter(), fixture.recruiter);
    assert_eq!(attempt.candidate(), fixture.candidate);
    assert_eq!(attempt.approach(), RecruitmentApproach::PersonalAppeal);
    assert!(matches!(
        attempt.authority(),
        RecruitmentAuthority::Delegated {
            mandate: found_mandate,
            manager,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
            policy: ApprovalPolicy::Delegated,
            ..
        } if found_mandate == mandate && manager == fixture.recruiter
    ));
    assert!(
        apply_due_autonomous_recruitment(&registry, &mut fixture.state)
            .expect("post-cadence autonomous recruitment should validate")
            .attempts
            .is_empty()
    );
    validate_state(&fixture.state).expect("autonomous recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn failed_delegated_autonomous_recruitment_is_atomic() {
    let registry = build_registry();
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    fixture
        .state
        .ids
        .set_next_raw_for_test(crate::core::id::IdKind::RecruitmentAttempt, u32::MAX);
    let error = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect_err("attempt allocator exhaustion must reject autonomous recruitment");
    assert!(matches!(
        error,
        AutonomousRecruitmentError::Recruitment(RecruitmentError::IdExhaustion(
            crate::core::id::IdExhaustionError::Exhausted {
                kind: "recruitment attempt",
                ..
            }
        ))
    ));
    assert_eq!(fixture.state.recruitment().attempts().count(), 0);
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist after rejected autonomous attempt")
            .organization(),
        Some(fixture.source)
    );

    validate_state(&fixture.state).expect("rejected autonomous recruitment must leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_recruitment_requires_personnel_authority_and_delegated_policy() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let error = match validate_delegated_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        personnel_authority(&fixture, mandate),
        protection_draft(&fixture),
    ) {
        Ok(_) => panic!("default approval-required policy must block independent recruitment"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        RecruitmentError::IndependentRecruitmentNotDelegated {
            manager: fixture.recruiter,
            policy: ApprovalPolicy::RequireApproval,
        }
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_manager_prefers_the_stronger_relationship_not_a_random_prospect() {
    let registry = build_registry();
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));

    let stronger_candidate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Trusted Delegated Prospect".to_owned(),
            organization: Some(fixture.source),
            supervisor: Some(fixture.incumbent),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second delegated candidate should validate");
    validate_set_relationship(
        &fixture.state,
        stronger_candidate,
        fixture.recruiter,
        relationship(95, 95, 0, 80, 15, 0, 50),
    )
    .expect("strong delegated candidate relationship should validate")
    .commit(&mut fixture.state);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("relationship-ranked delegated recruitment should validate");
    assert_eq!(outcome.attempts.len(), 1);
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("delegated attempt should persist");
    assert_eq!(attempt.candidate(), stronger_candidate);
    validate_invariants(&fixture.state);
}

#[test]
fn approval_required_recruitment_executes_only_after_approval() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let request = validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: personnel_authority(&fixture, mandate),
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
            attention: crate::core::attention::AttentionClass::Exception,
            summary: "Personnel manager requests authority to recruit a rival associate."
                .to_owned(),
        },
    )
    .expect("approval-required recruitment proposal should validate")
    .commit(&mut fixture.state)
    .expect("approval request should commit");

    assert_eq!(
        fixture
            .state
            .decisions()
            .pending_for_recruitment_approval(fixture.target, fixture.candidate),
        Some(request.decision)
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should exist before approval")
            .organization(),
        Some(fixture.source)
    );

    let resolution = validate_resolve_decision(
        &fixture.registry,
        &fixture.state,
        request.decision,
        fixture.target,
        DecisionResponse::Approve,
    )
    .expect("fresh personnel approval should validate")
    .commit(&mut fixture.state)
    .expect("approved personnel action should commit atomically");
    let attempt = resolution
        .recruitment_attempt
        .expect("approval should execute one recruitment attempt");
    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("approved recruitment attempt should persist");
    assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
    assert_eq!(
        record.authority(),
        RecruitmentAuthority::ApprovedDecision {
            decision: request.decision,
            mandate,
            manager: fixture.recruiter,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
            mandate_version: 1,
            manager_version: 1,
            policy: ApprovalPolicy::RequireApproval,
            policy_source: RecruitmentPolicySource::Organization(fixture.target),
        }
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .get_attempt_for_approval_decision(request.decision)
            .map(|record| record.id()),
        Some(attempt)
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("accepted candidate should persist")
            .organization(),
        Some(fixture.target)
    );
    let decision = fixture
        .state
        .decisions()
        .get_decision(request.decision)
        .expect("approval decision should persist historically");
    assert_eq!(decision.status(), DecisionStatus::Resolved);
    assert_eq!(
        decision
            .resolution()
            .expect("resolved approval should persist resolution")
            .response(),
        DecisionResponse::Approve
    );
    validate_state(&fixture.state).expect("approved recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn rejected_recruitment_approval_records_no_attempt() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let request = validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: personnel_authority(&fixture, mandate),
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
            attention: crate::core::attention::AttentionClass::Exception,
            summary: "Personnel manager requests authority to approach a rival associate."
                .to_owned(),
        },
    )
    .expect("approval request should validate")
    .commit(&mut fixture.state)
    .expect("approval request should commit");
    let resolution = validate_resolve_decision(
        &fixture.registry,
        &fixture.state,
        request.decision,
        fixture.target,
        DecisionResponse::Reject,
    )
    .expect("rejection should validate even though no recruitment executes")
    .commit(&mut fixture.state)
    .expect("rejection should commit");

    assert_eq!(resolution.recruitment_attempt, None);
    assert!(
        fixture
            .state
            .recruitment()
            .get_attempt_for_approval_decision(request.decision)
            .is_none()
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("rejected candidate should remain in world")
            .organization(),
        Some(fixture.source)
    );
    validate_state(&fixture.state).expect("rejected approval state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn stale_recruitment_approval_cannot_execute_but_can_be_rejected() {
    let registry = build_registry();
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let request = validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: personnel_authority(&fixture, mandate),
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
            attention: crate::core::attention::AttentionClass::Exception,
            summary: "Personnel manager requests approval before making an approach.".to_owned(),
        },
    )
    .expect("approval request should validate")
    .commit(&mut fixture.state)
    .expect("approval request should commit");

    set_policy(
        &registry,
        &mut fixture.state,
        fixture.target,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
    )
    .expect("organization should be able to delegate recruitment later");
    let error = match validate_resolve_decision(
        &fixture.registry,
        &fixture.state,
        request.decision,
        fixture.target,
        DecisionResponse::Approve,
    ) {
        Ok(_) => panic!("stale approval must not execute under changed authority"),
        Err(error) => error,
    };
    assert_eq!(error, DecisionError::StaleRecruitmentApprovalAuthority);
    assert!(
        fixture
            .state
            .recruitment()
            .get_attempt_for_approval_decision(request.decision)
            .is_none()
    );

    validate_resolve_decision(
        &fixture.registry,
        &fixture.state,
        request.decision,
        fixture.target,
        DecisionResponse::Reject,
    )
    .expect("stale request should remain dismissible")
    .commit(&mut fixture.state)
    .expect("stale request rejection should commit");
    validate_state(&fixture.state).expect("dismissed stale approval state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn save_round_trip_preserves_pending_recruitment_approval() {
    let registry = build_registry();
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let request = validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: personnel_authority(&fixture, mandate),
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
            attention: crate::core::attention::AttentionClass::Exception,
            summary: "Personnel manager requests approval for a recruitment approach.".to_owned(),
        },
    )
    .expect("approval request should validate")
    .commit(&mut fixture.state)
    .expect("approval request should commit");

    let envelope = build_save(&registry, &fixture.state).expect("pending approval should save");
    let bytes = bincode::serialize(&envelope).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let mut restored = restore_save(&registry, decoded).expect("save should restore");
    let restored_decision = restored
        .decisions()
        .get_decision(request.decision)
        .expect("pending approval should survive save/load");
    assert_eq!(restored_decision.status(), DecisionStatus::Pending);
    match restored_decision.context() {
        DecisionContext::RecruitmentApproval(context) => {
            assert_eq!(context.candidate(), fixture.candidate);
            assert_eq!(context.recruiter(), fixture.recruiter);
        }
        DecisionContext::OperationPoliceArrival { .. } => {
            panic!("restored personnel approval changed context")
        }
    }
    let resolution = validate_resolve_decision(
        &registry,
        &restored,
        request.decision,
        fixture.target,
        DecisionResponse::Approve,
    )
    .expect("restored approval should remain executable")
    .commit(&mut restored)
    .expect("restored approval should commit");
    assert!(resolution.recruitment_attempt.is_some());
    validate_state(&restored).expect("restored approved recruitment state should validate");
    validate_invariants(&restored);
}

#[test]
fn delegated_recruitment_persists_exact_mandate_and_policy_authority() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::Delegated));
    let attempt = validate_delegated_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        personnel_authority(&fixture, mandate),
        protection_draft(&fixture),
    )
    .expect("delegated personnel recruitment should validate")
    .commit(&mut fixture.state)
    .expect("delegated personnel recruitment should commit");
    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("delegated recruitment should persist");
    assert_eq!(
        record.authority(),
        RecruitmentAuthority::Delegated {
            mandate,
            manager: fixture.recruiter,
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
            mandate_version: 1,
            manager_version: 1,
            policy: ApprovalPolicy::Delegated,
            policy_source: RecruitmentPolicySource::Mandate(mandate),
        }
    );
    assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
    validate_state(&fixture.state).expect("delegated recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_recruitment_token_rejects_organization_policy_change_without_mutation() {
    let registry = build_registry();
    let mut fixture = fixture();
    set_policy(
        &registry,
        &mut fixture.state,
        fixture.target,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
    )
    .expect("delegated organization policy should validate");
    let mandate = assign_personnel_mandate(&mut fixture, None);
    let token = validate_delegated_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        personnel_authority(&fixture, mandate),
        protection_draft(&fixture),
    )
    .expect("organization policy should initially authorize recruitment");
    set_policy(
        &registry,
        &mut fixture.state,
        fixture.target,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    )
    .expect("policy revision should validate");
    let error = token
        .commit(&mut fixture.state)
        .expect_err("stale delegated recruitment must not survive policy revocation");
    assert_eq!(error, RecruitmentError::StaleRecruitmentPolicy);
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .organization(),
        Some(fixture.source)
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn protection_offer_uses_only_candidate_known_legal_pressure_and_stales_when_knowledge_changes() {
    let mut fixture = fixture();
    let draft = protection_draft(&fixture);
    let token = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("recruitment should validate before candidate learns new information");
    let before = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("internal decision calculation should succeed");
    assert_eq!(before.context.factors.perceived_legal_pressure(), 0);

    let information = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Character(fixture.candidate),
            source_kind: InformationSourceKind::PoliceContact,
            topic: InformationTopic::PoliceActivity,
            source_entity: None,
            subject: EntityRef::Character(fixture.candidate),
            observed_at: fixture.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: "The candidate learned that detectives are actively asking about them."
                .to_owned(),
        },
    )
    .expect("candidate-held legal pressure information should validate")
    .commit(&mut fixture.state)
    .expect("candidate-held legal pressure information should commit");

    let error = token
        .commit(&mut fixture.state)
        .expect_err("new candidate knowledge must invalidate an older social decision");
    assert_eq!(
        error,
        RecruitmentError::StalePressureKnowledge {
            candidate: fixture.candidate,
        }
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );

    let after = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("fresh decision should incorporate candidate-held police pressure");
    assert_eq!(after.context.pressure_information, Some(information));
    assert_eq!(after.context.factors.perceived_legal_pressure(), 100);
    let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("fresh pressure-aware recruitment should validate")
        .commit(&mut fixture.state)
        .expect("fresh pressure-aware recruitment should commit");
    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("pressure-aware attempt should persist");
    assert_eq!(record.pressure_information(), Some(information));
    assert_eq!(record.factors().perceived_legal_pressure(), 100);
    validate_state(&fixture.state).expect("pressure-aware recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn expired_pressure_knowledge_neither_scores_nor_stales_a_validated_recruitment() {
    let mut fixture = fixture();
    let observed_at = fixture.state.now();
    let record_expired_pressure = |fixture: &mut Fixture, summary: &str| {
        validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(fixture.candidate),
                source_kind: InformationSourceKind::PoliceContact,
                topic: InformationTopic::PoliceActivity,
                source_entity: None,
                subject: EntityRef::Character(fixture.candidate),
                observed_at,
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: summary.to_owned(),
            },
        )
        .expect("expired pressure fixture information should validate")
        .commit(&mut fixture.state)
        .expect("expired pressure fixture information should commit")
    };
    let first = record_expired_pressure(&mut fixture, "Old police pressure");
    fixture.state.advance_clock(
        fixture
            .registry
            .recruitment()
            .perceived_legal_pressure_max_age(),
    );

    let draft = protection_draft(&fixture);
    let plan = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("expired knowledge should not block recruitment planning");
    assert_eq!(plan.context.pressure_information, None);
    assert_eq!(plan.context.factors.perceived_legal_pressure(), 0);
    assert!(
        !plan
            .dependencies
            .pressure_information_snapshot
            .contains(&first),
        "zero-value expired information must not remain a live decision dependency"
    );
    let token = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("recruitment with only expired pressure should validate");

    let second = record_expired_pressure(&mut fixture, "Another old police report");
    assert_ne!(first, second);
    token
        .commit(&mut fixture.state)
        .expect("newly recorded but already expired knowledge must not stale the decision");
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("expired-pressure recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn protection_offer_uses_drives_and_relationships_and_moves_accepted_candidate_atomically() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(15, 25, 20, 10, 20, 75, 0),
    )
    .expect("incumbent relationship should validate")
    .commit(&mut fixture.state);

    let plan = decide_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("recruitment should produce a causal plan");
    assert_eq!(plan.context.outcome, RecruitmentOutcome::Accepted);
    assert!(plan.context.factors.drive_alignment() >= 90);
    assert!(plan.context.factors.incumbent_resentment() >= 75);
    assert!(plan.context.margin >= 0);
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("fresh recruitment should validate")
    .commit(&mut fixture.state)
    .expect("accepted recruitment should commit atomically");

    let candidate = fixture
        .state
        .world()
        .get_character(fixture.candidate)
        .expect("candidate should persist");
    assert_eq!(candidate.organization(), Some(fixture.target));
    assert_eq!(candidate.supervisor(), Some(fixture.recruiter));
    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("attempt should persist");
    assert_eq!(record.previous_organization(), Some(fixture.source));
    assert_eq!(record.previous_supervisor(), Some(fixture.incumbent));
    assert_eq!(record.authority(), RecruitmentAuthority::ExecutiveApproval);
    assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
    let outcome_information = fixture
        .state
        .intelligence()
        .get_information(record.outcome_information())
        .expect("recruiting organization should retain personnel outcome information");
    assert_eq!(
        outcome_information.holder(),
        KnowledgeHolder::Organization(fixture.target)
    );
    assert_eq!(outcome_information.topic(), InformationTopic::Personnel);
    assert_eq!(
        outcome_information.source_entity(),
        Some(EntityRef::Character(fixture.recruiter))
    );
    assert_eq!(
        outcome_information.subject(),
        EntityRef::Character(fixture.candidate)
    );
    assert!(
        !fixture
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(fixture.source))
            .any(|information| information.id() == record.outcome_information())
    );
    let history = fixture
        .state
        .history()
        .get_event(
            record
                .history_event()
                .expect("acceptance should create history"),
        )
        .expect("recruitment history should persist");
    assert_eq!(history.kind(), HistoryEventKind::Recruitment);
    assert!(
        history
            .entities()
            .contains(&EntityRef::Character(fixture.candidate))
    );
    // This is a defection out of `fixture.source`, so campaign history must not leak the
    // hidden recruiting organization: the player finds the destination through surveillance,
    // not a global history read. The recruiter is omitted too — their membership would
    // resolve straight back to the destination organization.
    assert!(
        !history
            .entities()
            .contains(&EntityRef::Character(fixture.recruiter))
    );
    assert!(
        !history
            .entities()
            .contains(&EntityRef::Organization(fixture.target))
    );
    let target_name = fixture
        .state
        .world()
        .get_organization(fixture.target)
        .expect("target organization should persist")
        .name();
    assert!(!history.summary().contains(target_name));
    let recruiter_name = fixture
        .state
        .world()
        .get_character(fixture.recruiter)
        .expect("recruiter should persist")
        .name();
    assert!(!history.summary().contains(recruiter_name));
    let departure_report = fixture
        .state
        .reports()
        .get_report(
            record
                .member_report()
                .expect("accepted defection should link its departure report"),
        )
        .expect("linked departure report should persist");
    assert_eq!(departure_report.recipient(), fixture.source);
    assert_eq!(departure_report.kind(), ReportKind::AfterAction);
    assert_eq!(departure_report.entries().len(), 1);
    assert!(
        departure_report.entries()[0]
            .summary
            .contains("Frightened Associate left North Crew")
    );
    assert!(
        departure_report.entries()[0]
            .entities
            .contains(&EntityRef::Character(fixture.candidate))
    );
    assert!(
        !departure_report.entries()[0]
            .entities
            .contains(&EntityRef::Organization(fixture.target))
    );
    validate_state(&fixture.state).expect("accepted recruitment state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn recruitment_outcome_information_rejects_unauthored_persisted_summary() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(15, 25, 20, 10, 20, 75, 0),
    )
    .expect("incumbent relationship should validate")
    .commit(&mut fixture.state);
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("accepted recruitment should validate")
    .commit(&mut fixture.state)
    .expect("accepted recruitment should commit");
    let summary = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("attempt should persist")
        .outcome_information();
    let summary = fixture
        .state
        .intelligence()
        .get_information(summary)
        .expect("recruitment outcome information should persist")
        .summary()
        .to_owned();
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid recruitment outcome should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 1);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten recruitment outcome must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidRecruitmentAttempt {
                    attempt: invalid,
                }
            ) if invalid == attempt
        ),
        "expected invalid recruitment attempt, got {error:?}"
    );
}

#[test]
fn accepted_recruitment_history_rejects_unauthored_persisted_summary() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(15, 25, 20, 10, 20, 75, 0),
    )
    .expect("incumbent relationship should validate")
    .commit(&mut fixture.state);
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("accepted recruitment should validate")
    .commit(&mut fixture.state)
    .expect("accepted recruitment should commit");
    let history = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("attempt should persist")
        .history_event()
        .expect("accepted recruitment should persist history");
    let summary = fixture
        .state
        .history()
        .get_event(history)
        .expect("accepted recruitment history should persist")
        .summary()
        .to_owned();
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid recruitment history should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 1);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten recruitment history must fail the real load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidRecruitmentAttempt {
                    attempt: invalid,
                }
            ) if invalid == attempt
        ),
        "expected invalid recruitment attempt, got {error:?}"
    );
}

#[test]
fn later_reassignment_does_not_rewrite_accepted_recruitment_history() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(15, 25, 20, 10, 20, 75, 0),
    )
    .expect("incumbent relationship should validate")
    .commit(&mut fixture.state);
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("accepted recruitment should validate")
    .commit(&mut fixture.state)
    .expect("accepted recruitment should commit");
    let resulting_version = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("accepted attempt should persist")
        .resulting_candidate_version();
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .version(),
        resulting_version
    );

    validate_reassign_character(
        &fixture.state,
        fixture.candidate,
        Some(fixture.source),
        Some(fixture.incumbent),
    )
    .expect("a later canonical membership change should be allowed")
    .commit(&mut fixture.state)
    .expect("later membership change should commit");
    validate_state(&fixture.state)
        .expect("later membership must not invalidate accepted recruitment history");
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("accepted recruitment history must remain registry-valid after reassignment");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("reassigned accepted-recruitment state should save"),
    )
    .expect("reassigned accepted-recruitment state should restore");
    assert!(restored.recruitment().get_attempt(attempt).is_some());
    assert!(
        restored
            .world()
            .get_character(fixture.candidate)
            .expect("restored candidate should persist")
            .version()
            > resulting_version
    );
    validate_invariants(&restored);
}

#[test]
fn persisted_relationship_snapshots_keep_recruitment_history_valid_after_social_change() {
    let mut fixture = fixture();
    let recruiter_dimensions = relationship(55, 65, 10, 35, 15, 5, 10);
    let incumbent_dimensions = relationship(20, 30, 15, 10, 25, 70, 0);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        incumbent_dimensions,
    )
    .expect("incumbent relationship should validate")
    .commit(&mut fixture.state);
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("recruitment should validate")
    .commit(&mut fixture.state)
    .expect("recruitment should commit");

    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(5, 10, 80, 0, 0, 60, 0),
    )
    .expect("later recruiter relationship change should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(80, 80, 0, 60, 60, 5, 0),
    )
    .expect("later incumbent relationship change should validate")
    .commit(&mut fixture.state);

    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("recruitment attempt should persist");
    assert_eq!(
        record.recruiter_relationship().dimensions(),
        Some(recruiter_dimensions)
    );
    assert_eq!(
        record
            .incumbent_relationship()
            .expect("historical incumbent snapshot should persist")
            .dimensions(),
        Some(incumbent_dimensions)
    );
    assert_ne!(
        fixture
            .state
            .social()
            .get_relationship(fixture.candidate, fixture.recruiter)
            .expect("current recruiter relationship should exist")
            .dimensions(),
        recruiter_dimensions
    );
    validate_state(&fixture.state)
        .expect("later social changes must not invalidate recruitment history");
    validate_invariants(&fixture.state);
}

#[test]
fn strong_incumbent_attachment_can_produce_refusal_without_membership_mutation() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(95, 95, 10, 85, 90, 0, 0),
    )
    .expect("strong incumbent relationship should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(10, 20, 30, 5, 0, 0, 0),
    )
    .expect("weak recruiter relationship should validate")
    .commit(&mut fixture.state);
    let draft = RecruitmentDraft {
        approach: RecruitmentApproach::Advancement,
        ..protection_draft(&fixture)
    };
    let plan = decide_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("valid recruitment should still produce a refusal plan");
    assert_eq!(plan.context.outcome, RecruitmentOutcome::Refused);
    assert!(plan.context.factors.incumbent_attachment() >= 90);
    let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("refusal should validate")
        .commit(&mut fixture.state)
        .expect("refusal should persist without moving candidate");
    let candidate = fixture
        .state
        .world()
        .get_character(fixture.candidate)
        .expect("candidate should persist");
    assert_eq!(candidate.organization(), Some(fixture.source));
    assert_eq!(candidate.supervisor(), Some(fixture.incumbent));
    assert_eq!(
        fixture
            .state
            .recruitment()
            .get_attempt(attempt)
            .expect("refusal should persist")
            .history_event(),
        None
    );
    validate_invariants(&fixture.state);
}

#[test]
fn later_reassignment_does_not_rewrite_refused_recruitment_history() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(95, 95, 10, 85, 90, 0, 0),
    )
    .expect("strong incumbent relationship should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(10, 20, 30, 5, 0, 0, 0),
    )
    .expect("weak recruiter relationship should validate")
    .commit(&mut fixture.state);
    let draft = RecruitmentDraft {
        approach: RecruitmentApproach::Advancement,
        ..protection_draft(&fixture)
    };
    let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("refused recruitment should validate")
        .commit(&mut fixture.state)
        .expect("refused recruitment should persist");
    let resulting_version = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("refused attempt should persist")
        .resulting_candidate_version();
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .version(),
        resulting_version,
        "refusal itself performs no membership reassignment"
    );

    validate_reassign_character(&fixture.state, fixture.candidate, None, None)
        .expect("a later canonical departure should be allowed")
        .commit(&mut fixture.state)
        .expect("later departure should commit");
    validate_state(&fixture.state)
        .expect("later departure must not invalidate refused recruitment history");
    validate_state_against_registry(&fixture.registry, &fixture.state)
        .expect("refused recruitment history must remain registry-valid after departure");
    let restored = restore_save(
        &fixture.registry,
        build_save(&fixture.registry, &fixture.state)
            .expect("reassigned refused-recruitment state should save"),
    )
    .expect("reassigned refused-recruitment state should restore");
    assert!(restored.recruitment().get_attempt(attempt).is_some());
    validate_invariants(&restored);
}

#[test]
fn refused_poaching_approach_is_reported_to_the_candidates_organization() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(95, 95, 10, 85, 90, 0, 0),
    )
    .expect("strong incumbent relationship should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(10, 20, 30, 5, 0, 0, 0),
    )
    .expect("weak recruiter relationship should validate")
    .commit(&mut fixture.state);
    let draft = RecruitmentDraft {
        approach: RecruitmentApproach::Advancement,
        ..protection_draft(&fixture)
    };
    let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("refusal should validate")
        .commit(&mut fixture.state)
        .expect("refusal should persist without moving candidate");
    // A loyal member reports the outside pitch to their own leadership, so the organization
    // learns both that it happened and who made it — without any membership change.
    let record = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("refused recruitment attempt should persist");
    let approach_report = fixture
        .state
        .reports()
        .get_report(
            record
                .member_report()
                .expect("refused poaching should link its loyalty report"),
        )
        .expect("linked loyalty report should persist");
    assert_eq!(approach_report.recipient(), fixture.source);
    assert_eq!(approach_report.kind(), ReportKind::AfterAction);
    assert_eq!(approach_report.entries().len(), 1);
    assert_eq!(
        approach_report.entries()[0].attention,
        AttentionClass::Notable
    );
    let recruiter_name = fixture
        .state
        .world()
        .get_character(fixture.recruiter)
        .expect("recruiter should persist")
        .name()
        .to_owned();
    let target_name = fixture
        .state
        .world()
        .get_organization(fixture.target)
        .expect("target organization should persist")
        .name()
        .to_owned();
    let summary = &approach_report.entries()[0].summary;
    assert!(summary.contains("turned the approach down"));
    assert!(summary.contains(&recruiter_name));
    assert!(summary.contains(&target_name));
    let entities = &approach_report.entries()[0].entities;
    assert!(entities.contains(&EntityRef::Character(fixture.candidate)));
    assert!(entities.contains(&EntityRef::Character(fixture.recruiter)));
    assert!(entities.contains(&EntityRef::Organization(fixture.target)));
    assert!(entities.contains(&EntityRef::Organization(fixture.source)));
    validate_state(&fixture.state).expect("refused-approach state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn linked_recruitment_member_report_rejects_persisted_summary_rewrite() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(95, 95, 10, 85, 90, 0, 0),
    )
    .expect("strong incumbent relationship should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(10, 20, 30, 5, 0, 0, 0),
    )
    .expect("weak recruiter relationship should validate")
    .commit(&mut fixture.state);
    let draft = RecruitmentDraft {
        approach: RecruitmentApproach::Advancement,
        ..protection_draft(&fixture)
    };
    let attempt = validate_recruitment_attempt(&fixture.registry, &fixture.state, draft)
        .expect("refusal should validate")
        .commit(&mut fixture.state)
        .expect("refusal should commit");
    let report = fixture
        .state
        .recruitment()
        .get_attempt(attempt)
        .and_then(|record| record.member_report())
        .and_then(|report| fixture.state.reports().get_report(report))
        .expect("refused recruitment must persist its linked member report");
    let summary = report.entries()[0].summary.clone();
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("linked recruitment report should save before corruption");
    let corrupted = tamper_serialized_summary(envelope, &summary, 1);
    let error = restore_save(&fixture.registry, corrupted)
        .expect_err("rewritten linked recruitment report must fail the load boundary");
    assert!(
        matches!(
            error,
            LoadError::InvalidState(
                crate::core::invariants::StateValidationError::InvalidRecruitmentAttempt {
                    attempt: invalid,
                }
            ) if invalid == attempt
        ),
        "expected invalid recruitment attempt, got {error:?}"
    );
}

#[test]
fn recruitment_cooldown_blocks_spam_and_allows_a_later_social_reassessment() {
    let mut accepted_fixture = fixture();
    let draft = protection_draft(&accepted_fixture);
    let first =
        validate_recruitment_attempt(&accepted_fixture.registry, &accepted_fixture.state, draft)
            .expect("initial recruitment should validate")
            .commit(&mut accepted_fixture.state)
            .expect("initial recruitment should commit");
    let first_record = accepted_fixture
        .state
        .recruitment()
        .get_attempt(first)
        .expect("first recruitment should persist");
    let expected_next =
        first_record.occurred_at() + accepted_fixture.registry.recruitment().cooldown();
    assert_eq!(
        decide_recruitment_attempt(&accepted_fixture.registry, &accepted_fixture.state, draft)
            .expect_err("immediate retry must fail"),
        RecruitmentError::CandidateAlreadyMember {
            candidate: accepted_fixture.candidate,
            organization: accepted_fixture.target,
        }
    );

    let mut refusal_fixture = fixture();
    validate_set_relationship(
        &refusal_fixture.state,
        refusal_fixture.candidate,
        refusal_fixture.incumbent,
        relationship(100, 100, 0, 100, 100, 0, 0),
    )
    .expect("attachment relationship should validate")
    .commit(&mut refusal_fixture.state);
    let refusal_draft = RecruitmentDraft {
        approach: RecruitmentApproach::Advancement,
        ..protection_draft(&refusal_fixture)
    };
    validate_recruitment_attempt(
        &refusal_fixture.registry,
        &refusal_fixture.state,
        refusal_draft,
    )
    .expect("refusal should validate")
    .commit(&mut refusal_fixture.state)
    .expect("refusal should commit");
    let error = decide_recruitment_attempt(
        &refusal_fixture.registry,
        &refusal_fixture.state,
        refusal_draft,
    )
    .expect_err("refused candidate should not be rerolled immediately");
    assert_eq!(
        error,
        RecruitmentError::Cooldown {
            candidate: refusal_fixture.candidate,
            organization: refusal_fixture.target,
            next_eligible_at: expected_next,
        }
    );
    refusal_fixture
        .state
        .advance_clock(refusal_fixture.registry.recruitment().cooldown());
    decide_recruitment_attempt(
        &refusal_fixture.registry,
        &refusal_fixture.state,
        refusal_draft,
    )
    .expect("candidate should become approachable when cooldown expires");
    validate_invariants(&refusal_fixture.state);
}

#[test]
fn relationship_change_invalidates_validated_attempt_without_partial_mutation() {
    let mut fixture = fixture();
    let token = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("recruitment should initially validate");
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(5, 5, 80, 0, 0, 0, 0),
    )
    .expect("relationship mutation should validate")
    .commit(&mut fixture.state);
    let error = token
        .commit(&mut fixture.state)
        .expect_err("stale social decision must not commit");
    assert!(matches!(error, RecruitmentError::StaleRelationship { .. }));
    let candidate = fixture
        .state
        .world()
        .get_character(fixture.candidate)
        .expect("candidate should remain present");
    assert_eq!(candidate.organization(), Some(fixture.source));
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn canonical_world_dependencies_block_poaching_a_manager_with_direct_reports() {
    let mut fixture = fixture();
    let subordinate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Dependent Soldier".to_owned(),
            organization: Some(fixture.source),
            supervisor: Some(fixture.candidate),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("subordinate should validate");
    let error = decide_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect_err("manager must hand off direct reports before defecting");
    assert_eq!(
        error,
        RecruitmentError::World(WorldError::DirectReportAssignment {
            character: fixture.candidate,
            direct_report: subordinate,
        })
    );
    assert_eq!(
        fixture
            .state
            .recruitment()
            .attempts_for_candidate(fixture.candidate)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn save_round_trip_preserves_recruitment_history_and_drive_authorship() {
    let registry = build_registry();
    let mut fixture = fixture();
    let attempt = validate_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        protection_draft(&fixture),
    )
    .expect("recruitment should validate")
    .commit(&mut fixture.state)
    .expect("recruitment should commit");
    let envelope = build_save(&registry, &fixture.state).expect("state should save");
    let bytes = bincode::serialize(&envelope).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
    let restored = restore_save(&registry, decoded).expect("save should restore");
    let record = restored
        .recruitment()
        .get_attempt(attempt)
        .expect("recruitment attempt should survive save/load");
    assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
    assert_eq!(
        restored
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should survive save/load")
            .organization(),
        Some(fixture.target)
    );
    assert!(
        record
            .history_event()
            .and_then(|history| restored.history().get_event(history))
            .is_some()
    );
    validate_state(&restored).expect("restored recruitment state should validate");
    validate_invariants(&restored);
}

#[test]
fn require_approval_manager_autonomously_raises_and_leadership_resolves_its_own_request() {
    let registry = build_registry();
    let mut fixture = fixture();
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("autonomous approval should resolve atomically");
    // A non-player organization resolves its own queue in-pass. Leadership approves the
    // manager's proposal without consulting the candidate's future response, and this fixture's
    // actual pitch then succeeds under the ApprovedDecision authority.
    assert_eq!(outcome.approval_requests.len(), 1);
    assert_eq!(outcome.attempts.len(), 1);
    let decision = fixture
        .state
        .decisions()
        .get_decision(outcome.approval_requests[0])
        .expect("approval decision should persist");
    assert_eq!(decision.status(), DecisionStatus::Resolved);
    assert_eq!(
        decision
            .resolution()
            .expect("auto-resolved request carries a resolution")
            .response(),
        DecisionResponse::Approve
    );
    let record = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("approved autonomous attempt should persist");
    assert_eq!(record.outcome(), RecruitmentOutcome::Accepted);
    assert!(matches!(
        record.authority(),
        RecruitmentAuthority::ApprovedDecision {
            policy: ApprovalPolicy::RequireApproval,
            ..
        }
    ));
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .organization(),
        Some(fixture.target),
        "the approved pitch moves the member"
    );
    validate_state(&fixture.state).expect("autonomous approval state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn npc_approval_does_not_oracle_candidate_refusal() {
    let registry = build_registry();
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(95, 95, 10, 85, 90, 0, 0),
    )
    .expect("strong incumbent relationship should validate")
    .commit(&mut fixture.state);
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.recruiter,
        relationship(10, 20, 30, 5, 0, 0, 0),
    )
    .expect("weak recruiter relationship should validate")
    .commit(&mut fixture.state);

    let autonomous_draft = RecruitmentDraft {
        target_organization: fixture.target,
        recruiter: fixture.recruiter,
        candidate: fixture.candidate,
        approach: RecruitmentApproach::PersonalAppeal,
    };
    let plan = decide_recruitment_attempt(&registry, &fixture.state, autonomous_draft)
        .expect("the manager may still make a weak recruitment pitch");
    assert_eq!(plan.context.outcome, RecruitmentOutcome::Refused);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("NPC approval and refused pitch should resolve atomically");
    assert_eq!(outcome.approval_requests.len(), 1);
    assert_eq!(outcome.attempts.len(), 1);
    let decision = fixture
        .state
        .decisions()
        .get_decision(outcome.approval_requests[0])
        .expect("NPC approval should persist");
    assert_eq!(decision.status(), DecisionStatus::Resolved);
    assert_eq!(
        decision
            .resolution()
            .expect("NPC approval should carry a resolution")
            .response(),
        DecisionResponse::Approve
    );
    let attempt = fixture
        .state
        .recruitment()
        .get_attempt(outcome.attempts[0])
        .expect("the approved pitch should persist even when refused");
    assert_eq!(attempt.outcome(), RecruitmentOutcome::Refused);
    assert!(matches!(
        attempt.authority(),
        RecruitmentAuthority::ApprovedDecision {
            policy: ApprovalPolicy::RequireApproval,
            ..
        }
    ));
    let candidate = fixture
        .state
        .world()
        .get_character(fixture.candidate)
        .expect("refusing candidate should persist");
    assert_eq!(candidate.organization(), Some(fixture.source));
    assert_eq!(candidate.supervisor(), Some(fixture.incumbent));

    // The approval and refused pitch are historical after this point. Revoking their authority
    // releases the manager, and a later organization change must not rewrite either record's
    // authority snapshot into an invalid present-day membership requirement.
    validate_revoke_mandate(&fixture.state, mandate)
        .expect("resolved recruitment work should leave the mandate revocable")
        .commit(&mut fixture.state)
        .expect("mandate revocation should commit");
    validate_reassign_character(
        &fixture.state,
        fixture.recruiter,
        Some(fixture.source),
        None,
    )
    .expect("revoked recruitment authority should release the former manager")
    .commit(&mut fixture.state)
    .expect("former recruiting manager should transfer");

    validate_state(&fixture.state).expect("refused approved pitch should remain valid");
    let restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state)
            .expect("resolved approval history should save after manager transfer"),
    )
    .expect("resolved approval history should restore after manager transfer");
    assert_eq!(
        restored
            .decisions()
            .get_decision(outcome.approval_requests[0])
            .expect("resolved approval should survive restore")
            .status(),
        DecisionStatus::Resolved
    );
    assert_eq!(
        restored
            .recruitment()
            .get_attempt(outcome.attempts[0])
            .expect("refused approved attempt should survive restore")
            .outcome(),
        RecruitmentOutcome::Refused
    );
    validate_invariants(&restored);
}

#[test]
fn player_organization_approval_requests_wait_for_the_player() {
    let registry = build_registry();
    let mut fixture = fixture();
    // The recruiting organization is the player's: its manager must raise a durable
    // request and wait, never resolve it autonomously.
    fixture.state.set_player_organization(fixture.target);
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("player approval request should validate");
    assert!(outcome.attempts.is_empty());
    assert_eq!(outcome.approval_requests.len(), 1);
    assert_eq!(
        fixture
            .state
            .decisions()
            .pending_for_recruitment_approval(fixture.target, fixture.candidate),
        Some(outcome.approval_requests[0]),
        "the player's own queue stays pending"
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(fixture.candidate)
            .expect("candidate should persist")
            .organization(),
        Some(fixture.source)
    );
    validate_state(&fixture.state).expect("pending request state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn approval_required_manager_prefers_the_stronger_relationship_not_the_lower_character_id() {
    let registry = build_registry();
    let mut fixture = fixture();
    fixture.state.set_player_organization(fixture.target);
    assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));

    let stronger_candidate = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Trusted Prospect".to_owned(),
            organization: Some(fixture.source),
            supervisor: Some(fixture.incumbent),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second candidate should validate");
    assert!(
        stronger_candidate > fixture.candidate,
        "fixture must prove that a later-created candidate can outrank an earlier ID"
    );
    validate_set_relationship(
        &fixture.state,
        stronger_candidate,
        fixture.recruiter,
        relationship(95, 95, 0, 80, 15, 0, 50),
    )
    .expect("strong candidate relationship should validate")
    .commit(&mut fixture.state);

    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let outcome = apply_due_autonomous_recruitment(&registry, &mut fixture.state)
        .expect("relationship-ranked approval request should validate");
    assert_eq!(outcome.approval_requests.len(), 1);
    let request = fixture
        .state
        .decisions()
        .get_decision(outcome.approval_requests[0])
        .expect("relationship-ranked approval request should persist");
    match request.context() {
        DecisionContext::RecruitmentApproval(context) => {
            assert_eq!(context.candidate(), stronger_candidate);
        }
        DecisionContext::OperationPoliceArrival { .. } => {
            panic!("personnel pass produced the wrong decision context")
        }
    }
    assert_eq!(request.status(), DecisionStatus::Pending);
    validate_state(&fixture.state).expect("relationship-ranked approval state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn delegated_channel_cannot_race_a_pending_recruitment_approval() {
    use crate::delegation::delegation_system::{MandateRevisionDraft, validate_revise_mandate};

    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let request = validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: personnel_authority(&fixture, mandate),
            target_organization: fixture.target,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::Protection,
            attention: crate::core::attention::AttentionClass::Exception,
            summary: "Personnel manager requests authority to recruit a rival associate."
                .to_owned(),
        },
    )
    .expect("approval-required recruitment proposal should validate")
    .commit(&mut fixture.state)
    .expect("approval request should commit");

    // Leadership loosens the standing order while the request is still open: the manager
    // may now recruit autonomously in general, but this candidate's pending approval keeps
    // the pair exclusive across channels.
    validate_revise_mandate(
        &fixture.state,
        mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::Delegated),
            )]),
            budget: None,
        },
    )
    .expect("mandate revision should validate")
    .commit(&mut fixture.state)
    .expect("mandate revision should commit");

    let error = match validate_delegated_recruitment_attempt(
        &fixture.registry,
        &fixture.state,
        personnel_authority(&fixture, mandate),
        protection_draft(&fixture),
    ) {
        Err(error) => error,
        Ok(_) => panic!("a pending approval must block the delegated channel"),
    };
    assert_eq!(
        error,
        RecruitmentError::PendingRecruitmentApproval {
            decision: request.decision
        }
    );
    validate_state(&fixture.state).expect("rejected race should preserve valid state");
}
