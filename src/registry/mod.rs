//! Immutable code-owned definitions and validated lookup tables loaded once at startup.

use crate::core::attention::AttentionClass;
use crate::core::time::SimDuration;
use crate::enterprises::{EnterpriseKind, ALL_ENTERPRISE_KINDS};
use crate::finance::Money;
use crate::intelligence::{InformationTopic, Reliability, Specificity};
use crate::legal::{EvidenceKind, InvestigationWorkKind, ALL_INVESTIGATION_WORK_KINDS};
use crate::operations::{OperationApproach, OperationKind, RoleKind, ALL_OPERATION_KINDS};
use crate::recruitment::{RecruitmentApproach, ALL_RECRUITMENT_APPROACHES};
use crate::world::{
    BusinessFunction, BusinessKind, CapabilityKind, DriveKind, PolicyKind, PolicySetting,
    TraitKind, ALL_BUSINESS_KINDS, ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_POLICY_KINDS,
    ALL_TRAIT_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct CapabilityDefinition {
    kind: CapabilityKind,
    display_name: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentRelationshipSupportDefinition {
    pub trust_weight: u8,
    pub respect_weight: u8,
    pub affection_weight: u8,
    pub debt_weight: u8,
    pub divisor: u8,
    pub fear_penalty_weight: u8,
    pub fear_penalty_divisor: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentIncumbentRelationshipDefinition {
    pub trust_weight: u8,
    pub respect_weight: u8,
    pub affection_weight: u8,
    pub dependence_weight: u8,
    pub divisor: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentRelationshipDefinition {
    pub recruiter_support: RecruitmentRelationshipSupportDefinition,
    pub incumbent_attachment: RecruitmentIncumbentRelationshipDefinition,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentInformationQualityDefinition {
    pub unknown_reliability: u8,
    pub unreliable_reliability: u8,
    pub mixed_reliability: u8,
    pub generally_reliable: u8,
    pub direct_access: u8,
    pub vague_specificity: u8,
    pub general_specificity: u8,
    pub specific_specificity: u8,
    pub precise_specificity: u8,
}

impl RecruitmentInformationQualityDefinition {
    pub fn reliability_score(self, reliability: Reliability) -> u8 {
        match reliability {
            Reliability::Unknown => self.unknown_reliability,
            Reliability::Unreliable => self.unreliable_reliability,
            Reliability::Mixed => self.mixed_reliability,
            Reliability::GenerallyReliable => self.generally_reliable,
            Reliability::DirectAccess => self.direct_access,
        }
    }

    pub fn specificity_score(self, specificity: Specificity) -> u8 {
        match specificity {
            Specificity::Vague => self.vague_specificity,
            Specificity::General => self.general_specificity,
            Specificity::Specific => self.specific_specificity,
            Specificity::Precise => self.precise_specificity,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentWeightsDefinition {
    pub recruiter_influence: u8,
    pub drive_alignment: u8,
    pub relationship_support: u8,
    pub incumbent_resentment: u8,
    pub perceived_legal_pressure: u8,
    pub incumbent_attachment: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentTimingDefinition {
    pub cooldown: SimDuration,
    pub autonomous_attempt_cadence: SimDuration,
    pub perceived_legal_pressure_max_age: SimDuration,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentScoringDefinition {
    pub base_willingness: i16,
    pub acceptance_score: i16,
    pub existing_membership_resistance: u8,
    pub charismatic_recruiter_bonus: u8,
    pub weights: RecruitmentWeightsDefinition,
}

#[derive(Clone, Copy, Debug)]
pub struct RecruitmentTraitRuleDefinition {
    pub trait_kind: TraitKind,
    pub approach: Option<RecruitmentApproach>,
    pub minimum_incumbent_resentment: Option<u8>,
    pub adjustment: i16,
}

#[derive(Clone, Debug)]
pub struct RecruitmentDefinitionSpec {
    pub timing: RecruitmentTimingDefinition,
    pub scoring: RecruitmentScoringDefinition,
    pub recruiter_capabilities: BTreeSet<CapabilityKind>,
    pub relationships: RecruitmentRelationshipDefinition,
    pub information_quality: RecruitmentInformationQualityDefinition,
    pub approach_drives: BTreeMap<RecruitmentApproach, BTreeSet<DriveKind>>,
    pub trait_rules: Vec<RecruitmentTraitRuleDefinition>,
}

#[derive(Clone, Debug)]
pub struct RecruitmentDefinition {
    timing: RecruitmentTimingDefinition,
    scoring: RecruitmentScoringDefinition,
    recruiter_capabilities: BTreeSet<CapabilityKind>,
    relationships: RecruitmentRelationshipDefinition,
    information_quality: RecruitmentInformationQualityDefinition,
    approach_drives: BTreeMap<RecruitmentApproach, BTreeSet<DriveKind>>,
    trait_rules: Vec<RecruitmentTraitRuleDefinition>,
}

impl RecruitmentDefinition {
    pub fn cooldown(&self) -> SimDuration {
        self.timing.cooldown
    }

    pub fn autonomous_attempt_cadence(&self) -> SimDuration {
        self.timing.autonomous_attempt_cadence
    }

    pub fn base_willingness(&self) -> i16 {
        self.scoring.base_willingness
    }

    pub fn acceptance_score(&self) -> i16 {
        self.scoring.acceptance_score
    }

    pub fn existing_membership_resistance(&self) -> u8 {
        self.scoring.existing_membership_resistance
    }

    pub fn perceived_legal_pressure_max_age(&self) -> SimDuration {
        self.timing.perceived_legal_pressure_max_age
    }

    pub fn weights(&self) -> RecruitmentWeightsDefinition {
        self.scoring.weights
    }

    pub fn recruiter_capabilities(&self) -> &BTreeSet<CapabilityKind> {
        &self.recruiter_capabilities
    }

    pub fn charismatic_recruiter_bonus(&self) -> u8 {
        self.scoring.charismatic_recruiter_bonus
    }

    pub fn relationships(&self) -> RecruitmentRelationshipDefinition {
        self.relationships
    }

    pub fn information_quality(&self) -> RecruitmentInformationQualityDefinition {
        self.information_quality
    }

    pub fn drives_for_approach(&self, approach: RecruitmentApproach) -> &BTreeSet<DriveKind> {
        self.approach_drives
            .get(&approach)
            .unwrap_or_else(|| panic!("missing recruitment drive definition: {approach:?}"))
    }

    pub fn trait_rules(&self) -> &[RecruitmentTraitRuleDefinition] {
        &self.trait_rules
    }
}

#[derive(Clone, Debug)]
pub struct InvestigationWorkDefinition {
    kind: InvestigationWorkKind,
    display_name: &'static str,
    pub(crate) duration: SimDuration,
    pub(crate) base_difficulty: u8,
    pub(crate) additional_source_difficulty: u8,
    pub(crate) source_support_weight: u8,
    pub(crate) variance_limit: u8,
    pub(crate) connected_margin: i16,
}

#[derive(Clone, Copy, Debug)]
pub struct InvestigationWorkDefinitionSpec {
    pub duration: SimDuration,
    pub base_difficulty: u8,
    pub additional_source_difficulty: u8,
    pub source_support_weight: u8,
    pub variance_limit: u8,
    pub connected_margin: i16,
}

impl InvestigationWorkDefinition {
    pub fn kind(&self) -> InvestigationWorkKind {
        self.kind
    }

    pub fn display_name(&self) -> &'static str {
        self.display_name
    }

    pub fn duration(&self) -> SimDuration {
        self.duration
    }

    pub fn base_difficulty(&self) -> u8 {
        self.base_difficulty
    }

    pub fn additional_source_difficulty(&self) -> u8 {
        self.additional_source_difficulty
    }

    pub fn source_support_weight(&self) -> u8 {
        self.source_support_weight
    }

    pub fn variance_limit(&self) -> u8 {
        self.variance_limit
    }

    pub fn connected_margin(&self) -> i16 {
        self.connected_margin
    }
}

impl CapabilityDefinition {
    pub fn kind(&self) -> CapabilityKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
}

#[derive(Clone, Debug)]
pub struct TraitDefinition {
    kind: TraitKind,
    display_name: &'static str,
}

impl TraitDefinition {
    pub fn kind(&self) -> TraitKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
}

#[derive(Clone, Debug)]
pub struct DriveDefinition {
    kind: DriveKind,
    display_name: &'static str,
}

impl DriveDefinition {
    pub fn kind(&self) -> DriveKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
}

#[derive(Clone, Debug)]
pub struct PolicyDefinition {
    kind: PolicyKind,
    display_name: &'static str,
    default: PolicySetting,
}

impl PolicyDefinition {
    pub fn kind(&self) -> PolicyKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
    pub fn default(&self) -> PolicySetting {
        self.default
    }
}

#[derive(Clone, Debug)]
pub struct OperationDefinition {
    kind: OperationKind,
    display_name: &'static str,
    supported_approaches: BTreeSet<OperationApproach>,
    required_roles: BTreeSet<RoleKind>,
    execution: OperationExecutionDefinition,
}

#[derive(Clone, Debug)]
pub struct OperationExecutionDefinition {
    pub(crate) difficulty: OperationDifficultyDefinition,
    pub(crate) leader_capability: CapabilityKind,
    pub(crate) intelligence: OperationIntelligenceDefinition,
    pub(crate) exposure: OperationExposureDefinition,
    pub(crate) police_response: OperationPoliceResponseDefinition,
    pub(crate) property_proceeds: Option<OperationPropertyProceedsDefinition>,
}

#[derive(Clone, Debug)]
pub struct OperationDifficultyDefinition {
    pub(crate) duration: SimDuration,
    pub(crate) base_difficulty: u8,
    pub(crate) role_capabilities: BTreeMap<RoleKind, CapabilityKind>,
    pub(crate) approach_difficulty_adjustments: BTreeMap<OperationApproach, i8>,
    pub(crate) police_pressure_weight: u8,
    pub(crate) variance_limit: u8,
    pub(crate) achieved_margin: i16,
    pub(crate) partial_margin: i16,
}

#[derive(Clone, Debug)]
pub struct OperationIntelligenceDefinition {
    pub(crate) relevant_topics: BTreeSet<InformationTopic>,
    pub(crate) max_difficulty_reduction: u8,
    pub(crate) max_useful_age: SimDuration,
}

#[derive(Clone, Debug)]
pub struct OperationExposureDefinition {
    pub(crate) base_exposure: u8,
    pub(crate) approach_adjustments: BTreeMap<OperationApproach, i8>,
    pub(crate) police_observation_weight: u8,
    pub(crate) stealth_mitigation_weight: u8,
    pub(crate) intelligence_mitigation_weight: u8,
    pub(crate) variance_limit: u8,
    pub(crate) trace_threshold: i16,
    pub(crate) witnessed_threshold: i16,
    pub(crate) identifying_threshold: i16,
    pub(crate) evidence_kind: EvidenceKind,
}

#[derive(Clone, Debug)]
pub struct OperationPoliceResponseDefinition {
    pub(crate) dispatch_threshold: i16,
    pub(crate) base_response_delay: SimDuration,
    pub(crate) minimum_response_delay: SimDuration,
    pub(crate) patrol_reduction_minutes: u16,
    pub(crate) entry_offset: Option<SimDuration>,
    pub(crate) arrival_difficulty_penalty: u8,
    pub(crate) arrival_exposure_penalty: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct OperationPropertyProceedsDefinition {
    pub(crate) business_gross_basis_points: u32,
    pub(crate) partial_recovery_basis_points: u16,
    pub(crate) liquidation_recovery_basis_points: u16,
}

impl OperationExecutionDefinition {
    pub fn duration(&self) -> SimDuration {
        self.difficulty.duration
    }
    pub fn base_difficulty(&self) -> u8 {
        self.difficulty.base_difficulty
    }
    pub fn leader_capability(&self) -> CapabilityKind {
        self.leader_capability
    }
    pub fn capability_for_role(&self, role: RoleKind) -> Option<CapabilityKind> {
        self.difficulty.role_capabilities.get(&role).copied()
    }
    pub fn approach_difficulty_adjustment(&self, approach: OperationApproach) -> Option<i8> {
        self.difficulty
            .approach_difficulty_adjustments
            .get(&approach)
            .copied()
    }
    pub fn police_pressure_weight(&self) -> u8 {
        self.difficulty.police_pressure_weight
    }
    pub fn variance_limit(&self) -> u8 {
        self.difficulty.variance_limit
    }
    pub fn achieved_margin(&self) -> i16 {
        self.difficulty.achieved_margin
    }
    pub fn partial_margin(&self) -> i16 {
        self.difficulty.partial_margin
    }
    pub fn relevant_intelligence_topics(&self) -> &BTreeSet<InformationTopic> {
        &self.intelligence.relevant_topics
    }
    pub fn max_intelligence_difficulty_reduction(&self) -> u8 {
        self.intelligence.max_difficulty_reduction
    }
    pub fn max_intelligence_age(&self) -> SimDuration {
        self.intelligence.max_useful_age
    }
    pub fn base_exposure(&self) -> u8 {
        self.exposure.base_exposure
    }
    pub fn exposure_approach_adjustment(&self, approach: OperationApproach) -> Option<i8> {
        self.exposure.approach_adjustments.get(&approach).copied()
    }
    pub fn police_observation_weight(&self) -> u8 {
        self.exposure.police_observation_weight
    }
    pub fn stealth_mitigation_weight(&self) -> u8 {
        self.exposure.stealth_mitigation_weight
    }
    pub fn intelligence_mitigation_weight(&self) -> u8 {
        self.exposure.intelligence_mitigation_weight
    }
    pub fn exposure_variance_limit(&self) -> u8 {
        self.exposure.variance_limit
    }
    pub fn trace_exposure_threshold(&self) -> i16 {
        self.exposure.trace_threshold
    }
    pub fn witnessed_exposure_threshold(&self) -> i16 {
        self.exposure.witnessed_threshold
    }
    pub fn identifying_exposure_threshold(&self) -> i16 {
        self.exposure.identifying_threshold
    }
    pub fn exposure_evidence_kind(&self) -> EvidenceKind {
        self.exposure.evidence_kind
    }
    pub fn police_dispatch_threshold(&self) -> i16 {
        self.police_response.dispatch_threshold
    }
    pub fn base_police_response_delay(&self) -> SimDuration {
        self.police_response.base_response_delay
    }
    pub fn minimum_police_response_delay(&self) -> SimDuration {
        self.police_response.minimum_response_delay
    }
    pub fn patrol_response_reduction_minutes(&self) -> u16 {
        self.police_response.patrol_reduction_minutes
    }
    pub fn operation_entry_offset(&self) -> Option<SimDuration> {
        self.police_response.entry_offset
    }
    pub fn police_arrival_difficulty_penalty(&self) -> u8 {
        self.police_response.arrival_difficulty_penalty
    }
    pub fn police_arrival_exposure_penalty(&self) -> u8 {
        self.police_response.arrival_exposure_penalty
    }
    pub fn property_proceeds(&self) -> Option<OperationPropertyProceedsDefinition> {
        self.property_proceeds
    }
}

impl OperationPropertyProceedsDefinition {
    pub fn business_gross_basis_points(self) -> u32 {
        self.business_gross_basis_points
    }

    pub fn partial_recovery_basis_points(self) -> u16 {
        self.partial_recovery_basis_points
    }

    pub fn liquidation_recovery_basis_points(self) -> u16 {
        self.liquidation_recovery_basis_points
    }
}

impl OperationDefinition {
    pub fn kind(&self) -> OperationKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
    pub fn supported_approaches(&self) -> &BTreeSet<OperationApproach> {
        &self.supported_approaches
    }
    pub fn required_roles(&self) -> &BTreeSet<RoleKind> {
        &self.required_roles
    }
    pub fn execution(&self) -> &OperationExecutionDefinition {
        &self.execution
    }
}

#[derive(Clone, Debug)]
pub struct EnterpriseEconomicsDefinition {
    pub(crate) cycle: SimDuration,
    pub(crate) base_gross: Money,
    pub(crate) base_operating_cost: Money,
    pub(crate) demand_revenue_per_point: Money,
    pub(crate) commerce_revenue_per_point: Money,
    pub(crate) wealth_revenue_per_point: Money,
    pub(crate) management_revenue_per_point: Money,
    pub(crate) police_cost_per_point: Money,
    pub(crate) gross_variance_basis_points: u16,
    pub(crate) notable_variance_basis_points: u16,
}

impl EnterpriseEconomicsDefinition {
    pub fn cycle(&self) -> SimDuration {
        self.cycle
    }
    pub fn base_gross(&self) -> Money {
        self.base_gross
    }
    pub fn base_operating_cost(&self) -> Money {
        self.base_operating_cost
    }
    pub fn demand_revenue_per_point(&self) -> Money {
        self.demand_revenue_per_point
    }
    pub fn commerce_revenue_per_point(&self) -> Money {
        self.commerce_revenue_per_point
    }
    pub fn wealth_revenue_per_point(&self) -> Money {
        self.wealth_revenue_per_point
    }
    pub fn management_revenue_per_point(&self) -> Money {
        self.management_revenue_per_point
    }
    pub fn police_cost_per_point(&self) -> Money {
        self.police_cost_per_point
    }
    pub fn gross_variance_basis_points(&self) -> u16 {
        self.gross_variance_basis_points
    }
    pub fn notable_variance_basis_points(&self) -> u16 {
        self.notable_variance_basis_points
    }
}

#[derive(Clone, Debug)]
pub struct EnterpriseDefinition {
    kind: EnterpriseKind,
    display_name: &'static str,
    economics: EnterpriseEconomicsDefinition,
    policy: Option<PolicyKind>,
    required_business_functions: BTreeSet<BusinessFunction>,
    required_network_functions: BTreeSet<BusinessFunction>,
}

impl EnterpriseDefinition {
    pub fn kind(&self) -> EnterpriseKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
    pub fn economics(&self) -> &EnterpriseEconomicsDefinition {
        &self.economics
    }
    pub fn policy(&self) -> Option<PolicyKind> {
        self.policy
    }
    pub fn required_business_functions(&self) -> &BTreeSet<BusinessFunction> {
        &self.required_business_functions
    }
    pub fn required_network_functions(&self) -> &BTreeSet<BusinessFunction> {
        &self.required_network_functions
    }
}

#[derive(Clone, Debug)]
pub struct BusinessEconomicsDefinition {
    pub(crate) cycle: SimDuration,
    pub(crate) base_gross: Money,
    pub(crate) base_operating_cost: Money,
    pub(crate) wealth_revenue_per_point: Money,
    pub(crate) commerce_revenue_per_point: Money,
    pub(crate) gross_variance_basis_points: u16,
    pub(crate) notable_variance_basis_points: u16,
}

impl BusinessEconomicsDefinition {
    pub fn cycle(&self) -> SimDuration {
        self.cycle
    }
    pub fn base_gross(&self) -> Money {
        self.base_gross
    }
    pub fn base_operating_cost(&self) -> Money {
        self.base_operating_cost
    }
    pub fn wealth_revenue_per_point(&self) -> Money {
        self.wealth_revenue_per_point
    }
    pub fn commerce_revenue_per_point(&self) -> Money {
        self.commerce_revenue_per_point
    }
    pub fn gross_variance_basis_points(&self) -> u16 {
        self.gross_variance_basis_points
    }
    pub fn notable_variance_basis_points(&self) -> u16 {
        self.notable_variance_basis_points
    }
}

#[derive(Clone, Debug)]
pub struct BusinessDefinition {
    kind: BusinessKind,
    display_name: &'static str,
    typical_functions: BTreeSet<BusinessFunction>,
    economics: BusinessEconomicsDefinition,
}

impl BusinessDefinition {
    pub fn kind(&self) -> BusinessKind {
        self.kind
    }
    pub fn display_name(&self) -> &'static str {
        self.display_name
    }
    pub fn typical_functions(&self) -> &BTreeSet<BusinessFunction> {
        &self.typical_functions
    }
    pub fn economics(&self) -> &BusinessEconomicsDefinition {
        &self.economics
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutiveBriefDefinitionSpec {
    pub cadence: SimDuration,
    pub minimum_source_attention: AttentionClass,
    pub max_source_entries: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutiveBriefDefinition {
    cadence: SimDuration,
    minimum_source_attention: AttentionClass,
    max_source_entries: u16,
}

impl ExecutiveBriefDefinition {
    pub fn cadence(self) -> SimDuration {
        self.cadence
    }

    pub fn minimum_source_attention(self) -> AttentionClass {
        self.minimum_source_attention
    }

    pub fn max_source_entries(self) -> u16 {
        self.max_source_entries
    }
}

#[derive(Clone, Debug)]
pub struct Registry {
    content_revision: u32,
    capabilities: BTreeMap<CapabilityKind, CapabilityDefinition>,
    traits: BTreeMap<TraitKind, TraitDefinition>,
    drives: BTreeMap<DriveKind, DriveDefinition>,
    recruitment: RecruitmentDefinition,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    investigation_work: BTreeMap<InvestigationWorkKind, InvestigationWorkDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
    executive_brief: ExecutiveBriefDefinition,
    legal: LegalConfigDefinition,
}

#[derive(Clone, Copy, Debug)]
pub struct LegalConfigDefinition {
    cold_case_window: SimDuration,
}

impl LegalConfigDefinition {
    /// How long an operation-originated investigation remains institutionally active after its
    /// last evidence/work activity before the owning authority deterministically shelves it.
    pub fn cold_case_window(self) -> SimDuration {
        self.cold_case_window
    }
}

impl Registry {
    pub fn content_revision(&self) -> u32 {
        self.content_revision
    }
    pub fn recruitment(&self) -> &RecruitmentDefinition {
        &self.recruitment
    }
    pub fn legal(&self) -> LegalConfigDefinition {
        self.legal
    }
    pub fn get_capability(&self, kind: CapabilityKind) -> &CapabilityDefinition {
        self.capabilities
            .get(&kind)
            .unwrap_or_else(|| panic!("missing capability definition: {kind:?}"))
    }
    pub fn get_trait(&self, kind: TraitKind) -> &TraitDefinition {
        self.traits
            .get(&kind)
            .unwrap_or_else(|| panic!("missing trait definition: {kind:?}"))
    }
    pub fn get_drive(&self, kind: DriveKind) -> &DriveDefinition {
        self.drives
            .get(&kind)
            .unwrap_or_else(|| panic!("missing drive definition: {kind:?}"))
    }
    pub fn get_policy(&self, kind: PolicyKind) -> &PolicyDefinition {
        self.policies
            .get(&kind)
            .unwrap_or_else(|| panic!("missing policy definition: {kind:?}"))
    }
    pub fn get_operation(&self, kind: OperationKind) -> &OperationDefinition {
        self.operations
            .get(&kind)
            .unwrap_or_else(|| panic!("missing operation definition: {kind:?}"))
    }
    pub fn get_investigation_work(
        &self,
        kind: InvestigationWorkKind,
    ) -> &InvestigationWorkDefinition {
        self.investigation_work
            .get(&kind)
            .unwrap_or_else(|| panic!("missing investigation work definition: {kind:?}"))
    }
    pub fn get_enterprise(&self, kind: EnterpriseKind) -> &EnterpriseDefinition {
        self.enterprises
            .get(&kind)
            .unwrap_or_else(|| panic!("missing enterprise definition: {kind:?}"))
    }
    pub fn get_business(&self, kind: BusinessKind) -> &BusinessDefinition {
        self.businesses
            .get(&kind)
            .unwrap_or_else(|| panic!("missing business definition: {kind:?}"))
    }
    pub fn executive_brief(&self) -> ExecutiveBriefDefinition {
        self.executive_brief
    }
    pub(crate) fn default_policies(&self) -> BTreeMap<PolicyKind, PolicySetting> {
        self.policies
            .iter()
            .map(|(kind, def)| (*kind, def.default()))
            .collect()
    }
}

#[derive(Debug, Error)]
pub(crate) enum RegistryBuildError {
    #[error("duplicate capability definition: {0:?}")]
    DuplicateCapability(CapabilityKind),
    #[error("duplicate trait definition: {0:?}")]
    DuplicateTrait(TraitKind),
    #[error("duplicate drive definition: {0:?}")]
    DuplicateDrive(DriveKind),
    #[error("duplicate recruitment definition")]
    DuplicateRecruitment,
    #[error("duplicate policy definition: {0:?}")]
    DuplicatePolicy(PolicyKind),
    #[error("duplicate operation definition: {0:?}")]
    DuplicateOperation(OperationKind),
    #[error("duplicate enterprise definition: {0:?}")]
    DuplicateEnterprise(EnterpriseKind),
    #[error("duplicate business definition: {0:?}")]
    DuplicateBusiness(BusinessKind),
    #[error("policy default kind mismatch for {0:?}")]
    PolicyDefaultMismatch(PolicyKind),
    #[error("missing capability definition: {0:?}")]
    MissingCapability(CapabilityKind),
    #[error("missing trait definition: {0:?}")]
    MissingTrait(TraitKind),
    #[error("missing drive definition: {0:?}")]
    MissingDrive(DriveKind),
    #[error("missing recruitment definition")]
    MissingRecruitment,
    #[error("recruitment cooldown and legal-pressure age must be positive")]
    InvalidRecruitmentDuration,
    #[error("recruitment weights and membership resistance must be in 0..=100")]
    InvalidRecruitmentWeight,
    #[error("recruitment willingness and acceptance scores must be in 0..=100")]
    InvalidRecruitmentScoring,
    #[error("recruitment relationship weighting is invalid")]
    InvalidRecruitmentRelationshipWeights,
    #[error("recruitment information quality scores must be in 0..=100")]
    InvalidRecruitmentInformationQuality,
    #[error("recruitment must define at least one recruiter capability")]
    MissingRecruitmentCapabilities,
    #[error("recruitment approach {0:?} must define at least one motivating drive")]
    MissingRecruitmentApproachDrives(RecruitmentApproach),
    #[error("recruitment trait rule for {0:?} is outside supported bounds")]
    InvalidRecruitmentTraitRule(TraitKind),
    #[error("combined recruitment trait adjustments exceed supported arithmetic bounds")]
    InvalidRecruitmentTraitAdjustmentTotal,
    #[error("missing policy definition: {0:?}")]
    MissingPolicy(PolicyKind),
    #[error("missing operation definition: {0:?}")]
    MissingOperation(OperationKind),
    #[error("duplicate investigation work definition: {0:?}")]
    DuplicateInvestigationWork(InvestigationWorkKind),
    #[error("missing investigation work definition: {0:?}")]
    MissingInvestigationWork(InvestigationWorkKind),
    #[error("investigation work {0:?} must have a positive duration")]
    InvalidInvestigationWorkDuration(InvestigationWorkKind),
    #[error("investigation work {0:?} difficulty values must be in 0..=100")]
    InvalidInvestigationWorkDifficulty(InvestigationWorkKind),
    #[error("investigation work {0:?} source-support weight must be in 0..=100")]
    InvalidInvestigationWorkSupportWeight(InvestigationWorkKind),
    #[error("investigation work {0:?} variance must be in 0..=50")]
    InvalidInvestigationWorkVariance(InvestigationWorkKind),
    #[error("operation {0:?} must have a positive execution duration")]
    InvalidOperationDuration(OperationKind),
    #[error("operation {0:?} base difficulty must be in 0..=100")]
    InvalidOperationDifficulty(OperationKind),
    #[error("operation {0:?} police pressure weight must be in 0..=100")]
    InvalidOperationPoliceWeight(OperationKind),
    #[error("operation {0:?} variance limit must be in 0..=50")]
    InvalidOperationVariance(OperationKind),
    #[error("operation {0:?} outcome margins are ordered incorrectly")]
    InvalidOperationOutcomeMargins(OperationKind),
    #[error("operation {0:?} must define at least one relevant intelligence topic")]
    MissingOperationIntelligenceTopics(OperationKind),
    #[error("operation {0:?} intelligence difficulty reduction must be in 0..=50")]
    InvalidOperationIntelligenceReduction(OperationKind),
    #[error("operation {0:?} intelligence maximum age must be positive")]
    InvalidOperationIntelligenceAge(OperationKind),
    #[error("operation {0:?} exposure base and weights must be in 0..=100")]
    InvalidOperationExposureWeight(OperationKind),
    #[error("operation {0:?} exposure variance must be in 0..=50")]
    InvalidOperationExposureVariance(OperationKind),
    #[error("operation {0:?} exposure thresholds are ordered incorrectly")]
    InvalidOperationExposureThresholds(OperationKind),
    #[error("operation {0:?} police response dispatch threshold must be in 0..=100")]
    InvalidOperationResponseThreshold(OperationKind),
    #[error("operation {0:?} police response delays are invalid")]
    InvalidOperationResponseDelay(OperationKind),
    #[error("operation {0:?} patrol response reduction exceeds the authored delay range")]
    InvalidOperationResponseReduction(OperationKind),
    #[error("operation {0:?} entry milestone must fall strictly inside execution duration")]
    InvalidOperationEntryOffset(OperationKind),
    #[error("operation {0:?} police arrival penalties must be in 0..=100")]
    InvalidOperationResponsePenalty(OperationKind),
    #[error(
        "operation {0:?} property-proceeds business multiplier must be in 1..=100000 basis points"
    )]
    InvalidOperationPropertyValueMultiplier(OperationKind),
    #[error("operation {0:?} partial property recovery must be in 1..=10000 basis points")]
    InvalidOperationPartialPropertyRecovery(OperationKind),
    #[error("operation {0:?} property liquidation recovery must be in 1..=10000 basis points")]
    InvalidOperationPropertyLiquidationRecovery(OperationKind),
    #[error("operation {operation:?} has no capability mapping for required role {role:?}")]
    MissingOperationRoleCapability {
        operation: OperationKind,
        role: RoleKind,
    },
    #[error(
        "operation {operation:?} has no difficulty adjustment for supported approach {approach:?}"
    )]
    MissingOperationApproachAdjustment {
        operation: OperationKind,
        approach: OperationApproach,
    },
    #[error(
        "operation {operation:?} has no exposure adjustment for supported approach {approach:?}"
    )]
    MissingOperationExposureApproachAdjustment {
        operation: OperationKind,
        approach: OperationApproach,
    },
    #[error("missing enterprise definition: {0:?}")]
    MissingEnterprise(EnterpriseKind),
    #[error("missing business definition: {0:?}")]
    MissingBusiness(BusinessKind),
    #[error("duplicate executive brief definition")]
    DuplicateExecutiveBrief,
    #[error("missing executive brief definition")]
    MissingExecutiveBrief,
    #[error("missing legal configuration definition")]
    MissingLegalConfig,
    #[error("duplicate legal configuration definition")]
    DuplicateLegalConfig,
    #[error("legal cold-case window must be positive")]
    InvalidLegalColdWindow,
    #[error("executive brief cadence must be positive")]
    InvalidExecutiveBriefCadence,
    #[error("executive brief must suppress routine source entries")]
    InvalidExecutiveBriefAttention,
    #[error("executive brief detailed source-entry limit must be in 1..=100")]
    InvalidExecutiveBriefEntryLimit,
    #[error("business {0:?} must have a positive cycle duration")]
    InvalidBusinessCycle(BusinessKind),
    #[error("business {0:?} contains a negative authored economic value")]
    NegativeBusinessEconomicValue(BusinessKind),
    #[error("business {0:?} gross variance exceeds 5000 basis points")]
    BusinessVarianceOutOfRange(BusinessKind),
    #[error("business {0:?} notable variance threshold exceeds its variance range")]
    BusinessNotableVarianceOutOfRange(BusinessKind),
    #[error("enterprise {0:?} must have a positive cycle duration")]
    InvalidEnterpriseCycle(EnterpriseKind),
    #[error("enterprise {0:?} contains a negative authored economic value")]
    NegativeEnterpriseEconomicValue(EnterpriseKind),
    #[error("enterprise {0:?} gross variance exceeds 5000 basis points")]
    EnterpriseVarianceOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} notable variance threshold exceeds its variance range")]
    EnterpriseNotableVarianceOutOfRange(EnterpriseKind),
}

#[derive(Default)]
pub(crate) struct RegistryBuilder {
    capabilities: BTreeMap<CapabilityKind, CapabilityDefinition>,
    traits: BTreeMap<TraitKind, TraitDefinition>,
    drives: BTreeMap<DriveKind, DriveDefinition>,
    recruitment: Option<RecruitmentDefinition>,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    investigation_work: BTreeMap<InvestigationWorkKind, InvestigationWorkDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
    executive_brief: Option<ExecutiveBriefDefinition>,
    legal: Option<LegalConfigDefinition>,
}

impl RegistryBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn register_capability(
        &mut self,
        kind: CapabilityKind,
        display_name: &'static str,
    ) -> Result<(), RegistryBuildError> {
        if self
            .capabilities
            .insert(kind, CapabilityDefinition { kind, display_name })
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateCapability(kind));
        }
        Ok(())
    }
    pub(crate) fn register_legal(
        &mut self,
        cold_case_window: SimDuration,
    ) -> Result<(), RegistryBuildError> {
        if self.legal.is_some() {
            return Err(RegistryBuildError::DuplicateLegalConfig);
        }
        if cold_case_window.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidLegalColdWindow);
        }
        self.legal = Some(LegalConfigDefinition { cold_case_window });
        Ok(())
    }
    pub(crate) fn register_executive_brief(
        &mut self,
        spec: ExecutiveBriefDefinitionSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.executive_brief.is_some() {
            return Err(RegistryBuildError::DuplicateExecutiveBrief);
        }
        if spec.cadence.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidExecutiveBriefCadence);
        }
        if spec.minimum_source_attention == AttentionClass::Routine {
            return Err(RegistryBuildError::InvalidExecutiveBriefAttention);
        }
        if !(1..=100).contains(&spec.max_source_entries) {
            return Err(RegistryBuildError::InvalidExecutiveBriefEntryLimit);
        }
        self.executive_brief = Some(ExecutiveBriefDefinition {
            cadence: spec.cadence,
            minimum_source_attention: spec.minimum_source_attention,
            max_source_entries: spec.max_source_entries,
        });
        Ok(())
    }
    pub(crate) fn register_recruitment(
        &mut self,
        spec: RecruitmentDefinitionSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.recruitment.is_some() {
            return Err(RegistryBuildError::DuplicateRecruitment);
        }
        let RecruitmentDefinitionSpec {
            timing,
            scoring,
            recruiter_capabilities,
            relationships,
            information_quality,
            approach_drives,
            trait_rules,
        } = spec;
        let RecruitmentTimingDefinition {
            cooldown,
            autonomous_attempt_cadence,
            perceived_legal_pressure_max_age,
        } = timing;
        let RecruitmentScoringDefinition {
            base_willingness,
            acceptance_score,
            existing_membership_resistance,
            charismatic_recruiter_bonus,
            weights,
        } = scoring;
        let RecruitmentRelationshipDefinition {
            recruiter_support,
            incumbent_attachment,
        } = relationships;
        let RecruitmentRelationshipSupportDefinition {
            trust_weight: support_trust_weight,
            respect_weight: support_respect_weight,
            affection_weight: support_affection_weight,
            debt_weight: support_debt_weight,
            divisor: support_divisor,
            fear_penalty_weight,
            fear_penalty_divisor,
        } = recruiter_support;
        let RecruitmentIncumbentRelationshipDefinition {
            trust_weight: attachment_trust_weight,
            respect_weight: attachment_respect_weight,
            affection_weight: attachment_affection_weight,
            dependence_weight: attachment_dependence_weight,
            divisor: attachment_divisor,
        } = incumbent_attachment;
        if cooldown.as_minutes() == 0
            || autonomous_attempt_cadence.as_minutes() == 0
            || perceived_legal_pressure_max_age.as_minutes() == 0
        {
            return Err(RegistryBuildError::InvalidRecruitmentDuration);
        }
        if [
            weights.recruiter_influence,
            weights.drive_alignment,
            weights.relationship_support,
            weights.incumbent_resentment,
            weights.perceived_legal_pressure,
            weights.incumbent_attachment,
            existing_membership_resistance,
            charismatic_recruiter_bonus,
        ]
        .into_iter()
        .any(|value| value > 100)
        {
            return Err(RegistryBuildError::InvalidRecruitmentWeight);
        }
        // The residual scoring constants participate in the same bounded 0..=100 margin space as
        // every other recruitment input; an out-of-range willingness or acceptance score would
        // silently skew margins relative to the calibrated capability/relationship/pressure terms.
        if !(0..=100).contains(&base_willingness) || !(0..=100).contains(&acceptance_score) {
            return Err(RegistryBuildError::InvalidRecruitmentScoring);
        }
        if recruiter_capabilities.is_empty() {
            return Err(RegistryBuildError::MissingRecruitmentCapabilities);
        }
        let support_weight_total = u16::from(support_trust_weight)
            + u16::from(support_respect_weight)
            + u16::from(support_affection_weight)
            + u16::from(support_debt_weight);
        let attachment_weight_total = u16::from(attachment_trust_weight)
            + u16::from(attachment_respect_weight)
            + u16::from(attachment_affection_weight)
            + u16::from(attachment_dependence_weight);
        if support_divisor == 0
            || fear_penalty_divisor == 0
            || attachment_divisor == 0
            || support_weight_total == 0
            || attachment_weight_total == 0
            || support_weight_total.saturating_mul(100) / u16::from(support_divisor) > 100
            || attachment_weight_total.saturating_mul(100) / u16::from(attachment_divisor) > 100
            || u16::from(fear_penalty_weight).saturating_mul(100) / u16::from(fear_penalty_divisor)
                > 100
        {
            return Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights);
        }
        if [
            information_quality.unknown_reliability,
            information_quality.unreliable_reliability,
            information_quality.mixed_reliability,
            information_quality.generally_reliable,
            information_quality.direct_access,
            information_quality.vague_specificity,
            information_quality.general_specificity,
            information_quality.specific_specificity,
            information_quality.precise_specificity,
        ]
        .into_iter()
        .any(|score| score > 100)
        {
            return Err(RegistryBuildError::InvalidRecruitmentInformationQuality);
        }
        for approach in ALL_RECRUITMENT_APPROACHES {
            if approach_drives
                .get(&approach)
                .is_none_or(BTreeSet::is_empty)
            {
                return Err(RegistryBuildError::MissingRecruitmentApproachDrives(
                    approach,
                ));
            }
        }
        let mut maximum_absolute_trait_adjustment = 0_i32;
        for rule in &trait_rules {
            if rule
                .minimum_incumbent_resentment
                .is_some_and(|minimum| minimum > 100)
                || !(-50..=50).contains(&rule.adjustment)
            {
                return Err(RegistryBuildError::InvalidRecruitmentTraitRule(
                    rule.trait_kind,
                ));
            }
            maximum_absolute_trait_adjustment = maximum_absolute_trait_adjustment
                .checked_add(i32::from(rule.adjustment).abs())
                .ok_or(RegistryBuildError::InvalidRecruitmentTraitAdjustmentTotal)?;
        }
        if maximum_absolute_trait_adjustment > i32::from(i16::MAX) {
            return Err(RegistryBuildError::InvalidRecruitmentTraitAdjustmentTotal);
        }
        self.recruitment = Some(RecruitmentDefinition {
            timing: RecruitmentTimingDefinition {
                cooldown,
                autonomous_attempt_cadence,
                perceived_legal_pressure_max_age,
            },
            scoring: RecruitmentScoringDefinition {
                base_willingness,
                acceptance_score,
                existing_membership_resistance,
                charismatic_recruiter_bonus,
                weights,
            },
            recruiter_capabilities,
            relationships: RecruitmentRelationshipDefinition {
                recruiter_support: RecruitmentRelationshipSupportDefinition {
                    trust_weight: support_trust_weight,
                    respect_weight: support_respect_weight,
                    affection_weight: support_affection_weight,
                    debt_weight: support_debt_weight,
                    divisor: support_divisor,
                    fear_penalty_weight,
                    fear_penalty_divisor,
                },
                incumbent_attachment: RecruitmentIncumbentRelationshipDefinition {
                    trust_weight: attachment_trust_weight,
                    respect_weight: attachment_respect_weight,
                    affection_weight: attachment_affection_weight,
                    dependence_weight: attachment_dependence_weight,
                    divisor: attachment_divisor,
                },
            },
            information_quality,
            approach_drives,
            trait_rules,
        });
        Ok(())
    }
    pub(crate) fn register_drive(
        &mut self,
        kind: DriveKind,
        display_name: &'static str,
    ) -> Result<(), RegistryBuildError> {
        if self
            .drives
            .insert(kind, DriveDefinition { kind, display_name })
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateDrive(kind));
        }
        Ok(())
    }
    pub(crate) fn register_investigation_work(
        &mut self,
        kind: InvestigationWorkKind,
        display_name: &'static str,
        spec: InvestigationWorkDefinitionSpec,
    ) -> Result<(), RegistryBuildError> {
        if spec.duration.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidInvestigationWorkDuration(kind));
        }
        if spec.base_difficulty > 100 || spec.additional_source_difficulty > 100 {
            return Err(RegistryBuildError::InvalidInvestigationWorkDifficulty(kind));
        }
        if spec.source_support_weight > 100 {
            return Err(RegistryBuildError::InvalidInvestigationWorkSupportWeight(
                kind,
            ));
        }
        if spec.variance_limit > 50 {
            return Err(RegistryBuildError::InvalidInvestigationWorkVariance(kind));
        }
        if self
            .investigation_work
            .insert(
                kind,
                InvestigationWorkDefinition {
                    kind,
                    display_name,
                    duration: spec.duration,
                    base_difficulty: spec.base_difficulty,
                    additional_source_difficulty: spec.additional_source_difficulty,
                    source_support_weight: spec.source_support_weight,
                    variance_limit: spec.variance_limit,
                    connected_margin: spec.connected_margin,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateInvestigationWork(kind));
        }
        Ok(())
    }
    pub(crate) fn register_trait(
        &mut self,
        kind: TraitKind,
        display_name: &'static str,
    ) -> Result<(), RegistryBuildError> {
        if self
            .traits
            .insert(kind, TraitDefinition { kind, display_name })
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateTrait(kind));
        }
        Ok(())
    }
    pub(crate) fn register_policy(
        &mut self,
        kind: PolicyKind,
        display_name: &'static str,
        default: PolicySetting,
    ) -> Result<(), RegistryBuildError> {
        if default.kind() != kind {
            return Err(RegistryBuildError::PolicyDefaultMismatch(kind));
        }
        if self
            .policies
            .insert(
                kind,
                PolicyDefinition {
                    kind,
                    display_name,
                    default,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicatePolicy(kind));
        }
        Ok(())
    }
    pub(crate) fn register_operation(
        &mut self,
        kind: OperationKind,
        display_name: &'static str,
        supported_approaches: BTreeSet<OperationApproach>,
        required_roles: BTreeSet<RoleKind>,
        execution: OperationExecutionDefinition,
    ) -> Result<(), RegistryBuildError> {
        if execution.difficulty.duration.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidOperationDuration(kind));
        }
        if execution.difficulty.base_difficulty > 100 {
            return Err(RegistryBuildError::InvalidOperationDifficulty(kind));
        }
        if execution.difficulty.police_pressure_weight > 100 {
            return Err(RegistryBuildError::InvalidOperationPoliceWeight(kind));
        }
        if execution.difficulty.variance_limit > 50 {
            return Err(RegistryBuildError::InvalidOperationVariance(kind));
        }
        if execution.difficulty.partial_margin >= execution.difficulty.achieved_margin {
            return Err(RegistryBuildError::InvalidOperationOutcomeMargins(kind));
        }
        if execution.intelligence.relevant_topics.is_empty() {
            return Err(RegistryBuildError::MissingOperationIntelligenceTopics(kind));
        }
        if execution.intelligence.max_difficulty_reduction > 50 {
            return Err(RegistryBuildError::InvalidOperationIntelligenceReduction(
                kind,
            ));
        }
        if execution.intelligence.max_useful_age.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidOperationIntelligenceAge(kind));
        }
        if execution.exposure.base_exposure > 100
            || execution.exposure.police_observation_weight > 100
            || execution.exposure.stealth_mitigation_weight > 100
            || execution.exposure.intelligence_mitigation_weight > 100
        {
            return Err(RegistryBuildError::InvalidOperationExposureWeight(kind));
        }
        if execution.exposure.variance_limit > 50 {
            return Err(RegistryBuildError::InvalidOperationExposureVariance(kind));
        }
        if execution.exposure.trace_threshold >= execution.exposure.witnessed_threshold
            || execution.exposure.witnessed_threshold >= execution.exposure.identifying_threshold
        {
            return Err(RegistryBuildError::InvalidOperationExposureThresholds(kind));
        }
        if !(0..=100).contains(&execution.police_response.dispatch_threshold) {
            return Err(RegistryBuildError::InvalidOperationResponseThreshold(kind));
        }
        let base_delay = execution.police_response.base_response_delay.as_minutes();
        let minimum_delay = execution
            .police_response
            .minimum_response_delay
            .as_minutes();
        if base_delay == 0 || minimum_delay == 0 || minimum_delay > base_delay {
            return Err(RegistryBuildError::InvalidOperationResponseDelay(kind));
        }
        if u32::from(execution.police_response.patrol_reduction_minutes)
            > base_delay.saturating_sub(minimum_delay)
        {
            return Err(RegistryBuildError::InvalidOperationResponseReduction(kind));
        }
        if execution
            .police_response
            .entry_offset
            .is_some_and(|offset| {
                offset.as_minutes() == 0
                    || offset.as_minutes() >= execution.difficulty.duration.as_minutes()
            })
        {
            return Err(RegistryBuildError::InvalidOperationEntryOffset(kind));
        }
        if execution.police_response.arrival_difficulty_penalty > 100
            || execution.police_response.arrival_exposure_penalty > 100
        {
            return Err(RegistryBuildError::InvalidOperationResponsePenalty(kind));
        }
        if let Some(property) = execution.property_proceeds {
            if !(1..=100_000).contains(&property.business_gross_basis_points) {
                return Err(RegistryBuildError::InvalidOperationPropertyValueMultiplier(
                    kind,
                ));
            }
            if !(1..=10_000).contains(&property.partial_recovery_basis_points) {
                return Err(RegistryBuildError::InvalidOperationPartialPropertyRecovery(
                    kind,
                ));
            }
            if !(1..=10_000).contains(&property.liquidation_recovery_basis_points) {
                return Err(RegistryBuildError::InvalidOperationPropertyLiquidationRecovery(kind));
            }
        }
        for role in &required_roles {
            if !execution.difficulty.role_capabilities.contains_key(role) {
                return Err(RegistryBuildError::MissingOperationRoleCapability {
                    operation: kind,
                    role: *role,
                });
            }
        }
        for approach in &supported_approaches {
            if !execution
                .difficulty
                .approach_difficulty_adjustments
                .contains_key(approach)
            {
                return Err(RegistryBuildError::MissingOperationApproachAdjustment {
                    operation: kind,
                    approach: *approach,
                });
            }
            if !execution
                .exposure
                .approach_adjustments
                .contains_key(approach)
            {
                return Err(
                    RegistryBuildError::MissingOperationExposureApproachAdjustment {
                        operation: kind,
                        approach: *approach,
                    },
                );
            }
        }
        if self
            .operations
            .insert(
                kind,
                OperationDefinition {
                    kind,
                    display_name,
                    supported_approaches,
                    required_roles,
                    execution,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateOperation(kind));
        }
        Ok(())
    }
    pub(crate) fn register_enterprise(
        &mut self,
        kind: EnterpriseKind,
        display_name: &'static str,
        economics: EnterpriseEconomicsDefinition,
        policy: Option<PolicyKind>,
        required_business_functions: BTreeSet<BusinessFunction>,
        required_network_functions: BTreeSet<BusinessFunction>,
    ) -> Result<(), RegistryBuildError> {
        if economics.cycle.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidEnterpriseCycle(kind));
        }
        let authored_money = [
            economics.base_gross,
            economics.base_operating_cost,
            economics.demand_revenue_per_point,
            economics.commerce_revenue_per_point,
            economics.wealth_revenue_per_point,
            economics.management_revenue_per_point,
            economics.police_cost_per_point,
        ];
        if authored_money.iter().any(|money| money.cents() < 0) {
            return Err(RegistryBuildError::NegativeEnterpriseEconomicValue(kind));
        }
        if economics.gross_variance_basis_points > 5_000 {
            return Err(RegistryBuildError::EnterpriseVarianceOutOfRange(kind));
        }
        if economics.notable_variance_basis_points > economics.gross_variance_basis_points {
            return Err(RegistryBuildError::EnterpriseNotableVarianceOutOfRange(
                kind,
            ));
        }
        if self
            .enterprises
            .insert(
                kind,
                EnterpriseDefinition {
                    kind,
                    display_name,
                    economics,
                    policy,
                    required_business_functions,
                    required_network_functions,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateEnterprise(kind));
        }
        Ok(())
    }
    pub(crate) fn register_business(
        &mut self,
        kind: BusinessKind,
        display_name: &'static str,
        typical_functions: BTreeSet<BusinessFunction>,
        economics: BusinessEconomicsDefinition,
    ) -> Result<(), RegistryBuildError> {
        if economics.cycle.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidBusinessCycle(kind));
        }
        let authored_money = [
            economics.base_gross,
            economics.base_operating_cost,
            economics.wealth_revenue_per_point,
            economics.commerce_revenue_per_point,
        ];
        if authored_money.iter().any(|money| money.cents() < 0) {
            return Err(RegistryBuildError::NegativeBusinessEconomicValue(kind));
        }
        if economics.gross_variance_basis_points > 5_000 {
            return Err(RegistryBuildError::BusinessVarianceOutOfRange(kind));
        }
        if economics.notable_variance_basis_points > economics.gross_variance_basis_points {
            return Err(RegistryBuildError::BusinessNotableVarianceOutOfRange(kind));
        }
        if self
            .businesses
            .insert(
                kind,
                BusinessDefinition {
                    kind,
                    display_name,
                    typical_functions,
                    economics,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateBusiness(kind));
        }
        Ok(())
    }
    pub(crate) fn build(self, content_revision: u32) -> Result<Registry, RegistryBuildError> {
        for kind in ALL_CAPABILITY_KINDS {
            if !self.capabilities.contains_key(&kind) {
                return Err(RegistryBuildError::MissingCapability(kind));
            }
        }
        for kind in ALL_TRAIT_KINDS {
            if !self.traits.contains_key(&kind) {
                return Err(RegistryBuildError::MissingTrait(kind));
            }
        }
        for kind in ALL_DRIVE_KINDS {
            if !self.drives.contains_key(&kind) {
                return Err(RegistryBuildError::MissingDrive(kind));
            }
        }
        for kind in ALL_POLICY_KINDS {
            if !self.policies.contains_key(&kind) {
                return Err(RegistryBuildError::MissingPolicy(kind));
            }
        }
        for kind in ALL_OPERATION_KINDS {
            if !self.operations.contains_key(&kind) {
                return Err(RegistryBuildError::MissingOperation(kind));
            }
        }
        for kind in ALL_INVESTIGATION_WORK_KINDS {
            if !self.investigation_work.contains_key(&kind) {
                return Err(RegistryBuildError::MissingInvestigationWork(kind));
            }
        }
        for kind in ALL_ENTERPRISE_KINDS {
            if !self.enterprises.contains_key(&kind) {
                return Err(RegistryBuildError::MissingEnterprise(kind));
            }
        }
        for kind in ALL_BUSINESS_KINDS {
            if !self.businesses.contains_key(&kind) {
                return Err(RegistryBuildError::MissingBusiness(kind));
            }
        }
        let recruitment = self
            .recruitment
            .ok_or(RegistryBuildError::MissingRecruitment)?;
        let executive_brief = self
            .executive_brief
            .ok_or(RegistryBuildError::MissingExecutiveBrief)?;
        let legal = self.legal.ok_or(RegistryBuildError::MissingLegalConfig)?;
        Ok(Registry {
            content_revision,
            capabilities: self.capabilities,
            traits: self.traits,
            drives: self.drives,
            recruitment,
            policies: self.policies,
            operations: self.operations,
            investigation_work: self.investigation_work,
            enterprises: self.enterprises,
            businesses: self.businesses,
            executive_brief,
            legal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;

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
    fn operation_leaders_use_their_authored_domain_capability() {
        let registry = build_registry();

        assert_eq!(
            registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .leader_capability(),
            CapabilityKind::Management
        );
        assert_eq!(
            registry
                .get_operation(OperationKind::Surveillance)
                .execution()
                .leader_capability(),
            CapabilityKind::Surveillance
        );
        assert_eq!(
            registry
                .get_operation(OperationKind::Bribery)
                .execution()
                .leader_capability(),
            CapabilityKind::Negotiation
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
        assert!(definition
            .trait_rules()
            .iter()
            .any(|rule| rule.trait_kind == TraitKind::EasilyFrightened
                && rule.approach == Some(RecruitmentApproach::Protection)
                && rule.adjustment > 0));
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
    fn authored_alcohol_distribution_requires_concrete_commercial_network() {
        let registry = build_registry();
        let definition = registry.get_enterprise(EnterpriseKind::AlcoholDistribution);
        assert_eq!(definition.kind(), EnterpriseKind::AlcoholDistribution);
        assert_eq!(definition.display_name(), "Alcohol distribution");
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
}
