//! Autonomous detective-work scheduling policy.
//!
//! Direct schedule/cancel/resolve commands remain owned by the parent investigation-work
//! system. This child owns only the deterministic per-minute choice of which canonical work
//! command a free case investigator should receive.

use super::{
    InvestigationWorkError, is_reviewable_evidence_kind, scheduled_work_for_investigator,
    validate_schedule_investigation_work,
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
    state.ids.reserve(
        IdKind::InvestigationWork,
        u32::try_from(planned.len()).expect("persisted investigation count must fit the ID space"),
    )?;
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

/// Schedules witness interviews for staffed active cases whose registered witnesses have not
/// given a statement yet. Kept as a focused owner surface for tests and explicit callers; the
/// canonical tick uses the combined scheduler so it does not rescan active cases.
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
    let mut scheduled = Vec::new();
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
            scheduled.push(work.commit(state)?);
        }
    }
    Ok(scheduled)
}

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
    let mut scheduled = Vec::new();
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
            scheduled.push(work.commit(state)?);
        }
    }
    Ok(scheduled)
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
    let work = validate_schedule_investigation_work(
        registry,
        state,
        InvestigationWorkDraft {
            investigation,
            investigator,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(source),
        },
    )?;
    Ok(Some(work))
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
        let work = validate_schedule_investigation_work(
            registry,
            state,
            InvestigationWorkDraft {
                investigation,
                investigator,
                kind: InvestigationWorkKind::WitnessInterview,
                focus,
            },
        )?;
        return Ok(Some(work));
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
    let mut oldest = None;
    for evidence_id in investigation.evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if !is_reviewable_evidence_kind(evidence.kind())
            || state.legal.evidence_review_attempt(evidence.id()).is_some()
        {
            continue;
        }
        let candidate = (evidence.discovered_at(), evidence.id());
        if oldest.is_none_or(|current| candidate < current) {
            oldest = Some(candidate);
        }
    }
    Ok(oldest.map(|(_, evidence)| evidence))
}
