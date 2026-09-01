//! Durable typed decision records for authority exceptions and organizational approvals; `decision_system` owns request and resolution transactions.

pub mod decision_system;

use crate::core::attention::AttentionClass;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    CharacterId, DecisionRequestId, OperationId, OrganizationId, PoliceResponseId,
};
use crate::core::time::SimTime;
use crate::delegation::MandateAuthority;
use crate::recruitment::{RecruitmentApproach, RecruitmentPolicySource};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentApprovalAuthoritySnapshot {
    authority: MandateAuthority,
    mandate_version: u32,
    manager_version: u32,
    policy_source: RecruitmentPolicySource,
}

impl RecruitmentApprovalAuthoritySnapshot {
    pub fn authority(self) -> MandateAuthority {
        self.authority
    }

    pub fn mandate_version(self) -> u32 {
        self.mandate_version
    }

    pub fn manager_version(self) -> u32 {
        self.manager_version
    }

    pub fn policy_source(self) -> RecruitmentPolicySource {
        self.policy_source
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentApprovalContext {
    target_organization: OrganizationId,
    recruiter: CharacterId,
    candidate: CharacterId,
    approach: RecruitmentApproach,
    authority: RecruitmentApprovalAuthoritySnapshot,
}

impl RecruitmentApprovalContext {
    pub fn target_organization(self) -> OrganizationId {
        self.target_organization
    }

    pub fn recruiter(self) -> CharacterId {
        self.recruiter
    }

    pub fn candidate(self) -> CharacterId {
        self.candidate
    }

    pub fn approach(self) -> RecruitmentApproach {
        self.approach
    }

    pub fn authority(self) -> RecruitmentApprovalAuthoritySnapshot {
        self.authority
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionContext {
    /// A post-entry police arrival paused an in-progress operation and leadership must
    /// choose whether the crew continues or stands down.
    OperationPoliceArrival {
        operation: OperationId,
        response: PoliceResponseId,
    },
    RecruitmentApproval(RecruitmentApprovalContext),
}

impl DecisionContext {
    pub fn operation(self) -> Option<OperationId> {
        match self {
            Self::OperationPoliceArrival { operation, .. } => Some(operation),
            Self::RecruitmentApproval(_) => None,
        }
    }

    fn pending_key(self) -> DecisionPendingKey {
        match self {
            Self::OperationPoliceArrival { operation, .. } => {
                DecisionPendingKey::Operation(operation)
            }
            Self::RecruitmentApproval(context) => DecisionPendingKey::RecruitmentApproval {
                target_organization: context.target_organization(),
                candidate: context.candidate(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DecisionResponse {
    Continue,
    Abort,
    Approve,
    Reject,
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
    by_operation: BTreeMap<OperationId, BTreeSet<DecisionRequestId>>,
    pending_by_recipient: BTreeMap<OrganizationId, BTreeSet<DecisionRequestId>>,
    pending_by_context: BTreeMap<DecisionPendingKey, DecisionRequestId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum DecisionPendingKey {
    Operation(OperationId),
    RecruitmentApproval {
        target_organization: OrganizationId,
        candidate: CharacterId,
    },
}

impl DecisionState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_decision(&self, id: DecisionRequestId) -> Option<&DecisionRequestRecord> {
        self.records.get(&id)
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
        self.pending_by_context
            .get(&DecisionPendingKey::Operation(operation))
            .copied()
    }

    pub fn decisions_for_operation(
        &self,
        operation: OperationId,
    ) -> impl Iterator<Item = &DecisionRequestRecord> {
        self.by_operation
            .get(&operation)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    /// One live approval per (target organization, candidate): two managers of the same
    /// organization cannot both hold an approvable request for the same candidate, which
    /// would strand the loser as permanently unresolvable after the first approval flips
    /// membership.
    pub fn pending_for_recruitment_approval(
        &self,
        target_organization: OrganizationId,
        candidate: CharacterId,
    ) -> Option<DecisionRequestId> {
        self.pending_by_context
            .get(&DecisionPendingKey::RecruitmentApproval {
                target_organization,
                candidate,
            })
            .copied()
    }

    pub(crate) fn decisions(&self) -> impl Iterator<Item = &DecisionRequestRecord> {
        self.records.values()
    }
    pub(crate) fn decision_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }

    pub(crate) fn insert(&mut self, record: DecisionRequestRecord) {
        let id = record.id();
        let recipient = record.recipient();
        let operation = record.context().operation();
        let pending_key = record.context().pending_key();
        if let Some(operation) = operation {
            self.by_operation.entry(operation).or_default().insert(id);
        }
        self.pending_by_recipient
            .entry(recipient)
            .or_default()
            .insert(id);
        let previous_context = self.pending_by_context.insert(pending_key, id);
        debug_assert!(
            previous_context.is_none(),
            "Index Uniqueness: decision context already has a pending decision"
        );
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate decision request ID inserted"
        );
    }

    pub(crate) fn resolve(&mut self, id: DecisionRequestId, resolution: DecisionResolution) {
        let (recipient, pending_key) = {
            let record = self
                .records
                .get(&id)
                .expect("validated decision disappeared before resolution commit");
            (record.recipient(), record.context().pending_key())
        };

        if let Some(ids) = self.pending_by_recipient.get_mut(&recipient) {
            ids.remove(&id);
            if ids.is_empty() {
                self.pending_by_recipient.remove(&recipient);
            }
        }
        let removed = self.pending_by_context.remove(&pending_key);
        debug_assert_eq!(
            removed,
            Some(id),
            "Derived Data Consistency: pending decision context index disagrees with record"
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
            if let Some(operation) = record.context().operation()
                && !self
                    .by_operation
                    .get(&operation)
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
                    if self.pending_by_context.get(&record.context().pending_key())
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
                        || self.pending_by_context.get(&record.context().pending_key())
                            == Some(&record.id())
                    {
                        return false;
                    }
                }
            }
        }

        for (operation, ids) in &self.by_operation {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.context().operation() == Some(*operation))
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
        for (pending_key, id) in &self.pending_by_context {
            if !self.records.get(id).is_some_and(|record| {
                record.context().pending_key() == *pending_key
                    && record.status() == DecisionStatus::Pending
            }) {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct DecisionRequestDraft {
    pub requester: CharacterId,
    pub context: DecisionContext,
    pub attention: AttentionClass,
    pub summary: String,
}

#[derive(Clone, Debug)]
pub struct RecruitmentApprovalRequestDraft {
    pub authority: MandateAuthority,
    pub target_organization: OrganizationId,
    pub recruiter: CharacterId,
    pub candidate: CharacterId,
    pub approach: RecruitmentApproach,
    pub attention: AttentionClass,
    pub summary: String,
}

pub(crate) fn build_recruitment_approval_context(
    target_organization: OrganizationId,
    recruiter: CharacterId,
    candidate: CharacterId,
    approach: RecruitmentApproach,
    authority: RecruitmentApprovalAuthoritySnapshot,
) -> DecisionContext {
    DecisionContext::RecruitmentApproval(RecruitmentApprovalContext {
        target_organization,
        recruiter,
        candidate,
        approach,
        authority,
    })
}

pub(crate) fn build_recruitment_approval_authority_snapshot(
    authority: MandateAuthority,
    mandate_version: u32,
    manager_version: u32,
    policy_source: RecruitmentPolicySource,
) -> RecruitmentApprovalAuthoritySnapshot {
    RecruitmentApprovalAuthoritySnapshot {
        authority,
        mandate_version,
        manager_version,
        policy_source,
    }
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
