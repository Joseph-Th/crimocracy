//! Specific investigations and evidence graphs; `investigation_system` owns case/evidence transactions.

pub mod investigation_system;
pub mod jurisdiction_system;

use crate::core::entity::EntityRef;
use crate::core::id::{EvidenceId, InvestigationId, NeighborhoodId, OrganizationId};
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
    origin: Option<EntityRef>,
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
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
    pub fn origin(&self) -> Option<EntityRef> {
        self.origin
    }
    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }
    pub fn strength(&self) -> EvidenceStrength {
        self.strength
    }
    pub fn reliability(&self) -> EvidenceReliability {
        self.reliability
    }
    pub fn admissibility(&self) -> Admissibility {
        self.admissibility
    }
    pub fn discovered_at(&self) -> SimTime {
        self.discovered_at
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
pub struct LegalState {
    investigations: BTreeMap<InvestigationId, InvestigationRecord>,
    evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    jurisdictions: BTreeMap<OrganizationId, JurisdictionRecord>,
    by_owner: BTreeMap<OrganizationId, BTreeSet<InvestigationId>>,
    investigations_by_subject: BTreeMap<EntityRef, BTreeSet<InvestigationId>>,
    evidence_by_subject: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    evidence_by_origin: BTreeMap<EntityRef, BTreeSet<EvidenceId>>,
    evidence_by_kind: BTreeMap<EvidenceKind, BTreeSet<EvidenceId>>,
    jurisdictions_by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<OrganizationId>>,
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
    pub fn get_jurisdiction(&self, organization: OrganizationId) -> Option<&JurisdictionRecord> {
        self.jurisdictions.get(&organization)
    }
    pub fn jurisdictions_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &JurisdictionRecord> {
        self.jurisdictions_by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .filter_map(|organization| self.jurisdictions.get(organization))
    }
    pub fn evidence_from_origin(&self, origin: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence_by_origin
            .get(&origin)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn investigations_for_subject(
        &self,
        subject: EntityRef,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.investigations_by_subject
            .get(&subject)
            .into_iter()
            .flatten()
            .filter_map(|id| self.investigations.get(id))
    }
    pub fn evidence_of_kind(&self, kind: EvidenceKind) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence_by_kind
            .get(&kind)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub(crate) fn investigations(&self) -> impl Iterator<Item = &InvestigationRecord> {
        self.investigations.values()
    }
    pub(crate) fn all_evidence(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence.values()
    }
    pub(crate) fn jurisdictions(&self) -> impl Iterator<Item = &JurisdictionRecord> {
        self.jurisdictions.values()
    }
    pub(crate) fn insert_investigation(&mut self, record: InvestigationRecord) {
        self.by_owner
            .entry(record.owner())
            .or_default()
            .insert(record.id());
        for subject in record.subjects() {
            self.investigations_by_subject
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
        self.investigations_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.investigation());
        self.evidence_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.id());
        if let Some(origin) = record.origin() {
            self.evidence_by_origin
                .entry(origin)
                .or_default()
                .insert(record.id());
        }
        self.evidence_by_kind
            .entry(record.kind())
            .or_default()
            .insert(record.id());
        let previous = self.evidence.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate evidence ID inserted"
        );
    }
    pub(crate) fn set_jurisdiction(&mut self, record: JurisdictionRecord) {
        let organization = record.organization();
        let previous_neighborhoods = self
            .jurisdictions
            .get(&organization)
            .map(|previous| previous.neighborhoods().iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for neighborhood in previous_neighborhoods {
            if let Some(organizations) = self.jurisdictions_by_neighborhood.get_mut(&neighborhood) {
                organizations.remove(&organization);
                if organizations.is_empty() {
                    self.jurisdictions_by_neighborhood.remove(&neighborhood);
                }
            }
        }
        for neighborhood in record.neighborhoods() {
            self.jurisdictions_by_neighborhood
                .entry(*neighborhood)
                .or_default()
                .insert(organization);
        }
        self.jurisdictions.insert(organization, record);
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
            for subject in investigation.subjects() {
                if !self
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
        for (subject, ids) in &self.investigations_by_subject {
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
                .evidence_by_subject
                .get(&evidence.subject())
                .is_some_and(|ids| ids.contains(&evidence.id()))
            {
                return false;
            }
            if let Some(origin) = evidence.origin() {
                if !self
                    .evidence_by_origin
                    .get(&origin)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
            if !self
                .evidence_by_kind
                .get(&evidence.kind())
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
        for (kind, ids) in &self.evidence_by_kind {
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
        for (origin, ids) in &self.evidence_by_origin {
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
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                if !self
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
        for (neighborhood, organizations) in &self.jurisdictions_by_neighborhood {
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
                self.by_owner
                    .get(&investigation.owner())
                    .is_some_and(|ids| ids.contains(&investigation.id())),
                "Index Completeness: investigation owner index is missing a case"
            );
            for subject in investigation.subjects() {
                debug_assert!(
                    self.investigations_by_subject
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
        }
        for evidence in self.evidence.values() {
            debug_assert!(
                self.evidence_by_subject
                    .get(&evidence.subject())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence subject index is missing evidence"
            );
            if let Some(origin) = evidence.origin() {
                debug_assert!(
                    self.evidence_by_origin
                        .get(&origin)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence origin index is missing evidence"
                );
            }
            debug_assert!(
                self.evidence_by_kind
                    .get(&evidence.kind())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence kind index is missing evidence"
            );
        }
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                debug_assert!(
                    self.jurisdictions_by_neighborhood
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
