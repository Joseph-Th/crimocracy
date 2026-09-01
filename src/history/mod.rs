//! Durable entity-linked campaign history; `history_system` owns event validation and insertion.

pub mod history_system;

use crate::core::entity::EntityRef;
use crate::core::id::HistoryEventId;
use crate::core::id::IdKeyedBounds;
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Persistent campaign event categories actually produced by simulation systems.
/// Unused slots were deleted to preserve exhaustive-match discipline per ARCHITECTURE.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryEventKind {
    Operation,
    Recruitment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEventRecord {
    id: HistoryEventId,
    occurred_at: SimTime,
    kind: HistoryEventKind,
    summary: String,
    entities: BTreeSet<EntityRef>,
}

impl HistoryEventRecord {
    pub fn id(&self) -> HistoryEventId {
        self.id
    }
    pub fn occurred_at(&self) -> SimTime {
        self.occurred_at
    }
    pub fn kind(&self) -> HistoryEventKind {
        self.kind
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn entities(&self) -> &BTreeSet<EntityRef> {
        &self.entities
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryState {
    records: BTreeMap<HistoryEventId, HistoryEventRecord>,
}

impl HistoryState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_event(&self, id: HistoryEventId) -> Option<&HistoryEventRecord> {
        self.records.get(&id)
    }
    pub(crate) fn events(&self) -> impl Iterator<Item = &HistoryEventRecord> {
        self.records.values()
    }
    pub(crate) fn event_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }
    pub(crate) fn insert(&mut self, event: HistoryEventRecord) {
        let previous = self.records.insert(event.id(), event);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate history event ID inserted"
        );
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        // History is append-only with no derived indexes; consistency is key uniqueness,
        // which BTreeMap guarantees. Keep the hook so validate_indexes covers every substate.
        true
    }
}

pub struct HistoryEventDraft {
    pub occurred_at: SimTime,
    pub kind: HistoryEventKind,
    pub summary: String,
    pub entities: BTreeSet<EntityRef>,
}
