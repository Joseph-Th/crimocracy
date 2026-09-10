//! Immutable code-owned registry: definition types, validated lookup tables, and assembly.
//!
//! Sibling files: `definitions.rs` owns the authored definition types; `builder.rs` owns
//! registration and completeness validation; `operation_validation.rs` owns operation-specific
//! authoring contracts; this module owns the `Registry` lookup surface.

mod builder;
mod definitions;
mod operation_validation;
mod recruitment_validation;

pub use definitions::*;

pub(crate) use builder::RegistryBuilder;

#[cfg(test)]
use crate::core::time::SimDuration;
#[cfg(test)]
use crate::registry::builder::RegistryBuildError;

use crate::enterprises::EnterpriseKind;
use crate::legal::InvestigationWorkKind;
use crate::operations::OperationKind;
use crate::world::{BusinessKind, PolicyKind, PolicySetting};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Registry {
    content_revision: u32,
    information_quality: InformationQualityDefinition,
    recruitment: RecruitmentDefinition,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
    investigation_work: BTreeMap<InvestigationWorkKind, InvestigationWorkDefinition>,
    enterprises: BTreeMap<EnterpriseKind, EnterpriseDefinition>,
    businesses: BTreeMap<BusinessKind, BusinessDefinition>,
    executive_brief: ExecutiveBriefDefinition,
    legal: LegalConfigDefinition,
    upkeep: UpkeepConfigDefinition,
    business_disruption: BusinessDisruptionDefinition,
    laundering: LaunderingConfigDefinition,
    reputation: ReputationConfigDefinition,
}

impl Registry {
    pub fn content_revision(&self) -> u32 {
        self.content_revision
    }
    pub fn information_quality(&self) -> InformationQualityDefinition {
        self.information_quality
    }
    pub fn recruitment(&self) -> &RecruitmentDefinition {
        &self.recruitment
    }
    pub fn legal(&self) -> LegalConfigDefinition {
        self.legal
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
    pub fn upkeep(&self) -> UpkeepConfigDefinition {
        self.upkeep
    }
    pub fn business_disruption(&self) -> BusinessDisruptionDefinition {
        self.business_disruption
    }
    pub fn laundering(&self) -> LaunderingConfigDefinition {
        self.laundering
    }
    pub fn reputation(&self) -> ReputationConfigDefinition {
        self.reputation
    }
    pub(crate) fn default_policies(&self) -> BTreeMap<PolicyKind, PolicySetting> {
        self.policies
            .iter()
            .map(|(kind, def)| (*kind, def.default()))
            .collect()
    }
}

#[cfg(test)]
mod tests;
