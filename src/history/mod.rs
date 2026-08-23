//! Durable entity-linked campaign history; `history_system` owns event insertion and indexing.

pub mod history_system;

use crate::core::entity::EntityRef;
use crate::core::id::HistoryEventId;
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
    pub(crate) fn insert(&mut self, event: HistoryEventRecord) {
        let previous = self.records.insert(event.id(), event);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate history event ID inserted"
        );
    }
}

pub struct HistoryEventDraft {
    pub occurred_at: SimTime,
    pub kind: HistoryEventKind,
    pub summary: String,
    pub entities: BTreeSet<EntityRef>,
}
