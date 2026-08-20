//! Code-owned authored definitions assembled into the immutable startup registry.

use crate::core::time::SimDuration;
use crate::enterprises::EnterpriseKind;
use crate::finance::Money;
use crate::intelligence::InformationTopic;
use crate::legal::{EvidenceKind, InvestigationWorkKind};
use crate::operations::{
    OperationApproach, OperationKind, RoleKind, ALL_OPERATION_APPROACHES, ALL_OPERATION_KINDS,
};
use crate::recruitment::RecruitmentApproach;
use crate::registry::{
    BusinessEconomicsDefinition, EnterpriseEconomicsDefinition, ExecutiveBriefDefinitionSpec,
    InvestigationWorkDefinitionSpec, OperationDifficultyDefinition, OperationExecutionDefinition,
    OperationExposureDefinition, OperationIntelligenceDefinition,
    OperationPoliceResponseDefinition, OperationPropertyProceedsDefinition,
    RecruitmentDefinitionSpec, RecruitmentIncumbentRelationshipDefinition,
    RecruitmentInformationQualityDefinition, RecruitmentRelationshipDefinition,
    RecruitmentRelationshipSupportDefinition, RecruitmentScoringDefinition,
    RecruitmentTimingDefinition, RecruitmentTraitRuleDefinition, RecruitmentWeightsDefinition,
    Registry, RegistryBuilder,
};
use crate::world::{
    ApprovalPolicy, BusinessFunction, BusinessKind, CapabilityKind, CasualtyPolicy, DriveKind,
    ForcePolicy, LegalSupportPolicy, PolicyKind, PolicySetting, TraitKind, ALL_CAPABILITY_KINDS,
    ALL_DRIVE_KINDS, ALL_TRAIT_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CURRENT_CONTENT_REVISION: u32 = 18;

pub fn build_registry() -> Registry {
    let mut builder = RegistryBuilder::new();
    for kind in ALL_CAPABILITY_KINDS {
        builder
            .register_capability(kind, capability_name(kind))
            .unwrap_or_else(|error| panic!("invalid capability registry: {error}"));
    }
    for kind in ALL_TRAIT_KINDS {
        builder
            .register_trait(kind, trait_name(kind))
            .unwrap_or_else(|error| panic!("invalid trait registry: {error}"));
    }
    for kind in ALL_DRIVE_KINDS {
        builder
            .register_drive(kind, drive_name(kind))
            .unwrap_or_else(|error| panic!("invalid drive registry: {error}"));
    }
    register_recruitment(&mut builder);
    builder
        .register_legal(SimDuration::from_minutes(10_080))
        .unwrap_or_else(|error| panic!("invalid legal registry: {error}"));
    register_policies(&mut builder);
    let approaches: BTreeSet<_> = ALL_OPERATION_APPROACHES.into_iter().collect();
    for kind in ALL_OPERATION_KINDS {
        let roles = required_roles(kind);
        builder
            .register_operation(
                kind,
                operation_name(kind),
                approaches.clone(),
                roles.clone(),
                operation_execution(kind),
            )
            .unwrap_or_else(|error| panic!("invalid operation registry: {error}"));
    }
    register_investigation_work(&mut builder);
    register_enterprises(&mut builder);
    register_businesses(&mut builder);
    register_executive_brief(&mut builder);
    builder
        .build(CURRENT_CONTENT_REVISION)
        .unwrap_or_else(|error| panic!("invalid content registry: {error}"))
}

fn register_executive_brief(builder: &mut RegistryBuilder) {
    builder
        .register_executive_brief(ExecutiveBriefDefinitionSpec {
            cadence: SimDuration::from_minutes(1_440),
            minimum_source_attention: crate::core::attention::AttentionClass::Notable,
            max_source_entries: 8,
        })
        .unwrap_or_else(|error| panic!("invalid executive brief registry: {error}"));
}

fn register_recruitment(builder: &mut RegistryBuilder) {
    builder
        .register_recruitment(RecruitmentDefinitionSpec {
            timing: RecruitmentTimingDefinition {
                cooldown: SimDuration::from_minutes(10_080),
                autonomous_attempt_cadence: SimDuration::from_minutes(1_440),
                perceived_legal_pressure_max_age: SimDuration::from_minutes(20_160),
            },
            scoring: RecruitmentScoringDefinition {
                base_willingness: 20,
                acceptance_score: 45,
                existing_membership_resistance: 15,
                charismatic_recruiter_bonus: 10,
                weights: RecruitmentWeightsDefinition {
                    recruiter_influence: 30,
                    drive_alignment: 25,
                    relationship_support: 25,
                    incumbent_resentment: 15,
                    perceived_legal_pressure: 15,
                    incumbent_attachment: 25,
                },
            },
            recruiter_capabilities: BTreeSet::from([
                CapabilityKind::Negotiation,
                CapabilityKind::SocialAccess,
            ]),
            relationships: RecruitmentRelationshipDefinition {
                recruiter_support: RecruitmentRelationshipSupportDefinition {
                    trust_weight: 2,
                    respect_weight: 1,
                    affection_weight: 1,
                    debt_weight: 1,
                    divisor: 5,
                    fear_penalty_weight: 1,
                    fear_penalty_divisor: 3,
                },
                incumbent_attachment: RecruitmentIncumbentRelationshipDefinition {
                    trust_weight: 1,
                    respect_weight: 1,
                    affection_weight: 1,
                    dependence_weight: 1,
                    divisor: 4,
                },
            },
            information_quality: RecruitmentInformationQualityDefinition {
                unknown_reliability: 20,
                unreliable_reliability: 10,
                mixed_reliability: 40,
                generally_reliable: 70,
                direct_access: 100,
                vague_specificity: 25,
                general_specificity: 50,
                specific_specificity: 75,
                precise_specificity: 100,
            },
            approach_drives: BTreeMap::from([
                (
                    RecruitmentApproach::FinancialOpportunity,
                    BTreeSet::from([DriveKind::Money]),
                ),
                (
                    RecruitmentApproach::Advancement,
                    BTreeSet::from([DriveKind::Status, DriveKind::Independence]),
                ),
                (
                    RecruitmentApproach::Protection,
                    BTreeSet::from([DriveKind::Safety, DriveKind::FamilySecurity]),
                ),
                (
                    RecruitmentApproach::PersonalAppeal,
                    BTreeSet::from([DriveKind::Respect]),
                ),
            ]),
            trait_rules: vec![
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Secretive,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: -8,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Cautious,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: -4,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Impulsive,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: 3,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Vindictive,
                    approach: None,
                    minimum_incumbent_resentment: Some(50),
                    adjustment: 8,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Greedy,
                    approach: Some(RecruitmentApproach::FinancialOpportunity),
                    minimum_incumbent_resentment: None,
                    adjustment: 12,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Ambitious,
                    approach: Some(RecruitmentApproach::FinancialOpportunity),
                    minimum_incumbent_resentment: None,
                    adjustment: 3,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Ambitious,
                    approach: Some(RecruitmentApproach::Advancement),
                    minimum_incumbent_resentment: None,
                    adjustment: 12,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::Advancement),
                    minimum_incumbent_resentment: None,
                    adjustment: 5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::EasilyFrightened,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 15,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Cautious,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 6,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::LoyalToFamily,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: -5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::PersonalAppeal),
                    minimum_incumbent_resentment: None,
                    adjustment: 4,
                },
            ],
        })
        .unwrap_or_else(|error| panic!("invalid recruitment registry: {error}"));
}

fn register_investigation_work(builder: &mut RegistryBuilder) {
    builder
        .register_investigation_work(
            InvestigationWorkKind::PatternAnalysis,
            "Pattern analysis",
            InvestigationWorkDefinitionSpec {
                duration: SimDuration::from_minutes(360),
                base_difficulty: 55,
                additional_source_difficulty: 8,
                source_support_weight: 30,
                variance_limit: 12,
                connected_margin: 0,
            },
        )
        .unwrap_or_else(|error| panic!("invalid investigation work registry: {error}"));
    builder
        .register_investigation_work(
            InvestigationWorkKind::EvidenceReview,
            "Evidence review",
            InvestigationWorkDefinitionSpec {
                duration: SimDuration::from_minutes(180),
                base_difficulty: 45,
                additional_source_difficulty: 0,
                source_support_weight: 35,
                variance_limit: 12,
                connected_margin: 0,
            },
        )
        .unwrap_or_else(|error| panic!("invalid investigation work registry: {error}"));
}

fn register_businesses(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            BusinessKind::Retail,
            "Retail",
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(12_000),
                base_operating_cost: Money::from_cents(10_000),
                wealth_revenue_per_point: Money::from_cents(40),
                commerce_revenue_per_point: Money::from_cents(80),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 1_000,
                notable_variance_basis_points: 800,
            },
        ),
        (
            BusinessKind::Hospitality,
            "Hospitality",
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(15_000),
                base_operating_cost: Money::from_cents(12_000),
                wealth_revenue_per_point: Money::from_cents(60),
                commerce_revenue_per_point: Money::from_cents(90),
                police_cost_per_point: Money::from_cents(30),
                gross_variance_basis_points: 1_200,
                notable_variance_basis_points: 900,
            },
        ),
        (
            BusinessKind::Automotive,
            "Automotive",
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::MeetingSpace,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(14_000),
                base_operating_cost: Money::from_cents(11_000),
                wealth_revenue_per_point: Money::from_cents(50),
                commerce_revenue_per_point: Money::from_cents(70),
                police_cost_per_point: Money::from_cents(25),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 700,
            },
        ),
        (
            BusinessKind::Transportation,
            "Transportation",
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::UnionAccess,
                BusinessFunction::DistributionInfrastructure,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(18_000),
                base_operating_cost: Money::from_cents(15_000),
                wealth_revenue_per_point: Money::from_cents(40),
                commerce_revenue_per_point: Money::from_cents(100),
                police_cost_per_point: Money::from_cents(35),
                gross_variance_basis_points: 700,
                notable_variance_basis_points: 600,
            },
        ),
        (
            BusinessKind::Warehouse,
            "Warehouse",
            BTreeSet::from([
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(9_000),
                base_operating_cost: Money::from_cents(7_500),
                wealth_revenue_per_point: Money::from_cents(10),
                commerce_revenue_per_point: Money::from_cents(60),
                police_cost_per_point: Money::from_cents(15),
                gross_variance_basis_points: 500,
                notable_variance_basis_points: 450,
            },
        ),
        (
            BusinessKind::ProfessionalServices,
            "Professional services",
            BTreeSet::from([
                BusinessFunction::ProfessionalRecords,
                BusinessFunction::CustomerAccess,
                BusinessFunction::MeetingSpace,
            ]),
            BusinessEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(16_000),
                base_operating_cost: Money::from_cents(11_000),
                wealth_revenue_per_point: Money::from_cents(100),
                commerce_revenue_per_point: Money::from_cents(40),
                police_cost_per_point: Money::from_cents(20),
                gross_variance_basis_points: 900,
                notable_variance_basis_points: 700,
            },
        ),
    ];
    for (kind, name, functions, economics) in definitions {
        builder
            .register_business(kind, name, functions, economics)
            .unwrap_or_else(|error| panic!("invalid business registry: {error}"));
    }
}

fn register_enterprises(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            EnterpriseKind::Protection,
            "Protection",
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(4_000),
                base_operating_cost: Money::from_cents(2_500),
                demand_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(140),
                wealth_revenue_per_point: Money::from_cents(60),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(35),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 600,
            },
            Some(PolicyKind::CollectionForce),
            BTreeSet::new(),
            BTreeSet::new(),
        ),
        (
            EnterpriseKind::Gambling,
            "Gambling",
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(8_000),
                base_operating_cost: Money::from_cents(4_500),
                demand_revenue_per_point: Money::from_cents(160),
                commerce_revenue_per_point: Money::from_cents(40),
                wealth_revenue_per_point: Money::from_cents(100),
                management_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(45),
                gross_variance_basis_points: 1_200,
                notable_variance_basis_points: 900,
            },
            None,
            BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            BTreeSet::new(),
        ),
        (
            EnterpriseKind::AlcoholDistribution,
            "Alcohol distribution",
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(16_000),
                base_operating_cost: Money::from_cents(10_000),
                demand_revenue_per_point: Money::from_cents(130),
                commerce_revenue_per_point: Money::from_cents(50),
                wealth_revenue_per_point: Money::from_cents(25),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(40),
                gross_variance_basis_points: 1_800,
                notable_variance_basis_points: 1_200,
            },
            None,
            BTreeSet::new(),
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
                BusinessFunction::CustomerAccess,
            ]),
        ),
    ];
    for (kind, name, economics, policy, required_business_functions, required_network_functions) in
        definitions
    {
        builder
            .register_enterprise(
                kind,
                name,
                economics,
                policy,
                required_business_functions,
                required_network_functions,
            )
            .unwrap_or_else(|error| panic!("invalid enterprise registry: {error}"));
    }
}

fn register_policies(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            PolicyKind::CollectionForce,
            "Collection force",
            PolicySetting::CollectionForce(ForcePolicy::ThreatsOnly),
        ),
        (
            PolicyKind::PatrolBribery,
            "Patrol bribery",
            PolicySetting::PatrolBribery(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::IndependentRecruitment,
            "Independent recruitment",
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::CasualtyResponse,
            "Casualty response",
            PolicySetting::CasualtyResponse(CasualtyPolicy::RequestDecision),
        ),
        (
            PolicyKind::AssociateLegalSupport,
            "Associate legal support",
            PolicySetting::AssociateLegalSupport(LegalSupportPolicy::CaseByCase),
        ),
    ];
    for (kind, name, default) in definitions {
        builder
            .register_policy(kind, name, default)
            .unwrap_or_else(|error| panic!("invalid policy registry: {error}"));
    }
}

fn capability_name(kind: CapabilityKind) -> &'static str {
    match kind {
        CapabilityKind::Violence => "Violence",
        CapabilityKind::Intimidation => "Intimidation",
        CapabilityKind::Stealth => "Stealth",
        CapabilityKind::Burglary => "Burglary",
        CapabilityKind::Driving => "Driving",
        CapabilityKind::Surveillance => "Surveillance",
        CapabilityKind::Investigation => "Investigation",
        CapabilityKind::Accounting => "Accounting",
        CapabilityKind::Negotiation => "Negotiation",
        CapabilityKind::Management => "Management",
        CapabilityKind::PoliticalInfluence => "Political influence",
        CapabilityKind::LegalKnowledge => "Legal knowledge",
        CapabilityKind::SocialAccess => "Social access",
    }
}
fn trait_name(kind: TraitKind) -> &'static str {
    match kind {
        TraitKind::Cautious => "Cautious",
        TraitKind::Impulsive => "Impulsive",
        TraitKind::Greedy => "Greedy",
        TraitKind::Proud => "Proud",
        TraitKind::Patient => "Patient",
        TraitKind::Cruel => "Cruel",
        TraitKind::Charismatic => "Charismatic",
        TraitKind::Vindictive => "Vindictive",
        TraitKind::Secretive => "Secretive",
        TraitKind::Ambitious => "Ambitious",
        TraitKind::LoyalToFamily => "Loyal to family",
        TraitKind::EasilyFrightened => "Easily frightened",
    }
}
fn drive_name(kind: DriveKind) -> &'static str {
    match kind {
        DriveKind::Money => "Money",
        DriveKind::Status => "Status",
        DriveKind::Safety => "Safety",
        DriveKind::Respect => "Respect",
        DriveKind::Revenge => "Revenge",
        DriveKind::FamilySecurity => "Family security",
        DriveKind::PoliticalAdvancement => "Political advancement",
        DriveKind::Independence => "Independence",
        DriveKind::IdeologicalCause => "Ideological cause",
    }
}
fn operation_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Burglary => "Burglary",
        OperationKind::Robbery => "Robbery",
        OperationKind::Hijacking => "Hijacking",
        OperationKind::Smuggling => "Smuggling",
        OperationKind::Intimidation => "Intimidation",
        OperationKind::Kidnapping => "Kidnapping",
        OperationKind::Surveillance => "Surveillance",
        OperationKind::Sabotage => "Sabotage",
        OperationKind::Bribery => "Bribery",
        OperationKind::WitnessPressure => "Witness pressure",
        OperationKind::DocumentTheft => "Document theft",
        OperationKind::GamblingEvent => "Gambling event",
        OperationKind::CovertTransfer => "Covert transfer",
        OperationKind::Extraction => "Extraction",
        OperationKind::RivalInfiltration => "Rival infiltration",
    }
}
fn required_roles(kind: OperationKind) -> BTreeSet<RoleKind> {
    let roles: &[RoleKind] = match kind {
        OperationKind::Burglary => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::Robbery => &[RoleKind::Coordinator, RoleKind::Muscle],
        OperationKind::Hijacking => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Smuggling => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Intimidation => &[RoleKind::Coordinator],
        OperationKind::Kidnapping => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Surveillance => &[RoleKind::Surveillance],
        OperationKind::Sabotage => &[RoleKind::Coordinator],
        OperationKind::Bribery => &[RoleKind::Negotiator],
        OperationKind::WitnessPressure => &[RoleKind::Coordinator],
        OperationKind::DocumentTheft => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::GamblingEvent => &[RoleKind::Coordinator],
        OperationKind::CovertTransfer => &[RoleKind::Coordinator],
        OperationKind::Extraction => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::RivalInfiltration => &[RoleKind::Coordinator],
    };
    roles.iter().copied().collect()
}

fn operation_execution(kind: OperationKind) -> OperationExecutionDefinition {
    let (duration_minutes, base_difficulty, police_pressure_weight, base_exposure) = match kind {
        OperationKind::Burglary => (45, 52, 45, 38),
        OperationKind::Robbery => (25, 55, 55, 58),
        OperationKind::Hijacking => (35, 50, 45, 48),
        OperationKind::Smuggling => (90, 48, 35, 35),
        OperationKind::Intimidation => (20, 42, 25, 42),
        OperationKind::Kidnapping => (60, 65, 55, 62),
        OperationKind::Surveillance => (120, 40, 20, 24),
        OperationKind::Sabotage => (50, 55, 40, 44),
        OperationKind::Bribery => (30, 45, 20, 28),
        OperationKind::WitnessPressure => (30, 50, 35, 48),
        OperationKind::DocumentTheft => (30, 50, 40, 36),
        OperationKind::GamblingEvent => (180, 38, 30, 45),
        OperationKind::CovertTransfer => (45, 42, 30, 30),
        OperationKind::Extraction => (60, 58, 50, 52),
        OperationKind::RivalInfiltration => (180, 68, 25, 26),
    };
    let role_capabilities = BTreeMap::from([
        (
            RoleKind::Driver,
            capability_for_operation_role(RoleKind::Driver),
        ),
        (
            RoleKind::Lookout,
            capability_for_operation_role(RoleKind::Lookout),
        ),
        (
            RoleKind::EntrySpecialist,
            capability_for_operation_role(RoleKind::EntrySpecialist),
        ),
        (
            RoleKind::SafeSpecialist,
            capability_for_operation_role(RoleKind::SafeSpecialist),
        ),
        (
            RoleKind::Muscle,
            capability_for_operation_role(RoleKind::Muscle),
        ),
        (
            RoleKind::InsideContact,
            capability_for_operation_role(RoleKind::InsideContact),
        ),
        (
            RoleKind::Coordinator,
            capability_for_operation_role(RoleKind::Coordinator),
        ),
        (
            RoleKind::Surveillance,
            capability_for_operation_role(RoleKind::Surveillance),
        ),
        (
            RoleKind::Negotiator,
            capability_for_operation_role(RoleKind::Negotiator),
        ),
    ]);
    let leader_capability = match kind {
        OperationKind::Surveillance => CapabilityKind::Surveillance,
        OperationKind::Bribery => CapabilityKind::Negotiation,
        OperationKind::WitnessPressure => CapabilityKind::Intimidation,
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::Kidnapping
        | OperationKind::Sabotage
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::CovertTransfer
        | OperationKind::Extraction
        | OperationKind::RivalInfiltration => CapabilityKind::Management,
    };
    let approach_difficulty_adjustments = ALL_OPERATION_APPROACHES
        .into_iter()
        .map(|approach| {
            let adjustment = match approach {
                OperationApproach::Covert => -5,
                OperationApproach::Deceptive => -2,
                OperationApproach::Intimidating => 3,
                OperationApproach::Violent => 6,
                OperationApproach::InsideAssistance => -8,
                OperationApproach::Opportunistic => 4,
            };
            (approach, adjustment)
        })
        .collect();
    let exposure_approach_adjustments = ALL_OPERATION_APPROACHES
        .into_iter()
        .map(|approach| {
            let adjustment = match approach {
                OperationApproach::Covert => -12,
                OperationApproach::Deceptive => -5,
                OperationApproach::Intimidating => 10,
                OperationApproach::Violent => 18,
                OperationApproach::InsideAssistance => -10,
                OperationApproach::Opportunistic => 6,
            };
            (approach, adjustment)
        })
        .collect();
    let (
        dispatch_threshold,
        base_response_minutes,
        entry_minutes,
        response_difficulty,
        response_exposure,
    ) = match kind {
        OperationKind::Burglary => (20, 12, Some(10), 14, 18),
        OperationKind::Robbery => (12, 8, Some(6), 18, 24),
        OperationKind::Hijacking => (18, 10, Some(5), 16, 22),
        OperationKind::Smuggling => (28, 15, None, 12, 18),
        OperationKind::Intimidation => (24, 10, None, 12, 18),
        OperationKind::Kidnapping => (16, 10, Some(8), 20, 26),
        OperationKind::Surveillance => (45, 18, None, 10, 14),
        OperationKind::Sabotage => (22, 12, Some(10), 16, 20),
        OperationKind::Bribery => (48, 20, None, 8, 12),
        OperationKind::WitnessPressure => (24, 12, None, 14, 20),
        OperationKind::DocumentTheft => (20, 12, Some(8), 14, 18),
        OperationKind::GamblingEvent => (35, 15, None, 10, 16),
        OperationKind::CovertTransfer => (32, 15, None, 12, 16),
        OperationKind::Extraction => (18, 10, Some(8), 18, 24),
        OperationKind::RivalInfiltration => (48, 20, None, 10, 14),
    };
    OperationExecutionDefinition {
        difficulty: OperationDifficultyDefinition {
            duration: SimDuration::from_minutes(duration_minutes),
            base_difficulty,
            role_capabilities,
            approach_difficulty_adjustments,
            police_pressure_weight,
            variance_limit: 12,
            achieved_margin: 5,
            partial_margin: -12,
        },
        leader_capability,
        intelligence: OperationIntelligenceDefinition {
            relevant_topics: relevant_operation_intelligence(kind),
            max_difficulty_reduction: 14,
            max_useful_age: SimDuration::from_minutes(10_080),
        },
        exposure: OperationExposureDefinition {
            base_exposure,
            approach_adjustments: exposure_approach_adjustments,
            police_observation_weight: 35,
            stealth_mitigation_weight: 45,
            intelligence_mitigation_weight: 20,
            variance_limit: 12,
            trace_threshold: 20,
            witnessed_threshold: 45,
            identifying_threshold: 65,
            evidence_kind: operation_exposure_evidence_kind(kind),
        },
        police_response: OperationPoliceResponseDefinition {
            dispatch_threshold,
            base_response_delay: SimDuration::from_minutes(base_response_minutes),
            minimum_response_delay: SimDuration::from_minutes(3),
            patrol_reduction_minutes: u16::try_from(base_response_minutes - 3)
                .expect("authored police response delay range must fit u16"),
            entry_offset: entry_minutes.map(SimDuration::from_minutes),
            arrival_difficulty_penalty: response_difficulty,
            arrival_exposure_penalty: response_exposure,
        },
        property_proceeds: match kind {
            OperationKind::Burglary => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 30_000,
                partial_recovery_basis_points: 4_000,
                liquidation_recovery_basis_points: 6_500,
            }),
            OperationKind::Hijacking => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 25_000,
                partial_recovery_basis_points: 3_500,
                liquidation_recovery_basis_points: 5_500,
            }),
            OperationKind::DocumentTheft => Some(OperationPropertyProceedsDefinition {
                business_gross_basis_points: 12_500,
                partial_recovery_basis_points: 5_000,
                liquidation_recovery_basis_points: 4_000,
            }),
            OperationKind::Robbery
            | OperationKind::Smuggling
            | OperationKind::Intimidation
            | OperationKind::Kidnapping
            | OperationKind::Surveillance
            | OperationKind::Sabotage
            | OperationKind::Bribery
            | OperationKind::WitnessPressure
            | OperationKind::GamblingEvent
            | OperationKind::CovertTransfer
            | OperationKind::Extraction
            | OperationKind::RivalInfiltration => None,
        },
    }
}

fn operation_exposure_evidence_kind(kind: OperationKind) -> EvidenceKind {
    match kind {
        OperationKind::Burglary | OperationKind::DocumentTheft | OperationKind::Sabotage => {
            EvidenceKind::Fingerprint
        }
        OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::CovertTransfer
        | OperationKind::Extraction => EvidenceKind::VehicleDescription,
        OperationKind::Intimidation
        | OperationKind::Kidnapping
        | OperationKind::WitnessPressure => EvidenceKind::WitnessTestimony,
        OperationKind::Surveillance | OperationKind::RivalInfiltration => {
            EvidenceKind::Surveillance
        }
        OperationKind::Bribery => EvidenceKind::CommunicationRecord,
        OperationKind::GamblingEvent => EvidenceKind::FinancialRecord,
    }
}

fn relevant_operation_intelligence(kind: OperationKind) -> BTreeSet<InformationTopic> {
    let topics: &[InformationTopic] = match kind {
        OperationKind::Burglary => &[
            InformationTopic::TargetSecurity,
            InformationTopic::MarketAccess,
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::DocumentTheft | OperationKind::Sabotage => &[
            InformationTopic::TargetSecurity,
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::Robbery => &[
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
        ],
        OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::CovertTransfer
        | OperationKind::Extraction => &[
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
            InformationTopic::Personnel,
        ],
        OperationKind::Intimidation
        | OperationKind::Kidnapping
        | OperationKind::WitnessPressure
        | OperationKind::Bribery
        | OperationKind::RivalInfiltration => &[
            InformationTopic::Personnel,
            InformationTopic::Relationship,
            InformationTopic::PoliceActivity,
        ],
        OperationKind::Surveillance => &[
            InformationTopic::Personnel,
            InformationTopic::Schedule,
            InformationTopic::Route,
            InformationTopic::PoliceActivity,
        ],
        OperationKind::GamblingEvent => &[
            InformationTopic::PoliceActivity,
            InformationTopic::Personnel,
            InformationTopic::MarketAccess,
        ],
    };
    topics.iter().copied().collect()
}

fn capability_for_operation_role(role: RoleKind) -> CapabilityKind {
    match role {
        RoleKind::Driver => CapabilityKind::Driving,
        RoleKind::Lookout => CapabilityKind::Surveillance,
        RoleKind::EntrySpecialist => CapabilityKind::Burglary,
        RoleKind::SafeSpecialist => CapabilityKind::Burglary,
        RoleKind::Muscle => CapabilityKind::Violence,
        RoleKind::InsideContact => CapabilityKind::SocialAccess,
        RoleKind::Coordinator => CapabilityKind::Management,
        RoleKind::Surveillance => CapabilityKind::Surveillance,
        RoleKind::Negotiator => CapabilityKind::Negotiation,
    }
}
