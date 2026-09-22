//! Typed failures for authored registry construction and completeness validation.

use crate::enterprises::EnterpriseKind;
use crate::legal::InvestigationWorkKind;
use crate::operations::{OperationApproach, OperationKind, RoleKind};
use crate::recruitment::RecruitmentApproach;
use crate::world::{BusinessKind, CapabilityKind, DriveKind, PolicyKind, TraitKind};
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
    #[error("duplicate information-quality definition")]
    DuplicateInformationQuality,
    #[error("missing information-quality definition")]
    MissingInformationQuality,
    #[error("information-quality scores must be in 0..=100")]
    InvalidInformationQuality,
    #[error("recruitment cooldown, autonomous cadence, and legal-pressure age must be positive")]
    InvalidRecruitmentDuration,
    #[error("recruitment weights and membership resistance must be in 0..=100")]
    InvalidRecruitmentWeight,
    #[error("recruitment willingness and acceptance scores must be in 0..=100")]
    InvalidRecruitmentScoring,
    #[error("recruitment relationship weighting is invalid")]
    InvalidRecruitmentRelationshipWeights,
    #[error("recruitment must define at least one recruiter capability")]
    MissingRecruitmentCapabilities,
    #[error("recruitment approach {0:?} must define at least one motivating drive")]
    MissingRecruitmentApproachDrives(RecruitmentApproach),
    #[error("recruitment trait rule for {0:?} is outside supported bounds")]
    InvalidRecruitmentTraitRule(TraitKind),
    #[error("recruitment contains a duplicate semantic trait rule for {0:?}")]
    DuplicateRecruitmentTraitRule(TraitKind),
    #[error("combined recruitment trait adjustments exceed supported arithmetic bounds")]
    InvalidRecruitmentTraitAdjustmentTotal,
    #[error("recruitment scoring can overflow its persisted margin range")]
    InvalidRecruitmentArithmeticRange,
    #[error("recruitment approach {0:?} makes either acceptance or refusal unreachable")]
    InvalidRecruitmentOutcomeRange(RecruitmentApproach),
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
    #[error("investigation work {0:?} base difficulty must be in 0..=100")]
    InvalidInvestigationWorkDifficulty(InvestigationWorkKind),
    #[error("investigation work {0:?} source-support weight must be in 0..=100")]
    InvalidInvestigationWorkSupportWeight(InvestigationWorkKind),
    #[error("investigation work {0:?} variance must be in 0..=50")]
    InvalidInvestigationWorkVariance(InvestigationWorkKind),
    #[error("investigation work {0:?} connected margin makes an outcome unreachable")]
    InvalidInvestigationWorkConnectedMargin(InvestigationWorkKind),
    #[error("investigation work {0:?} source-support scores must be in 0..=100")]
    InvalidInvestigationWorkSupportScore(InvestigationWorkKind),
    #[error("investigation work {0:?} interview-outcome tuning does not match the work kind")]
    InvalidInvestigationWorkInterviewOutcome(InvestigationWorkKind),
    #[error("operation {0:?} must have a positive execution duration")]
    InvalidOperationDuration(OperationKind),
    #[error("operation {0:?} base difficulty must be in 0..=100")]
    InvalidOperationDifficulty(OperationKind),
    #[error("operation {0:?} role/leader ability weights must be in 0..=100 and not both zero")]
    InvalidOperationAbilityWeights(OperationKind),
    #[error("operation {0:?} police pressure weight must be in 0..=100")]
    InvalidOperationPoliceWeight(OperationKind),
    #[error("operation {0:?} maximum time pressure must be in 1..=100")]
    InvalidOperationTimePressure(OperationKind),
    #[error("operation {0:?} variance limit must be in 0..=50")]
    InvalidOperationVariance(OperationKind),
    #[error("operation {0:?} outcome margins are ordered incorrectly")]
    InvalidOperationOutcomeMargins(OperationKind),
    #[error(
        "operation {0:?} outcome margins make an outcome unreachable under its authored factors"
    )]
    InvalidOperationOutcomeMarginRange(OperationKind),
    #[error("operation {0:?} business-target definition does not match its objective contract")]
    InvalidOperationBusinessTarget(OperationKind),
    #[error("operation {0:?} approach difficulty adjustments must be in -50..=50")]
    InvalidOperationApproachAdjustment(OperationKind),
    #[error("operation {0:?} must support at least one approach")]
    MissingOperationApproaches(OperationKind),
    #[error("operation {0:?} must define at least one relevant intelligence topic")]
    MissingOperationIntelligenceTopics(OperationKind),
    #[error("operation {0:?} intelligence difficulty reduction must be in 0..=50")]
    InvalidOperationIntelligenceReduction(OperationKind),
    #[error("operation {0:?} intelligence maximum age must be positive")]
    InvalidOperationIntelligenceAge(OperationKind),
    #[error("operation {0:?} patrol-observation bucket must be a positive divisor of one day")]
    InvalidOperationPatrolObservationBucket(OperationKind),
    #[error("operation {0:?} exposure base and weights must be in 0..=100")]
    InvalidOperationExposureWeight(OperationKind),
    #[error("operation {0:?} exposure variance must be in 0..=50")]
    InvalidOperationExposureVariance(OperationKind),
    #[error("operation {0:?} exposure approach adjustments must be in -50..=50")]
    InvalidOperationExposureApproachAdjustment(OperationKind),
    #[error("operation {0:?} exposure thresholds are ordered incorrectly")]
    InvalidOperationExposureThresholds(OperationKind),
    #[error(
        "operation {0:?} exposure thresholds make either no exposure or identifying exposure unreachable"
    )]
    InvalidOperationExposureThresholdRange(OperationKind),
    #[error("operation {0:?} witness/police exposure thresholds are invalid")]
    InvalidOperationWitnessPoliceThresholds(OperationKind),
    #[error("operation {0:?} police response dispatch threshold must be in 0..=100")]
    InvalidOperationResponseThreshold(OperationKind),
    #[error("operation {0:?} police response dispatch threshold is unreachable")]
    InvalidOperationResponseThresholdRange(OperationKind),
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
    #[error("operation {0:?} partial property recovery must be in 1..10000 basis points")]
    InvalidOperationPartialPropertyRecovery(OperationKind),
    #[error("operation {0:?} property liquidation recovery must be in 1..=10000 basis points")]
    InvalidOperationPropertyLiquidationRecovery(OperationKind),
    #[error("operation {0:?} repeat-take recovery tuning is invalid")]
    InvalidOperationTakeRecovery(OperationKind),
    #[error("operation {0:?} property-liquidation police adjustment tuning is invalid")]
    InvalidOperationPropertyLiquidationPoliceAdjustment(OperationKind),
    #[error("operation {0:?} property-proceeds definition does not match its objective contract")]
    OperationPropertyObjectiveContractMismatch(OperationKind),
    #[error("operation {0:?} cash-take business multiplier must be in 1..=100000 basis points")]
    InvalidOperationCashTakeMultiplier(OperationKind),
    #[error("operation {0:?} partial cash take must be in 1..10000 basis points")]
    InvalidOperationPartialCashTake(OperationKind),
    #[error("operation {0:?} cash-proceeds definition does not match its objective contract")]
    OperationCashObjectiveContractMismatch(OperationKind),
    #[error("operation {0:?} proceeds can overflow against a valid registered business gross")]
    OperationProceedsArithmeticOutOfRange(OperationKind),
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
        "operation {operation:?} has a difficulty adjustment for unsupported approach {approach:?}"
    )]
    UnexpectedOperationApproachAdjustment {
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
    #[error(
        "operation {operation:?} has an exposure adjustment for unsupported approach {approach:?}"
    )]
    UnexpectedOperationExposureApproachAdjustment {
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
    #[error("legal off-window patrol presence percent must be in 1..100")]
    InvalidLegalOffWindowPatrolPresencePercent,
    #[error("legal cold-case window must be positive")]
    InvalidLegalColdWindow,
    #[error("legal witness-interview attempt limit must be positive")]
    InvalidLegalInterviewLimit,
    #[error("legal witness-testimony confidence bands or cooperation discounts are invalid")]
    InvalidLegalWitnessTestimony,
    #[error("legal informant decision delay must be positive")]
    InvalidLegalInformantDelay,
    #[error("legal arrest qualifying-source count must be positive")]
    InvalidLegalArrestEvidenceCount,
    #[error("legal informant chance tuning must remain within a 0..=100 percent range")]
    InvalidLegalInformantChance,
    #[error("automatic legal-support retainer must be positive")]
    InvalidLegalAutomaticSupportRetainer,
    #[error("legal maximum detention must be positive and exceed the informant decision delay")]
    InvalidLegalMaximumDetention,
    #[error("missing upkeep configuration definition")]
    MissingUpkeepConfig,
    #[error("duplicate upkeep configuration definition")]
    DuplicateUpkeepConfig,
    #[error("upkeep per-member daily wage must be positive")]
    InvalidUpkeepWage,
    #[error("upkeep per-member wage can overflow a full persistent-character payroll")]
    InvalidUpkeepArithmeticRange,
    #[error("upkeep shortfall resentment increment must be in 1..=100")]
    InvalidUpkeepResentment,
    #[error("duplicate business disruption definition")]
    DuplicateBusinessDisruption,
    #[error("missing business disruption definition")]
    MissingBusinessDisruption,
    #[error("business disruption duration must be positive")]
    InvalidBusinessDisruptionDuration,
    #[error("business disruption gross basis points must be in 1..10000")]
    InvalidBusinessDisruptionGrossBasisPoints,
    #[error("duplicate laundering configuration definition")]
    DuplicateLaunderingConfig,
    #[error("missing laundering configuration definition")]
    MissingLaunderingConfig,
    #[error("laundering fee basis points must be in 1..10000")]
    InvalidLaunderingFee,
    #[error("laundering plausibility basis points must be in 1..=10000")]
    InvalidLaunderingPlausibility,
    #[error("duplicate reputation configuration definition")]
    DuplicateReputationConfig,
    #[error("missing reputation configuration definition")]
    MissingReputationConfig,
    #[error("reputation baseline must be in 0..=100")]
    InvalidReputationBaseline,
    #[error("reputation expansion fear ceiling must be above baseline and at most 100")]
    InvalidReputationCeiling,
    #[error("authored reputation consequence deltas must stay in -25..=25")]
    InvalidReputationDelta,
    #[error("authored reputation consequences must move standing in their named direction")]
    InvalidReputationConsequence,
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
    #[error("business {0:?} authored economics can overflow production settlement arithmetic")]
    BusinessEconomicArithmeticOutOfRange(BusinessKind),
    #[error("business {0:?} gross variance exceeds 5000 basis points")]
    BusinessVarianceOutOfRange(BusinessKind),
    #[error("business {0:?} notable variance threshold exceeds its variance range")]
    BusinessNotableVarianceOutOfRange(BusinessKind),
    #[error("business {0:?} losing-cycle suspension threshold must be at least one cycle")]
    BusinessSuspensionThresholdOutOfRange(BusinessKind),
    #[error("business {0:?} acquisition cost must be positive")]
    InvalidBusinessAcquisitionCost(BusinessKind),
    #[error("enterprise {0:?} must have a positive cycle duration")]
    InvalidEnterpriseCycle(EnterpriseKind),
    #[error("enterprise {0:?} contains a negative authored economic value")]
    NegativeEnterpriseEconomicValue(EnterpriseKind),
    #[error("enterprise {0:?} authored economics can overflow production settlement arithmetic")]
    EnterpriseEconomicArithmeticOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} gross variance exceeds 5000 basis points")]
    EnterpriseVarianceOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} notable variance threshold exceeds its variance range")]
    EnterpriseNotableVarianceOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} losing-cycle suspension threshold must be at least one cycle")]
    EnterpriseSuspensionThresholdOutOfRange(EnterpriseKind),
    #[error("enterprise {0:?} enforcement-attention basis points must be in 0..=10000")]
    EnterpriseEnforcementAttentionOutOfRange(EnterpriseKind),
    #[error(
        "enterprise {0:?} requires separate supporting businesses but defines no network functions"
    )]
    SeparateEnterpriseNetworkWithoutRequirements(EnterpriseKind),
}
