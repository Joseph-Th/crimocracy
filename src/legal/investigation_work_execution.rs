//! Scheduled detective work; this sibling system derives new case evidence only from evidence already owned by the investigation.

use crate::core::entity::EntityRef;
use crate::core::id::CaseWitnessId;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, InvestigationId, InvestigationWorkId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::{
    Admissibility, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, InvestigationStatus,
    InvestigationWorkDraft, InvestigationWorkFactors, InvestigationWorkFocus,
    InvestigationWorkIdentity, InvestigationWorkKind, InvestigationWorkOutcome,
    InvestigationWorkRecord, InvestigationWorkResolution, InvestigationWorkRuntime,
    InvestigationWorkStatus, WitnessCooperation, WitnessStatementDraft,
};
use crate::registry::{InvestigationWorkDefinition, Registry};
use crate::world::{CapabilityKind, Rating};
use std::collections::BTreeSet;
use thiserror::Error;

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
    #[error("evidence {evidence} has already been reviewed as evidence {derived}")]
    EvidenceAlreadyReviewed {
        evidence: EvidenceId,
        derived: EvidenceId,
    },
    #[error("scheduled investigation work {work} already covers this case focus")]
    DuplicateScheduledWork { work: InvestigationWorkId },
    #[error("investigation evidence set is too large to persist as one work item")]
    SourceEvidenceCountOverflow,
    #[error("investigation {investigation} changed after work validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("investigator {investigator} changed after work validation; expected version {expected}, found {found}")]
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
    #[error("investigation work {work} changed after resolution planning; expected version {expected}, found {found}")]
    StaleWork {
        work: InvestigationWorkId,
        expected: u32,
        found: u32,
    },
    #[error("resolution context for investigation work {work} changed after resolution planning")]
    StaleResolutionContext { work: InvestigationWorkId },
    #[error("investigation work resolution was planned at {expected:?}, but simulation time is now {found:?}")]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error("investigation work variance {variance} exceeds authored limit {limit}")]
    VarianceOutOfRange { variance: i8, limit: u8 },
    #[error("investigation work source evidence {0} no longer belongs to the case")]
    InvalidSourceEvidence(EvidenceId),
    #[error("witness interview for work {work} could not record a statement: {error}")]
    InterviewStatementFailed {
        work: InvestigationWorkId,
        error: crate::legal::witness_system::WitnessError,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
        // The investigation version snapshot above is authoritative for evidence-set
        // staleness: every evidence mutation bumps the investigation version, so no separate
        // source-set comparison is needed (and one could not report a meaningful
        // expected/found version pair).

        let id = state.ids.next_investigation_work()?;
        let scheduled_at = state.now();
        let due_at =
            scheduled_at + crate::core::time::SimDuration::from_minutes(self.duration_minutes);
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
                    version: 1,
                },
            });
        Ok(id)
    }
}

pub fn validate_schedule_investigation_work(
    registry: &Registry,
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<ValidatedInvestigationWorkSchedule, InvestigationWorkError> {
    validate_case_and_investigator(state, draft.investigation, draft.investigator)?;
    validate_no_duplicate_work(state, draft)?;
    let source_evidence = resolve_work_sources(state, draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must still exist");
    let investigator = state
        .world
        .get_character(draft.investigator)
        .expect("validated investigator must still exist");
    let duration = registry.get_investigation_work(draft.kind).duration();
    Ok(ValidatedInvestigationWorkSchedule {
        draft,
        source_evidence,
        expected_investigation_version: investigation.version(),
        expected_investigator_version: investigator.version(),
        duration_minutes: duration.as_minutes(),
    })
}

fn resolve_work_sources(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    match draft.kind {
        InvestigationWorkKind::EvidenceReview => resolve_review_source(state, draft),
        InvestigationWorkKind::WitnessInterview => resolve_interview_focus(state, draft),
    }
}

/// An interview's source is a registered case witness, not an evidence record; its support
/// comes from the witness's cooperation at resolution time.
fn resolve_interview_focus(
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
    if !is_reviewable_evidence_kind(evidence.kind()) {
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
    if investigation.investigator_role(investigator_id).is_none() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvestigationWorkRandomness {
    variance: i8,
}

impl InvestigationWorkRandomness {
    pub fn new(variance: i8) -> Self {
        Self { variance }
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationWorkResolutionPlan {
    work: InvestigationWorkId,
    expected_work_version: u32,
    expected_investigation_version: u32,
    expected_investigator_version: u32,
    resolved_at: SimTime,
    outcome: InvestigationWorkOutcome,
    factors: InvestigationWorkFactors,
    margin: i16,
}

impl InvestigationWorkResolutionPlan {
    pub fn work(&self) -> InvestigationWorkId {
        self.work
    }

    pub fn outcome(&self) -> InvestigationWorkOutcome {
        self.outcome
    }

    pub fn factors(&self) -> InvestigationWorkFactors {
        self.factors
    }

    pub fn margin(&self) -> i16 {
        self.margin
    }
}

pub fn decide_investigation_work_resolution(
    registry: &Registry,
    state: &AppState,
    work_id: InvestigationWorkId,
    randomness: InvestigationWorkRandomness,
) -> Result<InvestigationWorkResolutionPlan, InvestigationWorkError> {
    let work = validate_due_work(state, work_id)?;
    let definition = registry.get_investigation_work(work.kind());
    if randomness.variance().unsigned_abs() > definition.variance_limit() {
        return Err(InvestigationWorkError::VarianceOutOfRange {
            variance: randomness.variance(),
            limit: definition.variance_limit(),
        });
    }
    let investigator = state
        .world
        .get_character(work.investigator())
        .expect("validated scheduled investigator must exist");
    let investigation = state
        .legal
        .get_investigation(work.investigation())
        .expect("validated scheduled work must have an investigation");
    let (factors, margin) =
        resolve_work_factors_and_margin(definition, state, work, randomness.variance())?;
    let outcome = if margin >= definition.connected_margin() {
        match work.kind() {
            InvestigationWorkKind::EvidenceReview => InvestigationWorkOutcome::Developed,
            InvestigationWorkKind::WitnessInterview => InvestigationWorkOutcome::Connected,
        }
    } else {
        InvestigationWorkOutcome::Inconclusive
    };
    Ok(InvestigationWorkResolutionPlan {
        work: work.id(),
        expected_work_version: work.version(),
        expected_investigation_version: investigation.version(),
        expected_investigator_version: investigator.version(),
        resolved_at: state.now(),
        outcome,
        factors,
        margin,
    })
}

fn validate_due_work(
    state: &AppState,
    work_id: InvestigationWorkId,
) -> Result<&InvestigationWorkRecord, InvestigationWorkError> {
    let work = state
        .legal
        .get_investigation_work(work_id)
        .ok_or(InvestigationWorkError::MissingWork(work_id))?;
    if work.status() != InvestigationWorkStatus::Scheduled {
        return Err(InvestigationWorkError::WorkNotScheduled(work_id));
    }
    if state.now() < work.due_at() {
        return Err(InvestigationWorkError::WorkNotDue {
            work: work_id,
            due_at: work.due_at(),
        });
    }
    validate_case_and_investigator(state, work.investigation(), work.investigator())?;
    validate_source_evidence(state, work)?;
    Ok(work)
}

fn validate_source_evidence(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<(), InvestigationWorkError> {
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if evidence.investigation() != work.investigation() {
            return Err(InvestigationWorkError::InvalidSourceEvidence(*evidence_id));
        }
    }
    Ok(())
}

pub(crate) fn resolve_work_difficulty(
    definition: &InvestigationWorkDefinition,
    source_evidence_count: u8,
) -> u8 {
    let additional_count = source_evidence_count.saturating_sub(2);
    let additional = u16::from(additional_count)
        .saturating_mul(u16::from(definition.additional_source_difficulty()));
    u8::try_from(
        u16::from(definition.base_difficulty())
            .saturating_add(additional)
            .min(100),
    )
    .expect("clamped investigation difficulty must fit u8")
}

pub(crate) fn resolve_work_factors_and_margin(
    definition: &InvestigationWorkDefinition,
    state: &AppState,
    work: &InvestigationWorkRecord,
    variance: i8,
) -> Result<(InvestigationWorkFactors, i16), InvestigationWorkError> {
    let investigator = state.world.get_character(work.investigator()).ok_or(
        InvestigationWorkError::MissingInvestigator(work.investigator()),
    )?;
    let investigation_capability = investigator
        .capability(CapabilityKind::Investigation)
        .ok_or(InvestigationWorkError::MissingInvestigationCapability(
            work.investigator(),
        ))?;
    let source_support = resolve_source_support(state, work)?;
    let source_evidence_count = u8::try_from(work.source_evidence().len())
        .map_err(|_| InvestigationWorkError::SourceEvidenceCountOverflow)?;
    let difficulty = resolve_work_difficulty(definition, source_evidence_count);
    let support_adjustment =
        i16::from(source_support.value()) * i16::from(definition.source_support_weight()) / 100;
    let margin =
        i16::from(investigation_capability.value()) + support_adjustment + i16::from(variance)
            - i16::from(difficulty);
    Ok((
        InvestigationWorkFactors {
            investigation_capability,
            source_support,
            source_evidence_count,
            difficulty,
            variance,
        },
        margin,
    ))
}

pub(crate) fn resolve_source_support(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<Rating, InvestigationWorkError> {
    // Interview support is the witness's current cooperation, not evidence quality.
    if work.kind() == InvestigationWorkKind::WitnessInterview {
        let case_witness = work
            .focus()
            .witness_id()
            .ok_or(InvestigationWorkError::InvalidFocus)?;
        let witness = state
            .legal
            .get_case_witness(case_witness)
            .ok_or(InvestigationWorkError::InvalidFocus)?;
        let score = match witness.cooperation() {
            WitnessCooperation::Hostile => 20_u8,
            WitnessCooperation::Reluctant => 50,
            WitnessCooperation::Cooperative => 85,
        };
        return Rating::try_new(score).map_err(|_| InvestigationWorkError::InvalidFocus);
    }
    let mut total = 0_u32;
    let mut count = 0_u32;
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if evidence.investigation() != work.investigation() {
            return Err(InvestigationWorkError::InvalidSourceEvidence(*evidence_id));
        }
        total = total
            .saturating_add(u32::from(strength_score(evidence.strength())))
            .saturating_add(u32::from(reliability_score(evidence.reliability())))
            .saturating_add(u32::from(admissibility_score(evidence.admissibility())));
        count = count.saturating_add(3);
    }
    let average = total.checked_div(count).unwrap_or(0);
    Ok(
        Rating::try_new(u8::try_from(average).expect("evidence support average must fit u8"))
            .expect("bounded evidence support average must be a valid rating"),
    )
}

fn strength_score(strength: EvidenceStrength) -> u8 {
    match strength {
        EvidenceStrength::Weak => 20,
        EvidenceStrength::Corroborating => 45,
        EvidenceStrength::Strong => 70,
        EvidenceStrength::Direct => 95,
    }
}

fn reliability_score(reliability: EvidenceReliability) -> u8 {
    match reliability {
        EvidenceReliability::Questionable => 15,
        EvidenceReliability::Mixed => 40,
        EvidenceReliability::Credible => 70,
        EvidenceReliability::HighlyReliable => 95,
    }
}

fn admissibility_score(admissibility: Admissibility) -> u8 {
    match admissibility {
        Admissibility::Unknown => 35,
        Admissibility::Inadmissible => 0,
        Admissibility::Disputed => 50,
        Admissibility::Admissible => 90,
    }
}

pub struct ValidatedInvestigationWorkResolution {
    plan: InvestigationWorkResolutionPlan,
    interview_statement: Option<crate::legal::witness_system::ValidatedWitnessStatement>,
}

struct DerivedEvidenceDraft {
    investigation: InvestigationId,
    custodian: crate::core::id::OrganizationId,
    subject: EntityRef,
    origin: Option<EntityRef>,
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
    derived_from: BTreeSet<EvidenceId>,
}

impl ValidatedInvestigationWorkResolution {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<InvestigationWorkId, InvestigationWorkError> {
        validate_resolution_snapshot(state, &self.plan)?;
        let derived_evidence_draft = match self.plan.outcome {
            InvestigationWorkOutcome::Connected => {
                // A connected interview is committed through the canonical witness-
                // statement path below; it produces testimony evidence plus the named
                // statement record rather than a derived evidence draft.
                debug_assert_eq!(
                    state
                        .legal
                        .get_investigation_work(self.plan.work)
                        .expect("validated investigation work must exist")
                        .kind(),
                    InvestigationWorkKind::WitnessInterview
                );
                None
            }
            InvestigationWorkOutcome::Developed => {
                let work = state
                    .legal
                    .get_investigation_work(self.plan.work)
                    .expect("validated investigation work must exist");
                debug_assert_eq!(work.kind(), InvestigationWorkKind::EvidenceReview);
                let source_id = work
                    .focus()
                    .evidence_id()
                    .expect("evidence review work must focus one evidence record");
                let source = state
                    .legal
                    .get_evidence(source_id)
                    .expect("validated evidence review source must exist");
                Some(DerivedEvidenceDraft {
                    investigation: work.investigation(),
                    custodian: state
                        .legal
                        .get_investigation(work.investigation())
                        .expect("validated investigation must exist")
                        .owner(),
                    subject: source.subject(),
                    origin: source.origin(),
                    kind: EvidenceKind::ForensicAnalysis,
                    strength: source.strength(),
                    reliability: resolve_improved_evidence_reliability(source.reliability()),
                    admissibility: source.admissibility(),
                    derived_from: BTreeSet::from([source_id]),
                })
            }
            InvestigationWorkOutcome::Inconclusive => None,
        };
        // Successful witness interviews record the testimony through the canonical
        // witness-statement path validated during plan validation.
        let interview_statement_outcome = match self.interview_statement {
            Some(statement) => Some(statement.commit(state).map_err(|error| {
                InvestigationWorkError::InterviewStatementFailed {
                    work: self.plan.work,
                    error,
                }
            })?),
            None => None,
        };
        let derived_evidence = if let Some(draft) = derived_evidence_draft {
            let id = state.ids.next_evidence()?;
            state.legal.insert_evidence(
                EvidenceRecord {
                    identity: EvidenceIdentity {
                        id,
                        investigation: draft.investigation,
                        custodian: draft.custodian,
                    },
                    connection: EvidenceConnection {
                        subject: draft.subject,
                        origin: draft.origin,
                        source: None,
                        derived_from: draft.derived_from,
                    },
                    assessment: EvidenceAssessment {
                        kind: draft.kind,
                        strength: draft.strength,
                        reliability: draft.reliability,
                        admissibility: draft.admissibility,
                    },
                    discovered_at: self.plan.resolved_at,
                },
                self.plan.resolved_at,
            );
            Some(id)
        } else {
            None
        };
        state.legal.complete_investigation_work(
            self.plan.work,
            InvestigationWorkResolution {
                resolved_at: self.plan.resolved_at,
                outcome: self.plan.outcome,
                factors: self.plan.factors,
                margin: self.plan.margin,
                // For interviews this is the testimony evidence produced by the recorded
                // statement; for other kinds it is the work's own derived evidence.
                derived_evidence: derived_evidence
                    .or(interview_statement_outcome.map(|outcome| outcome.evidence)),
            },
        );
        Ok(self.plan.work)
    }
}

pub(crate) fn resolve_improved_evidence_reliability(
    reliability: EvidenceReliability,
) -> EvidenceReliability {
    match reliability {
        EvidenceReliability::Questionable => EvidenceReliability::Mixed,
        EvidenceReliability::Mixed => EvidenceReliability::Credible,
        EvidenceReliability::Credible | EvidenceReliability::HighlyReliable => {
            EvidenceReliability::HighlyReliable
        }
    }
}

/// Builds the canonical statement an interview records. The testimony targets, in order of
/// preference: the character backed by the strongest evidence already in the case graph
/// (minimum ID breaking strength ties), the case's origin operation when no person has been
/// tied to the case yet, a character named as a case subject, and finally the lowest case
/// subject of any kind so institution-authored cases without an operation origin still
/// produce a connected statement. Confidence is a deterministic function of the margin.
fn resolve_interview_statement_draft(
    state: &AppState,
    work: &InvestigationWorkRecord,
    case_witness: CaseWitnessId,
    margin: i16,
) -> Result<WitnessStatementDraft, InvestigationWorkError> {
    use std::cmp::Reverse;

    let investigation = state
        .legal
        .get_investigation(work.investigation())
        .expect("validated interview investigation must exist");
    let subject = investigation
        .evidence()
        .iter()
        .filter_map(|id| state.legal.get_evidence(*id))
        .filter(|evidence| matches!(evidence.subject(), EntityRef::Character(_)))
        .max_by_key(|evidence| (evidence.strength(), Reverse(evidence.subject())))
        .map(|evidence| evidence.subject())
        .or_else(|| {
            investigation
                .origin()
                .filter(|origin| {
                    matches!(origin, EntityRef::Operation(_) | EntityRef::Enterprise(_))
                })
                .or_else(|| {
                    investigation
                        .subjects()
                        .iter()
                        .find(|subject| matches!(subject, EntityRef::Character(_)))
                        .copied()
                })
                .or_else(|| investigation.subjects().iter().next().copied())
        })
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    let confidence = if margin >= 20 {
        Rating::try_new(85).expect("interview confidence must be valid")
    } else if margin >= 10 {
        Rating::try_new(65).expect("interview confidence must be valid")
    } else {
        Rating::try_new(40).expect("interview confidence must be valid")
    };
    Ok(WitnessStatementDraft {
        case_witness,
        subject,
        origin: None,
        confidence,
        summary: format!(
            "Statement recorded from witness interview on work {} regarding {subject:?}.",
            work.id()
        ),
    })
}

/// Schedules witness interviews for staffed active cases whose registered witnesses have not
/// given a statement yet. Witnesses typically enter a case after the initial evidence review
/// is already scheduled, so this runs every tick over the (small) set of active cases.
pub fn apply_witness_interview_scheduling(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let mut scheduled = Vec::new();
    let candidates: Vec<InvestigationId> = state
        .legal
        .active_investigations()
        .map(|investigation| investigation.id())
        .collect();
    for investigation_id in candidates {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("indexed active investigation must exist");
        // Deterministic investigator choice: the lead when present and available, otherwise
        // the lowest assigned ID. Detained investigators are skipped so an autonomous pass
        // never schedules work they could not perform; cases without an available
        // investigator wait for a later minute or the staffing pass.
        let lead = investigation.lead_investigator();
        let investigator: Option<CharacterId> =
            if lead.is_some_and(|lead| state.legal.active_arrest_for_character(lead).is_none()) {
                lead
            } else {
                // The lead is detained or absent; fall through to the lowest assigned ID.
                investigation
                    .assigned_investigators()
                    .iter()
                    .copied()
                    .find(|id| state.legal.active_arrest_for_character(*id).is_none())
            };
        let Some(investigator) = investigator else {
            continue;
        };
        let witnesses: Vec<_> = state
            .legal
            .case_witnesses_for_investigation(investigation_id)
            .filter(|witness| witness.statements().is_empty())
            // A witness who has sat through the authored attempt limit without producing a
            // statement stops consuming institutional work: further interviews are futile,
            // and each one would otherwise keep the case's activity clock fresh forever.
            .filter(|witness| {
                witness.interview_attempts() < registry.legal().witness_interview_attempt_limit()
            })
            .map(|witness| witness.id())
            .collect();
        for case_witness in witnesses {
            let focus = InvestigationWorkFocus::witness(case_witness);
            // A pending scheduled interview covers this witness; a completed interview was
            // counted against the witness's attempt budget above, so only witnesses with
            // remaining attempts reach this point.
            if state
                .legal
                .scheduled_work_for_focus(
                    investigation_id,
                    InvestigationWorkKind::WitnessInterview,
                    focus,
                )
                .is_some()
            {
                continue;
            }
            // An autonomous pass must not abort the tick: a canonical rejection (for example
            // an investigator detained between selection and validation) leaves this witness
            // for a later minute, like the staffing pass's unstaffed cases.
            let Ok(work) = validate_schedule_investigation_work(
                registry,
                state,
                InvestigationWorkDraft {
                    investigation: investigation_id,
                    investigator,
                    kind: InvestigationWorkKind::WitnessInterview,
                    focus,
                },
            ) else {
                continue;
            };
            let Ok(work) = work.commit(state) else {
                continue;
            };
            scheduled.push(work);
        }
    }
    Ok(scheduled)
}

pub(crate) fn apply_initial_evidence_reviews(
    registry: &Registry,
    state: &mut AppState,
    staffed: &[(InvestigationId, CharacterId)],
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let mut scheduled = Vec::new();
    for (investigation_id, investigator) in staffed {
        if state
            .legal
            .work_for_investigation(*investigation_id)
            .next()
            .is_some()
        {
            continue;
        }
        let source = state
            .legal
            .get_investigation(*investigation_id)
            .into_iter()
            .flat_map(|investigation| investigation.evidence().iter())
            .filter_map(|id| state.legal.get_evidence(*id))
            .find(|evidence| is_reviewable_evidence_kind(evidence.kind()))
            .map(|evidence| evidence.id());
        let Some(source) = source else {
            continue;
        };
        let work = validate_schedule_investigation_work(
            registry,
            state,
            InvestigationWorkDraft {
                investigation: *investigation_id,
                investigator: *investigator,
                kind: InvestigationWorkKind::EvidenceReview,
                focus: InvestigationWorkFocus::evidence(source),
            },
        )?
        .commit(state)?;
        scheduled.push(work);
    }
    Ok(scheduled)
}

pub(crate) fn is_reviewable_evidence_kind(kind: EvidenceKind) -> bool {
    matches!(
        kind,
        EvidenceKind::Fingerprint
            | EvidenceKind::RecoveredProperty
            | EvidenceKind::FinancialRecord
            | EvidenceKind::Surveillance
            | EvidenceKind::CommunicationRecord
            | EvidenceKind::Document
            | EvidenceKind::Ballistics
            | EvidenceKind::VehicleDescription
    )
}

pub fn validate_investigation_work_resolution_plan(
    registry: &Registry,
    state: &AppState,
    plan: InvestigationWorkResolutionPlan,
) -> Result<ValidatedInvestigationWorkResolution, InvestigationWorkError> {
    validate_resolution_snapshot(state, &plan)?;
    let work = state
        .legal
        .get_investigation_work(plan.work)
        .expect("validated work must exist");
    let definition = registry.get_investigation_work(work.kind());
    let (expected_factors, expected_margin) =
        resolve_work_factors_and_margin(definition, state, work, plan.factors.variance())?;
    let expected_outcome = if expected_margin >= definition.connected_margin() {
        match work.kind() {
            InvestigationWorkKind::EvidenceReview => InvestigationWorkOutcome::Developed,
            InvestigationWorkKind::WitnessInterview => InvestigationWorkOutcome::Connected,
        }
    } else {
        InvestigationWorkOutcome::Inconclusive
    };
    if plan.factors != expected_factors
        || plan.factors.variance().unsigned_abs() > definition.variance_limit()
        || plan.margin != expected_margin
        || plan.outcome != expected_outcome
    {
        // The recomputed factors/margin/outcome disagree with the plan even though the work
        // record itself may be unchanged, so this reports context drift rather than a
        // version mismatch.
        return Err(InvestigationWorkError::StaleResolutionContext { work: plan.work });
    }
    // A connected interview will record a statement at commit; validate it now so commit
    // only re-checks staleness.
    let interview_statement = if plan.outcome == InvestigationWorkOutcome::Connected
        && work.kind() == InvestigationWorkKind::WitnessInterview
    {
        let case_witness = work
            .focus()
            .witness_id()
            .expect("interview focus must reference a case witness");
        Some(
            crate::legal::witness_system::validate_record_witness_statement(
                state,
                resolve_interview_statement_draft(state, work, case_witness, plan.margin)?,
            )
            .map_err(|error| InvestigationWorkError::InterviewStatementFailed {
                work: plan.work,
                error,
            })?,
        )
    } else {
        None
    };
    Ok(ValidatedInvestigationWorkResolution {
        plan,
        interview_statement,
    })
}

fn validate_resolution_snapshot(
    state: &AppState,
    plan: &InvestigationWorkResolutionPlan,
) -> Result<(), InvestigationWorkError> {
    let work = validate_due_work(state, plan.work)?;
    if work.version() != plan.expected_work_version {
        return Err(InvestigationWorkError::StaleWork {
            work: plan.work,
            expected: plan.expected_work_version,
            found: work.version(),
        });
    }
    let investigator = state
        .world
        .get_character(work.investigator())
        .expect("validated investigator must exist");
    if investigator.version() != plan.expected_investigator_version {
        return Err(InvestigationWorkError::StaleInvestigator {
            investigator: investigator.id(),
            expected: plan.expected_investigator_version,
            found: investigator.version(),
        });
    }
    let investigation = state
        .legal
        .get_investigation(work.investigation())
        .expect("validated work must have an investigation");
    if investigation.version() != plan.expected_investigation_version {
        return Err(InvestigationWorkError::StaleInvestigation {
            investigation: investigation.id(),
            expected: plan.expected_investigation_version,
            found: investigation.version(),
        });
    }
    if state.now() != plan.resolved_at {
        return Err(InvestigationWorkError::StaleResolutionTime {
            expected: plan.resolved_at,
            found: state.now(),
        });
    }
    Ok(())
}

pub(crate) fn find_due_scheduled_investigation_work(state: &AppState) -> Vec<InvestigationWorkId> {
    state
        .legal
        .find_investigation_work_due_at_or_before(state.now())
}

#[cfg(test)]
mod tests;
