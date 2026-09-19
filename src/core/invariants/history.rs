//! Release-safe structural validation for persisted campaign history.

use crate::core::entity::is_entity_present;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;

pub(super) fn validate_history(state: &AppState) -> Result<(), StateValidationError> {
    for event in state.history.events() {
        if event.summary().trim().is_empty() {
            return Err(StateValidationError::EmptyHistorySummary { event: event.id() });
        }
        if event.entities().is_empty() {
            return Err(StateValidationError::HistoryEventHasNoEntities { event: event.id() });
        }
        if event.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "history event",
            });
        }
        for entity in event.entities() {
            if !is_entity_present(state, *entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "history event",
                    entity: *entity,
                });
            }
        }
    }
    Ok(())
}
