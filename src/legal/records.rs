//! Legal records: identity, lifecycle state, drafts, and derived index structures.
//!
//! Records and indexes are owned by [`LegalState`](crate::legal::LegalState), whose system
//! methods synchronize every backing collection atomically. This file holds the record
//! definitions and index structs only; the state owner and its mutation/observation
//! methods live in `legal_state.rs`. The `pub(super)` field visibility keeps every
//! mutation inside the `legal` module tree while exposing nothing outside it.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, ContactId, EvidenceId, FinancialAccountId,
    InformantDisclosureId, InformantId, InformationId, InvestigationId, InvestigationWorkId,
    LedgerTransactionId, LegalRepresentationId, NeighborhoodId, OperationId, OrganizationId,
    PatrolDeploymentId, PoliceResponseId, ProsecutionCaseId, ProsecutionReferralId, ReportId,
    WitnessStatementId,
};
use crate::core::time::SimTime;
use crate::delegation::MandateAuthority;
use crate::finance::Money;
use crate::world::Rating;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationStatus {
    Active,
    Suspended,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArrestStatus {
    Detained,
    Released,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArrestRecord {
    pub(super) id: ArrestId,
    pub(super) character: CharacterId,
    pub(super) authority: OrganizationId,
    pub(super) investigation: InvestigationId,
    pub(super) evidence: BTreeSet<EvidenceId>,
    pub(super) arrested_at: SimTime,
    pub(super) released_at: Option<SimTime>,
    pub(super) status: ArrestStatus,
    pub(super) version: u32,
}

impl ArrestRecord {
    pub fn id(&self) -> ArrestId {
        self.id
    }

    pub fn character(&self) -> CharacterId {
        self.character
    }

    pub fn authority(&self) -> OrganizationId {
        self.authority
    }

    pub fn investigation(&self) -> InvestigationId {
        self.investigation
    }

    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }

    pub fn arrested_at(&self) -> SimTime {
        self.arrested_at
    }

    pub fn released_at(&self) -> Option<SimTime> {
        self.released_at
    }

    pub fn status(&self) -> ArrestStatus {
        self.status
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug)]
pub struct ArrestDraft {
    pub character: CharacterId,
    pub investigation: InvestigationId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalRepresentationStatus {
    Active,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalRepresentationEndReason {
    MatterConcluded,
    Replaced,
    SponsorWithdrawn,
    CounselWithdrawn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationParties {
    pub(super) arrest: ArrestId,
    pub(super) defendant: CharacterId,
    pub(super) sponsor: OrganizationId,
    pub(super) counsel: CharacterId,
    pub(super) counsel_institution: OrganizationId,
    pub(super) contact: ContactId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationPayment {
    pub(super) fee: Money,
    pub(super) payer_account: FinancialAccountId,
    pub(super) provider_account: FinancialAccountId,
    pub(super) payment: LedgerTransactionId,
    pub(super) authorization: Option<MandateAuthority>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationLifecycle {
    pub(super) retained_at: SimTime,
    pub(super) ended_at: Option<SimTime>,
    pub(super) end_reason: Option<LegalRepresentationEndReason>,
    pub(super) status: LegalRepresentationStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LegalRepresentationArtifacts {
    pub(super) information: InformationId,
    pub(super) report: ReportId,
    pub(super) ended_information: Option<InformationId>,
    pub(super) ended_report: Option<ReportId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegalRepresentationRecord {
    pub(super) id: LegalRepresentationId,
    pub(super) parties: LegalRepresentationParties,
    pub(super) payment: LegalRepresentationPayment,
    pub(super) lifecycle: LegalRepresentationLifecycle,
    pub(super) artifacts: LegalRepresentationArtifacts,
    pub(super) version: u32,
}

impl LegalRepresentationRecord {
    pub fn id(&self) -> LegalRepresentationId {
        self.id
    }

    pub fn arrest(&self) -> ArrestId {
        self.parties.arrest
    }

    pub fn defendant(&self) -> CharacterId {
        self.parties.defendant
    }

    pub fn sponsor(&self) -> OrganizationId {
        self.parties.sponsor
    }

    pub fn counsel(&self) -> CharacterId {
        self.parties.counsel
    }

    pub fn counsel_institution(&self) -> OrganizationId {
        self.parties.counsel_institution
    }

    pub fn contact(&self) -> ContactId {
        self.parties.contact
    }

    pub fn fee(&self) -> Money {
        self.payment.fee
    }

    pub fn payer_account(&self) -> FinancialAccountId {
        self.payment.payer_account
    }

    pub fn provider_account(&self) -> FinancialAccountId {
        self.payment.provider_account
    }

    pub fn payment(&self) -> LedgerTransactionId {
        self.payment.payment
    }

    pub fn authorization(&self) -> Option<MandateAuthority> {
        self.payment.authorization
    }

    pub fn retained_at(&self) -> SimTime {
        self.lifecycle.retained_at
    }

    pub fn ended_at(&self) -> Option<SimTime> {
        self.lifecycle.ended_at
    }

    pub fn end_reason(&self) -> Option<LegalRepresentationEndReason> {
        self.lifecycle.end_reason
    }

    pub fn status(&self) -> LegalRepresentationStatus {
        self.lifecycle.status
    }

    pub fn information(&self) -> InformationId {
        self.artifacts.information
    }

    pub fn report(&self) -> ReportId {
        self.artifacts.report
    }

    pub fn ended_information(&self) -> Option<InformationId> {
        self.artifacts.ended_information
    }

    pub fn ended_report(&self) -> Option<ReportId> {
        self.artifacts.ended_report
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug)]
pub struct LegalRepresentationDraft {
    pub arrest: ArrestId,
    pub sponsor: OrganizationId,
    pub contact: ContactId,
    pub fee: Money,
    pub payer_account: FinancialAccountId,
    pub provider_account: FinancialAccountId,
    pub authorization: Option<MandateAuthority>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct LegalRepresentationIndexes {
    pub(super) by_arrest: BTreeMap<ArrestId, BTreeSet<LegalRepresentationId>>,
    pub(super) by_defendant: BTreeMap<CharacterId, BTreeSet<LegalRepresentationId>>,
    pub(super) by_sponsor: BTreeMap<OrganizationId, BTreeSet<LegalRepresentationId>>,
    pub(super) by_counsel: BTreeMap<CharacterId, BTreeSet<LegalRepresentationId>>,
    pub(super) by_contact: BTreeMap<ContactId, BTreeSet<LegalRepresentationId>>,
    pub(super) active_by_arrest: BTreeMap<ArrestId, LegalRepresentationId>,
    pub(super) active_by_contact: BTreeMap<ContactId, BTreeSet<LegalRepresentationId>>,
}

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
    pub(super) fn status(self) -> ProsecutionCaseStatus {
        match self {
            Self::Declined => ProsecutionCaseStatus::Declined,
            Self::Closed => ProsecutionCaseStatus::Closed,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseContext {
    pub(super) arrest: ArrestId,
    pub(super) defendant: CharacterId,
    pub(super) source_investigation: InvestigationId,
    pub(super) source_authority: OrganizationId,
    pub(super) prosecutor_office: OrganizationId,
    pub(super) lead_prosecutor: CharacterId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseReferrals {
    pub(super) evidence: BTreeSet<EvidenceId>,
    pub(super) initial_referral: ProsecutionReferralId,
    pub(super) referrals: BTreeSet<ProsecutionReferralId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseLifecycle {
    pub(super) opened_at: SimTime,
    pub(super) resolved_at: Option<SimTime>,
    pub(super) status: ProsecutionCaseStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProsecutionCaseResolutionArtifacts {
    pub(super) resolution_information: Option<InformationId>,
    pub(super) resolution_report: Option<ReportId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProsecutionCaseRecord {
    pub(super) id: ProsecutionCaseId,
    pub(super) context: ProsecutionCaseContext,
    pub(super) referrals: ProsecutionCaseReferrals,
    pub(super) lifecycle: ProsecutionCaseLifecycle,
    pub(super) resolution_artifacts: ProsecutionCaseResolutionArtifacts,
    pub(super) version: u32,
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
    pub fn lead_prosecutor(&self) -> CharacterId {
        self.context.lead_prosecutor
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
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProsecutionReferralRecord {
    pub(super) id: ProsecutionReferralId,
    pub(super) prosecution_case: ProsecutionCaseId,
    pub(super) source_investigation: InvestigationId,
    pub(super) source_authority: OrganizationId,
    pub(super) prosecutor_office: OrganizationId,
    pub(super) evidence: BTreeSet<EvidenceId>,
    pub(super) referred_at: SimTime,
    pub(super) information: InformationId,
    pub(super) report: ReportId,
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
    pub lead_prosecutor: CharacterId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Debug)]
pub struct ProsecutionReferralDraft {
    pub prosecution_case: ProsecutionCaseId,
    pub evidence: BTreeSet<EvidenceId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct ProsecutionIndexes {
    pub(super) cases_by_arrest: BTreeMap<ArrestId, BTreeSet<ProsecutionCaseId>>,
    pub(super) cases_by_source_investigation:
        BTreeMap<InvestigationId, BTreeSet<ProsecutionCaseId>>,
    pub(super) cases_by_lead: BTreeMap<CharacterId, BTreeSet<ProsecutionCaseId>>,
    pub(super) cases_by_evidence: BTreeMap<EvidenceId, BTreeSet<ProsecutionCaseId>>,
    pub(super) open_by_arrest_office: BTreeMap<(ArrestId, OrganizationId), ProsecutionCaseId>,
    pub(super) referrals_by_case: BTreeMap<ProsecutionCaseId, BTreeSet<ProsecutionReferralId>>,
    pub(super) referrals_by_evidence: BTreeMap<EvidenceId, BTreeSet<ProsecutionReferralId>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigatorRole {
    Lead,
    Investigator,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct ArrestIndexes {
    pub(super) by_character: BTreeMap<CharacterId, BTreeSet<ArrestId>>,
    pub(super) by_investigation: BTreeMap<InvestigationId, BTreeSet<ArrestId>>,
    pub(super) active_by_character: BTreeMap<CharacterId, ArrestId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigationWorkKind {
    PatternAnalysis,
    EvidenceReview,
    WitnessInterview,
}

pub const ALL_INVESTIGATION_WORK_KINDS: [InvestigationWorkKind; 3] = [
    InvestigationWorkKind::PatternAnalysis,
    InvestigationWorkKind::EvidenceReview,
    InvestigationWorkKind::WitnessInterview,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigationWorkFocus {
    EntityConnection { from: EntityRef, to: EntityRef },
    Evidence(EvidenceId),
    Witness(CaseWitnessId),
}

impl InvestigationWorkFocus {
    pub fn new(from: EntityRef, to: EntityRef) -> Self {
        if from <= to {
            Self::EntityConnection { from, to }
        } else {
            Self::EntityConnection { from: to, to: from }
        }
    }

    pub fn evidence(evidence: EvidenceId) -> Self {
        Self::Evidence(evidence)
    }

    pub fn witness(case_witness: CaseWitnessId) -> Self {
        Self::Witness(case_witness)
    }

    pub fn from(self) -> EntityRef {
        match self {
            Self::EntityConnection { from, .. } => from,
            Self::Evidence(evidence) => EntityRef::Evidence(evidence),
            // Connection endpoints are meaningless for an interview focus; every caller
            // guards on work kind before touching them.
            Self::Witness(_) => unreachable!("witness focus has no connection endpoints"),
        }
    }

    pub fn to(self) -> EntityRef {
        match self {
            Self::EntityConnection { to, .. } => to,
            Self::Evidence(evidence) => EntityRef::Evidence(evidence),
            Self::Witness(_) => unreachable!("witness focus has no connection endpoints"),
        }
    }

    pub fn evidence_id(self) -> Option<EvidenceId> {
        match self {
            Self::Evidence(evidence) => Some(evidence),
            Self::EntityConnection { .. } | Self::Witness(_) => None,
        }
    }

    pub fn witness_id(self) -> Option<CaseWitnessId> {
        match self {
            Self::Witness(case_witness) => Some(case_witness),
            Self::EntityConnection { .. } | Self::Evidence(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationWorkStatus {
    Scheduled,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationWorkOutcome {
    Connected,
    Developed,
    Inconclusive,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationWorkFactors {
    pub(super) investigation_capability: Rating,
    pub(super) source_support: Rating,
    pub(super) source_evidence_count: u8,
    pub(super) difficulty: u8,
    pub(super) variance: i8,
}

impl InvestigationWorkFactors {
    pub fn investigation_capability(self) -> Rating {
        self.investigation_capability
    }

    pub fn source_support(self) -> Rating {
        self.source_support
    }

    pub fn source_evidence_count(self) -> u8 {
        self.source_evidence_count
    }

    pub fn difficulty(self) -> u8 {
        self.difficulty
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationWorkResolution {
    pub(super) resolved_at: SimTime,
    pub(super) outcome: InvestigationWorkOutcome,
    pub(super) factors: InvestigationWorkFactors,
    pub(super) margin: i16,
    pub(super) superseded_by: Option<EvidenceId>,
    pub(super) derived_evidence: Option<EvidenceId>,
}

impl InvestigationWorkResolution {
    pub fn resolved_at(&self) -> SimTime {
        self.resolved_at
    }

    pub fn outcome(&self) -> InvestigationWorkOutcome {
        self.outcome
    }

    pub fn factors(&self) -> InvestigationWorkFactors {
        self.factors
    }

    pub fn margin(&self) -> i16 {
        self.margin
    }

    pub fn superseded_by(&self) -> Option<EvidenceId> {
        self.superseded_by
    }

    pub fn derived_evidence(&self) -> Option<EvidenceId> {
        self.derived_evidence
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkIdentity {
    pub(super) id: InvestigationWorkId,
    pub(super) investigation: InvestigationId,
    pub(super) investigator: CharacterId,
    pub(super) kind: InvestigationWorkKind,
    pub(super) focus: InvestigationWorkFocus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkRuntime {
    pub(super) scheduled_at: SimTime,
    pub(super) due_at: SimTime,
    pub(super) status: InvestigationWorkStatus,
    pub(super) resolution: Option<InvestigationWorkResolution>,
    pub(super) version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkRecord {
    pub(super) identity: InvestigationWorkIdentity,
    pub(super) source_evidence: BTreeSet<EvidenceId>,
    pub(super) runtime: InvestigationWorkRuntime,
}

#[derive(Clone, Copy, Debug)]
pub struct InvestigationWorkDraft {
    pub investigation: InvestigationId,
    pub investigator: CharacterId,
    pub kind: InvestigationWorkKind,
    pub focus: InvestigationWorkFocus,
}

impl InvestigationWorkRecord {
    pub fn id(&self) -> InvestigationWorkId {
        self.identity.id
    }

    pub fn investigation(&self) -> InvestigationId {
        self.identity.investigation
    }

    pub fn investigator(&self) -> CharacterId {
        self.identity.investigator
    }

    pub fn kind(&self) -> InvestigationWorkKind {
        self.identity.kind
    }

    pub fn focus(&self) -> InvestigationWorkFocus {
        self.identity.focus
    }

    pub fn source_evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.source_evidence
    }

    pub fn scheduled_at(&self) -> SimTime {
        self.runtime.scheduled_at
    }

    pub fn due_at(&self) -> SimTime {
        self.runtime.due_at
    }

    pub fn status(&self) -> InvestigationWorkStatus {
        self.runtime.status
    }

    pub fn resolution(&self) -> Option<&InvestigationWorkResolution> {
        self.runtime.resolution.as_ref()
    }

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    WitnessTestimony,
    VehicleDescription,
    Fingerprint,
    RecoveredProperty,
    FinancialRecord,
    InformantStatement,
    Surveillance,
    CommunicationRecord,
    KnownAssociation,
    Document,
    Ballistics,
    PatternLink,
    ForensicAnalysis,
}

/// Ordered weakest to strongest so assessments can be compared against gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceStrength {
    Weak,
    Corroborating,
    Strong,
    Direct,
}

/// Ordered least to most reliable so assessments can be compared against gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceReliability {
    Questionable,
    Mixed,
    Credible,
    HighlyReliable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Admissibility {
    Unknown,
    Inadmissible,
    Disputed,
    Admissible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WitnessCooperation {
    Hostile,
    Reluctant,
    Cooperative,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseWitnessRecord {
    pub(super) id: CaseWitnessId,
    pub(super) investigation: InvestigationId,
    pub(super) witness: CharacterId,
    pub(super) cooperation: WitnessCooperation,
    pub(super) registered_at: SimTime,
    pub(super) statements: BTreeSet<WitnessStatementId>,
    pub(super) version: u32,
}

impl CaseWitnessRecord {
    pub fn id(&self) -> CaseWitnessId {
        self.id
    }

    pub fn investigation(&self) -> InvestigationId {
        self.investigation
    }

    pub fn witness(&self) -> CharacterId {
        self.witness
    }

    pub fn cooperation(&self) -> WitnessCooperation {
        self.cooperation
    }

    pub fn registered_at(&self) -> SimTime {
        self.registered_at
    }

    pub fn statements(&self) -> &BTreeSet<WitnessStatementId> {
        &self.statements
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessStatementRecord {
    pub(super) id: WitnessStatementId,
    pub(super) case_witness: CaseWitnessId,
    pub(super) subject: EntityRef,
    pub(super) origin: Option<EntityRef>,
    pub(super) confidence: Rating,
    /// Cooperation in effect when the statement was recorded. Later cooperation changes
    /// must not retroactively re-grade already-persisted testimony.
    pub(super) cooperation: WitnessCooperation,
    pub(super) summary: String,
    pub(super) evidence: EvidenceId,
    pub(super) recorded_at: SimTime,
}

impl WitnessStatementRecord {
    pub fn id(&self) -> WitnessStatementId {
        self.id
    }

    pub fn case_witness(&self) -> CaseWitnessId {
        self.case_witness
    }

    pub fn subject(&self) -> EntityRef {
        self.subject
    }

    pub fn origin(&self) -> Option<EntityRef> {
        self.origin
    }

    pub fn confidence(&self) -> Rating {
        self.confidence
    }

    pub fn cooperation(&self) -> WitnessCooperation {
        self.cooperation
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn evidence(&self) -> EvidenceId {
        self.evidence
    }

    pub fn recorded_at(&self) -> SimTime {
        self.recorded_at
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CaseWitnessDraft {
    pub investigation: InvestigationId,
    pub witness: CharacterId,
    pub cooperation: WitnessCooperation,
}

#[derive(Clone, Debug)]
pub struct WitnessStatementDraft {
    pub case_witness: CaseWitnessId,
    pub subject: EntityRef,
    pub origin: Option<EntityRef>,
    pub confidence: Rating,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformantStatus {
    Active,
    Terminated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InformantRecord {
    pub(super) id: InformantId,
    pub(super) character: CharacterId,
    pub(super) handler: OrganizationId,
    pub(super) status: InformantStatus,
    pub(super) established_at: SimTime,
    pub(super) terminated_at: Option<SimTime>,
    pub(super) version: u32,
}

impl InformantRecord {
    pub fn id(&self) -> InformantId {
        self.id
    }

    pub fn character(&self) -> CharacterId {
        self.character
    }

    pub fn handler(&self) -> OrganizationId {
        self.handler
    }

    pub fn status(&self) -> InformantStatus {
        self.status
    }

    pub fn established_at(&self) -> SimTime {
        self.established_at
    }

    pub fn terminated_at(&self) -> Option<SimTime> {
        self.terminated_at
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InformantDisclosureRecord {
    pub(super) id: InformantDisclosureId,
    pub(super) informant: InformantId,
    pub(super) investigation: InvestigationId,
    pub(super) source_information: InformationId,
    pub(super) evidence: EvidenceId,
    pub(super) disclosed_at: SimTime,
}

impl InformantDisclosureRecord {
    pub fn id(&self) -> InformantDisclosureId {
        self.id
    }

    pub fn informant(&self) -> InformantId {
        self.informant
    }

    pub fn investigation(&self) -> InvestigationId {
        self.investigation
    }

    pub fn source_information(&self) -> InformationId {
        self.source_information
    }

    pub fn evidence(&self) -> EvidenceId {
        self.evidence
    }

    pub fn disclosed_at(&self) -> SimTime {
        self.disclosed_at
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationRecord {
    pub(super) id: InvestigationId,
    pub(super) owner: OrganizationId,
    pub(super) title: String,
    pub(super) status: InvestigationStatus,
    pub(super) lead_investigator: Option<CharacterId>,
    pub(super) assigned_investigators: BTreeSet<CharacterId>,
    pub(super) subjects: BTreeSet<EntityRef>,
    pub(super) evidence: BTreeSet<EvidenceId>,
    pub(super) opened_at: SimTime,
    /// The operation whose exposure opened this case. Only operation-originated cases are
    /// eligible for deterministic cold-case decay; institution-authored cases keep their own
    /// lifecycle until an explicit transition.
    pub(super) origin_operation: Option<OperationId>,
    /// Organizations that were surfaced the case-open legal-activity knowledge when the case was
    /// opened. Surveillance of the owning authority uses this set (never the hidden evidence
    /// graph) to report whether that organization's case is still being actively worked.
    pub(super) notified_organizations: BTreeSet<OrganizationId>,
    /// The most recent minute the case gained evidence, subjects, scheduled work, or resolved
    /// work. Cold-case decay measures institutional inactivity from this instant.
    pub(super) last_activity_at: SimTime,
    pub(super) version: u32,
}

impl InvestigationRecord {
    pub fn id(&self) -> InvestigationId {
        self.id
    }
    pub fn owner(&self) -> OrganizationId {
        self.owner
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn status(&self) -> InvestigationStatus {
        self.status
    }
    pub fn lead_investigator(&self) -> Option<CharacterId> {
        self.lead_investigator
    }
    pub fn assigned_investigators(&self) -> &BTreeSet<CharacterId> {
        &self.assigned_investigators
    }
    pub fn investigator_role(&self, investigator: CharacterId) -> Option<InvestigatorRole> {
        if self.lead_investigator == Some(investigator) {
            Some(InvestigatorRole::Lead)
        } else if self.assigned_investigators.contains(&investigator) {
            Some(InvestigatorRole::Investigator)
        } else {
            None
        }
    }
    pub fn subjects(&self) -> &BTreeSet<EntityRef> {
        &self.subjects
    }
    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }
    pub fn opened_at(&self) -> SimTime {
        self.opened_at
    }
    pub fn origin_operation(&self) -> Option<OperationId> {
        self.origin_operation
    }
    pub fn notified_organizations(&self) -> &BTreeSet<OrganizationId> {
        &self.notified_organizations
    }
    pub fn last_activity_at(&self) -> SimTime {
        self.last_activity_at
    }
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceIdentity {
    pub(super) id: EvidenceId,
    pub(super) investigation: InvestigationId,
    pub(super) custodian: OrganizationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceConnection {
    pub(super) subject: EntityRef,
    pub(super) origin: Option<EntityRef>,
    pub(super) source: Option<EntityRef>,
    pub(super) derived_from: BTreeSet<EvidenceId>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EvidenceAssessment {
    pub(super) kind: EvidenceKind,
    pub(super) strength: EvidenceStrength,
    pub(super) reliability: EvidenceReliability,
    pub(super) admissibility: Admissibility,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub(super) identity: EvidenceIdentity,
    pub(super) connection: EvidenceConnection,
    pub(super) assessment: EvidenceAssessment,
    pub(super) discovered_at: SimTime,
}

impl EvidenceRecord {
    pub fn id(&self) -> EvidenceId {
        self.identity.id
    }
    pub fn investigation(&self) -> InvestigationId {
        self.identity.investigation
    }
    pub fn custodian(&self) -> OrganizationId {
        self.identity.custodian
    }
    pub fn subject(&self) -> EntityRef {
        self.connection.subject
    }
    pub fn origin(&self) -> Option<EntityRef> {
        self.connection.origin
    }
    pub fn source(&self) -> Option<EntityRef> {
        self.connection.source
    }
    pub fn kind(&self) -> EvidenceKind {
        self.assessment.kind
    }
    pub fn strength(&self) -> EvidenceStrength {
        self.assessment.strength
    }
    pub fn reliability(&self) -> EvidenceReliability {
        self.assessment.reliability
    }
    pub fn admissibility(&self) -> Admissibility {
        self.assessment.admissibility
    }
    pub fn discovered_at(&self) -> SimTime {
        self.discovered_at
    }
    pub fn derived_from(&self) -> &BTreeSet<EvidenceId> {
        &self.connection.derived_from
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JurisdictionRecord {
    pub(super) organization: OrganizationId,
    pub(super) neighborhoods: BTreeSet<NeighborhoodId>,
    pub(super) case_intake_priority: Rating,
    pub(super) version: u32,
}

impl JurisdictionRecord {
    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn neighborhoods(&self) -> &BTreeSet<NeighborhoodId> {
        &self.neighborhoods
    }

    pub fn case_intake_priority(&self) -> Rating {
        self.case_intake_priority
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct DayMinute(u16);

impl DayMinute {
    pub const MAX: u16 = 1_439;

    pub fn try_new(value: u16) -> Result<Self, DayMinuteError> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(DayMinuteError { value })
        }
    }

    pub const fn value(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for DayMinute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("minute of day {value} is outside the inclusive range 0..=1439")]
pub struct DayMinuteError {
    value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PatrolWindow {
    start: DayMinute,
    duration_minutes: u16,
    presence: Rating,
}

impl PatrolWindow {
    pub const MIN_DURATION_MINUTES: u16 = 1;
    pub const MAX_DURATION_MINUTES: u16 = 1_440;

    pub fn try_new(
        start: DayMinute,
        duration_minutes: u16,
        presence: Rating,
    ) -> Result<Self, PatrolWindowError> {
        if !(Self::MIN_DURATION_MINUTES..=Self::MAX_DURATION_MINUTES).contains(&duration_minutes) {
            return Err(PatrolWindowError { duration_minutes });
        }
        Ok(Self {
            start,
            duration_minutes,
            presence,
        })
    }

    pub const fn start(self) -> DayMinute {
        self.start
    }

    pub const fn duration_minutes(self) -> u16 {
        self.duration_minutes
    }

    pub const fn presence(self) -> Rating {
        self.presence
    }
}

impl<'de> Deserialize<'de> for PatrolWindow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedPatrolWindow {
            start: DayMinute,
            duration_minutes: u16,
            presence: Rating,
        }

        let serialized = SerializedPatrolWindow::deserialize(deserializer)?;
        Self::try_new(
            serialized.start,
            serialized.duration_minutes,
            serialized.presence,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("patrol window duration {duration_minutes} is outside the inclusive range 1..=1440")]
pub struct PatrolWindowError {
    duration_minutes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatrolDeploymentStatus {
    Active,
    Suspended,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatrolDeploymentRecord {
    pub(super) id: PatrolDeploymentId,
    pub(super) organization: OrganizationId,
    pub(super) neighborhood: NeighborhoodId,
    pub(super) windows: Vec<PatrolWindow>,
    pub(super) status: PatrolDeploymentStatus,
    pub(super) established_at: SimTime,
    pub(super) last_changed_at: SimTime,
    pub(super) version: u32,
}

impl PatrolDeploymentRecord {
    pub fn id(&self) -> PatrolDeploymentId {
        self.id
    }

    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn neighborhood(&self) -> NeighborhoodId {
        self.neighborhood
    }

    pub fn windows(&self) -> &[PatrolWindow] {
        &self.windows
    }

    pub fn status(&self) -> PatrolDeploymentStatus {
        self.status
    }

    pub fn established_at(&self) -> SimTime {
        self.established_at
    }

    pub fn last_changed_at(&self) -> SimTime {
        self.last_changed_at
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug)]
pub struct PatrolDeploymentDraft {
    pub organization: OrganizationId,
    pub neighborhood: NeighborhoodId,
    pub windows: Vec<PatrolWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoliceResponseStatus {
    Dispatched,
    Arrived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliceResponsePatrolSnapshot {
    deployment: PatrolDeploymentId,
    version: u32,
}

impl PoliceResponsePatrolSnapshot {
    pub(crate) fn new(deployment: PatrolDeploymentId, version: u32) -> Self {
        Self {
            deployment,
            version,
        }
    }

    pub fn deployment(self) -> PatrolDeploymentId {
        self.deployment
    }

    pub fn version(self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseRouting {
    pub(super) authority: OrganizationId,
    pub(super) neighborhood: NeighborhoodId,
    pub(super) source_operation: OperationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseTiming {
    pub(super) dispatched_at: SimTime,
    pub(super) arrival_due_at: SimTime,
    pub(super) arrived_at: Option<SimTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseState {
    pub(super) alert_score: i16,
    pub(super) response_presence: Rating,
    pub(super) jurisdiction_version: u32,
    pub(super) patrol: Option<PoliceResponsePatrolSnapshot>,
    pub(super) status: PoliceResponseStatus,
    pub(super) version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoliceResponseRecord {
    pub(super) id: PoliceResponseId,
    pub(super) routing: PoliceResponseRouting,
    pub(super) timing: PoliceResponseTiming,
    pub(super) state: PoliceResponseState,
}

impl PoliceResponseRecord {
    pub fn id(&self) -> PoliceResponseId {
        self.id
    }
    pub fn authority(&self) -> OrganizationId {
        self.routing.authority
    }
    pub fn neighborhood(&self) -> NeighborhoodId {
        self.routing.neighborhood
    }
    pub fn source_operation(&self) -> OperationId {
        self.routing.source_operation
    }
    pub fn dispatched_at(&self) -> SimTime {
        self.timing.dispatched_at
    }
    pub fn arrival_due_at(&self) -> SimTime {
        self.timing.arrival_due_at
    }
    pub fn arrived_at(&self) -> Option<SimTime> {
        self.timing.arrived_at
    }
    pub fn alert_score(&self) -> i16 {
        self.state.alert_score
    }
    pub fn response_presence(&self) -> Rating {
        self.state.response_presence
    }
    pub fn jurisdiction_version(&self) -> u32 {
        self.state.jurisdiction_version
    }
    pub fn patrol(&self) -> Option<PoliceResponsePatrolSnapshot> {
        self.state.patrol
    }
    pub fn status(&self) -> PoliceResponseStatus {
        self.state.status
    }
    pub fn version(&self) -> u32 {
        self.state.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct InvestigationIndexes {
    pub(super) by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    pub(super) investigations_by_subject: BTreeMap<EntityRef, BTreeSet<InvestigationId>>,
    pub(super) investigations_by_investigator: BTreeMap<CharacterId, BTreeSet<InvestigationId>>,
    pub(super) active_without_lead: BTreeSet<InvestigationId>,
    /// Every active case keyed by its last activity instant, so cold-case decay finds due
    /// institutional-inactivity candidates deterministically without scanning the case set.
    pub(super) cases_by_last_activity: BTreeMap<SimTime, BTreeSet<InvestigationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct EvidenceIndexes {
    pub(super) evidence_by_origin: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    pub(super) evidence_by_source: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    pub(super) evidence_by_kind: BTreeMap<EvidenceKind, BTreeSet<EvidenceId>>,
    pub(super) derived_evidence_by_source: BTreeMap<EvidenceId, BTreeSet<EvidenceId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct WitnessIndexes {
    pub(super) case_witness_by_case_character:
        BTreeMap<(InvestigationId, CharacterId), CaseWitnessId>,
    pub(super) case_witnesses_by_investigation: BTreeMap<InvestigationId, BTreeSet<CaseWitnessId>>,
    pub(super) witness_statement_by_evidence: BTreeMap<EvidenceId, WitnessStatementId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct InformantIndexes {
    pub(super) active_by_character_handler: BTreeMap<(CharacterId, OrganizationId), InformantId>,
    pub(super) by_character: BTreeMap<CharacterId, BTreeSet<InformantId>>,
    pub(super) disclosures_by_informant: BTreeMap<InformantId, BTreeSet<InformantDisclosureId>>,
    pub(super) disclosure_by_evidence: BTreeMap<EvidenceId, InformantDisclosureId>,
    pub(super) disclosures_by_information: BTreeMap<InformationId, BTreeSet<InformantDisclosureId>>,
    pub(super) disclosure_by_case_information:
        BTreeMap<(InvestigationId, InformationId), InformantDisclosureId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct InvestigationWorkIndexes {
    pub(super) work_by_investigation: BTreeMap<InvestigationId, BTreeSet<InvestigationWorkId>>,
    pub(super) work_by_investigator: BTreeMap<CharacterId, BTreeSet<InvestigationWorkId>>,
    pub(super) scheduled_work_by_due_at: BTreeMap<SimTime, BTreeSet<InvestigationWorkId>>,
    pub(super) scheduled_work_by_focus: BTreeMap<
        (
            InvestigationId,
            InvestigationWorkKind,
            InvestigationWorkFocus,
        ),
        InvestigationWorkId,
    >,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct JurisdictionIndexes {
    pub(super) jurisdictions_by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<OrganizationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct PatrolIndexes {
    pub(super) active_by_organization_neighborhood:
        BTreeMap<(OrganizationId, NeighborhoodId), PatrolDeploymentId>,
    pub(super) active_by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<PatrolDeploymentId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct PoliceResponseIndexes {
    pub(super) by_source_operation: BTreeMap<OperationId, PoliceResponseId>,
    pub(super) dispatched_by_arrival_due: BTreeMap<SimTime, BTreeSet<PoliceResponseId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct LegalIndexes {
    pub(super) investigations: InvestigationIndexes,
    pub(super) evidence: EvidenceIndexes,
    pub(super) witnesses: WitnessIndexes,
    pub(super) informants: InformantIndexes,
    pub(super) work: InvestigationWorkIndexes,
    pub(super) jurisdictions: JurisdictionIndexes,
    pub(super) patrols: PatrolIndexes,
    pub(super) police_responses: PoliceResponseIndexes,
    pub(super) arrests: ArrestIndexes,
    pub(super) representations: LegalRepresentationIndexes,
    pub(super) prosecutions: ProsecutionIndexes,
}
#[derive(Clone, Debug)]
pub struct JurisdictionDraft {
    pub organization: OrganizationId,
    pub neighborhoods: BTreeSet<NeighborhoodId>,
    pub case_intake_priority: Rating,
}

pub struct InvestigationDraft {
    pub owner: OrganizationId,
    pub title: String,
    pub subjects: BTreeSet<EntityRef>,
}

#[derive(Clone, Copy, Debug)]
pub struct InformantDraft {
    pub character: CharacterId,
    pub handler: OrganizationId,
}

#[derive(Clone, Copy, Debug)]
pub struct InformantDisclosureDraft {
    pub informant: InformantId,
    pub investigation: InvestigationId,
    pub source_information: InformationId,
}

#[derive(Clone, Debug)]
pub struct IncidentEvidenceDraft {
    pub subject: EntityRef,
    pub origin: Option<EntityRef>,
    pub kind: EvidenceKind,
    pub strength: EvidenceStrength,
    pub reliability: EvidenceReliability,
    pub admissibility: Admissibility,
    pub discovered_at: SimTime,
}

#[derive(Clone, Debug)]
pub struct IncidentIntakeDraft {
    pub owner: OrganizationId,
    pub title: String,
    pub subjects: BTreeSet<EntityRef>,
    pub evidence: Vec<IncidentEvidenceDraft>,
    /// The operation whose exposure opened this case; only operation-originated incidents carry
    /// this link so cold-case decay never touches institution-authored casework.
    pub origin_operation: Option<OperationId>,
    /// Organizations surfaced the case-open legal-activity knowledge at intake; the owning
    /// authority and later surveillance read only this set to decide what is visible about the
    /// case, never the hidden evidence or investigation internals.
    pub notified_organizations: BTreeSet<OrganizationId>,
    /// A named witness registered with the case at intake (for example an identifiable
    /// business owner who saw the incident). Anonymous testimony remains ordinary evidence.
    pub witness: Option<IncidentWitnessDraft>,
}

#[derive(Clone, Debug)]
pub struct IncidentWitnessDraft {
    pub character: CharacterId,
    pub cooperation: WitnessCooperation,
}

pub struct EvidenceDraft {
    pub investigation: InvestigationId,
    pub custodian: OrganizationId,
    pub subject: EntityRef,
    pub origin: Option<EntityRef>,
    pub kind: EvidenceKind,
    pub strength: EvidenceStrength,
    pub reliability: EvidenceReliability,
    pub admissibility: Admissibility,
    pub discovered_at: SimTime,
}
