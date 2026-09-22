//! Investigation, evidence, witness, informant, and investigation-work record vocabulary.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, EvidenceId, InformantDisclosureId, InformantId,
    InformationId, InvestigationId, InvestigationWorkId, OrganizationId, WitnessStatementId,
};
use crate::core::time::SimTime;
use crate::world::Rating;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationStatus {
    Active,
    Suspended,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigationWorkKind {
    EvidenceReview,
    WitnessInterview,
}

pub const ALL_INVESTIGATION_WORK_KINDS: [InvestigationWorkKind; 2] = [
    InvestigationWorkKind::EvidenceReview,
    InvestigationWorkKind::WitnessInterview,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigationWorkFocus {
    Evidence(EvidenceId),
    Witness(CaseWitnessId),
}

impl InvestigationWorkFocus {
    pub fn evidence(evidence: EvidenceId) -> Self {
        Self::Evidence(evidence)
    }

    pub fn witness(case_witness: CaseWitnessId) -> Self {
        Self::Witness(case_witness)
    }

    pub fn evidence_id(self) -> Option<EvidenceId> {
        match self {
            Self::Evidence(evidence) => Some(evidence),
            Self::Witness(_) => None,
        }
    }

    pub fn witness_id(self) -> Option<CaseWitnessId> {
        match self {
            Self::Witness(case_witness) => Some(case_witness),
            Self::Evidence(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationWorkStatus {
    Scheduled,
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationWorkCancellationReason {
    InvestigatorDetained(ArrestId),
    /// Actionable evidence made the assigned investigator a subject of the case they were
    /// working. The evidence remains authoritative; the conflicted work and lead seat do not.
    InvestigatorBecameCaseSubject(EvidenceId),
    /// Later case development made the interview target an arrest-eligible subject of the same
    /// investigation. The witness registration remains historical, but unfinished interview work
    /// cannot continue across that conflict.
    WitnessBecameCaseSubject(EvidenceId),
    /// A statement entered through the canonical witness path before the scheduled interview
    /// completed, making that interview redundant. The statement id keeps cancellation
    /// provenance exact without duplicating witness facts on the work record.
    WitnessStatementRecorded(WitnessStatementId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationWorkCancellation {
    pub(in crate::legal) cancelled_at: SimTime,
    pub(in crate::legal) reason: InvestigationWorkCancellationReason,
}

impl InvestigationWorkCancellation {
    pub fn cancelled_at(self) -> SimTime {
        self.cancelled_at
    }

    pub fn reason(self) -> InvestigationWorkCancellationReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationWorkOutcome {
    Connected,
    Developed,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationWorkFactors {
    pub(in crate::legal) investigation_capability: Rating,
    pub(in crate::legal) source_support: Rating,
    pub(in crate::legal) difficulty: u8,
    pub(in crate::legal) variance: i8,
}

impl InvestigationWorkFactors {
    pub fn investigation_capability(self) -> Rating {
        self.investigation_capability
    }

    pub fn source_support(self) -> Rating {
        self.source_support
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
    pub(in crate::legal) resolved_at: SimTime,
    pub(in crate::legal) outcome: InvestigationWorkOutcome,
    pub(in crate::legal) factors: InvestigationWorkFactors,
    pub(in crate::legal) margin: i16,
    pub(in crate::legal) derived_evidence: Option<EvidenceId>,
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

    pub fn derived_evidence(&self) -> Option<EvidenceId> {
        self.derived_evidence
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkIdentity {
    pub(in crate::legal) id: InvestigationWorkId,
    pub(in crate::legal) investigation: InvestigationId,
    pub(in crate::legal) investigator: CharacterId,
    pub(in crate::legal) kind: InvestigationWorkKind,
    pub(in crate::legal) focus: InvestigationWorkFocus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkRuntime {
    pub(in crate::legal) scheduled_at: SimTime,
    pub(in crate::legal) due_at: SimTime,
    pub(in crate::legal) status: InvestigationWorkStatus,
    pub(in crate::legal) resolution: Option<InvestigationWorkResolution>,
    pub(in crate::legal) cancellation: Option<InvestigationWorkCancellation>,
    pub(in crate::legal) version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkRecord {
    pub(in crate::legal) identity: InvestigationWorkIdentity,
    pub(in crate::legal) source_evidence: BTreeSet<EvidenceId>,
    pub(in crate::legal) runtime: InvestigationWorkRuntime,
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

    pub fn cancellation(&self) -> Option<InvestigationWorkCancellation> {
        self.runtime.cancellation
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
    pub(in crate::legal) id: CaseWitnessId,
    pub(in crate::legal) investigation: InvestigationId,
    pub(in crate::legal) witness: CharacterId,
    /// Subject matter this witness was registered as having observed. Persisting the binding
    /// prevents a later interview from choosing a different suspect merely because stronger
    /// evidence entered the case after registration.
    pub(in crate::legal) subject: EntityRef,
    pub(in crate::legal) cooperation: WitnessCooperation,
    pub(in crate::legal) registered_at: SimTime,
    pub(in crate::legal) statements: BTreeSet<WitnessStatementId>,
    /// Completed interviews this witness has sat through, counted whether or not any
    /// produced a statement. Caps futile re-interviews of witnesses who never open up.
    pub(in crate::legal) interview_attempts: u8,
    pub(in crate::legal) version: u32,
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

    pub fn subject(&self) -> EntityRef {
        self.subject
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

    pub fn interview_attempts(&self) -> u8 {
        self.interview_attempts
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessStatementRecord {
    pub(in crate::legal) id: WitnessStatementId,
    pub(in crate::legal) case_witness: CaseWitnessId,
    pub(in crate::legal) subject: EntityRef,
    pub(in crate::legal) origin: Option<EntityRef>,
    pub(in crate::legal) confidence: Rating,
    /// Cooperation in effect when the statement was recorded. Later cooperation changes
    /// must not retroactively re-grade already-persisted testimony.
    pub(in crate::legal) cooperation: WitnessCooperation,
    pub(in crate::legal) summary: String,
    pub(in crate::legal) evidence: EvidenceId,
    pub(in crate::legal) recorded_at: SimTime,
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
    pub subject: EntityRef,
    pub cooperation: WitnessCooperation,
}

#[derive(Clone, Debug)]
pub struct WitnessStatementDraft {
    pub case_witness: CaseWitnessId,
    pub origin: Option<EntityRef>,
    pub confidence: Rating,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InformantRecord {
    pub(in crate::legal) id: InformantId,
    pub(in crate::legal) character: CharacterId,
    pub(in crate::legal) handler: OrganizationId,
    pub(in crate::legal) established_at: SimTime,
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

    pub fn established_at(&self) -> SimTime {
        self.established_at
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InformantDisclosureRecord {
    pub(in crate::legal) id: InformantDisclosureId,
    pub(in crate::legal) informant: InformantId,
    pub(in crate::legal) investigation: InvestigationId,
    pub(in crate::legal) source_information: InformationId,
    pub(in crate::legal) evidence: EvidenceId,
    pub(in crate::legal) disclosed_at: SimTime,
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
    pub(in crate::legal) id: InvestigationId,
    pub(in crate::legal) owner: OrganizationId,
    pub(in crate::legal) title: String,
    pub(in crate::legal) status: InvestigationStatus,
    pub(in crate::legal) lead_investigator: Option<CharacterId>,
    /// Subject matter explicitly declared when the case was opened or later continued by a
    /// validated incident. `subjects` is the effective tracked set and additionally includes
    /// entities promoted by actionable evidence. Keeping both makes that promotion provenance
    /// re-derivable at restore instead of letting an evidence-produced subject justify itself.
    pub(in crate::legal) declared_subjects: BTreeSet<EntityRef>,
    pub(in crate::legal) subjects: BTreeSet<EntityRef>,
    pub(in crate::legal) evidence: BTreeSet<EvidenceId>,
    pub(in crate::legal) opened_at: SimTime,
    /// The entity whose exposure or notoriety opened this case (currently an operation or an
    /// enterprise). Only originated cases are eligible for
    /// deterministic cold-case decay; institution-authored cases keep their own lifecycle
    /// until an explicit transition.
    pub(in crate::legal) origin: Option<EntityRef>,
    /// The most recent minute the institution advanced the case through evidence, subjects,
    /// witness registration, scheduled work, resolved work, or explicit resumption. External
    /// witness-cooperation changes and custody-forced staffing/work cancellation do not count as
    /// investigative activity. Cold-case decay measures institutional inactivity from this instant.
    pub(in crate::legal) last_activity_at: SimTime,
    pub(in crate::legal) version: u32,
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
    pub fn declared_subjects(&self) -> &BTreeSet<EntityRef> {
        &self.declared_subjects
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
    pub fn origin(&self) -> Option<EntityRef> {
        self.origin
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
    pub(in crate::legal) id: EvidenceId,
    pub(in crate::legal) investigation: InvestigationId,
    pub(in crate::legal) custodian: OrganizationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceConnection {
    pub(in crate::legal) subject: EntityRef,
    pub(in crate::legal) origin: Option<EntityRef>,
    pub(in crate::legal) source: Option<EntityRef>,
    pub(in crate::legal) derived_from: BTreeSet<EvidenceId>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EvidenceAssessment {
    pub(in crate::legal) kind: EvidenceKind,
    pub(in crate::legal) strength: EvidenceStrength,
    pub(in crate::legal) reliability: EvidenceReliability,
    pub(in crate::legal) admissibility: Admissibility,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub(in crate::legal) identity: EvidenceIdentity,
    pub(in crate::legal) connection: EvidenceConnection,
    pub(in crate::legal) assessment: EvidenceAssessment,
    pub(in crate::legal) discovered_at: SimTime,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct InvestigationIndexes {
    pub(in crate::legal) by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    /// Active cases grouped by owner so handler-specific recurring legal work does not scan
    /// unrelated active institutions or historical case records.
    pub(in crate::legal) active_by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    /// Suspended originated shelves grouped by owner. Incident continuation consults only these
    /// resumable files rather than rescanning an institution's lifetime active/closed case history.
    pub(in crate::legal) suspended_originated_by_owner:
        BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    pub(in crate::legal) investigations_by_subject: BTreeMap<EntityRef, BTreeSet<InvestigationId>>,
    pub(in crate::legal) investigations_by_investigator:
        BTreeMap<CharacterId, BTreeSet<InvestigationId>>,
    pub(in crate::legal) active_without_lead: BTreeSet<InvestigationId>,
    /// Every active case regardless of lead status, so per-tick institutional passes
    /// (evidence arrests, witness scheduling, informant disclosures) iterate live work
    /// instead of the full case history, which grows for the life of the campaign.
    pub(in crate::legal) active: BTreeSet<InvestigationId>,
    /// Every active case keyed by its last activity instant, so cold-case decay finds due
    /// institutional-inactivity candidates deterministically without scanning the case set.
    pub(in crate::legal) cases_by_last_activity: BTreeMap<SimTime, BTreeSet<InvestigationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct EvidenceIndexes {
    pub(in crate::legal) derived_evidence_by_source: BTreeMap<EvidenceId, BTreeSet<EvidenceId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct WitnessIndexes {
    pub(in crate::legal) case_witness_by_case_character:
        BTreeMap<(InvestigationId, CharacterId), CaseWitnessId>,
    pub(in crate::legal) case_witnesses_by_investigation:
        BTreeMap<InvestigationId, BTreeSet<CaseWitnessId>>,
    /// Every registration naming a character as case witness, so witness-pressure targeting
    /// scans that character's own registrations instead of the full witness history, which
    /// grows for the life of the campaign.
    pub(in crate::legal) case_witnesses_by_character:
        BTreeMap<CharacterId, BTreeSet<CaseWitnessId>>,
    pub(in crate::legal) witness_statement_by_evidence: BTreeMap<EvidenceId, WitnessStatementId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct InformantIndexes {
    pub(in crate::legal) by_character_handler: BTreeMap<(CharacterId, OrganizationId), InformantId>,
    /// Informants grouped by handler so recurring disclosure work touches only relationships
    /// whose institution currently owns relevant active casework.
    pub(in crate::legal) by_handler: BTreeMap<OrganizationId, BTreeSet<InformantId>>,
    pub(in crate::legal) disclosure_by_case_information:
        BTreeMap<(InvestigationId, InformationId), InformantDisclosureId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct InvestigationWorkIndexes {
    pub(in crate::legal) work_by_investigation:
        BTreeMap<InvestigationId, BTreeSet<InvestigationWorkId>>,
    pub(in crate::legal) work_by_investigator: BTreeMap<CharacterId, BTreeSet<InvestigationWorkId>>,
    /// The single currently scheduled work item per investigator. Historical completed and
    /// cancelled work remains in `work_by_investigator`, while hot capacity/preemption checks use
    /// this live projection instead of rescanning an investigator's lifetime work history.
    pub(in crate::legal) scheduled_work_by_investigator: BTreeMap<CharacterId, InvestigationWorkId>,
    /// One real evidence-review attempt per source. Scheduled and completed reviews occupy this
    /// projection; cancelled work releases it because no review occurred. This both enforces the
    /// one-attempt rule for direct commands and avoids rescanning casework history every minute.
    pub(in crate::legal) evidence_review_attempt_by_source:
        BTreeMap<EvidenceId, InvestigationWorkId>,
    pub(in crate::legal) scheduled_work_by_due_at: BTreeMap<SimTime, BTreeSet<InvestigationWorkId>>,
    pub(in crate::legal) scheduled_work_by_focus: BTreeMap<
        (
            InvestigationId,
            InvestigationWorkKind,
            InvestigationWorkFocus,
        ),
        InvestigationWorkId,
    >,
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
    /// The entity whose exposure or notoriety opened this case; only originated incidents carry
    /// this link so cold-case decay never touches institution-authored casework.
    pub origin: Option<EntityRef>,
    /// A named witness registered with the case at intake (for example an identifiable
    /// business owner who saw the incident). Anonymous testimony remains ordinary evidence.
    pub witness: Option<IncidentWitnessDraft>,
}

#[derive(Clone, Debug)]
pub struct IncidentWitnessDraft {
    pub character: CharacterId,
    /// Subject this named witness actually observed in the incident.
    pub subject: EntityRef,
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
