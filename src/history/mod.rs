//! Durable entity-linked campaign history; `history_system` owns event validation and insertion.

pub mod history_system;

use crate::core::entity::EntityRef;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{CharacterId, HistoryEventId, OrganizationId};
use crate::core::time::SimTime;
use crate::finance::Money;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Persistent campaign event categories actually produced by simulation systems.
/// Unused slots were deleted to preserve exhaustive-match discipline per ARCHITECTURE.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryEventKind {
    Operation,
    Recruitment,
    PayrollDeparture {
        member: CharacterId,
        organization: OrganizationId,
        resulting_version: u32,
        daily_wage: Money,
    },
    CampaignVictory {
        organization: OrganizationId,
        path: crate::core::campaign_victory::VictoryPath,
        settlement: Option<crate::core::id::LedgerTransactionId>,
    },
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
    #[serde(skip)]
    latest_departures: BTreeMap<(CharacterId, OrganizationId), HistoryEventId>,
    #[serde(skip)]
    victory: Option<HistoryEventId>,
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
    pub(crate) fn latest_payroll_departure(
        &self,
        member: CharacterId,
        organization: OrganizationId,
    ) -> Option<&HistoryEventRecord> {
        self.latest_departures
            .get(&(member, organization))
            .and_then(|id| self.get_event(*id))
    }
    pub(crate) fn organization_has_payroll_departures(&self, organization: OrganizationId) -> bool {
        self.latest_departures
            .keys()
            .any(|(_, owner)| *owner == organization)
    }

    pub fn campaign_victory(&self, organization: OrganizationId) -> Option<&HistoryEventRecord> {
        self.victory.and_then(|id| self.get_event(id)).filter(|event| {
            matches!(event.kind(), HistoryEventKind::CampaignVictory { organization: owner, .. } if owner == organization)
        })
    }

    fn derive_victory_index(&self) -> Option<HistoryEventId> {
        self.records.values().find_map(|event| {
            matches!(event.kind(), HistoryEventKind::CampaignVictory { .. }).then_some(event.id())
        })
    }

    fn derive_departure_index(&self) -> BTreeMap<(CharacterId, OrganizationId), HistoryEventId> {
        self.records
            .values()
            .filter_map(|record| match record.kind() {
                HistoryEventKind::PayrollDeparture {
                    member,
                    organization,
                    ..
                } => Some(((member, organization), record.id())),
                HistoryEventKind::Operation
                | HistoryEventKind::Recruitment
                | HistoryEventKind::CampaignVictory { .. } => None,
            })
            .collect()
    }
    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.latest_departures = self.derive_departure_index();
        self.victory = self.derive_victory_index();
    }
    fn insert(&mut self, event: HistoryEventRecord) {
        if matches!(event.kind(), HistoryEventKind::CampaignVictory { .. }) {
            debug_assert!(self.victory.is_none());
            self.victory = Some(event.id());
        }
        if let HistoryEventKind::PayrollDeparture {
            member,
            organization,
            ..
        } = event.kind()
        {
            self.latest_departures
                .insert((member, organization), event.id());
        }
        let previous = self.records.insert(event.id(), event);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate history event ID inserted"
        );
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        // Reconcile the departure index and the authoritative map key and monotone event
        // chronology are still part of durable identity. Canonical event IDs are allocated at
        // commit time, so iteration by ID must never rewind campaign time.
        if self.latest_departures != self.derive_departure_index()
            || self.victory != self.derive_victory_index()
        {
            return false;
        }
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
