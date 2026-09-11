//! Reputation consequence, decay, clamping, and reporting tests.

use super::*;
use crate::build_registry;
use crate::core::invariants::validate_invariants;
use crate::core::time::SimDuration;
use crate::social::RelationshipLevel;
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{OrganizationDraft, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

fn level(value: u8) -> RelationshipLevel {
    RelationshipLevel::try_new(value).expect("fixture level should validate")
}

fn make_state() -> (Registry, AppState, OrganizationId) {
    let registry = build_registry();
    let mut state = AppState::new(0x5E9E);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Reputation Test Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization should validate");
    (registry, state, organization)
}

#[test]
fn reputation_deltas_create_sparse_records_clamped_to_the_score_range() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();

    // First touch creates one record at baseline-plus-delta; other dimensions stay
    // at baseline and untouched audiences stay absent entirely.
    let after = apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        7,
    )
    .expect("delta on a live organization should apply");
    assert_eq!(after, baseline + 7);
    assert_eq!(state.reputation.records().count(), 1);
    let record = state
        .reputation
        .get_record(organization, AudienceKind::Police)
        .expect("touched impression should persist");
    assert_eq!(record.score(ReputationDimension::Fear), baseline + 7);
    assert_eq!(
        record.score(ReputationDimension::Competence),
        baseline,
        "untouched dimensions keep the baseline"
    );
    assert!(
        state
            .reputation
            .get_record(organization, AudienceKind::Underworld)
            .is_none()
    );

    // Clamping holds at both rails no matter how large the authored swing.
    let clamped_high = apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        100,
    )
    .expect("clamped delta should apply");
    assert_eq!(clamped_high, 100);
    let clamped_low = apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        -120,
    )
    .expect("clamped delta should apply");
    assert_eq!(clamped_low, 0);

    validate_invariants(&state);
}

#[test]
fn standing_shift_reports_the_clamped_delta_that_was_actually_applied() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();
    let raise_to_rail = i8::try_from(99_i16 - i16::from(baseline))
        .expect("fixture baseline should allow reaching score 99 in one delta");
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        raise_to_rail,
    )
    .expect("fixture standing should move near the upper rail");

    let shifts = apply_vice_inquiry_reputation_consequences(&registry, &mut state, organization)
        .expect("vice inquiry should apply its remaining bounded standing change");
    assert_eq!(shifts.len(), 1);
    assert_eq!(
        shifts[0].delta, 1,
        "reported shift must equal the score movement after clamping, not the larger authored request"
    );
    assert_eq!(
        resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear,
        ),
        100
    );
    validate_invariants(&state);
}

#[test]
fn reputation_deltas_reject_unknown_organizations_without_state_change() {
    let (registry, mut state, _organization) = make_state();
    let missing = crate::core::id::OrganizationId::from_raw(9_999);
    let error = match apply_reputation_delta(
        &registry,
        &mut state,
        missing,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        3,
    ) {
        Err(error) => error,
        Ok(_) => panic!("unknown organizations must be rejected"),
    };
    assert_eq!(error, ReputationError::MissingOrganization(missing));
    assert!(state.reputation.records().next().is_none());
}

#[test]
fn operation_consequences_move_exactly_the_modeled_audiences() {
    let (registry, mut state, organization) = make_state();

    apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Violent,
        OperationObjectiveOutcome::Achieved,
        OperationExposureLevel::Identifying,
    )
    .expect("consequences should apply");
    let touched: Vec<AudienceKind> = state
        .reputation
        .records()
        .filter(|record| record.organization() == organization)
        .filter(|record| {
            crate::reputation::ALL_REPUTATION_DIMENSIONS
                .iter()
                .any(|dimension| record.score(*dimension) != registry.reputation().baseline())
        })
        .map(|record| record.audience())
        .collect();
    assert_eq!(touched.len(), 3);
    for audience in &touched {
        let record = state
            .reputation
            .get_record(organization, *audience)
            .expect("touched audience should hold a record");
        match audience {
            AudienceKind::Underworld => assert_eq!(
                record.score(ReputationDimension::Competence),
                registry.reputation().baseline()
                    + registry.reputation().achieved_underworld_competence() as u8
            ),
            AudienceKind::Police => assert_eq!(
                record.score(ReputationDimension::Fear),
                registry.reputation().baseline()
                    + registry.reputation().identifying_exposure_police_fear() as u8
            ),
            AudienceKind::Businesses => assert_eq!(
                record.score(ReputationDimension::Fear),
                registry.reputation().baseline()
                    + registry.reputation().violent_businesses_fear() as u8
            ),
            AudienceKind::Residents | AudienceKind::Political | AudienceKind::Press => {
                panic!("violent success must not touch {:?}", record.audience())
            }
        }
    }
    validate_invariants(&state);
}

#[test]
fn operation_consequences_do_not_report_clamped_scores_as_movement() {
    let (registry, mut state, organization) = make_state();

    // Police fear already sits on the upper rail before the job.
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        100,
    )
    .expect("pre-clamp should apply");
    let shifts = apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Violent,
        OperationObjectiveOutcome::Achieved,
        OperationExposureLevel::Identifying,
    )
    .expect("consequences should apply");
    // The clamped fear dimension did not move, so it must not surface as standing
    // feedback; the dimensions with headroom still report normally.
    assert!(!shifts.iter().any(|shift| {
        shift.audience == AudienceKind::Police && shift.dimension == ReputationDimension::Fear
    }));
    assert!(shifts.iter().any(|shift| {
        shift.audience == AudienceKind::Underworld
            && shift.dimension == ReputationDimension::Competence
    }));
    assert_eq!(
        state
            .reputation
            .get_record(organization, AudienceKind::Police)
            .expect("police impression should persist")
            .score(ReputationDimension::Fear),
        100,
        "the clamp itself is unchanged"
    );
    validate_invariants(&state);
}

#[test]
fn daily_decay_drifts_touched_impressions_back_to_the_baseline() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();

    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        25,
    )
    .expect("adjustment should apply");

    // Advance onto a day boundary repeatedly: each boundary erodes exactly one step.
    let mut last = baseline + 25;
    for day in 1..=30 {
        state.advance_clock(SimDuration::from_minutes(1_440));
        apply_daily_reputation_decay(&registry, &mut state);
        last = resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Underworld,
            ReputationDimension::Competence,
        );
        let expected = (baseline + 25).saturating_sub(day.min(25) as u8);
        assert_eq!(last, expected, "decay step {day}");
        if last == baseline {
            break;
        }
    }
    assert_eq!(
        last, baseline,
        "impressions fully decay back to the baseline"
    );
    // The faded impression is erased, not pinned at baseline: absent means unremarkable.
    assert!(
        state
            .reputation
            .get_record(organization, AudienceKind::Underworld)
            .is_none(),
        "fully decayed record must be removed"
    );
}

#[test]
fn fresh_reputation_does_not_decay_at_the_next_day_boundary() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();
    state.advance_clock(SimDuration::from_minutes(1_439));
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        10,
    )
    .expect("fresh police fear should apply");

    state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(apply_daily_reputation_decay(&registry, &mut state), 0);
    assert_eq!(
        resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear,
        ),
        baseline + 10,
        "a one-minute-old consequence must not lose a full daily decay step"
    );

    state.advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(apply_daily_reputation_decay(&registry, &mut state), 1);
    assert_eq!(
        resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear,
        ),
        baseline + 9,
        "the first later day boundary after a full day of age should decay normally"
    );
}

#[test]
fn reputation_dimensions_age_independently() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Treachery,
        10,
    )
    .expect("old treachery impression should apply");
    state.advance_clock(SimDuration::from_minutes(1_439));
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        10,
    )
    .expect("fresh competence impression should apply");

    state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(apply_daily_reputation_decay(&registry, &mut state), 1);
    let record = state
        .reputation()
        .get_record(organization, AudienceKind::Underworld)
        .expect("one audience record should retain both active dimensions");
    assert_eq!(record.score(ReputationDimension::Treachery), baseline + 9);
    assert_eq!(record.score(ReputationDimension::Competence), baseline + 10);
}

#[test]
fn clamped_noop_does_not_refresh_reputation_age() {
    let (registry, mut state, organization) = make_state();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        100,
    )
    .expect("initial fear should clamp at the upper rail");
    state.advance_clock(SimDuration::from_minutes(1_439));
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        1,
    )
    .expect("a clamped no-op remains a valid reputation request");

    state.advance_clock(SimDuration::ONE_MINUTE);
    assert_eq!(apply_daily_reputation_decay(&registry, &mut state), 1);
    assert_eq!(
        resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear,
        ),
        99,
        "an event that changed nothing must not make an old impression artificially fresh"
    );
}

#[test]
fn direct_neutralization_removes_sparse_reputation_record_immediately() {
    let (registry, mut state, organization) = make_state();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Businesses,
        ReputationDimension::Fear,
        7,
    )
    .expect("positive standing movement should apply");
    assert!(
        state
            .reputation()
            .get_record(organization, AudienceKind::Businesses)
            .is_some()
    );

    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Businesses,
        ReputationDimension::Fear,
        -7,
    )
    .expect("countervailing standing movement should apply");
    assert!(
        state
            .reputation()
            .get_record(organization, AudienceKind::Businesses)
            .is_none(),
        "a fully neutral impression is represented by absence, not a stored baseline record"
    );
}

#[test]
fn save_rejects_future_dated_reputation_movement() {
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::{SaveError, build_save};

    let (registry, mut state, organization) = make_state();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        5,
    )
    .expect("valid reputation should exist before corruption");
    let future = state
        .now()
        .checked_add(SimDuration::ONE_MINUTE)
        .expect("fixture clock has room");
    let score = state
        .reputation()
        .get_record(organization, AudienceKind::Police)
        .expect("fixture reputation should persist")
        .score(ReputationDimension::Fear);
    state
        .reputation
        .record_mut((organization, AudienceKind::Police))
        .expect("fixture reputation should remain mutable inside its owner test")
        .set_score(ReputationDimension::Fear, score, future);

    let error = build_save(&registry, &state)
        .expect_err("future-dated reputation freshness must fail the real save boundary");
    assert_eq!(
        error,
        SaveError::InvalidState(StateValidationError::InvalidReputationChronology {
            organization,
            audience: AudienceKind::Police,
        })
    );
}

#[test]
fn save_rejects_persisted_neutral_reputation_record() {
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::{SaveError, build_save};

    let (registry, mut state, organization) = make_state();
    state
        .reputation
        .insert_record(ReputationRecord::at_baseline(
            organization,
            AudienceKind::Residents,
            registry.reputation().baseline(),
            state.now(),
        ));

    let error = build_save(&registry, &state)
        .expect_err("neutral sparse reputation must fail the registry-relative save boundary");
    assert_eq!(
        error,
        SaveError::InvalidState(StateValidationError::NeutralReputationRecord {
            organization,
            audience: AudienceKind::Residents,
        })
    );
}

#[test]
fn decay_never_fires_off_the_day_boundary() {
    let (registry, mut state, organization) = make_state();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        10,
    )
    .expect("adjustment should apply");
    state.advance_clock(SimDuration::from_minutes(1_000));
    assert_eq!(apply_daily_reputation_decay(&registry, &mut state), 0);
    let score = resolve_score(
        &registry,
        &state.reputation,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
    );
    assert_eq!(score, registry.reputation().baseline() + 10);
}

#[test]
fn reputation_records_survive_the_persistence_envelope_and_stay_decidable() {
    use crate::core::persistence::{build_save, restore_save};

    let (registry, mut state, organization) = make_state();
    apply_reputation_delta(
        &registry,
        &mut state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        registry.reputation().identifying_exposure_police_fear(),
    )
    .expect("adjustment should apply");

    let envelope = build_save(&registry, &state).expect("save should build");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: crate::core::persistence::SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored =
        restore_save(&registry, decoded).expect("reputation save should restore cleanly");
    let record = restored
        .reputation()
        .get_record(organization, AudienceKind::Police)
        .expect("touched impression should survive the round trip");
    assert_eq!(
        record.score(ReputationDimension::Fear),
        registry.reputation().baseline()
            + registry.reputation().identifying_exposure_police_fear() as u8
    );
    validate_invariants(&restored);
}

#[test]
fn underworld_competence_sways_recruitment_margins() {
    use crate::recruitment::recruitment_system::RecruitmentFactorContext;
    use crate::recruitment::scoring::{
        resolve_recruitment_factors_from_context, resolve_recruitment_margin,
    };
    use crate::recruitment::{
        RecruitmentApproach, RecruitmentFactors, build_recruitment_relationship_snapshot,
    };
    use crate::world::{AutonomyLevel, CharacterDraft};

    let registry = build_registry();
    let definition = registry.recruitment();
    let mut state = AppState::new(0x0C0A);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Competence Target".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization should validate");
    let candidate = insert_character(
        &mut state,
        CharacterDraft {
            name: "Candidate".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("candidate should validate");
    let recruiter = insert_character(
        &mut state,
        CharacterDraft {
            name: "Recruiter".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("recruiter should validate");
    // A neutral positive relationship so the support term contributes identically
    // across both margins. Built through the canonical relationship snapshot builder.
    let dimensions = crate::social::RelationshipDimensions {
        trust: level(50),
        respect: level(50),
        fear: level(0),
        affection: level(50),
        dependence: level(0),
        resentment: level(0),
        debt: level(0),
    };
    let snapshot =
        build_recruitment_relationship_snapshot(candidate, recruiter, Some(dimensions), Some(1));

    let make_factors = |competence: u8| -> RecruitmentFactors {
        resolve_recruitment_factors_from_context(RecruitmentFactorContext {
            definition,
            candidate: state.world.get_character(candidate).expect("candidate"),
            recruiter: state.world.get_character(recruiter).expect("recruiter"),
            approach: RecruitmentApproach::FinancialOpportunity,
            recruiter_relationship: snapshot,
            incumbent_relationship: None,
            perceived_legal_pressure: 0,
            organization_competence: competence,
            had_previous_organization: false,
        })
        .expect("fixture context should resolve")
    };

    let baseline = registry.reputation().baseline();
    let weak = resolve_recruitment_margin(
        definition,
        make_factors(baseline),
        RecruitmentApproach::FinancialOpportunity,
    );
    let raised = (baseline + 20).min(100);
    let strong = resolve_recruitment_margin(
        definition,
        make_factors(raised),
        RecruitmentApproach::FinancialOpportunity,
    );
    let gap = i64::from(raised) - i64::from(baseline);
    let expected_gap = (gap * i64::from(definition.weights().organization_competence) / 100) as i16;
    assert!(expected_gap > 0);
    assert_eq!(strong - weak, expected_gap);
    assert!(weak != strong, "competence reputation must move the margin");
}
fn make_state_with_player() -> (Registry, AppState, OrganizationId) {
    let registry = build_registry();
    let mut state = AppState::new(0x57A7);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Standing Test Family".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization should validate");
    crate::world::world_system::designate_player_organization(&mut state, organization)
        .expect("designation should validate");
    (registry, state, organization)
}

fn standing_reports(state: &AppState, organization: OrganizationId) -> usize {
    state
        .reports()
        .reports_for(organization)
        .filter(|report| report.kind() == ReportKind::Standing)
        .count()
}

#[test]
fn player_standing_shifts_surface_as_notable_standing_reports() {
    let (registry, mut state, organization) = make_state_with_player();

    apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Covert,
        OperationObjectiveOutcome::Achieved,
        OperationExposureLevel::Identifying,
    )
    .expect("standing consequences and feedback should commit");

    assert_eq!(standing_reports(&state, organization), 1);
    let report = state
        .reports()
        .reports_for(organization)
        .find(|report| report.kind() == ReportKind::Standing)
        .expect("standing report should persist");
    assert_eq!(report.entries().len(), 1);
    assert!(matches!(
        report.entries()[0].attention,
        crate::core::attention::AttentionClass::Notable
    ));
    let summary = &report.entries()[0].summary;
    assert!(
        summary.contains("underworld rates our competence higher")
            && summary.contains("police watch us more warily"),
        "the entry must name both shifts: {summary}"
    );
    validate_invariants(&state);
}

#[test]
fn non_player_organizations_keep_their_standing_private() {
    let (registry, mut state, organization) = make_state();

    apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Violent,
        OperationObjectiveOutcome::Achieved,
        OperationExposureLevel::Witnessed,
    )
    .expect("consequences should apply");

    // The reputation moved, but no Standing report exists: this organization is not
    // the player, and rival street standing is not free information.
    assert_eq!(state.reputation.records().count(), 3);
    assert_eq!(standing_reports(&state, organization), 0);
    validate_invariants(&state);
}

#[test]
fn shifts_that_move_nothing_produce_no_feedback() {
    let (registry, mut state, organization) = make_state_with_player();
    apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Covert,
        OperationObjectiveOutcome::Failed,
        OperationExposureLevel::None,
    )
    .expect("zero-shift consequence pass should be a no-op");
    assert_eq!(standing_reports(&state, organization), 0);
    validate_invariants(&state);
}

#[test]
fn player_feedback_id_exhaustion_leaves_reputation_unchanged() {
    let (registry, mut state, organization) = make_state_with_player();
    state.ids.set_next_raw_for_test(IdKind::Report, u32::MAX);

    let error = apply_operation_reputation_consequences(
        &registry,
        &mut state,
        organization,
        OperationApproach::Violent,
        OperationObjectiveOutcome::Achieved,
        OperationExposureLevel::Identifying,
    )
    .expect_err("feedback allocation failure must reject the whole consequence set");
    assert!(matches!(
        error,
        ReputationError::IdExhaustion(IdExhaustionError::Exhausted { kind: "report", .. })
    ));
    assert_eq!(state.reputation.records().count(), 0);
    assert_eq!(standing_reports(&state, organization), 0);
}

#[test]
fn vice_inquiry_raises_owner_police_fear_through_the_canonical_path() {
    let (registry, mut state, organization) = make_state();
    let baseline = registry.reputation().baseline();
    let authored = registry.reputation().vice_inquiry_police_fear();
    assert!(authored > 0, "a dedicated inquiry must scare its owner");

    // A criminal racket owner's police fear rises by exactly the authored step.
    let shifts = apply_vice_inquiry_reputation_consequences(&registry, &mut state, organization)
        .expect("vice-inquiry consequences should apply");
    assert_eq!(shifts.len(), 1);
    assert_eq!(shifts[0].audience, AudienceKind::Police);
    assert_eq!(shifts[0].dimension, ReputationDimension::Fear);
    assert_eq!(shifts[0].delta, authored);
    assert_eq!(
        resolve_score(
            &registry,
            &state.reputation,
            organization,
            AudienceKind::Police,
            ReputationDimension::Fear
        ),
        baseline + u8::try_from(authored).expect("authored fear delta must be non-negative")
    );

    // Non-criminal institutions hold no street reputation to move.
    let precinct = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Vice Test Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let shifts = apply_vice_inquiry_reputation_consequences(&registry, &mut state, precinct)
        .expect("consequence pass should succeed");
    assert!(
        shifts.is_empty(),
        "an institution cannot be scared of itself"
    );
    assert!(
        state
            .reputation
            .get_record(precinct, AudienceKind::Police)
            .is_none()
    );

    validate_invariants(&state);
}

#[test]
fn player_racket_vice_heat_surfaces_as_standing_report() {
    let (registry, mut state, organization) = make_state_with_player();

    apply_vice_inquiry_reputation_consequences(&registry, &mut state, organization)
        .expect("vice-inquiry consequences and feedback should apply");

    assert_eq!(standing_reports(&state, organization), 1);
    let report = state
        .reports()
        .reports_for(organization)
        .find(|report| report.kind() == ReportKind::Standing)
        .expect("standing report should persist");
    let summary = &report.entries()[0].summary;
    assert!(
        summary.starts_with("News of the rackets travels:")
            && summary.contains("police watch us more warily"),
        "the entry must name the racket heat and its consequence: {summary}"
    );
    validate_invariants(&state);
}
