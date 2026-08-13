//! Durable authority exceptions and decision records; `decision_system` owns request and resolution transactions.

pub mod decision_system;

use crate::core::attention::AttentionClass;
use crate::core::id::{CharacterId, DecisionRequestId, OperationId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationExceptionReason {
    UnexpectedCondition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionContext {
    OperationException {
        operation: OperationId,
        reason: OperationExceptionReason,
    },
}

impl DecisionContext {
    pub fn operation(self) -> OperationId {
        match self {
            Self::OperationException {
                operation,
                reason: _,
            } => operation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DecisionResponse {
    Continue,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStatus {
    Pending,
    Resolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionResolution {
    response: DecisionResponse,
    resolved_at: SimTime,
    resolved_by: OrganizationId,
}

impl DecisionResolution {
    pub fn response(self) -> DecisionResponse {
        self.response
    }

    pub fn resolved_at(self) -> SimTime {
        self.resolved_at
    }

    pub fn resolved_by(self) -> OrganizationId {
        self.resolved_by
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum DecisionLifecycle {
    Pending,
    Resolved(DecisionResolution),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecisionRequestRecord {
    id: DecisionRequestId,
    recipient: OrganizationId,
    requester: CharacterId,
    context: DecisionContext,
    attention: AttentionClass,
    summary: String,
    requested_at: SimTime,
    options: BTreeSet<DecisionResponse>,
    lifecycle: DecisionLifecycle,
    version: u32,
}

impl DecisionRequestRecord {
    pub fn id(&self) -> DecisionRequestId {
        self.id
    }

    pub fn recipient(&self) -> OrganizationId {
        self.recipient
    }

    pub fn requester(&self) -> CharacterId {
        self.requester
    }

    pub fn context(&self) -> DecisionContext {
        self.context
    }

    pub fn attention(&self) -> AttentionClass {
        self.attention
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn requested_at(&self) -> SimTime {
        self.requested_at
    }

    pub fn options(&self) -> &BTreeSet<DecisionResponse> {
        &self.options
    }

    pub fn status(&self) -> DecisionStatus {
        match self.lifecycle {
            DecisionLifecycle::Pending => DecisionStatus::Pending,
            DecisionLifecycle::Resolved(_) => DecisionStatus::Resolved,
        }
    }

    pub fn resolution(&self) -> Option<DecisionResolution> {
        match self.lifecycle {
            DecisionLifecycle::Pending => None,
            DecisionLifecycle::Resolved(resolution) => Some(resolution),
        }
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DecisionState {
    records: BTreeMap<DecisionRequestId, DecisionRequestRecord>,
    by_recipient: BTreeMap<OrganizationId, BTreeSet<DecisionRequestId>>,
    pending_by_recipient: BTreeMap<OrganizationId, BTreeSet<DecisionRequestId>>,
    pending_by_operation: BTreeMap<OperationId, DecisionRequestId>,
}

impl DecisionState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_decision(&self, id: DecisionRequestId) -> Option<&DecisionRequestRecord> {
        self.records.get(&id)
    }

    pub fn decisions_for_recipient(
        &self,
        recipient: OrganizationId,
    ) -> impl Iterator<Item = &DecisionRequestRecord> {
        self.by_recipient
            .get(&recipient)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn pending_for_recipient(
        &self,
        recipient: OrganizationId,
    ) -> impl Iterator<Item = &DecisionRequestRecord> {
        self.pending_by_recipient
            .get(&recipient)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn pending_for_operation(&self, operation: OperationId) -> Option<DecisionRequestId> {
        self.pending_by_operation.get(&operation).copied()
    }

    pub(crate) fn decisions(&self) -> impl Iterator<Item = &DecisionRequestRecord> {
        self.records.values()
    }

    pub(crate) fn insert(&mut self, record: DecisionRequestRecord) {
        let id = record.id();
        let recipient = record.recipient();
        let operation = record.context().operation();
        self.by_recipient.entry(recipient).or_default().insert(id);
        self.pending_by_recipient
            .entry(recipient)
            .or_default()
            .insert(id);
        let previous_operation = self.pending_by_operation.insert(operation, id);
        debug_assert!(
            previous_operation.is_none(),
            "Index Uniqueness: operation already has a pending decision"
        );
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate decision request ID inserted"
        );
    }

    pub(crate) fn resolve(&mut self, id: DecisionRequestId, resolution: DecisionResolution) {
        let (recipient, operation) = {
            let record = self
                .records
                .get(&id)
                .expect("validated decision disappeared before resolution commit");
            (record.recipient(), record.context().operation())
        };

        if let Some(ids) = self.pending_by_recipient.get_mut(&recipient) {
            ids.remove(&id);
            if ids.is_empty() {
                self.pending_by_recipient.remove(&recipient);
            }
        }
        let removed = self.pending_by_operation.remove(&operation);
        debug_assert_eq!(
            removed,
            Some(id),
            "Derived Data Consistency: pending operation decision index disagrees with record"
        );

        let record = self
            .records
            .get_mut(&id)
            .expect("validated decision disappeared before resolution commit");
        record.lifecycle = DecisionLifecycle::Resolved(resolution);
        record.version = record
            .version
            .checked_add(1)
            .expect("decision request version counter exhausted");
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            if !self
                .by_recipient
                .get(&record.recipient())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            match record.status() {
                DecisionStatus::Pending => {
                    if !self
                        .pending_by_recipient
                        .get(&record.recipient())
                        .is_some_and(|ids| ids.contains(&record.id()))
                    {
                        return false;
                    }
                    if self.pending_by_operation.get(&record.context().operation())
                        != Some(&record.id())
                    {
                        return false;
                    }
                }
                DecisionStatus::Resolved => {
                    if self
                        .pending_by_recipient
                        .get(&record.recipient())
                        .is_some_and(|ids| ids.contains(&record.id()))
                        || self.pending_by_operation.get(&record.context().operation())
                            == Some(&record.id())
                    {
                        return false;
                    }
                }
            }
        }

        for (recipient, ids) in &self.by_recipient {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.recipient() == *recipient)
                {
                    return false;
                }
            }
        }
        for (recipient, ids) in &self.pending_by_recipient {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.recipient() == *recipient && record.status() == DecisionStatus::Pending
                }) {
                    return false;
                }
            }
        }
        for (operation, id) in &self.pending_by_operation {
            if !self.records.get(id).is_some_and(|record| {
                record.context().operation() == *operation
                    && record.status() == DecisionStatus::Pending
            }) {
                return false;
            }
        }
        true
    }

    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: decision indexes disagree with source records"
        );
    }
}

#[derive(Clone, Debug)]
pub struct DecisionRequestDraft {
    pub requester: CharacterId,
    pub context: DecisionContext,
    pub attention: AttentionClass,
    pub summary: String,
}

pub(crate) struct DecisionRecordParts {
    pub id: DecisionRequestId,
    pub recipient: OrganizationId,
    pub draft: DecisionRequestDraft,
    pub requested_at: SimTime,
    pub options: BTreeSet<DecisionResponse>,
}

impl From<DecisionRecordParts> for DecisionRequestRecord {
    fn from(parts: DecisionRecordParts) -> Self {
        let DecisionRecordParts {
            id,
            recipient,
            draft,
            requested_at,
            options,
        } = parts;
        let DecisionRequestDraft {
            requester,
            context,
            attention,
            summary,
        } = draft;
        Self {
            id,
            recipient,
            requester,
            context,
            attention,
            summary,
            requested_at,
            options,
            lifecycle: DecisionLifecycle::Pending,
            version: 1,
        }
    }
}

pub(crate) fn build_resolution(
    response: DecisionResponse,
    resolved_at: SimTime,
    resolved_by: OrganizationId,
) -> DecisionResolution {
    DecisionResolution {
        response,
        resolved_at,
        resolved_by,
    }
}
