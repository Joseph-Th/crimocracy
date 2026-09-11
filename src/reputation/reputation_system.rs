//! Canonical reputation mutation, operation-consequence producers, and daily baseline decay.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{IdExhaustionError, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::operations::{OperationApproach, OperationExposureLevel, OperationObjectiveOutcome};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::reputation::{
    AudienceKind, ReputationDimension, ReputationRecord, ReputationScore, ReputationState,
};
use crate::world::OrganizationKind;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReputationError {
    #[error("organization {0} does not exist")]
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
    if next == current {
        return Ok(current);
    }
    let key = (organization, audience);
    if !state.reputation.records_contains_key(key) {
        let baseline = registry.reputation().baseline();
        let mut record =
            ReputationRecord::at_baseline(organization, audience, baseline, state.now());
        record.set_score(dimension, next, state.now());
        state.reputation.insert_record(record);
    } else {
        let changed_at = state.now();
        let record = state
            .reputation
            .record_mut(key)
            .expect("touched reputation record must exist");
        record.set_score(dimension, next, changed_at);
    }
    state
        .reputation
        .remove_if_at_baseline(key, registry.reputation().baseline());
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
    let applied_delta =
        i8::try_from(i16::from(next) - i16::from(current)).expect("bounded score delta fits i8");
    (next != current).then_some(AppliedStandingShift {
        audience,
        dimension,
        delta: applied_delta,
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

/// Day-boundary decay: every touched dimension at least one campaign day old drifts one authored
/// step toward the baseline from both sides, so old events fade instead of ratcheting forever.
/// Fresh dimensions wait until a later day boundary rather than losing impact simply because
/// their event happened shortly before, or earlier within, the current boundary tick. Absent
/// records stay absent; decay never manufactures impressions.
pub(crate) fn apply_daily_reputation_decay(registry: &Registry, state: &mut AppState) -> usize {
    if !crate::core::time::is_day_boundary(state.now()) {
        return 0;
    }
    // Snapshot the touched impressions first: mutation goes through the canonical path,
    // which cannot run while the records map is borrowed for iteration.
    let touched: Vec<ReputationRecord> = state.reputation().records().copied().collect();
    let step = i8::try_from(i64::from(registry.reputation().daily_decay_step()))
        .expect("authored decay step fits i8");
    let baseline = registry.reputation().baseline();
    let mut adjusted = 0_usize;
    for record in touched {
        let organization = record.organization();
        let audience = record.audience();
        for dimension in crate::reputation::ALL_REPUTATION_DIMENSIONS {
            let current = record.score(dimension);
            let changed_at = record.changed_at(dimension);
            let age = state
                .now()
                .as_minutes()
                .checked_sub(changed_at.as_minutes())
                .expect("reputation chronology must not place a change in the future");
            if age < crate::core::time::DAY_MINUTES {
                continue;
            }
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
    adjusted
}

impl ReputationRecord {
    fn at_baseline(
        organization: OrganizationId,
        audience: AudienceKind,
        baseline: u8,
        changed_at: crate::core::time::SimTime,
    ) -> Self {
        let score = ReputationScore::at(baseline, changed_at);
        Self {
            organization,
            audience,
            fear: score,
            reliability: score,
            competence: score,
            treachery: score,
        }
    }
}

impl ReputationState {
    fn records_contains_key(&self, key: (OrganizationId, AudienceKind)) -> bool {
        self.records.contains_key(&key)
    }

    fn record_mut(&mut self, key: (OrganizationId, AudienceKind)) -> Option<&mut ReputationRecord> {
        self.records.get_mut(&key)
    }
}

#[cfg(test)]
mod tests;
