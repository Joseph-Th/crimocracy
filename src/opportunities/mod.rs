//! Persistent provenance-backed strategic opportunities; sibling systems own discovery and lifecycle transactions.

pub mod opportunity_system;

use crate::core::entity::EntityRef;
use crate::core::id::{InformationId, OperationId, OpportunityId, OrganizationId, ReportId};
use crate::core::time::SimTime;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpportunityContext {
    Operation(OperationOpportunityContext),
}

impl OpportunityContext {
    pub fn operation(&self) -> &OperationOpportunityContext {
        match self {
            Self::Operation(context) => context,
        }
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
    context: OpportunityContext,
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

    pub fn context(&self) -> &OpportunityContext {
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
        let context = record.context.operation();
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
    by_report: BTreeMap<ReportId, OpportunityId>,
    open_by_context: BTreeMap<OperationOpportunityKey, OpportunityId>,
    open_by_expiry: BTreeMap<SimTime, BTreeSet<OpportunityId>>,
    by_operation: BTreeMap<OperationId, OpportunityId>,
}

impl OpportunityState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_opportunity(&self, id: OpportunityId) -> Option<&OpportunityRecord> {
        self.records.get(&id)
    }

    pub fn opportunity_for_report(&self, report: ReportId) -> Option<&OpportunityRecord> {
        self.by_report
            .get(&report)
            .and_then(|id| self.records.get(id))
    }

    pub fn opportunity_for_operation(&self, operation: OperationId) -> Option<&OpportunityRecord> {
        self.by_operation
            .get(&operation)
            .and_then(|id| self.records.get(id))
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
            .and_then(|id| self.records.get(id))
    }

    pub(crate) fn opportunities(&self) -> impl Iterator<Item = &OpportunityRecord> {
        self.records.values()
    }

    pub(crate) fn due_expiring_at_or_before(&self, now: SimTime) -> Vec<OpportunityId> {
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
        record.version = record
            .version
            .checked_add(1)
            .expect("opportunity version overflowed u32");
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            let id = record.id();
            if self.by_report.get(&record.report()) != Some(&id) {
                return false;
            }
            if let Some(report) = record.resolution().and_then(OpportunityResolution::report) {
                if self.by_report.get(&report) != Some(&id) {
                    return false;
                }
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
        for (key, id) in &self.open_by_context {
            if self.records.get(id).is_none_or(|record| {
                record.status() != OpportunityStatus::Open
                    || OperationOpportunityKey::from_record(record) != *key
            }) {
                return false;
            }
        }
        for (at, ids) in &self.open_by_expiry {
            if ids.iter().any(|id| {
                self.records.get(id).is_none_or(|record| {
                    record.status() != OpportunityStatus::Open || record.valid_until() != Some(*at)
                })
            }) {
                return false;
            }
        }
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

    #[cfg(debug_assertions)]
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: opportunity indexes disagree with source records"
        );
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
