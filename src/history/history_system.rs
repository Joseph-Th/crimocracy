//! Campaign-history validation and insertion; sibling history state owns the record map.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{HistoryEventId, IdExhaustionError};
use crate::core::state::AppState;
use crate::core::time::{SimTime, ensure_time_current};
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
    #[error("history event was validated at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedHistoryEvent {
    draft: HistoryEventDraft,
    occurred_at: SimTime,
}
impl ValidatedHistoryEvent {
    pub fn commit(self, state: &mut AppState) -> Result<HistoryEventId, HistoryError> {
        ensure_time_current(state.now(), self.occurred_at)
            .map_err(|(expected, found)| HistoryError::StaleTime { expected, found })?;
        let id = state.ids.next_history_event()?;
        state.history.insert(HistoryEventRecord {
            id,
            occurred_at: self.occurred_at,
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
    for entity in &draft.entities {
        if !is_entity_present(state, *entity) {
            return Err(HistoryError::MissingEntity(*entity));
        }
    }
    Ok(ValidatedHistoryEvent {
        draft,
        occurred_at: state.now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::time::SimDuration;
    use crate::history::{HistoryEventDraft, HistoryEventKind};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};
    use std::collections::BTreeSet;

    fn draft(organization: crate::core::id::OrganizationId, summary: &str) -> HistoryEventDraft {
        HistoryEventDraft {
            kind: HistoryEventKind::Recruitment,
            summary: summary.to_owned(),
            entities: BTreeSet::from([EntityRef::Organization(organization)]),
        }
    }

    #[test]
    fn validated_history_event_rejects_clock_drift_before_allocating_or_inserting() {
        let registry = build_registry();
        let mut state = AppState::new(0x4859_5354);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "History Freshness Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("history fixture organization should validate");

        state.advance_clock(SimDuration::from_minutes(10));
        let stale = validate_record_event(&state, draft(organization, "Prepared history event"))
            .expect("history event should validate at the current instant");

        state.advance_clock(SimDuration::ONE_MINUTE);
        let fresh = validate_record_event(&state, draft(organization, "Fresh history event"))
            .expect("fresh history event should validate")
            .commit(&mut state)
            .expect("fresh history event should commit");
        assert_eq!(
            state
                .history()
                .get_event(fresh)
                .expect("fresh history event should persist")
                .occurred_at(),
            state.now()
        );

        let error = stale
            .commit(&mut state)
            .expect_err("history token held across a clock advance must stale");
        assert_eq!(
            error,
            HistoryError::StaleTime {
                expected: crate::core::time::SimTime::from_minutes(10),
                found: crate::core::time::SimTime::from_minutes(11),
            }
        );
        assert_eq!(
            state.history().events().count(),
            1,
            "stale history commit must not allocate or insert a record"
        );
        crate::core::invariants::validate_invariants(&state);
    }
}
