//! Case-opening, investigator-staffing, and evidence transactions; sibling legal state keeps indexes synchronized.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId,
    InvestigationWorkId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::legal::{
    CaseWitnessRecord, EvidenceAssessment, EvidenceConnection, EvidenceDraft, EvidenceIdentity,
    EvidenceRecord, IncidentIntakeDraft, InvestigationDraft, InvestigationRecord,
    InvestigationStatus, InvestigatorRole,
};
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
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
    #[error("character {0} has no Investigation capability")]
    MissingInvestigationCapability(CharacterId),
    #[error("character {investigator} already has role {role:?} on investigation {investigation}")]
    AlreadyAssignedRole {
        investigation: InvestigationId,
        investigator: CharacterId,
        role: InvestigatorRole,
    },
    #[error("character {investigator} already leads an active investigation and cannot take another active case")]
    InvestigatorAtCaseCapacity { investigator: CharacterId },
    #[error("character {investigator} is not assigned to investigation {investigation}")]
    InvestigatorNotAssigned {
        investigation: InvestigationId,
        investigator: CharacterId,
    },
    #[error("character {investigator} owns scheduled investigation work {work}")]
    ScheduledInvestigationWork {
        investigator: CharacterId,
        work: InvestigationWorkId,
    },
    #[error("investigation {investigation} changed after validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("investigator {investigator} changed after validation; expected version {expected}, found {found}")]
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
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
        state.legal.insert_investigation(InvestigationRecord {
            id,
            owner: self.draft.owner,
            title: self.draft.title,
            status: InvestigationStatus::Active,
            lead_investigator: None,
            assigned_investigators: Default::default(),
            subjects: self.draft.subjects,
            evidence: Default::default(),
            opened_at: state.now(),
            origin_operation: None,
            notified_organizations: Default::default(),
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
            return Err(InvestigationError::InvalidOwnerKind(draft.owner))
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

impl ValidatedInvestigationTransition {
    pub fn commit(self, state: &mut AppState) -> Result<(), InvestigationError> {
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
        validate_investigation_transition_dependencies(state, self.investigation, self.transition)?;
        let status = match self.transition {
            InvestigationTransition::Suspend => InvestigationStatus::Suspended,
            InvestigationTransition::Resume => InvestigationStatus::Active,
            InvestigationTransition::Close => InvestigationStatus::Closed,
        };
        state
            .legal
            .set_investigation_status(self.investigation, status, state.now());
        // The lead investigator personally knows their own case was shelved, resumed, or
        // closed — the same refresh the cold-decay pass performs for its own transitions.
        crate::legal::case_knowledge::record_lead_case_activity_knowledge(
            state,
            self.investigation,
        );
        Ok(())
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
    if transition == InvestigationTransition::Suspend {
        if let Some(arrest) = state
            .legal
            .arrests_for_investigation(investigation_id)
            .find(|arrest| arrest.status() == crate::legal::ArrestStatus::Detained)
        {
            return Err(InvestigationError::ActiveArrestBlocksTransition {
                investigation: investigation_id,
                arrest: arrest.id(),
            });
        }
    }
    if transition == InvestigationTransition::Resume {
        let _ = state.world.get_organization(investigation.owner()).ok_or(
            InvestigationError::MissingOrganization(investigation.owner()),
        )?;
        for investigator_id in investigation.assigned_investigators() {
            let investigator = state
                .world
                .get_character(*investigator_id)
                .ok_or(InvestigationError::MissingCharacter(*investigator_id))?;
            if let Some(arrest) = state.legal.active_arrest_for_character(*investigator_id) {
                return Err(InvestigationError::DetainedInvestigator {
                    investigator: *investigator_id,
                    arrest: arrest.id(),
                });
            }
            if investigator.organization() != Some(investigation.owner()) {
                return Err(InvestigationError::InvestigatorOwnerMismatch {
                    investigator: *investigator_id,
                    owner: investigation.owner(),
                });
            }
            if investigator
                .capability(CapabilityKind::Investigation)
                .is_none()
            {
                return Err(InvestigationError::MissingInvestigationCapability(
                    *investigator_id,
                ));
            }
        }
    }
    Ok(())
}

/// Deterministically shelves operation-originated investigations whose owning authority has been
/// institutionally inactive for the authored cold window.
///
/// Cold cases are suspended through the canonical lifecycle transition, which revalidates every
/// dependency (no scheduled work, no active arrest) at the current minute, so work that appeared
/// between the deadline index scan and this call simply keeps the case active and decay retries on
/// the refreshed deadline. Only cases carrying an operation origination link are eligible:
/// institution-authored casework keeps its lifecycle until an explicit staff decision. A case whose
/// evidence identified a concrete character is a real, actionable lead and is never auto-shelved.
/// The owned authority keeps the sitting case history intact so a later operation exposure in the
/// same jurisdiction can resume the same shelf rather than starting from silence.
pub(crate) fn apply_cold_case_decay(
    state: &mut AppState,
    cold_case_window: SimDuration,
) -> Result<ColdCaseDecayOutcome, InvestigationError> {
    let threshold_minutes = state
        .now()
        .as_minutes()
        .saturating_sub(u64::from(cold_case_window.as_minutes()));
    let candidates = state
        .legal
        .active_case_ids_with_last_activity_at_or_before(SimTime::from_minutes(threshold_minutes));
    let mut suspended = Vec::new();
    let mut closed = Vec::new();
    for investigation in candidates {
        let record = state
            .legal
            .get_investigation(investigation)
            .expect("cold-case candidate must still exist");
        if record.origin_operation().is_none() {
            continue;
        }
        // An operation-originated case whose every identified subject is in custody is fully
        // worked: the institutional trail ends, so the case closes rather than sitting active
        // forever. Closing is allowed while arrests hold (cleared by arrest); cases with
        // subjects still at large keep their investigator attention.
        let identified_subjects: Vec<CharacterId> = record
            .subjects()
            .iter()
            .filter_map(|subject| match subject {
                EntityRef::Character(character) => Some(*character),
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
            .collect();
        if !identified_subjects.is_empty()
            && identified_subjects.iter().all(|character| {
                state
                    .legal
                    .active_arrest_for_character(*character)
                    .is_some()
            })
        {
            if let Ok(transition) = validate_transition_investigation(
                state,
                investigation,
                InvestigationTransition::Close,
            ) {
                transition
                    .commit(state)
                    .expect("validated cold-case closure must commit atomically");
                closed.push(investigation);
            }
            continue;
        }
        if !identified_subjects.is_empty() {
            continue;
        }
        let transition = validate_transition_investigation(
            state,
            investigation,
            InvestigationTransition::Suspend,
        );
        let Ok(transition) = transition else { continue };
        transition
            .commit(state)
            .expect("validated cold-case suspension must commit atomically");
        // The transition commit refreshes the lead's personal knowledge to "shelved".
        suspended.push(investigation);
    }
    Ok(ColdCaseDecayOutcome { suspended, closed })
}

/// Cold-window decay results, split so observers can distinguish shelved cases from cases
/// fully closed because every identified subject is already in custody.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColdCaseDecayOutcome {
    pub suspended: Vec<InvestigationId>,
    pub closed: Vec<InvestigationId>,
}

#[derive(Debug)]
pub struct ValidatedInvestigatorAssignment {
    investigation: InvestigationId,
    investigator: CharacterId,
    role: InvestigatorRole,
    expected_investigation_version: u32,
    expected_investigator_version: u32,
}

impl ValidatedInvestigatorAssignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), InvestigationError> {
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
        let investigator = state
            .world
            .get_character(self.investigator)
            .ok_or(InvestigationError::MissingCharacter(self.investigator))?;
        if investigator.version() != self.expected_investigator_version {
            return Err(InvestigationError::StaleInvestigator {
                investigator: self.investigator,
                expected: self.expected_investigator_version,
                found: investigator.version(),
            });
        }
        validate_investigator_assignment_dependencies(
            state,
            self.investigation,
            self.investigator,
            self.role,
        )?;
        state
            .legal
            .set_investigator_role(self.investigation, self.investigator, self.role);
        // Only a change of the lead seat is a material case-activity fact: promoting a new
        // lead refreshes their personal knowledge, while a plain investigator assignment
        // leaves the existing lead's record exactly as current as it was.
        if self.role == InvestigatorRole::Lead {
            crate::legal::case_knowledge::record_lead_case_activity_knowledge(
                state,
                self.investigation,
            );
        }
        Ok(())
    }
}

pub fn validate_assign_investigator(
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
    role: InvestigatorRole,
) -> Result<ValidatedInvestigatorAssignment, InvestigationError> {
    validate_investigator_assignment_dependencies(state, investigation, investigator, role)?;
    let investigation_record = state
        .legal
        .get_investigation(investigation)
        .expect("validated investigation must still exist");
    let investigator_record = state
        .world
        .get_character(investigator)
        .expect("validated investigator must still exist");
    Ok(ValidatedInvestigatorAssignment {
        investigation,
        investigator,
        role,
        expected_investigation_version: investigation_record.version(),
        expected_investigator_version: investigator_record.version(),
    })
}

pub(crate) fn apply_autonomous_investigator_staffing(
    state: &mut AppState,
) -> Result<Vec<(InvestigationId, CharacterId)>, InvestigationError> {
    let investigations: Vec<_> = state.legal.active_investigations_without_lead().collect();
    let mut staffed = Vec::new();

    for investigation_id in investigations {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .ok_or(InvestigationError::MissingInvestigation(investigation_id))?;
        let owner = investigation.owner();
        let assigned_candidate = investigation
            .assigned_investigators()
            .iter()
            .filter_map(|investigator| {
                let record = state.world.get_character(*investigator)?;
                let capability = record.capability(CapabilityKind::Investigation)?;
                // Investigators already attached to this case are exempt from the one-case
                // exclusion; attachment to any other active case disqualifies them.
                let case_is_this_one = state
                    .legal
                    .active_investigation_for_investigator(*investigator)
                    .is_none_or(|active| active.id() == investigation_id);
                (case_is_this_one
                    && record.organization() == Some(owner)
                    && state
                        .legal
                        .active_arrest_for_character(*investigator)
                        .is_none())
                .then_some((*investigator, capability.value()))
            })
            .min_by_key(|(investigator, capability)| (Reverse(*capability), *investigator))
            .map(|(investigator, _)| investigator);

        let investigator = assigned_candidate.or_else(|| {
            state
                .world
                .characters_in_organization(owner)
                .filter(|record| {
                    state
                        .legal
                        .active_arrest_for_character(record.id())
                        .is_none()
                        && state
                            .legal
                            .active_investigation_for_investigator(record.id())
                            .is_none()
                })
                .filter_map(|record| {
                    record
                        .capability(CapabilityKind::Investigation)
                        .map(|capability| (record.id(), capability.value()))
                })
                .min_by_key(|(investigator, capability)| (Reverse(*capability), *investigator))
                .map(|(investigator, _)| investigator)
        });
        let Some(investigator) = investigator else {
            continue;
        };

        // An autonomous staffing pass must not abort the tick: a case whose best candidate
        // fails canonical validation stays unstaffed for a later minute, like cold-case decay.
        let Ok(assignment) = validate_assign_investigator(
            state,
            investigation_id,
            investigator,
            InvestigatorRole::Lead,
        ) else {
            continue;
        };
        if assignment.commit(state).is_err() {
            continue;
        }
        // The assignment commit records the new lead's personal case-activity knowledge, so
        // contact channels can disclose it without any case-graph read.
        staffed.push((investigation_id, investigator));
    }
    Ok(staffed)
}

fn validate_investigator_assignment_dependencies(
    state: &AppState,
    investigation_id: InvestigationId,
    investigator_id: CharacterId,
    role: InvestigatorRole,
) -> Result<(), InvestigationError> {
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(InvestigationError::MissingInvestigation(investigation_id))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationError::InactiveInvestigation);
    }
    let investigator = state
        .world
        .get_character(investigator_id)
        .ok_or(InvestigationError::MissingCharacter(investigator_id))?;
    if let Some(arrest) = state.legal.active_arrest_for_character(investigator_id) {
        return Err(InvestigationError::DetainedInvestigator {
            investigator: investigator_id,
            arrest: arrest.id(),
        });
    }
    if investigator.organization() != Some(investigation.owner()) {
        return Err(InvestigationError::InvestigatorOwnerMismatch {
            investigator: investigator_id,
            owner: investigation.owner(),
        });
    }
    // One active case per investigator: an investigator already assigned to another active
    // investigation cannot take a second active case. The same case is exempt so a support
    // investigator can be promoted to lead of the case they already staff.
    if state
        .legal
        .active_investigation_for_investigator(investigator_id)
        .is_some_and(|active| active.id() != investigation_id)
    {
        return Err(InvestigationError::InvestigatorAtCaseCapacity {
            investigator: investigator_id,
        });
    }
    if investigator
        .capability(CapabilityKind::Investigation)
        .is_none()
    {
        return Err(InvestigationError::MissingInvestigationCapability(
            investigator_id,
        ));
    }
    if investigation.investigator_role(investigator_id) == Some(role) {
        return Err(InvestigationError::AlreadyAssignedRole {
            investigation: investigation_id,
            investigator: investigator_id,
            role,
        });
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedInvestigatorRemoval {
    investigation: InvestigationId,
    investigator: CharacterId,
    expected_investigation_version: u32,
}

impl ValidatedInvestigatorRemoval {
    pub fn commit(self, state: &mut AppState) -> Result<(), InvestigationError> {
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
        if investigation.investigator_role(self.investigator).is_none() {
            return Err(InvestigationError::InvestigatorNotAssigned {
                investigation: self.investigation,
                investigator: self.investigator,
            });
        }
        validate_no_scheduled_investigation_work(state, self.investigation, self.investigator)?;
        state
            .legal
            .remove_investigator(self.investigation, self.investigator);
        Ok(())
    }
}

pub fn validate_remove_investigator(
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
) -> Result<ValidatedInvestigatorRemoval, InvestigationError> {
    let investigation_record = state
        .legal
        .get_investigation(investigation)
        .ok_or(InvestigationError::MissingInvestigation(investigation))?;
    if investigation_record
        .investigator_role(investigator)
        .is_none()
    {
        return Err(InvestigationError::InvestigatorNotAssigned {
            investigation,
            investigator,
        });
    }
    validate_no_scheduled_investigation_work(state, investigation, investigator)?;
    Ok(ValidatedInvestigatorRemoval {
        investigation,
        investigator,
        expected_investigation_version: investigation_record.version(),
    })
}

fn validate_no_scheduled_investigation_work(
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
) -> Result<(), InvestigationError> {
    if let Some(work) = state
        .legal
        .work_for_investigator(investigator)
        .find(|work| {
            work.investigation() == investigation
                && work.status() == crate::legal::InvestigationWorkStatus::Scheduled
        })
    {
        return Err(InvestigationError::ScheduledInvestigationWork {
            investigator,
            work: work.id(),
        });
    }
    Ok(())
}

pub struct ValidatedEvidence {
    draft: EvidenceDraft,
}
impl ValidatedEvidence {
    pub fn commit(self, state: &mut AppState) -> Result<EvidenceId, InvestigationError> {
        validate_evidence_draft(state, &self.draft)?;
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
    Ok(ValidatedEvidence { draft })
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
    if let Some(origin) = draft.origin {
        if !is_entity_present(state, origin) {
            return Err(InvestigationError::MissingEntity(origin));
        }
    }
    if draft.discovered_at > state.now() {
        return Err(InvestigationError::DiscoveryInFuture);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncidentIntakeOutcome {
    pub investigation: InvestigationId,
    pub evidence: Vec<EvidenceId>,
    pub case_witness: Option<crate::core::id::CaseWitnessId>,
}

pub struct ValidatedIncidentIntake {
    draft: IncidentIntakeDraft,
}

impl ValidatedIncidentIntake {
    pub(crate) fn evidence_count(&self) -> u32 {
        u32::try_from(self.draft.evidence.len()).expect("incident evidence count must fit u32")
    }

    pub(crate) fn has_witness(&self) -> bool {
        self.draft.witness.is_some()
    }

    pub fn commit(self, state: &mut AppState) -> Result<IncidentIntakeOutcome, InvestigationError> {
        validate_incident_intake_dependencies(state, &self.draft)?;
        state.ids.reserve_many(&[
            (IdKind::Investigation, 1),
            (IdKind::Evidence, self.evidence_count()),
            (IdKind::CaseWitness, u32::from(self.draft.witness.is_some())),
        ])?;
        let investigation = state.ids.next_investigation()?;
        state.legal.insert_investigation(InvestigationRecord {
            id: investigation,
            owner: self.draft.owner,
            title: self.draft.title,
            status: InvestigationStatus::Active,
            lead_investigator: None,
            assigned_investigators: Default::default(),
            subjects: self.draft.subjects.clone(),
            evidence: Default::default(),
            opened_at: state.now(),
            origin_operation: self.draft.origin_operation,
            notified_organizations: self.draft.notified_organizations,
            last_activity_at: state.now(),
            version: 1,
        });
        let case_witness = if let Some(witness) = self.draft.witness {
            let id = state.ids.next_case_witness()?;
            state.legal.insert_case_witness(
                CaseWitnessRecord {
                    id,
                    investigation,
                    witness: witness.character,
                    cooperation: witness.cooperation,
                    registered_at: state.now(),
                    statements: Default::default(),
                    version: 1,
                },
                state.now(),
            );
            Some(id)
        } else {
            None
        };
        let mut evidence_ids = Vec::with_capacity(self.draft.evidence.len());
        for evidence in self.draft.evidence {
            let id = state.ids.next_evidence()?;
            state.legal.insert_evidence(
                EvidenceRecord {
                    identity: EvidenceIdentity {
                        id,
                        investigation,
                        custodian: self.draft.owner,
                    },
                    connection: EvidenceConnection {
                        subject: evidence.subject,
                        origin: evidence.origin,
                        source: None,
                        derived_from: Default::default(),
                    },
                    assessment: EvidenceAssessment {
                        kind: evidence.kind,
                        strength: evidence.strength,
                        reliability: evidence.reliability,
                        admissibility: evidence.admissibility,
                    },
                    discovered_at: evidence.discovered_at,
                },
                state.now(),
            );
            evidence_ids.push(id);
        }
        Ok(IncidentIntakeOutcome {
            investigation,
            evidence: evidence_ids,
            case_witness,
        })
    }
}

pub fn validate_incident_intake(
    state: &AppState,
    draft: IncidentIntakeDraft,
) -> Result<ValidatedIncidentIntake, InvestigationError> {
    validate_incident_intake_dependencies(state, &draft)?;
    Ok(ValidatedIncidentIntake { draft })
}

fn validate_incident_intake_dependencies(
    state: &AppState,
    draft: &IncidentIntakeDraft,
) -> Result<(), InvestigationError> {
    validate_investigation_draft(
        state,
        &InvestigationDraft {
            owner: draft.owner,
            title: draft.title.clone(),
            subjects: draft.subjects.clone(),
        },
    )?;
    if draft.evidence.is_empty() {
        return Err(InvestigationError::NoIncidentEvidence);
    }
    if let Some(operation) = draft.origin_operation {
        if !is_entity_present(state, EntityRef::Operation(operation)) {
            return Err(InvestigationError::MissingEntity(EntityRef::Operation(
                operation,
            )));
        }
    }
    for organization in &draft.notified_organizations {
        if !is_entity_present(state, EntityRef::Organization(*organization)) {
            return Err(InvestigationError::MissingEntity(EntityRef::Organization(
                *organization,
            )));
        }
    }
    if let Some(witness) = &draft.witness {
        state
            .world
            .get_character(witness.character)
            .ok_or(InvestigationError::MissingEntity(EntityRef::Character(
                witness.character,
            )))?;
        // A case's subject cannot also be its named witness.
        if draft
            .subjects
            .contains(&EntityRef::Character(witness.character))
        {
            return Err(InvestigationError::WitnessIsCaseSubject {
                character: witness.character,
            });
        }
    }
    for evidence in &draft.evidence {
        // Incident intake inserts evidence with `source: None`; work-derived kinds and
        // informant statements may only exist through their canonical production paths.
        validate_evidence_kind_allowed(evidence.kind)?;
        if !is_entity_present(state, evidence.subject) {
            return Err(InvestigationError::MissingEntity(evidence.subject));
        }
        if let Some(origin) = evidence.origin {
            if !is_entity_present(state, origin) {
                return Err(InvestigationError::MissingEntity(origin));
            }
        }
        if evidence.discovered_at > state.now() {
            return Err(InvestigationError::DiscoveryInFuture);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
