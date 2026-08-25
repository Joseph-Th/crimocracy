//! Focused tests for `investigation_system` staffing, transitions, and cold-case decay.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save};
use crate::core::time::SimDuration;
use crate::legal::investigation_work_execution::{
    decide_investigation_work_resolution, validate_investigation_work_resolution_plan,
    validate_schedule_investigation_work, InvestigationWorkRandomness,
};
use crate::legal::{
    Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength, InvestigationWorkDraft,
    InvestigationWorkFocus, InvestigationWorkKind,
};
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character, WorldError,
};
use crate::world::{
    AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn insert_test_investigator(
    state: &mut AppState,
    organization: OrganizationId,
    name: &str,
    skill: u8,
) -> CharacterId {
    insert_character(
        state,
        CharacterDraft {
            name: name.to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(skill))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("investigator fixture should validate")
}

#[test]
fn incident_intake_cannot_forge_informant_statement() {
    let registry = build_registry();
    let mut state = AppState::new(0x14F0_5EED);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Intake Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Intake Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");

    let error = match validate_incident_intake(
        &state,
        IncidentIntakeDraft {
            owner: police,
            title: "Forged statement inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Organization(criminal),
                origin: None,
                kind: EvidenceKind::InformantStatement,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            }],
            origin: None,
            notified_organizations: BTreeSet::new(),
            witness: None,
        },
    ) {
        Ok(_) => panic!("incident intake must reject informant statements"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        InvestigationError::InformantStatementRequiresDisclosure
    );
    assert_eq!(
        state
            .legal()
            .evidence_of_kind(EvidenceKind::InformantStatement)
            .count(),
        0
    );
    assert!(state.legal().investigations().next().is_none());
    validate_invariants(&state);
}

#[test]
fn incident_intake_resumes_a_matching_suspended_shelf_instead_of_opening_a_parallel_case() {
    let registry = build_registry();
    let mut state = AppState::new(0x0E51_4E2F);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Shelf Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Shelf Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Shelf Crew Leader".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(60))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let scheduled_for = state.now() + SimDuration::ONE_MINUTE;
    let origin = crate::operations::operation_system::validate_authorize_operation(
        &registry,
        &mut state,
        crate::operations::OperationDraft {
            title: "Origin surveillance".to_owned(),
            kind: crate::operations::OperationKind::Surveillance,
            responsible_organization: criminal,
            leader,
            objective: crate::operations::OperationObjective::GatherInformation {
                target: EntityRef::Organization(criminal),
            },
            approach: crate::operations::OperationApproach::Covert,
            roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for,
        },
    )
    .expect("origin operation should validate")
    .commit(&mut state)
    .expect("origin operation should commit");

    let make_draft = |title: &str, now: SimTime| IncidentIntakeDraft {
        owner: police,
        title: title.to_owned(),
        subjects: BTreeSet::from([EntityRef::Operation(origin)]),
        evidence: vec![crate::legal::IncidentEvidenceDraft {
            subject: EntityRef::Operation(origin),
            origin: Some(EntityRef::Operation(origin)),
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: now,
        }],
        origin: Some(EntityRef::Operation(origin)),
        notified_organizations: BTreeSet::from([criminal]),
        witness: None,
    };

    let first = validate_incident_intake(&state, make_draft("First exposure inquiry", state.now()))
        .expect("first intake should validate")
        .commit(&mut state)
        .expect("first intake should commit");
    assert!(!first.resumed_shelf);

    validate_transition_investigation(
        &state,
        first.investigation,
        InvestigationTransition::Suspend,
    )
    .expect("shelving should validate")
    .commit(&mut state)
    .expect("shelving should commit");

    // The second exposure shares the shelf's subject matter, so it continues the existing
    // file: same case id, fresh evidence folded into it, no parallel case opened.
    let second =
        validate_incident_intake(&state, make_draft("Second exposure inquiry", state.now()))
            .expect("second intake should validate")
            .commit(&mut state)
            .expect("second intake should resume the shelf");
    assert_eq!(second.investigation, first.investigation);
    assert!(second.resumed_shelf);
    assert_eq!(
        state
            .legal()
            .get_investigation(first.investigation)
            .expect("resumed shelf should exist")
            .evidence()
            .len(),
        2,
        "the shelf carries its original and the new exposure evidence"
    );
    assert!(state
        .legal()
        .active_investigations()
        .any(|case| case.id() == first.investigation));
    assert_eq!(state.legal().investigations_for_owner(police).count(), 1);

    // An unrelated subject never resurrects the shelf: a distinct originated matter opens
    // its own case.
    let unrelated = validate_incident_intake(
        &state,
        IncidentIntakeDraft {
            owner: police,
            title: "Unrelated inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Organization(criminal),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            }],
            origin: None,
            notified_organizations: BTreeSet::new(),
            witness: None,
        },
    )
    .expect("unrelated intake should validate")
    .commit(&mut state)
    .expect("unrelated intake should open a fresh case");
    assert!(!unrelated.resumed_shelf);
    assert_ne!(unrelated.investigation, first.investigation);
    validate_state(&state).expect("resumed shelf state should remain structurally valid");
    validate_state_against_registry(&registry, &state)
        .expect("resumed shelf state should match authored state");
    validate_invariants(&state);
}

#[test]
fn autonomous_staffing_assigns_best_available_detective_and_respects_active_case_capacity() {
    let registry = build_registry();
    let mut state = AppState::new(0x57AF_F193);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Staffing Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Staffing Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let junior = insert_test_investigator(&mut state, police, "Junior", 70);
    let senior = insert_test_investigator(&mut state, police, "Senior", 92);
    let first = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "First autonomous staffing inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("first case should validate")
    .commit(&mut state)
    .expect("first case should commit");

    state = restore_save(
        &registry,
        build_save(&registry, &state).expect("unstaffed case state should save"),
    )
    .expect("unstaffed case index should survive save restoration");

    let staffed = apply_autonomous_investigator_staffing(&mut state)
        .expect("available detectives should staff the first case");
    assert_eq!(staffed, vec![(first, senior)]);
    assert_eq!(
        state
            .legal()
            .get_investigation(first)
            .expect("first case should persist")
            .lead_investigator(),
        Some(senior)
    );

    let second = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Second autonomous staffing inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("second case should validate")
    .commit(&mut state)
    .expect("second case should commit");
    let staffed = apply_autonomous_investigator_staffing(&mut state)
        .expect("remaining detective should staff the second case");
    assert_eq!(staffed, vec![(second, junior)]);
    assert_eq!(
        state
            .legal()
            .get_investigation(second)
            .expect("second case should persist")
            .lead_investigator(),
        Some(junior)
    );
    assert!(apply_autonomous_investigator_staffing(&mut state)
        .expect("already staffed cases should be a no-op")
        .is_empty());
    validate_state(&state).expect("autonomous staffing state should validate");
    validate_invariants(&state);
}

#[test]
fn investigation_suspend_resume_is_versioned_persistent_and_disables_active_mutation() {
    let registry = build_registry();
    let mut state = AppState::new(0x5A5E_1931);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Lifecycle Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Lifecycle Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let detective = insert_test_investigator(&mut state, police, "Harlan", 82);
    let second_detective = insert_test_investigator(&mut state, police, "Meyer", 74);
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Suspended conspiracy inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    validate_assign_investigator(&state, investigation, detective)
        .expect("lead assignment should validate")
        .commit(&mut state)
        .expect("lead assignment should commit");

    let stale_suspend =
        validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
            .expect("suspension should initially validate");
    validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Organization(criminal),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    )
    .expect("case mutation should validate before suspension")
    .commit(&mut state)
    .expect("case mutation should commit");
    assert!(matches!(
        stale_suspend.commit(&mut state),
        Err(InvestigationError::StaleInvestigation { .. })
    ));

    validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
        .expect("fresh suspension should validate")
        .commit(&mut state)
        .expect("fresh suspension should commit");
    assert_eq!(
        state
            .legal()
            .get_investigation(investigation)
            .expect("investigation should exist")
            .status(),
        InvestigationStatus::Suspended
    );

    let evidence_error = match validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Organization(criminal),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    ) {
        Ok(_) => panic!("suspended investigation must reject new evidence"),
        Err(error) => error,
    };
    assert_eq!(evidence_error, InvestigationError::InactiveInvestigation);
    let staffing_error = validate_assign_investigator(&state, investigation, second_detective)
        .expect_err("suspended investigation must reject new staffing");
    assert_eq!(staffing_error, InvestigationError::InactiveInvestigation);

    let mut restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("suspended investigation should save"),
    )
    .expect("suspended investigation should restore");
    assert_eq!(
        restored
            .legal()
            .get_investigation(investigation)
            .expect("restored investigation should exist")
            .status(),
        InvestigationStatus::Suspended
    );
    validate_transition_investigation(&restored, investigation, InvestigationTransition::Resume)
        .expect("valid retained staffing should permit resume")
        .commit(&mut restored)
        .expect("resume should commit");
    assert_eq!(
        restored
            .legal()
            .get_investigation(investigation)
            .expect("resumed investigation should exist")
            .status(),
        InvestigationStatus::Active
    );
    validate_state(&restored).expect("resumed investigation should be structurally valid");
    validate_state_against_registry(&registry, &restored)
        .expect("resumed investigation should match authored state");
    validate_invariants(&restored);
}

#[test]
fn scheduled_detective_work_blocks_case_transition_until_resolution_then_close_is_terminal() {
    let registry = build_registry();
    let mut state = AppState::new(0xC105_E193);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Closure Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let other_police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Closure Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let detective = insert_test_investigator(&mut state, police, "Doyle", 90);
    let first = insert_character(
        &mut state,
        CharacterDraft {
            name: "First Subject".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("first subject should validate");
    let middle = insert_character(
        &mut state,
        CharacterDraft {
            name: "Middle Subject".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("middle subject should validate");
    let target = insert_character(
        &mut state,
        CharacterDraft {
            name: "Target Subject".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("target subject should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Pattern closure inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(first)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    validate_assign_investigator(&state, investigation, detective)
        .expect("lead assignment should validate")
        .commit(&mut state)
        .expect("lead assignment should commit");

    for (subject, origin) in [
        (EntityRef::Character(middle), EntityRef::Character(first)),
        (EntityRef::Character(target), EntityRef::Character(middle)),
    ] {
        validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject,
                origin: Some(origin),
                kind: if subject == EntityRef::Character(middle) {
                    // Reviewable so the scheduled evidence review below can focus it.
                    EvidenceKind::Fingerprint
                } else {
                    EvidenceKind::KnownAssociation
                },
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("path evidence should validate")
        .commit(&mut state)
        .expect("path evidence should commit");
    }
    let reviewable = state
        .legal
        .evidence_of_kind(EvidenceKind::Fingerprint)
        .next()
        .expect("fingerprint evidence should exist")
        .id();
    let work = validate_schedule_investigation_work(
        &registry,
        &state,
        InvestigationWorkDraft {
            investigation,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(reviewable),
        },
    )
    .expect("evidence review should validate")
    .commit(&mut state)
    .expect("pattern analysis should schedule");

    for transition in [
        InvestigationTransition::Suspend,
        InvestigationTransition::Close,
    ] {
        assert_eq!(
            validate_transition_investigation(&state, investigation, transition)
                .expect_err("scheduled work must block case lifecycle transition"),
            InvestigationError::ScheduledWorkBlocksTransition {
                investigation,
                work,
            }
        );
    }

    state.advance_clock(SimDuration::from_minutes(360));
    let plan = decide_investigation_work_resolution(
        &registry,
        &state,
        work,
        InvestigationWorkRandomness::new(0),
    )
    .expect("due work should resolve");
    validate_investigation_work_resolution_plan(&registry, &state, plan)
        .expect("fresh work plan should validate")
        .commit(&mut state)
        .expect("work resolution should commit");
    validate_transition_investigation(&state, investigation, InvestigationTransition::Close)
        .expect("completed work should permit case closure")
        .commit(&mut state)
        .expect("case closure should commit");
    assert_eq!(
        state
            .legal()
            .get_investigation(investigation)
            .expect("closed case should exist")
            .status(),
        InvestigationStatus::Closed
    );

    validate_reassign_character(&state, detective, Some(other_police), None)
        .expect("closed historical case must not lock investigator membership")
        .commit(&mut state)
        .expect("detective transfer after closure should commit");
    for transition in [
        InvestigationTransition::Suspend,
        InvestigationTransition::Resume,
        InvestigationTransition::Close,
    ] {
        assert_eq!(
            validate_transition_investigation(&state, investigation, transition)
                .expect_err("closed case must be terminal"),
            InvestigationError::InvalidInvestigationTransition {
                status: InvestigationStatus::Closed,
                transition,
            }
        );
    }
    validate_state(&state).expect("closed case should remain structurally valid");
    validate_state_against_registry(&registry, &state)
        .expect("closed case history should remain registry-valid");
    validate_invariants(&state);
}

#[test]
fn suspending_a_case_releases_its_investigator_and_resume_restafs_from_the_free_pool() {
    let registry = build_registry();
    let mut state = AppState::new(0x5A57_AFF1);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Original Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let other_police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Transferred Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Resume Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let detective = insert_test_investigator(&mut state, police, "Reed", 80);
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Retained staffing inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    validate_assign_investigator(&state, investigation, detective)
        .expect("lead assignment should validate")
        .commit(&mut state)
        .expect("lead assignment should commit");

    validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
        .expect("suspension should validate")
        .commit(&mut state)
        .expect("suspension should commit");

    // Shelving releases institutional attention: the case holds no investigators, so its
    // former lead is free to transfer while the shelf sits cold.
    assert_eq!(
        state
            .legal()
            .get_investigation(investigation)
            .expect("suspended case should exist")
            .assigned_investigators()
            .len(),
        0
    );
    validate_reassign_character(&state, detective, Some(other_police), None)
        .expect("released detective should be free to transfer")
        .commit(&mut state)
        .expect("detective transfer should commit");

    validate_transition_investigation(&state, investigation, InvestigationTransition::Resume)
        .expect("a released shelf always resumes; staffing re-applies later")
        .commit(&mut state)
        .expect("resume should commit");
    let resumed = state
        .legal()
        .get_investigation(investigation)
        .expect("resumed case should exist");
    assert_eq!(resumed.status(), InvestigationStatus::Active);
    assert!(resumed.lead_investigator().is_none());
    // The resumed case is back in the unstaffed index, so autonomous staffing will fill it.
    assert!(state
        .legal()
        .active_investigations_without_lead()
        .any(|case| case == investigation));
    validate_state(&state).expect("resumed case should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn suspending_a_case_frees_its_investigator_for_other_casework() {
    // Shelving releases institutional attention: the case's lead can take another active
    // case while the shelf sits, and resumption does not claw the detective back — fresh
    // staffing fills the resumed case instead.
    let registry = build_registry();
    let mut state = AppState::new(0x51E5_1A7E);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Central Detectives".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let detective = insert_test_investigator(&mut state, police, "Vera", 80);
    let shelved = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Shelved ledger inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(police)]),
        },
    )
    .expect("shelved investigation should validate")
    .commit(&mut state)
    .expect("shelved investigation should commit");
    validate_assign_investigator(&state, shelved, detective)
        .expect("lead assignment should validate")
        .commit(&mut state)
        .expect("lead assignment should commit");
    validate_transition_investigation(&state, shelved, InvestigationTransition::Suspend)
        .expect("suspension should validate")
        .commit(&mut state)
        .expect("suspension should commit");

    let replacement = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Active harbor inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(police)]),
        },
    )
    .expect("active investigation should validate")
    .commit(&mut state)
    .expect("active investigation should commit");
    validate_assign_investigator(&state, replacement, detective)
        .expect("a shelved case must not hold its investigator back")
        .commit(&mut state)
        .expect("second lead assignment should commit");

    validate_transition_investigation(&state, shelved, InvestigationTransition::Resume)
        .expect("resume re-engages the case without retained staffing")
        .commit(&mut state)
        .expect("resume should commit");
    assert!(state
        .legal()
        .get_investigation(shelved)
        .expect("resumed case should exist")
        .lead_investigator()
        .is_none());
    // The one-active-case rule still binds live assignments: promoting the same detective
    // again onto their current case's rival fails at capacity.
    let third = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Third inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(police)]),
        },
    )
    .expect("third investigation should validate")
    .commit(&mut state)
    .expect("third investigation should commit");
    assert_eq!(
        validate_assign_investigator(&state, third, detective)
            .expect_err("an investigator already leading another active case cannot take a third"),
        InvestigationError::InvestigatorAtCaseCapacity {
            investigator: detective,
        }
    );
    validate_state(&state).expect("rejected assignment should preserve structural validity");
    validate_invariants(&state);
}

#[test]
fn investigator_staffing_is_versioned_indexed_and_blocks_foreign_reassignment() {
    let registry = build_registry();
    let mut state = AppState::new(0xD37E_C71E);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Central Detectives".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let other_police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Detectives".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "South Ward Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let first = insert_test_investigator(&mut state, police, "Harlan", 82);
    let second = insert_test_investigator(&mut state, police, "Meyer", 74);
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "South Ward conspiracy".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");

    validate_assign_investigator(&state, investigation, first)
        .expect("lead assignment should validate")
        .commit(&mut state)
        .expect("lead assignment should commit");
    assert_eq!(
        validate_assign_investigator(&state, investigation, second)
            .expect_err("an active case has a single lead seat while its current lead is assigned"),
        InvestigationError::LeadSeatFilled {
            investigation,
            lead: first,
        }
    );

    let record = state
        .legal()
        .get_investigation(investigation)
        .expect("investigation should exist");
    assert_eq!(record.lead_investigator(), Some(first));
    assert_eq!(record.assigned_investigators().len(), 1);
    assert_eq!(
        state
            .legal()
            .investigations_for_investigator(first)
            .map(|case| case.id())
            .collect::<Vec<_>>(),
        vec![investigation]
    );

    let restored = restore_save(
        &registry,
        build_save(&registry, &state).expect("staffed case state should save"),
    )
    .expect("staffed case state should restore");
    let restored_case = restored
        .legal()
        .get_investigation(investigation)
        .expect("restored investigation should exist");
    assert_eq!(restored_case.lead_investigator(), Some(first));
    assert_eq!(
        restored
            .legal()
            .investigations_for_investigator(first)
            .map(|case| case.id())
            .collect::<Vec<_>>(),
        vec![investigation]
    );

    let error = validate_reassign_character(&state, first, Some(other_police), None)
        .expect_err("active case assignment must block organization reassignment");
    assert_eq!(
        error,
        WorldError::ActiveInvestigationAssignment {
            character: first,
            investigation,
        }
    );

    // Shelving releases the seat: the detective becomes free to transfer, exactly as a cold
    // shelf holds no institutional attention.
    validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
        .expect("suspension should validate")
        .commit(&mut state)
        .expect("suspension should commit");
    validate_reassign_character(&state, first, Some(other_police), None)
        .expect("released investigator should be free to transfer")
        .commit(&mut state)
        .expect("released investigator transfer should commit");
    assert_eq!(
        state.legal().investigations_for_investigator(first).count(),
        0
    );
    validate_state(&state).expect("staffed investigation should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn investigator_assignment_token_rejects_case_changes_after_validation() {
    let registry = build_registry();
    let mut state = AppState::new(0x57A1_ECA5);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Versioned Case Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Versioned Case Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let detective = insert_test_investigator(&mut state, police, "Doyle", 79);
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Changing evidence file".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let assignment = validate_assign_investigator(&state, investigation, detective)
        .expect("assignment should validate against initial case version");

    validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Organization(criminal),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    )
    .expect("new evidence should validate")
    .commit(&mut state)
    .expect("new evidence should commit");

    let error = assignment
        .commit(&mut state)
        .expect_err("case mutation must invalidate older staffing token");
    assert_eq!(
        error,
        InvestigationError::StaleInvestigation {
            investigation,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .legal()
            .get_investigation(investigation)
            .expect("investigation should exist")
            .lead_investigator(),
        None
    );
    validate_state(&state).expect("stale token rejection must leave state valid");
    validate_invariants(&state);
}

#[test]
fn case_graph_indexes_track_shared_subjects_and_evidence_kinds() {
    let registry = build_registry();
    let mut state = AppState::new(0xCA53_1933);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Case Graph Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let other_police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Foreign Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("second police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Case Graph Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let character = insert_character(
        &mut state,
        CharacterDraft {
            name: "Case Graph Associate".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("character fixture should validate");

    let first = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "First linked incident".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("first investigation should validate")
    .commit(&mut state)
    .expect("validated first investigation should commit");
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation: first,
            custodian: police,
            subject: EntityRef::Character(character),
            origin: Some(EntityRef::Organization(criminal)),
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    )
    .expect("case-link evidence should validate")
    .commit(&mut state)
    .expect("validated case-link evidence should commit");
    let second = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Second linked incident".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(character)]),
        },
    )
    .expect("second investigation should validate")
    .commit(&mut state)
    .expect("validated second investigation should commit");

    assert_eq!(
        state
            .legal()
            .investigations_for_subject(EntityRef::Character(character))
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(
        state
            .legal()
            .evidence_of_kind(EvidenceKind::KnownAssociation)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![evidence]
    );
    assert_eq!(
        state
            .legal()
            .evidence_from_origin(EntityRef::Organization(criminal))
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![evidence]
    );

    let error = match validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation: first,
            custodian: other_police,
            subject: EntityRef::Character(character),
            origin: None,
            kind: EvidenceKind::WitnessTestimony,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    ) {
        Ok(_) => {
            panic!("foreign precinct must not append evidence to another authority's case")
        }
        Err(error) => error,
    };
    assert_eq!(
        error,
        InvestigationError::CustodianMismatch {
            investigation: first,
            custodian: other_police,
        }
    );
    validate_state(&state).expect("case graph indexes should remain structurally valid");
    validate_invariants(&state);
}

#[test]
fn operation_originated_cases_cool_and_reopen_through_the_canonical_transition() {
    let registry = build_registry();
    let mut state = AppState::new(0xC01D_1933);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cold Case Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cold Case Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Cold Case Leader".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, rating(80)),
                (CapabilityKind::Management, rating(80)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let origin = crate::operations::operation_system::validate_authorize_operation(
        &registry,
        &state,
        crate::operations::OperationDraft {
            title: "Origin surveillance".to_owned(),
            kind: crate::operations::OperationKind::Surveillance,
            responsible_organization: criminal,
            leader,
            objective: crate::operations::OperationObjective::GatherInformation {
                target: EntityRef::Organization(criminal),
            },
            approach: crate::operations::OperationApproach::Covert,
            roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("origin operation should validate")
    .commit(&mut state)
    .expect("origin operation should commit");
    let case = validate_incident_intake(
        &state,
        IncidentIntakeDraft {
            owner: police,
            title: "Sober incident inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(origin)]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Operation(origin),
                origin: Some(EntityRef::Operation(origin)),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            }],
            origin: Some(EntityRef::Operation(origin)),
            notified_organizations: BTreeSet::from([criminal]),
            witness: None,
        },
    )
    .expect("incident intake should validate")
    .commit(&mut state)
    .expect("incident intake should commit")
    .investigation;
    let institution_authored = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Institution-authored case stays put".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("institution-authored case should validate")
    .commit(&mut state)
    .expect("institution-authored case should commit");

    // A short cold window shelves only the operation-originated case without an identified
    // suspect; an operation-originated case that named a concrete character is a real lead and
    // stays active.
    let identified = validate_incident_intake(
        &state,
        IncidentIntakeDraft {
            owner: police,
            title: "Identified incident inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Operation(origin), EntityRef::Character(leader)]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Character(leader),
                origin: Some(EntityRef::Operation(origin)),
                kind: EvidenceKind::KnownAssociation,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            }],
            origin: Some(EntityRef::Operation(origin)),
            notified_organizations: BTreeSet::from([criminal]),
            witness: None,
        },
    )
    .expect("identified incident intake should validate")
    .commit(&mut state)
    .expect("identified incident intake should commit")
    .investigation;
    state.advance_clock(SimDuration::from_minutes(121));
    let suspended = apply_cold_case_decay(&mut state, SimDuration::from_minutes(120))
        .expect("cold-case decay should resolve");
    assert_eq!(
        suspended,
        ColdCaseDecayOutcome {
            suspended: vec![case],
            closed: Vec::new()
        }
    );
    let record = state
        .legal()
        .get_investigation(case)
        .expect("cold case should persist");
    assert_eq!(record.status(), InvestigationStatus::Suspended);
    assert_eq!(
        state
            .legal()
            .get_investigation(institution_authored)
            .expect("institution-authored case should persist")
            .status(),
        InvestigationStatus::Active
    );
    assert_eq!(
        state
            .legal()
            .get_investigation(identified)
            .expect("identified case should persist")
            .status(),
        InvestigationStatus::Active
    );
    validate_state(&state).expect("cold decay state should validate");
    validate_invariants(&state);

    // The cold-decay index and case provenance survive save/restore, so a campaign loaded
    // after the shelf decision keeps the same institutional state.
    state = restore_save(
        &registry,
        build_save(&registry, &state).expect("cold case state should save"),
    )
    .expect("cold case state should restore");
    validate_state(&state).expect("restored cold decay state should validate");
    validate_invariants(&state);

    // The owning authority can resume the shelved case through the canonical transition; the
    // resume refreshes institutional activity so it does not immediately re-cool.
    validate_transition_investigation(&state, case, InvestigationTransition::Resume)
        .expect("resume should validate")
        .commit(&mut state)
        .expect("resume should commit");
    validate_state(&state).expect("resumed cold case state should validate");
    validate_invariants(&state);
}

#[test]
fn cold_case_decay_closes_a_fully_worked_case_whose_every_subject_is_detained() {
    let registry = build_registry();
    let mut state = AppState::new(0xC1EA_1933);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cleared Case Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Cleared Case Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Cleared Case Leader".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([
                (CapabilityKind::Surveillance, rating(80)),
                (CapabilityKind::Management, rating(80)),
            ]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let lieutenant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Cleared Case Lieutenant".to_owned(),
            organization: Some(criminal),
            supervisor: Some(leader),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(60))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("lieutenant fixture should validate");
    let origin = crate::operations::operation_system::validate_authorize_operation(
        &registry,
        &state,
        crate::operations::OperationDraft {
            title: "Origin surveillance".to_owned(),
            kind: crate::operations::OperationKind::Surveillance,
            responsible_organization: criminal,
            leader,
            objective: crate::operations::OperationObjective::GatherInformation {
                target: EntityRef::Organization(criminal),
            },
            approach: crate::operations::OperationApproach::Covert,
            roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("origin operation should validate")
    .commit(&mut state)
    .expect("origin operation should commit");
    let identified = validate_incident_intake(
        &state,
        IncidentIntakeDraft {
            owner: police,
            title: "Cleared identified inquiry".to_owned(),
            subjects: BTreeSet::from([
                EntityRef::Operation(origin),
                EntityRef::Character(lieutenant),
            ]),
            evidence: vec![crate::legal::IncidentEvidenceDraft {
                subject: EntityRef::Character(lieutenant),
                origin: Some(EntityRef::Operation(origin)),
                kind: EvidenceKind::KnownAssociation,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            }],
            origin: Some(EntityRef::Operation(origin)),
            notified_organizations: BTreeSet::from([criminal]),
            witness: None,
        },
    )
    .expect("identified incident intake should validate")
    .commit(&mut state)
    .expect("identified incident intake should commit")
    .investigation;
    let evidence = *state
        .legal()
        .get_investigation(identified)
        .and_then(|record| record.evidence().iter().next())
        .expect("intake recorded its evidence");

    // The subject's arrest sits under this very case; the case is cleared by arrest and
    // must close through decay instead of lingering active with a held investigator slot.
    crate::legal::arrest_system::validate_arrest(
        &state,
        crate::legal::ArrestDraft {
            character: lieutenant,
            investigation: identified,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("evidence-backed arrest should validate")
    .commit(&mut state)
    .expect("evidence-backed arrest should commit");

    state.advance_clock(SimDuration::from_minutes(121));
    let decayed = apply_cold_case_decay(&mut state, SimDuration::from_minutes(120))
        .expect("cold-case decay should resolve");
    assert_eq!(
        decayed,
        ColdCaseDecayOutcome {
            suspended: Vec::new(),
            closed: vec![identified]
        }
    );
    assert_eq!(
        state
            .legal()
            .get_investigation(identified)
            .map(|record| record.status()),
        Some(InvestigationStatus::Closed)
    );
    validate_state(&state).expect("cleared-case state should validate");
    validate_invariants(&state);
}

#[test]
fn weak_evidence_does_not_promote_a_character_to_identified_suspect() {
    let registry = build_registry();
    let mut state = AppState::new(0x0DD_555);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Promotion Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let suspect = insert_character(
        &mut state,
        CharacterDraft {
            name: "Ray Cusack".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("suspect fixture should validate");
    let outfit = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Promotion Outfit".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("outfit fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Weak tip inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(outfit)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");

    let weak_tip = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    )
    .expect("weak evidence should validate")
    .commit(&mut state)
    .expect("weak evidence should commit");
    assert!(
        !state
            .legal()
            .get_investigation(investigation)
            .expect("investigation should exist")
            .subjects()
            .contains(&EntityRef::Character(suspect)),
        "weak evidence must not promote a character to case subject"
    );

    let corroboration = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating evidence should validate")
    .commit(&mut state)
    .expect("corroborating evidence should commit");
    assert!(state
        .legal()
        .get_investigation(investigation)
        .expect("investigation should exist")
        .subjects()
        .contains(&EntityRef::Character(suspect)));
    assert_ne!(weak_tip, corroboration);

    validate_state(&state).expect("subject promotion state should validate");
    validate_invariants(&state);
}
