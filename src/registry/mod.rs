//! Immutable code-owned definitions and validated lookup tables loaded once at startup.

use crate::operations::{OperationApproach, OperationKind, RoleKind, ALL_OPERATION_KINDS};
use crate::world::{
    CapabilityKind, PolicyKind, PolicySetting, TraitKind, ALL_CAPABILITY_KINDS, ALL_POLICY_KINDS,
    ALL_TRAIT_KINDS,
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
}

#[derive(Clone, Debug)]
pub struct Registry {
    content_revision: u32,
    capabilities: BTreeMap<CapabilityKind, CapabilityDefinition>,
    traits: BTreeMap<TraitKind, TraitDefinition>,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
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
}

#[derive(Default)]
pub(crate) struct RegistryBuilder {
    capabilities: BTreeMap<CapabilityKind, CapabilityDefinition>,
    traits: BTreeMap<TraitKind, TraitDefinition>,
    policies: BTreeMap<PolicyKind, PolicyDefinition>,
    operations: BTreeMap<OperationKind, OperationDefinition>,
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
    ) -> Result<(), RegistryBuildError> {
        if self
            .operations
            .insert(
                kind,
                OperationDefinition {
                    kind,
                    display_name,
                    supported_approaches,
                    required_roles,
                },
            )
            .is_some()
        {
            return Err(RegistryBuildError::DuplicateOperation(kind));
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
        Ok(Registry {
            content_revision,
            capabilities: self.capabilities,
            traits: self.traits,
            policies: self.policies,
            operations: self.operations,
        })
    }
}
