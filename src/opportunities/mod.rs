//! Persistent provenance-backed strategic opportunities; sibling systems own discovery and lifecycle transactions.

pub mod opportunity_system;

use crate::core::entity::EntityRef;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{InformationId, OperationId, OpportunityId, OrganizationId, ReportId};
use crate::core::time::SimTime;
use crate::core::version::advance_version_preflighted;
use crate::operations::OperationKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OperationOpportunityContext {
    operation_kind: OperationKind,
    targets: BTreeSet<EntityRef>,
}

impl OperationOpportunityContext {
    pub fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    pub fn targets(&self) -> &BTreeSet<EntityRef> {
        &self.targets
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OpportunityStatus {
    Open,
    Dismissed,
    Expired,
    Converted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpportunityResolution {
    Dismissed { at: SimTime },
    Expired { at: SimTime, report: ReportId },
    Converted { at: SimTime, operation: OperationId },
}

impl OpportunityResolution {
    pub fn at(self) -> SimTime {
        match self {
            Self::Dismissed { at } | Self::Expired { at, .. } | Self::Converted { at, .. } => at,
        }
    }

    pub fn operation(self) -> Option<OperationId> {
        match self {
            Self::Converted { operation, .. } => Some(operation),
            Self::Dismissed { .. } | Self::Expired { .. } => None,
        }
    }

    pub fn report(self) -> Option<ReportId> {
        match self {
            Self::Expired { report, .. } => Some(report),
            Self::Dismissed { .. } | Self::Converted { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpportunityRecord {
    id: OpportunityId,
    organization: OrganizationId,
    context: OperationOpportunityContext,
    discovered_at: SimTime,
    valid_until: Option<SimTime>,
    source_information: BTreeSet<InformationId>,
    summary: String,
    report: ReportId,
    resolution: Option<OpportunityResolution>,
    version: u32,
}

impl OpportunityRecord {
    pub fn id(&self) -> OpportunityId {
        self.id
    }

    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn context(&self) -> &OperationOpportunityContext {
        &self.context
    }

    pub fn discovered_at(&self) -> SimTime {
        self.discovered_at
    }

    pub fn valid_until(&self) -> Option<SimTime> {
        self.valid_until
    }

    pub fn source_information(&self) -> &BTreeSet<InformationId> {
        &self.source_information
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn report(&self) -> ReportId {
        self.report
    }

    pub fn resolution(&self) -> Option<OpportunityResolution> {
        self.resolution
    }

    pub fn status(&self) -> OpportunityStatus {
        match self.resolution {
            None => OpportunityStatus::Open,
            Some(OpportunityResolution::Dismissed { .. }) => OpportunityStatus::Dismissed,
            Some(OpportunityResolution::Expired { .. }) => OpportunityStatus::Expired,
            Some(OpportunityResolution::Converted { .. }) => OpportunityStatus::Converted,
        }
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct OperationOpportunityKey {
    organization: OrganizationId,
    operation_kind: OperationKind,
    targets: BTreeSet<EntityRef>,
}

impl OperationOpportunityKey {
    fn from_record(record: &OpportunityRecord) -> Self {
        let context = &record.context;
        Self {
            organization: record.organization,
            operation_kind: context.operation_kind,
            targets: context.targets.clone(),
        }
    }

    fn new(
        organization: OrganizationId,
        operation_kind: OperationKind,
        targets: BTreeSet<EntityRef>,
    ) -> Self {
        Self {
            organization,
            operation_kind,
            targets,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpportunityState {
    records: BTreeMap<OpportunityId, OpportunityRecord>,
    #[serde(skip)]
    by_report: BTreeMap<ReportId, OpportunityId>,
    #[serde(skip)]
    open_by_context: BTreeMap<OperationOpportunityKey, OpportunityId>,
    #[serde(skip)]
    open_by_expiry: BTreeMap<SimTime, BTreeSet<OpportunityId>>,
    #[serde(skip)]
    by_operation: BTreeMap<OperationId, OpportunityId>,
}

impl OpportunityState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.by_report.clear();
        self.open_by_context.clear();
        self.open_by_expiry.clear();
        self.by_operation.clear();
        for record in self.records.values() {
            let id = record.id();
            self.by_report.insert(record.report(), id);
            match record.resolution() {
                None => {
                    self.open_by_context
                        .insert(OperationOpportunityKey::from_record(record), id);
                    if let Some(valid_until) = record.valid_until() {
                        self.open_by_expiry
                            .entry(valid_until)
                            .or_default()
                            .insert(id);
                    }
                }
                Some(OpportunityResolution::Expired { report, .. }) => {
                    self.by_report.insert(report, id);
                }
                Some(OpportunityResolution::Converted { operation, .. }) => {
                    self.by_operation.insert(operation, id);
                }
                Some(OpportunityResolution::Dismissed { .. }) => {}
            }
        }
    }

    pub fn get_opportunity(&self, id: OpportunityId) -> Option<&OpportunityRecord> {
        self.records.get(&id)
    }

    pub fn opportunity_for_report(&self, report: ReportId) -> Option<&OpportunityRecord> {
        self.by_report.get(&report).map(|id| {
            self.records
                .get(id)
                .expect("opportunity report index must reference an opportunity")
        })
    }

    pub fn opportunity_for_operation(&self, operation: OperationId) -> Option<&OpportunityRecord> {
        self.by_operation.get(&operation).map(|id| {
            self.records
                .get(id)
                .expect("opportunity operation index must reference an opportunity")
        })
    }

    pub fn find_open_operation(
        &self,
        organization: OrganizationId,
        operation_kind: OperationKind,
        targets: &BTreeSet<EntityRef>,
    ) -> Option<&OpportunityRecord> {
        self.open_by_context
            .get(&OperationOpportunityKey::new(
                organization,
                operation_kind,
                targets.clone(),
            ))
            .map(|id| {
                self.records
                    .get(id)
                    .expect("open opportunity context index must reference an opportunity")
            })
    }

    pub(crate) fn opportunities(&self) -> impl Iterator<Item = &OpportunityRecord> {
        self.records.values()
    }
    pub(crate) fn opportunity_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }

    pub(crate) fn find_due_expiring(&self, now: SimTime) -> Vec<OpportunityId> {
        self.open_by_expiry
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn insert(&mut self, record: OpportunityRecord) {
        let id = record.id();
        let key = OperationOpportunityKey::from_record(&record);
        debug_assert_eq!(record.status(), OpportunityStatus::Open);
        debug_assert!(!self.records.contains_key(&id));
        debug_assert!(!self.open_by_context.contains_key(&key));

        let previous_report = self.by_report.insert(record.report(), id);
        debug_assert!(
            previous_report.is_none(),
            "one discovery report may describe only one opportunity"
        );
        self.open_by_context.insert(key, id);
        if let Some(valid_until) = record.valid_until() {
            self.open_by_expiry
                .entry(valid_until)
                .or_default()
                .insert(id);
        }
        self.records.insert(id, record);
    }

    pub(crate) fn dismiss(&mut self, id: OpportunityId, at: SimTime) {
        self.resolve(id, OpportunityResolution::Dismissed { at });
    }

    pub(crate) fn expire(&mut self, id: OpportunityId, at: SimTime, report: ReportId) {
        self.resolve(id, OpportunityResolution::Expired { at, report });
        let previous = self.by_report.insert(report, id);
        debug_assert!(
            previous.is_none(),
            "one opportunity lifecycle report may describe only one opportunity"
        );
    }

    pub(crate) fn convert(&mut self, id: OpportunityId, operation: OperationId, at: SimTime) {
        self.resolve(id, OpportunityResolution::Converted { at, operation });
        let previous = self.by_operation.insert(operation, id);
        debug_assert!(
            previous.is_none(),
            "one operation may convert only one opportunity"
        );
    }

    fn resolve(&mut self, id: OpportunityId, resolution: OpportunityResolution) {
        let record = self
            .records
            .get(&id)
            .expect("validated opportunity disappeared before lifecycle commit");
        debug_assert_eq!(record.status(), OpportunityStatus::Open);
        let key = OperationOpportunityKey::from_record(record);
        let valid_until = record.valid_until();
        let removed = self.open_by_context.remove(&key);
        debug_assert_eq!(removed, Some(id));
        if let Some(valid_until) = valid_until {
            let ids = self
                .open_by_expiry
                .get_mut(&valid_until)
                .expect("open opportunity expiry index must contain scheduled record");
            let removed = ids.remove(&id);
            debug_assert!(removed);
            if ids.is_empty() {
                self.open_by_expiry.remove(&valid_until);
            }
        }
        let record = self
            .records
            .get_mut(&id)
            .expect("validated opportunity disappeared before lifecycle mutation");
        record.resolution = Some(resolution);
        record.version = advance_version_preflighted(record.version);
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        self.records_have_consistent_indexes()
            && self.report_index_is_consistent()
            && self.open_context_index_is_consistent()
            && self.expiry_index_is_consistent()
            && self.operation_index_is_consistent()
    }

    /// Every opportunity must occupy exactly the projections implied by its lifecycle state.
    fn records_have_consistent_indexes(&self) -> bool {
        for (stored_id, record) in &self.records {
            let id = record.id();
            if *stored_id != id {
                return false;
            }
            if self.by_report.get(&record.report()) != Some(&id) {
                return false;
            }
            if let Some(report) = record.resolution().and_then(OpportunityResolution::report)
                && self.by_report.get(&report) != Some(&id)
            {
                return false;
            }
            let key = OperationOpportunityKey::from_record(record);
            match record.resolution() {
                None => {
                    if self.open_by_context.get(&key) != Some(&id) {
                        return false;
                    }
                    match record.valid_until() {
                        Some(at) => {
                            if !self
                                .open_by_expiry
                                .get(&at)
                                .is_some_and(|ids| ids.contains(&id))
                            {
                                return false;
                            }
                        }
                        None => {
                            if self.open_by_expiry.values().any(|ids| ids.contains(&id)) {
                                return false;
                            }
                        }
                    }
                }
                Some(OpportunityResolution::Converted { operation, .. }) => {
                    if self.open_by_context.get(&key) == Some(&id)
                        || self.by_operation.get(&operation) != Some(&id)
                        || self.open_by_expiry.values().any(|ids| ids.contains(&id))
                    {
                        return false;
                    }
                }
                Some(OpportunityResolution::Dismissed { .. })
                | Some(OpportunityResolution::Expired { .. }) => {
                    if self.open_by_context.get(&key) == Some(&id)
                        || self
                            .by_operation
                            .values()
                            .any(|opportunity| *opportunity == id)
                        || self.open_by_expiry.values().any(|ids| ids.contains(&id))
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Every report reverse lookup must be either the opportunity's discovery or expiry report.
    fn report_index_is_consistent(&self) -> bool {
        for (report, id) in &self.by_report {
            let Some(record) = self.records.get(id) else {
                return false;
            };
            let is_discovery_report = record.report() == *report;
            let is_resolution_report = match record.resolution() {
                Some(OpportunityResolution::Expired {
                    report: resolution_report,
                    ..
                }) => resolution_report == *report,
                None
                | Some(OpportunityResolution::Dismissed { .. })
                | Some(OpportunityResolution::Converted { .. }) => false,
            };
            if !is_discovery_report && !is_resolution_report {
                return false;
            }
        }
        true
    }

    /// Context deduplication contains open opportunities only and preserves the exact key.
    fn open_context_index_is_consistent(&self) -> bool {
        for (key, id) in &self.open_by_context {
            if self.records.get(id).is_none_or(|record| {
                record.status() != OpportunityStatus::Open
                    || OperationOpportunityKey::from_record(record) != *key
            }) {
                return false;
            }
        }
        true
    }

    /// The expiry schedule contains only open opportunities with that exact expiry instant.
    fn expiry_index_is_consistent(&self) -> bool {
        for (at, ids) in &self.open_by_expiry {
            if ids.iter().any(|id| {
                self.records.get(id).is_none_or(|record| {
                    record.status() != OpportunityStatus::Open || record.valid_until() != Some(*at)
                })
            }) {
                return false;
            }
        }
        true
    }

    /// Converted-operation lookups must point back to the opportunity that created them.
    fn operation_index_is_consistent(&self) -> bool {
        for (operation, id) in &self.by_operation {
            if self.records.get(id).is_none_or(|record| {
                record
                    .resolution()
                    .and_then(OpportunityResolution::operation)
                    != Some(*operation)
            }) {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct OperationOpportunityDraft {
    pub organization: OrganizationId,
    pub operation_kind: OperationKind,
    pub targets: BTreeSet<EntityRef>,
    pub source_information: BTreeSet<InformationId>,
    pub summary: String,
    pub valid_until: Option<SimTime>,
}
