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
use crate::world::{CapabilityKind, Lifecycle, OrganizationKind};
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
    #[error("organization {0} is not active and cannot own new legal work")]
    InactiveOwner(OrganizationId),
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
    #[error("character {0} is not active and cannot be assigned investigative work")]
    InactiveInvestigator(CharacterId),
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
    #[error("pattern-link evidence must be produced by canonical investigation work")]
    PatternLinkRequiresInvestigationWork,
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
    if owner.lifecycle() != Lifecycle::Active {
        return Err(InvestigationError::InactiveOwner(draft.owner));
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
        let owner = state.world.get_organization(investigation.owner()).ok_or(
            InvestigationError::MissingOrganization(investigation.owner()),
        )?;
        if owner.lifecycle() != Lifecycle::Active {
            return Err(InvestigationError::InactiveOwner(investigation.owner()));
        }
        for investigator_id in investigation.assigned_investigators() {
            let investigator = state
                .world
                .get_character(*investigator_id)
                .ok_or(InvestigationError::MissingCharacter(*investigator_id))?;
            if investigator.lifecycle() != Lifecycle::Active {
                return Err(InvestigationError::InactiveInvestigator(*investigator_id));
            }
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
                (record.lifecycle() == Lifecycle::Active
                    && case_is_this_one
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
                    record.lifecycle() == Lifecycle::Active
                        && state
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
    if investigator.lifecycle() != Lifecycle::Active {
        return Err(InvestigationError::InactiveInvestigator(investigator_id));
    }
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
        crate::legal::EvidenceKind::PatternLink => {
            Err(InvestigationError::PatternLinkRequiresInvestigationWork)
        }
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
    let custodian = state
        .world
        .get_organization(draft.custodian)
        .ok_or(InvestigationError::MissingOrganization(draft.custodian))?;
    if draft.custodian != investigation.owner() {
        return Err(InvestigationError::CustodianMismatch {
            investigation: draft.investigation,
            custodian: draft.custodian,
        });
    }
    if custodian.lifecycle() != Lifecycle::Active {
        return Err(InvestigationError::InactiveOwner(draft.custodian));
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
            state.legal.insert_case_witness(CaseWitnessRecord {
                id,
                investigation,
                witness: witness.character,
                cooperation: witness.cooperation,
                registered_at: state.now(),
                statements: Default::default(),
                version: 1,
            });
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
        let record = state.world.get_character(witness.character).ok_or(
            InvestigationError::MissingEntity(EntityRef::Character(witness.character)),
        )?;
        if record.lifecycle() != Lifecycle::Active {
            return Err(InvestigationError::MissingEntity(EntityRef::Character(
                witness.character,
            )));
        }
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{
        validate_invariants, validate_state, validate_state_against_registry,
    };
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::SimDuration;
    use crate::legal::investigation_work_execution::{
        decide_investigation_work_resolution, validate_investigation_work_resolution_plan,
        validate_schedule_investigation_work, InvestigationWorkRandomness,
    };
    use crate::legal::{
        Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength, InvestigationWorkDraft,
        InvestigationWorkFocus, InvestigationWorkKind, InvestigatorRole,
    };
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{
        AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("test rating must be valid")
    }

    fn insert_test_investigator(
        registry: &crate::Registry,
        state: &mut AppState,
        organization: OrganizationId,
        name: &str,
        skill: u8,
    ) -> CharacterId {
        insert_character(
            registry,
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(skill))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("investigator fixture should validate")
    }

    #[test]
    fn incident_intake_cannot_forge_informant_statement() {
        let registry = build_registry();
        let mut state = AppState::new(0x14F0_5EED);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Intake Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Intake Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");

        let error = match validate_incident_intake(
            &state,
            IncidentIntakeDraft {
                owner: police,
                title: "Forged statement inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
                evidence: vec![crate::legal::IncidentEvidenceDraft {
                    subject: EntityRef::Organization(criminal),
                    origin: None,
                    kind: EvidenceKind::InformantStatement,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::Credible,
                    admissibility: Admissibility::Unknown,
                    discovered_at: state.now(),
                }],
                origin_operation: None,
                notified_organizations: BTreeSet::new(),
                witness: None,
            },
        ) {
            Ok(_) => panic!("incident intake must reject informant statements"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            InvestigationError::InformantStatementRequiresDisclosure
        );
        assert_eq!(
            state
                .legal()
                .evidence_of_kind(EvidenceKind::InformantStatement)
                .count(),
            0
        );
        assert!(state.legal().investigations().next().is_none());
        validate_invariants(&state);
    }

    #[test]
    fn autonomous_staffing_assigns_best_available_detective_and_respects_active_case_capacity() {
        let registry = build_registry();
        let mut state = AppState::new(0x57AF_F193);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Staffing Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Staffing Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let junior = insert_test_investigator(&registry, &mut state, police, "Junior", 70);
        let senior = insert_test_investigator(&registry, &mut state, police, "Senior", 92);
        let first = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "First autonomous staffing inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("first case should validate")
        .commit(&mut state)
        .expect("first case should commit");

        state = restore_save(
            &registry,
            build_save(&registry, &state).expect("unstaffed case state should save"),
        )
        .expect("unstaffed case index should survive save restoration");

        let staffed = apply_autonomous_investigator_staffing(&mut state)
            .expect("available detectives should staff the first case");
        assert_eq!(staffed, vec![(first, senior)]);
        assert_eq!(
            state
                .legal()
                .get_investigation(first)
                .expect("first case should persist")
                .lead_investigator(),
            Some(senior)
        );

        let second = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Second autonomous staffing inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("second case should validate")
        .commit(&mut state)
        .expect("second case should commit");
        let staffed = apply_autonomous_investigator_staffing(&mut state)
            .expect("remaining detective should staff the second case");
        assert_eq!(staffed, vec![(second, junior)]);
        assert_eq!(
            state
                .legal()
                .get_investigation(second)
                .expect("second case should persist")
                .lead_investigator(),
            Some(junior)
        );
        assert!(apply_autonomous_investigator_staffing(&mut state)
            .expect("already staffed cases should be a no-op")
            .is_empty());
        validate_state(&state).expect("autonomous staffing state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn investigation_suspend_resume_is_versioned_persistent_and_disables_active_mutation() {
        let registry = build_registry();
        let mut state = AppState::new(0x5A5E_1931);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Lifecycle Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Lifecycle Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let detective = insert_test_investigator(&registry, &mut state, police, "Harlan", 82);
        let second_detective = insert_test_investigator(&registry, &mut state, police, "Meyer", 74);
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Suspended conspiracy inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        validate_assign_investigator(&state, investigation, detective, InvestigatorRole::Lead)
            .expect("lead assignment should validate")
            .commit(&mut state)
            .expect("lead assignment should commit");

        let stale_suspend = validate_transition_investigation(
            &state,
            investigation,
            InvestigationTransition::Suspend,
        )
        .expect("suspension should initially validate");
        validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Organization(criminal),
                origin: None,
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("case mutation should validate before suspension")
        .commit(&mut state)
        .expect("case mutation should commit");
        assert!(matches!(
            stale_suspend.commit(&mut state),
            Err(InvestigationError::StaleInvestigation { .. })
        ));

        validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
            .expect("fresh suspension should validate")
            .commit(&mut state)
            .expect("fresh suspension should commit");
        assert_eq!(
            state
                .legal()
                .get_investigation(investigation)
                .expect("investigation should exist")
                .status(),
            InvestigationStatus::Suspended
        );

        let evidence_error = match validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Organization(criminal),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        ) {
            Ok(_) => panic!("suspended investigation must reject new evidence"),
            Err(error) => error,
        };
        assert_eq!(evidence_error, InvestigationError::InactiveInvestigation);
        let staffing_error = validate_assign_investigator(
            &state,
            investigation,
            second_detective,
            InvestigatorRole::Investigator,
        )
        .expect_err("suspended investigation must reject new staffing");
        assert_eq!(staffing_error, InvestigationError::InactiveInvestigation);

        let mut restored = restore_save(
            &registry,
            build_save(&registry, &state).expect("suspended investigation should save"),
        )
        .expect("suspended investigation should restore");
        assert_eq!(
            restored
                .legal()
                .get_investigation(investigation)
                .expect("restored investigation should exist")
                .status(),
            InvestigationStatus::Suspended
        );
        validate_transition_investigation(
            &restored,
            investigation,
            InvestigationTransition::Resume,
        )
        .expect("valid retained staffing should permit resume")
        .commit(&mut restored)
        .expect("resume should commit");
        assert_eq!(
            restored
                .legal()
                .get_investigation(investigation)
                .expect("resumed investigation should exist")
                .status(),
            InvestigationStatus::Active
        );
        validate_state(&restored).expect("resumed investigation should be structurally valid");
        validate_state_against_registry(&registry, &restored)
            .expect("resumed investigation should match authored state");
        validate_invariants(&restored);
    }

    #[test]
    fn scheduled_detective_work_blocks_case_transition_until_resolution_then_close_is_terminal() {
        let registry = build_registry();
        let mut state = AppState::new(0xC105_E193);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Closure Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let other_police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Harbor Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("second police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Closure Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let detective = insert_test_investigator(&registry, &mut state, police, "Doyle", 90);
        let first = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "First Subject".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("first subject should validate");
        let middle = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Middle Subject".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("middle subject should validate");
        let target = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Target Subject".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("target subject should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Pattern closure inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(first)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        validate_assign_investigator(&state, investigation, detective, InvestigatorRole::Lead)
            .expect("lead assignment should validate")
            .commit(&mut state)
            .expect("lead assignment should commit");

        for (subject, origin) in [
            (EntityRef::Character(middle), EntityRef::Character(first)),
            (EntityRef::Character(target), EntityRef::Character(middle)),
        ] {
            validate_add_evidence(
                &state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject,
                    origin: Some(origin),
                    kind: EvidenceKind::KnownAssociation,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::Credible,
                    admissibility: Admissibility::Admissible,
                    discovered_at: state.now(),
                },
            )
            .expect("path evidence should validate")
            .commit(&mut state)
            .expect("path evidence should commit");
        }
        let work = validate_schedule_investigation_work(
            &registry,
            &state,
            InvestigationWorkDraft {
                investigation,
                investigator: detective,
                kind: InvestigationWorkKind::PatternAnalysis,
                focus: InvestigationWorkFocus::new(
                    EntityRef::Character(first),
                    EntityRef::Character(target),
                ),
            },
        )
        .expect("pattern analysis should validate")
        .commit(&mut state)
        .expect("pattern analysis should schedule");

        for transition in [
            InvestigationTransition::Suspend,
            InvestigationTransition::Close,
        ] {
            assert_eq!(
                validate_transition_investigation(&state, investigation, transition)
                    .expect_err("scheduled work must block case lifecycle transition"),
                InvestigationError::ScheduledWorkBlocksTransition {
                    investigation,
                    work,
                }
            );
        }

        state.advance_clock(SimDuration::from_minutes(360));
        let plan = decide_investigation_work_resolution(
            &registry,
            &state,
            work,
            InvestigationWorkRandomness::new(0),
        )
        .expect("due work should resolve");
        validate_investigation_work_resolution_plan(&registry, &state, plan)
            .expect("fresh work plan should validate")
            .commit(&mut state)
            .expect("work resolution should commit");
        validate_transition_investigation(&state, investigation, InvestigationTransition::Close)
            .expect("completed work should permit case closure")
            .commit(&mut state)
            .expect("case closure should commit");
        assert_eq!(
            state
                .legal()
                .get_investigation(investigation)
                .expect("closed case should exist")
                .status(),
            InvestigationStatus::Closed
        );

        validate_reassign_character(&state, detective, Some(other_police), None)
            .expect("closed historical case must not lock investigator membership")
            .commit(&mut state)
            .expect("detective transfer after closure should commit");
        for transition in [
            InvestigationTransition::Suspend,
            InvestigationTransition::Resume,
            InvestigationTransition::Close,
        ] {
            assert_eq!(
                validate_transition_investigation(&state, investigation, transition)
                    .expect_err("closed case must be terminal"),
                InvestigationError::InvalidInvestigationTransition {
                    status: InvestigationStatus::Closed,
                    transition,
                }
            );
        }
        validate_state(&state).expect("closed case should remain structurally valid");
        validate_state_against_registry(&registry, &state)
            .expect("closed case history should remain registry-valid");
        validate_invariants(&state);
    }

    #[test]
    fn suspended_case_resume_revalidates_retained_staffing_after_detective_transfer() {
        let registry = build_registry();
        let mut state = AppState::new(0x5A57_AFF1);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Original Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let other_police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Transferred Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("second police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Resume Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let detective = insert_test_investigator(&registry, &mut state, police, "Reed", 80);
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Retained staffing inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        validate_assign_investigator(&state, investigation, detective, InvestigatorRole::Lead)
            .expect("lead assignment should validate")
            .commit(&mut state)
            .expect("lead assignment should commit");
        validate_transition_investigation(&state, investigation, InvestigationTransition::Suspend)
            .expect("suspension should validate")
            .commit(&mut state)
            .expect("suspension should commit");

        validate_reassign_character(&state, detective, Some(other_police), None)
            .expect("suspended case should not lock detective organization membership")
            .commit(&mut state)
            .expect("detective transfer should commit");
        assert_eq!(
            validate_transition_investigation(
                &state,
                investigation,
                InvestigationTransition::Resume,
            )
            .expect_err("resume must reject retained investigator who transferred away"),
            InvestigationError::InvestigatorOwnerMismatch {
                investigator: detective,
                owner: police,
            }
        );

        validate_remove_investigator(&state, investigation, detective)
            .expect("invalid retained staffing should be removable while suspended")
            .commit(&mut state)
            .expect("staffing cleanup should commit");
        validate_transition_investigation(&state, investigation, InvestigationTransition::Resume)
            .expect("case with cleaned staffing should resume")
            .commit(&mut state)
            .expect("resume after staffing cleanup should commit");
        assert_eq!(
            state
                .legal()
                .get_investigation(investigation)
                .expect("resumed case should exist")
                .status(),
            InvestigationStatus::Active
        );
        validate_state(&state).expect("resumed case should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn investigator_staffing_is_versioned_indexed_and_blocks_foreign_reassignment() {
        let registry = build_registry();
        let mut state = AppState::new(0xD37E_C71E);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Central Detectives".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let other_police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Harbor Detectives".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("second police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "South Ward Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let first = insert_test_investigator(&registry, &mut state, police, "Harlan", 82);
        let second = insert_test_investigator(&registry, &mut state, police, "Meyer", 74);
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "South Ward conspiracy".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");

        validate_assign_investigator(&state, investigation, first, InvestigatorRole::Lead)
            .expect("lead assignment should validate")
            .commit(&mut state)
            .expect("lead assignment should commit");
        validate_assign_investigator(
            &state,
            investigation,
            second,
            InvestigatorRole::Investigator,
        )
        .expect("supporting assignment should validate")
        .commit(&mut state)
        .expect("supporting assignment should commit");
        validate_assign_investigator(&state, investigation, second, InvestigatorRole::Lead)
            .expect("lead promotion should validate")
            .commit(&mut state)
            .expect("lead promotion should commit");

        let record = state
            .legal()
            .get_investigation(investigation)
            .expect("investigation should exist");
        assert_eq!(record.lead_investigator(), Some(second));
        assert_eq!(
            record.investigator_role(first),
            Some(InvestigatorRole::Investigator)
        );
        assert_eq!(
            record.investigator_role(second),
            Some(InvestigatorRole::Lead)
        );
        assert_eq!(
            state
                .legal()
                .investigations_for_investigator(first)
                .map(|case| case.id())
                .collect::<Vec<_>>(),
            vec![investigation]
        );

        let restored = restore_save(
            &registry,
            build_save(&registry, &state).expect("staffed case state should save"),
        )
        .expect("staffed case state should restore");
        let restored_case = restored
            .legal()
            .get_investigation(investigation)
            .expect("restored investigation should exist");
        assert_eq!(restored_case.lead_investigator(), Some(second));
        assert_eq!(
            restored
                .legal()
                .investigations_for_investigator(first)
                .map(|case| case.id())
                .collect::<Vec<_>>(),
            vec![investigation]
        );

        let error = validate_reassign_character(&state, first, Some(other_police), None)
            .expect_err("active case assignment must block organization reassignment");
        assert_eq!(
            error,
            WorldError::ActiveInvestigationAssignment {
                character: first,
                investigation,
            }
        );

        validate_remove_investigator(&state, investigation, first)
            .expect("investigator release should validate")
            .commit(&mut state)
            .expect("investigator release should commit");
        validate_reassign_character(&state, first, Some(other_police), None)
            .expect("released investigator should be free to transfer")
            .commit(&mut state)
            .expect("released investigator transfer should commit");
        assert_eq!(
            state.legal().investigations_for_investigator(first).count(),
            0
        );
        validate_state(&state).expect("staffed investigation should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn investigator_assignment_token_rejects_case_changes_after_validation() {
        let registry = build_registry();
        let mut state = AppState::new(0x57A1_ECA5);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Versioned Case Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Versioned Case Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let detective = insert_test_investigator(&registry, &mut state, police, "Doyle", 79);
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Changing evidence file".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        let assignment =
            validate_assign_investigator(&state, investigation, detective, InvestigatorRole::Lead)
                .expect("assignment should validate against initial case version");

        validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Organization(criminal),
                origin: None,
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("new evidence should validate")
        .commit(&mut state)
        .expect("new evidence should commit");

        let error = assignment
            .commit(&mut state)
            .expect_err("case mutation must invalidate older staffing token");
        assert_eq!(
            error,
            InvestigationError::StaleInvestigation {
                investigation,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .legal()
                .get_investigation(investigation)
                .expect("investigation should exist")
                .lead_investigator(),
            None
        );
        validate_state(&state).expect("stale token rejection must leave state valid");
        validate_invariants(&state);
    }

    #[test]
    fn case_graph_indexes_track_shared_subjects_and_evidence_kinds() {
        let registry = build_registry();
        let mut state = AppState::new(0xCA53_1933);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Case Graph Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let other_police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Foreign Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("second police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Case Graph Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let character = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Case Graph Associate".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("character fixture should validate");

        let first = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "First linked incident".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("first investigation should validate")
        .commit(&mut state)
        .expect("validated first investigation should commit");
        let evidence = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation: first,
                custodian: police,
                subject: EntityRef::Character(character),
                origin: Some(EntityRef::Organization(criminal)),
                kind: EvidenceKind::KnownAssociation,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("case-link evidence should validate")
        .commit(&mut state)
        .expect("validated case-link evidence should commit");
        let second = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Second linked incident".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(character)]),
            },
        )
        .expect("second investigation should validate")
        .commit(&mut state)
        .expect("validated second investigation should commit");

        assert_eq!(
            state
                .legal()
                .investigations_for_subject(EntityRef::Character(character))
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(
            state
                .legal()
                .evidence_of_kind(EvidenceKind::KnownAssociation)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![evidence]
        );
        assert_eq!(
            state
                .legal()
                .evidence_from_origin(EntityRef::Organization(criminal))
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![evidence]
        );

        let error = match validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation: first,
                custodian: other_police,
                subject: EntityRef::Character(character),
                origin: None,
                kind: EvidenceKind::WitnessTestimony,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        ) {
            Ok(_) => {
                panic!("foreign precinct must not append evidence to another authority's case")
            }
            Err(error) => error,
        };
        assert_eq!(
            error,
            InvestigationError::CustodianMismatch {
                investigation: first,
                custodian: other_police,
            }
        );
        validate_state(&state).expect("case graph indexes should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn operation_originated_cases_cool_and_reopen_through_the_canonical_transition() {
        let registry = build_registry();
        let mut state = AppState::new(0xC01D_1933);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Cold Case Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Cold Case Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Cold Case Leader".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Surveillance, rating(80)),
                    (CapabilityKind::Management, rating(80)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("leader fixture should validate");
        let origin = crate::operations::operation_system::validate_authorize_operation(
            &registry,
            &state,
            crate::operations::OperationDraft {
                title: "Origin surveillance".to_owned(),
                kind: crate::operations::OperationKind::Surveillance,
                responsible_organization: criminal,
                leader,
                objective: crate::operations::OperationObjective::GatherInformation {
                    target: EntityRef::Organization(criminal),
                },
                approach: crate::operations::OperationApproach::Covert,
                roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("origin operation should validate")
        .commit(&mut state)
        .expect("origin operation should commit");
        let case = validate_incident_intake(
            &state,
            IncidentIntakeDraft {
                owner: police,
                title: "Sober incident inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Operation(origin)]),
                evidence: vec![crate::legal::IncidentEvidenceDraft {
                    subject: EntityRef::Operation(origin),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: state.now(),
                }],
                origin_operation: Some(origin),
                notified_organizations: BTreeSet::from([criminal]),
                witness: None,
            },
        )
        .expect("incident intake should validate")
        .commit(&mut state)
        .expect("incident intake should commit")
        .investigation;
        let institution_authored = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Institution-authored case stays put".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("institution-authored case should validate")
        .commit(&mut state)
        .expect("institution-authored case should commit");

        // A short cold window shelves only the operation-originated case without an identified
        // suspect; an operation-originated case that named a concrete character is a real lead and
        // stays active.
        let identified = validate_incident_intake(
            &state,
            IncidentIntakeDraft {
                owner: police,
                title: "Identified incident inquiry".to_owned(),
                subjects: BTreeSet::from([
                    EntityRef::Operation(origin),
                    EntityRef::Character(leader),
                ]),
                evidence: vec![crate::legal::IncidentEvidenceDraft {
                    subject: EntityRef::Character(leader),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::KnownAssociation,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: state.now(),
                }],
                origin_operation: Some(origin),
                notified_organizations: BTreeSet::from([criminal]),
                witness: None,
            },
        )
        .expect("identified incident intake should validate")
        .commit(&mut state)
        .expect("identified incident intake should commit")
        .investigation;
        state.advance_clock(SimDuration::from_minutes(121));
        let suspended = apply_cold_case_decay(&mut state, SimDuration::from_minutes(120))
            .expect("cold-case decay should resolve");
        assert_eq!(
            suspended,
            ColdCaseDecayOutcome {
                suspended: vec![case],
                closed: Vec::new()
            }
        );
        let record = state
            .legal()
            .get_investigation(case)
            .expect("cold case should persist");
        assert_eq!(record.status(), InvestigationStatus::Suspended);
        assert_eq!(
            state
                .legal()
                .get_investigation(institution_authored)
                .expect("institution-authored case should persist")
                .status(),
            InvestigationStatus::Active
        );
        assert_eq!(
            state
                .legal()
                .get_investigation(identified)
                .expect("identified case should persist")
                .status(),
            InvestigationStatus::Active
        );
        validate_state(&state).expect("cold decay state should validate");
        validate_invariants(&state);

        // The cold-decay index and case provenance survive save/restore, so a campaign loaded
        // after the shelf decision keeps the same institutional state.
        state = restore_save(
            &registry,
            build_save(&registry, &state).expect("cold case state should save"),
        )
        .expect("cold case state should restore");
        validate_state(&state).expect("restored cold decay state should validate");
        validate_invariants(&state);

        // The owning authority can resume the shelved case through the canonical transition; the
        // resume refreshes institutional activity so it does not immediately re-cool.
        validate_transition_investigation(&state, case, InvestigationTransition::Resume)
            .expect("resume should validate")
            .commit(&mut state)
            .expect("resume should commit");
        validate_state(&state).expect("resumed cold case state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn cold_case_decay_closes_a_fully_worked_case_whose_every_subject_is_detained() {
        let registry = build_registry();
        let mut state = AppState::new(0xC1EA_1933);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Cleared Case Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Cleared Case Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Cleared Case Leader".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Surveillance, rating(80)),
                    (CapabilityKind::Management, rating(80)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("leader fixture should validate");
        let lieutenant = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Cleared Case Lieutenant".to_owned(),
                organization: Some(criminal),
                supervisor: Some(leader),
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(60))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("lieutenant fixture should validate");
        let origin = crate::operations::operation_system::validate_authorize_operation(
            &registry,
            &state,
            crate::operations::OperationDraft {
                title: "Origin surveillance".to_owned(),
                kind: crate::operations::OperationKind::Surveillance,
                responsible_organization: criminal,
                leader,
                objective: crate::operations::OperationObjective::GatherInformation {
                    target: EntityRef::Organization(criminal),
                },
                approach: crate::operations::OperationApproach::Covert,
                roles: BTreeMap::from([(crate::operations::RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("origin operation should validate")
        .commit(&mut state)
        .expect("origin operation should commit");
        let identified = validate_incident_intake(
            &state,
            IncidentIntakeDraft {
                owner: police,
                title: "Cleared identified inquiry".to_owned(),
                subjects: BTreeSet::from([
                    EntityRef::Operation(origin),
                    EntityRef::Character(lieutenant),
                ]),
                evidence: vec![crate::legal::IncidentEvidenceDraft {
                    subject: EntityRef::Character(lieutenant),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::KnownAssociation,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: state.now(),
                }],
                origin_operation: Some(origin),
                notified_organizations: BTreeSet::from([criminal]),
                witness: None,
            },
        )
        .expect("identified incident intake should validate")
        .commit(&mut state)
        .expect("identified incident intake should commit")
        .investigation;
        let evidence = *state
            .legal()
            .get_investigation(identified)
            .and_then(|record| record.evidence().iter().next())
            .expect("intake recorded its evidence");

        // The subject's arrest sits under this very case; the case is cleared by arrest and
        // must close through decay instead of lingering active with a held investigator slot.
        crate::legal::arrest_system::validate_arrest(
            &state,
            crate::legal::ArrestDraft {
                character: lieutenant,
                investigation: identified,
                evidence: BTreeSet::from([evidence]),
            },
        )
        .expect("evidence-backed arrest should validate")
        .commit(&mut state)
        .expect("evidence-backed arrest should commit");

        state.advance_clock(SimDuration::from_minutes(121));
        let decayed = apply_cold_case_decay(&mut state, SimDuration::from_minutes(120))
            .expect("cold-case decay should resolve");
        assert_eq!(
            decayed,
            ColdCaseDecayOutcome {
                suspended: Vec::new(),
                closed: vec![identified]
            }
        );
        assert_eq!(
            state
                .legal()
                .get_investigation(identified)
                .map(|record| record.status()),
            Some(InvestigationStatus::Closed)
        );
        validate_state(&state).expect("cleared-case state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn weak_evidence_does_not_promote_a_character_to_identified_suspect() {
        let registry = build_registry();
        let mut state = AppState::new(0x0DD_555);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Promotion Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let suspect = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Ray Cusack".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("suspect fixture should validate");
        let outfit = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Promotion Outfit".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("outfit fixture should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Weak tip inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(outfit)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");

        let weak_tip = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(suspect),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("weak evidence should validate")
        .commit(&mut state)
        .expect("weak evidence should commit");
        assert!(
            !state
                .legal()
                .get_investigation(investigation)
                .expect("investigation should exist")
                .subjects()
                .contains(&EntityRef::Character(suspect)),
            "weak evidence must not promote a character to case subject"
        );

        let corroboration = validate_add_evidence(
            &state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(suspect),
                origin: None,
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Mixed,
                admissibility: Admissibility::Unknown,
                discovered_at: state.now(),
            },
        )
        .expect("corroborating evidence should validate")
        .commit(&mut state)
        .expect("corroborating evidence should commit");
        assert!(state
            .legal()
            .get_investigation(investigation)
            .expect("investigation should exist")
            .subjects()
            .contains(&EntityRef::Character(suspect)));
        assert_ne!(weak_tip, corroboration);

        validate_state(&state).expect("subject promotion state should validate");
        validate_invariants(&state);
    }
}
