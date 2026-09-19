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
    fn insert(&mut self, event: HistoryEventRecord) {
        let previous = self.records.insert(event.id(), event);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate history event ID inserted"
        );
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        // History has no secondary indexes, but the authoritative map key and monotone event
        // chronology are still part of durable identity. Canonical event IDs are allocated at
        // commit time, so iteration by ID must never rewind campaign time.
        let mut previous_at = None;
        for (id, event) in &self.records {
            if *id != event.id() || previous_at.is_some_and(|at| event.occurred_at() < at) {
                return false;
            }
            previous_at = Some(event.occurred_at());
        }
        true
    }
}

pub struct HistoryEventDraft {
    pub kind: HistoryEventKind,
    pub summary: String,
    pub entities: BTreeSet<EntityRef>,
}
