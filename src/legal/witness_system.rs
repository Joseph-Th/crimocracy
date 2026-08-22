//! Case-witness registration, cooperation, and named testimony transactions; anonymous testimony remains ordinary incident evidence.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    CaseWitnessId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId,
    WitnessStatementId,
};
use crate::core::state::AppState;
use crate::legal::{
    Admissibility, CaseWitnessDraft, CaseWitnessRecord, EvidenceAssessment, EvidenceConnection,
    EvidenceIdentity, EvidenceKind, EvidenceRecord, EvidenceReliability, EvidenceStrength,
    InvestigationStatus, WitnessCooperation, WitnessStatementDraft, WitnessStatementRecord,
};
use crate::world::Lifecycle;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WitnessError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {0} is not active and cannot be registered as a current witness")]
    InactiveWitness(CharacterId),
    #[error("character {witness} is already registered as case witness {existing} for investigation {investigation}")]
    DuplicateCaseWitness {
        investigation: InvestigationId,
        witness: CharacterId,
        existing: CaseWitnessId,
    },
    #[error("case witness {0} does not exist")]
    MissingCaseWitness(CaseWitnessId),
    #[error("witness statement summary must not be empty")]
    EmptyStatement,
    #[error("witness statement references missing entity {0:?}")]
    MissingEntity(EntityRef),
    #[error("case witness {witness} already has cooperation state {cooperation:?}")]
    CooperationUnchanged {
        witness: CaseWitnessId,
        cooperation: WitnessCooperation,
    },
    #[error("investigation {investigation} changed after witness validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("character {witness} changed after witness validation; expected version {expected}, found {found}")]
    StaleWitnessCharacter {
        witness: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("case witness {witness} changed after validation; expected version {expected}, found {found}")]
    StaleCaseWitness {
        witness: CaseWitnessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
        let id = state.ids.next_case_witness()?;
        state.legal.insert_case_witness(
            CaseWitnessRecord {
                id,
                investigation: self.draft.investigation,
                witness: self.draft.witness,
                cooperation: self.draft.cooperation,
                registered_at: state.now(),
                statements: Default::default(),
                version: 1,
            },
            state.now(),
        );
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
    let witness = state
        .world
        .get_character(draft.witness)
        .ok_or(WitnessError::MissingCharacter(draft.witness))?;
    if witness.lifecycle() != Lifecycle::Active {
        return Err(WitnessError::InactiveWitness(draft.witness));
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

#[derive(Debug)]
pub struct ValidatedWitnessCooperation {
    case_witness: CaseWitnessId,
    cooperation: WitnessCooperation,
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessCooperation {
    pub fn commit(self, state: &mut AppState) -> Result<(), WitnessError> {
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
        state
            .legal
            .set_witness_cooperation(self.case_witness, self.cooperation, state.now());
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
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessStatement {
    pub fn commit(self, state: &mut AppState) -> Result<WitnessStatementOutcome, WitnessError> {
        state
            .ids
            .reserve_many(&[(IdKind::WitnessStatement, 1), (IdKind::Evidence, 1)])?;
        let (investigation_id, witness_id, cooperation) = {
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
                case_witness.cooperation(),
            )
        };

        let statement = state.ids.next_witness_statement()?;
        let evidence = state.ids.next_evidence()?;
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("validated witness investigation must exist");
        let recorded_at = state.now();
        state.legal.insert_evidence(
            EvidenceRecord {
                identity: EvidenceIdentity {
                    id: evidence,
                    investigation: investigation_id,
                    custodian: investigation.owner(),
                },
                connection: EvidenceConnection {
                    subject: self.draft.subject,
                    origin: self.draft.origin,
                    source: Some(EntityRef::Character(witness_id)),
                    derived_from: Default::default(),
                },
                assessment: EvidenceAssessment {
                    kind: EvidenceKind::WitnessTestimony,
                    strength: resolve_witness_strength(self.draft.confidence, cooperation),
                    reliability: resolve_witness_reliability(self.draft.confidence, cooperation),
                    admissibility: Admissibility::Unknown,
                },
                discovered_at: recorded_at,
            },
            recorded_at,
        );
        state
            .legal
            .insert_witness_statement(WitnessStatementRecord {
                id: statement,
                case_witness: self.draft.case_witness,
                subject: self.draft.subject,
                origin: self.draft.origin,
                confidence: self.draft.confidence,
                cooperation,
                summary: self.draft.summary,
                evidence,
                recorded_at,
            });
        Ok(WitnessStatementOutcome {
            statement,
            evidence,
        })
    }
}

pub fn validate_record_witness_statement(
    state: &AppState,
    draft: WitnessStatementDraft,
) -> Result<ValidatedWitnessStatement, WitnessError> {
    let case_witness = validate_case_witness_for_active_case(state, draft.case_witness)?;
    validate_statement_dependencies(state, case_witness, &draft)?;
    let investigation = state
        .legal
        .get_investigation(case_witness.investigation())
        .expect("validated witness investigation must exist");
    Ok(ValidatedWitnessStatement {
        draft,
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
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(investigation.id()));
    }
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
    Ok(witness)
}

fn validate_statement_dependencies(
    state: &AppState,
    case_witness: &CaseWitnessRecord,
    draft: &WitnessStatementDraft,
) -> Result<(), WitnessError> {
    if draft.summary.trim().is_empty() {
        return Err(WitnessError::EmptyStatement);
    }
    if !is_entity_present(state, draft.subject) {
        return Err(WitnessError::MissingEntity(draft.subject));
    }
    if let Some(origin) = draft.origin {
        if !is_entity_present(state, origin) {
            return Err(WitnessError::MissingEntity(origin));
        }
    }
    validate_current_witness_character(state, case_witness.witness())?;
    Ok(())
}

fn validate_current_witness_character(
    state: &AppState,
    witness: CharacterId,
) -> Result<(), WitnessError> {
    let record = state
        .world
        .get_character(witness)
        .ok_or(WitnessError::MissingCharacter(witness))?;
    if record.lifecycle() != Lifecycle::Active {
        return Err(WitnessError::InactiveWitness(witness));
    }
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

fn confidence_strength_band(confidence: crate::world::Rating) -> usize {
    match confidence.value() {
        0..=34 => 0,
        35..=59 => 1,
        60..=84 => 2,
        85..=100 => 3,
        _ => unreachable!("rating values are bounded by Rating::MAX"),
    }
}

fn confidence_reliability_band(confidence: crate::world::Rating) -> usize {
    match confidence.value() {
        0..=24 => 0,
        25..=49 => 1,
        50..=79 => 2,
        80..=100 => 3,
        _ => unreachable!("rating values are bounded by Rating::MAX"),
    }
}

/// Hostile testimony loses two qualification bands, reluctant one; bands never fall below Weak.
fn discount_band(band: usize, cooperation: WitnessCooperation) -> usize {
    match cooperation {
        WitnessCooperation::Cooperative => band,
        WitnessCooperation::Reluctant => band.saturating_sub(1),
        WitnessCooperation::Hostile => band.saturating_sub(2),
    }
}

pub(crate) fn resolve_witness_strength(
    confidence: crate::world::Rating,
    cooperation: WitnessCooperation,
) -> EvidenceStrength {
    STRENGTH_BANDS[discount_band(confidence_strength_band(confidence), cooperation)]
}

pub(crate) fn resolve_witness_reliability(
    confidence: crate::world::Rating,
    cooperation: WitnessCooperation,
) -> EvidenceReliability {
    RELIABILITY_BANDS[discount_band(confidence_reliability_band(confidence), cooperation)]
}

#[cfg(test)]
mod tests;
