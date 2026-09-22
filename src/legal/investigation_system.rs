//! Investigation transactions and autonomous case policies; sibling legal state keeps indexes synchronized.

mod autonomous_staffing;
mod cold_case_decay;
mod incident_intake;

pub(crate) use autonomous_staffing::apply_autonomous_investigator_staffing;
pub use autonomous_staffing::{ValidatedInvestigatorAssignment, validate_assign_investigator};
pub use cold_case_decay::ColdCaseDecayOutcome;
pub(crate) use cold_case_decay::apply_cold_case_decay;
pub(crate) use incident_intake::case_origin_responsible_organization;
pub use incident_intake::{
    IncidentIntakeOutcome, ValidatedIncidentIntake, validate_incident_intake,
};

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId,
    InvestigationWorkId, OrganizationId, ProsecutionCaseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::legal::{
    EvidenceAssessment, EvidenceConnection, EvidenceDraft, EvidenceIdentity, EvidenceRecord,
    InvestigationDraft, InvestigationRecord, InvestigationStatus,
};
use crate::world::OrganizationKind;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvestigationError {
    #[error("investigation title must not be empty")]
    EmptyTitle,
    #[error("investigation must have at least one subject")]
    NoSubjects,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} cannot own an investigation")]
    InvalidOwnerKind(OrganizationId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {investigator} does not belong to investigation owner {owner}")]
    InvestigatorOwnerMismatch {
        investigator: CharacterId,
        owner: OrganizationId,
    },
    #[error("character {investigator} is detained under arrest {arrest}")]
    DetainedInvestigator {
        investigator: CharacterId,
        arrest: ArrestId,
    },
    #[error(
        "character {investigator} is a subject of investigation {investigation} and cannot lead it"
    )]
    InvestigatorIsCaseSubject {
        investigation: InvestigationId,
        investigator: CharacterId,
    },
    #[error(
        "character {investigator} is named witness {witness} in investigation {investigation} and cannot lead it"
    )]
    InvestigatorIsCaseWitness {
        investigation: InvestigationId,
        investigator: CharacterId,
        witness: CaseWitnessId,
    },
    #[error("character {0} has no Investigation capability")]
    MissingInvestigationCapability(CharacterId),
    #[error("investigation {investigation} already has {lead} as its lead")]
    LeadSeatFilled {
        investigation: InvestigationId,
        lead: CharacterId,
    },
    #[error(
        "character {investigator} already leads an active investigation and cannot take another active case"
    )]
    InvestigatorAtCaseCapacity { investigator: CharacterId },
    #[error(
        "investigation {investigation} changed after validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "investigator {investigator} changed after validation; expected version {expected}, found {found}"
    )]
    StaleInvestigator {
        investigator: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("evidence discovery time cannot be in the future")]
    DiscoveryInFuture,
    #[error("evidence custodian {custodian} does not own investigation {investigation}")]
    CustodianMismatch {
        investigation: InvestigationId,
        custodian: OrganizationId,
    },
    #[error("evidence cannot be added to an inactive investigation")]
    InactiveInvestigation,
    #[error("incident intake must contain at least one evidence record")]
    NoIncidentEvidence,
    #[error("incident intake evidence set is too large to persist")]
    IncidentEvidenceCountOverflow,
    #[error(
        "incident intake names character {character} as a case subject without actionable evidence"
    )]
    UnsubstantiatedIncidentCharacterSubject { character: CharacterId },
    #[error("incident intake names {subject:?} as a case subject without matching evidence")]
    UnsubstantiatedIncidentSubject { subject: EntityRef },
    #[error(
        "incident intake resumable shelf changed after validation; expected {expected:?}, found {found:?}"
    )]
    StaleIncidentShelf {
        expected: Option<(InvestigationId, u32)>,
        found: Option<(InvestigationId, u32)>,
    },
    #[error("entity {0:?} cannot originate a case")]
    InvalidCaseOrigin(EntityRef),
    #[error("forensic-analysis evidence must be produced by canonical investigation work")]
    ForensicAnalysisRequiresInvestigationWork,
    #[error(
        "informant-statement evidence must be produced by the canonical informant disclosure path"
    )]
    InformantStatementRequiresDisclosure,
    #[error("transition {transition:?} is invalid from investigation status {status:?}")]
    InvalidInvestigationTransition {
        status: InvestigationStatus,
        transition: InvestigationTransition,
    },
    #[error(
        "investigation {investigation} has scheduled work {work} and cannot transition lifecycle"
    )]
    ScheduledWorkBlocksTransition {
        investigation: InvestigationId,
        work: InvestigationWorkId,
    },
    #[error(
        "investigation {investigation} has active arrest {arrest} and cannot transition lifecycle"
    )]
    ActiveArrestBlocksTransition {
        investigation: InvestigationId,
        arrest: ArrestId,
    },
    #[error("character {character} is a subject of this case and cannot be its named witness")]
    WitnessIsCaseSubject { character: CharacterId },
    #[error("named witness {character} cannot simultaneously lead investigation {investigation}")]
    WitnessIsLeadInvestigator {
        investigation: InvestigationId,
        character: CharacterId,
    },
    #[error(
        "named witness {character} cannot simultaneously prosecute case {case} sourced from investigation {investigation}"
    )]
    WitnessIsAssignedProsecutor {
        investigation: InvestigationId,
        character: CharacterId,
        case: ProsecutionCaseId,
    },
    #[error("named witness {character} cannot be bound to unrelated incident subject {subject:?}")]
    WitnessSubjectOutsideIncident {
        character: CharacterId,
        subject: EntityRef,
    },
    #[error(
        "character {witness} is already registered as case witness {existing} for investigation {investigation}"
    )]
    DuplicateIncidentWitness {
        investigation: InvestigationId,
        witness: CharacterId,
        existing: CaseWitnessId,
    },
    #[error("lead case-activity knowledge could not be recorded: {0}")]
    CaseKnowledge(#[from] crate::intelligence::intelligence_system::IntelligenceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub(crate) struct ValidatedInvestigatorDetentionRelease {
    investigation: InvestigationId,
    investigator: CharacterId,
    expected_investigation_version: u32,
    released_at: SimTime,
}

impl ValidatedInvestigatorDetentionRelease {
    pub(crate) fn investigation(&self) -> InvestigationId {
        self.investigation
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), InvestigationError> {
        let investigation = state
            .legal
            .get_investigation(self.investigation)
            .ok_or(InvestigationError::MissingInvestigation(self.investigation))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationError::StaleInvestigation {
                investigation: self.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        if investigation.status() != InvestigationStatus::Active
            || investigation.lead_investigator() != Some(self.investigator)
        {
            return Err(InvestigationError::InactiveInvestigation);
        }
        crate::core::time::ensure_time_current(state.now(), self.released_at)
            .map_err(|_| InvestigationError::InactiveInvestigation)?;
        ensure_version_can_advance(investigation.version(), "investigation")?;
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        state
            .legal
            .release_lead_investigator_for_detention(self.investigation, self.investigator);
    }
}

pub(crate) fn validate_release_investigator_for_detention(
    state: &AppState,
    investigator: CharacterId,
) -> Result<Option<ValidatedInvestigatorDetentionRelease>, InvestigationError> {
    let Some(investigation) = state
        .legal
        .active_investigation_for_investigator(investigator)
    else {
        return Ok(None);
    };
    ensure_version_can_advance(investigation.version(), "investigation")?;
    Ok(Some(ValidatedInvestigatorDetentionRelease {
        investigation: investigation.id(),
        investigator,
        expected_investigation_version: investigation.version(),
        released_at: state.now(),
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationTransition {
    Suspend,
    Resume,
    Close,
}

pub struct ValidatedInvestigation {
    draft: InvestigationDraft,
}
impl ValidatedInvestigation {
    pub fn commit(self, state: &mut AppState) -> Result<InvestigationId, InvestigationError> {
        validate_investigation_draft(state, &self.draft)?;
        let id = state.ids.next_investigation()?;
        let subjects = self.draft.subjects;
        state.legal.insert_investigation(InvestigationRecord {
            id,
            owner: self.draft.owner,
            title: self.draft.title,
            status: InvestigationStatus::Active,
            lead_investigator: None,
            declared_subjects: subjects.clone(),
            subjects,
            evidence: Default::default(),
            opened_at: state.now(),
            origin: None,
            last_activity_at: state.now(),
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_open_investigation(
    state: &AppState,
    draft: InvestigationDraft,
) -> Result<ValidatedInvestigation, InvestigationError> {
    validate_investigation_draft(state, &draft)?;
    Ok(ValidatedInvestigation { draft })
}

/// Evidence quality sufficient to turn a referenced entity into an actionable case subject.
/// The per-record floor matches the custody quality floor; custody additionally demands the
/// authored independent-source count with at least one Strong source, so actionability is the
/// tracking gate while corroboration remains the detention gate. Material the institution
/// itself still considers Questionable is only a lead to develop, not enough to keep a person
/// permanently tracked as an identified suspect.
pub(crate) fn evidence_assessment_is_actionable_case_lead(
    strength: crate::legal::EvidenceStrength,
    reliability: crate::legal::EvidenceReliability,
    admissibility: crate::legal::Admissibility,
) -> bool {
    strength != crate::legal::EvidenceStrength::Weak
        && reliability != crate::legal::EvidenceReliability::Questionable
        && admissibility != crate::legal::Admissibility::Inadmissible
}

/// Evidence is authoritative even when it creates an institutional conflict. Before any evidence
/// mutation, preflight the prosecution-case versions that would need to release a prosecutor who
/// becomes an actionable subject of the same source investigation.
pub(crate) fn ensure_evidence_prosecution_recusal_capacity(
    state: &AppState,
    investigation: InvestigationId,
    subject: EntityRef,
    strength: crate::legal::EvidenceStrength,
    reliability: crate::legal::EvidenceReliability,
    admissibility: crate::legal::Admissibility,
) -> Result<(), VersionCapacityError> {
    let EntityRef::Character(character) = subject else {
        return Ok(());
    };
    if !evidence_assessment_is_actionable_case_lead(strength, reliability, admissibility) {
        return Ok(());
    }
    for case in state
        .legal
        .reviewing_prosecution_cases_for_prosecutor(character)
        .filter(|case| case.source_investigation() == investigation)
    {
        ensure_version_can_advance(case.version(), "prosecution case")?;
    }
    Ok(())
}

pub(crate) fn evidence_is_actionable_case_lead(evidence: &crate::legal::EvidenceRecord) -> bool {
    evidence_assessment_is_actionable_case_lead(
        evidence.strength(),
        evidence.reliability(),
        evidence.admissibility(),
    )
}

fn validate_investigation_draft(
    state: &AppState,
    draft: &InvestigationDraft,
) -> Result<(), InvestigationError> {
    if draft.title.trim().is_empty() {
        return Err(InvestigationError::EmptyTitle);
    }
    if draft.subjects.is_empty() {
        return Err(InvestigationError::NoSubjects);
    }
    let owner = state
        .world
        .get_organization(draft.owner)
        .ok_or(InvestigationError::MissingOrganization(draft.owner))?;
    match owner.kind() {
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority => {}
        OrganizationKind::Criminal
        | OrganizationKind::LegalServices
        | OrganizationKind::Prosecutor
        | OrganizationKind::Political
        | OrganizationKind::Press
        | OrganizationKind::Labor
        | OrganizationKind::Civic
        | OrganizationKind::Commercial => {
            return Err(InvestigationError::InvalidOwnerKind(draft.owner));
        }
    }
    for subject in &draft.subjects {
        if !is_entity_present(state, *subject) {
            return Err(InvestigationError::MissingEntity(*subject));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedInvestigationTransition {
    investigation: InvestigationId,
    transition: InvestigationTransition,
    expected_version: u32,
}

struct PreparedInvestigationTransition {
    transition: ValidatedInvestigationTransition,
    next_status: InvestigationStatus,
    knowledge: Option<crate::intelligence::intelligence_system::ValidatedInformation>,
}

impl ValidatedInvestigationTransition {
    pub fn commit(self, state: &mut AppState) -> Result<(), InvestigationError> {
        let prepared = self.prepare_current(state)?;
        if prepared.knowledge.is_some() {
            state.ids.reserve(IdKind::Information, 1)?;
        }
        prepared.commit_preflighted(state);
        Ok(())
    }

    fn prepare_current(
        self,
        state: &AppState,
    ) -> Result<PreparedInvestigationTransition, InvestigationError> {
        let investigation = state
            .legal
            .get_investigation(self.investigation)
            .ok_or(InvestigationError::MissingInvestigation(self.investigation))?;
        if investigation.version() != self.expected_version {
            return Err(InvestigationError::StaleInvestigation {
                investigation: self.investigation,
                expected: self.expected_version,
                found: investigation.version(),
            });
        }
        ensure_version_can_advance(investigation.version(), "investigation")?;
        validate_investigation_transition_dependencies(state, self.investigation, self.transition)?;
        let next_status = match self.transition {
            InvestigationTransition::Suspend => InvestigationStatus::Suspended,
            InvestigationTransition::Resume => InvestigationStatus::Active,
            InvestigationTransition::Close => InvestigationStatus::Closed,
        };
        // The lead's refreshed knowledge is prepared before any mutation so a failure leaves
        // the case untouched; committing it after the status write keeps one canonical record.
        let knowledge = match investigation.lead_investigator() {
            Some(lead) => crate::legal::case_knowledge::prepare_case_activity_knowledge(
                state,
                self.investigation,
                crate::legal::case_knowledge::activity_for_status(next_status),
                lead,
            )?,
            None => None,
        };
        Ok(PreparedInvestigationTransition {
            transition: self,
            next_status,
            knowledge,
        })
    }
}

impl PreparedInvestigationTransition {
    fn commit_preflighted(self, state: &mut AppState) {
        state.legal.set_investigation_status(
            self.transition.investigation,
            self.next_status,
            state.now(),
        );
        if let Some(knowledge) = self.knowledge {
            knowledge
                .commit(state)
                .expect("case-activity information ID was preflighted before transition mutation");
        }
    }
}

pub fn validate_transition_investigation(
    state: &AppState,
    investigation: InvestigationId,
    transition: InvestigationTransition,
) -> Result<ValidatedInvestigationTransition, InvestigationError> {
    let record = state
        .legal
        .get_investigation(investigation)
        .ok_or(InvestigationError::MissingInvestigation(investigation))?;
    ensure_version_can_advance(record.version(), "investigation")?;
    validate_investigation_transition_dependencies(state, investigation, transition)?;
    Ok(ValidatedInvestigationTransition {
        investigation,
        transition,
        expected_version: record.version(),
    })
}

fn validate_investigation_transition_dependencies(
    state: &AppState,
    investigation_id: InvestigationId,
    transition: InvestigationTransition,
) -> Result<(), InvestigationError> {
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(InvestigationError::MissingInvestigation(investigation_id))?;
    let valid_transition = matches!(
        (investigation.status(), transition),
        (
            InvestigationStatus::Active,
            InvestigationTransition::Suspend
        ) | (
            InvestigationStatus::Suspended,
            InvestigationTransition::Resume
        ) | (InvestigationStatus::Active, InvestigationTransition::Close)
            | (
                InvestigationStatus::Suspended,
                InvestigationTransition::Close
            )
    );
    if !valid_transition {
        return Err(InvestigationError::InvalidInvestigationTransition {
            status: investigation.status(),
            transition,
        });
    }
    if let Some(work) = state
        .legal
        .work_for_investigation(investigation_id)
        .find(|work| work.status() == crate::legal::InvestigationWorkStatus::Scheduled)
    {
        return Err(InvestigationError::ScheduledWorkBlocksTransition {
            investigation: investigation_id,
            work: work.id(),
        });
    }
    // Suspending a case while one of its arrests still holds someone in custody would shelve
    // live institutional work, so only Resume escapes this gate. Closing stays allowed: a case
    // whose every identified subject is detained is cleared by arrest, and prosecution works
    // from the arrest and its evidence rather than from an active investigation.
    if transition == InvestigationTransition::Suspend
        && let Some(arrest) = state
            .legal
            .arrests_for_investigation(investigation_id)
            .find(|arrest| arrest.status() == crate::legal::ArrestStatus::Detained)
    {
        return Err(InvestigationError::ActiveArrestBlocksTransition {
            investigation: investigation_id,
            arrest: arrest.id(),
        });
    }
    if transition == InvestigationTransition::Resume {
        let _ = state.world.get_organization(investigation.owner()).ok_or(
            InvestigationError::MissingOrganization(investigation.owner()),
        )?;
        // No per-investigator checks are needed: shelving released the case's investigators,
        // so resumption re-enters the unstaffed index and fresh staffing re-applies the
        // one-active-case rule when the institution next works the case.
    }
    Ok(())
}

pub struct ValidatedEvidence {
    draft: EvidenceDraft,
    expected_investigation_version: u32,
}
impl ValidatedEvidence {
    pub fn commit(self, state: &mut AppState) -> Result<EvidenceId, InvestigationError> {
        validate_evidence_draft(state, &self.draft)?;
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .expect("validated evidence investigation must exist");
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        ensure_external_evidence_case_capacity(state, &self.draft)?;
        let id = state.ids.next_evidence()?;
        let EvidenceDraft {
            investigation,
            custodian,
            subject,
            origin,
            kind,
            strength,
            reliability,
            admissibility,
            discovered_at,
        } = self.draft;
        state.legal.insert_evidence(
            EvidenceRecord {
                identity: EvidenceIdentity {
                    id,
                    investigation,
                    custodian,
                },
                connection: EvidenceConnection {
                    subject,
                    origin,
                    source: None,
                    derived_from: Default::default(),
                },
                assessment: EvidenceAssessment {
                    kind,
                    strength,
                    reliability,
                    admissibility,
                },
                discovered_at,
            },
            state.now(),
        );
        Ok(id)
    }
}

pub fn validate_add_evidence(
    state: &AppState,
    draft: EvidenceDraft,
) -> Result<ValidatedEvidence, InvestigationError> {
    validate_evidence_draft(state, &draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated evidence investigation must exist");
    ensure_external_evidence_case_capacity(state, &draft)?;
    Ok(ValidatedEvidence {
        draft,
        expected_investigation_version: investigation.version(),
    })
}

/// Evidence may arrive while detective work is already scheduled. Preserve that work's complete
/// worst-case resolution budget unless this exact actionable evidence will cancel the conflicting
/// lead/witness work in the same owner mutation.
fn ensure_external_evidence_case_capacity(
    state: &AppState,
    draft: &EvidenceDraft,
) -> Result<(), VersionCapacityError> {
    let excluded_work = if evidence_assessment_is_actionable_case_lead(
        draft.strength,
        draft.reliability,
        draft.admissibility,
    ) {
        draft.subject.as_character().and_then(|character| {
            crate::legal::investigation_work_execution::
                scheduled_work_invalidated_by_actionable_character(
                    state,
                    draft.investigation,
                    character,
                )
        })
    } else {
        None
    };
    crate::legal::investigation_work_execution::ensure_external_investigation_mutation_capacity(
        state,
        draft.investigation,
        1,
        excluded_work,
    )
}

/// Work-derived evidence kinds and informant statements may only be created through their
/// canonical production paths, never hand-drafted onto a case.
fn validate_evidence_kind_allowed(
    kind: crate::legal::EvidenceKind,
) -> Result<(), InvestigationError> {
    match kind {
        crate::legal::EvidenceKind::ForensicAnalysis => {
            Err(InvestigationError::ForensicAnalysisRequiresInvestigationWork)
        }
        crate::legal::EvidenceKind::InformantStatement => {
            Err(InvestigationError::InformantStatementRequiresDisclosure)
        }
        crate::legal::EvidenceKind::WitnessTestimony
        | crate::legal::EvidenceKind::VehicleDescription
        | crate::legal::EvidenceKind::Fingerprint
        | crate::legal::EvidenceKind::RecoveredProperty
        | crate::legal::EvidenceKind::FinancialRecord
        | crate::legal::EvidenceKind::Surveillance
        | crate::legal::EvidenceKind::CommunicationRecord
        | crate::legal::EvidenceKind::KnownAssociation
        | crate::legal::EvidenceKind::Document
        | crate::legal::EvidenceKind::Ballistics => Ok(()),
    }
}

fn validate_evidence_draft(
    state: &AppState,
    draft: &EvidenceDraft,
) -> Result<(), InvestigationError> {
    validate_evidence_kind_allowed(draft.kind)?;
    let investigation = state.legal.get_investigation(draft.investigation).ok_or(
        InvestigationError::MissingInvestigation(draft.investigation),
    )?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationError::InactiveInvestigation);
    }
    let _ = state
        .world
        .get_organization(draft.custodian)
        .ok_or(InvestigationError::MissingOrganization(draft.custodian))?;
    if draft.custodian != investigation.owner() {
        return Err(InvestigationError::CustodianMismatch {
            investigation: draft.investigation,
            custodian: draft.custodian,
        });
    }
    if !is_entity_present(state, draft.subject) {
        return Err(InvestigationError::MissingEntity(draft.subject));
    }
    if let Some(origin) = draft.origin
        && !is_entity_present(state, origin)
    {
        return Err(InvestigationError::MissingEntity(origin));
    }
    if draft.discovered_at > state.now() {
        return Err(InvestigationError::DiscoveryInFuture);
    }
    ensure_evidence_prosecution_recusal_capacity(
        state,
        draft.investigation,
        draft.subject,
        draft.strength,
        draft.reliability,
        draft.admissibility,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
