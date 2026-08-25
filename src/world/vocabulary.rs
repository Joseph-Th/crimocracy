//! Closed vocabularies for city entities, personnel attributes, organizational policy, and
//! business identity. Every list is exhaustive over its own kind so adding a variant forces
//! a compile error in every consumer that matches or registers kinds.

use crate::core::id::{CharacterId, OrganizationId};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AutonomyLevel {
    Tight,
    Guided,
    Delegated,
    Broad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PolicyKind {
    IndependentRecruitment,
    AssociateLegalSupport,
}

pub const ALL_POLICY_KINDS: [PolicyKind; 2] = [
    PolicyKind::IndependentRecruitment,
    PolicyKind::AssociateLegalSupport,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ApprovalPolicy {
    RequireApproval,
    Delegated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicySetting {
    IndependentRecruitment(ApprovalPolicy),
    AssociateLegalSupport(LegalSupportPolicy),
}

impl PolicySetting {
    pub const fn kind(self) -> PolicyKind {
        match self {
            Self::IndependentRecruitment(_) => PolicyKind::IndependentRecruitment,
            Self::AssociateLegalSupport(_) => PolicyKind::AssociateLegalSupport,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LegalSupportPolicy {
    None,
    CaseByCase,
    Automatic,
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
