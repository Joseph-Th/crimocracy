//! Investigation-work resolution planning, factor validation, and evidence persistence.

use super::{InvestigationWorkError, validate_case_and_investigator};
use crate::core::entity::EntityRef;
use crate::core::id::{CaseWitnessId, EvidenceId, IdKind, InvestigationId, InvestigationWorkId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{ensure_version_can_advance, ensure_version_can_advance_by};
use crate::legal::{
    Admissibility, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, InvestigationWorkFactors,
    InvestigationWorkKind, InvestigationWorkOutcome, InvestigationWorkRecord,
    InvestigationWorkResolution, InvestigationWorkStatus, WitnessCooperation,
    WitnessStatementDraft,
};
use crate::registry::{
    InvestigationSourceSupportDefinition, InvestigationWorkDefinition, Registry,
};
use crate::world::{CapabilityKind, Rating};
use std::collections::BTreeSet;

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
    expected_case_witness_version: Option<u32>,
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
    let expected_case_witness_version = work.focus().witness_id().map(|case_witness| {
        state
            .legal
            .get_case_witness(case_witness)
            .expect("validated interview focus must reference an existing witness")
            .version()
    });
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
        expected_case_witness_version,
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
    let source_support = resolve_source_support(definition, state, work)?;
    let difficulty = definition.base_difficulty();
    let factors = InvestigationWorkFactors {
        investigation_capability,
        source_support,
        difficulty,
        variance,
    };
    Ok((
        factors,
        resolve_work_margin_from_factors(definition, factors),
    ))
}

/// Recomputes only the arithmetic encoded by a frozen work-factor snapshot. Historical
/// validation uses this instead of asking mutable world state what the source support is now.
pub(crate) fn resolve_work_margin_from_factors(
    definition: &InvestigationWorkDefinition,
    factors: InvestigationWorkFactors,
) -> i16 {
    let support_adjustment = i16::from(factors.source_support().value())
        * i16::from(definition.source_support_weight())
        / 100;
    i16::from(factors.investigation_capability().value())
        + support_adjustment
        + i16::from(factors.variance())
        - i16::from(factors.difficulty())
}

/// Validates a completed work record against the authored rules without rewriting history from
/// mutable present-day facts. Evidence assessments and investigator capabilities are immutable in
/// the current model and can still be re-derived. Witness cooperation is deliberately mutable, so
/// an interview's persisted support is validated as one of the canonical cooperation bands rather
/// than compared with the witness's current attitude.
pub(crate) fn validate_historical_work_factors(
    definition: &InvestigationWorkDefinition,
    state: &AppState,
    work: &InvestigationWorkRecord,
    factors: InvestigationWorkFactors,
) -> Result<i16, InvestigationWorkError> {
    let investigator = state.world.get_character(work.investigator()).ok_or(
        InvestigationWorkError::MissingInvestigator(work.investigator()),
    )?;
    let investigation_capability = investigator
        .capability(CapabilityKind::Investigation)
        .ok_or(InvestigationWorkError::MissingInvestigationCapability(
            work.investigator(),
        ))?;
    let expected_difficulty = definition.base_difficulty();
    let support_valid = match work.kind() {
        InvestigationWorkKind::EvidenceReview => {
            factors.source_support() == resolve_source_support(definition, state, work)?
        }
        InvestigationWorkKind::WitnessInterview => [
            WitnessCooperation::Hostile,
            WitnessCooperation::Reluctant,
            WitnessCooperation::Cooperative,
        ]
        .into_iter()
        .any(|cooperation| {
            factors.source_support()
                == witness_cooperation_support(definition.source_support(), cooperation)
        }),
    };
    if factors.investigation_capability() != investigation_capability
        || factors.difficulty() != expected_difficulty
        || !support_valid
    {
        return Err(InvestigationWorkError::StaleResolutionContext { work: work.id() });
    }
    Ok(resolve_work_margin_from_factors(definition, factors))
}

pub(crate) fn resolve_source_support(
    definition: &InvestigationWorkDefinition,
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<Rating, InvestigationWorkError> {
    let support = definition.source_support();
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
        return Ok(witness_cooperation_support(support, witness.cooperation()));
    }
    let mut total = 0_u64;
    let mut count = 0_u64;
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if evidence.investigation() != work.investigation() {
            return Err(InvestigationWorkError::InvalidSourceEvidence(*evidence_id));
        }
        total += u64::from(support.strength_score(evidence.strength()))
            + u64::from(support.reliability_score(evidence.reliability()))
            + u64::from(support.admissibility_score(evidence.admissibility()));
        count += 3;
    }
    let average = total
        .checked_div(count)
        .expect("evidence-review work must retain at least one source evidence record");
    Ok(
        Rating::try_new(u8::try_from(average).expect("evidence support average must fit u8"))
            .expect("bounded evidence support average must be a valid rating"),
    )
}

fn witness_cooperation_support(
    support: InvestigationSourceSupportDefinition,
    cooperation: WitnessCooperation,
) -> Rating {
    let score = support.witness_score(cooperation);
    Rating::try_new(score).expect("canonical witness support scores are valid ratings")
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
    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        match self.plan.outcome {
            InvestigationWorkOutcome::Connected => self
                .interview_statement
                .as_ref()
                .expect("connected investigation work must carry its validated witness statement")
                .id_budget(),
            InvestigationWorkOutcome::Developed => {
                debug_assert!(
                    self.interview_statement.is_none(),
                    "developed evidence review never carries a witness statement"
                );
                vec![(IdKind::Evidence, 1)]
            }
            InvestigationWorkOutcome::Inconclusive => Vec::new(),
        }
    }

    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<InvestigationWorkId, InvestigationWorkError> {
        validate_resolution_snapshot(state, &self.plan)?;
        if let Some(statement) = &self.interview_statement {
            statement.ensure_current(state).map_err(|error| {
                InvestigationWorkError::InterviewStatementFailed {
                    work: self.plan.work,
                    error,
                }
            })?;
        }
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
        if let Some(draft) = &derived_evidence_draft {
            crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
                state,
                draft.investigation,
                draft.subject,
                draft.strength,
                draft.reliability,
                draft.admissibility,
            )?;
        }
        // Successful witness interviews record the testimony through the canonical
        // witness-statement path validated during plan validation.
        let interview_statement_outcome = match self.interview_statement {
            Some(statement) => Some(
                statement
                    .commit_from_investigation_work(state, self.plan.work)
                    .map_err(|error| InvestigationWorkError::InterviewStatementFailed {
                        work: self.plan.work,
                        error,
                    })?,
            ),
            None => None,
        };
        let derived_evidence = if let Some(draft) = derived_evidence_draft {
            let id = state.ids.next_evidence()?;
            state.legal.insert_evidence_from_investigation_work(
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
                self.plan.work,
            );
            Some(id)
        } else {
            None
        };
        state.legal.set_investigation_work_resolution(
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

/// Builds the canonical statement an interview records. The testimony subject is fixed when the
/// witness is registered; later case evidence cannot retroactively make this person testify about
/// a different suspect. Confidence is a deterministic function of the margin.
fn resolve_interview_statement_draft(
    definition: &InvestigationWorkDefinition,
    state: &AppState,
    work: &InvestigationWorkRecord,
    case_witness: CaseWitnessId,
    margin: i16,
) -> Result<WitnessStatementDraft, InvestigationWorkError> {
    let subject = state
        .legal
        .get_case_witness(case_witness)
        .filter(|witness| witness.investigation() == work.investigation())
        .map(|witness| witness.subject())
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    let confidence = Rating::try_new(
        definition
            .interview_outcome()
            .expect("witness interview definition must author outcome confidence")
            .confidence_for_margin(margin),
    )
    .expect("authored interview confidence must be a valid rating");
    Ok(WitnessStatementDraft {
        case_witness,
        origin: None,
        confidence,
        summary: format!(
            "Statement recorded from witness interview on work {} regarding {subject:?}.",
            work.id()
        ),
    })
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
            crate::legal::witness_system::validate_record_witness_statement_for_investigation_work(
                registry,
                state,
                resolve_interview_statement_draft(
                    definition,
                    state,
                    work,
                    case_witness,
                    plan.margin,
                )?,
                plan.work,
            )
            .map_err(|error| InvestigationWorkError::InterviewStatementFailed {
                work: plan.work,
                error,
            })?,
        )
    } else {
        None
    };
    if plan.outcome == InvestigationWorkOutcome::Developed
        && work.kind() == InvestigationWorkKind::EvidenceReview
    {
        let source = state
            .legal
            .get_evidence(
                work.focus()
                    .evidence_id()
                    .expect("evidence-review focus must reference evidence"),
            )
            .expect("validated evidence-review source must exist");
        crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
            state,
            work.investigation(),
            source.subject(),
            source.strength(),
            resolve_improved_evidence_reliability(source.reliability()),
            source.admissibility(),
        )?;
    }
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
    ensure_version_can_advance(work.version(), "investigation work")?;
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
    let investigation_advances = match (work.kind(), plan.outcome) {
        (InvestigationWorkKind::EvidenceReview, InvestigationWorkOutcome::Developed) => 2,
        (InvestigationWorkKind::EvidenceReview, InvestigationWorkOutcome::Inconclusive)
        | (InvestigationWorkKind::WitnessInterview, InvestigationWorkOutcome::Inconclusive) => 1,
        (InvestigationWorkKind::WitnessInterview, InvestigationWorkOutcome::Connected) => 3,
        (InvestigationWorkKind::EvidenceReview, InvestigationWorkOutcome::Connected)
        | (InvestigationWorkKind::WitnessInterview, InvestigationWorkOutcome::Developed) => {
            return Err(InvestigationWorkError::StaleResolutionContext { work: plan.work });
        }
    };
    ensure_version_can_advance_by(
        investigation.version(),
        investigation_advances,
        "investigation",
    )?;
    match work.focus().witness_id() {
        Some(case_witness) => {
            let witness = state
                .legal
                .get_case_witness(case_witness)
                .ok_or(InvestigationWorkError::InvalidFocus)?;
            if Some(witness.version()) != plan.expected_case_witness_version {
                return Err(InvestigationWorkError::StaleResolutionContext { work: plan.work });
            }
            let witness_advances = if plan.outcome == InvestigationWorkOutcome::Connected {
                2
            } else {
                1
            };
            ensure_version_can_advance_by(witness.version(), witness_advances, "case witness")?;
            if witness.interview_attempts().checked_add(1).is_none() {
                return Err(InvestigationWorkError::WitnessInterviewAttemptCapacity {
                    witness: case_witness,
                });
            }
        }
        None if plan.expected_case_witness_version.is_some() => {
            return Err(InvestigationWorkError::StaleResolutionContext { work: plan.work });
        }
        None => {}
    }
    crate::core::time::ensure_time_current(state.now(), plan.resolved_at).map_err(
        |(expected, found)| InvestigationWorkError::StaleResolutionTime { expected, found },
    )?;
    Ok(())
}

pub(crate) fn find_due_scheduled_investigation_work(state: &AppState) -> Vec<InvestigationWorkId> {
    state
        .legal
        .find_investigation_work_due_at_or_before(state.now())
}
