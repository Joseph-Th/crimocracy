//! Autonomous detective-work scheduling policy.
//!
//! Direct schedule/cancel/resolve commands remain owned by the parent investigation-work
//! system. This child owns only the deterministic per-minute choice of which canonical work
//! command a free case investigator should receive.

use super::{
    InvestigationWorkError, scheduled_work_for_investigator, validate_schedule_investigation_work,
};
use crate::core::id::{CharacterId, IdKind, InvestigationId, InvestigationWorkId};
use crate::core::state::AppState;
use crate::legal::{
    InvestigationRecord, InvestigationWorkDraft, InvestigationWorkFocus, InvestigationWorkKind,
    WitnessCooperation,
};
use crate::registry::Registry;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InvestigationWorkSchedulingOutcome {
    pub(crate) evidence_reviews: Vec<InvestigationWorkId>,
    pub(crate) witness_interviews: Vec<InvestigationWorkId>,
}

/// Runs the authoritative autonomous scheduling policy over the active-case index once.
///
/// Evidence review has first claim on a free detective. Because one investigator may lead
/// only one active case, planning both work kinds in a single read-only pass preserves that
/// priority without traversing the active-case index twice.
pub(crate) fn apply_investigation_work_scheduling(
    registry: &Registry,
    state: &mut AppState,
) -> Result<InvestigationWorkSchedulingOutcome, InvestigationWorkError> {
    let mut planned = Vec::new();

    // All scheduling is planned before any mutation, so read the indexed case records directly.
    // Collecting IDs and looking each one up again allocated every simulation minute for no gain.
    for investigation in state.legal.active_investigations() {
        let investigation_id = investigation.id();
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
    let Some(source) = state
        .legal
        .next_unattempted_reviewable_evidence(investigation)
    else {
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
        .map(|witness| {
            (
                witness.interview_attempts(),
                witness_cooperation_priority(witness.cooperation()),
                witness.registered_at(),
                witness.id(),
            )
        })
        .collect();
    witnesses.sort_unstable();
    for (_, _, _, case_witness) in witnesses {
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

const fn witness_cooperation_priority(cooperation: WitnessCooperation) -> u8 {
    match cooperation {
        WitnessCooperation::Cooperative => 0,
        WitnessCooperation::Reluctant => 1,
        WitnessCooperation::Hostile => 2,
    }
}
