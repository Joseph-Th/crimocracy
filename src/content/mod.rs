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
    InvestigationWorkDefinitionSpec, OperationCashProceedsDefinition,
    OperationDifficultyDefinition, OperationExecutionDefinition, OperationExposureDefinition,
    OperationIntelligenceDefinition, OperationPoliceResponseDefinition,
    OperationPropertyProceedsDefinition, RecruitmentDefinitionSpec,
    RecruitmentIncumbentRelationshipDefinition, RecruitmentInformationQualityDefinition,
    RecruitmentRelationshipDefinition, RecruitmentRelationshipSupportDefinition,
    RecruitmentScoringDefinition, RecruitmentTimingDefinition, RecruitmentTraitRuleDefinition,
    RecruitmentWeightsDefinition, Registry, RegistryBuilder,
};
use crate::world::{
    ApprovalPolicy, BusinessFunction, BusinessKind, CapabilityKind, DriveKind, LegalSupportPolicy,
    PolicyKind, PolicySetting, TraitKind, ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_TRAIT_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CURRENT_CONTENT_REVISION: u32 = 22;

/// Authored floor for police response arrival delays; the patrol-reduction window is the
/// remainder above this minimum so a full-presence response arrives at exactly the floor.
const MINIMUM_POLICE_RESPONSE_DELAY_MINUTES: u32 = 3;

pub fn build_registry() -> Registry {
    let mut builder = RegistryBuilder::new();
    for kind in ALL_CAPABILITY_KINDS {
        builder
            .register_capability(kind)
            .unwrap_or_else(|error| panic!("invalid capability registry: {error}"));
    }
    for kind in ALL_TRAIT_KINDS {
        builder
            .register_trait(kind)
            .unwrap_or_else(|error| panic!("invalid trait registry: {error}"));
    }
    for kind in ALL_DRIVE_KINDS {
        builder
            .register_drive(kind)
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
    // Interview support is the witness's cooperation; a hostile witness can deny the
    // detective a usable statement entirely.
    builder
        .register_investigation_work(
            InvestigationWorkKind::WitnessInterview,
            InvestigationWorkDefinitionSpec {
                duration: SimDuration::from_minutes(120),
                base_difficulty: 30,
                additional_source_difficulty: 0,
                source_support_weight: 45,
                variance_limit: 10,
                connected_margin: 0,
            },
        )
        .unwrap_or_else(|error| panic!("invalid investigation work registry: {error}"));
}

fn register_businesses(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            BusinessKind::Retail,
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
    for (kind, economics) in definitions {
        builder
            .register_business(kind, economics)
            .unwrap_or_else(|error| panic!("invalid business registry: {error}"));
    }
}

fn register_enterprises(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            EnterpriseKind::Protection,
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(4_000),
                base_operating_cost: Money::from_cents(2_500),
                demand_revenue_per_point: Money::from_cents(20),
                commerce_revenue_per_point: Money::from_cents(140),
                wealth_revenue_per_point: Money::from_cents(60),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(35),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
                gross_variance_basis_points: 800,
                notable_variance_basis_points: 600,
            },
            // No special standing-order authority is required to manage a protection
            // racket; the former collection-force axis had no execution effect and was
            // removed with the rest of the inert policy vocabulary.
            None,
            BTreeSet::new(),
            BTreeSet::new(),
        ),
        (
            EnterpriseKind::Gambling,
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(8_000),
                base_operating_cost: Money::from_cents(4_500),
                demand_revenue_per_point: Money::from_cents(160),
                commerce_revenue_per_point: Money::from_cents(40),
                wealth_revenue_per_point: Money::from_cents(100),
                management_revenue_per_point: Money::from_cents(55),
                police_cost_per_point: Money::from_cents(45),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
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
            EnterpriseEconomicsDefinition {
                cycle: SimDuration::from_minutes(1_440),
                base_gross: Money::from_cents(16_000),
                base_operating_cost: Money::from_cents(10_000),
                demand_revenue_per_point: Money::from_cents(130),
                commerce_revenue_per_point: Money::from_cents(50),
                wealth_revenue_per_point: Money::from_cents(25),
                management_revenue_per_point: Money::from_cents(45),
                police_cost_per_point: Money::from_cents(40),
                support_surcharge_per_business: Money::from_cents(7_500),
                heat_surcharge_per_active_case: Money::from_cents(5_000),
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
    for (kind, economics, policy, required_business_functions, required_network_functions) in
        definitions
    {
        builder
            .register_enterprise(
                kind,
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
            PolicyKind::IndependentRecruitment,
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::AssociateLegalSupport,
            PolicySetting::AssociateLegalSupport(LegalSupportPolicy::CaseByCase),
        ),
    ];
    for (kind, default) in definitions {
        builder
            .register_policy(kind, default)
            .unwrap_or_else(|error| panic!("invalid policy registry: {error}"));
    }
}

fn operation_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Burglary => "Burglary",
        OperationKind::Robbery => "Robbery",
        OperationKind::Hijacking => "Hijacking",
        OperationKind::Smuggling => "Smuggling",
        OperationKind::Intimidation => "Intimidation",
        OperationKind::Surveillance => "Surveillance",
        OperationKind::WitnessPressure => "Witness pressure",
        OperationKind::DocumentTheft => "Document theft",
        OperationKind::GamblingEvent => "Gambling event",
        OperationKind::Extraction => "Extraction",
    }
}
fn required_roles(kind: OperationKind) -> BTreeSet<RoleKind> {
    let roles: &[RoleKind] = match kind {
        OperationKind::Burglary => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::Robbery => &[RoleKind::Coordinator, RoleKind::Muscle],
        OperationKind::Hijacking => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Smuggling => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Intimidation => &[RoleKind::Coordinator],
        OperationKind::Surveillance => &[RoleKind::Surveillance],
        OperationKind::WitnessPressure => &[RoleKind::Coordinator],
        OperationKind::DocumentTheft => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::GamblingEvent => &[RoleKind::Coordinator],
        OperationKind::Extraction => &[RoleKind::Coordinator, RoleKind::Driver],
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
        OperationKind::Surveillance => (120, 40, 20, 24),
        OperationKind::WitnessPressure => (30, 50, 35, 48),
        OperationKind::DocumentTheft => (30, 50, 40, 36),
        OperationKind::GamblingEvent => (180, 38, 30, 45),
        OperationKind::Extraction => (60, 58, 50, 52),
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
        OperationKind::WitnessPressure => CapabilityKind::Intimidation,
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Extraction => CapabilityKind::Management,
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
        OperationKind::Surveillance => (45, 18, None, 10, 14),
        OperationKind::WitnessPressure => (24, 12, None, 14, 20),
        OperationKind::DocumentTheft => (20, 12, Some(8), 14, 18),
        OperationKind::GamblingEvent => (35, 15, None, 10, 16),
        OperationKind::Extraction => (18, 10, Some(8), 18, 24),
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
            minimum_response_delay: SimDuration::from_minutes(
                MINIMUM_POLICE_RESPONSE_DELAY_MINUTES,
            ),
            patrol_reduction_minutes: u16::try_from(
                base_response_minutes - MINIMUM_POLICE_RESPONSE_DELAY_MINUTES,
            )
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
            | OperationKind::Surveillance
            | OperationKind::WitnessPressure
            | OperationKind::GamblingEvent
            | OperationKind::Extraction => None,
        },
        cash_proceeds: match kind {
            // Robbery takes the till directly; intimidation collects protection money;
            // a gambling event keeps the house edge; a smuggling run is paid on delivery.
            OperationKind::Robbery => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 40_000,
                partial_take_basis_points: 8_000,
            }),
            OperationKind::Intimidation => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 15_000,
                partial_take_basis_points: 3_000,
            }),
            OperationKind::GamblingEvent => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 20_000,
                partial_take_basis_points: 4_000,
            }),
            OperationKind::Smuggling => Some(OperationCashProceedsDefinition {
                business_take_basis_points: 18_000,
                partial_take_basis_points: 4_000,
            }),
            OperationKind::Burglary
            | OperationKind::Hijacking
            | OperationKind::Surveillance
            | OperationKind::WitnessPressure
            | OperationKind::DocumentTheft
            | OperationKind::Extraction => None,
        },
    }
}

fn operation_exposure_evidence_kind(kind: OperationKind) -> EvidenceKind {
    match kind {
        OperationKind::Burglary | OperationKind::DocumentTheft => EvidenceKind::Fingerprint,
        OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Extraction => EvidenceKind::VehicleDescription,
        OperationKind::Intimidation | OperationKind::WitnessPressure => {
            EvidenceKind::WitnessTestimony
        }
        OperationKind::Surveillance => EvidenceKind::Surveillance,
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
        OperationKind::DocumentTheft => &[
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
        OperationKind::Hijacking | OperationKind::Smuggling | OperationKind::Extraction => &[
            InformationTopic::Schedule,
            InformationTopic::PoliceActivity,
            InformationTopic::Route,
            InformationTopic::Personnel,
        ],
        OperationKind::Intimidation | OperationKind::WitnessPressure => &[
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
