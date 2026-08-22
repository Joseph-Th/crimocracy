//! Evidence-backed arrest and custody lifecycle transactions.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, InvestigationId, InvestigationWorkId,
    OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::legal::{
    ArrestDraft, ArrestRecord, ArrestStatus, InvestigationStatus, InvestigationWorkStatus,
};
use crate::operations::OperationStatus;
use crate::world::{Lifecycle, OrganizationKind};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ArrestError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {0} is not active and cannot enter a new custody record")]
    InactiveCharacter(CharacterId),
    #[error("arrest evidence {evidence} is too weak for custody: strength {strength:?}, reliability {reliability:?}")]
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
    #[error("character {character} is assigned to active operation {operation}")]
    ActiveOperationResponsibility {
        character: CharacterId,
        operation: OperationId,
    },
    #[error("character {character} owns scheduled investigation work {work}")]
    ScheduledInvestigationWork {
        character: CharacterId,
        work: InvestigationWorkId,
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
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("arrest {0} is not an active detention")]
    NotDetained(ArrestId),
    #[error("arrest {arrest} changed after release validation; expected version {expected}, found {found}")]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug)]
pub struct ValidatedArrest {
    draft: ArrestDraft,
    authority: OrganizationId,
    expected_investigation_version: u32,
    expected_character_version: u32,
}

impl ValidatedArrest {
    pub fn commit(self, state: &mut AppState) -> Result<ArrestId, ArrestError> {
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

        let id = state.ids.next_arrest()?;
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
    Ok(ValidatedArrest {
        draft,
        authority,
        expected_investigation_version: investigation.version(),
        expected_character_version: character.version(),
    })
}

fn validate_arrest_dependencies(
    state: &AppState,
    draft: &ArrestDraft,
) -> Result<OrganizationId, ArrestError> {
    let character = state
        .world
        .get_character(draft.character)
        .ok_or(ArrestError::MissingCharacter(draft.character))?;
    if character.lifecycle() != Lifecycle::Active {
        return Err(ArrestError::InactiveCharacter(draft.character));
    }
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
    if authority_record.kind() != OrganizationKind::LawEnforcement
        || authority_record.lifecycle() != Lifecycle::Active
    {
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
        if evidence.admissibility() == crate::legal::Admissibility::Inadmissible {
            return Err(ArrestError::InadmissibleEvidence {
                evidence: *evidence_id,
                admissibility: evidence.admissibility(),
            });
        }
        if evidence.strength() == crate::legal::EvidenceStrength::Weak {
            return Err(ArrestError::InsufficientEvidence {
                evidence: *evidence_id,
                strength: evidence.strength(),
                reliability: evidence.reliability(),
            });
        }
    }

    validate_character_can_enter_custody(state, draft.character)?;
    Ok(authority)
}

fn validate_character_can_enter_custody(
    state: &AppState,
    character: CharacterId,
) -> Result<(), ArrestError> {
    if let Some(work) = state
        .legal
        .work_for_investigator(character)
        .find(|work| work.status() == InvestigationWorkStatus::Scheduled)
    {
        return Err(ArrestError::ScheduledInvestigationWork {
            character,
            work: work.id(),
        });
    }
    for operation in state.operations.operations() {
        if matches!(
            operation.status(),
            OperationStatus::Authorized
                | OperationStatus::InProgress
                | OperationStatus::AwaitingDecision
        ) && (operation.leader() == character
            || operation
                .roles()
                .values()
                .any(|participant| *participant == character))
        {
            return Err(ArrestError::ActiveOperationResponsibility {
                character,
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedRelease {
    arrest: ArrestId,
    expected_version: u32,
}

impl ValidatedRelease {
    pub fn commit(self, state: &mut AppState) -> Result<(), ArrestError> {
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

/// Evidence bar for the autonomous conversion step: at least two qualifying items, at
/// least one of them Strong or Direct. Qualifying evidence targets the subject directly,
/// is held by the case's own authority, and is neither inadmissible nor weak. This is a
/// deliberately conservative institutional gate — it consumes case evidence that already
/// exists; it never generates new leads.
const MIN_ARREST_QUALIFYING_EVIDENCE: usize = 2;

/// Runs the police institution's evidence-to-custody conversion across operation-originated
/// cases: when an identified subject has enough admissible non-weak evidence against them,
/// the owning authority makes the arrest through the canonical validated path. Subjects who
/// currently hold any non-terminal operation booking are left alone until their work ends.
pub fn apply_autonomous_evidence_arrests(
    state: &mut AppState,
) -> Result<Vec<ArrestId>, ArrestError> {
    let candidates: Vec<(InvestigationId, CharacterId)> = state
        .legal()
        .investigations()
        .filter(|investigation| {
            investigation.status() == InvestigationStatus::Active
                && investigation.origin_operation().is_some()
        })
        .flat_map(|investigation| {
            investigation
                .subjects()
                .iter()
                .filter_map(|subject| match subject {
                    EntityRef::Character(character) => Some((investigation.id(), *character)),
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
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // One pass over the live operation set per tick, not per candidate: every participant
    // bound to a non-terminal operation is protected from custody conversion.
    let mut booked_characters = std::collections::BTreeSet::new();
    for status in [
        OperationStatus::Authorized,
        OperationStatus::InProgress,
        OperationStatus::AwaitingDecision,
    ] {
        for operation in state.operations().operations_with_status(status) {
            booked_characters.extend(operation.participants().iter().copied());
        }
    }

    let mut arrests = Vec::new();
    for (investigation_id, character) in candidates {
        // A detained character may not hold any non-terminal operation booking; skip
        // suspects whose crew work is still live rather than tearing it up mid-flight.
        if booked_characters.contains(&character) {
            continue;
        }
        if state.legal.active_arrest_for_character(character).is_some() {
            continue;
        }

        let investigation = match state.legal.get_investigation(investigation_id) {
            Some(investigation) if investigation.status() == InvestigationStatus::Active => {
                investigation
            }
            _ => continue,
        };
        let owner = investigation.owner();
        let qualifying: Vec<EvidenceId> = investigation
            .evidence()
            .iter()
            .filter_map(|id| state.legal.get_evidence(*id))
            .filter(|evidence| evidence.subject() == EntityRef::Character(character))
            .filter(|evidence| evidence.custodian() == owner)
            .filter(|evidence| evidence.strength() != crate::legal::EvidenceStrength::Weak)
            .filter(|evidence| {
                evidence.admissibility() != crate::legal::Admissibility::Inadmissible
            })
            .map(|evidence| evidence.id())
            .collect();
        let has_strong = qualifying.iter().any(|id| {
            matches!(
                state
                    .legal
                    .get_evidence(*id)
                    .map(crate::legal::EvidenceRecord::strength),
                Some(crate::legal::EvidenceStrength::Strong)
                    | Some(crate::legal::EvidenceStrength::Direct)
            )
        });
        if qualifying.len() < MIN_ARREST_QUALIFYING_EVIDENCE || !has_strong {
            continue;
        }

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
