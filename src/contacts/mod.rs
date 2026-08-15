//! Persistent institutional contacts and provenance-bearing disclosures into organization knowledge.

pub mod contact_system;

use crate::core::id::{CharacterId, ContactDisclosureId, ContactId, InformationId, OrganizationId};
use crate::core::time::SimTime;
use crate::social::RelationshipDimensions;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContactKind {
    Police,
    Legal,
    Political,
    Press,
    Labor,
    Professional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactStatus {
    Active,
    Terminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactRelationshipSnapshot {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
    version: u32,
}

impl ContactRelationshipSnapshot {
    pub fn from(self) -> CharacterId {
        self.from
    }

    pub fn to(self) -> CharacterId {
        self.to
    }

    pub fn dimensions(self) -> RelationshipDimensions {
        self.dimensions
    }

    pub fn version(self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstitutionalContactRecord {
    id: ContactId,
    sponsor: OrganizationId,
    handler: CharacterId,
    contact: CharacterId,
    institution: OrganizationId,
    kind: ContactKind,
    handler_to_contact: Option<ContactRelationshipSnapshot>,
    contact_to_handler: Option<ContactRelationshipSnapshot>,
    status: ContactStatus,
    established_at: SimTime,
    terminated_at: Option<SimTime>,
    version: u32,
}

impl InstitutionalContactRecord {
    pub fn id(&self) -> ContactId {
        self.id
    }

    pub fn sponsor(&self) -> OrganizationId {
        self.sponsor
    }

    pub fn handler(&self) -> CharacterId {
        self.handler
    }

    pub fn contact(&self) -> CharacterId {
        self.contact
    }

    pub fn institution(&self) -> OrganizationId {
        self.institution
    }

    pub fn kind(&self) -> ContactKind {
        self.kind
    }

    pub fn handler_to_contact(&self) -> Option<ContactRelationshipSnapshot> {
        self.handler_to_contact
    }

    pub fn contact_to_handler(&self) -> Option<ContactRelationshipSnapshot> {
        self.contact_to_handler
    }

    pub fn status(&self) -> ContactStatus {
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
pub struct ContactDisclosureRecord {
    id: ContactDisclosureId,
    contact: ContactId,
    source_information: InformationId,
    disclosed_information: InformationId,
    disclosed_at: SimTime,
}

impl ContactDisclosureRecord {
    pub fn id(&self) -> ContactDisclosureId {
        self.id
    }

    pub fn contact(&self) -> ContactId {
        self.contact
    }

    pub fn source_information(&self) -> InformationId {
        self.source_information
    }

    pub fn disclosed_information(&self) -> InformationId {
        self.disclosed_information
    }

    pub fn disclosed_at(&self) -> SimTime {
        self.disclosed_at
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ContactIndexes {
    by_sponsor: BTreeMap<OrganizationId, BTreeSet<ContactId>>,
    by_handler: BTreeMap<CharacterId, BTreeSet<ContactId>>,
    by_contact: BTreeMap<CharacterId, BTreeSet<ContactId>>,
    by_institution: BTreeMap<OrganizationId, BTreeSet<ContactId>>,
    active_by_sponsor_contact: BTreeMap<(OrganizationId, CharacterId), ContactId>,
    active_by_handler: BTreeMap<CharacterId, BTreeSet<ContactId>>,
    active_by_contact: BTreeMap<CharacterId, BTreeSet<ContactId>>,
    disclosures_by_contact: BTreeMap<ContactId, BTreeSet<ContactDisclosureId>>,
    disclosure_by_source: BTreeMap<(ContactId, InformationId), ContactDisclosureId>,
    disclosure_by_information: BTreeMap<InformationId, ContactDisclosureId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContactState {
    contacts: BTreeMap<ContactId, InstitutionalContactRecord>,
    disclosures: BTreeMap<ContactDisclosureId, ContactDisclosureRecord>,
    indexes: ContactIndexes,
}

impl ContactState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_contact(&self, id: ContactId) -> Option<&InstitutionalContactRecord> {
        self.contacts.get(&id)
    }

    pub fn get_disclosure(&self, id: ContactDisclosureId) -> Option<&ContactDisclosureRecord> {
        self.disclosures.get(&id)
    }

    pub fn contacts_for_sponsor(
        &self,
        sponsor: OrganizationId,
    ) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.indexes
            .by_sponsor
            .get(&sponsor)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contacts.get(id))
    }

    pub fn contacts_for_institution(
        &self,
        institution: OrganizationId,
    ) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.indexes
            .by_institution
            .get(&institution)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contacts.get(id))
    }

    pub fn contacts_for_character(
        &self,
        contact: CharacterId,
    ) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.indexes
            .by_contact
            .get(&contact)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contacts.get(id))
    }

    pub fn disclosures_for_contact(
        &self,
        contact: ContactId,
    ) -> impl Iterator<Item = &ContactDisclosureRecord> {
        self.indexes
            .disclosures_by_contact
            .get(&contact)
            .into_iter()
            .flatten()
            .filter_map(|id| self.disclosures.get(id))
    }

    pub fn disclosure_for_information(
        &self,
        information: InformationId,
    ) -> Option<&ContactDisclosureRecord> {
        self.indexes
            .disclosure_by_information
            .get(&information)
            .and_then(|id| self.disclosures.get(id))
    }

    pub(crate) fn active_contact_for(
        &self,
        sponsor: OrganizationId,
        contact: CharacterId,
    ) -> Option<&InstitutionalContactRecord> {
        self.indexes
            .active_by_sponsor_contact
            .get(&(sponsor, contact))
            .and_then(|id| self.contacts.get(id))
    }

    pub(crate) fn active_contacts_for_handler(
        &self,
        handler: CharacterId,
    ) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.indexes
            .active_by_handler
            .get(&handler)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contacts.get(id))
    }

    pub(crate) fn active_contacts_for_character(
        &self,
        contact: CharacterId,
    ) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.indexes
            .active_by_contact
            .get(&contact)
            .into_iter()
            .flatten()
            .filter_map(|id| self.contacts.get(id))
    }

    pub(crate) fn contacts(&self) -> impl Iterator<Item = &InstitutionalContactRecord> {
        self.contacts.values()
    }

    pub(crate) fn disclosures(&self) -> impl Iterator<Item = &ContactDisclosureRecord> {
        self.disclosures.values()
    }

    pub(crate) fn insert_contact(&mut self, record: InstitutionalContactRecord) {
        let id = record.id();
        self.indexes
            .by_sponsor
            .entry(record.sponsor())
            .or_default()
            .insert(id);
        self.indexes
            .by_handler
            .entry(record.handler())
            .or_default()
            .insert(id);
        self.indexes
            .by_contact
            .entry(record.contact())
            .or_default()
            .insert(id);
        self.indexes
            .by_institution
            .entry(record.institution())
            .or_default()
            .insert(id);
        let previous = self
            .indexes
            .active_by_sponsor_contact
            .insert((record.sponsor(), record.contact()), id);
        debug_assert!(previous.is_none(), "duplicate active institutional contact");
        self.indexes
            .active_by_handler
            .entry(record.handler())
            .or_default()
            .insert(id);
        self.indexes
            .active_by_contact
            .entry(record.contact())
            .or_default()
            .insert(id);
        let previous = self.contacts.insert(id, record);
        debug_assert!(previous.is_none(), "duplicate institutional contact ID");
    }

    pub(crate) fn terminate_contact(&mut self, id: ContactId, terminated_at: SimTime) {
        let (sponsor, handler, contact) = {
            let record = self
                .contacts
                .get_mut(&id)
                .expect("validated contact disappeared before termination commit");
            record.status = ContactStatus::Terminated;
            record.terminated_at = Some(terminated_at);
            record.version = record
                .version
                .checked_add(1)
                .expect("contact version counter exhausted");
            (record.sponsor(), record.handler(), record.contact())
        };
        let removed = self
            .indexes
            .active_by_sponsor_contact
            .remove(&(sponsor, contact));
        debug_assert_eq!(removed, Some(id));
        remove_active_contact_index(&mut self.indexes.active_by_handler, handler, id);
        remove_active_contact_index(&mut self.indexes.active_by_contact, contact, id);
    }

    pub(crate) fn insert_disclosure(&mut self, record: ContactDisclosureRecord) {
        let id = record.id();
        self.indexes
            .disclosures_by_contact
            .entry(record.contact())
            .or_default()
            .insert(id);
        let previous_source = self
            .indexes
            .disclosure_by_source
            .insert((record.contact(), record.source_information()), id);
        debug_assert!(previous_source.is_none());
        let previous_information = self
            .indexes
            .disclosure_by_information
            .insert(record.disclosed_information(), id);
        debug_assert!(previous_information.is_none());
        let previous = self.disclosures.insert(id, record);
        debug_assert!(previous.is_none(), "duplicate contact disclosure ID");
    }

    pub(crate) fn disclosure_from_source(
        &self,
        contact: ContactId,
        source: InformationId,
    ) -> Option<&ContactDisclosureRecord> {
        self.indexes
            .disclosure_by_source
            .get(&(contact, source))
            .and_then(|id| self.disclosures.get(id))
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.contacts.values() {
            let id = record.id();
            if !self
                .indexes
                .by_sponsor
                .get(&record.sponsor())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .by_handler
                    .get(&record.handler())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .by_contact
                    .get(&record.contact())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .by_institution
                    .get(&record.institution())
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            let pair = self
                .indexes
                .active_by_sponsor_contact
                .get(&(record.sponsor(), record.contact()));
            let handler_active = self
                .indexes
                .active_by_handler
                .get(&record.handler())
                .is_some_and(|ids| ids.contains(&id));
            let contact_active = self
                .indexes
                .active_by_contact
                .get(&record.contact())
                .is_some_and(|ids| ids.contains(&id));
            match record.status() {
                ContactStatus::Active
                    if pair != Some(&id) || !handler_active || !contact_active =>
                {
                    return false
                }
                ContactStatus::Terminated
                    if pair == Some(&id) || handler_active || contact_active =>
                {
                    return false
                }
                ContactStatus::Active | ContactStatus::Terminated => {}
            }
        }
        for disclosure in self.disclosures.values() {
            if !self.contacts.contains_key(&disclosure.contact())
                || !self
                    .indexes
                    .disclosures_by_contact
                    .get(&disclosure.contact())
                    .is_some_and(|ids| ids.contains(&disclosure.id()))
                || self
                    .indexes
                    .disclosure_by_source
                    .get(&(disclosure.contact(), disclosure.source_information()))
                    != Some(&disclosure.id())
                || self
                    .indexes
                    .disclosure_by_information
                    .get(&disclosure.disclosed_information())
                    != Some(&disclosure.id())
            {
                return false;
            }
        }
        for (sponsor, ids) in &self.indexes.by_sponsor {
            if ids.iter().any(|id| {
                !self
                    .contacts
                    .get(id)
                    .is_some_and(|record| record.sponsor() == *sponsor)
            }) {
                return false;
            }
        }
        for (handler, ids) in &self.indexes.by_handler {
            if ids.iter().any(|id| {
                !self
                    .contacts
                    .get(id)
                    .is_some_and(|record| record.handler() == *handler)
            }) {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.by_contact {
            if ids.iter().any(|id| {
                !self
                    .contacts
                    .get(id)
                    .is_some_and(|record| record.contact() == *contact)
            }) {
                return false;
            }
        }
        for (institution, ids) in &self.indexes.by_institution {
            if ids.iter().any(|id| {
                !self
                    .contacts
                    .get(id)
                    .is_some_and(|record| record.institution() == *institution)
            }) {
                return false;
            }
        }
        for (key, id) in &self.indexes.active_by_sponsor_contact {
            if !self.contacts.get(id).is_some_and(|record| {
                record.status() == ContactStatus::Active
                    && (record.sponsor(), record.contact()) == *key
            }) {
                return false;
            }
        }
        for (handler, ids) in &self.indexes.active_by_handler {
            if ids.iter().any(|id| {
                !self.contacts.get(id).is_some_and(|record| {
                    record.status() == ContactStatus::Active && record.handler() == *handler
                })
            }) {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.active_by_contact {
            if ids.iter().any(|id| {
                !self.contacts.get(id).is_some_and(|record| {
                    record.status() == ContactStatus::Active && record.contact() == *contact
                })
            }) {
                return false;
            }
        }
        for (information, disclosure) in &self.indexes.disclosure_by_information {
            if !self
                .disclosures
                .get(disclosure)
                .is_some_and(|record| record.disclosed_information() == *information)
            {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.disclosures_by_contact {
            if ids.iter().any(|id| {
                !self
                    .disclosures
                    .get(id)
                    .is_some_and(|record| record.contact() == *contact)
            }) {
                return false;
            }
        }
        for (key, disclosure) in &self.indexes.disclosure_by_source {
            if !self
                .disclosures
                .get(disclosure)
                .is_some_and(|record| (record.contact(), record.source_information()) == *key)
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: contact indexes disagree with source records"
        );
    }
}

fn remove_active_contact_index(
    index: &mut BTreeMap<CharacterId, BTreeSet<ContactId>>,
    character: CharacterId,
    contact: ContactId,
) {
    if let Some(ids) = index.get_mut(&character) {
        ids.remove(&contact);
        if ids.is_empty() {
            index.remove(&character);
        }
    }
}
