//! Owned record collections and the aggregate `WorldState` they back; indexes here are
//! derived views maintained only through `world_system` mutations.

use super::{
    BusinessOwner, BusinessOwnershipChangeRecord, BusinessRecord, CharacterRecord,
    NeighborhoodRecord, OrganizationRecord, PolicySetting,
};
use crate::core::id::{
    BusinessId, BusinessOwnershipChangeId, CharacterId, IdKeyedBounds, NeighborhoodId,
    OrganizationId,
};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CharacterStore {
    records: BTreeMap<CharacterId, CharacterRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<CharacterId>>,
    by_supervisor: BTreeMap<CharacterId, BTreeSet<CharacterId>>,
}
impl CharacterStore {
    fn insert(&mut self, record: CharacterRecord) {
        let id = record.id();
        if let Some(organization) = record.organization() {
            self.by_organization
                .entry(organization)
                .or_default()
                .insert(id);
        }
        if let Some(supervisor) = record.supervisor() {
            self.by_supervisor.entry(supervisor).or_default().insert(id);
        }
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate character ID inserted"
        );
    }
    fn reassign(
        &mut self,
        id: CharacterId,
        organization: Option<OrganizationId>,
        supervisor: Option<CharacterId>,
    ) {
        let record = self
            .records
            .get_mut(&id)
            .expect("validated character disappeared before reassign commit");
        let old_organization = record.membership.organization;
        let old_supervisor = record.membership.supervisor;
        if let Some(old) = old_organization {
            prune_index_entry(&mut self.by_organization, old, id);
        }
        if let Some(old) = old_supervisor {
            prune_index_entry(&mut self.by_supervisor, old, id);
        }
        record.membership.organization = organization;
        record.membership.supervisor = supervisor;
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("character version counter exhausted");
        if let Some(new) = organization {
            self.by_organization.entry(new).or_default().insert(id);
        }
        if let Some(new) = supervisor {
            self.by_supervisor.entry(new).or_default().insert(id);
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BusinessStore {
    records: BTreeMap<BusinessId, BusinessRecord>,
    by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<BusinessId>>,
    by_organization_owner: BTreeMap<OrganizationId, BTreeSet<BusinessId>>,
    by_character_owner: BTreeMap<CharacterId, BTreeSet<BusinessId>>,
    by_historical_organization_owner: BTreeMap<OrganizationId, BTreeSet<BusinessId>>,
    ownership_changes: BTreeMap<BusinessOwnershipChangeId, BusinessOwnershipChangeRecord>,
    ownership_change_by_business_version: BTreeMap<(BusinessId, u32), BusinessOwnershipChangeId>,
}
impl BusinessStore {
    fn insert(&mut self, record: BusinessRecord, initial_ownership: BusinessOwnershipChangeRecord) {
        let id = record.id();
        self.by_neighborhood
            .entry(record.neighborhood())
            .or_default()
            .insert(id);
        self.add_owner_index(id, record.owner());
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate business ID inserted"
        );
        self.insert_ownership_change(initial_ownership);
    }
    fn transfer_ownership(&mut self, change: BusinessOwnershipChangeRecord) {
        let business = change.business();
        let (previous_owner, previous_version) = {
            let record = self
                .records
                .get(&business)
                .expect("validated business disappeared before ownership commit");
            (record.owner(), record.version())
        };
        debug_assert_eq!(change.previous_owner(), Some(previous_owner));
        debug_assert_eq!(
            change.resulting_business_version(),
            previous_version
                .checked_add(1)
                .expect("business version counter exhausted")
        );
        self.remove_owner_index(business, previous_owner);
        let record = self
            .records
            .get_mut(&business)
            .expect("validated business disappeared before ownership commit");
        record.owner = change.new_owner();
        record.version = change.resulting_business_version();
        self.add_owner_index(business, change.new_owner());
        self.insert_ownership_change(change);
    }
    fn insert_ownership_change(&mut self, change: BusinessOwnershipChangeRecord) {
        let key = (change.business(), change.resulting_business_version());
        self.add_historical_owner_index(change.business(), change.new_owner());
        let previous_version = self
            .ownership_change_by_business_version
            .insert(key, change.id());
        debug_assert!(
            previous_version.is_none(),
            "Index Uniqueness: duplicate business ownership version inserted"
        );
        let previous = self.ownership_changes.insert(change.id(), change);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate business ownership change ID inserted"
        );
    }
    fn add_owner_index(&mut self, business: BusinessId, owner: BusinessOwner) {
        match owner {
            BusinessOwner::Independent => {}
            BusinessOwner::Organization(organization) => {
                self.by_organization_owner
                    .entry(organization)
                    .or_default()
                    .insert(business);
            }
            BusinessOwner::Character(character) => {
                self.by_character_owner
                    .entry(character)
                    .or_default()
                    .insert(business);
            }
        }
    }
    fn add_historical_owner_index(&mut self, business: BusinessId, owner: BusinessOwner) {
        match owner {
            BusinessOwner::Independent => {}
            BusinessOwner::Organization(organization) => {
                self.by_historical_organization_owner
                    .entry(organization)
                    .or_default()
                    .insert(business);
            }
            // Historical character ownership stays derivable from `ownership_changes`;
            // only the organization projection has a production reader (business reporting).
            BusinessOwner::Character(_) => {}
        }
    }
    fn remove_owner_index(&mut self, business: BusinessId, owner: BusinessOwner) {
        match owner {
            BusinessOwner::Independent => {}
            BusinessOwner::Organization(organization) => {
                prune_index_entry(&mut self.by_organization_owner, organization, business)
            }
            BusinessOwner::Character(character) => {
                prune_index_entry(&mut self.by_character_owner, character, business)
            }
        }
    }
}
/// Removes one entity from an owned-set index, dropping the key entirely when its set empties.
fn prune_index_entry<K: Ord + Copy, V: Ord + Copy>(
    index: &mut BTreeMap<K, BTreeSet<V>>,
    key: K,
    value: V,
) {
    if let Some(ids) = index.get_mut(&key) {
        ids.remove(&value);
        if ids.is_empty() {
            index.remove(&key);
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorldState {
    organizations: BTreeMap<OrganizationId, OrganizationRecord>,
    characters: CharacterStore,
    neighborhoods: BTreeMap<NeighborhoodId, NeighborhoodRecord>,
    businesses: BusinessStore,
}
impl WorldState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn get_organization(&self, id: OrganizationId) -> Option<&OrganizationRecord> {
        self.organizations.get(&id)
    }
    pub fn get_character(&self, id: CharacterId) -> Option<&CharacterRecord> {
        self.characters.records.get(&id)
    }
    pub fn get_neighborhood(&self, id: NeighborhoodId) -> Option<&NeighborhoodRecord> {
        self.neighborhoods.get(&id)
    }
    pub fn get_business(&self, id: BusinessId) -> Option<&BusinessRecord> {
        self.businesses.records.get(&id)
    }
    pub(crate) fn organization_id_bounds(&self) -> Option<(u32, u32)> {
        self.organizations.id_bounds()
    }
    pub(crate) fn character_id_bounds(&self) -> Option<(u32, u32)> {
        self.characters.records.id_bounds()
    }
    pub(crate) fn neighborhood_id_bounds(&self) -> Option<(u32, u32)> {
        self.neighborhoods.id_bounds()
    }
    pub(crate) fn business_id_bounds(&self) -> Option<(u32, u32)> {
        self.businesses.records.id_bounds()
    }
    pub(crate) fn ownership_change_id_bounds(&self) -> Option<(u32, u32)> {
        self.businesses.ownership_changes.id_bounds()
    }
    pub fn characters_in_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &CharacterRecord> {
        self.characters
            .by_organization
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|character_id| self.characters.records.get(character_id))
    }
    pub fn direct_reports(&self, id: CharacterId) -> impl Iterator<Item = &CharacterRecord> {
        self.characters
            .by_supervisor
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|character_id| self.characters.records.get(character_id))
    }
    pub fn businesses_in_neighborhood(
        &self,
        id: NeighborhoodId,
    ) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses
            .by_neighborhood
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|business_id| self.businesses.records.get(business_id))
    }
    pub fn businesses_ever_owned_by_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses
            .by_historical_organization_owner
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|business_id| self.businesses.records.get(business_id))
    }
    pub fn businesses_owned_by_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses
            .by_organization_owner
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|business_id| self.businesses.records.get(business_id))
    }
    pub fn businesses_owned_by_character(
        &self,
        id: CharacterId,
    ) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses
            .by_character_owner
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|business_id| self.businesses.records.get(business_id))
    }
    pub fn business_ownership_history(
        &self,
        business: BusinessId,
    ) -> impl Iterator<Item = &BusinessOwnershipChangeRecord> {
        let version = self
            .businesses
            .records
            .get(&business)
            .map_or(0, BusinessRecord::version);
        (1..=version).filter_map(move |business_version| {
            self.businesses
                .ownership_change_by_business_version
                .get(&(business, business_version))
                .and_then(|id| self.businesses.ownership_changes.get(id))
        })
    }
    pub fn get_business_ownership_change_for_version(
        &self,
        business: BusinessId,
        version: u32,
    ) -> Option<&BusinessOwnershipChangeRecord> {
        self.businesses
            .ownership_change_by_business_version
            .get(&(business, version))
            .and_then(|id| self.businesses.ownership_changes.get(id))
    }
    pub fn business_owner_at(&self, business: BusinessId, at: SimTime) -> Option<BusinessOwner> {
        self.business_ownership_history(business)
            .filter(|change| change.changed_at() <= at)
            .max_by_key(|change| (change.changed_at(), change.resulting_business_version()))
            .map(BusinessOwnershipChangeRecord::new_owner)
    }
    pub fn has_business_owner_during(
        &self,
        business: BusinessId,
        owner: BusinessOwner,
        period_start: SimTime,
        period_end: SimTime,
    ) -> bool {
        if period_start > period_end {
            return false;
        }
        let mut history = self.business_ownership_history(business).peekable();
        while let Some(change) = history.next() {
            let ownership_end = history.peek().map(|next| next.changed_at());
            if change.new_owner() == owner
                && change.changed_at() <= period_end
                && ownership_end.is_none_or(|end| end > period_start)
            {
                return true;
            }
        }
        false
    }
    pub(crate) fn organizations(&self) -> impl Iterator<Item = &OrganizationRecord> {
        self.organizations.values()
    }
    pub(crate) fn characters(&self) -> impl Iterator<Item = &CharacterRecord> {
        self.characters.records.values()
    }
    pub(crate) fn businesses(&self) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses.records.values()
    }
    pub(crate) fn insert_organization(&mut self, record: OrganizationRecord) {
        let previous = self.organizations.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate organization ID inserted"
        );
    }
    pub(crate) fn insert_character(&mut self, record: CharacterRecord) {
        self.characters.insert(record);
    }
    pub(crate) fn insert_neighborhood(&mut self, record: NeighborhoodRecord) {
        let previous = self.neighborhoods.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate neighborhood ID inserted"
        );
    }
    pub(crate) fn insert_business(
        &mut self,
        record: BusinessRecord,
        initial_ownership: BusinessOwnershipChangeRecord,
    ) {
        self.businesses.insert(record, initial_ownership);
    }
    pub(crate) fn transfer_business_ownership(&mut self, change: BusinessOwnershipChangeRecord) {
        self.businesses.transfer_ownership(change);
    }
    pub(crate) fn set_policy(&mut self, id: OrganizationId, setting: PolicySetting) {
        let record = self
            .organizations
            .get_mut(&id)
            .expect("validated organization disappeared before policy commit");
        record.policies.insert(setting.kind(), setting);
    }
    pub(crate) fn reassign_character(
        &mut self,
        id: CharacterId,
        organization: Option<OrganizationId>,
        supervisor: Option<CharacterId>,
    ) {
        self.characters.reassign(id, organization, supervisor);
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.characters.records.values() {
            if let Some(organization) = record.organization() {
                if !self
                    .characters
                    .by_organization
                    .get(&organization)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
            if let Some(supervisor) = record.supervisor() {
                if !self
                    .characters
                    .by_supervisor
                    .get(&supervisor)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
        }
        for (organization, ids) in &self.characters.by_organization {
            for id in ids {
                if !self
                    .characters
                    .records
                    .get(id)
                    .is_some_and(|record| record.organization() == Some(*organization))
                {
                    return false;
                }
            }
        }
        for (supervisor, ids) in &self.characters.by_supervisor {
            for id in ids {
                if !self
                    .characters
                    .records
                    .get(id)
                    .is_some_and(|record| record.supervisor() == Some(*supervisor))
                {
                    return false;
                }
            }
        }
        for record in self.businesses.records.values() {
            if !self
                .businesses
                .by_neighborhood
                .get(&record.neighborhood())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            if let BusinessOwner::Organization(organization) = record.owner() {
                if !self
                    .businesses
                    .by_organization_owner
                    .get(&organization)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
            if let BusinessOwner::Character(character) = record.owner() {
                if !self
                    .businesses
                    .by_character_owner
                    .get(&character)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
            }
            if record.version() == 0 {
                return false;
            }
            let mut previous_owner = None;
            let mut previous_time = None;
            for version in 1..=record.version() {
                let Some(change_id) = self
                    .businesses
                    .ownership_change_by_business_version
                    .get(&(record.id(), version))
                else {
                    return false;
                };
                let Some(change) = self.businesses.ownership_changes.get(change_id) else {
                    return false;
                };
                if change.business() != record.id()
                    || change.resulting_business_version() != version
                    || (version == 1 && change.previous_owner().is_some())
                    || (version > 1 && change.previous_owner() != previous_owner)
                    || change.previous_owner() == Some(change.new_owner())
                    || previous_time.is_some_and(|time| change.changed_at() < time)
                {
                    return false;
                }
                match change.new_owner() {
                    BusinessOwner::Independent => {}
                    BusinessOwner::Organization(organization) => {
                        if !self
                            .businesses
                            .by_historical_organization_owner
                            .get(&organization)
                            .is_some_and(|ids| ids.contains(&record.id()))
                        {
                            return false;
                        }
                    }
                    // Historical character ownership is derivable from `ownership_changes`
                    // and carries no maintained projection.
                    BusinessOwner::Character(_) => {}
                }
                previous_owner = Some(change.new_owner());
                previous_time = Some(change.changed_at());
            }
            if previous_owner != Some(record.owner()) {
                return false;
            }
        }
        for (neighborhood, ids) in &self.businesses.by_neighborhood {
            for id in ids {
                if !self
                    .businesses
                    .records
                    .get(id)
                    .is_some_and(|record| record.neighborhood() == *neighborhood)
                {
                    return false;
                }
            }
        }
        for (organization, ids) in &self.businesses.by_organization_owner {
            for id in ids {
                if !self.businesses.records.get(id).is_some_and(|record| {
                    record.owner() == BusinessOwner::Organization(*organization)
                }) {
                    return false;
                }
            }
        }
        for (character, ids) in &self.businesses.by_character_owner {
            for id in ids {
                if !self
                    .businesses
                    .records
                    .get(id)
                    .is_some_and(|record| record.owner() == BusinessOwner::Character(*character))
                {
                    return false;
                }
            }
        }
        for (organization, ids) in &self.businesses.by_historical_organization_owner {
            for id in ids {
                let Some(record) = self.businesses.records.get(id) else {
                    return false;
                };
                let found = (1..=record.version()).any(|version| {
                    self.businesses
                        .ownership_change_by_business_version
                        .get(&(*id, version))
                        .and_then(|change_id| self.businesses.ownership_changes.get(change_id))
                        .is_some_and(|change| {
                            change.new_owner() == BusinessOwner::Organization(*organization)
                        })
                });
                if !found {
                    return false;
                }
            }
        }
        for (key, id) in &self.businesses.ownership_change_by_business_version {
            if !self
                .businesses
                .ownership_changes
                .get(id)
                .is_some_and(|change| {
                    (change.business(), change.resulting_business_version()) == *key
                })
            {
                return false;
            }
        }
        for change in self.businesses.ownership_changes.values() {
            if self
                .businesses
                .ownership_change_by_business_version
                .get(&(change.business(), change.resulting_business_version()))
                != Some(&change.id())
                || !self.businesses.records.contains_key(&change.business())
            {
                return false;
            }
        }
        true
    }
}
