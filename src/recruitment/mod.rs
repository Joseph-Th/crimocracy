//! Relational recruitment records and indexes; `recruitment_system` owns candidate discovery, decisions, and membership changes.

pub mod recruitment_system;

use crate::core::id::{
    CharacterId, DecisionRequestId, HistoryEventId, InformationId, MandateId, OrganizationId,
    RecruitmentAttemptId,
};
use crate::core::time::SimTime;
use crate::delegation::ResponsibilityScope;
use crate::social::RelationshipDimensions;
use crate::world::ApprovalPolicy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RecruitmentApproach {
    FinancialOpportunity,
    Advancement,
    Protection,
    PersonalAppeal,
}

pub const ALL_RECRUITMENT_APPROACHES: [RecruitmentApproach; 4] = [
    RecruitmentApproach::FinancialOpportunity,
    RecruitmentApproach::Advancement,
    RecruitmentApproach::Protection,
    RecruitmentApproach::PersonalAppeal,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentRelationshipSnapshot {
    from: CharacterId,
    to: CharacterId,
    dimensions: Option<RelationshipDimensions>,
    version: Option<u32>,
}

impl RecruitmentRelationshipSnapshot {
    pub fn from(self) -> CharacterId {
        self.from
    }

    pub fn to(self) -> CharacterId {
        self.to
    }

    pub fn dimensions(self) -> Option<RelationshipDimensions> {
        self.dimensions
    }

    pub fn version(self) -> Option<u32> {
        self.version
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RecruitmentOutcome {
    Accepted,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecruitmentPolicySource {
    Organization(OrganizationId),
    Mandate(MandateId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecruitmentAuthority {
    ExecutiveApproval,
    ApprovedDecision {
        decision: DecisionRequestId,
        mandate: MandateId,
        manager: CharacterId,
        scope: ResponsibilityScope,
        mandate_version: u32,
        manager_version: u32,
        policy: ApprovalPolicy,
        policy_source: RecruitmentPolicySource,
    },
    Delegated {
        mandate: MandateId,
        manager: CharacterId,
        scope: ResponsibilityScope,
        mandate_version: u32,
        manager_version: u32,
        policy: ApprovalPolicy,
        policy_source: RecruitmentPolicySource,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentFactors {
    recruiter_influence: u8,
    drive_alignment: u8,
    relationship_support: u8,
    incumbent_attachment: u8,
    incumbent_resentment: u8,
    perceived_legal_pressure: u8,
    membership_resistance: u8,
    trait_adjustment: i16,
}

impl RecruitmentFactors {
    pub fn recruiter_influence(self) -> u8 {
        self.recruiter_influence
    }

    pub fn drive_alignment(self) -> u8 {
        self.drive_alignment
    }

    pub fn relationship_support(self) -> u8 {
        self.relationship_support
    }

    pub fn incumbent_attachment(self) -> u8 {
        self.incumbent_attachment
    }

    pub fn incumbent_resentment(self) -> u8 {
        self.incumbent_resentment
    }

    pub fn perceived_legal_pressure(self) -> u8 {
        self.perceived_legal_pressure
    }

    pub fn membership_resistance(self) -> u8 {
        self.membership_resistance
    }

    pub fn trait_adjustment(self) -> i16 {
        self.trait_adjustment
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecruitmentIdentity {
    id: RecruitmentAttemptId,
    recruiter: CharacterId,
    candidate: CharacterId,
    target_organization: OrganizationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecruitmentContext {
    approach: RecruitmentApproach,
    authority: RecruitmentAuthority,
    recruiter_relationship: RecruitmentRelationshipSnapshot,
    incumbent_relationship: Option<RecruitmentRelationshipSnapshot>,
    previous_organization: Option<OrganizationId>,
    previous_supervisor: Option<CharacterId>,
    pressure_information: Option<InformationId>,
    occurred_at: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecruitmentResolution {
    factors: RecruitmentFactors,
    margin: i16,
    outcome: RecruitmentOutcome,
    outcome_information: InformationId,
    history_event: Option<HistoryEventId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecruitmentAttemptRecord {
    identity: RecruitmentIdentity,
    context: RecruitmentContext,
    resolution: RecruitmentResolution,
}

impl RecruitmentAttemptRecord {
    pub fn id(&self) -> RecruitmentAttemptId {
        self.identity.id
    }

    pub fn recruiter(&self) -> CharacterId {
        self.identity.recruiter
    }

    pub fn candidate(&self) -> CharacterId {
        self.identity.candidate
    }

    pub fn target_organization(&self) -> OrganizationId {
        self.identity.target_organization
    }

    pub fn approach(&self) -> RecruitmentApproach {
        self.context.approach
    }

    pub fn authority(&self) -> RecruitmentAuthority {
        self.context.authority
    }

    pub fn recruiter_relationship(&self) -> RecruitmentRelationshipSnapshot {
        self.context.recruiter_relationship
    }

    pub fn incumbent_relationship(&self) -> Option<RecruitmentRelationshipSnapshot> {
        self.context.incumbent_relationship
    }

    pub fn previous_organization(&self) -> Option<OrganizationId> {
        self.context.previous_organization
    }

    pub fn previous_supervisor(&self) -> Option<CharacterId> {
        self.context.previous_supervisor
    }

    pub fn pressure_information(&self) -> Option<InformationId> {
        self.context.pressure_information
    }

    pub fn occurred_at(&self) -> SimTime {
        self.context.occurred_at
    }

    pub fn factors(&self) -> RecruitmentFactors {
        self.resolution.factors
    }

    pub fn margin(&self) -> i16 {
        self.resolution.margin
    }

    pub fn outcome(&self) -> RecruitmentOutcome {
        self.resolution.outcome
    }

    pub fn outcome_information(&self) -> InformationId {
        self.resolution.outcome_information
    }

    pub fn history_event(&self) -> Option<HistoryEventId> {
        self.resolution.history_event
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecruitmentState {
    records: BTreeMap<RecruitmentAttemptId, RecruitmentAttemptRecord>,
    by_candidate: BTreeMap<CharacterId, BTreeSet<RecruitmentAttemptId>>,
    by_candidate_organization:
        BTreeMap<(CharacterId, OrganizationId), BTreeSet<RecruitmentAttemptId>>,
    by_approval_decision: BTreeMap<DecisionRequestId, RecruitmentAttemptId>,
}

impl RecruitmentState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_attempt(&self, id: RecruitmentAttemptId) -> Option<&RecruitmentAttemptRecord> {
        self.records.get(&id)
    }

    pub fn attempts_for_candidate(
        &self,
        candidate: CharacterId,
    ) -> impl Iterator<Item = &RecruitmentAttemptRecord> {
        self.by_candidate
            .get(&candidate)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn attempts_for_candidate_organization(
        &self,
        candidate: CharacterId,
        organization: OrganizationId,
    ) -> impl Iterator<Item = &RecruitmentAttemptRecord> {
        self.by_candidate_organization
            .get(&(candidate, organization))
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }

    pub fn latest_attempt_for(
        &self,
        candidate: CharacterId,
        organization: OrganizationId,
    ) -> Option<&RecruitmentAttemptRecord> {
        self.by_candidate_organization
            .get(&(candidate, organization))
            .and_then(|ids| ids.last())
            .and_then(|id| self.records.get(id))
    }

    pub fn get_attempt_for_approval_decision(
        &self,
        decision: DecisionRequestId,
    ) -> Option<&RecruitmentAttemptRecord> {
        self.by_approval_decision
            .get(&decision)
            .and_then(|id| self.records.get(id))
    }

    pub(crate) fn attempts(&self) -> impl Iterator<Item = &RecruitmentAttemptRecord> {
        self.records.values()
    }

    pub(crate) fn insert(&mut self, record: RecruitmentAttemptRecord) {
        let id = record.id();
        self.by_candidate
            .entry(record.candidate())
            .or_default()
            .insert(id);
        self.by_candidate_organization
            .entry((record.candidate(), record.target_organization()))
            .or_default()
            .insert(id);
        match record.authority() {
            RecruitmentAuthority::ApprovedDecision { decision, .. } => {
                let previous = self.by_approval_decision.insert(decision, id);
                debug_assert!(
                    previous.is_none(),
                    "Index Uniqueness: approval decision already has a recruitment attempt"
                );
            }
            RecruitmentAuthority::ExecutiveApproval | RecruitmentAuthority::Delegated { .. } => {}
        }
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate recruitment attempt ID inserted"
        );
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            let id = record.id();
            if !self
                .by_candidate
                .get(&record.candidate())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .by_candidate_organization
                    .get(&(record.candidate(), record.target_organization()))
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            match record.authority() {
                RecruitmentAuthority::ApprovedDecision { decision, .. } => {
                    if self.by_approval_decision.get(&decision) != Some(&id) {
                        return false;
                    }
                }
                RecruitmentAuthority::ExecutiveApproval
                | RecruitmentAuthority::Delegated { .. } => {}
            }
        }
        for (candidate, ids) in &self.by_candidate {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.candidate() == *candidate)
                {
                    return false;
                }
            }
        }
        for (key, ids) in &self.by_candidate_organization {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    (record.candidate(), record.target_organization()) == *key
                }) {
                    return false;
                }
            }
        }
        for (decision, id) in &self.by_approval_decision {
            if !self.records.get(id).is_some_and(|record| {
                matches!(
                    record.authority(),
                    RecruitmentAuthority::ApprovedDecision {
                        decision: record_decision,
                        ..
                    } if record_decision == *decision
                )
            }) {
                return false;
            }
        }
        true
    }

    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: recruitment indexes disagree with source records"
        );
        for record in self.records.values() {
            debug_assert!(
                self.by_candidate
                    .get(&record.candidate())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: recruitment candidate index is missing an attempt"
            );
            debug_assert!(
                self.by_candidate_organization
                    .get(&(record.candidate(), record.target_organization()))
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: recruitment candidate-organization index is missing an attempt"
            );
            if let RecruitmentAuthority::ApprovedDecision { decision, .. } = record.authority() {
                debug_assert_eq!(
                    self.by_approval_decision.get(&decision),
                    Some(&record.id()),
                    "Index Completeness: recruitment approval index is missing an attempt"
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecruitmentDraft {
    pub target_organization: OrganizationId,
    pub recruiter: CharacterId,
    pub candidate: CharacterId,
    pub approach: RecruitmentApproach,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecruitmentRecordParts {
    pub id: RecruitmentAttemptId,
    pub draft: RecruitmentDraft,
    pub context: RecruitmentRecordContextParts,
    pub resolution: RecruitmentRecordResolutionParts,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecruitmentRecordContextParts {
    pub authority: RecruitmentAuthority,
    pub recruiter_relationship: RecruitmentRelationshipSnapshot,
    pub incumbent_relationship: Option<RecruitmentRelationshipSnapshot>,
    pub previous_organization: Option<OrganizationId>,
    pub previous_supervisor: Option<CharacterId>,
    pub pressure_information: Option<InformationId>,
    pub occurred_at: SimTime,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecruitmentRecordResolutionParts {
    pub factors: RecruitmentFactors,
    pub margin: i16,
    pub outcome: RecruitmentOutcome,
    pub outcome_information: InformationId,
    pub history_event: Option<HistoryEventId>,
}

pub(crate) fn build_recruitment_record(parts: RecruitmentRecordParts) -> RecruitmentAttemptRecord {
    let RecruitmentRecordParts {
        id,
        draft,
        context,
        resolution,
    } = parts;
    let RecruitmentRecordContextParts {
        authority,
        recruiter_relationship,
        incumbent_relationship,
        previous_organization,
        previous_supervisor,
        pressure_information,
        occurred_at,
    } = context;
    let RecruitmentRecordResolutionParts {
        factors,
        margin,
        outcome,
        outcome_information,
        history_event,
    } = resolution;
    RecruitmentAttemptRecord {
        identity: RecruitmentIdentity {
            id,
            recruiter: draft.recruiter,
            candidate: draft.candidate,
            target_organization: draft.target_organization,
        },
        context: RecruitmentContext {
            approach: draft.approach,
            authority,
            recruiter_relationship,
            incumbent_relationship,
            previous_organization,
            previous_supervisor,
            pressure_information,
            occurred_at,
        },
        resolution: RecruitmentResolution {
            factors,
            margin,
            outcome,
            outcome_information,
            history_event,
        },
    }
}

pub(crate) fn build_recruitment_relationship_snapshot(
    from: CharacterId,
    to: CharacterId,
    dimensions: Option<RelationshipDimensions>,
    version: Option<u32>,
) -> RecruitmentRelationshipSnapshot {
    RecruitmentRelationshipSnapshot {
        from,
        to,
        dimensions,
        version,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecruitmentFactorComponents {
    pub recruiter_influence: u8,
    pub drive_alignment: u8,
    pub relationship_support: u8,
    pub incumbent_attachment: u8,
    pub incumbent_resentment: u8,
    pub perceived_legal_pressure: u8,
    pub membership_resistance: u8,
    pub trait_adjustment: i16,
}

pub(crate) fn build_recruitment_factors(
    components: RecruitmentFactorComponents,
) -> RecruitmentFactors {
    let RecruitmentFactorComponents {
        recruiter_influence,
        drive_alignment,
        relationship_support,
        incumbent_attachment,
        incumbent_resentment,
        perceived_legal_pressure,
        membership_resistance,
        trait_adjustment,
    } = components;
    RecruitmentFactors {
        recruiter_influence,
        drive_alignment,
        relationship_support,
        incumbent_attachment,
        incumbent_resentment,
        perceived_legal_pressure,
        membership_resistance,
        trait_adjustment,
    }
}
