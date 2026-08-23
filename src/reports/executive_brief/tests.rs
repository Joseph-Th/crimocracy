//! Focused tests for executive brief synthesis, cadence, and attention selection.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::simulation::run_tick;
use crate::decisions::decision_system::{
    validate_request_recruitment_approval, validate_resolve_decision,
};
use crate::decisions::{DecisionResponse, RecruitmentApprovalRequestDraft};
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crate::recruitment::RecruitmentApproach;
use crate::reports::report_system::validate_record_report;
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel};
use crate::world::world_system::{
    designate_player_organization, insert_character, insert_organization,
};
use crate::world::{
    AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

struct BriefFixture {
    registry: Registry,
    state: AppState,
    organization: OrganizationId,
    recruiter: crate::core::id::CharacterId,
    candidate: crate::core::id::CharacterId,
    mandate: crate::core::id::MandateId,
}

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("fixture rating must be valid")
}

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
}

fn make_test_brief_fixture() -> BriefFixture {
    let registry = build_registry();
    let mut state = AppState::new(0xB21E_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Executive Brief Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("player organization fixture should validate");
    designate_player_organization(&mut state, organization)
        .expect("player organization designation should validate");
    let recruiter = insert_character(
        &mut state,
        CharacterDraft {
            name: "Personnel Manager".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(75))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("recruiter fixture should validate");
    let candidate = insert_character(
        &mut state,
        CharacterDraft {
            name: "Independent Candidate".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("candidate fixture should validate");
    validate_set_relationship(
        &state,
        candidate,
        recruiter,
        RelationshipDimensions {
            trust: level(60),
            respect: level(65),
            fear: level(5),
            affection: level(30),
            dependence: level(10),
            resentment: level(0),
            debt: level(5),
        },
    )
    .expect("recruitment relationship fixture should validate")
    .commit(&mut state);
    let mandate = validate_assign_mandate(
        &state,
        MandateDraft {
            organization,
            manager: recruiter,
            scopes: BTreeSet::from([ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            )]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .expect("personnel mandate fixture should validate")
    .commit(&mut state)
    .expect("personnel mandate fixture should commit");
    BriefFixture {
        registry,
        state,
        organization,
        recruiter,
        candidate,
        mandate,
    }
}

fn request_recruitment_approval(fixture: &mut BriefFixture) -> DecisionRequestId {
    validate_request_recruitment_approval(
        &fixture.registry,
        &fixture.state,
        RecruitmentApprovalRequestDraft {
            authority: MandateAuthority {
                mandate: fixture.mandate,
                manager: fixture.recruiter,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
            },
            target_organization: fixture.organization,
            recruiter: fixture.recruiter,
            candidate: fixture.candidate,
            approach: RecruitmentApproach::PersonalAppeal,
            attention: AttentionClass::Exception,
            summary: "Personnel manager requests approval for a recruitment approach.".to_owned(),
        },
    )
    .expect("approval request fixture should validate")
    .commit(&mut fixture.state)
    .expect("approval request fixture should commit")
    .decision
}

fn record_report(
    state: &mut AppState,
    recipient: OrganizationId,
    title: &str,
    entries: Vec<ReportEntry>,
) -> ReportId {
    validate_record_report(
        state,
        ReportDraft {
            recipient,
            kind: ReportKind::Legal,
            title: title.to_owned(),
            entries,
        },
    )
    .expect("source report fixture should validate")
    .commit(state)
    .expect("source report fixture should commit")
}

fn entry(attention: AttentionClass, summary: &str) -> ReportEntry {
    ReportEntry {
        attention,
        summary: summary.to_owned(),
        sources: Vec::new(),
        entities: BTreeSet::new(),
        decision: None,
    }
}

#[test]
fn synthesis_prioritizes_pending_decisions_filters_routine_and_deduplicates_sources() {
    let mut fixture = make_test_brief_fixture();
    let decision = request_recruitment_approval(&mut fixture);
    record_report(
        &mut fixture.state,
        fixture.organization,
        "First source report",
        vec![
            entry(
                AttentionClass::Routine,
                "Routine collection completed normally.",
            ),
            entry(
                AttentionClass::Notable,
                "Detectives increased questioning near the docks.",
            ),
        ],
    );
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Duplicate source report",
        vec![entry(
            AttentionClass::Notable,
            "Detectives increased questioning near the docks.",
        )],
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
        .expect("daily brief should synthesize current executive information");
    assert_eq!(plan.entries().len(), 2);
    assert_eq!(plan.entries()[0].attention, AttentionClass::Exception);
    assert_eq!(plan.entries()[0].decision, Some(decision));
    assert!(plan.entries()[0]
        .entities
        .contains(&EntityRef::DecisionRequest(decision)));
    assert_eq!(plan.entries()[1].attention, AttentionClass::Notable);
    assert_eq!(
        plan.entries()[1].summary,
        "Detectives increased questioning near the docks."
    );

    let report = validate_executive_brief_plan(&fixture.state, plan)
        .expect("fresh executive brief should validate")
        .commit(&mut fixture.state)
        .expect("validated executive brief should commit");
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(report)
            .expect("executive brief should persist")
            .kind(),
        ReportKind::ExecutiveBrief
    );
    assert_eq!(
        decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect_err("a second brief at the same boundary must be rejected"),
        ExecutiveBriefError::AlreadyGenerated { report }
    );
    validate_state(&fixture.state).expect("executive brief state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn stale_plan_rejects_pending_decision_changes_without_partial_mutation() {
    let mut fixture = make_test_brief_fixture();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
        .expect("empty daily brief should initially plan against no pending decisions");
    let decision = request_recruitment_approval(&mut fixture);

    let error = match validate_executive_brief_plan(&fixture.state, plan) {
        Ok(_) => panic!("new pending executive work must stale the older synthesis plan"),
        Err(error) => error,
    };
    assert_eq!(error, ExecutiveBriefError::StalePendingDecisions);
    assert_eq!(
        fixture
            .state
            .decisions()
            .pending_for_recipient(fixture.organization)
            .map(DecisionRequestRecord::id)
            .collect::<Vec<_>>(),
        vec![decision]
    );
    assert!(fixture
        .state
        .reports()
        .latest_for_kind(fixture.organization, ReportKind::ExecutiveBrief)
        .is_none());
    validate_invariants(&fixture.state);
}

#[test]
fn stale_plan_rejects_report_window_changes_without_partial_mutation() {
    let mut fixture = make_test_brief_fixture();
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Initial report",
        vec![entry(AttentionClass::Notable, "Initial notable item.")],
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
        .expect("brief plan should validate before a new source arrives");
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Late report",
        vec![entry(AttentionClass::Notable, "Late notable item.")],
    );

    let error = match validate_executive_brief_plan(&fixture.state, plan) {
        Ok(_) => panic!("new source reports must stale the older synthesis plan"),
        Err(error) => error,
    };
    assert_eq!(error, ExecutiveBriefError::StaleReportWindow);
    assert!(fixture
        .state
        .reports()
        .latest_for_kind(fixture.organization, ReportKind::ExecutiveBrief)
        .is_none());
    validate_invariants(&fixture.state);
}

#[test]
fn source_entry_limit_preserves_priority_and_discloses_overflow() {
    let mut fixture = make_test_brief_fixture();
    let mut entries = vec![
        entry(AttentionClass::Crisis, "Immediate crisis A."),
        entry(AttentionClass::Crisis, "Immediate crisis B."),
    ];
    entries.extend((0..8).map(|index| {
        entry(
            AttentionClass::Notable,
            &format!("Notable source item {index}."),
        )
    }));
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Dense source report",
        entries,
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
        .expect("dense source set should still produce a bounded brief");
    assert_eq!(plan.entries().len(), 9);
    assert_eq!(plan.entries()[0].attention, AttentionClass::Crisis);
    assert_eq!(plan.entries()[1].attention, AttentionClass::Crisis);
    assert!(plan.entries()[8].summary.contains("2 additional items"));
    validate_invariants(&fixture.state);
}

#[test]
fn resolved_decision_report_is_not_resurfaced_as_current_executive_work() {
    let mut fixture = make_test_brief_fixture();
    let decision = request_recruitment_approval(&mut fixture);
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Decision source report",
        vec![ReportEntry {
            attention: AttentionClass::Exception,
            summary: "Recruitment approval is waiting for leadership.".to_owned(),
            sources: Vec::new(),
            entities: BTreeSet::from([EntityRef::DecisionRequest(decision)]),
            decision: Some(decision),
        }],
    );
    validate_resolve_decision(
        &fixture.registry,
        &fixture.state,
        decision,
        fixture.organization,
        DecisionResponse::Reject,
    )
    .expect("decision rejection should validate")
    .commit(&mut fixture.state)
    .expect("decision rejection should commit");
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));

    let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
        .expect("resolved decision history should not block the daily brief");
    assert_eq!(plan.entries().len(), 1);
    assert_eq!(plan.entries()[0].attention, AttentionClass::Routine);
    assert!(plan.entries()[0].decision.is_none());
    validate_invariants(&fixture.state);
}

#[test]
fn next_brief_reads_only_reports_created_after_the_previous_brief() {
    let mut fixture = make_test_brief_fixture();
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Day one source",
        vec![entry(AttentionClass::Notable, "Day one notable item.")],
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let first_plan =
        decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("first daily brief should plan");
    assert_eq!(first_plan.entries()[0].summary, "Day one notable item.");
    validate_executive_brief_plan(&fixture.state, first_plan)
        .expect("first daily brief should validate")
        .commit(&mut fixture.state)
        .expect("first daily brief should commit");

    record_report(
        &mut fixture.state,
        fixture.organization,
        "Day two source",
        vec![entry(AttentionClass::Notable, "Day two notable item.")],
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let second_plan =
        decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("second daily brief should plan from the prior brief cursor");
    assert_eq!(second_plan.entries().len(), 1);
    assert_eq!(second_plan.entries()[0].summary, "Day two notable item.");
    assert!(!second_plan
        .entries()
        .iter()
        .any(|entry| entry.summary == "Day one notable item."));
    validate_invariants(&fixture.state);
}

#[test]
fn daily_tick_generation_is_deterministic_across_save_round_trip() {
    let mut fixture = make_test_brief_fixture();
    record_report(
        &mut fixture.state,
        fixture.organization,
        "Daily source report",
        vec![entry(
            AttentionClass::Notable,
            "A source report is waiting for the next executive cycle.",
        )],
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_439));
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("pre-brief state should build a valid save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored =
        restore_save(&fixture.registry, decoded).expect("pre-brief save should restore");

    let original = run_tick(&fixture.registry, &mut fixture.state);
    let continued = run_tick(&fixture.registry, &mut restored);
    assert_eq!(original, continued);
    let report = original
        .executive_brief
        .expect("daily boundary should generate an executive brief");
    assert_eq!(original.now, SimTime::from_minutes(1_440));
    assert_eq!(
        fixture
            .state
            .reports()
            .get_report(report)
            .expect("original executive brief should persist")
            .entries()[0]
            .summary,
        restored
            .reports()
            .get_report(report)
            .expect("restored executive brief should persist")
            .entries()[0]
            .summary
    );
    validate_state(&fixture.state).expect("original post-brief state should validate");
    validate_state(&restored).expect("restored post-brief state should validate");
    validate_invariants(&fixture.state);
    validate_invariants(&restored);
}
