//! Atomic incident-to-case routing, including active-file and suspended-shelf continuation.

use super::{
    InvestigationError, InvestigationTransition, ensure_evidence_prosecution_recusal_capacity,
    evidence_assessment_is_actionable_case_lead, validate_evidence_kind_allowed,
    validate_investigation_draft, validate_transition_investigation,
};
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{EvidenceId, IdKind, InvestigationId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{SimTime, ensure_time_current};
use crate::core::version::{VersionCapacityError, ensure_version_can_advance_by};
use crate::legal::{
    CaseWitnessRecord, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceRecord,
    IncidentIntakeDraft, InvestigationDraft, InvestigationRecord, InvestigationStatus,
};
use std::cmp::Reverse;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncidentIntakeOutcome {
    pub investigation: InvestigationId,
    /// Whether this intake folded into an existing suspended shelf instead of opening a
    /// fresh case; observers distinguish continued casework from brand-new attention.
    pub resumed_shelf: bool,
    pub evidence: Vec<EvidenceId>,
    pub case_witness: Option<crate::core::id::CaseWitnessId>,
}

pub struct ValidatedIncidentIntake {
    draft: IncidentIntakeDraft,
    validated_at: SimTime,
    /// Existing originated case and its pinned version, whether active or suspended.
    /// Recompute routing at commit, including new matching cases.
    continuing: Option<(InvestigationId, u32)>,
}

impl ValidatedIncidentIntake {
    pub(crate) fn witness_character(&self) -> Option<crate::core::id::CharacterId> {
        self.draft.witness.as_ref().map(|witness| witness.character)
    }

    pub(crate) fn evidence_count(&self) -> Result<u32, InvestigationError> {
        u32::try_from(self.draft.evidence.len())
            .map_err(|_| InvestigationError::IncidentEvidenceCountOverflow)
    }

    pub(crate) fn id_budget(&self) -> Result<Vec<(IdKind, u32)>, InvestigationError> {
        let mut budget = Vec::with_capacity(3);
        if self.continuing.is_none() {
            budget.push((IdKind::Investigation, 1));
        }
        budget.push((IdKind::Evidence, self.evidence_count()?));
        budget.push((IdKind::CaseWitness, u32::from(self.draft.witness.is_some())));
        Ok(budget)
    }

    /// Re-checks the read-only case-routing decision without allocating or mutating. Composite
    /// callers use this before committing unrelated state, so a shelf that appeared, resumed,
    /// or changed after validation cannot make incident intake fail after another domain moved.
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), InvestigationError> {
        ensure_time_current(state.now(), self.validated_at).map_err(|(expected, found)| {
            InvestigationError::StaleIncidentTime { expected, found }
        })?;
        validate_incident_intake_dependencies(state, &self.draft)?;
        let found = find_incident_target(state, &self.draft);
        if found != self.continuing {
            return Err(InvestigationError::StaleIncidentShelf {
                expected: self.continuing,
                found,
            });
        }
        if let Some((shelf, _)) = self.continuing {
            let record = state
                .legal
                .get_investigation(shelf)
                .expect("continued incident file must exist");
            let resuming = record.status() == InvestigationStatus::Suspended;
            if resuming {
                validate_transition_investigation(state, shelf, InvestigationTransition::Resume)?;
            }
            if let Some(witness) = &self.draft.witness
                && record
                    .subjects()
                    .contains(&EntityRef::Character(witness.character))
            {
                return Err(InvestigationError::WitnessIsCaseSubject {
                    character: witness.character,
                });
            }
            if let Some(witness) = &self.draft.witness
                && let Some(existing) = state.legal.case_witness_for(shelf, witness.character)
            {
                return Err(InvestigationError::DuplicateIncidentWitness {
                    investigation: shelf,
                    witness: witness.character,
                    existing: existing.id(),
                });
            }
            if let Some(witness) = &self.draft.witness {
                match crate::legal::witness_system::case_witness_role_conflict(
                    state,
                    shelf,
                    witness.character,
                ) {
                    Some(
                        crate::legal::witness_system::CaseWitnessRoleConflict::LeadInvestigator,
                    ) => {
                        return Err(InvestigationError::WitnessIsLeadInvestigator {
                            investigation: shelf,
                            character: witness.character,
                        });
                    }
                    Some(
                        crate::legal::witness_system::CaseWitnessRoleConflict::AssignedProsecutor(
                            case,
                        ),
                    ) => {
                        return Err(InvestigationError::WitnessIsAssignedProsecutor {
                            investigation: shelf,
                            character: witness.character,
                            case,
                        });
                    }
                    None => {}
                }
            }
            for evidence in &self.draft.evidence {
                ensure_evidence_prosecution_recusal_capacity(
                    state,
                    shelf,
                    evidence.subject,
                    evidence.strength,
                    evidence.reliability,
                    evidence.admissibility,
                )?;
            }
            let adds_incident_context = self
                .draft
                .subjects
                .difference(record.declared_subjects())
                .next()
                .is_some();
            let advances = self
                .evidence_count()?
                .checked_add(u32::from(self.draft.witness.is_some()))
                .and_then(|count| count.checked_add(u32::from(adds_incident_context)))
                .and_then(|count| count.checked_add(u32::from(resuming)))
                .ok_or_else(|| VersionCapacityError::new("investigation"))?;
            crate::legal::investigation_work_execution::ensure_external_investigation_mutation_capacity(state, shelf, advances, None)?;
        } else {
            let advances = self
                .evidence_count()?
                .checked_add(u32::from(self.draft.witness.is_some()))
                .ok_or_else(|| VersionCapacityError::new("investigation"))?;
            ensure_version_can_advance_by(1, advances, "investigation")?;
        }
        Ok(())
    }

    pub fn commit(
        mut self,
        state: &mut AppState,
    ) -> Result<IncidentIntakeOutcome, InvestigationError> {
        self.ensure_current(state)?;
        state.ids.reserve_many(&self.id_budget()?)?;
        // The draft is consumed by this commit, so its subject set moves into the record
        // instead of being cloned.
        let subjects = std::mem::take(&mut self.draft.subjects);
        let mut resumed_shelf = false;
        let investigation = match self.continuing {
            Some((shelf, _)) => {
                resumed_shelf = state
                    .legal
                    .get_investigation(shelf)
                    .expect("continued file")
                    .status()
                    == InvestigationStatus::Suspended;
                if resumed_shelf {
                    // Canonical resume: revalidate lifecycle before extending the file.
                    validate_transition_investigation(
                        state,
                        shelf,
                        InvestigationTransition::Resume,
                    )?
                    .commit(state)?;
                }
                // A fresh case stores every validated incident subject at opening time. Preserve
                // the same semantic boundary on continuation instead of making weak-but-valid
                // non-character subject matter disappear merely because this incident found a
                // resumable shelf.
                state
                    .legal
                    .extend_investigation_incident_subjects(shelf, subjects);
                shelf
            }
            None => {
                let investigation = state
                    .ids
                    .next_investigation()
                    .expect("incident investigation ID was preflighted before mutation");
                state.legal.insert_investigation(InvestigationRecord {
                    id: investigation,
                    owner: self.draft.owner,
                    title: self.draft.title,
                    status: InvestigationStatus::Active,
                    lead_investigator: None,
                    declared_subjects: subjects.clone(),
                    subjects,
                    evidence: Default::default(),
                    opened_at: self.validated_at,
                    origin: self.draft.origin,
                    last_activity_at: self.validated_at,
                    version: 1,
                });
                investigation
            }
        };
        let case_witness = if let Some(witness) = self.draft.witness {
            let id = state
                .ids
                .next_case_witness()
                .expect("incident witness ID was preflighted before mutation");
            state.legal.insert_case_witness(
                CaseWitnessRecord {
                    id,
                    investigation,
                    witness: witness.character,
                    subject: witness.subject,
                    cooperation: witness.cooperation,
                    registered_at: self.validated_at,
                    statements: Default::default(),
                    interview_attempts: 0,
                    version: 1,
                },
                self.validated_at,
            );
            Some(id)
        } else {
            None
        };
        let mut evidence_ids = Vec::with_capacity(self.draft.evidence.len());
        for evidence in self.draft.evidence {
            let id = state
                .ids
                .next_evidence()
                .expect("incident evidence IDs were preflighted before mutation");
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
                self.validated_at,
            );
            evidence_ids.push(id);
        }
        Ok(IncidentIntakeOutcome {
            investigation,
            resumed_shelf,
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
    let continuing = find_incident_target(state, &draft);
    let validated = ValidatedIncidentIntake {
        draft,
        continuing,
        validated_at: state.now(),
    };
    validated.ensure_current(state)?;
    Ok(validated)
}

/// Route independent, identified operation incidents into a compatible active police file
/// before falling back to the existing suspended-shelf rule. This is intake, not a merge of
/// existing files. Mere district, organization, or anonymous operation overlap is insufficient.
fn find_incident_target(
    state: &AppState,
    draft: &IncidentIntakeDraft,
) -> Option<(InvestigationId, u32)> {
    find_active_identified_file(state, draft).or_else(|| find_resumable_shelf(state, draft))
}

fn find_active_identified_file(
    state: &AppState,
    draft: &IncidentIntakeDraft,
) -> Option<(InvestigationId, u32)> {
    let origin @ EntityRef::Operation(_) = draft.origin? else {
        return None;
    };
    let organization = case_origin_responsible_organization(state, origin)?;
    // The production operation intake carries exactly one primary identification and the
    // operation itself. Other incident vocabularies retain their separate routing contract.
    let [evidence] = draft.evidence.as_slice() else {
        return None;
    };
    let suspect = evidence.subject.as_character()?;
    if evidence.origin != Some(origin)
        || !draft.subjects.contains(&origin)
        || !draft.subjects.contains(&evidence.subject)
        || draft.subjects.len() != 2
        || !evidence_assessment_is_actionable_case_lead(
            evidence.strength,
            evidence.reliability,
            evidence.admissibility,
        )
    {
        return None;
    }
    state.legal.active_investigations_for_subject(EntityRef::Character(suspect))
        .filter(|record| {
            if record.owner() != draft.owner || record.subjects().contains(&origin) { return false; }
            let Some(case_origin @ EntityRef::Operation(_)) = record.origin() else { return false; };
            if case_origin_responsible_organization(state, case_origin) != Some(organization) { return false; }
            // Avoid creating a second registration for the same named witness, or taking a
            // witness already assigned to casework. A separate file is valid in either case.
            if !case_can_accept_incident_witness(state, record, draft) { return false; }
            // Reserve the append plus declared-subject update without stealing revisions from
            // an already scheduled investigator task. No existing evidence is copied or upgraded.
            let advances = 2 + u32::from(draft.witness.is_some());
            if crate::legal::investigation_work_execution::ensure_external_investigation_mutation_capacity(state, record.id(), advances, None).is_err() { return false; }
            record.evidence().iter().any(|id| {
                let prior = state.legal.get_evidence(*id).expect("indexed case evidence");
                prior.subject() == evidence.subject
                    && prior.origin() != Some(origin)
                    && prior.origin().is_some_and(|prior_origin| matches!(prior_origin, EntityRef::Operation(_)) && case_origin_responsible_organization(state, prior_origin) == Some(organization))
                    && prior.derived_from().is_empty()
                    && evidence_assessment_is_actionable_case_lead(prior.strength(), prior.reliability(), prior.admissibility())
            })
        })
        .max_by_key(|record| (record.last_activity_at(), Reverse(record.id())))
        .map(|record| (record.id(), record.version()))
}

/// Finds the owner's suspended originated shelf sharing subject matter with the draft, so a
/// later incident continues the most relevant existing file instead of opening a parallel one.
/// Exact origin continuity outranks subject overlap, broader overlap outranks narrower overlap,
/// and more recently active casework outranks an older shelf. Investigation ID is only the final
/// deterministic tie-breaker. Institution-authored cases (no origination link) keep their
/// explicit lifecycle and are never auto-resumed.
fn find_resumable_shelf(
    state: &AppState,
    draft: &IncidentIntakeDraft,
) -> Option<(InvestigationId, u32)> {
    state
        .legal
        .suspended_originated_investigations_for_owner(draft.owner)
        .filter_map(|record| {
            // A distinct incident cannot continue a shelf on which its eyewitness is
            // already a witness, case subject, or officer of the case. In that case,
            // opening a separate case preserves both real incidents and their testimony.
            // Re-intake of the identical origin still reaches the explicit duplicate
            // witness rejection in canonical validation.
            if record.origin() != draft.origin
                && !case_can_accept_incident_witness(state, record, draft)
            {
                return None;
            }
            let overlap = record.subjects().intersection(&draft.subjects).count();
            (overlap > 0).then_some((record, overlap))
        })
        .max_by_key(|(record, overlap)| {
            (
                record.origin() == draft.origin,
                *overlap,
                record.last_activity_at(),
                Reverse(record.id()),
            )
        })
        .map(|(record, _)| (record.id(), record.version()))
}

fn case_can_accept_incident_witness(
    state: &AppState,
    case: &InvestigationRecord,
    draft: &IncidentIntakeDraft,
) -> bool {
    draft.witness.as_ref().is_none_or(|witness| {
        !case
            .subjects()
            .contains(&EntityRef::Character(witness.character))
            && state
                .legal
                .case_witness_for(case.id(), witness.character)
                .is_none()
            && crate::legal::witness_system::case_witness_role_conflict(
                state,
                case.id(),
                witness.character,
            )
            .is_none()
    })
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
    if let Some(origin) = draft.origin {
        // Only entities that can legitimately originate casework may carry the link; the
        // vocabulary grows deliberately so hidden consumers never meet a surprise variant.
        if !matches!(origin, EntityRef::Operation(_) | EntityRef::Enterprise(_)) {
            return Err(InvestigationError::InvalidCaseOrigin(origin));
        }
        if !is_entity_present(state, origin) {
            return Err(InvestigationError::MissingEntity(origin));
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
        if !is_entity_present(state, witness.subject) {
            return Err(InvestigationError::MissingEntity(witness.subject));
        }
        let subject_belongs_to_incident = draft.origin == Some(witness.subject)
            || draft.subjects.contains(&witness.subject)
            || draft
                .evidence
                .iter()
                .any(|evidence| evidence.subject == witness.subject);
        if !subject_belongs_to_incident {
            return Err(InvestigationError::WitnessSubjectOutsideIncident {
                character: witness.character,
                subject: witness.subject,
            });
        }
    }
    for evidence in &draft.evidence {
        // Incident intake inserts evidence with `source: None`; work-derived kinds and
        // informant statements may only exist through their canonical production paths.
        validate_evidence_kind_allowed(evidence.kind)?;
        if let Some(enterprise) =
            super::enterprise_observation_origin(evidence.kind, evidence.subject, evidence.origin)
        {
            return Err(InvestigationError::InvalidEnterpriseObservation(enterprise));
        }
        if !is_entity_present(state, evidence.subject) {
            return Err(InvestigationError::MissingEntity(evidence.subject));
        }
        if let Some(origin) = evidence.origin
            && !is_entity_present(state, origin)
        {
            return Err(InvestigationError::MissingEntity(origin));
        }
        if evidence.discovered_at > state.now() {
            return Err(InvestigationError::DiscoveryInFuture);
        }
    }
    // Every incident subject must belong to the incident itself or have evidence in this intake.
    // Without this boundary a caller could attach an unrelated business, organization, or
    // neighborhood and silently alter shelf matching and district pressure. Concrete characters
    // carry the stronger actionable-evidence threshold because identifying a suspect also keeps
    // an originated case from cooling automatically.
    for subject in &draft.subjects {
        if draft.origin == Some(*subject) {
            continue;
        }
        let matching_evidence = draft
            .evidence
            .iter()
            .filter(|evidence| evidence.subject == *subject);
        match subject {
            EntityRef::Character(character) => {
                if !matching_evidence.into_iter().any(|evidence| {
                    evidence_assessment_is_actionable_case_lead(
                        evidence.strength,
                        evidence.reliability,
                        evidence.admissibility,
                    )
                }) {
                    return Err(
                        InvestigationError::UnsubstantiatedIncidentCharacterSubject {
                            character: *character,
                        },
                    );
                }
            }
            EntityRef::Organization(_)
            | EntityRef::Neighborhood(_)
            | EntityRef::Business(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => {
                if matching_evidence.into_iter().next().is_none() {
                    return Err(InvestigationError::UnsubstantiatedIncidentSubject {
                        subject: *subject,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Resolves the organization whose activity caused an originated case. Keeping this mapping in
/// the legal owner prevents incident validation and persisted-state validation from drifting on
/// what operation/enterprise provenance means for case visibility.
pub(crate) fn case_origin_responsible_organization(
    state: &AppState,
    origin: EntityRef,
) -> Option<OrganizationId> {
    match origin {
        EntityRef::Operation(operation) => state
            .operations
            .get_operation(operation)
            .map(|operation| operation.responsible_organization()),
        EntityRef::Enterprise(enterprise) => state
            .enterprises
            .get_enterprise(enterprise)
            .map(|enterprise| enterprise.organization()),
        EntityRef::Organization(_)
        | EntityRef::Character(_)
        | EntityRef::Neighborhood(_)
        | EntityRef::Business(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => None,
    }
}
