//! Atomic incident-to-case routing, including suspended-shelf continuation and incident evidence.

use super::{
    InvestigationError, InvestigationTransition, ensure_evidence_prosecution_recusal_capacity,
    evidence_assessment_is_actionable_case_lead, validate_evidence_kind_allowed,
    validate_investigation_draft, validate_transition_investigation,
};
use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{EvidenceId, IdKind, InvestigationId, OrganizationId};
use crate::core::state::AppState;
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
    /// The suspended originated shelf (and its pinned version) this intake continues, found
    /// at validation time and re-checked at commit so a stale token cannot resurrect a shelf
    /// whose state moved on.
    resuming: Option<(InvestigationId, u32)>,
}

impl ValidatedIncidentIntake {
    pub(crate) fn evidence_count(&self) -> Result<u32, InvestigationError> {
        u32::try_from(self.draft.evidence.len())
            .map_err(|_| InvestigationError::IncidentEvidenceCountOverflow)
    }

    pub(crate) fn has_witness(&self) -> bool {
        self.draft.witness.is_some()
    }

    pub(crate) fn requires_new_investigation(&self) -> bool {
        self.resuming.is_none()
    }

    /// Re-checks the read-only case-routing decision without allocating or mutating. Composite
    /// callers use this before committing unrelated state, so a shelf that appeared, resumed,
    /// or changed after validation cannot make incident intake fail after another domain moved.
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), InvestigationError> {
        validate_incident_intake_dependencies(state, &self.draft)?;
        let found = find_resumable_shelf(state, &self.draft);
        if found != self.resuming {
            return Err(InvestigationError::StaleIncidentShelf {
                expected: self.resuming,
                found,
            });
        }
        if let Some((shelf, _)) = self.resuming {
            validate_transition_investigation(state, shelf, InvestigationTransition::Resume)?;
            let record = state
                .legal
                .get_investigation(shelf)
                .expect("resumable incident shelf must still exist");
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
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| VersionCapacityError::new("investigation"))?;
            ensure_version_can_advance_by(record.version(), advances, "investigation")?;
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
        let mut budget = Vec::with_capacity(3);
        if self.resuming.is_none() {
            budget.push((IdKind::Investigation, 1));
        }
        budget.push((IdKind::Evidence, self.evidence_count()?));
        budget.push((IdKind::CaseWitness, u32::from(self.draft.witness.is_some())));
        state.ids.reserve_many(&budget)?;
        // The draft is consumed by this commit, so its subject set moves into the record
        // instead of being cloned.
        let subjects = std::mem::take(&mut self.draft.subjects);
        let investigation = match self.resuming {
            Some((shelf, _)) => {
                // Canonical resume: revalidates the lifecycle gate and refreshes the lead's
                // personal knowledge through the shared transition path.
                validate_transition_investigation(state, shelf, InvestigationTransition::Resume)?
                    .commit(state)?;
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
                    opened_at: state.now(),
                    origin: self.draft.origin,
                    last_activity_at: state.now(),
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
                    registered_at: state.now(),
                    statements: Default::default(),
                    interview_attempts: 0,
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
                state.now(),
            );
            evidence_ids.push(id);
        }
        Ok(IncidentIntakeOutcome {
            investigation,
            resumed_shelf: self.resuming.is_some(),
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
    let resuming = find_resumable_shelf(state, &draft);
    let validated = ValidatedIncidentIntake { draft, resuming };
    validated.ensure_current(state)?;
    Ok(validated)
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
