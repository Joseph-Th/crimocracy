//! Immutable code-owned definitions and validated lookup tables loaded once at startup.

use crate::core::time::SimDuration;
use crate::enterprises::{EnterpriseKind, ALL_ENTERPRISE_KINDS};
use crate::finance::Money;
use crate::intelligence::InformationTopic;
use crate::legal::EvidenceKind;
use crate::operations::{OperationApproach, OperationKind, RoleKind, ALL_OPERATION_KINDS};
use crate::world::{
    BusinessFunction, BusinessKind, CapabilityKind, PolicyKind, PolicySetting, TraitKind,
    ALL_BUSINESS_KINDS, ALL_CAPABILITY_KINDS, ALL_POLICY_KINDS, ALL_TRAIT_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct CapabilityDefinition {
    kind: CapabilityKind,
    display_name: &'static str,
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
    pub(crate) intelligence: OperationIntelligenceDefinition,
    pub(crate) exposure: OperationExposureDefinition,
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

impl OperationExecutionDefinition {
    pub fn duration(&self) -> SimDuration {
        self.difficulty.duration
    }
    pub fn base_difficulty(&self) -> u8 {
        self.difficulty.base_difficulty
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

#[derive(Clone, Debug)]
pub struct Registry {
    content_revision: u32,
    capabilities: BTreeMap<CapabilityKind, CapabilityDefinition>,
    traits: BTreeMap<TraitKind, TraitDefinition>,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
}

impl Registry {
    pub fn content_revision(&self) -> u32 {
        self.content_revision
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
    #[error("missing policy definition: {0:?}")]
    MissingPolicy(PolicyKind),
    #[error("missing operation definition: {0:?}")]
    MissingOperation(OperationKind),
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
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
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
        Ok(Registry {
            content_revision,
            capabilities: self.capabilities,
            traits: self.traits,
            policies: self.policies,
            operations: self.operations,
            enterprises: self.enterprises,
            businesses: self.businesses,
        })
    }
}
