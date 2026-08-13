//! Provenance-bearing information records; `intelligence_system` controls knowledge insertion.

pub mod intelligence_system;

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, InformationId, OrganizationId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnowledgeHolder {
    Character(CharacterId),
    Organization(OrganizationId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformationSourceKind {
    DirectObservation,
    Informant,
    PoliceContact,
    Press,
    Lawyer,
    Accountant,
    Surveillance,
    StreetRumor,
    Intercept,
    AfterAction,
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
pub struct InformationRecord {
    id: InformationId,
    holder: KnowledgeHolder,
    source_kind: InformationSourceKind,
    source_entity: Option<EntityRef>,
    subject: EntityRef,
    observed_at: SimTime,
    recorded_at: SimTime,
    reliability: Reliability,
    specificity: Specificity,
    summary: String,
}

impl InformationRecord {
    pub fn id(&self) -> InformationId {
        self.id
    }
    pub fn holder(&self) -> KnowledgeHolder {
        self.holder
    }
    pub fn source_kind(&self) -> InformationSourceKind {
        self.source_kind
    }
    pub fn source_entity(&self) -> Option<EntityRef> {
        self.source_entity
    }
    pub fn subject(&self) -> EntityRef {
        self.subject
    }
    pub fn observed_at(&self) -> SimTime {
        self.observed_at
    }
    pub fn recorded_at(&self) -> SimTime {
        self.recorded_at
    }
    pub fn reliability(&self) -> Reliability {
        self.reliability
    }
    pub fn specificity(&self) -> Specificity {
        self.specificity
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IntelligenceState {
    records: BTreeMap<InformationId, InformationRecord>,
    by_holder: BTreeMap<KnowledgeHolder, BTreeSet<InformationId>>,
    by_subject: BTreeMap<EntityRef, BTreeSet<InformationId>>,
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
    pub fn information_about(
        &self,
        subject: EntityRef,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.by_subject
            .get(&subject)
            .into_iter()
            .flatten()
            .filter_map(|id| self.records.get(id))
    }
    pub(crate) fn information(&self) -> impl Iterator<Item = &InformationRecord> {
        self.records.values()
    }
    pub(crate) fn insert(&mut self, record: InformationRecord) {
        let id = record.id();
        self.by_holder
            .entry(record.holder())
            .or_default()
            .insert(id);
        self.by_subject
            .entry(record.subject())
            .or_default()
            .insert(id);
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
                .by_subject
                .get(&record.subject())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
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
        true
    }
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: intelligence indexes disagree with source records"
        );
        for record in self.records.values() {
            debug_assert!(
                self.by_holder
                    .get(&record.holder())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: information holder index is missing a record"
            );
            debug_assert!(
                self.by_subject
                    .get(&record.subject())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: information subject index is missing a record"
            );
        }
    }
}

pub struct InformationDraft {
    pub holder: KnowledgeHolder,
    pub source_kind: InformationSourceKind,
    pub source_entity: Option<EntityRef>,
    pub subject: EntityRef,
    pub observed_at: SimTime,
    pub reliability: Reliability,
    pub specificity: Specificity,
    pub summary: String,
}
