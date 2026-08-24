//! Provenance-bearing information records; `intelligence_system` controls knowledge insertion.

pub mod intelligence_system;

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, IdKeyedBounds, InformationId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnowledgeHolder {
    Character(CharacterId),
    Organization(OrganizationId),
}

impl KnowledgeHolder {
    pub const fn entity(self) -> EntityRef {
        match self {
            Self::Character(id) => EntityRef::Character(id),
            Self::Organization(id) => EntityRef::Organization(id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformationSourceKind {
    DirectObservation,
    Informant,
    PoliceContact,
    PoliticalContact,
    ProfessionalContact,
    Press,
    Lawyer,
    Accountant,
    Surveillance,
    StreetRumor,
    AfterAction,
    InternalReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InformationTopic {
    General,
    TargetSecurity,
    Personnel,
    Schedule,
    PoliceActivity,
    Route,
    FinancialPerformance,
    Relationship,
    LegalActivity,
    MarketAccess,
    OperationalOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reliability {
    Unknown,
    Unreliable,
    Mixed,
    GenerallyReliable,
    DirectAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Specificity {
    Vague,
    General,
    Specific,
    Precise,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InformationSource {
    holder: KnowledgeHolder,
    source_kind: InformationSourceKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InformationSubject {
    topic: InformationTopic,
    source_entity: Option<EntityRef>,
    subject: EntityRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InformationChronology {
    observed_at: SimTime,
    recorded_at: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InformationAssessment {
    reliability: Reliability,
    specificity: Specificity,
    derived_from: BTreeSet<InformationId>,
    summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InformationRecord {
    id: InformationId,
    source: InformationSource,
    subject: InformationSubject,
    chronology: InformationChronology,
    assessment: InformationAssessment,
}

impl InformationRecord {
    pub fn id(&self) -> InformationId {
        self.id
    }
    pub fn holder(&self) -> KnowledgeHolder {
        self.source.holder
    }
    pub fn source_kind(&self) -> InformationSourceKind {
        self.source.source_kind
    }
    pub fn topic(&self) -> InformationTopic {
        self.subject.topic
    }
    pub fn source_entity(&self) -> Option<EntityRef> {
        self.subject.source_entity
    }
    pub fn subject(&self) -> EntityRef {
        self.subject.subject
    }
    pub fn observed_at(&self) -> SimTime {
        self.chronology.observed_at
    }
    pub fn recorded_at(&self) -> SimTime {
        self.chronology.recorded_at
    }
    pub fn reliability(&self) -> Reliability {
        self.assessment.reliability
    }
    pub fn specificity(&self) -> Specificity {
        self.assessment.specificity
    }
    pub fn derived_from(&self) -> &BTreeSet<InformationId> {
        &self.assessment.derived_from
    }
    pub fn summary(&self) -> &str {
        &self.assessment.summary
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntelligenceState {
    records: BTreeMap<InformationId, InformationRecord>,
    by_holder: BTreeMap<KnowledgeHolder, BTreeSet<InformationId>>,
    by_holder_topic: BTreeMap<(KnowledgeHolder, InformationTopic), BTreeSet<InformationId>>,
    by_subject: BTreeMap<EntityRef, BTreeSet<InformationId>>,
    derived_by_source: BTreeMap<InformationId, BTreeSet<InformationId>>,
}

impl IntelligenceState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_information(&self, id: InformationId) -> Option<&InformationRecord> {
        self.records.get(&id)
    }
    pub fn information_for_holder(
        &self,
        holder: KnowledgeHolder,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.by_holder
            .get(&holder)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub fn information_for_holder_by_topic(
        &self,
        holder: KnowledgeHolder,
        topic: InformationTopic,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.by_holder_topic
            .get(&(holder, topic))
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub fn information_derived_from(
        &self,
        source: InformationId,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.derived_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub(crate) fn information(&self) -> impl Iterator<Item = &InformationRecord> {
        self.records.values()
    }
    pub(crate) fn information_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }
    pub(crate) fn insert(&mut self, record: InformationRecord) {
        let id = record.id();
        self.by_holder
            .entry(record.holder())
            .or_default()
            .insert(id);
        self.by_holder_topic
            .entry((record.holder(), record.topic()))
            .or_default()
            .insert(id);
        self.by_subject
            .entry(record.subject())
            .or_default()
            .insert(id);
        for source in record.derived_from() {
            self.derived_by_source
                .entry(*source)
                .or_default()
                .insert(id);
        }
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate information ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            if !self
                .by_holder
                .get(&record.holder())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            if !self
                .by_holder_topic
                .get(&(record.holder(), record.topic()))
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            if !self
                .by_subject
                .get(&record.subject())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            for source in record.derived_from() {
                if !self
                    .derived_by_source
                    .get(source)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
        }
        for ((holder, topic), ids) in &self.by_holder_topic {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.holder() == *holder && record.topic() == *topic)
                {
                    return false;
                }
            }
        }
        for (holder, ids) in &self.by_holder {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.holder() == *holder)
                {
                    return false;
                }
            }
        }
        for (subject, ids) in &self.by_subject {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.subject() == *subject)
                {
                    return false;
                }
            }
        }
        for (source, ids) in &self.derived_by_source {
            if !self.records.contains_key(source) {
                return false;
            }
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.derived_from().contains(source))
                {
                    return false;
                }
            }
        }
        true
    }
}

pub struct InformationDraft {
    pub holder: KnowledgeHolder,
    pub source_kind: InformationSourceKind,
    pub topic: InformationTopic,
    pub source_entity: Option<EntityRef>,
    pub subject: EntityRef,
    pub observed_at: SimTime,
    pub reliability: Reliability,
    pub specificity: Specificity,
    pub summary: String,
}

pub struct InformationTransferDraft {
    pub source: InformationId,
    pub recipient: KnowledgeHolder,
}
