//! Case-opening and evidence-link transactions; sibling legal state keeps the case graph synchronized.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{EvidenceId, InvestigationId, OrganizationId};
use crate::core::state::AppState;
use crate::legal::{
    EvidenceDraft, EvidenceRecord, IncidentIntakeDraft, InvestigationDraft, InvestigationRecord,
    InvestigationStatus,
};
use crate::world::{Lifecycle, OrganizationKind};
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
}

pub struct ValidatedInvestigation {
    draft: InvestigationDraft,
}
impl ValidatedInvestigation {
    pub fn commit(self, state: &mut AppState) -> Result<InvestigationId, InvestigationError> {
        validate_investigation_draft(state, &self.draft)?;
        let id = state.ids.next_investigation();
        state.legal.insert_investigation(InvestigationRecord {
            id,
            owner: self.draft.owner,
            title: self.draft.title,
            status: InvestigationStatus::Active,
            subjects: self.draft.subjects,
            evidence: Default::default(),
            opened_at: state.now(),
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

pub struct ValidatedEvidence {
    draft: EvidenceDraft,
}
impl ValidatedEvidence {
    pub fn commit(self, state: &mut AppState) -> Result<EvidenceId, InvestigationError> {
        validate_evidence_draft(state, &self.draft)?;
        let id = state.ids.next_evidence();
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
        state.legal.insert_evidence(EvidenceRecord {
            id,
            investigation,
            custodian,
            subject,
            origin,
            kind,
            strength,
            reliability,
            admissibility,
            discovered_at,
        });
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

fn validate_evidence_draft(
    state: &AppState,
    draft: &EvidenceDraft,
) -> Result<(), InvestigationError> {
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
}

pub struct ValidatedIncidentIntake {
    draft: IncidentIntakeDraft,
}

impl ValidatedIncidentIntake {
    pub fn commit(self, state: &mut AppState) -> Result<IncidentIntakeOutcome, InvestigationError> {
        validate_incident_intake_dependencies(state, &self.draft)?;
        let investigation = state.ids.next_investigation();
        state.legal.insert_investigation(InvestigationRecord {
            id: investigation,
            owner: self.draft.owner,
            title: self.draft.title,
            status: InvestigationStatus::Active,
            subjects: self.draft.subjects,
            evidence: Default::default(),
            opened_at: state.now(),
            version: 1,
        });
        let mut evidence_ids = Vec::with_capacity(self.draft.evidence.len());
        for evidence in self.draft.evidence {
            let id = state.ids.next_evidence();
            state.legal.insert_evidence(EvidenceRecord {
                id,
                investigation,
                custodian: self.draft.owner,
                subject: evidence.subject,
                origin: evidence.origin,
                kind: evidence.kind,
                strength: evidence.strength,
                reliability: evidence.reliability,
                admissibility: evidence.admissibility,
                discovered_at: evidence.discovered_at,
            });
            evidence_ids.push(id);
        }
        Ok(IncidentIntakeOutcome {
            investigation,
            evidence: evidence_ids,
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
    for evidence in &draft.evidence {
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
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::legal::{Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength};
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

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
}
