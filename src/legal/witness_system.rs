//! Case-witness registration, cooperation, and named testimony transactions; anonymous testimony remains ordinary incident evidence.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    CaseWitnessId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId,
    ProsecutionCaseId, WitnessStatementId,
};
use crate::core::state::AppState;
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
use crate::legal::{
    Admissibility, CaseWitnessDraft, CaseWitnessRecord, EvidenceAssessment, EvidenceConnection,
    EvidenceIdentity, EvidenceKind, EvidenceRecord, EvidenceReliability, EvidenceStrength,
    InvestigationStatus, WitnessCooperation, WitnessStatementDraft, WitnessStatementRecord,
};
use crate::registry::{Registry, WitnessTestimonyDefinition};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WitnessError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error(
        "character {witness} is already registered as case witness {existing} for investigation {investigation}"
    )]
    DuplicateCaseWitness {
        investigation: InvestigationId,
        witness: CharacterId,
        existing: CaseWitnessId,
    },
    #[error(
        "character {witness} is a subject of investigation {investigation} and cannot act as its witness"
    )]
    WitnessIsCaseSubject {
        investigation: InvestigationId,
        witness: CharacterId,
    },
    #[error(
        "character {witness} leads investigation {investigation} and cannot also be its named witness"
    )]
    WitnessIsLeadInvestigator {
        investigation: InvestigationId,
        witness: CharacterId,
    },
    #[error(
        "character {witness} is assigned to prosecution case {case} sourced from investigation {investigation} and cannot also be its named witness"
    )]
    WitnessIsAssignedProsecutor {
        investigation: InvestigationId,
        witness: CharacterId,
        case: ProsecutionCaseId,
    },
    #[error(
        "character {witness} cannot be registered to testify about unrelated subject {subject:?} in investigation {investigation}"
    )]
    WitnessSubjectOutsideCase {
        investigation: InvestigationId,
        witness: CharacterId,
        subject: EntityRef,
    },
    #[error("case witness {0} does not exist")]
    MissingCaseWitness(CaseWitnessId),
    #[error("case witness {0} already has a recorded statement")]
    WitnessAlreadyStatemented(CaseWitnessId),
    #[error("witness statement summary must not be empty")]
    EmptyStatement,
    #[error("witness statement references missing entity {0:?}")]
    MissingEntity(EntityRef),
    #[error("case witness {witness} already has cooperation state {cooperation:?}")]
    CooperationUnchanged {
        witness: CaseWitnessId,
        cooperation: WitnessCooperation,
    },
    #[error(
        "investigation {investigation} changed after witness validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "character {witness} changed after witness validation; expected version {expected}, found {found}"
    )]
    StaleWitnessCharacter {
        witness: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "case witness {witness} changed after validation; expected version {expected}, found {found}"
    )]
    StaleCaseWitness {
        witness: CaseWitnessId,
        expected: u32,
        found: u32,
    },
    #[error("lead case-witness knowledge could not be recorded: {0}")]
    CaseKnowledge(#[from] crate::intelligence::intelligence_system::IntelligenceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub struct ValidatedCaseWitnessRegistration {
    draft: CaseWitnessDraft,
    expected_investigation_version: u32,
    expected_character_version: u32,
}

impl ValidatedCaseWitnessRegistration {
    pub fn commit(self, state: &mut AppState) -> Result<CaseWitnessId, WitnessError> {
        validate_registration_snapshot(
            state,
            self.draft,
            self.expected_investigation_version,
            self.expected_character_version,
        )?;
        let lead = state
            .legal
            .get_investigation(self.draft.investigation)
            .expect("validated investigation must still exist")
            .lead_investigator();
        let knowledge = match lead {
            Some(lead) => crate::legal::case_knowledge::prepare_case_witness_knowledge(
                state,
                self.draft.investigation,
                self.draft.witness,
                lead,
            )?,
            None => None,
        };
        state.ids.reserve_many(&[
            (IdKind::CaseWitness, 1),
            (IdKind::Information, u32::from(knowledge.is_some())),
        ])?;
        let id = state.ids.next_case_witness()?;
        state.legal.insert_case_witness(
            CaseWitnessRecord {
                id,
                investigation: self.draft.investigation,
                witness: self.draft.witness,
                subject: self.draft.subject,
                cooperation: self.draft.cooperation,
                registered_at: state.now(),
                statements: Default::default(),
                interview_attempts: 0,
                version: 1,
            },
            state.now(),
        );
        if let Some(knowledge) = knowledge {
            knowledge
                .commit(state)
                .expect("case-witness information ID was preflighted before registration mutation");
        }
        Ok(id)
    }
}

pub fn validate_register_case_witness(
    state: &AppState,
    draft: CaseWitnessDraft,
) -> Result<ValidatedCaseWitnessRegistration, WitnessError> {
    validate_registration_dependencies(state, draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must still exist");
    ensure_version_can_advance(investigation.version(), "investigation")?;
    let witness = state
        .world
        .get_character(draft.witness)
        .expect("validated witness character must still exist");
    Ok(ValidatedCaseWitnessRegistration {
        draft,
        expected_investigation_version: investigation.version(),
        expected_character_version: witness.version(),
    })
}

fn validate_registration_snapshot(
    state: &AppState,
    draft: CaseWitnessDraft,
    expected_investigation_version: u32,
    expected_character_version: u32,
) -> Result<(), WitnessError> {
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(WitnessError::MissingInvestigation(draft.investigation))?;
    if investigation.version() != expected_investigation_version {
        return Err(WitnessError::StaleInvestigation {
            investigation: draft.investigation,
            expected: expected_investigation_version,
            found: investigation.version(),
        });
    }
    ensure_version_can_advance(investigation.version(), "investigation")?;
    let witness = state
        .world
        .get_character(draft.witness)
        .ok_or(WitnessError::MissingCharacter(draft.witness))?;
    if witness.version() != expected_character_version {
        return Err(WitnessError::StaleWitnessCharacter {
            witness: draft.witness,
            expected: expected_character_version,
            found: witness.version(),
        });
    }
    validate_registration_dependencies(state, draft)
}

fn validate_registration_dependencies(
    state: &AppState,
    draft: CaseWitnessDraft,
) -> Result<(), WitnessError> {
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(WitnessError::MissingInvestigation(draft.investigation))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(draft.investigation));
    }
    let _ = state
        .world
        .get_character(draft.witness)
        .ok_or(WitnessError::MissingCharacter(draft.witness))?;
    if investigation
        .subjects()
        .contains(&EntityRef::Character(draft.witness))
    {
        return Err(WitnessError::WitnessIsCaseSubject {
            investigation: draft.investigation,
            witness: draft.witness,
        });
    }
    match case_witness_role_conflict(state, draft.investigation, draft.witness) {
        Some(CaseWitnessRoleConflict::LeadInvestigator) => {
            return Err(WitnessError::WitnessIsLeadInvestigator {
                investigation: draft.investigation,
                witness: draft.witness,
            });
        }
        Some(CaseWitnessRoleConflict::AssignedProsecutor(case)) => {
            return Err(WitnessError::WitnessIsAssignedProsecutor {
                investigation: draft.investigation,
                witness: draft.witness,
                case,
            });
        }
        None => {}
    }
    if !is_entity_present(state, draft.subject) {
        return Err(WitnessError::MissingEntity(draft.subject));
    }
    if !witness_subject_is_case_relevant(state, investigation, draft.subject, None) {
        return Err(WitnessError::WitnessSubjectOutsideCase {
            investigation: draft.investigation,
            witness: draft.witness,
            subject: draft.subject,
        });
    }
    if let Some(existing) = state
        .legal
        .case_witness_for(draft.investigation, draft.witness)
    {
        return Err(WitnessError::DuplicateCaseWitness {
            investigation: draft.investigation,
            witness: draft.witness,
            existing: existing.id(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseWitnessRoleConflict {
    LeadInvestigator,
    AssignedProsecutor(ProsecutionCaseId),
}

/// Current institutional roles that conflict with acting as a named factual witness in the same
/// investigation. Historical investigator/prosecutor participation is not represented here:
/// once the current role ends, a later witness registration may be legitimate.
pub(crate) fn case_witness_role_conflict(
    state: &AppState,
    investigation: InvestigationId,
    character: CharacterId,
) -> Option<CaseWitnessRoleConflict> {
    if state
        .legal
        .get_investigation(investigation)
        .is_some_and(|record| record.lead_investigator() == Some(character))
    {
        return Some(CaseWitnessRoleConflict::LeadInvestigator);
    }
    state
        .legal
        .reviewing_prosecution_cases_for_prosecutor(character)
        .find(|case| case.source_investigation() == investigation)
        .map(|case| CaseWitnessRoleConflict::AssignedProsecutor(case.id()))
}

#[derive(Debug)]
pub struct ValidatedWitnessCooperation {
    case_witness: CaseWitnessId,
    cooperation: WitnessCooperation,
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessCooperation {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), WitnessError> {
        let witness = validate_witness_mutation_snapshot(
            state,
            self.case_witness,
            self.expected_witness_version,
            self.expected_investigation_version,
        )?;
        validate_current_witness_character(state, witness.witness())?;
        if witness.cooperation() == self.cooperation {
            return Err(WitnessError::CooperationUnchanged {
                witness: self.case_witness,
                cooperation: self.cooperation,
            });
        }
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), WitnessError> {
        self.ensure_current(state)?;
        state
            .legal
            .set_witness_cooperation(self.case_witness, self.cooperation);
        Ok(())
    }
}

pub fn validate_set_witness_cooperation(
    state: &AppState,
    case_witness: CaseWitnessId,
    cooperation: WitnessCooperation,
) -> Result<ValidatedWitnessCooperation, WitnessError> {
    let witness = validate_case_witness_for_active_case(state, case_witness)?;
    validate_current_witness_character(state, witness.witness())?;
    if witness.cooperation() == cooperation {
        return Err(WitnessError::CooperationUnchanged {
            witness: case_witness,
            cooperation,
        });
    }
    let investigation = state
        .legal
        .get_investigation(witness.investigation())
        .expect("validated case witness investigation must exist");
    ensure_version_can_advance(witness.version(), "case witness")?;
    ensure_version_can_advance(investigation.version(), "investigation")?;
    Ok(ValidatedWitnessCooperation {
        case_witness,
        cooperation,
        expected_witness_version: witness.version(),
        expected_investigation_version: investigation.version(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WitnessStatementOutcome {
    pub statement: WitnessStatementId,
    pub evidence: EvidenceId,
}

#[derive(Debug)]
pub struct ValidatedWitnessStatement {
    draft: WitnessStatementDraft,
    testimony: WitnessTestimonyDefinition,
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessStatement {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), WitnessError> {
        let case_witness = validate_witness_mutation_snapshot(
            state,
            self.draft.case_witness,
            self.expected_witness_version,
            self.expected_investigation_version,
        )?;
        validate_statement_dependencies(state, case_witness, &self.draft)?;
        let investigation = state
            .legal
            .get_investigation(case_witness.investigation())
            .expect("validated witness investigation must exist");
        ensure_version_can_advance_by(investigation.version(), 2, "investigation")?;
        crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
            state,
            case_witness.investigation(),
            case_witness.subject(),
            resolve_witness_strength(
                self.testimony,
                self.draft.confidence,
                case_witness.cooperation(),
            ),
            resolve_witness_reliability(
                self.testimony,
                self.draft.confidence,
                case_witness.cooperation(),
            ),
            Admissibility::Unknown,
        )?;
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<WitnessStatementOutcome, WitnessError> {
        self.commit_with_originating_work(state, None)
    }

    pub(crate) fn commit_from_investigation_work(
        self,
        state: &mut AppState,
        originating_work: crate::core::id::InvestigationWorkId,
    ) -> Result<WitnessStatementOutcome, WitnessError> {
        self.commit_with_originating_work(state, Some(originating_work))
    }

    fn commit_with_originating_work(
        self,
        state: &mut AppState,
        originating_work: Option<crate::core::id::InvestigationWorkId>,
    ) -> Result<WitnessStatementOutcome, WitnessError> {
        state
            .ids
            .reserve_many(&[(IdKind::WitnessStatement, 1), (IdKind::Evidence, 1)])?;
        self.ensure_current(state)?;
        let (investigation_id, witness_id, subject, cooperation) = {
            let case_witness = validate_witness_mutation_snapshot(
                state,
                self.draft.case_witness,
                self.expected_witness_version,
                self.expected_investigation_version,
            )?;
            validate_statement_dependencies(state, case_witness, &self.draft)?;
            (
                case_witness.investigation(),
                case_witness.witness(),
                case_witness.subject(),
                case_witness.cooperation(),
            )
        };
        let statement = state
            .ids
            .next_witness_statement()
            .expect("witness-statement ID was preflighted before mutation");
        let evidence = state
            .ids
            .next_evidence()
            .expect("witness-testimony evidence ID was preflighted before mutation");
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("validated witness investigation must exist");
        let recorded_at = state.now();
        let evidence_record = EvidenceRecord {
            identity: EvidenceIdentity {
                id: evidence,
                investigation: investigation_id,
                custodian: investigation.owner(),
            },
            connection: EvidenceConnection {
                subject,
                origin: self.draft.origin,
                source: Some(EntityRef::Character(witness_id)),
                derived_from: Default::default(),
            },
            assessment: EvidenceAssessment {
                kind: EvidenceKind::WitnessTestimony,
                strength: resolve_witness_strength(
                    self.testimony,
                    self.draft.confidence,
                    cooperation,
                ),
                reliability: resolve_witness_reliability(
                    self.testimony,
                    self.draft.confidence,
                    cooperation,
                ),
                admissibility: Admissibility::Unknown,
            },
            discovered_at: recorded_at,
        };
        match originating_work {
            Some(work) => state.legal.insert_evidence_from_investigation_work(
                evidence_record,
                recorded_at,
                work,
            ),
            None => state.legal.insert_evidence(evidence_record, recorded_at),
        }
        let statement_record = WitnessStatementRecord {
            id: statement,
            case_witness: self.draft.case_witness,
            subject,
            origin: self.draft.origin,
            confidence: self.draft.confidence,
            cooperation,
            summary: self.draft.summary,
            evidence,
            recorded_at,
        };
        match originating_work {
            Some(work) => state
                .legal
                .insert_witness_statement_from_investigation_work(statement_record, work),
            None => state.legal.insert_witness_statement(statement_record),
        }
        Ok(WitnessStatementOutcome {
            statement,
            evidence,
        })
    }
}

pub fn validate_record_witness_statement(
    registry: &Registry,
    state: &AppState,
    draft: WitnessStatementDraft,
) -> Result<ValidatedWitnessStatement, WitnessError> {
    let case_witness = validate_case_witness_for_active_case(state, draft.case_witness)?;
    validate_statement_dependencies(state, case_witness, &draft)?;
    let investigation = state
        .legal
        .get_investigation(case_witness.investigation())
        .expect("validated witness investigation must exist");
    ensure_version_can_advance_by(investigation.version(), 2, "investigation")?;
    crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
        state,
        case_witness.investigation(),
        case_witness.subject(),
        resolve_witness_strength(
            registry.legal().witness_testimony(),
            draft.confidence,
            case_witness.cooperation(),
        ),
        resolve_witness_reliability(
            registry.legal().witness_testimony(),
            draft.confidence,
            case_witness.cooperation(),
        ),
        Admissibility::Unknown,
    )?;
    Ok(ValidatedWitnessStatement {
        draft,
        testimony: registry.legal().witness_testimony(),
        expected_witness_version: case_witness.version(),
        expected_investigation_version: investigation.version(),
    })
}

fn validate_witness_mutation_snapshot(
    state: &AppState,
    case_witness_id: CaseWitnessId,
    expected_witness_version: u32,
    expected_investigation_version: u32,
) -> Result<&CaseWitnessRecord, WitnessError> {
    let case_witness = state
        .legal
        .get_case_witness(case_witness_id)
        .ok_or(WitnessError::MissingCaseWitness(case_witness_id))?;
    if case_witness.version() != expected_witness_version {
        return Err(WitnessError::StaleCaseWitness {
            witness: case_witness_id,
            expected: expected_witness_version,
            found: case_witness.version(),
        });
    }
    ensure_version_can_advance(case_witness.version(), "case witness")?;
    let investigation = state
        .legal
        .get_investigation(case_witness.investigation())
        .ok_or(WitnessError::MissingInvestigation(
            case_witness.investigation(),
        ))?;
    if investigation.version() != expected_investigation_version {
        return Err(WitnessError::StaleInvestigation {
            investigation: investigation.id(),
            expected: expected_investigation_version,
            found: investigation.version(),
        });
    }
    ensure_version_can_advance(investigation.version(), "investigation")?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(investigation.id()));
    }
    validate_witness_not_case_subject(investigation, case_witness.witness())?;
    Ok(case_witness)
}

fn validate_case_witness_for_active_case(
    state: &AppState,
    case_witness: CaseWitnessId,
) -> Result<&CaseWitnessRecord, WitnessError> {
    let witness = state
        .legal
        .get_case_witness(case_witness)
        .ok_or(WitnessError::MissingCaseWitness(case_witness))?;
    let investigation = state
        .legal
        .get_investigation(witness.investigation())
        .ok_or(WitnessError::MissingInvestigation(witness.investigation()))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(investigation.id()));
    }
    validate_witness_not_case_subject(investigation, witness.witness())?;
    Ok(witness)
}

fn validate_witness_not_case_subject(
    investigation: &crate::legal::InvestigationRecord,
    witness: CharacterId,
) -> Result<(), WitnessError> {
    if investigation
        .subjects()
        .contains(&EntityRef::Character(witness))
    {
        return Err(WitnessError::WitnessIsCaseSubject {
            investigation: investigation.id(),
            witness,
        });
    }
    Ok(())
}

/// Current role-conflict predicate shared by investigation-work and witness-pressure consumers.
/// A witness registration is historical and remains valid after later case development, but a
/// character who has become an arrest-eligible subject cannot continue supplying witness actions
/// in that same case.
pub(crate) fn case_witness_is_case_subject(
    state: &AppState,
    case_witness: &CaseWitnessRecord,
) -> bool {
    state
        .legal
        .get_investigation(case_witness.investigation())
        .is_some_and(|investigation| {
            investigation
                .subjects()
                .contains(&EntityRef::Character(case_witness.witness()))
        })
}

fn validate_statement_dependencies(
    state: &AppState,
    case_witness: &CaseWitnessRecord,
    draft: &WitnessStatementDraft,
) -> Result<(), WitnessError> {
    if !case_witness.statements().is_empty() {
        return Err(WitnessError::WitnessAlreadyStatemented(case_witness.id()));
    }
    if draft.summary.trim().is_empty() {
        return Err(WitnessError::EmptyStatement);
    }
    if let Some(origin) = draft.origin
        && !is_entity_present(state, origin)
    {
        return Err(WitnessError::MissingEntity(origin));
    }
    validate_current_witness_character(state, case_witness.witness())?;
    Ok(())
}

/// A witness may be bound only to declared case subject matter, the originating event, or an
/// entity already connected by other evidence. Registration uses this predicate before persisting
/// that binding. Restore validation supplies `excluded_evidence` to ignore the witness's own
/// testimony evidence, preventing circular justification of a binding that promoted its subject.
pub(crate) fn witness_subject_is_case_relevant(
    state: &AppState,
    investigation: &crate::legal::InvestigationRecord,
    subject: EntityRef,
    excluded_evidence: Option<EvidenceId>,
) -> bool {
    investigation.origin() == Some(subject)
        || investigation.declared_subjects().contains(&subject)
        || investigation.evidence().iter().copied().any(|evidence_id| {
            Some(evidence_id) != excluded_evidence
                && state
                    .legal
                    .get_evidence(evidence_id)
                    .is_some_and(|evidence| evidence.subject() == subject)
        })
}

fn validate_current_witness_character(
    state: &AppState,
    witness: CharacterId,
) -> Result<(), WitnessError> {
    let _ = state
        .world
        .get_character(witness)
        .ok_or(WitnessError::MissingCharacter(witness))?;
    Ok(())
}

/// Confidence bands qualify raw witness certainty. Cooperation then discounts the
/// assessment: uncooperative witnesses face pressure to minimize their own involvement,
/// so a hostile account corroborates at best and a reluctant one cannot carry a case alone.
const STRENGTH_BANDS: [EvidenceStrength; 4] = [
    EvidenceStrength::Weak,
    EvidenceStrength::Corroborating,
    EvidenceStrength::Strong,
    EvidenceStrength::Direct,
];
const RELIABILITY_BANDS: [EvidenceReliability; 4] = [
    EvidenceReliability::Questionable,
    EvidenceReliability::Mixed,
    EvidenceReliability::Credible,
    EvidenceReliability::HighlyReliable,
];

fn confidence_band(confidence: crate::world::Rating, thresholds: [u8; 3]) -> usize {
    thresholds
        .into_iter()
        .filter(|minimum| confidence.value() >= *minimum)
        .count()
}

fn discount_band(
    definition: WitnessTestimonyDefinition,
    band: usize,
    cooperation: WitnessCooperation,
) -> usize {
    band.saturating_sub(usize::from(
        definition.cooperation_band_discount(cooperation),
    ))
}

pub(crate) fn resolve_witness_strength(
    definition: WitnessTestimonyDefinition,
    confidence: crate::world::Rating,
    cooperation: WitnessCooperation,
) -> EvidenceStrength {
    STRENGTH_BANDS[discount_band(
        definition,
        confidence_band(confidence, definition.strength_thresholds()),
        cooperation,
    )]
}

pub(crate) fn resolve_witness_reliability(
    definition: WitnessTestimonyDefinition,
    confidence: crate::world::Rating,
    cooperation: WitnessCooperation,
) -> EvidenceReliability {
    RELIABILITY_BANDS[discount_band(
        definition,
        confidence_band(confidence, definition.reliability_thresholds()),
        cooperation,
    )]
}

#[cfg(test)]
mod tests;
