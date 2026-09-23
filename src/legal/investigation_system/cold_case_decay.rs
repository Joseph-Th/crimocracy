//! Autonomous cold-case lifecycle policy layered over canonical investigation transitions.

use super::{
    InvestigationError, InvestigationTransition, evidence_is_actionable_case_lead,
    validate_transition_investigation,
};
use crate::core::id::{CharacterId, IdKind, InvestigationId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use std::collections::BTreeSet;

/// Deterministically shelves origin-linked investigations whose owning authority has been
/// institutionally inactive for the authored cold window.
///
/// Cold cases are handled through the canonical lifecycle transition. Scheduled work defers decay,
/// suspension revalidates the no-active-arrest rule, and a case whose actionable subjects are all
/// detained takes the explicit close branch below. Work that appeared between the deadline index
/// scan and this call simply keeps the case active and decay retries on the refreshed deadline.
/// Only cases carrying a case-origination link (an operation or enterprise
/// whose exposure opened them) are eligible: institution-authored casework keeps its lifecycle
/// until an explicit staff decision. Identifying a concrete character does not manufacture
/// perpetual institutional activity: if the case produces no further work or evidence for the
/// full cold window, it shelves like any other originated file. A case whose every actionable
/// identified subject is already detained closes instead because custody cleared its live work.
/// Shelving releases the case's investigators, and a later incident sharing the shelf's subject
/// matter resumes the same file (`find_resumable_shelf`) rather than starting from silence.
pub(crate) fn apply_cold_case_decay(
    state: &mut AppState,
    cold_case_window: SimDuration,
) -> Result<ColdCaseDecayOutcome, InvestigationError> {
    let now_minutes = state.now().as_minutes();
    let window_minutes = u64::from(cold_case_window.as_minutes());
    // Before a complete inactivity window has elapsed, no timestamp can possibly be cold.
    // Saturating subtraction would incorrectly turn that interval into threshold minute zero,
    // causing cases opened at campaign start to qualify on the first tick because the activity
    // index query is inclusive.
    if now_minutes < window_minutes {
        return Ok(ColdCaseDecayOutcome {
            suspended: Vec::new(),
            closed: Vec::new(),
        });
    }
    let threshold_minutes = now_minutes - window_minutes;
    let candidates = state
        .legal
        .find_active_cases_inactive_since(SimTime::from_minutes(threshold_minutes));
    let mut transitions = Vec::new();
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
        // decay cannot ever make that record representably suspend or close. Leave it active and
        // continue processing other cold files instead of bricking every future simulation tick.
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
        // An originated case whose every actionable identified subject is in custody is fully
        // worked: the institutional trail ends, so the case closes rather than shelving while
        // its subjects are held. An at-large lead does not defeat the inactivity rule forever;
        // without new evidence or work for the authored window, the file shelves and releases
        // its investigator seat until a later incident reactivates it.
        //
        // Custody is scoped to this case's own arrests: a subject detained under an unrelated
        // file does not clear this investigation's live work. A case with live custody of its
        // own cannot suspend either, so it defers to a later pass instead of aborting the
        // whole batch on one detained file.
        let detained_here: BTreeSet<_> = state
            .legal
            .arrests_for_investigation(investigation)
            .filter(|arrest| arrest.status() == crate::legal::ArrestStatus::Detained)
            .map(|arrest| arrest.character())
            .collect();
        let identified_subjects = actionable_character_subjects(state, record);
        if !identified_subjects.is_empty()
            && identified_subjects
                .iter()
                .all(|character| detained_here.contains(character))
        {
            let transition = InvestigationTransition::Close;
            let prepared = validate_transition_investigation(state, investigation, transition)?
                .prepare_current(state)?;
            transitions.push((investigation, transition, prepared));
            continue;
        }
        if !detained_here.is_empty() {
            // Live custody under this file blocks suspension, so the case waits for custody
            // to resolve instead of aborting the whole decay batch on one detained file.
            continue;
        }
        let transition = InvestigationTransition::Suspend;
        let prepared = validate_transition_investigation(state, investigation, transition)?
            .prepare_current(state)?;
        transitions.push((investigation, transition, prepared));
    }

    // Cold decay is one due-set lifecycle pass. Preflight the complete information budget before
    // changing the first case so allocator exhaustion cannot shelf/close only the earliest IDs
    // and leave the rest active behind a failed tick.
    let information_count = u32::try_from(
        transitions
            .iter()
            .filter(|(_, _, prepared)| prepared.knowledge.is_some())
            .count(),
    )
    .expect("persisted investigation count must fit the u32 ID space");
    state.ids.reserve(IdKind::Information, information_count)?;

    let mut suspended = Vec::new();
    let mut closed = Vec::new();
    for (investigation, transition, prepared) in transitions {
        prepared.commit_preflighted(state);
        match transition {
            InvestigationTransition::Suspend => suspended.push(investigation),
            InvestigationTransition::Close => closed.push(investigation),
            InvestigationTransition::Resume => {
                unreachable!("cold-case decay only plans suspend or close transitions")
            }
        }
    }
    Ok(ColdCaseDecayOutcome { suspended, closed })
}

fn actionable_character_subjects(
    state: &AppState,
    investigation: &crate::legal::InvestigationRecord,
) -> Vec<CharacterId> {
    investigation
        .evidence()
        .iter()
        .filter_map(|evidence_id| {
            let evidence = state
                .legal
                .get_evidence(*evidence_id)
                .expect("investigation evidence index must reference persisted evidence");
            if !evidence_is_actionable_case_lead(evidence) {
                return None;
            }
            evidence.subject().as_character()
        })
        .collect()
}

/// Cold-window decay results, split so observers can distinguish shelved cases from cases
/// fully closed because every identified subject is already in custody.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColdCaseDecayOutcome {
    pub suspended: Vec<InvestigationId>,
    pub closed: Vec<InvestigationId>,
}
