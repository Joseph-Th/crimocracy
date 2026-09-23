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
    Negotiation,
    Management,
    LegalKnowledge,
    SocialAccess,
}

pub const ALL_CAPABILITY_KINDS: [CapabilityKind; 11] = [
    CapabilityKind::Violence,
    CapabilityKind::Intimidation,
    CapabilityKind::Stealth,
    CapabilityKind::Burglary,
    CapabilityKind::Driving,
    CapabilityKind::Surveillance,
    CapabilityKind::Investigation,
    CapabilityKind::Negotiation,
    CapabilityKind::Management,
    CapabilityKind::LegalKnowledge,
    CapabilityKind::SocialAccess,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TraitKind {
    Cautious,
    Impulsive,
    Greedy,
    Proud,
    Charismatic,
    Vindictive,
    Secretive,
    Ambitious,
    LoyalToFamily,
    EasilyFrightened,
}

pub const ALL_TRAIT_KINDS: [TraitKind; 10] = [
    TraitKind::Cautious,
    TraitKind::Impulsive,
    TraitKind::Greedy,
    TraitKind::Proud,
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
    FamilySecurity,
    Independence,
}

pub const ALL_DRIVE_KINDS: [DriveKind; 6] = [
    DriveKind::Money,
    DriveKind::Status,
    DriveKind::Safety,
    DriveKind::Respect,
    DriveKind::FamilySecurity,
    DriveKind::Independence,
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
    Brewery,
    Nightclub,
    Construction,
    Wholesale,
    Pawnshop,
    CoinMachineDistribution,
    Lodging,
    AthleticClub,
    Laundry,
    Printing,
    FinancialServices,
    GarmentFactory,
    Stevedoring,
    NewsService,
}

pub const ALL_BUSINESS_KINDS: [BusinessKind; 20] = [
    BusinessKind::Retail,
    BusinessKind::Hospitality,
    BusinessKind::Automotive,
    BusinessKind::Transportation,
    BusinessKind::Warehouse,
    BusinessKind::ProfessionalServices,
    BusinessKind::Brewery,
    BusinessKind::Nightclub,
    BusinessKind::Construction,
    BusinessKind::Wholesale,
    BusinessKind::Pawnshop,
    BusinessKind::CoinMachineDistribution,
    BusinessKind::Lodging,
    BusinessKind::AthleticClub,
    BusinessKind::Laundry,
    BusinessKind::Printing,
    BusinessKind::FinancialServices,
    BusinessKind::GarmentFactory,
    BusinessKind::Stevedoring,
    BusinessKind::NewsService,
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
    AlcoholProduction,
    Nightlife,
    Lodging,
    SportingVenue,
    PrintingPress,
    FinancialServices,
    VehicleWorkshop,
    DockAccess,
    RacingWire,
}

impl BusinessFunction {
    /// Canonical player-facing description of a concrete business capability. Keeping this
    /// vocabulary beside the enum prevents surveillance, records, enterprise, and report
    /// surfaces from inventing competing labels for the same modeled capability.
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::CashIntensive => "heavy cash handling",
            Self::VehicleFleet => "a vehicle fleet",
            Self::Warehousing => "storage space",
            Self::MeetingSpace => "private meeting space",
            Self::CustomerAccess => "regular customer access",
            Self::ResaleMarket => "resale-market access",
            Self::UnionAccess => "union access",
            Self::DistributionInfrastructure => "distribution infrastructure",
            Self::ProfessionalRecords => "professional record handling",
            Self::AlcoholProduction => "alcohol production",
            Self::Nightlife => "nightlife venue",
            Self::Lodging => "lodging rooms",
            Self::SportingVenue => "sporting venue",
            Self::PrintingPress => "printing press",
            Self::FinancialServices => "financial services",
            Self::VehicleWorkshop => "vehicle workshop",
            Self::DockAccess => "dock access",
            Self::RacingWire => "racing wire service",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BusinessOwner {
    Independent,
    Organization(OrganizationId),
    Character(CharacterId),
}
