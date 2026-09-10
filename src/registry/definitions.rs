//! Authored definition types for the immutable registry; `mod.rs` owns the lookup surface and `builder.rs` the validated assembly.

use crate::core::attention::AttentionClass;
use crate::core::time::SimDuration;
use crate::finance::Money;
use crate::intelligence::{InformationTopic, Reliability, Specificity};
use crate::legal::{
    Admissibility, EvidenceKind, EvidenceReliability, EvidenceStrength, WitnessCooperation,
};
use crate::operations::{OperationApproach, RoleKind};
use crate::recruitment::RecruitmentApproach;
use crate::world::{BusinessFunction, CapabilityKind, DriveKind, PolicySetting, TraitKind};
use std::collections::{BTreeMap, BTreeSet};

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
pub struct InformationQualityDefinition {
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
impl InformationQualityDefinition {
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
    pub(crate) fn values(self) -> [u8; 9] {
        [
            self.unknown_reliability,
            self.unreliable_reliability,
            self.mixed_reliability,
            self.generally_reliable,
            self.direct_access,
            self.vague_specificity,
            self.general_specificity,
            self.specific_specificity,
            self.precise_specificity,
        ]
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
    /// How much the recruiting organization's underworld competence reputation sways the
    /// candidate: people join outfits that visibly get things done.
    pub organization_competence: u8,
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
    pub approach_drives: BTreeMap<RecruitmentApproach, BTreeSet<DriveKind>>,
    pub trait_rules: Vec<RecruitmentTraitRuleDefinition>,
}
#[derive(Clone, Debug)]
pub struct RecruitmentDefinition {
    pub(super) timing: RecruitmentTimingDefinition,
    pub(super) scoring: RecruitmentScoringDefinition,
    pub(super) recruiter_capabilities: BTreeSet<CapabilityKind>,
    pub(super) relationships: RecruitmentRelationshipDefinition,
    pub(super) approach_drives: BTreeMap<RecruitmentApproach, BTreeSet<DriveKind>>,
    pub(super) trait_rules: Vec<RecruitmentTraitRuleDefinition>,
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
    pub(crate) duration: SimDuration,
    pub(crate) base_difficulty: u8,
    pub(crate) additional_source_difficulty: u8,
    pub(crate) source_support_weight: u8,
    pub(crate) variance_limit: u8,
    pub(crate) connected_margin: i16,
    pub(crate) source_support: InvestigationSourceSupportDefinition,
    pub(crate) interview_outcome: Option<InvestigationInterviewOutcomeDefinition>,
}
#[derive(Clone, Copy, Debug)]
pub struct InvestigationWorkDefinitionSpec {
    pub duration: SimDuration,
    pub base_difficulty: u8,
    pub additional_source_difficulty: u8,
    pub source_support_weight: u8,
    pub variance_limit: u8,
    pub connected_margin: i16,
    pub source_support: InvestigationSourceSupportDefinition,
    pub interview_outcome: Option<InvestigationInterviewOutcomeDefinition>,
}
#[derive(Clone, Copy, Debug)]
pub struct InvestigationSourceSupportDefinition {
    pub witness_hostile: u8,
    pub witness_reluctant: u8,
    pub witness_cooperative: u8,
    pub evidence_weak: u8,
    pub evidence_corroborating: u8,
    pub evidence_strong: u8,
    pub evidence_direct: u8,
    pub reliability_questionable: u8,
    pub reliability_mixed: u8,
    pub reliability_credible: u8,
    pub reliability_highly_reliable: u8,
    pub admissibility_unknown: u8,
    pub admissibility_inadmissible: u8,
    pub admissibility_disputed: u8,
    pub admissibility_admissible: u8,
}
impl InvestigationSourceSupportDefinition {
    pub fn witness_score(self, cooperation: WitnessCooperation) -> u8 {
        match cooperation {
            WitnessCooperation::Hostile => self.witness_hostile,
            WitnessCooperation::Reluctant => self.witness_reluctant,
            WitnessCooperation::Cooperative => self.witness_cooperative,
        }
    }
    pub fn strength_score(self, strength: EvidenceStrength) -> u8 {
        match strength {
            EvidenceStrength::Weak => self.evidence_weak,
            EvidenceStrength::Corroborating => self.evidence_corroborating,
            EvidenceStrength::Strong => self.evidence_strong,
            EvidenceStrength::Direct => self.evidence_direct,
        }
    }
    pub fn reliability_score(self, reliability: EvidenceReliability) -> u8 {
        match reliability {
            EvidenceReliability::Questionable => self.reliability_questionable,
            EvidenceReliability::Mixed => self.reliability_mixed,
            EvidenceReliability::Credible => self.reliability_credible,
            EvidenceReliability::HighlyReliable => self.reliability_highly_reliable,
        }
    }
    pub fn admissibility_score(self, admissibility: Admissibility) -> u8 {
        match admissibility {
            Admissibility::Unknown => self.admissibility_unknown,
            Admissibility::Inadmissible => self.admissibility_inadmissible,
            Admissibility::Disputed => self.admissibility_disputed,
            Admissibility::Admissible => self.admissibility_admissible,
        }
    }
    pub(crate) fn values(self) -> [u8; 15] {
        [
            self.witness_hostile,
            self.witness_reluctant,
            self.witness_cooperative,
            self.evidence_weak,
            self.evidence_corroborating,
            self.evidence_strong,
            self.evidence_direct,
            self.reliability_questionable,
            self.reliability_mixed,
            self.reliability_credible,
            self.reliability_highly_reliable,
            self.admissibility_unknown,
            self.admissibility_inadmissible,
            self.admissibility_disputed,
            self.admissibility_admissible,
        ]
    }
}
#[derive(Clone, Copy, Debug)]
pub struct InvestigationInterviewOutcomeDefinition {
    pub medium_margin: i16,
    pub high_margin: i16,
    pub low_confidence: u8,
    pub medium_confidence: u8,
    pub high_confidence: u8,
}
impl InvestigationInterviewOutcomeDefinition {
    pub fn confidence_for_margin(self, margin: i16) -> u8 {
        if margin >= self.high_margin {
            self.high_confidence
        } else if margin >= self.medium_margin {
            self.medium_confidence
        } else {
            self.low_confidence
        }
    }
}
impl InvestigationWorkDefinition {
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
    pub fn source_support(&self) -> InvestigationSourceSupportDefinition {
        self.source_support
    }
    pub fn interview_outcome(&self) -> Option<InvestigationInterviewOutcomeDefinition> {
        self.interview_outcome
    }
}
#[derive(Clone, Debug)]
pub struct PolicyDefinition {
    pub(super) default: PolicySetting,
}
impl PolicyDefinition {
    pub fn default(&self) -> PolicySetting {
        self.default
    }
}
#[derive(Clone, Debug)]
pub struct OperationDefinition {
    pub(super) display_name: &'static str,
    pub(super) supported_approaches: BTreeSet<OperationApproach>,
    pub(super) required_roles: BTreeSet<RoleKind>,
    pub(super) execution: OperationExecutionDefinition,
}
#[derive(Clone, Debug)]
pub struct OperationExecutionDefinition {
    pub(crate) difficulty: OperationDifficultyDefinition,
    pub(crate) leader_capability: CapabilityKind,
    pub(crate) intelligence: OperationIntelligenceDefinition,
    pub(crate) exposure: OperationExposureDefinition,
    pub(crate) police_response: OperationPoliceResponseDefinition,
    pub(crate) property_proceeds: Option<OperationPropertyProceedsDefinition>,
    pub(crate) cash_proceeds: Option<OperationCashProceedsDefinition>,
}
#[derive(Clone, Debug)]
pub struct OperationDifficultyDefinition {
    pub(crate) duration: SimDuration,
    pub(crate) base_difficulty: u8,
    pub(crate) role_capabilities: BTreeMap<RoleKind, CapabilityKind>,
    /// Relative contribution of the crew's role-skill average to effective execution ability.
    pub(crate) role_capability_weight: u8,
    /// Relative contribution of the operation leader's authored domain capability.
    pub(crate) leader_capability_weight: u8,
    pub(crate) approach_difficulty_adjustments: BTreeMap<OperationApproach, i8>,
    pub(crate) police_pressure_weight: u8,
    /// Maximum difficulty penalty produced when an operation is forced to finish inside less
    /// time than its authored base duration.
    pub(crate) max_time_pressure: u8,
    pub(crate) variance_limit: u8,
    pub(crate) achieved_margin: i16,
    pub(crate) partial_margin: i16,
}
#[derive(Clone, Debug)]
pub struct OperationIntelligenceDefinition {
    pub(crate) relevant_topics: BTreeSet<InformationTopic>,
    pub(crate) max_difficulty_reduction: u8,
    pub(crate) max_useful_age: SimDuration,
    pub(crate) patrol_observation_bucket: SimDuration,
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
    pub(crate) witness_reluctant_police_presence: u8,
    pub(crate) witness_cooperative_police_presence: u8,
    pub(crate) high_police_presence_narrative_threshold: u8,
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
    pub(crate) recent_take_recovery_window: SimDuration,
    pub(crate) immediate_repeat_value_basis_points: u16,
    pub(crate) liquidation_recovery_basis_points: u16,
    pub(crate) liquidation_police_neutral_rating: u8,
    pub(crate) liquidation_police_adjustment_basis_points_per_point: u16,
    pub(crate) liquidation_min_recovery_basis_points: u16,
    pub(crate) liquidation_max_recovery_basis_points: u16,
}
/// Authored cash-take economics for kinds whose success yields money directly rather
/// than held property: `business_take_basis_points` of the target business's gross
/// potential on a fully achieved objective, scaled by `partial_take_basis_points` on a
/// partial outcome.
#[derive(Clone, Copy, Debug)]
pub struct OperationCashProceedsDefinition {
    pub(crate) business_take_basis_points: u32,
    pub(crate) partial_take_basis_points: u16,
    pub(crate) recent_take_recovery_window: SimDuration,
    pub(crate) immediate_repeat_value_basis_points: u16,
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
    pub fn role_capability_weight(&self) -> u8 {
        self.difficulty.role_capability_weight
    }
    pub fn leader_capability_weight(&self) -> u8 {
        self.difficulty.leader_capability_weight
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
    pub fn max_time_pressure(&self) -> u8 {
        self.difficulty.max_time_pressure
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
    pub fn patrol_observation_bucket(&self) -> SimDuration {
        self.intelligence.patrol_observation_bucket
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
    pub fn witness_reluctant_police_presence(&self) -> u8 {
        self.exposure.witness_reluctant_police_presence
    }
    pub fn witness_cooperative_police_presence(&self) -> u8 {
        self.exposure.witness_cooperative_police_presence
    }
    pub fn high_police_presence_narrative_threshold(&self) -> u8 {
        self.exposure.high_police_presence_narrative_threshold
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

    pub fn cash_proceeds(&self) -> Option<OperationCashProceedsDefinition> {
        self.cash_proceeds
    }
}
impl OperationPropertyProceedsDefinition {
    pub fn business_gross_basis_points(self) -> u32 {
        self.business_gross_basis_points
    }
    pub fn partial_recovery_basis_points(self) -> u16 {
        self.partial_recovery_basis_points
    }
    pub fn recent_take_recovery_window(self) -> SimDuration {
        self.recent_take_recovery_window
    }
    pub fn immediate_repeat_value_basis_points(self) -> u16 {
        self.immediate_repeat_value_basis_points
    }
    pub fn liquidation_recovery_basis_points(self) -> u16 {
        self.liquidation_recovery_basis_points
    }
    pub fn liquidation_police_neutral_rating(self) -> u8 {
        self.liquidation_police_neutral_rating
    }
    pub fn liquidation_police_adjustment_basis_points_per_point(self) -> u16 {
        self.liquidation_police_adjustment_basis_points_per_point
    }
    pub fn liquidation_min_recovery_basis_points(self) -> u16 {
        self.liquidation_min_recovery_basis_points
    }
    pub fn liquidation_max_recovery_basis_points(self) -> u16 {
        self.liquidation_max_recovery_basis_points
    }
}
impl OperationCashProceedsDefinition {
    pub fn business_take_basis_points(self) -> u32 {
        self.business_take_basis_points
    }
    pub fn partial_take_basis_points(self) -> u16 {
        self.partial_take_basis_points
    }
    pub fn recent_take_recovery_window(self) -> SimDuration {
        self.recent_take_recovery_window
    }
    pub fn immediate_repeat_value_basis_points(self) -> u16 {
        self.immediate_repeat_value_basis_points
    }
}
impl OperationDefinition {
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
    /// Per-cycle cost of maintaining each supporting business (supplies, payoffs, silence).
    pub(crate) support_surcharge_per_business: Money,
    /// Per-cycle cost per active investigation targeting the enterprise's neighborhood.
    pub(crate) heat_surcharge_per_active_case: Money,
    /// Per-cycle chance, in basis points, that each active originated case targeting the
    /// enterprise's neighborhood draws a vice inquiry onto this racket (5_000 = even odds
    /// per cycle). Sustained institutional attention eventually finds visible street work.
    pub(crate) vice_attention_basis_points_per_active_case: u16,
    pub(crate) gross_variance_basis_points: u16,
    pub(crate) notable_variance_basis_points: u16,
    /// Consecutive net-losing cycles after which the enterprise's own governance suspends it.
    pub(crate) losing_cycles_before_suspension: u8,
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
    pub fn support_surcharge_per_business(&self) -> Money {
        self.support_surcharge_per_business
    }
    pub fn heat_surcharge_per_active_case(&self) -> Money {
        self.heat_surcharge_per_active_case
    }
    pub fn vice_attention_basis_points_per_active_case(&self) -> u16 {
        self.vice_attention_basis_points_per_active_case
    }
    pub fn gross_variance_basis_points(&self) -> u16 {
        self.gross_variance_basis_points
    }
    pub fn notable_variance_basis_points(&self) -> u16 {
        self.notable_variance_basis_points
    }
    pub fn losing_cycles_before_suspension(&self) -> u8 {
        self.losing_cycles_before_suspension
    }
}
#[derive(Clone, Debug)]
pub struct EnterpriseDefinition {
    pub(super) economics: EnterpriseEconomicsDefinition,
    pub(super) required_business_functions: BTreeSet<BusinessFunction>,
    pub(super) required_network_functions: BTreeSet<BusinessFunction>,
}
impl EnterpriseDefinition {
    pub fn economics(&self) -> &EnterpriseEconomicsDefinition {
        &self.economics
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
    pub(crate) police_cost_per_point: Money,
    pub(crate) gross_variance_basis_points: u16,
    pub(crate) notable_variance_basis_points: u16,
    /// Consecutive net-losing cycles after which the operating economy suspends.
    pub(crate) losing_cycles_before_suspension: u8,
    /// Authored purchase price for acquiring an independently owned business of this kind.
    /// Acquisition pays this from accounted funds, so legitimate expansion is gated on
    /// laundering throughput rather than on raw street cash.
    pub(crate) acquisition_cost: Money,
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
    pub fn police_cost_per_point(&self) -> Money {
        self.police_cost_per_point
    }
    pub fn gross_variance_basis_points(&self) -> u16 {
        self.gross_variance_basis_points
    }
    pub fn notable_variance_basis_points(&self) -> u16 {
        self.notable_variance_basis_points
    }
    pub fn losing_cycles_before_suspension(&self) -> u8 {
        self.losing_cycles_before_suspension
    }
    pub fn acquisition_cost(&self) -> Money {
        self.acquisition_cost
    }
}
#[derive(Clone, Debug)]
pub struct BusinessDefinition {
    pub(super) economics: BusinessEconomicsDefinition,
}
impl BusinessDefinition {
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
    pub(super) cadence: SimDuration,
    pub(super) minimum_source_attention: AttentionClass,
    pub(super) max_source_entries: u16,
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
#[derive(Clone, Copy, Debug)]
pub struct WitnessTestimonyDefinition {
    /// Minimum confidence for Corroborating, Strong, and Direct testimony strength bands.
    pub strength_corroborating_min_confidence: u8,
    pub strength_strong_min_confidence: u8,
    pub strength_direct_min_confidence: u8,
    /// Minimum confidence for Mixed, Credible, and HighlyReliable testimony reliability bands.
    pub reliability_mixed_min_confidence: u8,
    pub reliability_credible_min_confidence: u8,
    pub reliability_highly_reliable_min_confidence: u8,
    /// Qualification-band reductions applied after confidence is classified.
    pub reluctant_band_discount: u8,
    pub hostile_band_discount: u8,
}
impl WitnessTestimonyDefinition {
    pub fn strength_thresholds(self) -> [u8; 3] {
        [
            self.strength_corroborating_min_confidence,
            self.strength_strong_min_confidence,
            self.strength_direct_min_confidence,
        ]
    }
    pub fn reliability_thresholds(self) -> [u8; 3] {
        [
            self.reliability_mixed_min_confidence,
            self.reliability_credible_min_confidence,
            self.reliability_highly_reliable_min_confidence,
        ]
    }
    pub fn cooperation_band_discount(self, cooperation: WitnessCooperation) -> u8 {
        match cooperation {
            WitnessCooperation::Cooperative => 0,
            WitnessCooperation::Reluctant => self.reluctant_band_discount,
            WitnessCooperation::Hostile => self.hostile_band_discount,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LegalConfigSpec {
    /// How long an origin-linked investigation remains institutionally active after
    /// its last evidence/work activity before deterministic shelving.
    pub cold_case_window: SimDuration,
    /// How many completed interviews a case witness may sit through without producing a
    /// statement before investigators stop scheduling further futile interviews.
    pub witness_interview_attempt_limit: u8,
    /// How witness confidence and cooperation become testimony evidence quality.
    pub witness_testimony: WitnessTestimonyDefinition,
    /// How long after detention a detainee faces their single informant-recruitment decision.
    pub informant_decision_delay: SimDuration,
    /// Independent qualifying evidence items required before police autonomously make an arrest.
    pub minimum_arrest_qualifying_evidence: u8,
    /// Baseline chance that a due detainee accepts an informant offer, before personal pressure.
    pub informant_base_flip_chance_percent: u8,
    /// Maximum percentage-point bonus contributed by a detainee's Safety drive at rating 100.
    pub informant_safety_bonus_percent: u8,
    /// Percentage-point reduction when active legal representation exists for the arrest.
    pub represented_informant_reduction_percent: u8,
    /// Flat fee paid when Automatic legal-support policy retains counsel.
    pub automatic_support_retainer: Money,
    /// Maximum continuous detention before the modeled arrest-custody window ends.
    /// Charging, bail, and trial are outside the current simulation scope, so custody itself
    /// must have a bounded lifecycle rather than silently becoming permanent confinement.
    pub maximum_detention: SimDuration,
}
#[derive(Clone, Copy, Debug)]
pub struct LegalConfigDefinition {
    pub(super) cold_case_window: SimDuration,
    pub(super) witness_interview_attempt_limit: u8,
    pub(super) witness_testimony: WitnessTestimonyDefinition,
    pub(super) informant_decision_delay: SimDuration,
    pub(super) minimum_arrest_qualifying_evidence: u8,
    pub(super) informant_base_flip_chance_percent: u8,
    pub(super) informant_safety_bonus_percent: u8,
    pub(super) represented_informant_reduction_percent: u8,
    pub(super) automatic_support_retainer: Money,
    pub(super) maximum_detention: SimDuration,
}
impl LegalConfigDefinition {
    pub fn cold_case_window(self) -> SimDuration {
        self.cold_case_window
    }
    pub fn witness_interview_attempt_limit(self) -> u8 {
        self.witness_interview_attempt_limit
    }
    pub fn witness_testimony(self) -> WitnessTestimonyDefinition {
        self.witness_testimony
    }
    pub fn informant_decision_delay(self) -> SimDuration {
        self.informant_decision_delay
    }
    pub fn minimum_arrest_qualifying_evidence(self) -> u8 {
        self.minimum_arrest_qualifying_evidence
    }
    pub fn informant_base_flip_chance_percent(self) -> u8 {
        self.informant_base_flip_chance_percent
    }
    pub fn informant_safety_bonus_percent(self) -> u8 {
        self.informant_safety_bonus_percent
    }
    pub fn represented_informant_reduction_percent(self) -> u8 {
        self.represented_informant_reduction_percent
    }
    pub fn automatic_support_retainer(self) -> Money {
        self.automatic_support_retainer
    }
    pub fn maximum_detention(self) -> SimDuration {
        self.maximum_detention
    }
}
#[derive(Clone, Copy, Debug)]
pub struct UpkeepConfigSpec {
    pub per_member_daily: Money,
    /// Resentment increment applied to each unpaid member's relationship toward their
    /// supervisor when a payroll run cannot fully fund itself.
    pub shortfall_resentment: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct UpkeepConfigDefinition {
    pub(super) per_member_daily: Money,
    pub(super) shortfall_resentment: u8,
}
impl UpkeepConfigDefinition {
    /// Daily wage owed per active organization member.
    pub fn per_member_daily(self) -> Money {
        self.per_member_daily
    }
    pub fn shortfall_resentment(self) -> u8 {
        self.shortfall_resentment
    }
}
#[derive(Clone, Copy, Debug)]
pub struct BusinessDisruptionSpec {
    /// How long a successful sabotage keeps the target's economy degraded.
    pub duration: SimDuration,
    /// Basis points of normal gross revenue a disrupted business earns per cycle
    /// (for example 4_000 = 40 percent of normal).
    pub gross_basis_points: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct BusinessDisruptionDefinition {
    pub(super) duration: SimDuration,
    pub(super) gross_basis_points: u32,
}
impl BusinessDisruptionDefinition {
    pub fn duration(self) -> SimDuration {
        self.duration
    }
    pub fn gross_basis_points(self) -> u32 {
        self.gross_basis_points
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LaunderingConfigSpec {
    /// Basis points of each laundered transfer kept by the front business as revenue
    /// (for example 1_500 = a 15 percent laundering cut).
    pub fee_basis_points: u32,
    /// Maximum single-transfer size as basis points of the front's legitimate gross
    /// potential (for example 5_000 = half of one cycle's legitimate gross).
    pub plausibility_gross_basis_points: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct LaunderingConfigDefinition {
    pub(super) fee_basis_points: u32,
    pub(super) plausibility_gross_basis_points: u32,
}
impl LaunderingConfigDefinition {
    pub fn fee_basis_points(self) -> u32 {
        self.fee_basis_points
    }
    pub fn plausibility_gross_basis_points(self) -> u32 {
        self.plausibility_gross_basis_points
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ReputationConfigSpec {
    /// The unremarkable standing every untouched impression sits at.
    pub baseline: u8,
    /// Points a touched impression drifts toward the baseline per campaign day.
    pub daily_decay_step: u8,
    /// Police fear at or above which a governed organization suspends delegated
    /// expansion for the day: outfits keep their head down while visibly hot.
    pub expansion_police_fear_ceiling: u8,
    pub witnessed_exposure_police_fear: i8,
    pub identifying_exposure_police_fear: i8,
    /// Police fear applied when one of the organization's own rackets draws a dedicated vice
    /// inquiry: a case built on the racket itself is at least as alarming as being seen.
    pub vice_inquiry_police_fear: i8,
    pub achieved_underworld_competence: i8,
    pub partial_underworld_competence: i8,
    pub violent_businesses_fear: i8,
}
#[derive(Clone, Copy, Debug)]
pub struct ReputationConfigDefinition {
    pub(super) baseline: u8,
    pub(super) daily_decay_step: u8,
    pub(super) expansion_police_fear_ceiling: u8,
    pub(super) witnessed_exposure_police_fear: i8,
    pub(super) identifying_exposure_police_fear: i8,
    pub(super) vice_inquiry_police_fear: i8,
    pub(super) achieved_underworld_competence: i8,
    pub(super) partial_underworld_competence: i8,
    pub(super) violent_businesses_fear: i8,
}
impl ReputationConfigDefinition {
    pub fn baseline(self) -> u8 {
        self.baseline
    }
    pub fn daily_decay_step(self) -> u8 {
        self.daily_decay_step
    }
    pub fn expansion_police_fear_ceiling(self) -> u8 {
        self.expansion_police_fear_ceiling
    }
    pub fn witnessed_exposure_police_fear(self) -> i8 {
        self.witnessed_exposure_police_fear
    }
    pub fn identifying_exposure_police_fear(self) -> i8 {
        self.identifying_exposure_police_fear
    }
    pub fn vice_inquiry_police_fear(self) -> i8 {
        self.vice_inquiry_police_fear
    }
    pub fn achieved_underworld_competence(self) -> i8 {
        self.achieved_underworld_competence
    }
    pub fn partial_underworld_competence(self) -> i8 {
        self.partial_underworld_competence
    }
    pub fn violent_businesses_fear(self) -> i8 {
        self.violent_businesses_fear
    }
}
