//! Recruitment requests must remain actionable across changes to personnel and authority.

use super::*;
use crate::world::character_life::{CivicPersonError, validate_set_civic_occupation};
use crate::world::world_system::{
    CharacterAutonomyError, insert_neighborhood, validate_set_character_autonomy,
    validate_set_character_supervisor,
};
use crate::world::{
    IndependentVocation, NeighborhoodDraft, NeighborhoodEconomyProfile,
    NeighborhoodInstitutionProfile, NeighborhoodProfile,
};

fn home(state: &mut AppState) -> crate::core::id::NeighborhoodId {
    insert_neighborhood(
        state,
        NeighborhoodDraft {
            name: "Manager's ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(50),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(50),
                },
            },
        },
    )
    .unwrap()
}

fn approval_draft(
    fixture: &Fixture,
    mandate: crate::core::id::MandateId,
) -> RecruitmentApprovalRequestDraft {
    RecruitmentApprovalRequestDraft {
        authority: personnel_authority(fixture, mandate),
        target_organization: fixture.target,
        recruiter: fixture.recruiter,
        candidate: fixture.candidate,
        approach: RecruitmentApproach::Protection,
        attention: AttentionClass::Exception,
        summary: "Personnel manager requests approval for a protection pitch.".to_owned(),
    }
}

#[test]
fn manager_revision_cancels_obsolete_recruitment_approval() {
    for revision in 0..3 {
        let mut fixture = fixture();
        let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
        let superior = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "New Executive".to_owned(),
                organization: Some(fixture.target),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .unwrap();
        let home = home(&mut fixture.state);
        let draft = approval_draft(&fixture, mandate);
        // Another manager in the same organization has an independent pending request.
        // Character revision must not cancel an unrelated authority's route.
        let other_mandate = validate_assign_mandate(
            &fixture.state,
            MandateDraft {
                organization: fixture.target,
                manager: superior,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Personnel,
                )]),
                standing_orders: BTreeMap::from([(
                    PolicyKind::IndependentRecruitment,
                    PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
                )]),
                budget: None,
            },
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
        let other_candidate = insert_character(
            &mut fixture.state,
            CharacterDraft {
                name: "Other prospect".to_owned(),
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
            &fixture.state,
            other_candidate,
            superior,
            relationship(70, 70, 0, 70, 0, 0, 0),
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
        let other = validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: MandateAuthority {
                    mandate: other_mandate,
                    manager: superior,
                    scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                },
                recruiter: superior,
                candidate: other_candidate,
                ..draft.clone()
            },
        )
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap()
        .decision;
        let decision =
            validate_request_recruitment_approval(&fixture.registry, &fixture.state, draft.clone())
                .unwrap()
                .commit(&mut fixture.state)
                .unwrap()
                .decision;
        let before = build_save(&fixture.registry, &fixture.state).unwrap();
        let mut restored = restore_save(&fixture.registry, before).unwrap();
        for state in [&mut fixture.state, &mut restored] {
            match revision {
                0 => validate_set_character_autonomy(
                    state,
                    fixture.target,
                    fixture.recruiter,
                    AutonomyLevel::Delegated,
                )
                .unwrap()
                .commit(state)
                .unwrap(),
                1 => validate_set_character_supervisor(state, fixture.recruiter, Some(superior))
                    .unwrap()
                    .commit(state)
                    .unwrap(),
                2 => validate_set_civic_occupation(
                    state,
                    fixture.recruiter,
                    home,
                    IndependentVocation::Resident,
                )
                .unwrap()
                .commit(state)
                .unwrap(),
                _ => unreachable!(),
            }
            let cancelled = state.decisions().get_decision(decision).unwrap();
            assert_eq!(cancelled.status(), DecisionStatus::Cancelled);
            assert_eq!(
                cancelled.cancellation().unwrap().reason(),
                DecisionCancellationReason::RecruitmentAuthorityChanged(mandate)
            );
            assert_eq!(
                cancelled.cancellation().unwrap().cancelled_at(),
                state.now()
            );
            assert_eq!(
                state.delegation().get_mandate(mandate).unwrap().version(),
                1,
                "manager revision must not manufacture a mandate revision"
            );
            assert_eq!(
                state
                    .decisions()
                    .pending_for_recruitment_approval(fixture.target, other_candidate),
                Some(other)
            );
            assert_eq!(state.decisions().get_decision(other).unwrap().version(), 1);
            assert!(
                state
                    .decisions()
                    .pending_for_recruitment_approval(fixture.target, fixture.candidate)
                    .is_none()
            );
            validate_state(state).unwrap();
            let fresh =
                validate_request_recruitment_approval(&fixture.registry, state, draft.clone())
                    .unwrap()
                    .commit(state)
                    .unwrap()
                    .decision;
            assert_ne!(fresh, decision);
            validate_resolve_decision(
                &fixture.registry,
                state,
                fresh,
                fixture.target,
                DecisionResponse::Approve,
            )
            .unwrap()
            .commit(state)
            .unwrap();
            *state = restore_save(
                &fixture.registry,
                build_save(&fixture.registry, state).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            bincode::serialize(&fixture.state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
    }
}

#[test]
fn held_manager_revisions_reject_new_approval_set_without_mutation() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    let superior = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Executive".to_owned(),
            organization: Some(fixture.target),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let home = home(&mut fixture.state);
    let autonomy = validate_set_character_autonomy(
        &fixture.state,
        fixture.target,
        fixture.recruiter,
        AutonomyLevel::Delegated,
    )
    .unwrap();
    let supervisor =
        validate_set_character_supervisor(&fixture.state, fixture.recruiter, Some(superior))
            .unwrap();
    let occupation = validate_set_civic_occupation(
        &fixture.state,
        fixture.recruiter,
        home,
        IndependentVocation::Resident,
    )
    .unwrap();
    let draft = approval_draft(&fixture, mandate);
    let decision = validate_request_recruitment_approval(&fixture.registry, &fixture.state, draft)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap()
        .decision;
    let before = bincode::serialize(&fixture.state).unwrap();
    let expected = WorldError::RecruitmentApprovalSetChanged {
        character: fixture.recruiter,
    };
    assert!(
        matches!(autonomy.commit(&mut fixture.state), Err(CharacterAutonomyError::World(error)) if error == expected)
    );
    assert_eq!(supervisor.commit(&mut fixture.state).err(), Some(expected));
    assert_eq!(
        occupation.commit(&mut fixture.state).err(),
        Some(CivicPersonError::RecruitmentApprovalSetChanged(
            fixture.recruiter
        ))
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    assert_eq!(
        fixture
            .state
            .decisions()
            .get_decision(decision)
            .unwrap()
            .status(),
        DecisionStatus::Pending
    );
    validate_state(&fixture.state).unwrap();
}

#[test]
fn refused_proposal_rejects_new_candidate_responsibility_before_request_commit() {
    let mut fixture = fixture();
    let mandate = assign_personnel_mandate(&mut fixture, Some(ApprovalPolicy::RequireApproval));
    validate_set_relationship(
        &fixture.state,
        fixture.candidate,
        fixture.incumbent,
        relationship(100, 100, 0, 100, 100, 0, 0),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let mut draft = approval_draft(&fixture, mandate);
    draft.approach = RecruitmentApproach::Advancement;
    assert_eq!(
        decide_recruitment_attempt(
            &fixture.registry,
            &fixture.state,
            RecruitmentDraft {
                approach: draft.approach,
                ..protection_draft(&fixture)
            }
        )
        .unwrap()
        .context
        .outcome,
        RecruitmentOutcome::Refused
    );
    let held =
        validate_request_recruitment_approval(&fixture.registry, &fixture.state, draft).unwrap();
    let report = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "New Direct Report".to_owned(),
            organization: Some(fixture.source),
            supervisor: Some(fixture.candidate),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let before = bincode::serialize(&fixture.state).unwrap();
    assert_eq!(
        held.commit(&mut fixture.state).err(),
        Some(
            crate::decisions::decision_system::DecisionError::Recruitment(RecruitmentError::World(
                WorldError::DirectReportAssignment {
                    character: fixture.candidate,
                    direct_report: report
                }
            ))
        )
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
}
