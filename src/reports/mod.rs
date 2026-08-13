//! Persisted player-facing reports; `report_system` validates source and entity links before insertion.

pub mod report_system;

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{DecisionRequestId, InformationId, OrganizationId, ReportId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportKind {
    ExecutiveBrief,
    Financial,
    Surveillance,
    PoliceIntelligence,
    Newspaper,
    Legal,
    Accounting,
    Informant,
    AfterAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportEntry {
    pub attention: AttentionClass,
    pub summary: String,
    pub sources: Vec<InformationId>,
    pub entities: BTreeSet<EntityRef>,
    pub decision: Option<DecisionRequestId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportRecord {
    id: ReportId,
    recipient: OrganizationId,
    kind: ReportKind,
    title: String,
    generated_at: SimTime,
    entries: Vec<ReportEntry>,
}

impl ReportRecord {
    pub fn id(&self) -> ReportId {
        self.id
    }
    pub fn recipient(&self) -> OrganizationId {
        self.recipient
    }
    pub fn kind(&self) -> ReportKind {
        self.kind
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn generated_at(&self) -> SimTime {
        self.generated_at
    }
    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReportState {
    records: BTreeMap<ReportId, ReportRecord>,
    by_recipient: BTreeMap<OrganizationId, BTreeSet<ReportId>>,
}

impl ReportState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_report(&self, id: ReportId) -> Option<&ReportRecord> {
        self.records.get(&id)
    }
    pub fn reports_for(&self, recipient: OrganizationId) -> impl Iterator<Item = &ReportRecord> {
        self.by_recipient
            .get(&recipient)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub(crate) fn reports(&self) -> impl Iterator<Item = &ReportRecord> {
        self.records.values()
    }
    pub(crate) fn insert(&mut self, report: ReportRecord) {
        self.by_recipient
            .entry(report.recipient())
            .or_default()
            .insert(report.id());
        let previous = self.records.insert(report.id(), report);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate report ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for report in self.records.values() {
            if !self
                .by_recipient
                .get(&report.recipient())
                .is_some_and(|ids| ids.contains(&report.id()))
            {
                return false;
            }
        }
        for (recipient, ids) in &self.by_recipient {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|report| report.recipient() == *recipient)
                {
                    return false;
                }
            }
        }
        true
    }
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: report indexes disagree with source records"
        );
        for report in self.records.values() {
            debug_assert!(
                self.by_recipient
                    .get(&report.recipient())
                    .is_some_and(|ids| ids.contains(&report.id())),
                "Index Completeness: report recipient index is missing a report"
            );
        }
    }
}

pub struct ReportDraft {
    pub recipient: OrganizationId,
    pub kind: ReportKind,
    pub title: String,
    pub entries: Vec<ReportEntry>,
}
