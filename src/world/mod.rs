//! Persistent city and organization records; `world_system` owns their canonical mutation paths.

pub mod world_system;

use crate::core::id::{
    BusinessId, BusinessOwnershipChangeId, CharacterId, NeighborhoodId, OrganizationId,
};
use crate::core::time::SimTime;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Lifecycle {
    Active,
    Inactive,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OrganizationKind {
    Criminal,
    LawEnforcement,
    LegalAuthority,
    LegalServices,
    Prosecutor,
    Political,
    Press,
    Labor,
    Civic,
    Commercial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CapabilityKind {
    Violence,
    Intimidation,
    Stealth,
    Burglary,
    Driving,
    Surveillance,
    Investigation,
    Accounting,
    Negotiation,
    Management,
    PoliticalInfluence,
    LegalKnowledge,
    SocialAccess,
}

pub const ALL_CAPABILITY_KINDS: [CapabilityKind; 13] = [
    CapabilityKind::Violence,
    CapabilityKind::Intimidation,
    CapabilityKind::Stealth,
    CapabilityKind::Burglary,
    CapabilityKind::Driving,
    CapabilityKind::Surveillance,
    CapabilityKind::Investigation,
    CapabilityKind::Accounting,
    CapabilityKind::Negotiation,
    CapabilityKind::Management,
    CapabilityKind::PoliticalInfluence,
    CapabilityKind::LegalKnowledge,
    CapabilityKind::SocialAccess,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TraitKind {
    Cautious,
    Impulsive,
    Greedy,
    Proud,
    Patient,
    Cruel,
    Charismatic,
    Vindictive,
    Secretive,
    Ambitious,
    LoyalToFamily,
    EasilyFrightened,
}

pub const ALL_TRAIT_KINDS: [TraitKind; 12] = [
    TraitKind::Cautious,
    TraitKind::Impulsive,
    TraitKind::Greedy,
    TraitKind::Proud,
    TraitKind::Patient,
    TraitKind::Cruel,
    TraitKind::Charismatic,
    TraitKind::Vindictive,
    TraitKind::Secretive,
    TraitKind::Ambitious,
    TraitKind::LoyalToFamily,
    TraitKind::EasilyFrightened,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DriveKind {
    Money,
    Status,
    Safety,
    Respect,
    Revenge,
    FamilySecurity,
    PoliticalAdvancement,
    Independence,
    IdeologicalCause,
}

pub const ALL_DRIVE_KINDS: [DriveKind; 9] = [
    DriveKind::Money,
    DriveKind::Status,
    DriveKind::Safety,
    DriveKind::Respect,
    DriveKind::Revenge,
    DriveKind::FamilySecurity,
    DriveKind::PoliticalAdvancement,
    DriveKind::Independence,
    DriveKind::IdeologicalCause,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Rating(u8);

impl Rating {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 100;

    pub fn try_new(value: u8) -> Result<Self, RatingError> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(RatingError { value })
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    pub const fn qualitative_band(self) -> QualitativeBand {
        match self.0 {
            0..=19 => QualitativeBand::Poor,
            20..=44 => QualitativeBand::Competent,
            45..=69 => QualitativeBand::Skilled,
            70..=89 => QualitativeBand::Excellent,
            90..=100 => QualitativeBand::Exceptional,
            _ => unreachable!(),
        }
    }
}

impl<'de> Deserialize<'de> for Rating {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("rating {value} is outside the inclusive range 0..=100")]
pub struct RatingError {
    value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualitativeBand {
    Poor,
    Competent,
    Skilled,
    Excellent,
    Exceptional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AutonomyLevel {
    Tight,
    Guided,
    Delegated,
    Broad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PolicyKind {
    CollectionForce,
    PatrolBribery,
    IndependentRecruitment,
    CasualtyResponse,
    AssociateLegalSupport,
}

pub const ALL_POLICY_KINDS: [PolicyKind; 5] = [
    PolicyKind::CollectionForce,
    PolicyKind::PatrolBribery,
    PolicyKind::IndependentRecruitment,
    PolicyKind::CasualtyResponse,
    PolicyKind::AssociateLegalSupport,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ForcePolicy {
    None,
    ThreatsOnly,
    NonLethal,
    LethalIfNecessary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ApprovalPolicy {
    RequireApproval,
    WithinBudget,
    Delegated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CasualtyPolicy {
    ContinueWithinPlan,
    RequestDecision,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LegalSupportPolicy {
    None,
    CaseByCase,
    Automatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicySetting {
    CollectionForce(ForcePolicy),
    PatrolBribery(ApprovalPolicy),
    IndependentRecruitment(ApprovalPolicy),
    CasualtyResponse(CasualtyPolicy),
    AssociateLegalSupport(LegalSupportPolicy),
}

impl PolicySetting {
    pub const fn kind(self) -> PolicyKind {
        match self {
            Self::CollectionForce(_) => PolicyKind::CollectionForce,
            Self::PatrolBribery(_) => PolicyKind::PatrolBribery,
            Self::IndependentRecruitment(_) => PolicyKind::IndependentRecruitment,
            Self::CasualtyResponse(_) => PolicyKind::CasualtyResponse,
            Self::AssociateLegalSupport(_) => PolicyKind::AssociateLegalSupport,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrganizationRecord {
    id: OrganizationId,
    name: String,
    kind: OrganizationKind,
    lifecycle: Lifecycle,
    policies: BTreeMap<PolicyKind, PolicySetting>,
}

impl OrganizationRecord {
    pub fn id(&self) -> OrganizationId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> OrganizationKind {
        self.kind
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn policy(&self, kind: PolicyKind) -> Option<PolicySetting> {
        self.policies.get(&kind).copied()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CharacterIdentity {
    id: CharacterId,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CharacterMembership {
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    autonomy: AutonomyLevel,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CharacterCapabilities {
    ratings: BTreeMap<CapabilityKind, Rating>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CharacterDisposition {
    traits: BTreeSet<TraitKind>,
    drives: BTreeMap<DriveKind, Rating>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CharacterRuntime {
    lifecycle: Lifecycle,
    version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterRecord {
    identity: CharacterIdentity,
    membership: CharacterMembership,
    capabilities: CharacterCapabilities,
    disposition: CharacterDisposition,
    runtime: CharacterRuntime,
}

impl CharacterRecord {
    pub fn id(&self) -> CharacterId {
        self.identity.id
    }

    pub fn name(&self) -> &str {
        &self.identity.name
    }

    pub fn organization(&self) -> Option<OrganizationId> {
        self.membership.organization
    }

    pub fn supervisor(&self) -> Option<CharacterId> {
        self.membership.supervisor
    }

    pub fn autonomy(&self) -> AutonomyLevel {
        self.membership.autonomy
    }

    pub fn capability(&self, kind: CapabilityKind) -> Option<Rating> {
        self.capabilities.ratings.get(&kind).copied()
    }

    pub fn has_trait(&self, kind: TraitKind) -> bool {
        self.disposition.traits.contains(&kind)
    }

    pub fn drive(&self, kind: DriveKind) -> Option<Rating> {
        self.disposition.drives.get(&kind).copied()
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.runtime.lifecycle
    }

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeighborhoodRecord {
    id: NeighborhoodId,
    name: String,
    profile: NeighborhoodProfile,
    lifecycle: Lifecycle,
}

impl NeighborhoodRecord {
    pub fn id(&self) -> NeighborhoodId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn profile(&self) -> NeighborhoodProfile {
        self.profile
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodEconomyProfile {
    pub wealth: Rating,
    pub commercial_activity: Rating,
    pub illicit_demand: Rating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodInstitutionProfile {
    pub police_presence: Rating,
    pub political_influence: Rating,
    pub social_cohesion: Rating,
    pub visible_violence_tolerance: Rating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodProfile {
    pub economy: NeighborhoodEconomyProfile,
    pub institutions: NeighborhoodInstitutionProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BusinessKind {
    Retail,
    Hospitality,
    Automotive,
    Transportation,
    Warehouse,
    ProfessionalServices,
}

pub const ALL_BUSINESS_KINDS: [BusinessKind; 6] = [
    BusinessKind::Retail,
    BusinessKind::Hospitality,
    BusinessKind::Automotive,
    BusinessKind::Transportation,
    BusinessKind::Warehouse,
    BusinessKind::ProfessionalServices,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BusinessFunction {
    CashIntensive,
    VehicleFleet,
    Warehousing,
    MeetingSpace,
    CustomerAccess,
    ResaleMarket,
    UnionAccess,
    DistributionInfrastructure,
    ProfessionalRecords,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BusinessOwner {
    Independent,
    Organization(OrganizationId),
    Character(CharacterId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusinessRecord {
    id: BusinessId,
    name: String,
    kind: BusinessKind,
    functions: BTreeSet<BusinessFunction>,
    neighborhood: NeighborhoodId,
    owner: BusinessOwner,
    lifecycle: Lifecycle,
    version: u32,
}

impl BusinessRecord {
    pub fn id(&self) -> BusinessId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> BusinessKind {
        self.kind
    }

    pub fn functions(&self) -> &BTreeSet<BusinessFunction> {
        &self.functions
    }

    pub fn has_function(&self, function: BusinessFunction) -> bool {
        self.functions.contains(&function)
    }

    pub fn neighborhood(&self) -> NeighborhoodId {
        self.neighborhood
    }

    pub fn owner(&self) -> BusinessOwner {
        self.owner
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusinessOwnershipChangeRecord {
    id: BusinessOwnershipChangeId,
    business: BusinessId,
    previous_owner: Option<BusinessOwner>,
    new_owner: BusinessOwner,
    changed_at: SimTime,
    resulting_business_version: u32,
}

impl BusinessOwnershipChangeRecord {
    pub fn id(&self) -> BusinessOwnershipChangeId {
        self.id
    }

    pub fn business(&self) -> BusinessId {
        self.business
    }

    pub fn previous_owner(&self) -> Option<BusinessOwner> {
        self.previous_owner
    }

    pub fn new_owner(&self) -> BusinessOwner {
        self.new_owner
    }

    pub fn changed_at(&self) -> SimTime {
        self.changed_at
    }

    pub fn resulting_business_version(&self) -> u32 {
        self.resulting_business_version
    }
}

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
            Self::remove_index_entry(&mut self.by_organization, old, id);
        }
        if let Some(old) = old_supervisor {
            Self::remove_index_entry(&mut self.by_supervisor, old, id);
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

    fn remove_index_entry<K: Ord + Copy>(
        index: &mut BTreeMap<K, BTreeSet<CharacterId>>,
        key: K,
        id: CharacterId,
    ) {
        if let Some(ids) = index.get_mut(&key) {
            ids.remove(&id);
            if ids.is_empty() {
                index.remove(&key);
            }
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
    by_historical_character_owner: BTreeMap<CharacterId, BTreeSet<BusinessId>>,
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
            BusinessOwner::Character(character) => {
                self.by_historical_character_owner
                    .entry(character)
                    .or_default()
                    .insert(business);
            }
        }
    }

    fn remove_owner_index(&mut self, business: BusinessId, owner: BusinessOwner) {
        match owner {
            BusinessOwner::Independent => {}
            BusinessOwner::Organization(organization) => {
                Self::remove_business_index(&mut self.by_organization_owner, organization, business)
            }
            BusinessOwner::Character(character) => {
                Self::remove_business_index(&mut self.by_character_owner, character, business)
            }
        }
    }

    fn remove_business_index<K: Ord + Copy>(
        index: &mut BTreeMap<K, BTreeSet<BusinessId>>,
        key: K,
        business: BusinessId,
    ) {
        if let Some(ids) = index.get_mut(&key) {
            ids.remove(&business);
            if ids.is_empty() {
                index.remove(&key);
            }
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

    pub fn get_business_ownership_change(
        &self,
        id: BusinessOwnershipChangeId,
    ) -> Option<&BusinessOwnershipChangeRecord> {
        self.businesses.ownership_changes.get(&id)
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

    pub fn businesses_ever_owned_by_character(
        &self,
        id: CharacterId,
    ) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses
            .by_historical_character_owner
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

    pub fn business_was_owned_during(
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

    pub(crate) fn neighborhoods(&self) -> impl Iterator<Item = &NeighborhoodRecord> {
        self.neighborhoods.values()
    }

    pub(crate) fn businesses(&self) -> impl Iterator<Item = &BusinessRecord> {
        self.businesses.records.values()
    }

    pub(crate) fn business_ownership_changes(
        &self,
    ) -> impl Iterator<Item = &BusinessOwnershipChangeRecord> {
        self.businesses.ownership_changes.values()
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
                    BusinessOwner::Character(character) => {
                        if !self
                            .businesses
                            .by_historical_character_owner
                            .get(&character)
                            .is_some_and(|ids| ids.contains(&record.id()))
                        {
                            return false;
                        }
                    }
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
        for (character, ids) in &self.businesses.by_historical_character_owner {
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
                            change.new_owner() == BusinessOwner::Character(*character)
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

    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: world indexes disagree with source records"
        );
        for record in self.characters.records.values() {
            if let Some(organization) = record.organization() {
                debug_assert!(
                    self.characters
                        .by_organization
                        .get(&organization)
                        .is_some_and(|ids| ids.contains(&record.id())),
                    "Index Completeness: character organization index is missing a member"
                );
            }
            if let Some(supervisor) = record.supervisor() {
                debug_assert!(
                    self.characters
                        .by_supervisor
                        .get(&supervisor)
                        .is_some_and(|ids| ids.contains(&record.id())),
                    "Index Completeness: character supervisor index is missing a report"
                );
            }
        }
        for (organization, ids) in &self.characters.by_organization {
            for id in ids {
                let record = self.characters.records.get(id).expect(
                    "Index Completeness: character organization index points to missing record",
                );
                debug_assert_eq!(
                    record.organization(),
                    Some(*organization),
                    "Derived Data Consistency: character organization index disagrees with record"
                );
            }
        }
        for (supervisor, ids) in &self.characters.by_supervisor {
            for id in ids {
                let record = self
                    .characters
                    .records
                    .get(id)
                    .expect("Index Completeness: supervisor index points to missing record");
                debug_assert_eq!(
                    record.supervisor(),
                    Some(*supervisor),
                    "Derived Data Consistency: supervisor index disagrees with record"
                );
            }
        }
        for record in self.businesses.records.values() {
            debug_assert!(
                self.businesses
                    .by_neighborhood
                    .get(&record.neighborhood())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: business neighborhood index is missing a business"
            );
            if let BusinessOwner::Organization(organization) = record.owner() {
                debug_assert!(
                    self.businesses
                        .by_organization_owner
                        .get(&organization)
                        .is_some_and(|ids| ids.contains(&record.id())),
                    "Index Completeness: business owner index is missing a business"
                );
            }
            if let BusinessOwner::Character(character) = record.owner() {
                debug_assert!(
                    self.businesses
                        .by_character_owner
                        .get(&character)
                        .is_some_and(|ids| ids.contains(&record.id())),
                    "Index Completeness: business character-owner index is missing a business"
                );
            }
        }
    }
}

pub struct OrganizationDraft {
    pub name: String,
    pub kind: OrganizationKind,
}

pub struct NeighborhoodDraft {
    pub name: String,
    pub profile: NeighborhoodProfile,
}

pub struct CharacterDraft {
    pub name: String,
    pub organization: Option<OrganizationId>,
    pub supervisor: Option<CharacterId>,
    pub autonomy: AutonomyLevel,
    pub capabilities: BTreeMap<CapabilityKind, Rating>,
    pub traits: BTreeSet<TraitKind>,
    pub drives: BTreeMap<DriveKind, Rating>,
}

pub struct BusinessDraft {
    pub name: String,
    pub kind: BusinessKind,
    pub functions: BTreeSet<BusinessFunction>,
    pub neighborhood: NeighborhoodId,
    pub owner: BusinessOwner,
}
