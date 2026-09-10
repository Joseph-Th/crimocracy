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
use crate::core::version::advance_version_preflighted;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CharacterStore {
    records: BTreeMap<CharacterId, CharacterRecord>,
    #[serde(skip)]
    by_organization: BTreeMap<OrganizationId, BTreeSet<CharacterId>>,
    #[serde(skip)]
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
        record.runtime.version = advance_version_preflighted(record.runtime.version);
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
    #[serde(skip)]
    by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<BusinessId>>,
    #[serde(skip)]
    by_organization_owner: BTreeMap<OrganizationId, BTreeSet<BusinessId>>,
    #[serde(skip)]
    by_character_owner: BTreeMap<CharacterId, BTreeSet<BusinessId>>,
    #[serde(skip)]
    by_historical_organization_owner: BTreeMap<OrganizationId, BTreeSet<BusinessId>>,
    ownership_changes: BTreeMap<BusinessOwnershipChangeId, BusinessOwnershipChangeRecord>,
    #[serde(skip)]
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
            advance_version_preflighted(previous_version)
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

    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.characters.by_organization.clear();
        self.characters.by_supervisor.clear();
        for record in self.characters.records.values() {
            if let Some(organization) = record.organization() {
                self.characters
                    .by_organization
                    .entry(organization)
                    .or_default()
                    .insert(record.id());
            }
            if let Some(supervisor) = record.supervisor() {
                self.characters
                    .by_supervisor
                    .entry(supervisor)
                    .or_default()
                    .insert(record.id());
            }
        }

        self.businesses.by_neighborhood.clear();
        self.businesses.by_organization_owner.clear();
        self.businesses.by_character_owner.clear();
        self.businesses.by_historical_organization_owner.clear();
        self.businesses.ownership_change_by_business_version.clear();
        for record in self.businesses.records.values() {
            self.businesses
                .by_neighborhood
                .entry(record.neighborhood())
                .or_default()
                .insert(record.id());
            match record.owner() {
                BusinessOwner::Independent => {}
                BusinessOwner::Organization(organization) => {
                    self.businesses
                        .by_organization_owner
                        .entry(organization)
                        .or_default()
                        .insert(record.id());
                }
                BusinessOwner::Character(character) => {
                    self.businesses
                        .by_character_owner
                        .entry(character)
                        .or_default()
                        .insert(record.id());
                }
            }
        }
        for change in self.businesses.ownership_changes.values() {
            self.businesses.ownership_change_by_business_version.insert(
                (change.business(), change.resulting_business_version()),
                change.id(),
            );
            if let BusinessOwner::Organization(organization) = change.new_owner() {
                self.businesses
                    .by_historical_organization_owner
                    .entry(organization)
                    .or_default()
                    .insert(change.business());
            }
        }
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
            .map(|character_id| {
                self.characters
                    .records
                    .get(character_id)
                    .expect("organization-character index must reference a character")
            })
    }
    pub fn direct_reports(&self, id: CharacterId) -> impl Iterator<Item = &CharacterRecord> {
        self.characters
            .by_supervisor
            .get(&id)
            .into_iter()
            .flatten()
            .map(|character_id| {
                self.characters
                    .records
                    .get(character_id)
                    .expect("supervision index must reference a character")
            })
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
            .map(|business_id| {
                self.businesses
                    .records
                    .get(business_id)
                    .expect("neighborhood-business index must reference a business")
            })
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
            .map(|business_id| {
                self.businesses
                    .records
                    .get(business_id)
                    .expect("historical business-owner index must reference a business")
            })
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
            .map(|business_id| {
                self.businesses
                    .records
                    .get(business_id)
                    .expect("organization business-owner index must reference a business")
            })
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
            .map(|business_id| {
                self.businesses
                    .records
                    .get(business_id)
                    .expect("character business-owner index must reference a business")
            })
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
        (1..=version).map(move |business_version| {
            let change = self
                .businesses
                .ownership_change_by_business_version
                .get(&(business, business_version))
                .expect("business version must have an ownership-change index entry");
            self.businesses
                .ownership_changes
                .get(change)
                .expect("ownership-change index must reference an ownership record")
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
            .map(|id| {
                self.businesses
                    .ownership_changes
                    .get(id)
                    .expect("business ownership-version index must reference a change record")
            })
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
    pub(crate) fn neighborhoods(&self) -> impl Iterator<Item = &NeighborhoodRecord> {
        self.neighborhoods.values()
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
        let kind = setting.kind();
        let version = record
            .policy_versions
            .get_mut(&kind)
            .expect("validated organization must retain every authored policy version");
        *version = advance_version_preflighted(*version);
        record.policies.insert(kind, setting);
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
        self.organizations
            .iter()
            .all(|(id, record)| *id == record.id())
            && self
                .neighborhoods
                .iter()
                .all(|(id, record)| *id == record.id())
            && self.characters_have_consistent_indexes()
            && self.businesses_have_consistent_indexes()
    }

    /// Forward membership plus exact-count agreement proves bidirectional index coherence
    /// for every functional index (each record owns at most one slot per index): matching
    /// entry totals rule out stale, duplicate, or foreign membership without re-walking
    /// each indexed id. Non-functional indexes keep explicit reverse walks.
    fn characters_have_consistent_indexes(&self) -> bool {
        let mut expected_member_entries = 0_usize;
        let mut expected_supervised_entries = 0_usize;
        for (stored_id, record) in &self.characters.records {
            if *stored_id != record.id() {
                return false;
            }
            if let Some(organization) = record.organization() {
                if !self
                    .characters
                    .by_organization
                    .get(&organization)
                    .is_some_and(|ids| ids.contains(&record.id()))
                {
                    return false;
                }
                expected_member_entries += 1;
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
                expected_supervised_entries += 1;
            }
        }
        if self
            .characters
            .by_organization
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            != expected_member_entries
        {
            return false;
        }
        self.characters
            .by_supervisor
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            == expected_supervised_entries
    }

    /// Business location/ownership indexes plus a full ownership-chain replay: every version
    /// of every business must trace through its change history to the current owner, and the
    /// historical-owner index must equal exactly the pairs the replay derives.
    fn businesses_have_consistent_indexes(&self) -> bool {
        let mut expected_neighborhood_entries = 0_usize;
        let mut expected_org_owner_entries = 0_usize;
        let mut expected_character_owner_entries = 0_usize;
        // Historical org-ownership pairs seen while replaying each business's ownership
        // chain; compared against the historical-owner index at the end.
        let mut expected_historical_pairs: BTreeSet<(OrganizationId, BusinessId)> = BTreeSet::new();
        for (stored_id, record) in &self.businesses.records {
            if *stored_id != record.id() {
                return false;
            }
            if !self
                .businesses
                .by_neighborhood
                .get(&record.neighborhood())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            expected_neighborhood_entries += 1;
            match record.owner() {
                BusinessOwner::Independent => {}
                BusinessOwner::Organization(organization) => {
                    if !self
                        .businesses
                        .by_organization_owner
                        .get(&organization)
                        .is_some_and(|ids| ids.contains(&record.id()))
                    {
                        return false;
                    }
                    expected_org_owner_entries += 1;
                }
                BusinessOwner::Character(character) => {
                    if !self
                        .businesses
                        .by_character_owner
                        .get(&character)
                        .is_some_and(|ids| ids.contains(&record.id()))
                    {
                        return false;
                    }
                    expected_character_owner_entries += 1;
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
                if let BusinessOwner::Organization(organization) = change.new_owner() {
                    expected_historical_pairs.insert((organization, record.id()));
                }
                previous_owner = Some(change.new_owner());
                previous_time = Some(change.changed_at());
            }
            if previous_owner != Some(record.owner()) {
                return false;
            }
        }
        if self
            .businesses
            .ownership_changes
            .iter()
            .any(|(id, change)| *id != change.id())
        {
            return false;
        }
        if self
            .businesses
            .by_neighborhood
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            != expected_neighborhood_entries
        {
            return false;
        }
        if self
            .businesses
            .by_organization_owner
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            != expected_org_owner_entries
        {
            return false;
        }
        if self
            .businesses
            .by_character_owner
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            != expected_character_owner_entries
        {
            return false;
        }
        // The historical-owner index is not a function of the current record, so its keys
        // must equal exactly the ownership pairs the replay derived.
        let indexed_historical_entries: usize = self
            .businesses
            .by_historical_organization_owner
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_historical_entries != expected_historical_pairs.len() {
            return false;
        }
        for (organization, ids) in &self.businesses.by_historical_organization_owner {
            for id in ids {
                if !expected_historical_pairs.contains(&(*organization, *id)) {
                    return false;
                }
            }
        }
        // Every ownership change is indexed under its own (business, version) key; entry
        // counts agreeing prove no foreign key exists.
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
        if self.businesses.ownership_change_by_business_version.len()
            != self.businesses.ownership_changes.len()
        {
            return false;
        }
        true
    }
}
