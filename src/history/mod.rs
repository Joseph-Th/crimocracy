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
    Investigation,
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
    by_entity: BTreeMap<EntityRef, BTreeSet<HistoryEventId>>,
}

impl HistoryState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_event(&self, id: HistoryEventId) -> Option<&HistoryEventRecord> {
        self.records.get(&id)
    }
    pub fn events_for(&self, entity: EntityRef) -> impl Iterator<Item = &HistoryEventRecord> {
        self.by_entity
            .get(&entity)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub(crate) fn events(&self) -> impl Iterator<Item = &HistoryEventRecord> {
        self.records.values()
    }
    pub(crate) fn insert(&mut self, event: HistoryEventRecord) {
        for entity in event.entities() {
            self.by_entity
                .entry(*entity)
                .or_default()
                .insert(event.id());
        }
        let previous = self.records.insert(event.id(), event);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate history event ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for event in self.records.values() {
            for entity in event.entities() {
                if !self
                    .by_entity
                    .get(entity)
                    .is_some_and(|ids| ids.contains(&event.id()))
                {
                    return false;
                }
            }
        }
        for (entity, ids) in &self.by_entity {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|event| event.entities().contains(entity))
                {
                    return false;
                }
            }
        }
        true
    }
    #[cfg(debug_assertions)]
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: history indexes disagree with source records"
        );
        for event in self.records.values() {
            for entity in event.entities() {
                debug_assert!(
                    self.by_entity
                        .get(entity)
                        .is_some_and(|ids| ids.contains(&event.id())),
                    "Index Completeness: history entity index is missing an event"
                );
            }
        }
    }
}

pub struct HistoryEventDraft {
    pub occurred_at: SimTime,
    pub kind: HistoryEventKind,
    pub summary: String,
    pub entities: BTreeSet<EntityRef>,
}
