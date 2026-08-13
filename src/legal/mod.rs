//! Specific investigations and evidence graphs; `investigation_system` owns case/evidence transactions.

pub mod investigation_system;

use crate::core::entity::EntityRef;
use crate::core::id::{EvidenceId, InvestigationId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationStatus {
    Active,
    Suspended,
    Closed,
    Referred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
pub enum Admissibility {
    Unknown,
    Inadmissible,
    Disputed,
    Admissible,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvestigationRecord {
    id: InvestigationId,
    owner: OrganizationId,
    title: String,
    status: InvestigationStatus,
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
pub struct EvidenceRecord {
    id: EvidenceId,
    investigation: InvestigationId,
    custodian: OrganizationId,
    subject: EntityRef,
    kind: EvidenceKind,
    strength: EvidenceStrength,
    admissibility: Admissibility,
    discovered_at: SimTime,
}

impl EvidenceRecord {
    pub fn id(&self) -> EvidenceId {
        self.id
    }
    pub fn investigation(&self) -> InvestigationId {
        self.investigation
    }
    pub fn custodian(&self) -> OrganizationId {
        self.custodian
    }
    pub fn subject(&self) -> EntityRef {
        self.subject
    }
    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }
    pub fn strength(&self) -> EvidenceStrength {
        self.strength
    }
    pub fn admissibility(&self) -> Admissibility {
        self.admissibility
    }
    pub fn discovered_at(&self) -> SimTime {
        self.discovered_at
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LegalState {
    investigations: BTreeMap<InvestigationId, InvestigationRecord>,
    evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    evidence_by_subject: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
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
    pub(crate) fn investigations(&self) -> impl Iterator<Item = &InvestigationRecord> {
        self.investigations.values()
    }
    pub(crate) fn all_evidence(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence.values()
    }
    pub(crate) fn insert_investigation(&mut self, record: InvestigationRecord) {
        self.by_owner
            .entry(record.owner())
            .or_default()
            .insert(record.id());
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
        self.evidence_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.id());
        let previous = self.evidence.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate evidence ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for investigation in self.investigations.values() {
            if !self
                .by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
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
        }
        for (owner, ids) in &self.by_owner {
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
        for evidence in self.evidence.values() {
            if !self
                .investigations
                .get(&evidence.investigation())
                .is_some_and(|investigation| investigation.evidence().contains(&evidence.id()))
            {
                return false;
            }
            if !self
                .evidence_by_subject
                .get(&evidence.subject())
                .is_some_and(|ids| ids.contains(&evidence.id()))
            {
                return false;
            }
        }
        for (subject, ids) in &self.evidence_by_subject {
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
        true
    }
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: legal indexes disagree with source records"
        );
        for investigation in self.investigations.values() {
            debug_assert!(
                self.by_owner
                    .get(&investigation.owner())
                    .is_some_and(|ids| ids.contains(&investigation.id())),
                "Index Completeness: investigation owner index is missing a case"
            );
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
        }
        for evidence in self.evidence.values() {
            debug_assert!(
                self.evidence_by_subject
                    .get(&evidence.subject())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence subject index is missing evidence"
            );
        }
    }
}

pub struct InvestigationDraft {
    pub owner: OrganizationId,
    pub title: String,
    pub subjects: BTreeSet<EntityRef>,
}
pub struct EvidenceDraft {
    pub investigation: InvestigationId,
    pub custodian: OrganizationId,
    pub subject: EntityRef,
    pub kind: EvidenceKind,
    pub strength: EvidenceStrength,
    pub admissibility: Admissibility,
    pub discovered_at: SimTime,
}
