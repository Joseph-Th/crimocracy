//! Persistent city and organization records; `world_system` owns their canonical mutation paths.
//!
//! `stores` owns the record collections; `payroll_execution` owns the daily wage pass;
//! `territory_influence` derives district control and validates bilateral
//! standstills, and `territory_conflict` resolves contested criminal pressure
//! through canonical enterprise lifecycle. `rating` and `vocabulary` are read-only.

pub mod character_life;
#[cfg(test)]
mod civic_tests;
pub mod freelance_contracts;
pub(crate) mod payroll_departures;
pub mod payroll_execution;
#[cfg(test)]
mod population_tests;
pub mod rating;
pub mod territory_conflict;
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
    /// Exact organization-level policy history. Version one is the authored default and every
    /// later entry is a real setting change. Persisting the journal makes historical authority
    /// validation independent of how many variants a policy enum happens to have.
    policy_revisions: BTreeMap<PolicyKind, Vec<PolicySetting>>,
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
        self.policy_revisions
            .get(&kind)
            .and_then(|revisions| revisions.last())
            .copied()
    }

    pub(crate) fn policy_version(&self, kind: PolicyKind) -> Option<u32> {
        u32::try_from(self.policy_revisions.get(&kind)?.len()).ok()
    }

    pub(crate) fn policy_revisions(&self, kind: PolicyKind) -> Option<&[PolicySetting]> {
        self.policy_revisions.get(&kind).map(Vec::as_slice)
    }

    pub(crate) fn policy_at_version(
        &self,
        kind: PolicyKind,
        historical_version: u32,
    ) -> Option<PolicySetting> {
        let index = historical_version.checked_sub(1)?;
        self.policy_revisions
            .get(&kind)?
            .get(usize::try_from(index).ok()?)
            .copied()
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

/// Every named person retains their own history of work and civic capacity,
/// independently of organization membership or business-title history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CivicActivity {
    BusinessManagement(BusinessId),
    PolicePatrol(NeighborhoodId),
    DetectiveWork(crate::core::id::InvestigationId),
    Politics(OrganizationId),
    LegalService(OrganizationId),
    Prosecution(OrganizationId),
    LaborAdvocacy(OrganizationId),
    CommercialAdvocacy(OrganizationId),
    Community,
    BusinessShift(BusinessId),
    FreelanceWork(NeighborhoodId),
    Journalism(NeighborhoodId),
    IndependentLegalPractice(NeighborhoodId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CivicProfession {
    BusinessOwner,
    PoliceOfficer,
    Politician,
    Lawyer,
    Prosecutor,
    LaborRepresentative,
    MerchantRepresentative,
    Criminal,
    Resident,
    FreelanceCriminal,
    BusinessEmployee,
    Journalist,
    IndependentLawyer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndependentVocation {
    Resident,
    FreelanceCriminal,
    BusinessEmployee(BusinessId),
    Journalist,
    IndependentLawyer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterLife {
    professional_standing: u8,
    fatigue: u8,
    career_days: u32,
    last_activity: Option<(SimTime, CivicActivity)>,
    home: Option<NeighborhoodId>,
    vocation: IndependentVocation,
}

impl Default for CharacterLife {
    fn default() -> Self {
        Self {
            professional_standing: 50,
            fatigue: 0,
            career_days: 0,
            last_activity: None,
            home: None,
            vocation: IndependentVocation::Resident,
        }
    }
}

impl CharacterLife {
    pub fn professional_standing(self) -> u8 {
        self.professional_standing
    }
    pub fn fatigue(self) -> u8 {
        self.fatigue
    }
    pub fn career_days(self) -> u32 {
        self.career_days
    }
    pub fn last_activity(self) -> Option<(SimTime, CivicActivity)> {
        self.last_activity
    }
    pub fn home(self) -> Option<NeighborhoodId> {
        self.home
    }
    pub fn vocation(self) -> IndependentVocation {
        self.vocation
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterRecord {
    identity: CharacterIdentity,
    membership: CharacterMembership,
    capabilities: CharacterCapabilities,
    disposition: CharacterDisposition,
    runtime: CharacterRuntime,
    life: CharacterLife,
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

    pub fn life(&self) -> CharacterLife {
        self.life
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
