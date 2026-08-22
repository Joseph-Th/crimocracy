//! Focused tests for deterministic investigation-work scheduling and resolution.

use super::*;
use crate::build_registry;
use crate::core::invariants::{
    validate_invariants, validate_state, validate_state_against_registry,
};
use crate::core::persistence::{build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::SimDuration;
use crate::legal::case_graph::resolve_evidence_path;
use crate::legal::investigation_system::{
    validate_add_evidence, validate_assign_investigator, validate_open_investigation,
    validate_remove_investigator, InvestigationError,
};
use crate::legal::{EvidenceDraft, InvestigationDraft, InvestigationWorkFocus, InvestigatorRole};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating};
use std::collections::{BTreeMap, BTreeSet};

struct WorkFixture {
    state: AppState,
    police: crate::core::id::OrganizationId,
    investigation: InvestigationId,
    investigator: CharacterId,
    second_investigator: CharacterId,
    first: CharacterId,
    middle: CharacterId,
    target: CharacterId,
    first_evidence: EvidenceId,
    second_evidence: EvidenceId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating must be valid")
}

fn make_fixture(
    investigator_skill: u8,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
) -> WorkFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x1A7E_5731);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Pattern Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Pattern Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let investigator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detective Harlan".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                rating(investigator_skill),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("investigator fixture should validate");
    let second_investigator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detective Vera".to_owned(),
            organization: Some(police),
            supervisor: Some(investigator),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                rating(investigator_skill),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("second investigator fixture should validate");
    let mut insert_subject = |name: &str| {
        insert_character(
            &mut state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("case subject fixture should validate")
    };
    let first = insert_subject("Frank Dello");
    let middle = insert_subject("Maria Vale");
    let target = insert_subject("Fulton Garage Manager");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Vehicle association inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(first)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("investigation fixture should commit");
    validate_assign_investigator(&state, investigation, investigator, InvestigatorRole::Lead)
        .expect("investigator assignment should validate")
        .commit(&mut state)
        .expect("investigator assignment should commit");

    let first_evidence = add_evidence(
        &mut state,
        TestEvidenceDraft {
            investigation,
            police,
            subject: EntityRef::Character(middle),
            origin: EntityRef::Character(first),
            strength,
            reliability,
            admissibility,
        },
    );
    let second_evidence = add_evidence(
        &mut state,
        TestEvidenceDraft {
            investigation,
            police,
            subject: EntityRef::Character(target),
            origin: EntityRef::Character(middle),
            strength,
            reliability,
            admissibility,
        },
    );
    WorkFixture {
        state,
        police,
        investigation,
        investigator,
        second_investigator,
        first,
        middle,
        target,
        first_evidence,
        second_evidence,
    }
}

struct TestEvidenceDraft {
    investigation: InvestigationId,
    police: crate::core::id::OrganizationId,
    subject: EntityRef,
    origin: EntityRef,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
}

fn add_evidence(state: &mut AppState, draft: TestEvidenceDraft) -> EvidenceId {
    let TestEvidenceDraft {
        investigation,
        police,
        subject,
        origin,
        strength,
        reliability,
        admissibility,
    } = draft;
    validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject,
            origin: Some(origin),
            kind: EvidenceKind::KnownAssociation,
            strength,
            reliability,
            admissibility,
            discovered_at: state.now(),
        },
    )
    .expect("evidence fixture should validate")
    .commit(state)
    .expect("evidence fixture should commit")
}

fn work_draft(fixture: &WorkFixture) -> InvestigationWorkDraft {
    InvestigationWorkDraft {
        investigation: fixture.investigation,
        investigator: fixture.investigator,
        kind: InvestigationWorkKind::PatternAnalysis,
        focus: InvestigationWorkFocus::new(
            EntityRef::Character(fixture.first),
            EntityRef::Character(fixture.target),
        ),
    }
}

#[test]
fn pattern_analysis_resolves_to_derived_evidence_with_provenance() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("pattern analysis should validate")
            .commit(&mut fixture.state)
            .expect("pattern analysis should schedule");

    let due_at = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("scheduled work should exist")
        .due_at();
    let early_error = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        work,
        InvestigationWorkRandomness::new(0),
    )
    .expect_err("work must not resolve before its due time");
    assert_eq!(
        early_error,
        InvestigationWorkError::WorkNotDue { work, due_at }
    );

    for _ in 0..359 {
        let outcome = run_tick(&registry, &mut fixture.state);
        assert!(outcome.resolved_investigation_work.is_empty());
    }
    let outcome = run_tick(&registry, &mut fixture.state);
    assert_eq!(outcome.resolved_investigation_work, vec![work]);

    let record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("completed work should exist");
    assert_eq!(record.status(), InvestigationWorkStatus::Completed);
    let resolution = record.resolution().expect("completed work must resolve");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Connected);
    assert!(resolution.margin() > 0);
    assert_eq!(resolution.superseded_by(), None);
    let derived_id = resolution
        .derived_evidence()
        .expect("connected pattern analysis must create evidence");
    let derived = fixture
        .state
        .legal()
        .get_evidence(derived_id)
        .expect("derived evidence should exist");
    assert_eq!(derived.kind(), EvidenceKind::PatternLink);
    assert_eq!(
        derived.derived_from(),
        &BTreeSet::from([fixture.first_evidence, fixture.second_evidence])
    );
    assert_eq!(derived.origin(), Some(EntityRef::Character(fixture.first)));
    assert_eq!(derived.subject(), EntityRef::Character(fixture.target));
    assert_eq!(
        fixture
            .state
            .legal()
            .derived_evidence_from(fixture.first_evidence)
            .map(|evidence| evidence.id())
            .collect::<Vec<_>>(),
        vec![derived_id]
    );
    let direct_path = resolve_evidence_path(
        &fixture.state,
        fixture.investigation,
        EntityRef::Character(fixture.first),
        EntityRef::Character(fixture.target),
    )
    .expect("case graph should resolve")
    .expect("derived evidence should create a direct case link");
    assert_eq!(direct_path.links.len(), 1);
    assert_eq!(direct_path.links[0].evidence, derived_id);

    validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator)
        .expect("completed work should release the investigator dependency")
        .commit(&mut fixture.state)
        .expect("investigator release should commit");
    validate_state(&fixture.state).expect("completed pattern analysis state should be valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("completed pattern analysis should match authored definitions");
    validate_invariants(&fixture.state);
}

#[test]
fn fully_inadmissible_sources_derive_inadmissible_pattern_links() {
    let registry = build_registry();
    // Strong, credible sources keep the analysis conclusive while their shared inadmissibility
    // must carry into the derived evidence instead of being laundered into "disputed".
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Inadmissible,
    );
    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("pattern analysis should validate")
            .commit(&mut fixture.state)
            .expect("pattern analysis should schedule");

    loop {
        let outcome = run_tick(&registry, &mut fixture.state);
        if outcome.resolved_investigation_work.contains(&work) {
            break;
        }
        assert!(
            outcome.resolved_investigation_work.is_empty(),
            "no other work should exist in this fixture"
        );
    }

    let record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("completed work should exist");
    let resolution = record.resolution().expect("completed work must resolve");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Connected);
    let derived_id = resolution
        .derived_evidence()
        .expect("connected pattern analysis must create evidence");
    let derived = fixture
        .state
        .legal()
        .get_evidence(derived_id)
        .expect("derived evidence should exist");
    assert_eq!(
        derived.admissibility(),
        Admissibility::Inadmissible,
        "evidence derived solely from inadmissible sources cannot pass as disputed"
    );
    validate_state(&fixture.state).expect("inadmissible derivation state should be valid");
    validate_invariants(&fixture.state);
}

#[test]
fn evidence_review_develops_case_owned_evidence_without_inventing_subjects() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let fingerprint = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.first),
            origin: None,
            kind: EvidenceKind::Fingerprint,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("fingerprint evidence should validate")
    .commit(&mut fixture.state)
    .expect("fingerprint evidence should commit");
    let subjects_before = fixture
        .state
        .legal()
        .get_investigation(fixture.investigation)
        .expect("investigation should persist")
        .subjects()
        .clone();
    let draft = InvestigationWorkDraft {
        investigation: fixture.investigation,
        investigator: fixture.investigator,
        kind: InvestigationWorkKind::EvidenceReview,
        focus: InvestigationWorkFocus::evidence(fingerprint),
    };
    let work = validate_schedule_investigation_work(&registry, &fixture.state, draft)
        .expect("case-owned fingerprint should support evidence review")
        .commit(&mut fixture.state)
        .expect("evidence review should schedule");
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("scheduled evidence review should persist")
            .due_at(),
        SimTime::from_minutes(180)
    );

    for _ in 0..179 {
        assert!(run_tick(&registry, &mut fixture.state)
            .resolved_investigation_work
            .is_empty());
    }
    let outcome = run_tick(&registry, &mut fixture.state);
    assert_eq!(outcome.resolved_investigation_work, vec![work]);
    let record = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("completed evidence review should persist");
    let resolution = record
        .resolution()
        .expect("review should have a resolution");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Developed);
    let derived_id = resolution
        .derived_evidence()
        .expect("successful evidence review should derive forensic analysis");
    let source = fixture
        .state
        .legal()
        .get_evidence(fingerprint)
        .expect("source fingerprint should persist");
    let derived = fixture
        .state
        .legal()
        .get_evidence(derived_id)
        .expect("forensic analysis should persist");
    assert_eq!(derived.kind(), EvidenceKind::ForensicAnalysis);
    assert_eq!(derived.subject(), source.subject());
    assert_eq!(derived.origin(), source.origin());
    assert_eq!(derived.strength(), source.strength());
    assert_eq!(derived.reliability(), EvidenceReliability::Credible);
    assert_eq!(derived.admissibility(), source.admissibility());
    assert_eq!(derived.derived_from(), &BTreeSet::from([fingerprint]));
    assert_eq!(
        fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist after review")
            .subjects(),
        &subjects_before
    );
    assert!(matches!(
        validate_schedule_investigation_work(&registry, &fixture.state, draft),
        Err(InvestigationWorkError::EvidenceAlreadyReviewed {
            evidence,
            derived
        }) if evidence == fingerprint && derived == derived_id
    ));
    validate_state(&fixture.state).expect("evidence review state should validate");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("evidence review should remain registry-valid");
    validate_invariants(&fixture.state);
}

#[test]
fn weak_pattern_analysis_is_inconclusive_without_fabricating_evidence() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        5,
        EvidenceStrength::Weak,
        EvidenceReliability::Questionable,
        Admissibility::Inadmissible,
    );
    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("weak pattern analysis should still be valid work")
            .commit(&mut fixture.state)
            .expect("weak pattern analysis should schedule");
    fixture.state.advance_clock(SimDuration::from_minutes(360));
    let plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        work,
        InvestigationWorkRandomness::new(12),
    )
    .expect("due weak work should resolve a plan");
    assert_eq!(plan.outcome(), InvestigationWorkOutcome::Inconclusive);
    assert!(plan.margin() < 0);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
        .expect("fresh weak-work plan should validate")
        .commit(&mut fixture.state)
        .expect("inconclusive work should commit");
    let resolution = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("work should exist")
        .resolution()
        .expect("work should be completed");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Inconclusive);
    assert_eq!(resolution.derived_evidence(), None);
    assert_eq!(
        fixture
            .state
            .legal()
            .evidence_of_kind(EvidenceKind::PatternLink)
            .count(),
        0
    );
    validate_state(&fixture.state).expect("inconclusive work state should be valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("inconclusive work should match authored definitions");
}

#[test]
fn new_direct_evidence_supersedes_scheduled_pattern_analysis() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("pattern analysis should validate")
            .commit(&mut fixture.state)
            .expect("pattern analysis should schedule");
    let direct = add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: EntityRef::Character(fixture.first),
            strength: EvidenceStrength::Direct,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
        },
    );
    fixture.state.advance_clock(SimDuration::from_minutes(360));
    let plan = decide_investigation_work_resolution(
        &registry,
        &fixture.state,
        work,
        InvestigationWorkRandomness::new(0),
    )
    .expect("superseded work should still resolve normally");
    assert_eq!(plan.outcome(), InvestigationWorkOutcome::Superseded);
    validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
        .expect("superseded plan should validate")
        .commit(&mut fixture.state)
        .expect("superseded work should commit without derived evidence");
    let resolution = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("work should exist")
        .resolution()
        .expect("work should resolve");
    assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Superseded);
    assert_eq!(resolution.superseded_by(), Some(direct));
    assert_eq!(resolution.derived_evidence(), None);
    assert_eq!(
        fixture
            .state
            .legal()
            .evidence_of_kind(EvidenceKind::PatternLink)
            .count(),
        0
    );
    validate_state(&fixture.state).expect("superseded work state should be valid");
    validate_state_against_registry(&registry, &fixture.state)
        .expect("superseded work should match authored definitions");
}

#[test]
fn scheduling_is_versioned_deduplicated_and_blocks_investigator_release() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let stale_removal =
        validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator)
            .expect("investigator should initially be releasable");
    let stale_schedule =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("initial schedule token should validate");
    add_evidence(
        &mut fixture.state,
        TestEvidenceDraft {
            investigation: fixture.investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.middle),
            origin: EntityRef::Character(fixture.target),
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Mixed,
            admissibility: Admissibility::Unknown,
        },
    );
    assert!(matches!(
        stale_schedule.commit(&mut fixture.state),
        Err(InvestigationWorkError::StaleInvestigation { .. })
    ));

    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("fresh schedule should validate after case change")
            .commit(&mut fixture.state)
            .expect("fresh schedule should commit");
    assert!(matches!(
        stale_removal.commit(&mut fixture.state),
        Err(InvestigationError::StaleInvestigation { .. })
    ));
    assert_eq!(
        validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator,)
            .expect_err("scheduled work must block investigator release"),
        InvestigationError::ScheduledInvestigationWork {
            investigator: fixture.investigator,
            work,
        }
    );
    let reverse_focus = InvestigationWorkDraft {
        investigation: fixture.investigation,
        investigator: fixture.investigator,
        kind: InvestigationWorkKind::PatternAnalysis,
        focus: InvestigationWorkFocus::new(
            EntityRef::Character(fixture.target),
            EntityRef::Character(fixture.first),
        ),
    };
    assert_eq!(
        validate_schedule_investigation_work(&registry, &fixture.state, reverse_focus)
            .expect_err("reverse focus must canonicalize to the same scheduled work"),
        InvestigationWorkError::DuplicateScheduledWork { work }
    );
    validate_state(&fixture.state).expect("scheduled work dependencies should remain valid");
}

#[test]
fn generic_evidence_path_cannot_forge_pattern_link() {
    let fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let error = match validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: Some(EntityRef::Character(fixture.first)),
            kind: EvidenceKind::PatternLink,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    ) {
        Ok(_) => panic!("pattern links must require the canonical analysis pipeline"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        InvestigationError::PatternLinkRequiresInvestigationWork
    );
}

#[test]
fn save_round_trip_preserves_due_work_and_deterministic_resolution() {
    let registry = build_registry();
    let mut fixture = make_fixture(
        90,
        EvidenceStrength::Strong,
        EvidenceReliability::Credible,
        Admissibility::Admissible,
    );
    let work =
        validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
            .expect("pattern analysis should validate")
            .commit(&mut fixture.state)
            .expect("pattern analysis should schedule");
    for _ in 0..359 {
        run_tick(&registry, &mut fixture.state);
    }
    let mut restored = restore_save(
        &registry,
        build_save(&registry, &fixture.state).expect("pending work should save"),
    )
    .expect("pending work should restore");
    let original_outcome = run_tick(&registry, &mut fixture.state);
    let restored_outcome = run_tick(&registry, &mut restored);
    assert_eq!(original_outcome, restored_outcome);
    assert_eq!(original_outcome.resolved_investigation_work, vec![work]);

    let original_resolution = fixture
        .state
        .legal()
        .get_investigation_work(work)
        .expect("original work should exist")
        .resolution()
        .expect("original work should resolve")
        .clone();
    let restored_resolution = restored
        .legal()
        .get_investigation_work(work)
        .expect("restored work should exist")
        .resolution()
        .expect("restored work should resolve")
        .clone();
    assert_eq!(original_resolution, restored_resolution);
    let original_derived = original_resolution
        .derived_evidence()
        .expect("strong work should derive evidence");
    let restored_derived = restored_resolution
        .derived_evidence()
        .expect("restored strong work should derive evidence");
    assert_eq!(original_derived, restored_derived);
    assert_eq!(
        fixture
            .state
            .legal()
            .get_evidence(original_derived)
            .expect("original derived evidence should exist")
            .derived_from(),
        restored
            .legal()
            .get_evidence(restored_derived)
            .expect("restored derived evidence should exist")
            .derived_from()
    );

    let second_investigation = validate_open_investigation(
        &restored,
        InvestigationDraft {
            owner: fixture.police,
            title: "Post-restore association inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.first)]),
        },
    )
    .expect("post-restore investigation should validate")
    .commit(&mut restored)
    .expect("post-restore investigation should commit");
    validate_assign_investigator(
        &restored,
        second_investigation,
        fixture.second_investigator,
        InvestigatorRole::Lead,
    )
    .expect("post-restore investigator assignment should validate")
    .commit(&mut restored)
    .expect("post-restore investigator assignment should commit");
    add_evidence(
        &mut restored,
        TestEvidenceDraft {
            investigation: second_investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.middle),
            origin: EntityRef::Character(fixture.first),
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    add_evidence(
        &mut restored,
        TestEvidenceDraft {
            investigation: second_investigation,
            police: fixture.police,
            subject: EntityRef::Character(fixture.target),
            origin: EntityRef::Character(fixture.middle),
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Admissible,
        },
    );
    let second_work = validate_schedule_investigation_work(
        &registry,
        &restored,
        InvestigationWorkDraft {
            investigation: second_investigation,
            investigator: fixture.second_investigator,
            kind: InvestigationWorkKind::PatternAnalysis,
            focus: InvestigationWorkFocus::new(
                EntityRef::Character(fixture.first),
                EntityRef::Character(fixture.target),
            ),
        },
    )
    .expect("post-restore pattern analysis should validate")
    .commit(&mut restored)
    .expect("post-restore pattern analysis should allocate a fresh work ID");
    assert!(second_work.raw() > work.raw());
    validate_state_against_registry(&registry, &restored)
        .expect("restored work should retain authored causal validity");
}
