//! Validated registry assembly: registration methods plus completeness and range checks.
//!
//! Sibling `build_error` owns the typed construction failures; domain-specific validation stays
//! in its existing validation modules or in the registration method that owns the definition.

pub(crate) use super::build_error::RegistryBuildError;
use super::definitions::*;
use super::{LegalConfigDefinition, Registry};
use crate::core::attention::AttentionClass;
use crate::enterprises::{ALL_ENTERPRISE_KINDS, EnterpriseKind};
use crate::legal::{ALL_INVESTIGATION_WORK_KINDS, InvestigationWorkKind};
use crate::operations::{ALL_OPERATION_KINDS, OperationApproach, OperationKind, RoleKind};
use crate::world::{
    ALL_BUSINESS_KINDS, ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_POLICY_KINDS, ALL_TRAIT_KINDS,
    BusinessFunction, BusinessKind, CapabilityKind, DriveKind, PolicyKind, PolicySetting,
    TraitKind,
};
use std::collections::{BTreeMap, BTreeSet};

/// Registry-time proof that a nonnegative gross composition remains representable after its
/// largest authored positive variance. Runtime uses checked `Money` arithmetic because state is
/// still an external input, but authored values themselves must never be able to make a normal
/// settlement fail.
fn maximum_varied_gross_fits_i64(
    base_cents: i64,
    per_point_cents: &[i64],
    variance_basis_points: u16,
) -> bool {
    let mut gross = i128::from(base_cents);
    for per_point in per_point_cents {
        gross += i128::from(*per_point) * 100;
    }
    let factor = 10_000_i128 + i128::from(variance_basis_points);
    crate::finance::helpers::round_basis_point_product(gross * factor)
        .is_some_and(|varied| varied <= i128::from(i64::MAX))
}

fn maximum_business_gross_cents(economics: &BusinessEconomicsDefinition) -> i128 {
    i128::from(economics.base_gross.cents())
        + i128::from(economics.wealth_revenue_per_point.cents()) * 100
        + i128::from(economics.commerce_revenue_per_point.cents()) * 100
}

fn operation_proceeds_fit_business_gross(
    definition: &OperationDefinition,
    economics: &BusinessEconomicsDefinition,
) -> bool {
    let multiplier = definition
        .execution()
        .property_proceeds()
        .map(OperationPropertyProceedsDefinition::business_gross_basis_points)
        .or_else(|| {
            definition
                .execution()
                .cash_proceeds()
                .map(OperationCashProceedsDefinition::business_take_basis_points)
        });
    let Some(multiplier) = multiplier else {
        return true;
    };
    let scaled = maximum_business_gross_cents(economics) * i128::from(multiplier);
    crate::finance::helpers::round_basis_point_product(scaled)
        .is_some_and(|proceeds| proceeds <= i128::from(i64::MAX))
}

fn business_economics_fit_production_arithmetic(economics: &BusinessEconomicsDefinition) -> bool {
    if !maximum_varied_gross_fits_i64(
        economics.base_gross.cents(),
        &[
            economics.wealth_revenue_per_point.cents(),
            economics.commerce_revenue_per_point.cents(),
        ],
        economics.gross_variance_basis_points,
    ) {
        return false;
    }
    let maximum_operating_cost = i128::from(economics.base_operating_cost.cents())
        + i128::from(economics.police_cost_per_point.cents()) * 100;
    maximum_operating_cost <= i128::from(i64::MAX)
}

fn enterprise_economics_fit_production_arithmetic(
    economics: &EnterpriseEconomicsDefinition,
) -> bool {
    if !maximum_varied_gross_fits_i64(
        economics.base_gross.cents(),
        &[
            economics.demand_revenue_per_point.cents(),
            economics.commerce_revenue_per_point.cents(),
            economics.wealth_revenue_per_point.cents(),
            economics.management_revenue_per_point.cents(),
        ],
        economics.gross_variance_basis_points,
    ) {
        return false;
    }
    // Both supporting businesses and active investigations are keyed by persistent u32 IDs, so
    // u32::MAX is a conservative upper bound for either unique count in one enterprise cycle.
    let maximum_count = i128::from(u32::MAX);
    let maximum_operating_cost = i128::from(economics.base_operating_cost.cents())
        + i128::from(economics.police_cost_per_point.cents()) * 100
        + i128::from(economics.support_surcharge_per_business.cents()) * maximum_count
        + i128::from(economics.heat_surcharge_per_active_case.cents()) * maximum_count;
    maximum_operating_cost <= i128::from(i64::MAX)
}

#[derive(Default)]
pub(crate) struct RegistryBuilder {
    capabilities: BTreeSet<CapabilityKind>,
    traits: BTreeSet<TraitKind>,
    drives: BTreeSet<DriveKind>,
    information_quality: Option<InformationQualityDefinition>,
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
    pub(crate) fn register_information_quality(
        &mut self,
        definition: InformationQualityDefinition,
    ) -> Result<(), RegistryBuildError> {
        if self.information_quality.is_some() {
            return Err(RegistryBuildError::DuplicateInformationQuality);
        }
        if definition.values().into_iter().any(|score| score > 100) {
            return Err(RegistryBuildError::InvalidInformationQuality);
        }
        self.information_quality = Some(definition);
        Ok(())
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
        if !(1..100).contains(&spec.off_window_patrol_presence_percent) {
            return Err(RegistryBuildError::InvalidLegalOffWindowPatrolPresencePercent);
        }
        if spec.cold_case_window.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidLegalColdWindow);
        }
        if spec.witness_interview_attempt_limit == 0 {
            return Err(RegistryBuildError::InvalidLegalInterviewLimit);
        }
        let [strength_corroborating, strength_strong, strength_direct] =
            spec.witness_testimony.strength_thresholds();
        let [
            reliability_mixed,
            reliability_credible,
            reliability_highly_reliable,
        ] = spec.witness_testimony.reliability_thresholds();
        if strength_corroborating == 0
            || strength_corroborating >= strength_strong
            || strength_strong >= strength_direct
            || strength_direct > 100
            || reliability_mixed == 0
            || reliability_mixed >= reliability_credible
            || reliability_credible >= reliability_highly_reliable
            || reliability_highly_reliable > 100
            || spec.witness_testimony.reluctant_band_discount > 3
            || spec.witness_testimony.hostile_band_discount > 3
            || spec.witness_testimony.reluctant_band_discount
                > spec.witness_testimony.hostile_band_discount
        {
            return Err(RegistryBuildError::InvalidLegalWitnessTestimony);
        }
        if spec.informant_decision_delay.as_minutes() == 0 {
            return Err(RegistryBuildError::InvalidLegalInformantDelay);
        }
        if spec.minimum_arrest_qualifying_evidence == 0 {
            return Err(RegistryBuildError::InvalidLegalArrestEvidenceCount);
        }
        if u16::from(spec.informant_base_flip_chance_percent)
            + u16::from(spec.informant_safety_bonus_percent)
            > 100
            || spec.represented_informant_reduction_percent > 100
        {
            return Err(RegistryBuildError::InvalidLegalInformantChance);
        }
        if spec.automatic_support_retainer.cents() <= 0 {
            return Err(RegistryBuildError::InvalidLegalAutomaticSupportRetainer);
        }
        if spec.maximum_detention.as_minutes() == 0
            || spec.maximum_detention <= spec.informant_decision_delay
        {
            return Err(RegistryBuildError::InvalidLegalMaximumDetention);
        }
        self.legal = Some(LegalConfigDefinition {
            off_window_patrol_presence_percent: spec.off_window_patrol_presence_percent,
            cold_case_window: spec.cold_case_window,
            witness_interview_attempt_limit: spec.witness_interview_attempt_limit,
            witness_testimony: spec.witness_testimony,
            informant_decision_delay: spec.informant_decision_delay,
            minimum_arrest_qualifying_evidence: spec.minimum_arrest_qualifying_evidence,
            informant_base_flip_chance_percent: spec.informant_base_flip_chance_percent,
            informant_safety_bonus_percent: spec.informant_safety_bonus_percent,
            represented_informant_reduction_percent: spec.represented_informant_reduction_percent,
            automatic_support_retainer: spec.automatic_support_retainer,
            maximum_detention: spec.maximum_detention,
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
        if i128::from(spec.per_member_daily.cents()) * i128::from(u32::MAX) > i128::from(i64::MAX) {
            return Err(RegistryBuildError::InvalidUpkeepArithmeticRange);
        }
        if !(1..=crate::social::RelationshipLevel::MAX_VALUE).contains(&spec.shortfall_resentment) {
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
        if spec.gross_basis_points == 0 || spec.gross_basis_points >= 10_000 {
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
        if spec.fee_basis_points == 0 || spec.fee_basis_points >= 10_000 {
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
        if spec.expansion_police_fear_ceiling <= spec.baseline {
            return Err(RegistryBuildError::InvalidReputationCeiling);
        }
        for delta in [
            spec.witnessed_exposure_police_fear,
            spec.identifying_exposure_police_fear,
            spec.racket_inquiry_police_fear,
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
        if spec.witnessed_exposure_police_fear <= 0
            || spec.identifying_exposure_police_fear < spec.witnessed_exposure_police_fear
            || spec.racket_inquiry_police_fear < spec.witnessed_exposure_police_fear
            || spec.achieved_underworld_competence <= 0
            || spec.partial_underworld_competence <= 0
            || spec.partial_underworld_competence > spec.achieved_underworld_competence
            || spec.violent_businesses_fear <= 0
            || !(1..=25).contains(&spec.intimidation_business_fear_max_adjustment)
        {
            return Err(RegistryBuildError::InvalidReputationConsequence);
        }
        if spec.daily_decay_step == 0 {
            // A zero decay step would make decay a structural no-op: impressions never
            // recover and faded records are never erased.
            return Err(RegistryBuildError::InvalidReputationDecayStep);
        }
        if spec.daily_decay_step > 25 {
            return Err(RegistryBuildError::InvalidReputationDelta);
        }
        self.reputation = Some(ReputationConfigDefinition {
            baseline: spec.baseline,
            daily_decay_step: spec.daily_decay_step,
            expansion_police_fear_ceiling: spec.expansion_police_fear_ceiling,
            witnessed_exposure_police_fear: spec.witnessed_exposure_police_fear,
            identifying_exposure_police_fear: spec.identifying_exposure_police_fear,
            racket_inquiry_police_fear: spec.racket_inquiry_police_fear,
            achieved_underworld_competence: spec.achieved_underworld_competence,
            partial_underworld_competence: spec.partial_underworld_competence,
            violent_businesses_fear: spec.violent_businesses_fear,
            intimidation_business_fear_max_adjustment: spec
                .intimidation_business_fear_max_adjustment,
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
        super::recruitment_validation::validate_recruitment_definition(&spec)?;
        self.recruitment = Some(RecruitmentDefinition {
            timing: spec.timing,
            scoring: spec.scoring,
            recruiter_capabilities: spec.recruiter_capabilities,
            relationships: spec.relationships,
            approach_drives: spec.approach_drives,
            trait_rules: spec.trait_rules,
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
        if spec.base_difficulty > 100 {
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
        if spec
            .source_support
            .values()
            .into_iter()
            .any(|score| score > 100)
        {
            return Err(RegistryBuildError::InvalidInvestigationWorkSupportScore(
                kind,
            ));
        }
        let (minimum_support, maximum_support) = match kind {
            InvestigationWorkKind::WitnessInterview => {
                let values = [
                    spec.source_support.witness_hostile,
                    spec.source_support.witness_reluctant,
                    spec.source_support.witness_cooperative,
                ];
                (
                    *values
                        .iter()
                        .min()
                        .expect("fixed witness support set is nonempty"),
                    *values
                        .iter()
                        .max()
                        .expect("fixed witness support set is nonempty"),
                )
            }
            InvestigationWorkKind::EvidenceReview => {
                let strength = [
                    spec.source_support.evidence_weak,
                    spec.source_support.evidence_corroborating,
                    spec.source_support.evidence_strong,
                    spec.source_support.evidence_direct,
                ];
                let reliability = [
                    spec.source_support.reliability_questionable,
                    spec.source_support.reliability_mixed,
                    spec.source_support.reliability_credible,
                    spec.source_support.reliability_highly_reliable,
                ];
                let admissibility = [
                    spec.source_support.admissibility_unknown,
                    spec.source_support.admissibility_inadmissible,
                    spec.source_support.admissibility_disputed,
                    spec.source_support.admissibility_admissible,
                ];
                let minimum = (u16::from(*strength.iter().min().expect("fixed strength set"))
                    + u16::from(*reliability.iter().min().expect("fixed reliability set"))
                    + u16::from(*admissibility.iter().min().expect("fixed admissibility set")))
                    / 3;
                let maximum = (u16::from(*strength.iter().max().expect("fixed strength set"))
                    + u16::from(*reliability.iter().max().expect("fixed reliability set"))
                    + u16::from(*admissibility.iter().max().expect("fixed admissibility set")))
                    / 3;
                (
                    u8::try_from(minimum).expect("averaged bounded support fits u8"),
                    u8::try_from(maximum).expect("averaged bounded support fits u8"),
                )
            }
        };
        let support_weight = i16::from(spec.source_support_weight);
        let minimum_margin = i16::from(minimum_support) * support_weight / 100
            - i16::from(spec.base_difficulty)
            - i16::from(spec.variance_limit);
        let maximum_margin = 100 + i16::from(maximum_support) * support_weight / 100
            - i16::from(spec.base_difficulty)
            + i16::from(spec.variance_limit);
        // Resolution uses margin >= connected_margin. The threshold must therefore sit strictly
        // above the lowest reachable margin and no higher than the maximum, otherwise one branch
        // is dead for this exact authored work definition.
        if spec.connected_margin <= minimum_margin || spec.connected_margin > maximum_margin {
            return Err(RegistryBuildError::InvalidInvestigationWorkConnectedMargin(
                kind,
            ));
        }
        // Exhaustive over both work kinds so a new kind forces an explicit interview-outcome
        // contract here instead of silently falling into a rejection arm.
        match (kind, spec.interview_outcome) {
            (InvestigationWorkKind::WitnessInterview, Some(interview))
                if spec.connected_margin < interview.medium_margin
                    && interview.medium_margin < interview.high_margin
                    && interview.high_margin <= maximum_margin
                    && interview.low_confidence < interview.medium_confidence
                    && interview.medium_confidence < interview.high_confidence
                    && interview.high_confidence <= 100 => {}
            (InvestigationWorkKind::WitnessInterview, Some(_))
            | (InvestigationWorkKind::WitnessInterview, None)
            | (InvestigationWorkKind::EvidenceReview, Some(_)) => {
                return Err(RegistryBuildError::InvalidInvestigationWorkInterviewOutcome(kind));
            }
            (InvestigationWorkKind::EvidenceReview, None) => {}
        }
        if self
            .investigation_work
            .insert(
                kind,
                InvestigationWorkDefinition {
                    duration: spec.duration,
                    base_difficulty: spec.base_difficulty,
                    source_support_weight: spec.source_support_weight,
                    variance_limit: spec.variance_limit,
                    connected_margin: spec.connected_margin,
                    source_support: spec.source_support,
                    interview_outcome: spec.interview_outcome,
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
        let definition = OperationDefinition {
            display_name,
            supported_approaches,
            required_roles,
            execution,
        };
        super::operation_validation::validate_operation_definition(kind, &definition)?;
        if self.businesses.values().any(|business| {
            !operation_proceeds_fit_business_gross(&definition, business.economics())
        }) {
            return Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(
                kind,
            ));
        }
        if self.operations.insert(kind, definition).is_some() {
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
        network_mode: EnterpriseNetworkMode,
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
        if !enterprise_economics_fit_production_arithmetic(&economics) {
            return Err(RegistryBuildError::EnterpriseEconomicArithmeticOutOfRange(
                kind,
            ));
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
        // The per-cycle visibility roll is a draw from 0..=9999 compared against these
        // basis points, so anything above 10_000 would mean "an inquiry every cycle".
        if economics.enforcement_attention_basis_points_per_active_case > 10_000 {
            return Err(RegistryBuildError::EnterpriseEnforcementAttentionOutOfRange(kind));
        }
        if network_mode == EnterpriseNetworkMode::SupportingBusinessesOnly
            && required_network_functions.is_empty()
        {
            return Err(RegistryBuildError::SeparateEnterpriseNetworkWithoutRequirements(kind));
        }
        if self
            .enterprises
            .insert(
                kind,
                EnterpriseDefinition {
                    economics,
                    required_business_functions,
                    required_network_functions,
                    network_mode,
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
        if !business_economics_fit_production_arithmetic(&economics) {
            return Err(RegistryBuildError::BusinessEconomicArithmeticOutOfRange(
                kind,
            ));
        }
        if let Some(operation) = self.operations.iter().find_map(|(operation, definition)| {
            (!operation_proceeds_fit_business_gross(definition, &economics)).then_some(*operation)
        }) {
            return Err(RegistryBuildError::OperationProceedsArithmeticOutOfRange(
                operation,
            ));
        }
        if economics.acquisition_cost.cents() <= 0 {
            return Err(RegistryBuildError::InvalidBusinessAcquisitionCost(kind));
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
        let information_quality = self
            .information_quality
            .ok_or(RegistryBuildError::MissingInformationQuality)?;
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
            information_quality,
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
