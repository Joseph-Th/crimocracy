//! Case-opening and evidence-link transactions; sibling legal state keeps the case graph synchronized.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{EvidenceId, InvestigationId, OrganizationId};
use crate::core::state::AppState;
use crate::legal::{
    EvidenceDraft, EvidenceRecord, InvestigationDraft, InvestigationRecord, InvestigationStatus,
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
    #[error("evidence discovery time cannot be in the future")]
    DiscoveryInFuture,
    #[error("evidence cannot be added to an inactive investigation")]
    InactiveInvestigation,
}

pub struct ValidatedInvestigation {
    draft: InvestigationDraft,
}
impl ValidatedInvestigation {
    pub fn commit(self, state: &mut AppState) -> InvestigationId {
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
        id
    }
}

pub fn validate_open_investigation(
    state: &AppState,
    draft: InvestigationDraft,
) -> Result<ValidatedInvestigation, InvestigationError> {
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
    for subject in &draft.subjects {
        if !is_entity_present(state, *subject) {
            return Err(InvestigationError::MissingEntity(*subject));
        }
    }
    Ok(ValidatedInvestigation { draft })
}

pub struct ValidatedEvidence {
    draft: EvidenceDraft,
}
impl ValidatedEvidence {
    pub fn commit(self, state: &mut AppState) -> EvidenceId {
        let id = state.ids.next_evidence();
        let EvidenceDraft {
            investigation,
            custodian,
            subject,
            kind,
            strength,
            admissibility,
            discovered_at,
        } = self.draft;
        state.legal.insert_evidence(EvidenceRecord {
            id,
            investigation,
            custodian,
            subject,
            kind,
            strength,
            admissibility,
            discovered_at,
        });
        id
    }
}

pub fn validate_add_evidence(
    state: &AppState,
    draft: EvidenceDraft,
) -> Result<ValidatedEvidence, InvestigationError> {
    let investigation = state.legal.get_investigation(draft.investigation).ok_or(
        InvestigationError::MissingInvestigation(draft.investigation),
    )?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationError::InactiveInvestigation);
    }
    if state.world.get_organization(draft.custodian).is_none() {
        return Err(InvestigationError::MissingOrganization(draft.custodian));
    }
    if !is_entity_present(state, draft.subject) {
        return Err(InvestigationError::MissingEntity(draft.subject));
    }
    if draft.discovered_at > state.now() {
        return Err(InvestigationError::DiscoveryInFuture);
    }
    Ok(ValidatedEvidence { draft })
}
