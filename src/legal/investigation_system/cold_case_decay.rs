//! Autonomous cold-case lifecycle policy layered over canonical investigation transitions.

use super::{InvestigationError, InvestigationTransition, validate_transition_investigation};
use crate::core::id::{IdKind, InvestigationId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};

/// Deterministically shelves origin-linked investigations whose owning authority has been
/// institutionally inactive for the authored cold window.
///
/// Cold cases are handled through the canonical lifecycle transition. Scheduled work and live
/// custody under the same file defer decay. Work that appeared between the deadline index scan and
/// this call simply keeps the case active and decay retries on the refreshed deadline. Only cases
/// carrying a case-origination link (an operation or enterprise whose exposure opened them) are
/// eligible: institution-authored casework keeps its lifecycle until an explicit staff decision.
/// Identifying a concrete character does not manufacture perpetual institutional activity. Live
/// same-case custody temporarily blocks shelving without resetting the inactivity clock; once
/// custody ends, a file whose authored inactivity window has elapsed shelves like any other
/// originated case. Shelving releases the case's investigators, and a later
/// incident sharing the shelf's subject matter resumes the same file (`find_resumable_shelf`)
/// rather than starting from silence.
pub(crate) fn apply_cold_case_decay(
    state: &mut AppState,
    cold_case_window: SimDuration,
) -> Result<Vec<InvestigationId>, InvestigationError> {
    let now_minutes = state.now().as_minutes();
    let window_minutes = u64::from(cold_case_window.as_minutes());
    // Before a complete inactivity window has elapsed, no timestamp can possibly be cold.
    // Saturating subtraction would incorrectly turn that interval into threshold minute zero,
    // causing cases opened at campaign start to qualify on the first tick because the activity
    // index query is inclusive.
    if now_minutes < window_minutes {
        return Ok(Vec::new());
    }
    let threshold_minutes = now_minutes - window_minutes;
    let candidates = state
        .legal
        .find_active_cases_inactive_since(SimTime::from_minutes(threshold_minutes));
    let mut planned_suspensions = Vec::new();
    for investigation in candidates {
        let record = state
            .legal
            .get_investigation(investigation)
            .expect("cold-case candidate must still exist");
        if record.origin().is_none() {
            continue;
        }
        // A current-version save may legitimately contain an active case at the finite version
        // rail. Direct lifecycle commands still fail closed with VersionCapacity, but autonomous
        // decay cannot ever make that record representably suspend. Leave it active and continue
        // processing other cold files instead of bricking every future simulation tick.
        if record.version() == u32::MAX {
            continue;
        }
        // Scheduled work is a modeled reason for an apparently cold case to remain active:
        // the institution has already committed resources even if the old inactivity deadline
        // was present in the index snapshot. Defer explicitly; every other transition failure
        // below is exceptional and must surface.
        if record
            .lead_investigator()
            .and_then(|lead| state.legal.scheduled_work_for_investigator(lead))
            .is_some_and(|work| work.investigation() == investigation)
        {
            continue;
        }
        // Live custody under this file is not a terminal investigative outcome. Arrest is not
        // conviction, custody is bounded, and prosecution may continue after release. The case
        // therefore waits for custody to resolve instead of being auto-closed or poisoning the
        // rest of the due batch. Custody under an unrelated file is intentionally irrelevant.
        if state
            .legal
            .arrests_for_investigation(investigation)
            .any(|arrest| arrest.status() == crate::legal::ArrestStatus::Detained)
        {
            continue;
        }
        let prepared = validate_transition_investigation(
            state,
            investigation,
            InvestigationTransition::Suspend,
        )?
        .prepare_current(state)?;
        planned_suspensions.push((investigation, prepared));
    }

    // Cold decay is one due-set lifecycle pass. Preflight the complete information budget before
    // changing the first case so allocator exhaustion cannot shelf only the earliest IDs and leave
    // the rest active behind a failed tick.
    let information_count = u32::try_from(
        planned_suspensions
            .iter()
            .filter(|(_, prepared)| prepared.knowledge.is_some())
            .count(),
    )
    .expect("persisted investigation count must fit the u32 ID space");
    state.ids.reserve(IdKind::Information, information_count)?;

    let mut suspended = Vec::with_capacity(planned_suspensions.len());
    for (investigation, prepared) in planned_suspensions {
        prepared.commit_preflighted(state);
        suspended.push(investigation);
    }
    Ok(suspended)
}
