//! Autonomous detective-work scheduling policy.
//!
//! Direct schedule/cancel/resolve commands remain owned by the parent investigation-work
//! system. This child owns only the deterministic per-minute choice of which canonical work
//! command a free case investigator should receive.

use super::{
    InvestigationWorkError, scheduled_work_for_investigator, validate_schedule_investigation_work,
};
use crate::core::id::{CharacterId, EvidenceId, IdKind, InvestigationId, InvestigationWorkId};
use crate::core::state::AppState;
use crate::legal::{
    InvestigationRecord, InvestigationWorkDraft, InvestigationWorkFocus, InvestigationWorkKind,
};
use crate::registry::Registry;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InvestigationWorkSchedulingOutcome {
    pub(crate) evidence_reviews: Vec<InvestigationWorkId>,
    pub(crate) witness_interviews: Vec<InvestigationWorkId>,
}

/// Runs the authoritative autonomous scheduling policy over the active-case index once.
///
/// Evidence review has first claim on a free detective, matching the historical two-phase
/// ordering. Because one investigator may lead only one active case, this single pass is
/// equivalent to scanning every active case once for reviews and then scanning them all again
/// for interviews, while avoiding the duplicate per-minute traversal and allocation.
pub(crate) fn apply_investigation_work_scheduling(
    registry: &Registry,
    state: &mut AppState,
) -> Result<InvestigationWorkSchedulingOutcome, InvestigationWorkError> {
    let investigations: Vec<InvestigationId> = state
        .legal
        .active_investigations()
        .map(|investigation| investigation.id())
        .collect();
    let mut planned = Vec::new();

    for investigation_id in investigations {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("indexed active investigation must exist");
        let Some(investigator) = available_case_investigator(state, investigation) else {
            continue;
        };
        if scheduled_work_for_investigator(state, investigator).is_some() {
            continue;
        }
        if investigation.version() <= u32::MAX - 3
            && let Some(work) =
                plan_next_evidence_review(registry, state, investigation_id, investigator)?
        {
            planned.push((InvestigationWorkKind::EvidenceReview, work));
            continue;
        }
        if investigation.version() <= u32::MAX - 4
            && let Some(work) =
                plan_next_witness_interview(registry, state, investigation_id, investigator)?
        {
            planned.push((InvestigationWorkKind::WitnessInterview, work));
        }
    }

    // One canonical scheduling pass allocates at most one work record per staffed case. Every
    // plan above targets a distinct case/investigator pair, so preflight the complete ID budget
    // before the first schedule mutation instead of allowing a later capacity failure to leave
    // only the earlier case IDs scheduled.
    if state
        .ids
        .reserve(
            IdKind::InvestigationWork,
            u32::try_from(planned.len())
                .expect("persisted investigation count must fit the ID space"),
        )
        .is_err()
    {
        return Ok(InvestigationWorkSchedulingOutcome::default());
    }
    let mut outcome = InvestigationWorkSchedulingOutcome::default();
    for (kind, work) in planned {
        let id = work
            .commit(state)
            .expect("prevalidated investigation-work plan must remain current within one pass");
        match kind {
            InvestigationWorkKind::EvidenceReview => outcome.evidence_reviews.push(id),
            InvestigationWorkKind::WitnessInterview => outcome.witness_interviews.push(id),
        }
    }
    Ok(outcome)
}

/// Focused witness-only policy surface for tests that isolate interview ordering/headroom from
/// evidence-review priority. It uses the same cohort preflight/commit discipline as the canonical
/// combined scheduler rather than a sequential test-only mutation path.
#[cfg(test)]
pub(crate) fn apply_witness_interview_scheduling(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let candidates: Vec<InvestigationId> = state
        .legal
        .active_investigations()
        .map(|investigation| investigation.id())
        .collect();
    let mut planned = Vec::new();
    for investigation_id in candidates {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("indexed active investigation must exist");
        if investigation.version() > u32::MAX - 4 {
            continue;
        }
        let Some(investigator) = available_case_investigator(state, investigation) else {
            continue;
        };
        if scheduled_work_for_investigator(state, investigator).is_some() {
            continue;
        }
        if let Some(work) =
            plan_next_witness_interview(registry, state, investigation_id, investigator)?
        {
            planned.push(work);
        }
    }
    commit_planned_work_cohort(state, planned)
}

/// Focused review-only policy surface for tests that isolate evidence ordering/retry behavior.
/// The canonical combined scheduler remains the production tick path.
#[cfg(test)]
pub(crate) fn apply_evidence_review_scheduling(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let investigations: Vec<InvestigationId> = state
        .legal
        .active_investigations()
        .map(|investigation| investigation.id())
        .collect();
    let mut planned = Vec::new();
    for investigation_id in investigations {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("indexed active investigation must exist");
        if investigation.version() > u32::MAX - 3 {
            continue;
        }
        let Some(investigator) = available_case_investigator(state, investigation) else {
            continue;
        };
        if scheduled_work_for_investigator(state, investigator).is_some() {
            continue;
        }
        if let Some(work) =
            plan_next_evidence_review(registry, state, investigation_id, investigator)?
        {
            planned.push(work);
        }
    }
    commit_planned_work_cohort(state, planned)
}

#[cfg(test)]
fn commit_planned_work_cohort(
    state: &mut AppState,
    planned: Vec<super::ValidatedInvestigationWorkSchedule>,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    state.ids.reserve(
        IdKind::InvestigationWork,
        u32::try_from(planned.len()).expect("persisted investigation count must fit the ID space"),
    )?;
    Ok(planned
        .into_iter()
        .map(|work| {
            work.commit(state)
                .expect("prevalidated focused scheduling cohort must remain current")
        })
        .collect())
}

/// The investigator an autonomous casework scheduler may currently use. The lead is the single
/// modeled case seat. Detention pauses work rather than silently assigning a different
/// institution or character.
fn available_case_investigator(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Option<CharacterId> {
    investigation
        .lead_investigator()
        .filter(|lead| state.legal.active_arrest_for_character(*lead).is_none())
}

fn plan_next_evidence_review(
    registry: &Registry,
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
) -> Result<Option<super::ValidatedInvestigationWorkSchedule>, InvestigationWorkError> {
    let record = state
        .legal
        .get_investigation(investigation)
        .expect("indexed active investigation must exist");
    let Some(source) = next_unattempted_review_source(state, record)? else {
        return Ok(None);
    };
    match validate_schedule_investigation_work(
        registry,
        state,
        InvestigationWorkDraft {
            investigation,
            investigator,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(source),
        },
    ) {
        Ok(work) => Ok(Some(work)),
        // Direct scheduling should report the finite-clock error. Autonomous scheduling simply
        // has no useful task to create when that task cannot finish inside the simulation horizon.
        Err(InvestigationWorkError::SimulationTimeOverflow) => Ok(None),
        Err(error) => Err(error),
    }
}

fn plan_next_witness_interview(
    registry: &Registry,
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
) -> Result<Option<super::ValidatedInvestigationWorkSchedule>, InvestigationWorkError> {
    let mut witnesses: Vec<_> = state
        .legal
        .case_witnesses_for_investigation(investigation)
        .filter(|witness| witness.statements().is_empty())
        .filter(|witness| witness.version() <= u32::MAX - 2)
        .filter(|witness| {
            !crate::legal::witness_system::case_witness_is_case_subject(state, witness)
        })
        .filter(|witness| {
            witness.interview_attempts() < registry.legal().witness_interview_attempt_limit()
        })
        .map(|witness| (witness.interview_attempts(), witness.id()))
        .collect();
    witnesses.sort_unstable();
    for (_, case_witness) in witnesses {
        let focus = InvestigationWorkFocus::witness(case_witness);
        if state
            .legal
            .scheduled_work_for_focus(
                investigation,
                InvestigationWorkKind::WitnessInterview,
                focus,
            )
            .is_some()
        {
            continue;
        }
        match validate_schedule_investigation_work(
            registry,
            state,
            InvestigationWorkDraft {
                investigation,
                investigator,
                kind: InvestigationWorkKind::WitnessInterview,
                focus,
            },
        ) {
            Ok(work) => return Ok(Some(work)),
            Err(InvestigationWorkError::SimulationTimeOverflow) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// Returns the oldest case-owned reviewable evidence that has not received a real autonomous
/// review attempt. Scheduled and completed work consume the source; cancelled work does not.
/// Discovery time is substantive age; ID breaks exact-time ties.
fn next_unattempted_review_source(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<Option<EvidenceId>, InvestigationWorkError> {
    Ok(state
        .legal
        .next_unattempted_reviewable_evidence(investigation.id()))
}
