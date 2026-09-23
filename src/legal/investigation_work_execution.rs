//! Scheduled detective work; this sibling system derives new case evidence only from evidence already owned by the investigation.

use crate::core::id::CaseWitnessId;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, InvestigationId, InvestigationWorkId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
use crate::legal::{
    EvidenceKind, InvestigationStatus, InvestigationWorkCancellation,
    InvestigationWorkCancellationReason, InvestigationWorkDraft, InvestigationWorkIdentity,
    InvestigationWorkKind, InvestigationWorkRecord, InvestigationWorkRuntime,
    InvestigationWorkStatus,
};
use crate::registry::Registry;
use crate::world::CapabilityKind;
use std::collections::BTreeSet;
use thiserror::Error;

mod resolution;
pub use resolution::{
    InvestigationWorkRandomness, InvestigationWorkResolutionPlan,
    ValidatedInvestigationWorkResolution, decide_investigation_work_resolution,
    validate_investigation_work_resolution_plan,
};
pub(crate) use resolution::{
    find_due_scheduled_investigation_work, resolve_improved_evidence_reliability,
    validate_historical_work_factors,
};

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvestigationWorkError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("investigator {0} does not exist")]
    MissingInvestigator(CharacterId),
    #[error("investigator {investigator} is not assigned to investigation {investigation}")]
    InvestigatorNotAssigned {
        investigation: InvestigationId,
        investigator: CharacterId,
    },
    #[error("investigator {investigator} is detained under arrest {arrest}")]
    DetainedInvestigator {
        investigator: CharacterId,
        arrest: ArrestId,
    },
    #[error("investigator {0} has no Investigation capability")]
    MissingInvestigationCapability(CharacterId),
    #[error("investigation work focus must match the work kind")]
    InvalidFocus,
    #[error("case witness {witness} already gave a statement and needs no further interview")]
    WitnessAlreadyStatemented { witness: CaseWitnessId },
    #[error(
        "case witness {witness} is now case subject {character} and cannot be interviewed as a witness"
    )]
    WitnessIsCaseSubject {
        witness: CaseWitnessId,
        character: CharacterId,
    },
    #[error(
        "case witness {witness} has exhausted the interview attempt limit ({attempts}/{limit})"
    )]
    WitnessInterviewLimitReached {
        witness: CaseWitnessId,
        attempts: u8,
        limit: u8,
    },
    #[error("evidence {evidence} has already been reviewed as evidence {derived}")]
    EvidenceAlreadyReviewed {
        evidence: EvidenceId,
        derived: EvidenceId,
    },
    #[error("evidence {evidence} already consumed review attempt {work}")]
    EvidenceReviewAlreadyAttempted {
        evidence: EvidenceId,
        work: InvestigationWorkId,
    },
    #[error("scheduled investigation work {work} already covers this case focus")]
    DuplicateScheduledWork { work: InvestigationWorkId },
    #[error("investigator {investigator} already has scheduled investigation work {work}")]
    InvestigatorBusy {
        investigator: CharacterId,
        work: InvestigationWorkId,
    },
    #[error(
        "investigation {investigation} changed after work validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "investigator {investigator} changed after work validation; expected version {expected}, found {found}"
    )]
    StaleInvestigator {
        investigator: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("investigation work {0} does not exist")]
    MissingWork(InvestigationWorkId),
    #[error("investigation work {0} is not scheduled")]
    WorkNotScheduled(InvestigationWorkId),
    #[error("investigation work {work} is not due until {due_at:?}")]
    WorkNotDue {
        work: InvestigationWorkId,
        due_at: SimTime,
    },
    #[error(
        "investigation work {work} changed after resolution planning; expected version {expected}, found {found}"
    )]
    StaleWork {
        work: InvestigationWorkId,
        expected: u32,
        found: u32,
    },
    #[error("resolution context for investigation work {work} changed after resolution planning")]
    StaleResolutionContext { work: InvestigationWorkId },
    #[error(
        "investigation work resolution was planned at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error("investigation work variance {variance} exceeds authored limit {limit}")]
    VarianceOutOfRange { variance: i8, limit: u8 },
    #[error("investigation work scheduling exceeds the representable simulation clock")]
    SimulationTimeOverflow,
    #[error("investigation work source evidence {0} no longer belongs to the case")]
    InvalidSourceEvidence(EvidenceId),
    #[error("witness interview for work {work} could not record a statement: {error}")]
    InterviewStatementFailed {
        work: InvestigationWorkId,
        error: crate::legal::witness_system::WitnessError,
    },
    #[error("case witness {witness} interview-attempt counter is exhausted")]
    WitnessInterviewAttemptCapacity { witness: CaseWitnessId },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub struct ValidatedInvestigationWorkSchedule {
    draft: InvestigationWorkDraft,
    source_evidence: BTreeSet<EvidenceId>,
    expected_investigation_version: u32,
    expected_investigator_version: u32,
    duration_minutes: u32,
}

impl ValidatedInvestigationWorkSchedule {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<InvestigationWorkId, InvestigationWorkError> {
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .ok_or(InvestigationWorkError::MissingInvestigation(
                self.draft.investigation,
            ))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationWorkError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        ensure_work_lifecycle_capacity(state, self.draft)?;
        let investigator = state.world.get_character(self.draft.investigator).ok_or(
            InvestigationWorkError::MissingInvestigator(self.draft.investigator),
        )?;
        if investigator.version() != self.expected_investigator_version {
            return Err(InvestigationWorkError::StaleInvestigator {
                investigator: self.draft.investigator,
                expected: self.expected_investigator_version,
                found: investigator.version(),
            });
        }
        validate_case_and_investigator(state, self.draft.investigation, self.draft.investigator)?;
        validate_no_duplicate_work(state, self.draft)?;
        validate_investigator_capacity(state, self.draft.investigator)?;
        // The investigation version snapshot above is authoritative for evidence-set
        // staleness: every evidence mutation bumps the investigation version, so no separate
        // source-set comparison is needed (and one could not report a meaningful
        // expected/found version pair).

        let scheduled_at = state.now();
        let due_at = scheduled_at
            .checked_add(crate::core::time::SimDuration::from_minutes(
                self.duration_minutes,
            ))
            .ok_or(InvestigationWorkError::SimulationTimeOverflow)?;
        let id = state.ids.next_investigation_work()?;
        state
            .legal
            .insert_investigation_work(InvestigationWorkRecord {
                identity: InvestigationWorkIdentity {
                    id,
                    investigation: self.draft.investigation,
                    investigator: self.draft.investigator,
                    kind: self.draft.kind,
                    focus: self.draft.focus,
                },
                source_evidence: self.source_evidence,
                runtime: InvestigationWorkRuntime {
                    scheduled_at,
                    due_at,
                    status: InvestigationWorkStatus::Scheduled,
                    resolution: None,
                    cancellation: None,
                    version: 1,
                },
            });
        Ok(id)
    }
}

#[derive(Debug)]
pub(crate) struct ValidatedInvestigationWorkCancellation {
    work: InvestigationWorkId,
    investigator: CharacterId,
    expected_version: u32,
    expected_investigation_version: u32,
    cancelled_at: SimTime,
}

impl ValidatedInvestigationWorkCancellation {
    pub(crate) fn work(&self) -> InvestigationWorkId {
        self.work
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), InvestigationWorkError> {
        let work = state
            .legal
            .get_investigation_work(self.work)
            .ok_or(InvestigationWorkError::MissingWork(self.work))?;
        if work.version() != self.expected_version {
            return Err(InvestigationWorkError::StaleWork {
                work: self.work,
                expected: self.expected_version,
                found: work.version(),
            });
        }
        if work.status() != InvestigationWorkStatus::Scheduled {
            return Err(InvestigationWorkError::WorkNotScheduled(self.work));
        }
        ensure_version_can_advance(work.version(), "investigation work")?;
        let investigation = state.legal.get_investigation(work.investigation()).ok_or(
            InvestigationWorkError::MissingInvestigation(work.investigation()),
        )?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationWorkError::StaleInvestigation {
                investigation: investigation.id(),
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        ensure_version_can_advance(investigation.version(), "investigation")?;
        if work.investigator() != self.investigator {
            return Err(InvestigationWorkError::StaleResolutionContext { work: self.work });
        }
        crate::core::time::ensure_time_current(state.now(), self.cancelled_at)
            .map_err(|_| InvestigationWorkError::StaleResolutionContext { work: self.work })?;
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState, arrest: ArrestId) {
        state.legal.set_investigation_work_cancellation(
            self.work,
            InvestigationWorkCancellation {
                cancelled_at: self.cancelled_at,
                reason: InvestigationWorkCancellationReason::InvestigatorDetained(arrest),
            },
        );
    }
}

pub(crate) fn validate_cancel_investigation_work_for_detention(
    state: &AppState,
    investigator: CharacterId,
) -> Result<Option<ValidatedInvestigationWorkCancellation>, InvestigationWorkError> {
    let Some(work_id) = scheduled_work_for_investigator(state, investigator) else {
        return Ok(None);
    };
    let work = state
        .legal
        .get_investigation_work(work_id)
        .ok_or(InvestigationWorkError::MissingWork(work_id))?;
    ensure_version_can_advance(work.version(), "investigation work")?;
    let investigation = state.legal.get_investigation(work.investigation()).ok_or(
        InvestigationWorkError::MissingInvestigation(work.investigation()),
    )?;
    ensure_version_can_advance(investigation.version(), "investigation")?;
    Ok(Some(ValidatedInvestigationWorkCancellation {
        work: work_id,
        investigator,
        expected_version: work.version(),
        expected_investigation_version: investigation.version(),
        cancelled_at: state.now(),
    }))
}

pub fn validate_schedule_investigation_work(
    registry: &Registry,
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<ValidatedInvestigationWorkSchedule, InvestigationWorkError> {
    validate_case_and_investigator(state, draft.investigation, draft.investigator)?;
    validate_no_duplicate_work(state, draft)?;
    validate_investigator_capacity(state, draft.investigator)?;
    let source_evidence = resolve_work_sources(registry, state, draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must still exist");
    ensure_work_lifecycle_capacity(state, draft)?;
    let investigator = state
        .world
        .get_character(draft.investigator)
        .expect("validated investigator must still exist");
    let duration = registry.get_investigation_work(draft.kind).duration();
    state
        .now()
        .checked_add(duration)
        .ok_or(InvestigationWorkError::SimulationTimeOverflow)?;
    Ok(ValidatedInvestigationWorkSchedule {
        draft,
        source_evidence,
        expected_investigation_version: investigation.version(),
        expected_investigator_version: investigator.version(),
        duration_minutes: duration.as_minutes(),
    })
}

/// Scheduling is only useful if every authored resolution branch remains representable. One
/// schedule advances the case immediately; evidence review can then need two more case revisions
/// (derived evidence + work completion), while a connected witness interview can need three
/// (testimony evidence + statement + work completion). Interviews also advance the witness twice.
/// Reserving that headroom here prevents canonical scheduling from creating due work that can
/// never resolve at the finite version rail.
fn ensure_work_lifecycle_capacity(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<(), InvestigationWorkError> {
    let investigation = state.legal.get_investigation(draft.investigation).ok_or(
        InvestigationWorkError::MissingInvestigation(draft.investigation),
    )?;
    let investigation_advances = match draft.kind {
        InvestigationWorkKind::EvidenceReview => 3,
        InvestigationWorkKind::WitnessInterview => 4,
    };
    ensure_version_can_advance_by(
        investigation.version(),
        investigation_advances,
        "investigation",
    )?;
    if draft.kind == InvestigationWorkKind::WitnessInterview {
        let witness = draft
            .focus
            .witness_id()
            .and_then(|witness| state.legal.get_case_witness(witness))
            .ok_or(InvestigationWorkError::InvalidFocus)?;
        ensure_version_can_advance_by(witness.version(), 2, "case witness")?;
    }
    Ok(())
}

/// Remaining investigation-version budget owned by already-scheduled detective work. External
/// case mutations must preserve this headroom or they can turn valid scheduled work into a due
/// record that can never complete. The schedule mutation itself already happened, so only the
/// worst-case resolution revisions remain here.
pub(crate) fn scheduled_work_investigation_headroom(
    state: &AppState,
    investigation: InvestigationId,
    excluding: Option<InvestigationWorkId>,
) -> u32 {
    state
        .legal
        .work_for_investigation(investigation)
        .filter(|work| work.status() == InvestigationWorkStatus::Scheduled)
        .filter(|work| Some(work.id()) != excluding)
        .map(|work| match work.kind() {
            InvestigationWorkKind::EvidenceReview => 2,
            InvestigationWorkKind::WitnessInterview => 3,
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn ensure_external_investigation_mutation_capacity(
    state: &AppState,
    investigation: InvestigationId,
    immediate_advances: u32,
    excluding: Option<InvestigationWorkId>,
) -> Result<(), VersionCapacityError> {
    let record = state
        .legal
        .get_investigation(investigation)
        .expect("case mutation capacity must reference a live investigation");
    let total = immediate_advances
        .checked_add(scheduled_work_investigation_headroom(
            state,
            investigation,
            excluding,
        ))
        .ok_or_else(|| VersionCapacityError::new("investigation"))?;
    ensure_version_can_advance_by(record.version(), total, "investigation")
}

pub(crate) fn ensure_external_case_witness_mutation_capacity(
    state: &AppState,
    case_witness: CaseWitnessId,
    immediate_advances: u32,
    excluding: Option<InvestigationWorkId>,
) -> Result<(), VersionCapacityError> {
    let witness = state
        .legal
        .get_case_witness(case_witness)
        .expect("witness mutation capacity must reference a live case witness");
    let scheduled_headroom = state
        .legal
        .scheduled_work_for_focus(
            witness.investigation(),
            InvestigationWorkKind::WitnessInterview,
            crate::legal::InvestigationWorkFocus::witness(case_witness),
        )
        .filter(|work| Some(work.id()) != excluding)
        .map_or(0, |_| 2);
    let total = immediate_advances
        .checked_add(scheduled_headroom)
        .ok_or_else(|| VersionCapacityError::new("case witness"))?;
    ensure_version_can_advance_by(witness.version(), total, "case witness")
}

/// Actionable evidence can itself invalidate the one scheduled work item whose institutional role
/// it conflicts with. Callers may exclude that work's future headroom because the evidence owner
/// cancels it in the same atomic case mutation.
pub(crate) fn scheduled_work_invalidated_by_actionable_character(
    state: &AppState,
    investigation: InvestigationId,
    character: CharacterId,
) -> Option<InvestigationWorkId> {
    let record = state.legal.get_investigation(investigation)?;
    if record.lead_investigator() == Some(character) {
        return state
            .legal
            .scheduled_work_for_investigator(character)
            .filter(|work| work.investigation() == investigation)
            .map(|work| work.id());
    }
    let witness = state.legal.case_witness_for(investigation, character)?;
    state
        .legal
        .scheduled_work_for_focus(
            investigation,
            InvestigationWorkKind::WitnessInterview,
            crate::legal::InvestigationWorkFocus::witness(witness.id()),
        )
        .map(|work| work.id())
}

fn resolve_work_sources(
    registry: &Registry,
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    match draft.kind {
        InvestigationWorkKind::EvidenceReview => resolve_review_source(state, draft),
        InvestigationWorkKind::WitnessInterview => resolve_interview_focus(registry, state, draft),
    }
}

/// An interview's source is a registered case witness, not an evidence record; its support
/// comes from the witness's cooperation at resolution time.
fn resolve_interview_focus(
    registry: &Registry,
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    let case_witness = draft
        .focus
        .witness_id()
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    let witness = state
        .legal
        .get_case_witness(case_witness)
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    if witness.investigation() != draft.investigation {
        return Err(InvestigationWorkError::InvalidFocus);
    }
    if crate::legal::witness_system::case_witness_is_case_subject(state, witness) {
        return Err(InvestigationWorkError::WitnessIsCaseSubject {
            witness: case_witness,
            character: witness.witness(),
        });
    }
    if !witness.statements().is_empty() {
        return Err(InvestigationWorkError::WitnessAlreadyStatemented {
            witness: case_witness,
        });
    }
    let limit = registry.legal().witness_interview_attempt_limit();
    if witness.interview_attempts() >= limit {
        return Err(InvestigationWorkError::WitnessInterviewLimitReached {
            witness: case_witness,
            attempts: witness.interview_attempts(),
            limit,
        });
    }
    Ok(BTreeSet::new())
}

fn resolve_review_source(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    let evidence_id = draft
        .focus
        .evidence_id()
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    let evidence = state
        .legal
        .get_evidence(evidence_id)
        .ok_or(InvestigationWorkError::InvalidSourceEvidence(evidence_id))?;
    if evidence.investigation() != draft.investigation {
        return Err(InvestigationWorkError::InvalidSourceEvidence(evidence_id));
    }
    if !evidence.kind().is_reviewable() {
        return Err(InvestigationWorkError::InvalidFocus);
    }
    if let Some(derived) = state
        .legal
        .derived_evidence_from(evidence_id)
        .find(|derived| derived.kind() == EvidenceKind::ForensicAnalysis)
    {
        return Err(InvestigationWorkError::EvidenceAlreadyReviewed {
            evidence: evidence_id,
            derived: derived.id(),
        });
    }
    if let Some(work) = state.legal.evidence_review_attempt(evidence_id) {
        return Err(InvestigationWorkError::EvidenceReviewAlreadyAttempted {
            evidence: evidence_id,
            work: work.id(),
        });
    }
    Ok(BTreeSet::from([evidence_id]))
}

fn validate_case_and_investigator(
    state: &AppState,
    investigation_id: InvestigationId,
    investigator_id: CharacterId,
) -> Result<(), InvestigationWorkError> {
    let investigation = state.legal.get_investigation(investigation_id).ok_or(
        InvestigationWorkError::MissingInvestigation(investigation_id),
    )?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationWorkError::InactiveInvestigation(
            investigation_id,
        ));
    }
    let investigator = state
        .world
        .get_character(investigator_id)
        .ok_or(InvestigationWorkError::MissingInvestigator(investigator_id))?;
    if investigation.lead_investigator() != Some(investigator_id) {
        return Err(InvestigationWorkError::InvestigatorNotAssigned {
            investigation: investigation_id,
            investigator: investigator_id,
        });
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(investigator_id) {
        return Err(InvestigationWorkError::DetainedInvestigator {
            investigator: investigator_id,
            arrest: arrest.id(),
        });
    }
    if investigator.organization() != Some(investigation.owner()) {
        return Err(InvestigationWorkError::InvestigatorNotAssigned {
            investigation: investigation_id,
            investigator: investigator_id,
        });
    }
    if investigator
        .capability(CapabilityKind::Investigation)
        .is_none()
    {
        return Err(InvestigationWorkError::MissingInvestigationCapability(
            investigator_id,
        ));
    }
    Ok(())
}

fn validate_no_duplicate_work(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<(), InvestigationWorkError> {
    if let Some(work) =
        state
            .legal
            .scheduled_work_for_focus(draft.investigation, draft.kind, draft.focus)
    {
        return Err(InvestigationWorkError::DuplicateScheduledWork { work: work.id() });
    }
    Ok(())
}

pub(super) fn scheduled_work_for_investigator(
    state: &AppState,
    investigator: CharacterId,
) -> Option<InvestigationWorkId> {
    state
        .legal
        .scheduled_work_for_investigator(investigator)
        .map(|work| work.id())
}

fn validate_investigator_capacity(
    state: &AppState,
    investigator: CharacterId,
) -> Result<(), InvestigationWorkError> {
    if let Some(work) = scheduled_work_for_investigator(state, investigator) {
        return Err(InvestigationWorkError::InvestigatorBusy { investigator, work });
    }
    Ok(())
}

mod autonomous_scheduling;
pub(crate) use autonomous_scheduling::{
    InvestigationWorkSchedulingOutcome, apply_investigation_work_scheduling,
};
#[cfg(test)]
pub(crate) use autonomous_scheduling::{
    apply_evidence_review_scheduling, apply_witness_interview_scheduling,
};

#[cfg(test)]
mod tests;
