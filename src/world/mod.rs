//! Persistent city and organization records; `world_system` owns their canonical mutation paths.

pub mod payroll_execution;
pub mod rating;
pub mod territory_influence;
pub mod vocabulary;
pub mod world_system;

mod stores;

use crate::core::id::{
    BusinessId, BusinessOwnershipChangeId, CharacterId, NeighborhoodId, OrganizationId,
};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use rating::{QualitativeBand, Rating, RatingError};
pub use stores::WorldState;
pub use vocabulary::{
    ALL_BUSINESS_KINDS, ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_POLICY_KINDS, ALL_TRAIT_KINDS,
    ApprovalPolicy, AutonomyLevel, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    DriveKind, LegalSupportPolicy, OrganizationKind, PolicyKind, PolicySetting, TraitKind,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrganizationRecord {
    id: OrganizationId,
    name: String,
    kind: OrganizationKind,
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

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NeighborhoodRecord {
    id: NeighborhoodId,
    name: String,
    profile: NeighborhoodProfile,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodEconomyProfile {
    pub wealth: Rating,
    pub commercial_activity: Rating,
    pub illicit_demand: Rating,
}

/// Only institution attributes with a consuming system are modeled; unread authored ratings
/// would persist forever without ever informing a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodInstitutionProfile {
    pub police_presence: Rating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodProfile {
    pub economy: NeighborhoodEconomyProfile,
    pub institutions: NeighborhoodInstitutionProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusinessRecord {
    id: BusinessId,
    name: String,
    kind: BusinessKind,
    functions: BTreeSet<BusinessFunction>,
    neighborhood: NeighborhoodId,
    owner: BusinessOwner,
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
