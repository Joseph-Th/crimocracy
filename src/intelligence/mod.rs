//! Provenance-bearing information records; `intelligence_system` controls knowledge insertion.

pub mod intelligence_system;

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, IdKeyedBounds, InformationId, InvestigationId, OrganizationId,
};
use crate::core::time::{DAY_MINUTES_U16, SimTime};
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

/// Typed semantic facts carried by player-visible information. These are deliberately sparse:
/// only facts that production consumers must reason about belong here. Display summaries remain
/// presentation and are never parsed back into state.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InformationSignal {
    CaseActivity(CaseActivitySignal),
    LegalPersonStatus(LegalPersonStatusSignal),
    PersonnelPresence {
        characters: BTreeSet<CharacterId>,
    },
    PatrolPattern {
        intervals: BTreeSet<PatrolIntervalSignal>,
    },
}

impl InformationSignal {
    pub(crate) fn is_compatible(&self, topic: InformationTopic, subject: EntityRef) -> bool {
        match self {
            Self::CaseActivity(_) => {
                matches!(topic, InformationTopic::LegalActivity)
                    && matches!(
                        subject,
                        EntityRef::Organization(_)
                            | EntityRef::Operation(_)
                            | EntityRef::Investigation(_)
                            | EntityRef::Enterprise(_)
                    )
            }
            Self::LegalPersonStatus(_) => {
                matches!(topic, InformationTopic::LegalActivity)
                    && matches!(subject, EntityRef::Character(_))
            }
            Self::PersonnelPresence { characters } => {
                !characters.is_empty()
                    && matches!(topic, InformationTopic::Personnel)
                    && matches!(subject, EntityRef::Organization(_))
            }
            Self::PatrolPattern { intervals } => {
                !intervals.is_empty()
                    && intervals.iter().all(|interval| interval.is_valid())
                    && matches!(topic, InformationTopic::PoliceActivity)
                    && matches!(subject, EntityRef::Neighborhood(_))
            }
        }
    }

    pub(crate) fn referenced_entities(&self) -> Vec<EntityRef> {
        match self {
            Self::CaseActivity(_) => Vec::new(),
            Self::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness { investigation }) => {
                vec![EntityRef::Investigation(*investigation)]
            }
            Self::LegalPersonStatus(LegalPersonStatusSignal::Detained { .. }) => Vec::new(),
            Self::PersonnelPresence { characters } => characters
                .iter()
                .copied()
                .map(EntityRef::Character)
                .collect(),
            Self::PatrolPattern { .. } => Vec::new(),
        }
    }
}

/// One non-wrapping interval from an approximate recurring patrol pattern disclosed by an
/// information record. End minute 1440 represents midnight at the end of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PatrolIntervalSignal {
    start_minute: u16,
    end_minute: u16,
}

impl PatrolIntervalSignal {
    pub fn try_new(start_minute: u16, end_minute: u16) -> Option<Self> {
        (start_minute < end_minute
            && start_minute < DAY_MINUTES_U16
            && end_minute <= DAY_MINUTES_U16)
            .then_some(Self {
                start_minute,
                end_minute,
            })
    }

    pub fn start_minute(self) -> u16 {
        self.start_minute
    }

    pub fn end_minute(self) -> u16 {
        self.end_minute
    }

    const fn is_valid(self) -> bool {
        self.start_minute < self.end_minute
            && self.start_minute < DAY_MINUTES_U16
            && self.end_minute <= DAY_MINUTES_U16
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CaseActivitySignal {
    Active,
    Shelved,
    Closed,
}

/// A concrete legal role or custody fact about a person, bound to the legal episode the holder
/// learned about. These values do not assert current world truth by themselves; consumers that
/// act on them must still validate the corresponding current legal state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LegalPersonStatusSignal {
    CaseWitness { investigation: InvestigationId },
    Detained { arrest: ArrestId },
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
    signal: Option<InformationSignal>,
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
    pub fn signal(&self) -> Option<&InformationSignal> {
        self.assessment.signal.as_ref()
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
    #[serde(skip)]
    by_holder: BTreeMap<KnowledgeHolder, BTreeSet<InformationId>>,
    #[serde(skip)]
    by_holder_topic: BTreeMap<(KnowledgeHolder, InformationTopic), BTreeSet<InformationId>>,
    #[serde(skip)]
    by_holder_subject: BTreeMap<(KnowledgeHolder, EntityRef), BTreeSet<InformationId>>,
    #[serde(skip)]
    by_subject: BTreeMap<EntityRef, BTreeSet<InformationId>>,
    #[serde(skip)]
    derived_by_source: BTreeMap<InformationId, BTreeSet<InformationId>>,
    #[serde(skip)]
    internal_transfer_by_source_recipient:
        BTreeMap<(InformationId, KnowledgeHolder), InformationId>,
}

impl IntelligenceState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.by_holder.clear();
        self.by_holder_topic.clear();
        self.by_holder_subject.clear();
        self.by_subject.clear();
        self.derived_by_source.clear();
        self.internal_transfer_by_source_recipient.clear();
        for record in self.records.values() {
            let id = record.id();
            self.by_holder
                .entry(record.holder())
                .or_default()
                .insert(id);
            self.by_holder_topic
                .entry((record.holder(), record.topic()))
                .or_default()
                .insert(id);
            self.by_holder_subject
                .entry((record.holder(), record.subject()))
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
            if record.source_kind() == InformationSourceKind::InternalReport
                && record.derived_from().len() == 1
            {
                let source = *record
                    .derived_from()
                    .iter()
                    .next()
                    .expect("single-source internal report must have one source");
                self.internal_transfer_by_source_recipient
                    .insert((source, record.holder()), id);
            }
        }
    }
    pub fn get_information(&self, id: InformationId) -> Option<&InformationRecord> {
        self.records.get(&id)
    }
    pub fn information_for_holder(
        &self,
        holder: KnowledgeHolder,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.by_holder.get(&holder).into_iter().flatten().map(|id| {
            self.records
                .get(id)
                .expect("information holder index must reference information")
        })
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
            .map(|id| {
                self.records
                    .get(id)
                    .expect("information holder-topic index must reference information")
            })
    }
    pub(crate) fn information_for_holder_subject(
        &self,
        holder: KnowledgeHolder,
        subject: EntityRef,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.by_holder_subject
            .get(&(holder, subject))
            .into_iter()
            .flatten()
            .map(|id| {
                self.records
                    .get(id)
                    .expect("information holder-subject index must reference information")
            })
    }
    pub(crate) fn internal_transfer_for(
        &self,
        source: InformationId,
        recipient: KnowledgeHolder,
    ) -> Option<&InformationRecord> {
        self.internal_transfer_by_source_recipient
            .get(&(source, recipient))
            .map(|id| {
                self.records
                    .get(id)
                    .expect("internal transfer index must reference information")
            })
    }
    pub fn information_derived_from(
        &self,
        source: InformationId,
    ) -> impl Iterator<Item = &InformationRecord> {
        self.derived_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .map(|id| {
                self.records
                    .get(id)
                    .expect("information lineage index must reference information")
            })
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
        self.by_holder_subject
            .entry((record.holder(), record.subject()))
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
        if record.source_kind() == InformationSourceKind::InternalReport {
            debug_assert_eq!(record.derived_from().len(), 1);
            if let Some(source) = record.derived_from().iter().next().copied() {
                let previous = self
                    .internal_transfer_by_source_recipient
                    .insert((source, record.holder()), id);
                debug_assert!(
                    previous.is_none(),
                    "duplicate internal information transfer inserted"
                );
            }
        }
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate information ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        // Forward direction plus exact-count agreement replaces per-entry reverse walks:
        // every record is verified present under its own key(s), and because ids are unique
        // and each record occupies at most one slot per key, the indexed entry totals can
        // equal the expected totals only when no stale, duplicate, or foreign entry exists.
        let mut expected_holder_entries = 0_usize;
        let mut expected_holder_topic_entries = 0_usize;
        let mut expected_holder_subject_entries = 0_usize;
        let mut expected_subject_entries = 0_usize;
        let mut expected_source_entries = 0_usize;
        for (stored_id, record) in &self.records {
            if *stored_id != record.id() {
                return false;
            }
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
                .by_holder_subject
                .get(&(record.holder(), record.subject()))
                .is_some_and(|ids| ids.contains(&record.id()))
                || !self
                    .by_subject
                    .get(&record.subject())
                    .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            let derived_from = record.derived_from();
            for source in derived_from {
                if !self
                    .derived_by_source
                    .get(source)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
            expected_holder_entries += 1;
            expected_holder_topic_entries += 1;
            expected_holder_subject_entries += 1;
            expected_subject_entries += 1;
            expected_source_entries += derived_from.len();
            if record.source_kind() == InformationSourceKind::InternalReport {
                if derived_from.len() != 1 {
                    return false;
                }
                let source = *derived_from
                    .iter()
                    .next()
                    .expect("single-source internal report must have one source");
                if self
                    .internal_transfer_by_source_recipient
                    .get(&(source, record.holder()))
                    != Some(&record.id())
                {
                    return false;
                }
            }
        }
        let indexed_holder_entries: usize = self.by_holder.values().map(BTreeSet::len).sum();
        if indexed_holder_entries != expected_holder_entries {
            return false;
        }
        let indexed_holder_topic_entries: usize =
            self.by_holder_topic.values().map(BTreeSet::len).sum();
        if indexed_holder_topic_entries != expected_holder_topic_entries {
            return false;
        }
        let indexed_holder_subject_entries: usize =
            self.by_holder_subject.values().map(BTreeSet::len).sum();
        if indexed_holder_subject_entries != expected_holder_subject_entries {
            return false;
        }
        let indexed_subject_entries: usize = self.by_subject.values().map(BTreeSet::len).sum();
        if indexed_subject_entries != expected_subject_entries {
            return false;
        }
        // Provenance sources must themselves exist, and each reverse entry must name a real
        // derivation edge; this index is not a partition of the records (a source may have
        // no derivations), so it keeps an explicit reverse walk.
        let indexed_source_entries: usize =
            self.derived_by_source.values().map(BTreeSet::len).sum();
        if indexed_source_entries != expected_source_entries {
            return false;
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
        let expected_internal_transfers = self
            .records
            .values()
            .filter(|record| record.source_kind() == InformationSourceKind::InternalReport)
            .count();
        if self.internal_transfer_by_source_recipient.len() != expected_internal_transfers {
            return false;
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
