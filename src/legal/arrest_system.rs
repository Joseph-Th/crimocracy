//! Evidence-backed arrest and custody lifecycle transactions, including atomic preemption of
//! operation, investigation, prosecution, and retained-counsel responsibilities.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId, OperationId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::decisions::decision_system::{
    DecisionError, ValidatedOperationDecisionCancellation,
    validate_cancel_operation_decision_for_detention,
};
use crate::legal::investigation_system::{
    InvestigationError, ValidatedInvestigatorDetentionRelease,
    validate_release_investigator_for_detention,
};
use crate::legal::investigation_work_execution::{
    InvestigationWorkError, ValidatedInvestigationWorkCancellation,
    validate_cancel_investigation_work_for_detention,
};
use crate::legal::legal_representation_system::{
    LegalRepresentationError, ValidatedCounselDetentionEnds,
    validate_end_representations_for_counsel_detention,
};
use crate::legal::prosecution_system::{
    ProsecutionStaffingError, ValidatedProsecutorDetentionRelease,
    validate_release_prosecution_cases_for_detention,
};
use crate::legal::{ArrestDraft, ArrestRecord, ArrestStatus, InvestigationStatus};
use crate::operations::operation_abort::{
    ValidatedOperationAbort, validate_participant_detention_abort_operation,
};
use crate::operations::operation_system::OperationError;
use crate::world::OrganizationKind;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ArrestError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error(
        "arrest evidence {evidence} is too weak for custody: strength {strength:?}, reliability {reliability:?}"
    )]
    InsufficientEvidence {
        evidence: EvidenceId,
        strength: crate::legal::EvidenceStrength,
        reliability: crate::legal::EvidenceReliability,
    },
    #[error("arrest evidence {evidence} is inadmissible and cannot justify custody")]
    InadmissibleEvidence {
        evidence: EvidenceId,
        admissibility: crate::legal::Admissibility,
    },
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("investigation owner {0} is not an active law-enforcement authority")]
    InvalidAuthority(OrganizationId),
    #[error("character {character} is not a subject of investigation {investigation}")]
    CharacterNotSubject {
        character: CharacterId,
        investigation: InvestigationId,
    },
    #[error("arrest must cite at least one evidence record")]
    NoEvidence,
    #[error("evidence {0} does not exist")]
    MissingEvidence(EvidenceId),
    #[error("evidence {evidence} does not belong to investigation {investigation}")]
    EvidenceInvestigationMismatch {
        evidence: EvidenceId,
        investigation: InvestigationId,
    },
    #[error("evidence {evidence} is not held by arresting authority {authority}")]
    EvidenceCustodianMismatch {
        evidence: EvidenceId,
        authority: OrganizationId,
    },
    #[error("evidence {evidence} does not identify character {character} as its subject")]
    EvidenceSubjectMismatch {
        evidence: EvidenceId,
        character: CharacterId,
    },
    #[error("character {character} is already detained under arrest {arrest}")]
    AlreadyDetained {
        character: CharacterId,
        arrest: ArrestId,
    },
    #[error(
        "investigation {investigation} changed after arrest validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "character {character} changed after arrest validation; expected version {expected}, found {found}"
    )]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("arrest was validated at {expected:?}, but simulation time is now {found:?}")]
    StaleArrestTime { expected: SimTime, found: SimTime },
    #[error("character {character}'s live responsibilities changed after arrest validation")]
    CustodyResponsibilitiesChanged { character: CharacterId },
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("arrest {0} is not an active detention")]
    NotDetained(ArrestId),
    #[error(
        "arrest {arrest} changed after release validation; expected version {expected}, found {found}"
    )]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    InvestigationWork(#[from] InvestigationWorkError),
    #[error(transparent)]
    Investigation(#[from] InvestigationError),
    #[error(transparent)]
    ProsecutionStaffing(#[from] ProsecutionStaffingError),
    #[error(transparent)]
    LegalRepresentation(#[from] LegalRepresentationError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

struct ValidatedCustodyOperationPreemption {
    abort: ValidatedOperationAbort,
    decision_cancellation: Option<ValidatedOperationDecisionCancellation>,
}

impl std::fmt::Debug for ValidatedCustodyOperationPreemption {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedCustodyOperationPreemption")
            .field("operation", &self.abort.operation())
            .field(
                "decision",
                &self
                    .decision_cancellation
                    .as_ref()
                    .map(ValidatedOperationDecisionCancellation::decision),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct ValidatedArrest {
    draft: ArrestDraft,
    authority: OrganizationId,
    expected_investigation_version: u32,
    expected_character_version: u32,
    validated_at: SimTime,
    work_cancellation: Option<ValidatedInvestigationWorkCancellation>,
    lead_release: Option<ValidatedInvestigatorDetentionRelease>,
    prosecution_release: ValidatedProsecutorDetentionRelease,
    counsel_representation_ends: ValidatedCounselDetentionEnds,
    operation_preemptions: Vec<ValidatedCustodyOperationPreemption>,
}

impl ValidatedArrest {
    pub fn commit(self, state: &mut AppState) -> Result<ArrestId, ArrestError> {
        if state.now() != self.validated_at {
            return Err(ArrestError::StaleArrestTime {
                expected: self.validated_at,
                found: state.now(),
            });
        }
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .ok_or(ArrestError::MissingInvestigation(self.draft.investigation))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(ArrestError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        let character = state
            .world
            .get_character(self.draft.character)
            .ok_or(ArrestError::MissingCharacter(self.draft.character))?;
        if character.version() != self.expected_character_version {
            return Err(ArrestError::StaleCharacter {
                character: self.draft.character,
                expected: self.expected_character_version,
                found: character.version(),
            });
        }
        let authority = validate_arrest_dependencies(state, &self.draft)?;
        debug_assert_eq!(authority, self.authority);

        let current_work = scheduled_work_for_investigator(state, self.draft.character);
        let expected_work = self.work_cancellation.as_ref().map(|token| token.work());
        let current_lead_case = state
            .legal
            .active_investigation_for_investigator(self.draft.character)
            .map(|investigation| investigation.id());
        let expected_lead_case = self
            .lead_release
            .as_ref()
            .map(ValidatedInvestigatorDetentionRelease::investigation);
        let current_operations =
            active_operation_bookings_for_character(state, self.draft.character);
        let expected_operations: Vec<OperationId> = self
            .operation_preemptions
            .iter()
            .map(|preemption| preemption.abort.operation())
            .collect();
        if current_work != expected_work
            || current_lead_case != expected_lead_case
            || current_operations != expected_operations
        {
            return Err(ArrestError::CustodyResponsibilitiesChanged {
                character: self.draft.character,
            });
        }

        if let Some(work) = &self.work_cancellation {
            work.ensure_current(state)?;
        }
        if let Some(release) = &self.lead_release {
            release.ensure_current(state)?;
        }
        self.prosecution_release.ensure_current(state)?;
        self.counsel_representation_ends.ensure_current(state)?;
        for preemption in &self.operation_preemptions {
            if let Some(decision) = &preemption.decision_cancellation {
                decision.ensure_current(state)?;
            }
            preemption.abort.ensure_current(state)?;
        }

        let mut id_budget = vec![(IdKind::Arrest, 1)];
        for preemption in &self.operation_preemptions {
            id_budget.extend(preemption.abort.id_budget());
        }
        id_budget.extend(self.counsel_representation_ends.id_budget());
        state.ids.reserve_many(&id_budget)?;

        let id = state
            .ids
            .next_arrest()
            .expect("arrest ID was preflighted before custody mutation");
        for preemption in self.operation_preemptions {
            if let Some(decision) = preemption.decision_cancellation {
                decision.commit_preflighted(state);
            }
            preemption.abort.commit_preflighted(state);
        }
        if let Some(work) = self.work_cancellation {
            work.commit_preflighted(state, id);
        }
        if let Some(release) = self.lead_release {
            release.commit_preflighted(state);
        }
        self.prosecution_release.commit_preflighted(state);
        self.counsel_representation_ends.commit_preflighted(state);
        state.legal.insert_arrest(ArrestRecord {
            id,
            character: self.draft.character,
            authority,
            investigation: self.draft.investigation,
            evidence: self.draft.evidence,
            arrested_at: state.now(),
            released_at: None,
            status: ArrestStatus::Detained,
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_arrest(
    state: &AppState,
    draft: ArrestDraft,
) -> Result<ValidatedArrest, ArrestError> {
    let authority = validate_arrest_dependencies(state, &draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must exist");
    let character = state
        .world
        .get_character(draft.character)
        .expect("validated arrest character must exist");
    let work_cancellation =
        validate_cancel_investigation_work_for_detention(state, draft.character)?;
    let lead_release = validate_release_investigator_for_detention(state, draft.character)?;
    let prosecution_release =
        validate_release_prosecution_cases_for_detention(state, draft.character)?;
    let counsel_representation_ends =
        validate_end_representations_for_counsel_detention(state, draft.character)?;
    let mut operation_preemptions = Vec::new();
    for operation in active_operation_bookings_for_character(state, draft.character) {
        let decision_cancellation =
            validate_cancel_operation_decision_for_detention(state, operation, draft.character)?;
        let abort =
            validate_participant_detention_abort_operation(state, operation, draft.character)?;
        operation_preemptions.push(ValidatedCustodyOperationPreemption {
            abort,
            decision_cancellation,
        });
    }
    Ok(ValidatedArrest {
        draft,
        authority,
        expected_investigation_version: investigation.version(),
        expected_character_version: character.version(),
        validated_at: state.now(),
        work_cancellation,
        lead_release,
        prosecution_release,
        counsel_representation_ends,
        operation_preemptions,
    })
}

fn validate_arrest_dependencies(
    state: &AppState,
    draft: &ArrestDraft,
) -> Result<OrganizationId, ArrestError> {
    let _ = state
        .world
        .get_character(draft.character)
        .ok_or(ArrestError::MissingCharacter(draft.character))?;
    if let Some(existing) = state.legal.active_arrest_for_character(draft.character) {
        return Err(ArrestError::AlreadyDetained {
            character: draft.character,
            arrest: existing.id(),
        });
    }

    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(ArrestError::MissingInvestigation(draft.investigation))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(ArrestError::InactiveInvestigation(draft.investigation));
    }
    if !investigation
        .subjects()
        .contains(&EntityRef::Character(draft.character))
    {
        return Err(ArrestError::CharacterNotSubject {
            character: draft.character,
            investigation: draft.investigation,
        });
    }
    let authority = investigation.owner();
    let authority_record = state
        .world
        .get_organization(authority)
        .ok_or(ArrestError::InvalidAuthority(authority))?;
    if authority_record.kind() != OrganizationKind::LawEnforcement {
        return Err(ArrestError::InvalidAuthority(authority));
    }

    if draft.evidence.is_empty() {
        return Err(ArrestError::NoEvidence);
    }
    for evidence_id in &draft.evidence {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
        if evidence.investigation() != draft.investigation {
            return Err(ArrestError::EvidenceInvestigationMismatch {
                evidence: *evidence_id,
                investigation: draft.investigation,
            });
        }
        if evidence.custodian() != authority {
            return Err(ArrestError::EvidenceCustodianMismatch {
                evidence: *evidence_id,
                authority,
            });
        }
        if evidence.subject() != EntityRef::Character(draft.character) {
            return Err(ArrestError::EvidenceSubjectMismatch {
                evidence: *evidence_id,
                character: draft.character,
            });
        }
        if !evidence_qualifies_for_custody(evidence) {
            if evidence.admissibility() == crate::legal::Admissibility::Inadmissible {
                return Err(ArrestError::InadmissibleEvidence {
                    evidence: *evidence_id,
                    admissibility: evidence.admissibility(),
                });
            }
            return Err(ArrestError::InsufficientEvidence {
                evidence: *evidence_id,
                strength: evidence.strength(),
                reliability: evidence.reliability(),
            });
        }
    }

    Ok(authority)
}

/// Custody is a stronger consequence than adding a subject to a case graph. Weak material or a
/// source the institution itself still considers Questionable can remain useful investigative
/// input, but neither can justify detention. Unknown or disputed admissibility remains usable at
/// the arrest stage unless the material is already known to be inadmissible; admissibility is a
/// separate legal axis and newly gathered testimony routinely starts as Unknown.
fn has_minimum_custody_quality(
    strength: crate::legal::EvidenceStrength,
    reliability: crate::legal::EvidenceReliability,
) -> bool {
    strength != crate::legal::EvidenceStrength::Weak
        && reliability != crate::legal::EvidenceReliability::Questionable
}

/// Single semantic predicate for evidence that may support custody. Runtime arrest validation,
/// autonomous arrest selection, and persistence invariants all consume this owner so a save can
/// never restore an arrest that the canonical transaction would reject.
pub(crate) fn evidence_qualifies_for_custody(evidence: &crate::legal::EvidenceRecord) -> bool {
    evidence.admissibility() != crate::legal::Admissibility::Inadmissible
        && has_minimum_custody_quality(evidence.strength(), evidence.reliability())
}

fn active_operation_bookings_for_character(
    state: &AppState,
    character: CharacterId,
) -> Vec<OperationId> {
    state
        .operations
        .active_operation_bookings(character)
        .collect()
}

fn scheduled_work_for_investigator(
    state: &AppState,
    character: CharacterId,
) -> Option<crate::core::id::InvestigationWorkId> {
    state
        .legal
        .work_for_investigator(character)
        .find(|work| work.status() == crate::legal::InvestigationWorkStatus::Scheduled)
        .map(|work| work.id())
}

#[derive(Debug)]
pub struct ValidatedRelease {
    arrest: ArrestId,
    expected_version: u32,
}

impl ValidatedRelease {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), ArrestError> {
        let record = state
            .legal
            .get_arrest(self.arrest)
            .ok_or(ArrestError::MissingArrest(self.arrest))?;
        if record.version() != self.expected_version {
            return Err(ArrestError::StaleArrest {
                arrest: self.arrest,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != ArrestStatus::Detained {
            return Err(ArrestError::NotDetained(self.arrest));
        }
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), ArrestError> {
        self.ensure_current(state)?;
        state.legal.release_arrest(self.arrest, state.now());
        Ok(())
    }
}

pub fn validate_release_arrest(
    state: &AppState,
    arrest: ArrestId,
) -> Result<ValidatedRelease, ArrestError> {
    let record = state
        .legal
        .get_arrest(arrest)
        .ok_or(ArrestError::MissingArrest(arrest))?;
    if record.status() != ArrestStatus::Detained {
        return Err(ArrestError::NotDetained(arrest));
    }
    Ok(ValidatedRelease {
        arrest,
        expected_version: record.version(),
    })
}

/// Releases detainees whose modeled custody window has elapsed. The current legal foundation
/// intentionally stops before charging, bail, and trial, so an arrest cannot imply permanent
/// confinement merely because no higher legal layer exists to advance it. The authored window
/// is long enough for the detainee informant decision to occur first.
pub(crate) fn apply_due_custody_releases(
    state: &mut AppState,
    maximum_detention: crate::core::time::SimDuration,
) -> Result<Vec<ArrestId>, ArrestError> {
    let due: Vec<ArrestId> = state
        .legal
        .detained_arrests()
        .filter(|arrest| state.now() >= arrest.arrested_at() + maximum_detention)
        .map(|arrest| arrest.id())
        .collect();
    for arrest in &due {
        // IDs came from the authoritative detained index in this same pass. A rejection here
        // is therefore broken current state, not an ordinary race to ignore.
        validate_release_arrest(state, *arrest)?.commit(state)?;
    }
    Ok(due)
}

/// Evidence bar for the autonomous conversion step: at least two qualifying items, at
/// least one of them Strong or Direct. Qualifying evidence targets the subject directly,
/// is held by the case's own authority, is not known inadmissible, and meets the same minimum
/// strength/reliability floor as the canonical arrest path. This is a
/// deliberately conservative institutional gate — it consumes case evidence that already
/// exists; it never generates new leads.
const MIN_ARREST_QUALIFYING_EVIDENCE: usize = 2;

/// Runs the police institution's evidence-to-custody conversion across active cases: when an
/// identified subject has enough usable independent evidence against them,
/// the owning authority makes the arrest through the canonical validated path. Custody preempts
/// conflicting operation bookings and scheduled detective work through their explicit abort or
/// cancellation lifecycles, so internal commitments cannot make an arrestable subject immune.
pub fn apply_autonomous_evidence_arrests(
    state: &mut AppState,
) -> Result<Vec<ArrestId>, ArrestError> {
    // Single scan over active cases. Case provenance controls lifecycle and information flow,
    // not whether equally strong evidence can produce custody; subject pairs append directly so
    // a tick with no candidates allocates nothing beyond the one candidate buffer.
    let mut candidates: Vec<(InvestigationId, CharacterId)> = Vec::new();
    for investigation in state.legal().active_investigations() {
        // Legal authorities can own investigative files, but only law-enforcement institutions
        // have custody authority. Skip non-police case owners here instead of turning a valid
        // legal-authority case with strong evidence into a tick-failing InvalidAuthority error.
        if !state
            .world()
            .get_organization(investigation.owner())
            .is_some_and(|owner| owner.kind() == OrganizationKind::LawEnforcement)
        {
            continue;
        }
        let investigation_id = investigation.id();
        candidates.extend(
            investigation
                .subjects()
                .iter()
                .filter_map(|subject| match subject {
                    EntityRef::Character(character) => Some((investigation_id, *character)),
                    EntityRef::Organization(_)
                    | EntityRef::Neighborhood(_)
                    | EntityRef::Business(_)
                    | EntityRef::Operation(_)
                    | EntityRef::Investigation(_)
                    | EntityRef::Evidence(_)
                    | EntityRef::FinancialAccount(_)
                    | EntityRef::DecisionRequest(_)
                    | EntityRef::Mandate(_)
                    | EntityRef::Enterprise(_) => None,
                }),
        );
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut arrests = Vec::new();
    for (investigation_id, character) in candidates {
        if state.legal.active_arrest_for_character(character).is_some() {
            continue;
        }
        // Autonomous custody is a conservative one-time conversion for one case/person pair.
        // Once that detention has ended, unchanged case evidence must not manufacture an
        // arrest-release-arrest loop every authored custody window. A later deliberate re-arrest
        // remains available through `validate_arrest`; a distinct investigation can also make
        // its own autonomous custody decision.
        if state
            .legal
            .arrests_for_investigation(investigation_id)
            .any(|arrest| arrest.character() == character)
        {
            continue;
        }

        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .ok_or(ArrestError::MissingInvestigation(investigation_id))?;
        if investigation.status() != InvestigationStatus::Active {
            // Another arrest earlier in this pass can make a duplicate character candidate
            // irrelevant, but this pass itself never transitions investigations. An inactive
            // record in the active-case candidate snapshot therefore signals a broken index.
            return Err(ArrestError::InactiveInvestigation(investigation_id));
        }
        let owner = investigation.owner();
        // Single lookup per evidence record decides qualification and the strong/direct
        // bar together, instead of resolving each qualifying item a second time.
        let mut qualifying: Vec<EvidenceId> = Vec::new();
        let mut has_strong = false;
        for evidence_id in investigation.evidence().iter() {
            let evidence = state
                .legal
                .get_evidence(*evidence_id)
                .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
            if evidence.subject() != EntityRef::Character(character)
                || evidence.custodian() != owner
                || !evidence_qualifies_for_custody(evidence)
            {
                continue;
            }
            // Corroboration means independent facts: a forensic analysis derived from case
            // evidence restates its source's subject and strength, so counting both would let
            // one underlying fact satisfy the two-item bar by itself. Derivatives still
            // strengthen the institution's hand through improved reliability and later
            // informant or witness work; they just cannot be their own second witness.
            if !evidence.derived_from().is_empty() {
                continue;
            }
            has_strong |= matches!(
                evidence.strength(),
                crate::legal::EvidenceStrength::Strong | crate::legal::EvidenceStrength::Direct
            );
            qualifying.push(evidence.id());
        }
        if qualifying.len() < MIN_ARREST_QUALIFYING_EVIDENCE || !has_strong {
            continue;
        }

        // The draft is assembled from current case/evidence state. Responsibility preemption is
        // part of the validated custody transaction, so validation or allocation failure is
        // exceptional and must surface rather than being mistaken for "not enough evidence yet".
        let arrest = validate_arrest(
            state,
            ArrestDraft {
                character,
                investigation: investigation_id,
                evidence: qualifying.into_iter().collect(),
            },
        )?
        .commit(state)?;
        arrests.push(arrest);
    }
    Ok(arrests)
}

#[cfg(test)]
mod tests;
