//! Canonical reputation mutation, operation-consequence producers, and daily baseline decay.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{IdExhaustionError, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::operations::{OperationApproach, OperationExposureLevel, OperationObjectiveOutcome};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::reputation::{AudienceKind, ReputationDimension, ReputationRecord, ReputationState};
use crate::world::OrganizationKind;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReputationError {
    #[error("organization {0} does not exist or is inactive")]
    MissingOrganization(OrganizationId),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

/// The single canonical reputation mutation: applies one clamped delta to one dimension of
/// one audience's impression of one organization. Records are created at the authored
/// baseline on first touch; zero-delta calls leave state untouched.
pub fn apply_reputation_delta(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    audience: AudienceKind,
    dimension: ReputationDimension,
    delta: i8,
) -> Result<u8, ReputationError> {
    if state.world().get_organization(organization).is_none() {
        return Err(ReputationError::MissingOrganization(organization));
    }
    apply_delta(registry, state, organization, audience, dimension, delta)
}

fn apply_delta(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    audience: AudienceKind,
    dimension: ReputationDimension,
    delta: i8,
) -> Result<u8, ReputationError> {
    let current = resolve_score(
        registry,
        &state.reputation,
        organization,
        audience,
        dimension,
    );
    if delta == 0 {
        return Ok(current);
    }
    let proposed = i32::from(current) + i32::from(delta);
    let next = u8::try_from(proposed.clamp(0, 100))
        .expect("clamped reputation arithmetic stays inside the score range");
    let key = (organization, audience);
    if !state.reputation.records_contains_key(key) {
        let baseline = registry.reputation().baseline();
        let mut record = ReputationRecord::at_baseline(organization, audience, baseline);
        record.set_score(dimension, next);
        state.reputation.insert_record(record);
    } else {
        let record = state
            .reputation
            .record_mut(key)
            .expect("touched reputation record must exist");
        record.set_score(dimension, next);
    }
    Ok(next)
}

/// The effective score for an impression: the stored value where touched, otherwise the
/// authored baseline.
pub fn resolve_score(
    registry: &Registry,
    reputation: &ReputationState,
    organization: OrganizationId,
    audience: AudienceKind,
    dimension: ReputationDimension,
) -> u8 {
    reputation
        .get_record(organization, audience)
        .map(|record| record.score(dimension))
        .unwrap_or_else(|| registry.reputation().baseline())
}

/// One audience impression that an operation consequence actually moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppliedStandingShift {
    pub audience: AudienceKind,
    pub dimension: ReputationDimension,
    pub delta: i8,
}

/// Deterministic consequence pass over an operation that reached terminal resolution this
/// tick. Success builds underworld competence; witnessed exposure raises police fear;
/// violent approaches raise business fear. The exact clamped shifts and any required player
/// feedback report are planned before mutation, so a report-allocation failure cannot leave
/// player standing changed without its causal artifact.
pub(crate) fn apply_operation_reputation_consequences(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    approach: OperationApproach,
    objective_outcome: OperationObjectiveOutcome,
    exposure_level: OperationExposureLevel,
) -> Result<Vec<AppliedStandingShift>, ReputationError> {
    // Non-criminal organizations do not accumulate street reputations; their institutional
    // standing is modeled by their own domains rather than an underworld impression.
    let responsible = state
        .world
        .get_organization(organization)
        .ok_or(ReputationError::MissingOrganization(organization))?;
    if responsible.kind() != OrganizationKind::Criminal {
        return Ok(Vec::new());
    }
    let config = registry.reputation();
    let mut shifts = Vec::new();

    let competence_delta = match objective_outcome {
        OperationObjectiveOutcome::Achieved => config.achieved_underworld_competence(),
        OperationObjectiveOutcome::Partial => config.partial_underworld_competence(),
        OperationObjectiveOutcome::Failed => 0,
    };
    shifts.extend(resolve_shift(
        registry,
        state,
        organization,
        AudienceKind::Underworld,
        ReputationDimension::Competence,
        competence_delta,
    ));

    let police_fear = match exposure_level {
        OperationExposureLevel::None | OperationExposureLevel::Trace => 0,
        OperationExposureLevel::Witnessed => config.witnessed_exposure_police_fear(),
        OperationExposureLevel::Identifying => config.identifying_exposure_police_fear(),
    };
    shifts.extend(resolve_shift(
        registry,
        state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        police_fear,
    ));

    if approach == OperationApproach::Violent
        && !matches!(
            exposure_level,
            OperationExposureLevel::None | OperationExposureLevel::Trace
        )
    {
        shifts.extend(resolve_shift(
            registry,
            state,
            organization,
            AudienceKind::Businesses,
            ReputationDimension::Fear,
            config.violent_businesses_fear(),
        ));
    }
    commit_consequence_shifts(
        registry,
        state,
        organization,
        "Word travels after the job:",
        shifts,
    )
}

/// Deterministic consequence pass over a racket that drew a dedicated vice inquiry this
/// tick. A case built on the racket itself is at least as alarming to its owner as being
/// witnessed on a job: police fear rises through the single canonical delta path, which
/// throttles delegated expansion while it decays. Player feedback is committed atomically
/// with the shift through the same composition helper as operation consequences.
pub(crate) fn apply_vice_inquiry_reputation_consequences(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
) -> Result<Vec<AppliedStandingShift>, ReputationError> {
    let responsible = state
        .world
        .get_organization(organization)
        .ok_or(ReputationError::MissingOrganization(organization))?;
    if responsible.kind() != OrganizationKind::Criminal {
        return Ok(Vec::new());
    }
    let fear = registry.reputation().vice_inquiry_police_fear();
    let shifts = resolve_shift(
        registry,
        state,
        organization,
        AudienceKind::Police,
        ReputationDimension::Fear,
        fear,
    )
    .into_iter()
    .collect();
    commit_consequence_shifts(
        registry,
        state,
        organization,
        "News of the rackets travels:",
        shifts,
    )
}

/// Resolves one authored consequence without mutation and returns it only when the score
/// would actually move. A score already clamped at a rail did not move, and reporting
/// movement that did not happen would fabricate causal feedback about standing.
fn resolve_shift(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    audience: AudienceKind,
    dimension: ReputationDimension,
    delta: i8,
) -> Option<AppliedStandingShift> {
    if delta == 0 {
        return None;
    }
    let current = resolve_score(
        registry,
        &state.reputation,
        organization,
        audience,
        dimension,
    );
    let proposed = i32::from(current) + i32::from(delta);
    let next = u8::try_from(proposed.clamp(0, 100))
        .expect("clamped reputation arithmetic stays inside the score range");
    (next != current).then_some(AppliedStandingShift {
        audience,
        dimension,
        delta,
    })
}

/// Player-facing labels for the standing audiences, exhaustive so a new audience must be
/// named here before it can appear in a report.
fn audience_label(audience: AudienceKind) -> &'static str {
    match audience {
        AudienceKind::Underworld => "the underworld",
        AudienceKind::Police => "the police",
        AudienceKind::Businesses => "business owners",
        AudienceKind::Residents => "residents",
        AudienceKind::Political => "political figures",
        AudienceKind::Press => "the press",
    }
}

/// Player-facing labels for the standing dimensions, exhaustive so a new dimension must be
/// named here before it can appear in a report.
fn dimension_label(dimension: ReputationDimension) -> &'static str {
    match dimension {
        ReputationDimension::Fear => "fear of us",
        ReputationDimension::Reliability => "reliability in us",
        ReputationDimension::Competence => "opinion of our competence",
        ReputationDimension::Treachery => "suspicion of our treachery",
    }
}

/// Commits a resolved consequence set. For the player organization the Standing report is
/// validated and its ID budget is reserved before the first reputation write, making the
/// standing changes and their player-facing causality one atomic semantic operation. Rival
/// organizations move silently because their street standing is not free player information.
fn commit_consequence_shifts(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    lead_in: &str,
    shifts: Vec<AppliedStandingShift>,
) -> Result<Vec<AppliedStandingShift>, ReputationError> {
    let feedback = if shifts.is_empty() || state.player_organization() != Some(organization) {
        None
    } else {
        let report = validate_standing_feedback_report(state, organization, lead_in, &shifts)?;
        state.ids.reserve(IdKind::Report, 1)?;
        Some(report)
    };
    for shift in &shifts {
        apply_delta(
            registry,
            state,
            organization,
            shift.audience,
            shift.dimension,
            shift.delta,
        )
        .expect("resolved standing shift organization was validated before mutation");
    }
    if let Some(report) = feedback {
        report
            .commit(state)
            .expect("standing report ID was preflighted before reputation mutation");
    }
    Ok(shifts)
}

/// Builds player-facing causality for a resolved standing shift set. `lead_in` names the
/// activity that moved the impression (a job or a racket drawing heat).
fn validate_standing_feedback_report(
    state: &AppState,
    organization: OrganizationId,
    lead_in: &str,
    shifts: &[AppliedStandingShift],
) -> Result<ValidatedReport, ReportError> {
    debug_assert!(!shifts.is_empty());
    let mut summary = String::from(lead_in);
    for (index, shift) in shifts.iter().enumerate() {
        // Hand-written prose per produced pair where it adds nuance; every other pair reads
        // as proper text through the exhaustive labels instead of leaking debug names.
        let rising = shift.delta > 0;
        let clause = match (shift.audience, shift.dimension) {
            (AudienceKind::Underworld, ReputationDimension::Competence) => {
                if rising {
                    " the underworld rates our competence higher".to_owned()
                } else {
                    " the underworld rates our competence lower".to_owned()
                }
            }
            (AudienceKind::Police, ReputationDimension::Fear) => {
                if rising {
                    " the police watch us more warily".to_owned()
                } else {
                    " police wariness toward us eases".to_owned()
                }
            }
            (AudienceKind::Businesses, ReputationDimension::Fear) => {
                if rising {
                    " business owners grow warier of us".to_owned()
                } else {
                    " business owners relax around us".to_owned()
                }
            }
            // Exhaustive fallback: `audience_label` and `dimension_label` each match every
            // variant of their enum, so adding an audience or dimension fails to compile
            // here until its label is authored.
            (audience, dimension) => format!(
                " {} {} among {} {}",
                dimension_label(dimension),
                if rising { "rises" } else { "falls" },
                audience_label(audience),
                if rising { "slightly" } else { "somewhat" }
            ),
        };
        if index > 0 {
            summary.push(';');
        }
        summary.push_str(&clause);
    }
    summary.push('.');
    validate_record_report(
        state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Standing,
            title: "Street standing".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::Organization(organization)]),
                decision: None,
            }],
        },
    )
}

/// Day-boundary decay: every touched impression drifts one authored step toward the
/// baseline from both sides, so old events fade instead of ratcheting forever. Absent
/// records stay absent — decay never manufactures impressions. Runs on the payroll's day
/// boundary so all daily passes observe the same campaign-day rhythm.
pub(crate) fn apply_daily_reputation_decay(registry: &Registry, state: &mut AppState) -> usize {
    if !crate::core::time::is_day_boundary(state.now()) {
        return 0;
    }
    // Snapshot the touched impressions first: mutation goes through the canonical path,
    // which cannot run while the records map is borrowed for iteration.
    let touched: Vec<(OrganizationId, AudienceKind)> = state
        .reputation()
        .records()
        .map(|record| (record.organization(), record.audience()))
        .collect();
    let step = i8::try_from(i64::from(registry.reputation().daily_decay_step()))
        .expect("authored decay step fits i8");
    let baseline = registry.reputation().baseline();
    let mut adjusted = 0_usize;
    for (organization, audience) in touched {
        for dimension in crate::reputation::ALL_REPUTATION_DIMENSIONS {
            let current = resolve_score(
                registry,
                &state.reputation,
                organization,
                audience,
                dimension,
            );
            let current_i = i64::from(current);
            let drifted = if current_i > i64::from(baseline) {
                (current_i - i64::from(step)).max(i64::from(baseline))
            } else if current_i < i64::from(baseline) {
                (current_i + i64::from(step)).min(i64::from(baseline))
            } else {
                current_i
            };
            if drifted != current_i {
                let change = drifted - current_i;
                let change = i8::try_from(change).expect("one-step drift fits i8");
                apply_delta(registry, state, organization, audience, dimension, change)
                    .expect("decay touches only existing world organizations");
                adjusted += 1;
            }
        }
    }
    // A fully faded impression is indistinguishable from an absent one by design: erase it
    // so the sparse-record contract stays literal and state does not grow monotonically.
    state.reputation.remove_at_baseline(baseline);
    adjusted
}

impl ReputationRecord {
    pub(crate) fn at_baseline(
        organization: OrganizationId,
        audience: AudienceKind,
        baseline: u8,
    ) -> Self {
        Self {
            organization,
            audience,
            fear: baseline,
            reliability: baseline,
            competence: baseline,
            treachery: baseline,
        }
    }
}

impl ReputationState {
    pub(crate) fn records_contains_key(&self, key: (OrganizationId, AudienceKind)) -> bool {
        self.records.contains_key(&key)
    }

    pub(crate) fn record_mut(
        &mut self,
        key: (OrganizationId, AudienceKind),
    ) -> Option<&mut ReputationRecord> {
        self.records.get_mut(&key)
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(state.reputation.len(), 1);
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
        assert!(state.reputation.is_empty());
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
            .records_for_organization(organization)
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
        let snapshot = build_recruitment_relationship_snapshot(
            candidate,
            recruiter,
            Some(dimensions),
            Some(1),
        );

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
        let expected_gap =
            (gap * i64::from(definition.weights().organization_competence) / 100) as i16;
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
        assert_eq!(state.reputation.len(), 3);
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
        assert_eq!(state.reputation.len(), 0);
        assert_eq!(standing_reports(&state, organization), 0);
    }

    #[test]
    fn vice_inquiry_raises_owner_police_fear_through_the_canonical_path() {
        let (registry, mut state, organization) = make_state();
        let baseline = registry.reputation().baseline();
        let authored = registry.reputation().vice_inquiry_police_fear();
        assert!(authored > 0, "a dedicated inquiry must scare its owner");

        // A criminal racket owner's police fear rises by exactly the authored step.
        let shifts =
            apply_vice_inquiry_reputation_consequences(&registry, &mut state, organization)
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
}
