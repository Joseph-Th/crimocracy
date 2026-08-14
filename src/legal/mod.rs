//! Specific investigations, staffing, and evidence graphs; sibling systems own case transactions and derived graph queries.

pub mod case_graph;
pub mod investigation_system;
pub mod investigation_work_execution;
pub mod jurisdiction_system;
pub mod witness_system;

use crate::core::entity::EntityRef;
use crate::core::id::{
    CaseWitnessId, CharacterId, EvidenceId, InvestigationId, InvestigationWorkId, NeighborhoodId,
    OrganizationId, WitnessStatementId,
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
    Referred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigatorRole {
    Lead,
    Investigator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InvestigationWorkKind {
    PatternAnalysis,
}

pub const ALL_INVESTIGATION_WORK_KINDS: [InvestigationWorkKind; 1] =
    [InvestigationWorkKind::PatternAnalysis];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InvestigationWorkFocus {
    from: EntityRef,
    to: EntityRef,
}

impl InvestigationWorkFocus {
    pub fn new(from: EntityRef, to: EntityRef) -> Self {
        if from <= to {
            Self { from, to }
        } else {
            Self { from: to, to: from }
        }
    }

    pub fn from(self) -> EntityRef {
        self.from
    }

    pub fn to(self) -> EntityRef {
        self.to
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
    Inconclusive,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationWorkFactors {
    investigation_capability: Rating,
    source_support: Rating,
    source_evidence_count: u8,
    difficulty: u8,
    variance: i8,
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
    resolved_at: SimTime,
    outcome: InvestigationWorkOutcome,
    factors: InvestigationWorkFactors,
    margin: i16,
    superseded_by: Option<EvidenceId>,
    derived_evidence: Option<EvidenceId>,
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
struct InvestigationWorkIdentity {
    id: InvestigationWorkId,
    investigation: InvestigationId,
    investigator: CharacterId,
    kind: InvestigationWorkKind,
    focus: InvestigationWorkFocus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InvestigationWorkRuntime {
    scheduled_at: SimTime,
    due_at: SimTime,
    status: InvestigationWorkStatus,
    resolution: Option<InvestigationWorkResolution>,
    version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationWorkRecord {
    identity: InvestigationWorkIdentity,
    source_evidence: BTreeSet<EvidenceId>,
    runtime: InvestigationWorkRuntime,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceStrength {
    Weak,
    Corroborating,
    Strong,
    Direct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    id: CaseWitnessId,
    investigation: InvestigationId,
    witness: CharacterId,
    cooperation: WitnessCooperation,
    registered_at: SimTime,
    statements: BTreeSet<WitnessStatementId>,
    version: u32,
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
    id: WitnessStatementId,
    case_witness: CaseWitnessId,
    subject: EntityRef,
    origin: Option<EntityRef>,
    confidence: Rating,
    summary: String,
    evidence: EvidenceId,
    recorded_at: SimTime,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationRecord {
    id: InvestigationId,
    owner: OrganizationId,
    title: String,
    status: InvestigationStatus,
    lead_investigator: Option<CharacterId>,
    assigned_investigators: BTreeSet<CharacterId>,
    subjects: BTreeSet<EntityRef>,
    evidence: BTreeSet<EvidenceId>,
    opened_at: SimTime,
    version: u32,
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
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EvidenceIdentity {
    id: EvidenceId,
    investigation: InvestigationId,
    custodian: OrganizationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EvidenceConnection {
    subject: EntityRef,
    origin: Option<EntityRef>,
    source: Option<EntityRef>,
    derived_from: BTreeSet<EvidenceId>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct EvidenceAssessment {
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceRecord {
    identity: EvidenceIdentity,
    connection: EvidenceConnection,
    assessment: EvidenceAssessment,
    discovered_at: SimTime,
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
    organization: OrganizationId,
    neighborhoods: BTreeSet<NeighborhoodId>,
    case_intake_priority: Rating,
    version: u32,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InvestigationIndexes {
    by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    investigations_by_subject: BTreeMap<EntityRef, BTreeSet<InvestigationId>>,
    investigations_by_investigator: BTreeMap<CharacterId, BTreeSet<InvestigationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct EvidenceIndexes {
    evidence_by_subject: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    evidence_by_origin: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    evidence_by_source: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    evidence_by_kind: BTreeMap<EvidenceKind, BTreeSet<EvidenceId>>,
    derived_evidence_by_source: BTreeMap<EvidenceId, BTreeSet<EvidenceId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct WitnessIndexes {
    case_witness_by_case_character: BTreeMap<(InvestigationId, CharacterId), CaseWitnessId>,
    case_witnesses_by_character: BTreeMap<CharacterId, BTreeSet<CaseWitnessId>>,
    case_witnesses_by_investigation: BTreeMap<InvestigationId, BTreeSet<CaseWitnessId>>,
    witness_statement_by_evidence: BTreeMap<EvidenceId, WitnessStatementId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InvestigationWorkIndexes {
    work_by_investigation: BTreeMap<InvestigationId, BTreeSet<InvestigationWorkId>>,
    work_by_investigator: BTreeMap<CharacterId, BTreeSet<InvestigationWorkId>>,
    scheduled_work_by_due_at: BTreeMap<SimTime, BTreeSet<InvestigationWorkId>>,
    scheduled_work_by_focus: BTreeMap<
        (
            InvestigationId,
            InvestigationWorkKind,
            InvestigationWorkFocus,
        ),
        InvestigationWorkId,
    >,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct JurisdictionIndexes {
    jurisdictions_by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<OrganizationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LegalIndexes {
    investigations: InvestigationIndexes,
    evidence: EvidenceIndexes,
    witnesses: WitnessIndexes,
    work: InvestigationWorkIndexes,
    jurisdictions: JurisdictionIndexes,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LegalState {
    investigations: BTreeMap<InvestigationId, InvestigationRecord>,
    investigation_work: BTreeMap<InvestigationWorkId, InvestigationWorkRecord>,
    case_witnesses: BTreeMap<CaseWitnessId, CaseWitnessRecord>,
    witness_statements: BTreeMap<WitnessStatementId, WitnessStatementRecord>,
    evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    jurisdictions: BTreeMap<OrganizationId, JurisdictionRecord>,
    indexes: LegalIndexes,
}

impl LegalState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_investigation(&self, id: InvestigationId) -> Option<&InvestigationRecord> {
        self.investigations.get(&id)
    }
    pub fn get_evidence(&self, id: EvidenceId) -> Option<&EvidenceRecord> {
        self.evidence.get(&id)
    }
    pub fn get_investigation_work(
        &self,
        id: InvestigationWorkId,
    ) -> Option<&InvestigationWorkRecord> {
        self.investigation_work.get(&id)
    }
    pub fn get_case_witness(&self, id: CaseWitnessId) -> Option<&CaseWitnessRecord> {
        self.case_witnesses.get(&id)
    }
    pub fn get_witness_statement(&self, id: WitnessStatementId) -> Option<&WitnessStatementRecord> {
        self.witness_statements.get(&id)
    }
    pub fn get_jurisdiction(&self, organization: OrganizationId) -> Option<&JurisdictionRecord> {
        self.jurisdictions.get(&organization)
    }
    pub fn jurisdictions_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &JurisdictionRecord> {
        self.indexes
            .jurisdictions
            .jurisdictions_by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .filter_map(|organization| self.jurisdictions.get(organization))
    }
    pub fn evidence_from_origin(&self, origin: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_origin
            .get(&origin)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn evidence_from_source(&self, source: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn derived_evidence_from(
        &self,
        source: EvidenceId,
    ) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .derived_evidence_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn investigations_for_subject(
        &self,
        subject: EntityRef,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes
            .investigations
            .investigations_by_subject
            .get(&subject)
            .into_iter()
            .flatten()
            .filter_map(|id| self.investigations.get(id))
    }
    pub fn case_witness_for(
        &self,
        investigation: InvestigationId,
        witness: CharacterId,
    ) -> Option<&CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witness_by_case_character
            .get(&(investigation, witness))
            .and_then(|id| self.case_witnesses.get(id))
    }
    pub fn case_witnesses_for_character(
        &self,
        witness: CharacterId,
    ) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witnesses_by_character
            .get(&witness)
            .into_iter()
            .flatten()
            .filter_map(|id| self.case_witnesses.get(id))
    }
    pub fn case_witnesses_for_investigation(
        &self,
        investigation: InvestigationId,
    ) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witnesses_by_investigation
            .get(&investigation)
            .into_iter()
            .flatten()
            .filter_map(|id| self.case_witnesses.get(id))
    }
    pub fn witness_statement_for_evidence(
        &self,
        evidence: EvidenceId,
    ) -> Option<&WitnessStatementRecord> {
        self.indexes
            .witnesses
            .witness_statement_by_evidence
            .get(&evidence)
            .and_then(|id| self.witness_statements.get(id))
    }
    pub fn statements_for_case_witness(
        &self,
        case_witness: CaseWitnessId,
    ) -> impl Iterator<Item = &WitnessStatementRecord> {
        self.case_witnesses
            .get(&case_witness)
            .into_iter()
            .flat_map(|witness| witness.statements().iter())
            .filter_map(|id| self.witness_statements.get(id))
    }
    pub fn work_for_investigation(
        &self,
        investigation: InvestigationId,
    ) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.indexes
            .work
            .work_by_investigation
            .get(&investigation)
            .into_iter()
            .flatten()
            .filter_map(|id| self.investigation_work.get(id))
    }
    pub fn work_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.indexes
            .work
            .work_by_investigator
            .get(&investigator)
            .into_iter()
            .flatten()
            .filter_map(|id| self.investigation_work.get(id))
    }
    pub(crate) fn scheduled_work_for_focus(
        &self,
        investigation: InvestigationId,
        kind: InvestigationWorkKind,
        focus: InvestigationWorkFocus,
    ) -> Option<&InvestigationWorkRecord> {
        self.indexes
            .work
            .scheduled_work_by_focus
            .get(&(investigation, kind, focus))
            .and_then(|id| self.investigation_work.get(id))
    }
    pub(crate) fn due_investigation_work_at_or_before(
        &self,
        now: SimTime,
    ) -> Vec<InvestigationWorkId> {
        self.indexes
            .work
            .scheduled_work_by_due_at
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
    pub fn evidence_of_kind(&self, kind: EvidenceKind) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_kind
            .get(&kind)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn investigations_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes
            .investigations
            .investigations_by_investigator
            .get(&investigator)
            .into_iter()
            .flatten()
            .filter_map(|id| self.investigations.get(id))
    }
    pub(crate) fn active_investigation_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> Option<&InvestigationRecord> {
        self.investigations_for_investigator(investigator)
            .find(|investigation| investigation.status() == InvestigationStatus::Active)
    }
    pub(crate) fn investigations(&self) -> impl Iterator<Item = &InvestigationRecord> {
        self.investigations.values()
    }
    pub(crate) fn investigation_work(&self) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.investigation_work.values()
    }
    pub(crate) fn case_witnesses(&self) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.case_witnesses.values()
    }
    pub(crate) fn witness_statements(&self) -> impl Iterator<Item = &WitnessStatementRecord> {
        self.witness_statements.values()
    }
    pub(crate) fn all_evidence(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence.values()
    }
    pub(crate) fn jurisdictions(&self) -> impl Iterator<Item = &JurisdictionRecord> {
        self.jurisdictions.values()
    }
    pub(crate) fn insert_investigation(&mut self, record: InvestigationRecord) {
        self.indexes
            .investigations
            .by_owner
            .entry(record.owner())
            .or_default()
            .insert(record.id());
        for subject in record.subjects() {
            self.indexes
                .investigations
                .investigations_by_subject
                .entry(*subject)
                .or_default()
                .insert(record.id());
        }
        let previous = self.investigations.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate investigation ID inserted"
        );
    }
    pub(crate) fn insert_evidence(&mut self, record: EvidenceRecord) {
        let investigation = self
            .investigations
            .get_mut(&record.investigation())
            .expect("validated investigation disappeared before evidence commit");
        investigation.subjects.insert(record.subject());
        investigation.evidence.insert(record.id());
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.indexes
            .investigations
            .investigations_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.investigation());
        self.indexes
            .evidence
            .evidence_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.id());
        if let Some(origin) = record.origin() {
            self.indexes
                .evidence
                .evidence_by_origin
                .entry(origin)
                .or_default()
                .insert(record.id());
        }
        if let Some(source) = record.source() {
            self.indexes
                .evidence
                .evidence_by_source
                .entry(source)
                .or_default()
                .insert(record.id());
        }
        self.indexes
            .evidence
            .evidence_by_kind
            .entry(record.kind())
            .or_default()
            .insert(record.id());
        for source in record.derived_from() {
            self.indexes
                .evidence
                .derived_evidence_by_source
                .entry(*source)
                .or_default()
                .insert(record.id());
        }
        let previous = self.evidence.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate evidence ID inserted"
        );
    }
    pub(crate) fn insert_case_witness(&mut self, record: CaseWitnessRecord) {
        let id = record.id();
        let key = (record.investigation(), record.witness());
        let previous_key = self
            .indexes
            .witnesses
            .case_witness_by_case_character
            .insert(key, id);
        debug_assert!(
            previous_key.is_none(),
            "Ownership Exclusivity: duplicate witness registration inserted for one investigation"
        );
        self.indexes
            .witnesses
            .case_witnesses_by_character
            .entry(record.witness())
            .or_default()
            .insert(id);
        self.indexes
            .witnesses
            .case_witnesses_by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        let investigation = self
            .investigations
            .get_mut(&record.investigation())
            .expect("validated investigation disappeared before witness registration");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        let previous = self.case_witnesses.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate case witness ID inserted"
        );
    }
    pub(crate) fn set_witness_cooperation(
        &mut self,
        case_witness: CaseWitnessId,
        cooperation: WitnessCooperation,
    ) {
        let investigation_id = {
            let record = self
                .case_witnesses
                .get_mut(&case_witness)
                .expect("validated case witness disappeared before cooperation commit");
            record.cooperation = cooperation;
            record.version = record
                .version
                .checked_add(1)
                .expect("case witness version counter exhausted");
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before witness cooperation commit");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
    }
    pub(crate) fn insert_witness_statement(&mut self, record: WitnessStatementRecord) {
        let id = record.id();
        let evidence = record.evidence();
        let case_witness = record.case_witness();
        let witness = self
            .case_witnesses
            .get_mut(&case_witness)
            .expect("validated case witness disappeared before statement commit");
        witness.statements.insert(id);
        witness.version = witness
            .version
            .checked_add(1)
            .expect("case witness version counter exhausted");
        let previous_evidence = self
            .indexes
            .witnesses
            .witness_statement_by_evidence
            .insert(evidence, id);
        debug_assert!(
            previous_evidence.is_none(),
            "Ownership Exclusivity: evidence is linked to multiple witness statements"
        );
        let previous = self.witness_statements.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate witness statement ID inserted"
        );
    }
    pub(crate) fn insert_investigation_work(&mut self, record: InvestigationWorkRecord) {
        let id = record.id();
        let investigation_id = record.investigation();
        debug_assert_eq!(
            record.status(),
            InvestigationWorkStatus::Scheduled,
            "Lifecycle Validity: new investigation work must be scheduled"
        );
        self.indexes
            .work
            .work_by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        self.indexes
            .work
            .work_by_investigator
            .entry(record.investigator())
            .or_default()
            .insert(id);
        self.indexes
            .work
            .scheduled_work_by_due_at
            .entry(record.due_at())
            .or_default()
            .insert(id);
        let previous_focus = self
            .indexes
            .work
            .scheduled_work_by_focus
            .insert((record.investigation(), record.kind(), record.focus()), id);
        debug_assert!(
            previous_focus.is_none(),
            "Ownership Exclusivity: duplicate scheduled investigation focus inserted"
        );
        let previous = self.investigation_work.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate investigation work ID inserted"
        );
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work insertion");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
    }
    pub(crate) fn complete_investigation_work(
        &mut self,
        id: InvestigationWorkId,
        resolution: InvestigationWorkResolution,
    ) {
        let (due_at, focus_key) = {
            let record = self
                .investigation_work
                .get(&id)
                .expect("validated investigation work disappeared before completion");
            (
                record.due_at(),
                (record.investigation(), record.kind(), record.focus()),
            )
        };
        if let Some(ids) = self.indexes.work.scheduled_work_by_due_at.get_mut(&due_at) {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes.work.scheduled_work_by_due_at.remove(&due_at);
            }
        }
        self.indexes.work.scheduled_work_by_focus.remove(&focus_key);
        let investigation_id = {
            let record = self
                .investigation_work
                .get_mut(&id)
                .expect("validated investigation work disappeared before completion");
            record.runtime.status = InvestigationWorkStatus::Completed;
            record.runtime.resolution = Some(resolution);
            record.runtime.version = record
                .runtime
                .version
                .checked_add(1)
                .expect("investigation work version counter exhausted");
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work completion");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
    }
    pub(crate) fn set_investigation_status(
        &mut self,
        investigation_id: InvestigationId,
        status: InvestigationStatus,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before lifecycle commit");
        investigation.status = status;
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
    }
    pub(crate) fn set_investigator_role(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
        role: InvestigatorRole,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing commit");
        investigation.assigned_investigators.insert(investigator);
        match role {
            InvestigatorRole::Lead => investigation.lead_investigator = Some(investigator),
            InvestigatorRole::Investigator => {
                if investigation.lead_investigator == Some(investigator) {
                    investigation.lead_investigator = None;
                }
            }
        }
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.indexes
            .investigations
            .investigations_by_investigator
            .entry(investigator)
            .or_default()
            .insert(investigation_id);
    }
    pub(crate) fn remove_investigator(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing commit");
        let removed = investigation.assigned_investigators.remove(&investigator);
        debug_assert!(
            removed,
            "validated investigator assignment disappeared before commit"
        );
        if investigation.lead_investigator == Some(investigator) {
            investigation.lead_investigator = None;
        }
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        if let Some(investigations) = self
            .indexes
            .investigations
            .investigations_by_investigator
            .get_mut(&investigator)
        {
            investigations.remove(&investigation_id);
            if investigations.is_empty() {
                self.indexes
                    .investigations
                    .investigations_by_investigator
                    .remove(&investigator);
            }
        }
    }
    pub(crate) fn set_jurisdiction(&mut self, record: JurisdictionRecord) {
        let organization = record.organization();
        let previous_neighborhoods = self
            .jurisdictions
            .get(&organization)
            .map(|previous| previous.neighborhoods().iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for neighborhood in previous_neighborhoods {
            if let Some(organizations) = self
                .indexes
                .jurisdictions
                .jurisdictions_by_neighborhood
                .get_mut(&neighborhood)
            {
                organizations.remove(&organization);
                if organizations.is_empty() {
                    self.indexes
                        .jurisdictions
                        .jurisdictions_by_neighborhood
                        .remove(&neighborhood);
                }
            }
        }
        for neighborhood in record.neighborhoods() {
            self.indexes
                .jurisdictions
                .jurisdictions_by_neighborhood
                .entry(*neighborhood)
                .or_default()
                .insert(organization);
        }
        self.jurisdictions.insert(organization, record);
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for investigation in self.investigations.values() {
            if !self
                .indexes
                .investigations
                .by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
            for subject in investigation.subjects() {
                if !self
                    .indexes
                    .investigations
                    .investigations_by_subject
                    .get(subject)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
                {
                    return false;
                }
            }
            for evidence_id in investigation.evidence() {
                if !self
                    .evidence
                    .get(evidence_id)
                    .is_some_and(|record| record.investigation() == investigation.id())
                {
                    return false;
                }
            }
            if investigation
                .lead_investigator()
                .is_some_and(|lead| !investigation.assigned_investigators().contains(&lead))
            {
                return false;
            }
            for investigator in investigation.assigned_investigators() {
                if !self
                    .indexes
                    .investigations
                    .investigations_by_investigator
                    .get(investigator)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
                {
                    return false;
                }
            }
        }
        for (source, ids) in &self.indexes.evidence.evidence_by_source {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.source() == Some(*source))
                {
                    return false;
                }
            }
        }
        for witness in self.case_witnesses.values() {
            if self
                .indexes
                .witnesses
                .case_witness_by_case_character
                .get(&(witness.investigation(), witness.witness()))
                != Some(&witness.id())
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id()))
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
                    .is_some_and(|ids| ids.contains(&witness.id()))
            {
                return false;
            }
            for statement in witness.statements() {
                if !self
                    .witness_statements
                    .get(statement)
                    .is_some_and(|record| record.case_witness() == witness.id())
                {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.witnesses.case_witness_by_case_character {
            if !self
                .case_witnesses
                .get(id)
                .is_some_and(|record| (record.investigation(), record.witness()) == *key)
            {
                return false;
            }
        }
        for (character, ids) in &self.indexes.witnesses.case_witnesses_by_character {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.witness() == *character)
                {
                    return false;
                }
            }
        }
        for (investigation, ids) in &self.indexes.witnesses.case_witnesses_by_investigation {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        for statement in self.witness_statements.values() {
            if !self
                .case_witnesses
                .get(&statement.case_witness())
                .is_some_and(|witness| witness.statements().contains(&statement.id()))
                || self
                    .indexes
                    .witnesses
                    .witness_statement_by_evidence
                    .get(&statement.evidence())
                    != Some(&statement.id())
                || !self.evidence.contains_key(&statement.evidence())
            {
                return false;
            }
        }
        for (evidence, statement) in &self.indexes.witnesses.witness_statement_by_evidence {
            if !self
                .witness_statements
                .get(statement)
                .is_some_and(|record| record.evidence() == *evidence)
            {
                return false;
            }
        }
        for (owner, ids) in &self.indexes.investigations.by_owner {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.owner() == *owner)
                {
                    return false;
                }
            }
        }
        for (subject, ids) in &self.indexes.investigations.investigations_by_subject {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.subjects().contains(subject))
                {
                    return false;
                }
            }
        }
        for evidence in self.evidence.values() {
            if !self
                .investigations
                .get(&evidence.investigation())
                .is_some_and(|investigation| investigation.evidence().contains(&evidence.id()))
            {
                return false;
            }
            if !self
                .indexes
                .evidence
                .evidence_by_subject
                .get(&evidence.subject())
                .is_some_and(|ids| ids.contains(&evidence.id()))
            {
                return false;
            }
            if let Some(origin) = evidence.origin() {
                if !self
                    .indexes
                    .evidence
                    .evidence_by_origin
                    .get(&origin)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
            if let Some(source) = evidence.source() {
                if !self
                    .indexes
                    .evidence
                    .evidence_by_source
                    .get(&source)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
            if !self
                .indexes
                .evidence
                .evidence_by_kind
                .get(&evidence.kind())
                .is_some_and(|ids| ids.contains(&evidence.id()))
            {
                return false;
            }
            for source in evidence.derived_from() {
                if !self
                    .indexes
                    .evidence
                    .derived_evidence_by_source
                    .get(source)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
        }
        for (subject, ids) in &self.indexes.evidence.evidence_by_subject {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.subject() == *subject)
                {
                    return false;
                }
            }
        }
        for (source, ids) in &self.indexes.evidence.derived_evidence_by_source {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.derived_from().contains(source))
                {
                    return false;
                }
            }
        }
        for work in self.investigation_work.values() {
            if !self
                .indexes
                .work
                .work_by_investigation
                .get(&work.investigation())
                .is_some_and(|ids| ids.contains(&work.id()))
                || !self
                    .indexes
                    .work
                    .work_by_investigator
                    .get(&work.investigator())
                    .is_some_and(|ids| ids.contains(&work.id()))
            {
                return false;
            }
            let due_indexed = self
                .indexes
                .work
                .scheduled_work_by_due_at
                .get(&work.due_at())
                .is_some_and(|ids| ids.contains(&work.id()));
            let focus_indexed = self.indexes.work.scheduled_work_by_focus.get(&(
                work.investigation(),
                work.kind(),
                work.focus(),
            )) == Some(&work.id());
            match work.status() {
                InvestigationWorkStatus::Scheduled => {
                    if work.resolution().is_some() || !due_indexed || !focus_indexed {
                        return false;
                    }
                }
                InvestigationWorkStatus::Completed => {
                    if work.resolution().is_none() || due_indexed || focus_indexed {
                        return false;
                    }
                }
            }
        }
        for (investigation, ids) in &self.indexes.work.work_by_investigation {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        for (investigator, ids) in &self.indexes.work.work_by_investigator {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigator() == *investigator)
                {
                    return false;
                }
            }
        }
        for (time, ids) in &self.indexes.work.scheduled_work_by_due_at {
            for id in ids {
                if !self.investigation_work.get(id).is_some_and(|work| {
                    work.status() == InvestigationWorkStatus::Scheduled && work.due_at() == *time
                }) {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.work.scheduled_work_by_focus {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.status() == InvestigationWorkStatus::Scheduled
                    && (work.investigation(), work.kind(), work.focus()) == *key
            }) {
                return false;
            }
        }
        for (kind, ids) in &self.indexes.evidence.evidence_by_kind {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.kind() == *kind)
                {
                    return false;
                }
            }
        }
        for (origin, ids) in &self.indexes.evidence.evidence_by_origin {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.origin() == Some(*origin))
                {
                    return false;
                }
            }
        }
        for (investigator, ids) in &self.indexes.investigations.investigations_by_investigator {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.assigned_investigators().contains(investigator))
                {
                    return false;
                }
            }
        }
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                if !self
                    .indexes
                    .jurisdictions
                    .jurisdictions_by_neighborhood
                    .get(neighborhood)
                    .is_some_and(|organizations| {
                        organizations.contains(&jurisdiction.organization())
                    })
                {
                    return false;
                }
            }
        }
        for (neighborhood, organizations) in
            &self.indexes.jurisdictions.jurisdictions_by_neighborhood
        {
            for organization in organizations {
                if !self
                    .jurisdictions
                    .get(organization)
                    .is_some_and(|record| record.neighborhoods().contains(neighborhood))
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
            "Derived Data Consistency: legal indexes disagree with source records"
        );
        for investigation in self.investigations.values() {
            debug_assert!(
                self.indexes
                    .investigations
                    .by_owner
                    .get(&investigation.owner())
                    .is_some_and(|ids| ids.contains(&investigation.id())),
                "Index Completeness: investigation owner index is missing a case"
            );
            for subject in investigation.subjects() {
                debug_assert!(
                    self.indexes
                        .investigations
                        .investigations_by_subject
                        .get(subject)
                        .is_some_and(|ids| ids.contains(&investigation.id())),
                    "Index Completeness: investigation subject index is missing a case"
                );
            }
            for evidence in investigation.evidence() {
                let record = self
                    .evidence
                    .get(evidence)
                    .expect("Record Reference Validity: investigation references missing evidence");
                debug_assert_eq!(
                    record.investigation(),
                    investigation.id(),
                    "Ownership Exclusivity: evidence belongs to a different investigation"
                );
            }
            if let Some(lead) = investigation.lead_investigator() {
                debug_assert!(
                    investigation.assigned_investigators().contains(&lead),
                    "Derived Data Consistency: investigation lead is not assigned to the case"
                );
            }
            for investigator in investigation.assigned_investigators() {
                debug_assert!(
                    self.indexes
                        .investigations
                        .investigations_by_investigator
                        .get(investigator)
                        .is_some_and(|ids| ids.contains(&investigation.id())),
                    "Index Completeness: investigator reverse index is missing an assigned case"
                );
            }
        }
        for evidence in self.evidence.values() {
            debug_assert!(
                self.indexes
                    .evidence
                    .evidence_by_subject
                    .get(&evidence.subject())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence subject index is missing evidence"
            );
            if let Some(origin) = evidence.origin() {
                debug_assert!(
                    self.indexes
                        .evidence
                        .evidence_by_origin
                        .get(&origin)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence origin index is missing evidence"
                );
            }
            if let Some(source) = evidence.source() {
                debug_assert!(
                    self.indexes
                        .evidence
                        .evidence_by_source
                        .get(&source)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence source index is missing evidence"
                );
            }
            debug_assert!(
                self.indexes
                    .evidence
                    .evidence_by_kind
                    .get(&evidence.kind())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence kind index is missing evidence"
            );
            for source in evidence.derived_from() {
                debug_assert!(
                    self.indexes.evidence.derived_evidence_by_source
                        .get(source)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence provenance reverse index is missing derived evidence"
                );
            }
        }
        for work in self.investigation_work.values() {
            debug_assert!(
                self.indexes
                    .work
                    .work_by_investigation
                    .get(&work.investigation())
                    .is_some_and(|ids| ids.contains(&work.id())),
                "Index Completeness: investigation work case index is missing work"
            );
            debug_assert!(
                self.indexes
                    .work
                    .work_by_investigator
                    .get(&work.investigator())
                    .is_some_and(|ids| ids.contains(&work.id())),
                "Index Completeness: investigation work investigator index is missing work"
            );
            match work.status() {
                InvestigationWorkStatus::Scheduled => {
                    debug_assert!(work.resolution().is_none());
                    debug_assert!(
                        self.indexes.work.scheduled_work_by_due_at
                            .get(&work.due_at())
                            .is_some_and(|ids| ids.contains(&work.id())),
                        "Index Completeness: scheduled investigation work due index is missing work"
                    );
                    debug_assert_eq!(
                        self.indexes.work.scheduled_work_by_focus.get(&(
                            work.investigation(),
                            work.kind(),
                            work.focus(),
                        )),
                        Some(&work.id()),
                        "Index Completeness: scheduled investigation work focus index is missing work"
                    );
                }
                InvestigationWorkStatus::Completed => {
                    debug_assert!(work.resolution().is_some());
                }
            }
        }
        for witness in self.case_witnesses.values() {
            debug_assert_eq!(
                self.indexes
                    .witnesses
                    .case_witness_by_case_character
                    .get(&(witness.investigation(), witness.witness())),
                Some(&witness.id()),
                "Index Completeness: case-witness uniqueness index is missing witness"
            );
            debug_assert!(
                self.indexes
                    .witnesses
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id())),
                "Index Completeness: character witness index is missing case witness"
            );
            debug_assert!(
                self.indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
                    .is_some_and(|ids| ids.contains(&witness.id())),
                "Index Completeness: investigation witness index is missing case witness"
            );
            for statement in witness.statements() {
                debug_assert!(
                    self.witness_statements
                        .get(statement)
                        .is_some_and(|record| record.case_witness() == witness.id()),
                    "Record Reference Validity: case witness references missing or foreign statement"
                );
            }
        }
        for statement in self.witness_statements.values() {
            debug_assert!(
                self.case_witnesses
                    .get(&statement.case_witness())
                    .is_some_and(|witness| witness.statements().contains(&statement.id())),
                "Record Reference Validity: witness statement is not owned by its case witness"
            );
            debug_assert_eq!(
                self.indexes
                    .witnesses
                    .witness_statement_by_evidence
                    .get(&statement.evidence()),
                Some(&statement.id()),
                "Index Completeness: witness statement evidence index is missing statement"
            );
        }
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                debug_assert!(
                    self.indexes.jurisdictions.jurisdictions_by_neighborhood
                        .get(neighborhood)
                        .is_some_and(|organizations| {
                            organizations.contains(&jurisdiction.organization())
                        }),
                    "Index Completeness: legal jurisdiction neighborhood index is missing authority"
                );
            }
        }
    }
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
