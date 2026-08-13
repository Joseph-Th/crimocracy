//! Knowledge validation and recording; sibling intelligence state never infers hidden truth for callers.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{CharacterId, InformationId, OrganizationId};
use crate::core::state::AppState;
use crate::intelligence::{InformationDraft, InformationRecord, KnowledgeHolder};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IntelligenceError {
    #[error("information summary must not be empty")]
    EmptySummary,
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("observation time cannot be later than the current simulation time")]
    ObservationInFuture,
}

pub struct ValidatedInformation {
    draft: InformationDraft,
}

impl ValidatedInformation {
    pub fn commit(self, state: &mut AppState) -> InformationId {
        let InformationDraft {
            holder,
            source_kind,
            source_entity,
            subject,
            observed_at,
            reliability,
            specificity,
            summary,
        } = self.draft;
        let id = state.ids.next_information();
        let recorded_at = state.now();
        state.intelligence.insert(InformationRecord {
            id,
            holder,
            source_kind,
            source_entity,
            subject,
            observed_at,
            recorded_at,
            reliability,
            specificity,
            summary,
        });
        id
    }
}

pub fn validate_record_information(
    state: &AppState,
    draft: InformationDraft,
) -> Result<ValidatedInformation, IntelligenceError> {
    if draft.summary.trim().is_empty() {
        return Err(IntelligenceError::EmptySummary);
    }
    match draft.holder {
        KnowledgeHolder::Character(id) if state.world.get_character(id).is_none() => {
            return Err(IntelligenceError::MissingCharacter(id))
        }
        KnowledgeHolder::Organization(id) if state.world.get_organization(id).is_none() => {
            return Err(IntelligenceError::MissingOrganization(id))
        }
        KnowledgeHolder::Character(_) | KnowledgeHolder::Organization(_) => {}
    }
    if !is_entity_present(state, draft.subject) {
        return Err(IntelligenceError::MissingEntity(draft.subject));
    }
    if let Some(source) = draft.source_entity {
        if !is_entity_present(state, source) {
            return Err(IntelligenceError::MissingEntity(source));
        }
    }
    if draft.observed_at > state.now() {
        return Err(IntelligenceError::ObservationInFuture);
    }
    Ok(ValidatedInformation { draft })
}
