//! Prosecution case/referral records, lifecycle artifacts, drafts, and live indexes.

use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, InformationId, InvestigationId, OrganizationId,
    ProsecutionCaseId, ProsecutionReferralId, ReportId,
};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProsecutionCaseStatus {
    Reviewing,
    Declined,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProsecutionCaseResolution {
    Declined,
    Closed,
}

impl ProsecutionCaseResolution {
    pub(in crate::legal) fn status(self) -> ProsecutionCaseStatus {
        match self {
            Self::Declined => ProsecutionCaseStatus::Declined,
            Self::Closed => ProsecutionCaseStatus::Closed,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseContext {
    pub(in crate::legal) arrest: ArrestId,
    pub(in crate::legal) defendant: CharacterId,
    pub(in crate::legal) source_investigation: InvestigationId,
    pub(in crate::legal) source_authority: OrganizationId,
    pub(in crate::legal) prosecutor_office: OrganizationId,
    /// Current reviewing attorney. Historical actor attribution lives on each referral and
    /// terminal resolution artifact, so replacing this assignment never rewrites history.
    pub(in crate::legal) assigned_prosecutor: Option<CharacterId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseReferrals {
    pub(in crate::legal) evidence: BTreeSet<EvidenceId>,
    pub(in crate::legal) initial_referral: ProsecutionReferralId,
    pub(in crate::legal) referrals: BTreeSet<ProsecutionReferralId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseLifecycle {
    pub(in crate::legal) opened_at: SimTime,
    pub(in crate::legal) resolved_at: Option<SimTime>,
    pub(in crate::legal) status: ProsecutionCaseStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseResolutionArtifacts {
    pub(in crate::legal) resolution_information: Option<InformationId>,
    pub(in crate::legal) resolution_report: Option<ReportId>,
    pub(in crate::legal) resolution_prosecutor: Option<CharacterId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProsecutionCaseRecord {
    pub(in crate::legal) id: ProsecutionCaseId,
    pub(in crate::legal) context: ProsecutionCaseContext,
    pub(in crate::legal) referrals: ProsecutionCaseReferrals,
    pub(in crate::legal) lifecycle: ProsecutionCaseLifecycle,
    pub(in crate::legal) resolution_artifacts: ProsecutionCaseResolutionArtifacts,
    pub(in crate::legal) version: u32,
}

impl ProsecutionCaseRecord {
    pub fn id(&self) -> ProsecutionCaseId {
        self.id
    }
    pub fn arrest(&self) -> ArrestId {
        self.context.arrest
    }
    pub fn defendant(&self) -> CharacterId {
        self.context.defendant
    }
    pub fn source_investigation(&self) -> InvestigationId {
        self.context.source_investigation
    }
    pub fn source_authority(&self) -> OrganizationId {
        self.context.source_authority
    }
    pub fn prosecutor_office(&self) -> OrganizationId {
        self.context.prosecutor_office
    }
    pub fn assigned_prosecutor(&self) -> Option<CharacterId> {
        self.context.assigned_prosecutor
    }
    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.referrals.evidence
    }
    pub fn initial_referral(&self) -> ProsecutionReferralId {
        self.referrals.initial_referral
    }
    pub fn referrals(&self) -> &BTreeSet<ProsecutionReferralId> {
        &self.referrals.referrals
    }
    pub fn opened_at(&self) -> SimTime {
        self.lifecycle.opened_at
    }
    pub fn resolved_at(&self) -> Option<SimTime> {
        self.lifecycle.resolved_at
    }
    pub fn status(&self) -> ProsecutionCaseStatus {
        self.lifecycle.status
    }
    pub fn resolution_information(&self) -> Option<InformationId> {
        self.resolution_artifacts.resolution_information
    }
    pub fn resolution_report(&self) -> Option<ReportId> {
        self.resolution_artifacts.resolution_report
    }
    pub fn resolution_prosecutor(&self) -> Option<CharacterId> {
        self.resolution_artifacts.resolution_prosecutor
    }
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProsecutionReferralRecord {
    pub(in crate::legal) id: ProsecutionReferralId,
    pub(in crate::legal) prosecution_case: ProsecutionCaseId,
    pub(in crate::legal) source_investigation: InvestigationId,
    pub(in crate::legal) source_authority: OrganizationId,
    pub(in crate::legal) prosecutor_office: OrganizationId,
    pub(in crate::legal) prosecutor: CharacterId,
    pub(in crate::legal) evidence: BTreeSet<EvidenceId>,
    pub(in crate::legal) referred_at: SimTime,
    pub(in crate::legal) information: InformationId,
    pub(in crate::legal) report: ReportId,
}

impl ProsecutionReferralRecord {
    pub fn id(&self) -> ProsecutionReferralId {
        self.id
    }
    pub fn prosecution_case(&self) -> ProsecutionCaseId {
        self.prosecution_case
    }
    pub fn source_investigation(&self) -> InvestigationId {
        self.source_investigation
    }
    pub fn source_authority(&self) -> OrganizationId {
        self.source_authority
    }
    pub fn prosecutor_office(&self) -> OrganizationId {
        self.prosecutor_office
    }
    pub fn prosecutor(&self) -> CharacterId {
        self.prosecutor
    }
    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }
    pub fn referred_at(&self) -> SimTime {
        self.referred_at
    }
    pub fn information(&self) -> InformationId {
        self.information
    }
    pub fn report(&self) -> ReportId {
        self.report
    }
}

#[derive(Clone, Debug)]
pub struct ProsecutionCaseDraft {
    pub arrest: ArrestId,
    pub prosecutor_office: OrganizationId,
    pub prosecutor: CharacterId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Debug)]
pub struct ProsecutionReferralDraft {
    pub prosecution_case: ProsecutionCaseId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct ProsecutionIndexes {
    /// Reviewing cases currently assigned to each prosecutor. Terminal case history is not
    /// retained here because actor attribution is stored on referral/resolution records.
    pub(in crate::legal) reviewing_cases_by_prosecutor:
        BTreeMap<CharacterId, BTreeSet<ProsecutionCaseId>>,
    pub(in crate::legal) reviewing_without_prosecutor: BTreeSet<ProsecutionCaseId>,
    pub(in crate::legal) open_by_arrest_office:
        BTreeMap<(ArrestId, OrganizationId), ProsecutionCaseId>,
    pub(in crate::legal) referrals_by_case:
        BTreeMap<ProsecutionCaseId, BTreeSet<ProsecutionReferralId>>,
}
