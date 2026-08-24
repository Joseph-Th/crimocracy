//! Validated registry assembly: registration methods, completeness and range checks, and
//! the typed build error.

use super::definitions::*;
use super::{LegalConfigDefinition, Registry};
use crate::core::attention::AttentionClass;
use crate::enterprises::{EnterpriseKind, ALL_ENTERPRISE_KINDS};
use crate::legal::{InvestigationWorkKind, ALL_INVESTIGATION_WORK_KINDS};
use crate::operations::{OperationApproach, OperationKind, RoleKind, ALL_OPERATION_KINDS};
use crate::recruitment::{RecruitmentApproach, ALL_RECRUITMENT_APPROACHES};
use crate::world::{
    BusinessFunction, BusinessKind, CapabilityKind, DriveKind, PolicyKind, PolicySetting,
    TraitKind, ALL_BUSINESS_KINDS, ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_POLICY_KINDS,
    ALL_TRAIT_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

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
    #[error("operation {0:?} property-proceeds definition does not match its objective contract")]
    OperationPropertyObjectiveContractMismatch(OperationKind),
    #[error("operation {0:?} cash-take business multiplier must be in 1..=100000 basis points")]
    InvalidOperationCashTakeMultiplier(OperationKind),
    #[error("operation {0:?} partial cash take must be nonzero and no greater than the full take")]
    InvalidOperationPartialCashTake(OperationKind),
    #[error("operation {0:?} cash-proceeds definition does not match its objective contract")]
    OperationCashObjectiveContractMismatch(OperationKind),
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
    #[error("legal witness-interview attempt limit must be positive")]
    InvalidLegalInterviewLimit,
    #[error("legal informant decision delay must be positive")]
    InvalidLegalInformantDelay,
    #[error("missing upkeep configuration definition")]
    MissingUpkeepConfig,
    #[error("duplicate upkeep configuration definition")]
    DuplicateUpkeepConfig,
    #[error("upkeep per-member daily wage must be positive")]
    InvalidUpkeepWage,
    #[error("upkeep shortfall resentment increment must be positive")]
    InvalidUpkeepResentment,
    #[error("duplicate business disruption definition")]
    DuplicateBusinessDisruption,
    #[error("missing business disruption definition")]
    MissingBusinessDisruption,
    #[error("business disruption duration must be positive")]
    InvalidBusinessDisruptionDuration,
    #[error("business disruption gross basis points must be in 1..=10000")]
    InvalidBusinessDisruptionGrossBasisPoints,
    #[error("duplicate laundering configuration definition")]
    DuplicateLaunderingConfig,
    #[error("missing laundering configuration definition")]
    MissingLaunderingConfig,
    #[error("laundering fee basis points must be in 1..=10000")]
    InvalidLaunderingFee,
    #[error("laundering plausibility basis points must be in 1..=10000")]
    InvalidLaunderingPlausibility,
    #[error("duplicate reputation configuration definition")]
    DuplicateReputationConfig,
    #[error("missing reputation configuration definition")]
    MissingReputationConfig,
    #[error("reputation baseline must be in 0..=100")]
    InvalidReputationBaseline,
    #[error("reputation expansion fear ceiling must be in 0..=100")]
    InvalidReputationCeiling,
    #[error("authored reputation consequence deltas must stay in -25..=25")]
    InvalidReputationDelta,
    #[error("reputation daily decay step must be positive")]
    InvalidReputationDecayStep,
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
    #[error("business {0:?} losing-cycle suspension threshold must be at least one cycle")]
    BusinessSuspensionThresholdOutOfRange(BusinessKind),
    #[error("enterprise {0:?} must have a positive cycle duration")]
    InvalidEnterpriseCycle(EnterpriseKind),
    #[error("enterprise {0:?} contains a negative authored economic value")]
    NegativeEnterpriseEconomicValue(EnterpriseKind),
    #[error("enterprise {0:?} gross variance exceeds 5000 basis points")]
    EnterpriseVarianceOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} notable variance threshold exceeds its variance range")]
    EnterpriseNotableVarianceOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} losing-cycle suspension threshold must be at least one cycle")]
    EnterpriseSuspensionThresholdOutOfRange(EnterpriseKind),
}

#[derive(Default)]
pub(crate) struct RegistryBuilder {
    capabilities: BTreeSet<CapabilityKind>,
    traits: BTreeSet<TraitKind>,
    drives: BTreeSet<DriveKind>,
    recruitment: Option<RecruitmentDefinition>,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    investigation_work: BTreeMap<InvestigationWorkKind, InvestigationWorkDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
    executive_brief: Option<ExecutiveBriefDefinition>,
    legal: Option<LegalConfigDefinition>,
    upkeep: Option<UpkeepConfigDefinition>,
    business_disruption: Option<BusinessDisruptionDefinition>,
    laundering: Option<LaunderingConfigDefinition>,
    reputation: Option<ReputationConfigDefinition>,
}

impl RegistryBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn register_capability(
        &mut self,
        kind: CapabilityKind,
    ) -> Result<(), RegistryBuildError> {
        if !self.capabilities.insert(kind) {
            return Err(RegistryBuildError::DuplicateCapability(kind));
        }
        Ok(())
    }
    pub(crate) fn register_legal(
        &mut self,
        spec: LegalConfigSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.legal.is_some() {
            return Err(RegistryBuildError::DuplicateLegalConfig);
        }
        if spec.cold_case_window.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidLegalColdWindow);
        }
        if spec.witness_interview_attempt_limit == 0 {
            return Err(RegistryBuildError::InvalidLegalInterviewLimit);
        }
        if spec.informant_decision_delay.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidLegalInformantDelay);
        }
        self.legal = Some(LegalConfigDefinition {
            cold_case_window: spec.cold_case_window,
            witness_interview_attempt_limit: spec.witness_interview_attempt_limit,
            informant_decision_delay: spec.informant_decision_delay,
        });
        Ok(())
    }
    pub(crate) fn register_upkeep(
        &mut self,
        spec: UpkeepConfigSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.upkeep.is_some() {
            return Err(RegistryBuildError::DuplicateUpkeepConfig);
        }
        if spec.per_member_daily.cents() <= 0 {
            return Err(RegistryBuildError::InvalidUpkeepWage);
        }
        if spec.shortfall_resentment == 0 {
            return Err(RegistryBuildError::InvalidUpkeepResentment);
        }
        self.upkeep = Some(UpkeepConfigDefinition {
            per_member_daily: spec.per_member_daily,
            shortfall_resentment: spec.shortfall_resentment,
        });
        Ok(())
    }
    pub(crate) fn register_business_disruption(
        &mut self,
        spec: BusinessDisruptionSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.business_disruption.is_some() {
            return Err(RegistryBuildError::DuplicateBusinessDisruption);
        }
        if spec.duration.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidBusinessDisruptionDuration);
        }
        if spec.gross_basis_points == 0 || spec.gross_basis_points > 10_000 {
            return Err(RegistryBuildError::InvalidBusinessDisruptionGrossBasisPoints);
        }
        self.business_disruption = Some(BusinessDisruptionDefinition {
            duration: spec.duration,
            gross_basis_points: spec.gross_basis_points,
        });
        Ok(())
    }
    pub(crate) fn register_laundering(
        &mut self,
        spec: LaunderingConfigSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.laundering.is_some() {
            return Err(RegistryBuildError::DuplicateLaunderingConfig);
        }
        if spec.fee_basis_points == 0 || spec.fee_basis_points > 10_000 {
            return Err(RegistryBuildError::InvalidLaunderingFee);
        }
        if spec.plausibility_gross_basis_points == 0
            || spec.plausibility_gross_basis_points > 10_000
        {
            return Err(RegistryBuildError::InvalidLaunderingPlausibility);
        }
        self.laundering = Some(LaunderingConfigDefinition {
            fee_basis_points: spec.fee_basis_points,
            plausibility_gross_basis_points: spec.plausibility_gross_basis_points,
        });
        Ok(())
    }
    pub(crate) fn register_reputation(
        &mut self,
        spec: ReputationConfigSpec,
    ) -> Result<(), RegistryBuildError> {
        if self.reputation.is_some() {
            return Err(RegistryBuildError::DuplicateReputationConfig);
        }
        if spec.baseline > 100 {
            return Err(RegistryBuildError::InvalidReputationBaseline);
        }
        if spec.expansion_police_fear_ceiling > 100 {
            return Err(RegistryBuildError::InvalidReputationCeiling);
        }
        for delta in [
            spec.witnessed_exposure_police_fear,
            spec.identifying_exposure_police_fear,
            spec.achieved_underworld_competence,
            spec.partial_underworld_competence,
            spec.violent_businesses_fear,
        ] {
            // A single authored consequence must move an impression by a bounded step so no
            // one event flips an audience's standing outright.
            if !(-25..=25).contains(&delta) {
                return Err(RegistryBuildError::InvalidReputationDelta);
            }
        }
        if spec.daily_decay_step == 0 {
            // A zero decay step would make decay a structural no-op: impressions never
            // recover and faded records are never erased.
            return Err(RegistryBuildError::InvalidReputationDecayStep);
        }
        self.reputation = Some(ReputationConfigDefinition {
            baseline: spec.baseline,
            daily_decay_step: spec.daily_decay_step,
            expansion_police_fear_ceiling: spec.expansion_police_fear_ceiling,
            witnessed_exposure_police_fear: spec.witnessed_exposure_police_fear,
            identifying_exposure_police_fear: spec.identifying_exposure_police_fear,
            achieved_underworld_competence: spec.achieved_underworld_competence,
            partial_underworld_competence: spec.partial_underworld_competence,
            violent_businesses_fear: spec.violent_businesses_fear,
        });
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
            weights.organization_competence,
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
    pub(crate) fn register_drive(&mut self, kind: DriveKind) -> Result<(), RegistryBuildError> {
        if !self.drives.insert(kind) {
            return Err(RegistryBuildError::DuplicateDrive(kind));
        }
        Ok(())
    }
    pub(crate) fn register_investigation_work(
        &mut self,
        kind: InvestigationWorkKind,
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
    pub(crate) fn register_trait(&mut self, kind: TraitKind) -> Result<(), RegistryBuildError> {
        if !self.traits.insert(kind) {
            return Err(RegistryBuildError::DuplicateTrait(kind));
        }
        Ok(())
    }
    pub(crate) fn register_policy(
        &mut self,
        kind: PolicyKind,
        default: PolicySetting,
    ) -> Result<(), RegistryBuildError> {
        if default.kind() != kind {
            return Err(RegistryBuildError::PolicyDefaultMismatch(kind));
        }
        if self
            .policies
            .insert(kind, PolicyDefinition { default })
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
        if execution.property_proceeds.is_some() != kind.can_acquire_property() {
            return Err(RegistryBuildError::OperationPropertyObjectiveContractMismatch(kind));
        }
        if let Some(cash) = execution.cash_proceeds {
            if !(1..=100_000).contains(&cash.business_take_basis_points) {
                return Err(RegistryBuildError::InvalidOperationCashTakeMultiplier(kind));
            }
            let partial = u32::from(cash.partial_take_basis_points);
            if partial == 0 || partial > cash.business_take_basis_points {
                return Err(RegistryBuildError::InvalidOperationPartialCashTake(kind));
            }
        }
        if execution.cash_proceeds.is_some() != kind.can_take_cash() {
            return Err(RegistryBuildError::OperationCashObjectiveContractMismatch(
                kind,
            ));
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
        economics: EnterpriseEconomicsDefinition,
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
            // Surcharges are applied as per-cycle costs at settlement; a negative value
            // would silently turn active investigations or supporting businesses revenue.
            economics.support_surcharge_per_business,
            economics.heat_surcharge_per_active_case,
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
        if economics.losing_cycles_before_suspension == 0 {
            return Err(RegistryBuildError::EnterpriseSuspensionThresholdOutOfRange(
                kind,
            ));
        }
        if self
            .enterprises
            .insert(
                kind,
                EnterpriseDefinition {
                    economics,
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
            economics.police_cost_per_point,
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
        if economics.losing_cycles_before_suspension == 0 {
            return Err(RegistryBuildError::BusinessSuspensionThresholdOutOfRange(
                kind,
            ));
        }
        if self
            .businesses
            .insert(kind, BusinessDefinition { economics })
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateBusiness(kind));
        }
        Ok(())
    }
    pub(crate) fn build(self, content_revision: u32) -> Result<Registry, RegistryBuildError> {
        for kind in ALL_CAPABILITY_KINDS {
            if !self.capabilities.contains(&kind) {
                return Err(RegistryBuildError::MissingCapability(kind));
            }
        }
        for kind in ALL_TRAIT_KINDS {
            if !self.traits.contains(&kind) {
                return Err(RegistryBuildError::MissingTrait(kind));
            }
        }
        for kind in ALL_DRIVE_KINDS {
            if !self.drives.contains(&kind) {
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
        let upkeep = self.upkeep.ok_or(RegistryBuildError::MissingUpkeepConfig)?;
        let business_disruption = self
            .business_disruption
            .ok_or(RegistryBuildError::MissingBusinessDisruption)?;
        let laundering = self
            .laundering
            .ok_or(RegistryBuildError::MissingLaunderingConfig)?;
        let reputation = self
            .reputation
            .ok_or(RegistryBuildError::MissingReputationConfig)?;
        Ok(Registry {
            content_revision,
            recruitment,
            policies: self.policies,
            operations: self.operations,
            investigation_work: self.investigation_work,
            enterprises: self.enterprises,
            businesses: self.businesses,
            executive_brief,
            legal,
            upkeep,
            business_disruption,
            laundering,
            reputation,
        })
    }
}
