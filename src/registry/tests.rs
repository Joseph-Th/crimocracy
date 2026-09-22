//! Immutable registry validation, completeness, and determinism tests.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::finance::Money;
use crate::intelligence::InformationTopic;
use crate::operations::{ALL_OPERATION_KINDS, OperationApproach, RoleKind};
use crate::recruitment::RecruitmentApproach;
use crate::world::{ALL_TRAIT_KINDS, BusinessFunction, CapabilityKind, DriveKind, TraitKind};
use std::collections::BTreeSet;

fn burglary_operation_parts() -> (
    BTreeSet<OperationApproach>,
    BTreeSet<RoleKind>,
    OperationExecutionDefinition,
) {
    let registry = build_registry();
    let definition = registry.get_operation(OperationKind::Burglary);
    (
        definition.supported_approaches().clone(),
        definition.required_roles().clone(),
        definition.execution().clone(),
    )
}

#[test]
fn surveillance_accepts_observed_enterprise_activity_as_planning_intelligence() {
    let registry = build_registry();
    let relevant = registry
        .get_operation(OperationKind::Surveillance)
        .execution()
        .relevant_intelligence_topics();
    assert!(relevant.contains(&InformationTopic::EnterpriseActivity));
}

#[test]
fn authored_bookmaking_requires_cash_customers_and_a_racing_wire() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Bookmaking);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
        ])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([BusinessFunction::RacingWire])
    );
    assert_eq!(
        definition.network_mode(),
        EnterpriseNetworkMode::SupportingBusinessesOnly
    );
    let economics = definition.economics();
    assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(190));
    assert_eq!(
        economics.support_surcharge_per_business(),
        Money::from_cents(3_000)
    );
    assert_eq!(economics.gross_variance_basis_points(), 2_200);
}

fn investigation_work_spec(kind: InvestigationWorkKind) -> InvestigationWorkDefinitionSpec {
    let registry = build_registry();
    let definition = registry.get_investigation_work(kind);
    InvestigationWorkDefinitionSpec {
        duration: definition.duration(),
        base_difficulty: definition.base_difficulty(),
        source_support_weight: definition.source_support_weight(),
        variance_limit: definition.variance_limit(),
        connected_margin: definition.connected_margin(),
        source_support: definition.source_support(),
        interview_outcome: definition.interview_outcome(),
    }
}

fn information_quality_spec() -> InformationQualityDefinition {
    InformationQualityDefinition {
        unknown_reliability: 20,
        unreliable_reliability: 10,
        mixed_reliability: 40,
        generally_reliable: 70,
        direct_access: 100,
        vague_specificity: 25,
        general_specificity: 50,
        specific_specificity: 75,
        precise_specificity: 100,
    }
}

fn legal_spec() -> LegalConfigSpec {
    LegalConfigSpec {
        off_window_patrol_presence_percent: 50,
        cold_case_window: SimDuration::from_minutes(1_440),
        witness_interview_attempt_limit: 2,
        witness_testimony: WitnessTestimonyDefinition {
            strength_corroborating_min_confidence: 35,
            strength_strong_min_confidence: 60,
            strength_direct_min_confidence: 85,
            reliability_mixed_min_confidence: 25,
            reliability_credible_min_confidence: 50,
            reliability_highly_reliable_min_confidence: 80,
            reluctant_band_discount: 1,
            hostile_band_discount: 2,
        },
        informant_decision_delay: SimDuration::from_minutes(1_440),
        minimum_arrest_qualifying_evidence: 2,
        informant_base_flip_chance_percent: 25,
        informant_safety_bonus_percent: 50,
        represented_informant_reduction_percent: 25,
        automatic_support_retainer: Money::from_cents(5_000),
        maximum_detention: SimDuration::from_minutes(2_880),
    }
}

#[test]
fn legal_registry_authors_a_bounded_off_window_patrol_floor() {
    assert_eq!(
        build_registry()
            .legal()
            .off_window_patrol_presence_percent(),
        50
    );

    for invalid in [0, 100] {
        let mut builder = RegistryBuilder::default();
        let mut spec = legal_spec();
        spec.off_window_patrol_presence_percent = invalid;
        assert!(matches!(
            builder.register_legal(spec),
            Err(RegistryBuildError::InvalidLegalOffWindowPatrolPresencePercent)
        ));
    }
}

#[test]
fn information_quality_definition_is_global_unique_and_bounded() {
    let registry = build_registry();
    let quality = registry.information_quality();
    assert_eq!(
        quality.reliability_score(crate::intelligence::Reliability::Unknown),
        20
    );
    assert_eq!(
        quality.specificity_score(crate::intelligence::Specificity::Precise),
        100
    );

    let mut builder = RegistryBuilder::default();
    let valid = information_quality_spec();
    builder
        .register_information_quality(valid)
        .expect("first information-quality definition should register");
    assert!(matches!(
        builder.register_information_quality(valid),
        Err(RegistryBuildError::DuplicateInformationQuality)
    ));

    let mut invalid = information_quality_spec();
    invalid.direct_access = 101;
    let mut invalid_builder = RegistryBuilder::default();
    assert!(matches!(
        invalid_builder.register_information_quality(invalid),
        Err(RegistryBuildError::InvalidInformationQuality)
    ));
}

#[test]
fn operation_leaders_use_their_authored_domain_capability() {
    let registry = build_registry();

    let burglary = registry.get_operation(OperationKind::Burglary).execution();
    assert_eq!(burglary.leader_capability(), CapabilityKind::Management);
    assert_eq!(burglary.role_capability_weight(), 3);
    assert_eq!(burglary.leader_capability_weight(), 1);
    assert_eq!(
        registry
            .get_operation(OperationKind::Surveillance)
            .execution()
            .leader_capability(),
        CapabilityKind::Surveillance
    );
}

#[test]
fn authored_fraud_requires_financial_records_and_customer_access() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Fraud);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::ProfessionalRecords,
            BusinessFunction::FinancialServices,
        ])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([BusinessFunction::CustomerAccess])
    );
    assert_eq!(
        definition.network_mode(),
        EnterpriseNetworkMode::HostMayContribute
    );
    let economics = definition.economics();
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(110));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(95)
    );
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(90)
    );
    assert_eq!(economics.gross_variance_basis_points(), 1_500);
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        400
    );
}

#[test]
fn authored_auto_theft_ring_requires_workshop_resale_and_records_network() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::AutoTheftRing);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::ResaleMarket,
            BusinessFunction::VehicleWorkshop,
        ])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::CustomerAccess,
            BusinessFunction::ProfessionalRecords,
        ])
    );
    let economics = definition.economics();
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(120)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(100));
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(75)
    );
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        520
    );
    assert_eq!(economics.gross_variance_basis_points(), 1_300);
}

#[test]
fn authored_smuggling_requires_dock_storage_and_documented_distribution() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Smuggling);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([BusinessFunction::Warehousing, BusinessFunction::DockAccess,])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::ProfessionalRecords,
        ])
    );
    let economics = definition.economics();
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(115)
    );
    assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(90));
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(80)
    );
    assert_eq!(economics.gross_variance_basis_points(), 1_600);
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        600
    );
}

fn recruitment_spec() -> RecruitmentDefinitionSpec {
    RecruitmentDefinitionSpec {
        timing: RecruitmentTimingDefinition {
            cooldown: SimDuration::from_minutes(60),
            autonomous_attempt_cadence: SimDuration::from_minutes(1_440),
            perceived_legal_pressure_max_age: SimDuration::from_minutes(1_440),
        },
        scoring: RecruitmentScoringDefinition {
            base_willingness: 10,
            acceptance_score: 40,
            existing_membership_resistance: 10,
            charismatic_recruiter_bonus: 5,
            weights: RecruitmentWeightsDefinition {
                recruiter_influence: 25,
                drive_alignment: 25,
                relationship_support: 25,
                incumbent_resentment: 10,
                perceived_legal_pressure: 10,
                incumbent_attachment: 25,
                organization_competence: 10,
            },
        },
        recruiter_capabilities: BTreeSet::from([CapabilityKind::Negotiation]),
        relationships: RecruitmentRelationshipDefinition {
            recruiter_support: RecruitmentRelationshipSupportDefinition {
                trust_weight: 1,
                respect_weight: 1,
                affection_weight: 1,
                debt_weight: 1,
                divisor: 4,
                fear_penalty_weight: 1,
                fear_penalty_divisor: 2,
            },
            incumbent_attachment: RecruitmentIncumbentRelationshipDefinition {
                trust_weight: 1,
                respect_weight: 1,
                affection_weight: 1,
                dependence_weight: 1,
                divisor: 4,
            },
        },
        approach_drives: BTreeMap::from([
            (
                RecruitmentApproach::FinancialOpportunity,
                BTreeSet::from([DriveKind::Money]),
            ),
            (
                RecruitmentApproach::Advancement,
                BTreeSet::from([DriveKind::Status]),
            ),
            (
                RecruitmentApproach::Protection,
                BTreeSet::from([DriveKind::Safety]),
            ),
            (
                RecruitmentApproach::PersonalAppeal,
                BTreeSet::from([DriveKind::Respect]),
            ),
        ]),
        trait_rules: vec![RecruitmentTraitRuleDefinition {
            trait_kind: TraitKind::Ambitious,
            approach: Some(RecruitmentApproach::Advancement),
            minimum_incumbent_resentment: None,
            adjustment: 5,
        }],
    }
}

fn reputation_spec() -> ReputationConfigSpec {
    let definition = build_registry().reputation();
    ReputationConfigSpec {
        baseline: definition.baseline(),
        daily_decay_step: definition.daily_decay_step(),
        expansion_police_fear_ceiling: definition.expansion_police_fear_ceiling(),
        witnessed_exposure_police_fear: definition.witnessed_exposure_police_fear(),
        identifying_exposure_police_fear: definition.identifying_exposure_police_fear(),
        racket_inquiry_police_fear: definition.racket_inquiry_police_fear(),
        achieved_underworld_competence: definition.achieved_underworld_competence(),
        partial_underworld_competence: definition.partial_underworld_competence(),
        violent_businesses_fear: definition.violent_businesses_fear(),
        intimidation_business_fear_max_adjustment: definition
            .intimidation_business_fear_max_adjustment(),
    }
}

#[test]
fn authored_recruitment_definition_is_complete_and_queryable() {
    let registry = build_registry();
    let definition = registry.recruitment();
    assert_eq!(definition.cooldown(), SimDuration::from_minutes(10_080));
    assert_eq!(
        definition.autonomous_attempt_cadence(),
        SimDuration::from_minutes(1_440)
    );
    assert_eq!(
        definition.recruiter_capabilities(),
        &BTreeSet::from([CapabilityKind::Negotiation, CapabilityKind::SocialAccess])
    );
    assert_eq!(
        definition.drives_for_approach(RecruitmentApproach::Protection),
        &BTreeSet::from([DriveKind::Safety, DriveKind::FamilySecurity])
    );
    assert!(
        definition
            .trait_rules()
            .iter()
            .any(|rule| rule.trait_kind == TraitKind::EasilyFrightened
                && rule.approach == Some(RecruitmentApproach::Protection)
                && rule.adjustment > 0)
    );
}

#[test]
fn reputation_authoring_rejects_neutral_throttle_and_inverted_consequences() {
    let mut spec = reputation_spec();
    spec.expansion_police_fear_ceiling = spec.baseline;
    assert!(matches!(
        RegistryBuilder::default().register_reputation(spec),
        Err(RegistryBuildError::InvalidReputationCeiling)
    ));

    let mut spec = reputation_spec();
    spec.identifying_exposure_police_fear = spec.witnessed_exposure_police_fear - 1;
    assert!(matches!(
        RegistryBuilder::default().register_reputation(spec),
        Err(RegistryBuildError::InvalidReputationConsequence)
    ));

    let mut spec = reputation_spec();
    spec.racket_inquiry_police_fear = spec.witnessed_exposure_police_fear - 1;
    assert!(matches!(
        RegistryBuilder::default().register_reputation(spec),
        Err(RegistryBuildError::InvalidReputationConsequence)
    ));

    let mut spec = reputation_spec();
    spec.partial_underworld_competence = -1;
    assert!(matches!(
        RegistryBuilder::default().register_reputation(spec),
        Err(RegistryBuildError::InvalidReputationConsequence)
    ));

    let mut spec = reputation_spec();
    spec.intimidation_business_fear_max_adjustment = 0;
    assert!(matches!(
        RegistryBuilder::default().register_reputation(spec),
        Err(RegistryBuildError::InvalidReputationConsequence)
    ));
}

#[test]
fn authored_legal_timing_keeps_informant_decision_inside_bounded_custody() {
    let legal = build_registry().legal();
    assert_eq!(
        legal.informant_decision_delay(),
        SimDuration::from_minutes(1_440)
    );
    assert_eq!(legal.maximum_detention(), SimDuration::from_minutes(2_880));
    assert!(legal.maximum_detention() > legal.informant_decision_delay());
    assert_eq!(legal.minimum_arrest_qualifying_evidence(), 2);
    assert_eq!(legal.automatic_support_retainer(), Money::from_cents(5_000));
    assert_eq!(
        legal.witness_testimony().strength_thresholds(),
        [35, 60, 85]
    );
}

#[test]
fn authored_economic_effects_must_change_the_resource_they_claim_to_model() {
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_business_disruption(BusinessDisruptionSpec {
            duration: SimDuration::ONE_MINUTE,
            gross_basis_points: 10_000,
        }),
        Err(RegistryBuildError::InvalidBusinessDisruptionGrossBasisPoints)
    ));

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_laundering(LaunderingConfigSpec {
            fee_basis_points: 10_000,
            plausibility_gross_basis_points: 10_000,
        }),
        Err(RegistryBuildError::InvalidLaunderingFee)
    ));
}

#[test]
fn legal_registry_rejects_a_custody_window_that_cannot_reach_the_informant_decision() {
    let mut builder = RegistryBuilder::default();
    let mut spec = legal_spec();
    spec.maximum_detention = spec.informant_decision_delay;
    assert!(matches!(
        builder.register_legal(spec),
        Err(RegistryBuildError::InvalidLegalMaximumDetention)
    ));
}

#[test]
fn legal_registry_rejects_invalid_witness_testimony_bands() {
    let mut builder = RegistryBuilder::default();
    let mut spec = legal_spec();
    spec.witness_testimony.strength_direct_min_confidence =
        spec.witness_testimony.strength_strong_min_confidence;
    assert!(matches!(
        builder.register_legal(spec),
        Err(RegistryBuildError::InvalidLegalWitnessTestimony)
    ));

    let mut builder = RegistryBuilder::default();
    let mut spec = legal_spec();
    spec.witness_testimony.reluctant_band_discount = 3;
    spec.witness_testimony.hostile_band_discount = 2;
    assert!(matches!(
        builder.register_legal(spec),
        Err(RegistryBuildError::InvalidLegalWitnessTestimony)
    ));
}

#[test]
fn authored_executive_brief_definition_is_bounded_and_queryable() {
    let definition = build_registry().executive_brief();
    assert_eq!(definition.cadence(), SimDuration::from_minutes(1_440));
    assert_eq!(
        definition.minimum_source_attention(),
        AttentionClass::Notable
    );
    assert_eq!(definition.max_source_entries(), 8);
}

#[test]
fn upkeep_definition_rejects_unrepresentable_payroll_and_out_of_rail_resentment() {
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_upkeep(UpkeepConfigSpec {
            per_member_daily: Money::from_cents(i64::MAX),
            shortfall_resentment: 10,
        }),
        Err(RegistryBuildError::InvalidUpkeepArithmeticRange)
    ));

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_upkeep(UpkeepConfigSpec {
            per_member_daily: Money::from_cents(1_000),
            shortfall_resentment: 101,
        }),
        Err(RegistryBuildError::InvalidUpkeepResentment)
    ));
}

#[test]
fn operation_proceeds_arithmetic_is_safe_regardless_of_registration_order() {
    let registry = build_registry();
    let business_kind = crate::world::BusinessKind::Retail;
    let mut business = registry.get_business(business_kind).economics().clone();
    business.base_gross = Money::from_cents(i64::MAX / 2);
    business.wealth_revenue_per_point = Money::ZERO;
    business.commerce_revenue_per_point = Money::ZERO;
    business.gross_variance_basis_points = 0;
    business.notable_variance_basis_points = 0;

    let operation_kind = OperationKind::Robbery;
    let operation = registry.get_operation(operation_kind);

    let mut business_first = RegistryBuilder::default();
    business_first
        .register_business(business_kind, business.clone())
        .expect("large business is individually representable before operation authorship");
    assert!(matches!(
        business_first.register_operation(
            operation_kind,
            "Robbery",
            operation.supported_approaches().clone(),
            operation.required_roles().clone(),
            operation.execution().clone(),
        ),
        Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(kind))
            if kind == operation_kind
    ));

    let mut operation_first = RegistryBuilder::default();
    operation_first
        .register_operation(
            operation_kind,
            "Robbery",
            operation.supported_approaches().clone(),
            operation.required_roles().clone(),
            operation.execution().clone(),
        )
        .expect("operation is individually representable before business authorship");
    assert!(matches!(
        operation_first.register_business(business_kind, business),
        Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(kind))
            if kind == operation_kind
    ));
}

#[test]
fn operation_proceeds_overflow_proof_matches_runtime_cent_rounding_boundary() {
    let registry = build_registry();
    let business_kind = crate::world::BusinessKind::Retail;
    let mut business = registry.get_business(business_kind).economics().clone();
    // Document theft pays 125% of modeled gross. This exact gross produces
    // i64::MAX + 0.5 cents before rounding: truncation would fit, while the runtime's canonical
    // round-half-away rule must reject it as one cent beyond Money's representable range.
    business.base_gross = Money::from_cents(7_378_697_629_483_820_646);
    business.wealth_revenue_per_point = Money::ZERO;
    business.commerce_revenue_per_point = Money::ZERO;
    business.gross_variance_basis_points = 0;
    business.notable_variance_basis_points = 0;

    let operation_kind = OperationKind::DocumentTheft;
    let operation = registry.get_operation(operation_kind);
    let mut business_first = RegistryBuilder::default();
    business_first
        .register_business(business_kind, business.clone())
        .expect("boundary business is individually representable");
    assert!(matches!(
        business_first.register_operation(
            operation_kind,
            "Document theft",
            operation.supported_approaches().clone(),
            operation.required_roles().clone(),
            operation.execution().clone(),
        ),
        Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(kind))
            if kind == operation_kind
    ));

    let mut operation_first = RegistryBuilder::default();
    operation_first
        .register_operation(
            operation_kind,
            "Document theft",
            operation.supported_approaches().clone(),
            operation.required_roles().clone(),
            operation.execution().clone(),
        )
        .expect("operation is individually representable before boundary business authorship");
    assert!(matches!(
        operation_first.register_business(business_kind, business),
        Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(kind))
            if kind == operation_kind
    ));
}

#[test]
fn economic_definitions_reject_values_that_can_overflow_normal_settlement() {
    let registry = build_registry();

    let business_kind = crate::world::BusinessKind::Retail;
    let mut business = registry.get_business(business_kind).economics().clone();
    business.wealth_revenue_per_point = Money::from_cents(i64::MAX);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_business(business_kind, business),
        Err(RegistryBuildError::BusinessEconomicArithmeticOutOfRange(kind))
            if kind == business_kind
    ));

    let enterprise_kind = EnterpriseKind::AlcoholDistribution;
    let definition = registry.get_enterprise(enterprise_kind);
    let mut enterprise = definition.economics().clone();
    enterprise.support_surcharge_per_business = Money::from_cents(i64::MAX);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_enterprise(
            enterprise_kind,
            enterprise,
            definition.required_business_functions().clone(),
            definition.required_network_functions().clone(),
            definition.network_mode(),
        ),
        Err(RegistryBuildError::EnterpriseEconomicArithmeticOutOfRange(kind))
            if kind == enterprise_kind
    ));
}

#[test]
fn enterprise_economic_validation_preserves_inclusive_authored_boundaries() {
    let registry = build_registry();
    let kind = EnterpriseKind::Protection;
    let definition = registry.get_enterprise(kind);
    let register = |economics: EnterpriseEconomicsDefinition| {
        RegistryBuilder::default().register_enterprise(
            kind,
            economics,
            definition.required_business_functions().clone(),
            definition.required_network_functions().clone(),
            definition.network_mode(),
        )
    };

    let mut zero_cost = definition.economics().clone();
    zero_cost.support_surcharge_per_business = Money::ZERO;
    register(zero_cost).expect("zero-valued enterprise economic inputs are valid");

    let mut negative_cost = definition.economics().clone();
    negative_cost.support_surcharge_per_business = Money::from_cents(-1);
    assert!(matches!(
        register(negative_cost),
        Err(RegistryBuildError::NegativeEnterpriseEconomicValue(error_kind))
            if error_kind == kind
    ));

    let mut maximum_variance = definition.economics().clone();
    maximum_variance.gross_variance_basis_points = 5_000;
    maximum_variance.notable_variance_basis_points = 5_000;
    register(maximum_variance).expect("the 5000 basis-point variance ceiling is inclusive");

    let mut excessive_variance = definition.economics().clone();
    excessive_variance.gross_variance_basis_points = 5_001;
    assert!(matches!(
        register(excessive_variance),
        Err(RegistryBuildError::EnterpriseVarianceOutOfRange(error_kind))
            if error_kind == kind
    ));

    let mut equal_notable_variance = definition.economics().clone();
    equal_notable_variance.notable_variance_basis_points =
        equal_notable_variance.gross_variance_basis_points;
    register(equal_notable_variance).expect("notable variance may equal the gross variance range");

    let mut excessive_notable_variance = definition.economics().clone();
    excessive_notable_variance.notable_variance_basis_points =
        excessive_notable_variance.gross_variance_basis_points + 1;
    assert!(matches!(
        register(excessive_notable_variance),
        Err(RegistryBuildError::EnterpriseNotableVarianceOutOfRange(error_kind))
            if error_kind == kind
    ));

    let mut certain_attention = definition.economics().clone();
    certain_attention.enforcement_attention_basis_points_per_active_case = 10_000;
    register(certain_attention).expect("10000 basis points is a valid certain-attention chance");

    let mut invalid_attention = definition.economics().clone();
    invalid_attention.enforcement_attention_basis_points_per_active_case = 10_001;
    assert!(matches!(
        register(invalid_attention),
        Err(RegistryBuildError::EnterpriseEnforcementAttentionOutOfRange(error_kind))
            if error_kind == kind
    ));
}

#[test]
fn business_economic_validation_preserves_inclusive_variance_boundary() {
    let registry = build_registry();
    let kind = crate::world::BusinessKind::Retail;
    let definition = registry.get_business(kind);
    let register = |economics: BusinessEconomicsDefinition| {
        RegistryBuilder::default().register_business(kind, economics)
    };

    let mut zero_cost = definition.economics().clone();
    zero_cost.base_operating_cost = Money::ZERO;
    register(zero_cost).expect("zero-valued business operating cost is valid");

    let mut negative_cost = definition.economics().clone();
    negative_cost.base_operating_cost = Money::from_cents(-1);
    assert!(matches!(
        register(negative_cost),
        Err(RegistryBuildError::NegativeBusinessEconomicValue(error_kind))
            if error_kind == kind
    ));

    let mut maximum_variance = definition.economics().clone();
    maximum_variance.gross_variance_basis_points = 5_000;
    maximum_variance.notable_variance_basis_points = 5_000;
    register(maximum_variance)
        .expect("the 5000 basis-point business variance ceiling is inclusive");

    let mut excessive_variance = definition.economics().clone();
    excessive_variance.gross_variance_basis_points = 5_001;
    assert!(matches!(
        register(excessive_variance),
        Err(RegistryBuildError::BusinessVarianceOutOfRange(error_kind))
            if error_kind == kind
    ));
}

#[test]
fn authored_alcohol_distribution_requires_concrete_commercial_network() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::AlcoholDistribution);
    assert!(definition.required_business_functions().is_empty());
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::VehicleFleet,
            BusinessFunction::Warehousing,
            BusinessFunction::DistributionInfrastructure,
            BusinessFunction::CustomerAccess,
        ])
    );
    let economics = definition.economics();
    assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
    assert_eq!(economics.base_gross(), Money::from_cents(16_000));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(10_000));
    assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(130));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(50)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(25));
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(45)
    );
    assert_eq!(economics.police_cost_per_point(), Money::from_cents(40));
    assert_eq!(economics.gross_variance_basis_points(), 1_800);
    assert_eq!(economics.notable_variance_basis_points(), 1_200);
}

#[test]
fn authored_numbers_racket_is_a_district_network_not_another_fixed_gambling_venue() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::NumbersRacket);
    assert!(
        definition.required_business_functions().is_empty(),
        "numbers writers operate across the district rather than requiring one dedicated venue"
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::ProfessionalRecords,
        ])
    );
    let economics = definition.economics();
    assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
    assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(165));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(20)
    );
    assert_eq!(economics.gross_variance_basis_points(), 700);
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        360
    );
}

#[test]
fn authored_slot_machine_route_is_distributed_commercial_gambling() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::SlotMachineRoute);
    assert!(
        definition.required_business_functions().is_empty(),
        "a slot route spans ordinary premises rather than requiring one gambling venue"
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::VehicleFleet,
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ])
    );
    let economics = definition.economics();
    assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
    assert_eq!(economics.demand_revenue_per_point(), Money::from_cents(125));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(85)
    );
    assert_eq!(economics.gross_variance_basis_points(), 1_000);
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        500
    );
}

#[test]
fn authored_brothel_is_a_wealth_sensitive_lodging_vice_venue() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Brothel);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::CashIntensive,
            BusinessFunction::CustomerAccess,
            BusinessFunction::Lodging,
        ])
    );
    assert!(definition.required_network_functions().is_empty());
    let economics = definition.economics();
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(155));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(35)
    );
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        760
    );
}

#[test]
fn authored_prizefighting_requires_a_real_sporting_venue_and_cash_network() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::PrizeFighting);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::MeetingSpace,
            BusinessFunction::CustomerAccess,
            BusinessFunction::SportingVenue,
        ])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([BusinessFunction::CashIntensive])
    );
    let economics = definition.economics();
    assert_eq!(economics.gross_variance_basis_points(), 2_000);
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(70)
    );
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        420
    );
}

#[test]
fn authored_counterfeiting_requires_a_press_and_commercial_passing_network() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Counterfeiting);
    assert_eq!(
        definition.required_business_functions(),
        &BTreeSet::from([
            BusinessFunction::PrintingPress,
            BusinessFunction::ProfessionalRecords,
        ])
    );
    assert_eq!(
        definition.required_network_functions(),
        &BTreeSet::from([
            BusinessFunction::CustomerAccess,
            BusinessFunction::DistributionInfrastructure,
        ])
    );
    assert_eq!(
        definition.network_mode(),
        EnterpriseNetworkMode::SupportingBusinessesOnly
    );
    let economics = definition.economics();
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(125)
    );
    assert_eq!(
        economics.management_revenue_per_point(),
        Money::from_cents(85)
    );
    assert_eq!(economics.gross_variance_basis_points(), 1_400);
    assert_eq!(
        economics.enforcement_attention_basis_points_per_active_case(),
        550
    );
}

#[test]
fn separate_enterprise_network_mode_requires_a_real_network_dependency() {
    let registry = build_registry();
    let definition = registry.get_enterprise(EnterpriseKind::Protection);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_enterprise(
            EnterpriseKind::Protection,
            definition.economics().clone(),
            BTreeSet::new(),
            BTreeSet::new(),
            EnterpriseNetworkMode::SupportingBusinessesOnly,
        ),
        Err(
            RegistryBuildError::SeparateEnterpriseNetworkWithoutRequirements(
                EnterpriseKind::Protection
            )
        )
    ));
}

#[test]
fn authored_construction_business_is_commerce_driven_and_capital_heavy() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Construction)
        .economics();
    assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
    assert_eq!(economics.base_gross(), Money::from_cents(19_000));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(130)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(25));
    assert_eq!(economics.acquisition_cost(), Money::from_cents(105_000));
}

#[test]
fn authored_wholesale_business_is_a_stable_commerce_driven_distribution_front() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Wholesale)
        .economics();
    assert_eq!(economics.cycle(), SimDuration::from_minutes(1_440));
    assert_eq!(economics.base_gross(), Money::from_cents(17_500));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(115)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(15));
    assert_eq!(economics.gross_variance_basis_points(), 650);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(82_000));
}

#[test]
fn authored_pawnshop_is_a_low_capital_neighborhood_commercial_front() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Pawnshop)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(11_500));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(8_500));
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(45));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(65)
    );
    assert_eq!(economics.acquisition_cost(), Money::from_cents(38_000));
}

#[test]
fn authored_coin_machine_distributor_is_commerce_driven_route_infrastructure() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::CoinMachineDistribution)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(15_500));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(105)
    );
    assert_eq!(economics.gross_variance_basis_points(), 750);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(68_000));
}

#[test]
fn authored_lodging_business_is_wealth_driven_and_capital_intensive() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Lodging)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(16_500));
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(95));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(55)
    );
    assert_eq!(economics.acquisition_cost(), Money::from_cents(76_000));
}

#[test]
fn authored_athletic_club_is_a_mid_market_recreation_business() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::AthleticClub)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(12_500));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(10_000));
    assert_eq!(economics.gross_variance_basis_points(), 900);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(45_000));
}

#[test]
fn authored_laundry_is_a_stable_low_capital_front() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Laundry)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(11_000));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(9_000));
    assert_eq!(economics.gross_variance_basis_points(), 550);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(34_000));
}

#[test]
fn authored_printing_business_is_a_stable_commercial_records_front() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Printing)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(13_500));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(10_500));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(85)
    );
    assert_eq!(economics.gross_variance_basis_points(), 650);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(48_000));
}

#[test]
fn authored_financial_services_business_is_wealth_driven_and_record_heavy() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::FinancialServices)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(15_000));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(10_500));
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(120));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(45)
    );
    assert_eq!(economics.acquisition_cost(), Money::from_cents(72_000));
}

#[test]
fn authored_garment_factory_is_capital_heavy_industrial_commerce() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::GarmentFactory)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(18_000));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(15_500));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(125)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(20));
    assert_eq!(economics.gross_variance_basis_points(), 700);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(88_000));
}

#[test]
fn authored_stevedoring_is_capital_heavy_waterfront_commerce() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::Stevedoring)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(19_500));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(17_000));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(140)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(15));
    assert_eq!(economics.gross_variance_basis_points(), 650);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(96_000));
}

#[test]
fn authored_news_service_is_commerce_driven_information_infrastructure() {
    let registry = build_registry();
    let economics = registry
        .get_business(crate::world::BusinessKind::NewsService)
        .economics();
    assert_eq!(economics.base_gross(), Money::from_cents(13_000));
    assert_eq!(economics.base_operating_cost(), Money::from_cents(10_500));
    assert_eq!(
        economics.commerce_revenue_per_point(),
        Money::from_cents(95)
    );
    assert_eq!(economics.wealth_revenue_per_point(), Money::from_cents(30));
    assert_eq!(economics.gross_variance_basis_points(), 550);
    assert_eq!(economics.acquisition_cost(), Money::from_cents(52_000));
}

#[test]
fn authored_police_response_definitions_are_bounded_and_queryable() {
    let registry = build_registry();
    let burglary = registry.get_operation(OperationKind::Burglary).execution();
    assert_eq!(burglary.police_dispatch_threshold(), 20);
    assert_eq!(
        burglary.base_police_response_delay(),
        SimDuration::from_minutes(12)
    );
    assert_eq!(
        burglary.minimum_police_response_delay(),
        SimDuration::from_minutes(3)
    );
    assert_eq!(burglary.patrol_response_reduction_minutes(), 9);
    assert_eq!(
        burglary.operation_entry_offset(),
        Some(SimDuration::from_minutes(10))
    );
    assert_eq!(burglary.police_arrival_difficulty_penalty(), 14);
    assert_eq!(burglary.police_arrival_exposure_penalty(), 18);

    let surveillance = registry
        .get_operation(OperationKind::Surveillance)
        .execution();
    assert_eq!(surveillance.operation_entry_offset(), None);
    assert!(surveillance.police_dispatch_threshold() > burglary.police_dispatch_threshold());
    for kind in ALL_OPERATION_KINDS {
        let response = registry.get_operation(kind).execution();
        assert!(response.police_dispatch_threshold() >= 0);
        assert!(response.police_dispatch_threshold() <= 100);
        assert!(response.minimum_police_response_delay().as_minutes() > 0);
        assert!(
            response.minimum_police_response_delay().as_minutes()
                <= response.base_police_response_delay().as_minutes()
        );
        assert!(response.police_arrival_difficulty_penalty() <= 100);
        assert!(response.police_arrival_exposure_penalty() <= 100);
    }
}

#[test]
fn operation_response_definition_rejects_invalid_timing_thresholds_and_penalties() {
    let (approaches, roles, execution) = burglary_operation_parts();
    let cases = [
        (
            {
                let mut execution = execution.clone();
                execution.police_response.dispatch_threshold = 101;
                execution
            },
            RegistryBuildError::InvalidOperationResponseThreshold(OperationKind::Burglary),
        ),
        (
            {
                let mut execution = execution.clone();
                execution.police_response.minimum_response_delay = SimDuration::from_minutes(0);
                execution
            },
            RegistryBuildError::InvalidOperationResponseDelay(OperationKind::Burglary),
        ),
        (
            {
                let mut execution = execution.clone();
                execution.police_response.patrol_reduction_minutes = 10;
                execution
            },
            RegistryBuildError::InvalidOperationResponseReduction(OperationKind::Burglary),
        ),
        (
            {
                let mut execution = execution.clone();
                execution.police_response.entry_offset = Some(execution.duration());
                execution
            },
            RegistryBuildError::InvalidOperationEntryOffset(OperationKind::Burglary),
        ),
        (
            {
                let mut execution = execution.clone();
                execution.police_response.arrival_exposure_penalty = 101;
                execution
            },
            RegistryBuildError::InvalidOperationResponsePenalty(OperationKind::Burglary),
        ),
    ];
    for (execution, expected_error) in cases {
        let mut builder = RegistryBuilder::default();
        let error = builder
            .register_operation(
                OperationKind::Burglary,
                "Burglary",
                approaches.clone(),
                roles.clone(),
                execution,
            )
            .expect_err("invalid police response authorship must be rejected");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected_error)
        );
    }
}

#[test]
fn operation_definition_rejects_outcome_thresholds_its_own_factors_cannot_reach() {
    let (approaches, roles, execution) = burglary_operation_parts();

    // This remains inside the old project-wide coarse upper bound but exceeds what the
    // burglary definition itself can achieve even with perfect crew, intelligence, and luck.
    let mut impossible_success = execution.clone();
    impossible_success.difficulty.achieved_margin = 100;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            impossible_success,
        ),
        Err(RegistryBuildError::InvalidOperationOutcomeMarginRange(
            OperationKind::Burglary
        ))
    ));

    // Likewise, a partial threshold below the definition's own worst possible margin makes
    // failure impossible and must be rejected even though the number is globally bounded.
    let mut impossible_failure = execution;
    impossible_failure.difficulty.partial_margin = -200;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            impossible_failure,
        ),
        Err(RegistryBuildError::InvalidOperationOutcomeMarginRange(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn operation_definition_rejects_inert_role_weight_and_unreachable_deadline_failure() {
    let (approaches, _roles, mut inert_role_weight) = burglary_operation_parts();
    inert_role_weight.difficulty.role_capabilities.clear();
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            BTreeSet::new(),
            inert_role_weight,
        ),
        Err(RegistryBuildError::InvalidOperationAbilityWeights(
            OperationKind::Burglary
        ))
    ));

    let (_, roles, mut compressed) = burglary_operation_parts();
    compressed.difficulty.duration = SimDuration::from_minutes(10);
    compressed.difficulty.base_difficulty = 0;
    compressed.difficulty.police_pressure_weight = 0;
    compressed.difficulty.max_time_pressure = 100;
    compressed.difficulty.variance_limit = 0;
    compressed.difficulty.partial_margin = -10;
    compressed.difficulty.achieved_margin = 5;
    for adjustment in compressed
        .difficulty
        .approach_difficulty_adjustments
        .values_mut()
    {
        *adjustment = 0;
    }
    compressed.police_response.base_response_delay = SimDuration::from_minutes(1);
    compressed.police_response.minimum_response_delay = SimDuration::from_minutes(1);
    compressed.police_response.patrol_reduction_minutes = 0;
    compressed.police_response.entry_offset = Some(SimDuration::from_minutes(8));
    compressed.police_response.arrival_difficulty_penalty = 0;
    // A legal deadline must be later than minute 8, so minute 9 is the tightest possible
    // completion. Runtime pressure is therefore only 10, making Failed (< -10) unreachable.
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            compressed,
        ),
        Err(RegistryBuildError::InvalidOperationOutcomeMarginRange(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn operation_definition_rejects_exposure_thresholds_its_own_factors_cannot_reach() {
    let (approaches, roles, execution) = burglary_operation_parts();

    let mut impossible_identification = execution.clone();
    impossible_identification.exposure.identifying_threshold = 200;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            impossible_identification,
        ),
        Err(RegistryBuildError::InvalidOperationExposureThresholdRange(
            OperationKind::Burglary
        ))
    ));

    let mut impossible_clean_escape = execution;
    impossible_clean_escape.exposure.trace_threshold = -100;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            impossible_clean_escape,
        ),
        Err(RegistryBuildError::InvalidOperationExposureThresholdRange(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn operation_definition_rejects_unreachable_police_dispatch_threshold() {
    let (approaches, roles, mut execution) = burglary_operation_parts();
    execution.police_response.dispatch_threshold = 100;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            execution,
        ),
        Err(RegistryBuildError::InvalidOperationResponseThresholdRange(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn operation_definition_rejects_response_that_cannot_arrive_before_resolution() {
    let (approaches, roles, mut execution) = burglary_operation_parts();
    let impossible_delay = execution.difficulty.duration.as_minutes() + 1;
    execution.police_response.base_response_delay = SimDuration::from_minutes(impossible_delay);
    execution.police_response.minimum_response_delay = SimDuration::from_minutes(impossible_delay);
    execution.police_response.patrol_reduction_minutes = 0;

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            execution,
        ),
        Err(RegistryBuildError::InvalidOperationResponseDelay(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn operation_definition_rejects_empty_or_inert_approach_authorship() {
    let (approaches, roles, execution) = burglary_operation_parts();

    let mut invalid_weights = execution.clone();
    invalid_weights.difficulty.role_capability_weight = 0;
    invalid_weights.difficulty.leader_capability_weight = 0;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            invalid_weights,
        ),
        Err(RegistryBuildError::InvalidOperationAbilityWeights(
            OperationKind::Burglary
        ))
    ));

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            BTreeSet::new(),
            roles.clone(),
            execution.clone(),
        ),
        Err(RegistryBuildError::MissingOperationApproaches(
            OperationKind::Burglary
        ))
    ));

    let mut difficulty_missing = execution.clone();
    let missing_approach = *approaches
        .first()
        .expect("burglary must have at least one authored approach");
    difficulty_missing
        .difficulty
        .approach_difficulty_adjustments
        .remove(&missing_approach);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            difficulty_missing,
        ),
        Err(RegistryBuildError::MissingOperationApproachAdjustment {
            operation: OperationKind::Burglary,
            approach,
        }) if approach == missing_approach
    ));

    let mut difficulty_extra = execution.clone();
    difficulty_extra
        .difficulty
        .approach_difficulty_adjustments
        .insert(OperationApproach::Violent, 0);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            difficulty_extra,
        ),
        Err(RegistryBuildError::UnexpectedOperationApproachAdjustment {
            operation: OperationKind::Burglary,
            approach: OperationApproach::Violent,
        })
    ));

    let mut excessive_exposure_adjustment = execution.clone();
    excessive_exposure_adjustment
        .exposure
        .approach_adjustments
        .insert(missing_approach, 51);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            excessive_exposure_adjustment,
        ),
        Err(
            RegistryBuildError::InvalidOperationExposureApproachAdjustment(OperationKind::Burglary)
        )
    ));

    let mut exposure_extra = execution;
    exposure_extra
        .exposure
        .approach_adjustments
        .insert(OperationApproach::Violent, 0);
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            exposure_extra,
        ),
        Err(
            RegistryBuildError::UnexpectedOperationExposureApproachAdjustment {
                operation: OperationKind::Burglary,
                approach: OperationApproach::Violent,
            }
        )
    ));
}

#[test]
fn operation_definition_requires_partial_and_repeat_take_penalties_to_be_material() {
    let registry = crate::build_registry();
    let definition = registry.get_operation(OperationKind::Robbery);
    let approaches = definition.supported_approaches().clone();
    let roles = definition.required_roles().clone();
    let mut execution = definition.execution().clone();
    execution
        .cash_proceeds
        .as_mut()
        .expect("robbery must author cash proceeds")
        .partial_take_basis_points = 10_000;

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Robbery,
            "Robbery",
            approaches,
            roles,
            execution,
        ),
        Err(RegistryBuildError::InvalidOperationPartialCashTake(
            OperationKind::Robbery
        ))
    ));

    let definition = registry.get_operation(OperationKind::Burglary);
    let approaches = definition.supported_approaches().clone();
    let roles = definition.required_roles().clone();
    let mut execution = definition.execution().clone();
    execution
        .property_proceeds
        .as_mut()
        .expect("burglary must author property proceeds")
        .immediate_repeat_value_basis_points = 10_000;

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            execution,
        ),
        Err(RegistryBuildError::InvalidOperationTakeRecovery(
            OperationKind::Burglary
        ))
    ));
}

#[test]
fn property_liquidation_police_adjustment_must_change_recovery_in_both_directions() {
    let registry = crate::build_registry();
    let definition = registry.get_operation(OperationKind::Burglary);
    let approaches = definition.supported_approaches().clone();
    let roles = definition.required_roles().clone();

    let mut execution = definition.execution().clone();
    execution
        .property_proceeds
        .as_mut()
        .expect("burglary must author property proceeds")
        .liquidation_police_adjustment_basis_points_per_point = 0;
    assert!(matches!(
        RegistryBuilder::default().register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches.clone(),
            roles.clone(),
            execution,
        ),
        Err(
            RegistryBuildError::InvalidOperationPropertyLiquidationPoliceAdjustment(
                OperationKind::Burglary
            )
        )
    ));

    let mut execution = definition.execution().clone();
    let property = execution
        .property_proceeds
        .as_mut()
        .expect("burglary must author property proceeds");
    property.liquidation_min_recovery_basis_points = property.liquidation_recovery_basis_points;
    assert!(matches!(
        RegistryBuilder::default().register_operation(
            OperationKind::Burglary,
            "Burglary",
            approaches,
            roles,
            execution,
        ),
        Err(
            RegistryBuildError::InvalidOperationPropertyLiquidationPoliceAdjustment(
                OperationKind::Burglary
            )
        )
    ));
}

#[test]
fn executive_brief_definition_rejects_invalid_cadence_attention_and_entry_limit() {
    let valid = ExecutiveBriefDefinitionSpec {
        cadence: SimDuration::from_minutes(1_440),
        minimum_source_attention: AttentionClass::Notable,
        max_source_entries: 8,
    };

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
            cadence: SimDuration::from_minutes(0),
            ..valid
        }),
        Err(RegistryBuildError::InvalidExecutiveBriefCadence)
    ));

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
            minimum_source_attention: AttentionClass::Routine,
            ..valid
        }),
        Err(RegistryBuildError::InvalidExecutiveBriefAttention)
    ));

    for max_source_entries in [0, 101] {
        let mut builder = RegistryBuilder::default();
        assert!(matches!(
            builder.register_executive_brief(ExecutiveBriefDefinitionSpec {
                max_source_entries,
                ..valid
            }),
            Err(RegistryBuildError::InvalidExecutiveBriefEntryLimit)
        ));
    }
}

#[test]
fn recruitment_definition_rejects_zero_duration_and_incomplete_drive_mapping() {
    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.timing.cooldown = SimDuration::from_minutes(0);
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::InvalidRecruitmentDuration)
    ));

    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.approach_drives
        .remove(&RecruitmentApproach::Protection);
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::MissingRecruitmentApproachDrives(
            RecruitmentApproach::Protection
        ))
    ));
}

#[test]
fn recruitment_definition_rejects_unsafe_relationship_math() {
    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.relationships.recruiter_support.divisor = 0;
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights)
    ));

    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.relationships.recruiter_support.trust_weight = 5;
    spec.relationships.recruiter_support.divisor = 1;
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights)
    ));
}

#[test]
fn recruitment_definition_rejects_duplicate_and_non_incumbent_threshold_trait_rules() {
    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.trait_rules.push(spec.trait_rules[0]);
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::DuplicateRecruitmentTraitRule(
            TraitKind::Ambitious
        ))
    ));

    let mut builder = RegistryBuilder::default();
    let mut spec = recruitment_spec();
    spec.trait_rules[0].minimum_incumbent_resentment = Some(0);
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::InvalidRecruitmentTraitRule(
            TraitKind::Ambitious
        ))
    ));
}

#[test]
fn recruitment_definition_rejects_predetermined_outcomes() {
    let mut builder = RegistryBuilder::default();
    let mut always_refuses = recruitment_spec();
    always_refuses.scoring.base_willingness = 0;
    always_refuses.scoring.acceptance_score = 100;
    always_refuses.scoring.weights = RecruitmentWeightsDefinition {
        recruiter_influence: 0,
        drive_alignment: 0,
        relationship_support: 0,
        incumbent_resentment: 0,
        perceived_legal_pressure: 0,
        incumbent_attachment: 0,
        organization_competence: 0,
    };
    always_refuses.trait_rules.clear();
    assert!(matches!(
        builder.register_recruitment(always_refuses),
        Err(RegistryBuildError::InvalidRecruitmentOutcomeRange(
            RecruitmentApproach::FinancialOpportunity
        ))
    ));

    let mut builder = RegistryBuilder::default();
    let mut always_accepts = recruitment_spec();
    always_accepts.scoring.base_willingness = 100;
    always_accepts.scoring.acceptance_score = 0;
    always_accepts.scoring.existing_membership_resistance = 0;
    always_accepts.scoring.weights.incumbent_attachment = 0;
    always_accepts.trait_rules.clear();
    assert!(matches!(
        builder.register_recruitment(always_accepts),
        Err(RegistryBuildError::InvalidRecruitmentOutcomeRange(
            RecruitmentApproach::FinancialOpportunity
        ))
    ));
}

#[test]
fn recruitment_definition_rejects_margin_arithmetic_overflow() {
    let mut spec = recruitment_spec();
    spec.trait_rules.clear();
    // 655 distinct thresholded +50 rules total 32,750, still inside the trait accumulator's
    // i16 range. Combined with ordinary positive recruitment factors they would overflow the
    // persisted i16 margin unless the registry checks the whole scoring expression.
    let mut remaining = 655_usize;
    for trait_kind in ALL_TRAIT_KINDS {
        for threshold in 1..=100_u8 {
            if remaining == 0 {
                break;
            }
            spec.trait_rules.push(RecruitmentTraitRuleDefinition {
                trait_kind,
                approach: Some(RecruitmentApproach::FinancialOpportunity),
                minimum_incumbent_resentment: Some(threshold),
                adjustment: 50,
            });
            remaining -= 1;
        }
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(
        remaining, 0,
        "fixture must author the intended trait-rule volume"
    );

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_recruitment(spec),
        Err(RegistryBuildError::InvalidRecruitmentArithmeticRange)
    ));
}

#[test]
fn investigation_work_definition_rejects_unreachable_resolution_branches() {
    let mut always_develops = investigation_work_spec(InvestigationWorkKind::EvidenceReview);
    // Weak/questionable/inadmissible support averages 11. With the authored 35% support
    // weight, difficulty 45, and variance 12, -54 is the exact minimum reachable margin.
    // Because resolution uses >=, setting the threshold there kills Inconclusive entirely.
    always_develops.connected_margin = -54;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_investigation_work(InvestigationWorkKind::EvidenceReview, always_develops),
        Err(RegistryBuildError::InvalidInvestigationWorkConnectedMargin(
            InvestigationWorkKind::EvidenceReview
        ))
    ));

    let mut never_develops = investigation_work_spec(InvestigationWorkKind::EvidenceReview);
    never_develops.connected_margin = 100;
    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_investigation_work(InvestigationWorkKind::EvidenceReview, never_develops),
        Err(RegistryBuildError::InvalidInvestigationWorkConnectedMargin(
            InvestigationWorkKind::EvidenceReview
        ))
    ));
}

#[test]
fn witness_interview_definition_requires_distinct_reachable_confidence_bands() {
    let mut spec = investigation_work_spec(InvestigationWorkKind::WitnessInterview);
    let outcome = spec
        .interview_outcome
        .as_mut()
        .expect("witness interview must author confidence bands");
    outcome.medium_confidence = outcome.low_confidence;

    let mut builder = RegistryBuilder::default();
    assert!(matches!(
        builder.register_investigation_work(InvestigationWorkKind::WitnessInterview, spec),
        Err(
            RegistryBuildError::InvalidInvestigationWorkInterviewOutcome(
                InvestigationWorkKind::WitnessInterview
            )
        )
    ));
}
