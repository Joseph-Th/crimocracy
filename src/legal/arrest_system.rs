//! Evidence-backed arrest and custody lifecycle transactions, including atomic preemption of
//! operation, investigation, prosecution, and retained-counsel responsibilities.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId, OperationId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
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
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::{BTreeMap, BTreeSet};
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
    #[error(
        "arrest requires at least {required} independent qualifying evidence sources; found {found}"
    )]
    InsufficientIndependentEvidence { found: usize, required: u8 },
    #[error("arrest requires at least one independent Strong or Direct evidence source")]
    NoStrongIndependentEvidence,
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
        "character {character} was released from arrest {prior_arrest} in investigation {investigation} at {released_at:?}; renewed custody requires a later custody minute and qualifying evidence from the release minute or later"
    )]
    RepeatCustodyWithoutNewEvidence {
        character: CharacterId,
        investigation: InvestigationId,
        prior_arrest: ArrestId,
        released_at: SimTime,
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
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

/// The latest instant custody may remain active. The simulation clock is finite, so an authored
/// detention window extending beyond it ends at the last representable minute rather than
/// becoming an immortal detention because `SimTime + duration` overflowed.
pub(crate) fn custody_release_at(arrested_at: SimTime, maximum_detention: SimDuration) -> SimTime {
    arrested_at
        .checked_add(maximum_detention)
        .unwrap_or(SimTime::from_minutes(u64::MAX))
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
    minimum_qualifying_evidence: u8,
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
        let authority =
            validate_arrest_dependencies(state, &self.draft, self.minimum_qualifying_evidence)?;
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
            preemptible_operation_bookings_for_character(state, self.draft.character);
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
        ensure_custody_investigation_version_budget(
            state,
            self.work_cancellation.as_ref(),
            self.lead_release.as_ref(),
        )?;
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
    registry: &Registry,
    state: &AppState,
    draft: ArrestDraft,
) -> Result<ValidatedArrest, ArrestError> {
    let minimum_qualifying_evidence = registry.legal().minimum_arrest_qualifying_evidence();
    let authority = validate_arrest_dependencies(state, &draft, minimum_qualifying_evidence)?;
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
    ensure_custody_investigation_version_budget(
        state,
        work_cancellation.as_ref(),
        lead_release.as_ref(),
    )?;
    let prosecution_release =
        validate_release_prosecution_cases_for_detention(state, draft.character)?;
    let counsel_representation_ends =
        validate_end_representations_for_counsel_detention(state, draft.character)?;
    let mut operation_preemptions = Vec::new();
    for operation in preemptible_operation_bookings_for_character(state, draft.character) {
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
        minimum_qualifying_evidence,
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

/// Custody can cancel a detective's scheduled work and release the same case's lead seat in one
/// transaction. Both effects advance that investigation, so their shared finite version budget
/// must be checked as a composite rather than as two individually valid one-step mutations.
fn ensure_custody_investigation_version_budget(
    state: &AppState,
    work_cancellation: Option<&ValidatedInvestigationWorkCancellation>,
    lead_release: Option<&ValidatedInvestigatorDetentionRelease>,
) -> Result<(), ArrestError> {
    let (Some(work_cancellation), Some(lead_release)) = (work_cancellation, lead_release) else {
        return Ok(());
    };
    let work = state
        .legal
        .get_investigation_work(work_cancellation.work())
        .ok_or(InvestigationWorkError::MissingWork(
            work_cancellation.work(),
        ))?;
    if work.investigation() != lead_release.investigation() {
        return Ok(());
    }
    let investigation = state.legal.get_investigation(work.investigation()).ok_or(
        InvestigationError::MissingInvestigation(work.investigation()),
    )?;
    ensure_version_can_advance_by(investigation.version(), 2, "investigation")?;
    Ok(())
}

fn validate_arrest_dependencies(
    state: &AppState,
    draft: &ArrestDraft,
    minimum_qualifying_evidence: u8,
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
    let mut assessment = CustodyEvidenceAssessment::default();
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
        assessment.observe(state, evidence);
    }

    if assessment.independent_qualifying() < usize::from(minimum_qualifying_evidence) {
        return Err(ArrestError::InsufficientIndependentEvidence {
            found: assessment.independent_qualifying(),
            required: minimum_qualifying_evidence,
        });
    }
    if !assessment.has_strong_independent() {
        return Err(ArrestError::NoStrongIndependentEvidence);
    }
    validate_repeat_custody_evidence(state, draft)?;

    Ok(authority)
}

fn latest_released_arrest_for_case_character(
    state: &AppState,
    investigation: InvestigationId,
    character: CharacterId,
) -> Option<&ArrestRecord> {
    state
        .legal
        .arrests_for_investigation(investigation)
        .filter(|arrest| {
            arrest.character() == character && arrest.status() == ArrestStatus::Released
        })
        .max_by_key(|arrest| arrest.id())
}

fn validate_repeat_custody_evidence(
    state: &AppState,
    draft: &ArrestDraft,
) -> Result<(), ArrestError> {
    let Some((prior_arrest, released_at)) = repeat_custody_without_new_evidence(
        state,
        draft.investigation,
        draft.character,
        &draft.evidence,
    ) else {
        return Ok(());
    };
    Err(ArrestError::RepeatCustodyWithoutNewEvidence {
        character: draft.character,
        investigation: draft.investigation,
        prior_arrest,
        released_at,
    })
}

fn repeat_custody_without_new_evidence(
    state: &AppState,
    investigation: InvestigationId,
    character: CharacterId,
    evidence: &BTreeSet<EvidenceId>,
) -> Option<(ArrestId, SimTime)> {
    let prior = latest_released_arrest_for_case_character(state, investigation, character)?;
    let released_at = prior
        .released_at()
        .expect("released arrest must retain its release instant");
    (state.now() <= released_at
        || !evidence.iter().any(|evidence| {
            state
                .legal
                .get_evidence(*evidence)
                .is_some_and(|record| record.discovered_at() >= released_at)
        }))
    .then_some((prior.id(), released_at))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CustodyCorroborationSource {
    /// Witness and informant evidence carry explicit source provenance. Multiple statements
    /// from the same source may add useful detail, but they remain one corroborating source.
    Named(EntityRef),
    /// Source-less primary evidence is the model's abstraction for a distinct physical,
    /// documentary, surveillance, or other independently gathered fact.
    PrimaryEvidence(EvidenceId),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CustodyEvidenceAssessment {
    sources: std::collections::BTreeMap<CustodyCorroborationSource, bool>,
}

impl CustodyEvidenceAssessment {
    fn corroboration_source(
        state: &AppState,
        evidence: &crate::legal::EvidenceRecord,
    ) -> Option<CustodyCorroborationSource> {
        if !evidence_qualifies_for_custody(evidence) {
            return None;
        }
        let source_evidence = match evidence.derived_from().len() {
            0 => evidence,
            1 => {
                let source_id = *evidence
                    .derived_from()
                    .iter()
                    .next()
                    .expect("single-source derived evidence must name its source");
                state.legal.get_evidence(source_id)?
            }
            _ => return None,
        };
        Some(
            source_evidence
                .source()
                .map(CustodyCorroborationSource::Named)
                .unwrap_or(CustodyCorroborationSource::PrimaryEvidence(
                    source_evidence.id(),
                )),
        )
    }

    fn observe(&mut self, state: &AppState, evidence: &crate::legal::EvidenceRecord) -> bool {
        let Some(source) = Self::corroboration_source(state, evidence) else {
            return false;
        };
        let strong = matches!(
            evidence.strength(),
            crate::legal::EvidenceStrength::Strong | crate::legal::EvidenceStrength::Direct
        );
        self.sources
            .entry(source)
            .and_modify(|has_strong| *has_strong |= strong)
            .or_insert(strong);
        true
    }

    fn independent_qualifying(&self) -> usize {
        self.sources.len()
    }

    fn has_strong_independent(&self) -> bool {
        self.sources.values().any(|has_strong| *has_strong)
    }

    fn strong_independent(&self) -> usize {
        self.sources
            .values()
            .filter(|has_strong| **has_strong)
            .count()
    }

    fn meets(&self, minimum_qualifying_evidence: u8) -> bool {
        self.independent_qualifying() >= usize::from(minimum_qualifying_evidence)
            && self.has_strong_independent()
    }
}

#[derive(Debug)]
struct AutonomousArrestCandidate {
    investigation: InvestigationId,
    character: CharacterId,
    evidence: BTreeSet<EvidenceId>,
    independent_sources: usize,
    strong_sources: usize,
}

fn resolve_autonomous_arrest_candidate(
    registry: &crate::registry::Registry,
    state: &AppState,
    investigation_id: InvestigationId,
    character: CharacterId,
) -> Result<Option<AutonomousArrestCandidate>, ArrestError> {
    if state.legal.active_arrest_for_character(character).is_some() {
        return Ok(None);
    }
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(ArrestError::MissingInvestigation(investigation_id))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(ArrestError::InactiveInvestigation(investigation_id));
    }
    let owner = investigation.owner();
    let mut assessment = CustodyEvidenceAssessment::default();
    let mut citations: BTreeMap<CustodyCorroborationSource, EvidenceId> = BTreeMap::new();
    for evidence_id in investigation.evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
        if evidence.subject() != EntityRef::Character(character) || evidence.custodian() != owner {
            continue;
        }
        let Some(source) = CustodyEvidenceAssessment::corroboration_source(state, evidence) else {
            continue;
        };
        assessment.observe(state, evidence);
        // Cite one qualifying record per independent source. Prefer the primary source when it
        // is itself custody-grade; otherwise a developed derivative must stand in for the weak
        // or questionable original that it legitimately improved. Evidence review preserves
        // source strength, so a custody-grade primary cannot discard a Strong/Direct property
        // that exists only on its derivative.
        let citation = match evidence.derived_from().iter().next().copied() {
            Some(source_id) => state
                .legal
                .get_evidence(source_id)
                .filter(|source_evidence| evidence_qualifies_for_custody(source_evidence))
                .map_or(evidence.id(), crate::legal::EvidenceRecord::id),
            None => evidence.id(),
        };
        citations
            .entry(source)
            .and_modify(|current| *current = (*current).min(citation))
            .or_insert(citation);
    }
    if !assessment.meets(registry.legal().minimum_arrest_qualifying_evidence()) {
        return Ok(None);
    }
    let evidence: BTreeSet<_> = citations.into_values().collect();
    if repeat_custody_without_new_evidence(state, investigation_id, character, &evidence).is_some()
    {
        return Ok(None);
    }
    Ok(Some(AutonomousArrestCandidate {
        investigation: investigation_id,
        character,
        evidence,
        independent_sources: assessment.independent_qualifying(),
        strong_sources: assessment.strong_independent(),
    }))
}

/// Registry-aware corroboration rule used by persistence validation. Canonical direct and
/// autonomous arrest paths use the same `CustodyEvidenceAssessment` owner while assembling their
/// evidence. Derived evidence may be cited on a direct arrest but cannot manufacture an
/// independent fact.
/// Multiple witness/informant records naming the same explicit source count once; source-less
/// primary records represent distinct gathered facts. At least one independent source must carry
/// Strong or Direct weight.
pub(crate) fn arrest_evidence_meets_threshold(
    state: &AppState,
    evidence_ids: &std::collections::BTreeSet<EvidenceId>,
    minimum_qualifying_evidence: u8,
) -> bool {
    let mut assessment = CustodyEvidenceAssessment::default();
    for evidence_id in evidence_ids {
        let Some(evidence) = state.legal.get_evidence(*evidence_id) else {
            return false;
        };
        assessment.observe(state, evidence);
    }
    assessment.meets(minimum_qualifying_evidence)
}

/// Single semantic predicate for evidence that may support custody. Runtime arrest validation,
/// autonomous arrest selection, and persistence invariants all consume this owner so a save can
/// never restore an arrest that the canonical transaction would reject.
pub(crate) fn evidence_qualifies_for_custody(evidence: &crate::legal::EvidenceRecord) -> bool {
    evidence.admissibility() != crate::legal::Admissibility::Inadmissible
        && has_minimum_custody_quality(evidence.strength(), evidence.reliability())
}

fn preemptible_operation_bookings_for_character(
    state: &AppState,
    character: CharacterId,
) -> Vec<OperationId> {
    state
        .operations
        .active_operations_for_participant(character)
        .filter(|operation| {
            matches!(
                operation.status(),
                crate::operations::OperationStatus::Authorized
                    | crate::operations::OperationStatus::InProgress
                    | crate::operations::OperationStatus::AwaitingDecision
            )
        })
        .map(|operation| operation.id())
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
    pub(crate) fn arrest(&self) -> ArrestId {
        self.arrest
    }

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
        ensure_version_can_advance(record.version(), "arrest")?;
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
    ensure_version_can_advance(record.version(), "arrest")?;
    Ok(ValidatedRelease {
        arrest,
        expected_version: record.version(),
    })
}

/// Releases detainees whose modeled custody window has elapsed. Prosecution referral and review
/// can continue after release, but charging, bail, trial, and sentence custody remain outside the
/// current legal foundation, so an arrest cannot imply permanent confinement merely because no
/// modeled court-custody layer exists to advance it. The authored window is long enough for the
/// detainee informant decision to occur first.
pub(crate) fn apply_due_custody_releases(
    state: &mut AppState,
    maximum_detention: SimDuration,
) -> Result<Vec<ArrestId>, ArrestError> {
    let due: Vec<ArrestId> = state
        .legal
        .detained_arrests()
        .filter(|arrest| state.now() >= custody_release_at(arrest.arrested_at(), maximum_detention))
        .map(|arrest| arrest.id())
        .collect();
    for arrest in &due {
        // IDs came from the authoritative detained index in this same pass. A rejection here
        // is therefore broken current state, not an ordinary race to ignore.
        validate_release_arrest(state, *arrest)?.commit(state)?;
    }
    Ok(due)
}

/// Evidence bar for the autonomous conversion step: the authored number of independent
/// qualifying sources, with at least one Strong or Direct. Qualifying evidence targets the subject directly,
/// is held by the case's own authority, is not known inadmissible, and meets the same minimum
/// strength/reliability floor as the canonical arrest path. This is a
/// deliberately conservative institutional gate — it consumes case evidence that already
/// exists; it never generates new leads.
/// Runs the police institution's evidence-to-custody conversion across active cases: when an
/// identified subject has enough usable independent evidence against them,
/// the owning authority makes the arrest through the canonical validated path. Custody preempts
/// conflicting operation bookings and scheduled detective work through their explicit abort or
/// cancellation lifecycles, so internal commitments cannot make an arrestable subject immune.
pub fn apply_autonomous_evidence_arrests(
    registry: &crate::registry::Registry,
    state: &mut AppState,
) -> Result<Vec<ArrestId>, ArrestError> {
    // Single scan over active cases. Case provenance controls lifecycle and information flow,
    // not whether equally strong evidence can produce custody; subject pairs append directly so
    // a tick with no candidates allocates nothing beyond the one candidate buffer.
    let mut case_subjects: Vec<(InvestigationId, CharacterId)> = Vec::new();
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
        case_subjects.extend(
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
    if case_subjects.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for (investigation, character) in case_subjects {
        if let Some(candidate) =
            resolve_autonomous_arrest_candidate(registry, state, investigation, character)?
        {
            candidates.push(candidate);
        }
    }
    // One character can be arrestable in several police files at once, but only one active
    // detention can own them. Prefer the case with more independent qualifying sources, then
    // more Strong/Direct independent sources. Stable case id breaks only a true evidentiary tie
    // instead of silently rewarding whichever investigation happened to be opened first.
    candidates.sort_unstable_by_key(|candidate| {
        (
            candidate.character,
            std::cmp::Reverse(candidate.independent_sources),
            std::cmp::Reverse(candidate.strong_sources),
            candidate.investigation,
        )
    });
    let mut arrests = Vec::new();
    for candidate in candidates {
        if state
            .legal
            .active_arrest_for_character(candidate.character)
            .is_some()
        {
            continue;
        }
        // The draft is assembled from current case/evidence state. Responsibility preemption is
        // part of the validated custody transaction, so validation or allocation failure is
        // exceptional and must surface rather than being mistaken for "not enough evidence yet".
        let arrest = validate_arrest(
            registry,
            state,
            ArrestDraft {
                character: candidate.character,
                investigation: candidate.investigation,
                evidence: candidate.evidence,
            },
        )?
        .commit(state)?;
        arrests.push(arrest);
    }
    Ok(arrests)
}

#[cfg(test)]
mod tests;
