//! Durable typed decision records for authority exceptions and organizational approvals; `decision_system` owns request, resolution, and cancellation transactions.

pub mod decision_system;

use crate::core::attention::AttentionClass;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    CharacterId, DecisionRequestId, MandateId, OperationId, OrganizationId, PoliceResponseId,
};
use crate::core::time::SimTime;
use crate::core::version::advance_version_preflighted;
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
    Cancelled,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionCancellationReason {
    OperationParticipantDetained(CharacterId),
    /// The mandate snapshot behind a recruitment approval was permanently superseded by a
    /// revision or revocation, so the old request can no longer be approved coherently.
    RecruitmentAuthorityChanged(MandateId),
    /// The organization-level recruitment policy that supplied this approval's effective rule
    /// changed, so the pending request no longer represents current delegated authority.
    RecruitmentOrganizationPolicyChanged(OrganizationId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCancellation {
    cancelled_at: SimTime,
    reason: DecisionCancellationReason,
}

impl DecisionCancellation {
    pub fn cancelled_at(self) -> SimTime {
        self.cancelled_at
    }

    pub fn reason(self) -> DecisionCancellationReason {
        self.reason
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum DecisionLifecycle {
    Pending,
    Resolved(DecisionResolution),
    Cancelled(DecisionCancellation),
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
    fn from_resolved(parts: DecisionRecordParts, resolution: DecisionResolution) -> Self {
        let mut record = Self::from(parts);
        record.lifecycle = DecisionLifecycle::Resolved(resolution);
        record.version = 2;
        record
    }

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
            DecisionLifecycle::Cancelled(_) => DecisionStatus::Cancelled,
        }
    }

    pub fn resolution(&self) -> Option<DecisionResolution> {
        match self.lifecycle {
            DecisionLifecycle::Pending | DecisionLifecycle::Cancelled(_) => None,
            DecisionLifecycle::Resolved(resolution) => Some(resolution),
        }
    }

    pub fn cancellation(&self) -> Option<DecisionCancellation> {
        match self.lifecycle {
            DecisionLifecycle::Cancelled(cancellation) => Some(cancellation),
            DecisionLifecycle::Pending | DecisionLifecycle::Resolved(_) => None,
        }
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DecisionState {
    records: BTreeMap<DecisionRequestId, DecisionRequestRecord>,
    #[serde(skip)]
    by_operation: BTreeMap<OperationId, BTreeSet<DecisionRequestId>>,
    #[serde(skip)]
    pending_by_recipient: BTreeMap<OrganizationId, BTreeSet<DecisionRequestId>>,
    #[serde(skip)]
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

    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.by_operation.clear();
        self.pending_by_recipient.clear();
        self.pending_by_context.clear();
        for record in self.records.values() {
            if let Some(operation) = record.context().operation() {
                self.by_operation
                    .entry(operation)
                    .or_default()
                    .insert(record.id());
            }
            if record.status() == DecisionStatus::Pending {
                self.pending_by_recipient
                    .entry(record.recipient())
                    .or_default()
                    .insert(record.id());
                self.pending_by_context
                    .insert(record.context().pending_key(), record.id());
            }
        }
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
            .map(|id| {
                self.records
                    .get(id)
                    .expect("pending-recipient index must reference a decision")
            })
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
            .map(|id| {
                self.records
                    .get(id)
                    .expect("operation-decision index must reference a decision")
            })
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

    fn insert(&mut self, record: DecisionRequestRecord) {
        debug_assert_eq!(record.status(), DecisionStatus::Pending);
        self.insert_record(record, true);
    }

    /// Inserts a decision that was requested and resolved as one atomic autonomous action.
    /// Resolved records are historical evidence only and must never enter pending indexes.
    fn insert_resolved(&mut self, record: DecisionRequestRecord) {
        debug_assert_eq!(record.status(), DecisionStatus::Resolved);
        self.insert_record(record, false);
    }

    fn insert_record(&mut self, record: DecisionRequestRecord, pending: bool) {
        let id = record.id();
        let recipient = record.recipient();
        let operation = record.context().operation();
        if let Some(operation) = operation {
            self.by_operation.entry(operation).or_default().insert(id);
        }
        if pending {
            self.pending_by_recipient
                .entry(recipient)
                .or_default()
                .insert(id);
            let previous_context = self
                .pending_by_context
                .insert(record.context().pending_key(), id);
            debug_assert!(
                previous_context.is_none(),
                "Index Uniqueness: decision context already has a pending decision"
            );
        }
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate decision request ID inserted"
        );
    }

    fn resolve(&mut self, id: DecisionRequestId, resolution: DecisionResolution) {
        self.remove_pending_indexes(id);
        let record = self
            .records
            .get_mut(&id)
            .expect("validated decision disappeared before resolution commit");
        record.lifecycle = DecisionLifecycle::Resolved(resolution);
        record.version = advance_version_preflighted(record.version);
    }

    fn cancel(&mut self, id: DecisionRequestId, cancellation: DecisionCancellation) {
        self.remove_pending_indexes(id);
        let record = self
            .records
            .get_mut(&id)
            .expect("validated decision disappeared before cancellation commit");
        record.lifecycle = DecisionLifecycle::Cancelled(cancellation);
        record.version = advance_version_preflighted(record.version);
    }

    fn remove_pending_indexes(&mut self, id: DecisionRequestId) {
        let (recipient, pending_key) = {
            let record = self
                .records
                .get(&id)
                .expect("validated decision disappeared before pending-index removal");
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
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        self.records_have_consistent_indexes()
            && self.operation_index_is_consistent()
            && self.pending_recipient_index_is_consistent()
            && self.pending_context_index_is_consistent()
    }

    /// Every decision must occupy the operation and pending projections implied by its record.
    fn records_have_consistent_indexes(&self) -> bool {
        for (stored_id, record) in &self.records {
            if *stored_id != record.id() {
                return false;
            }
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
                DecisionStatus::Resolved | DecisionStatus::Cancelled => {
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
        true
    }

    /// Reverse operation entries must point only to decisions about that operation.
    fn operation_index_is_consistent(&self) -> bool {
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
        true
    }

    /// Pending recipient entries contain pending decisions for that recipient only.
    fn pending_recipient_index_is_consistent(&self) -> bool {
        for (recipient, ids) in &self.pending_by_recipient {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.recipient() == *recipient && record.status() == DecisionStatus::Pending
                }) {
                    return false;
                }
            }
        }
        true
    }

    /// The pending-context key is exclusive and must resolve to the exact pending decision.
    fn pending_context_index_is_consistent(&self) -> bool {
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

fn build_cancellation(
    cancelled_at: SimTime,
    reason: DecisionCancellationReason,
) -> DecisionCancellation {
    DecisionCancellation {
        cancelled_at,
        reason,
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

fn build_recruitment_approval_context(
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

fn build_recruitment_approval_authority_snapshot(
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

struct DecisionRecordParts {
    id: DecisionRequestId,
    recipient: OrganizationId,
    draft: DecisionRequestDraft,
    requested_at: SimTime,
    options: BTreeSet<DecisionResponse>,
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

fn build_resolution(
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
