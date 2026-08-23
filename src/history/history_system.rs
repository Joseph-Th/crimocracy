//! Campaign-history validation and insertion; sibling history state owns the record map.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{HistoryEventId, IdExhaustionError};
use crate::core::state::AppState;
use crate::history::{HistoryEventDraft, HistoryEventRecord};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum HistoryError {
    #[error("history summary must not be empty")]
    EmptySummary,
    #[error("history event must reference at least one entity")]
    NoEntities,
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("history event cannot occur in the future")]
    OccursInFuture,
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedHistoryEvent {
    draft: HistoryEventDraft,
}
impl ValidatedHistoryEvent {
    pub fn commit(self, state: &mut AppState) -> Result<HistoryEventId, HistoryError> {
        let id = state.ids.next_history_event()?;
        state.history.insert(HistoryEventRecord {
            id,
            occurred_at: self.draft.occurred_at,
            kind: self.draft.kind,
            summary: self.draft.summary,
            entities: self.draft.entities,
        });
        Ok(id)
    }
}

pub fn validate_record_event(
    state: &AppState,
    draft: HistoryEventDraft,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    if draft.summary.trim().is_empty() {
        return Err(HistoryError::EmptySummary);
    }
    if draft.entities.is_empty() {
        return Err(HistoryError::NoEntities);
    }
    if draft.occurred_at > state.now() {
        return Err(HistoryError::OccursInFuture);
    }
    for entity in &draft.entities {
        if !is_entity_present(state, *entity) {
            return Err(HistoryError::MissingEntity(*entity));
        }
    }
    Ok(ValidatedHistoryEvent { draft })
}
