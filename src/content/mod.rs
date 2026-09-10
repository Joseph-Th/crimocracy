//! Code-owned authored definitions assembled into the immutable startup registry.

mod businesses;
mod enterprises;
mod governance;
mod legal;
mod operations;
mod recruitment;

use crate::registry::{InformationQualityDefinition, Registry, RegistryBuilder};
use crate::world::{ALL_CAPABILITY_KINDS, ALL_DRIVE_KINDS, ALL_TRAIT_KINDS};

pub const CURRENT_CONTENT_REVISION: u32 = 43;

const INFORMATION_QUALITY: InformationQualityDefinition = InformationQualityDefinition {
    unknown_reliability: 20,
    unreliable_reliability: 10,
    mixed_reliability: 40,
    generally_reliable: 70,
    direct_access: 100,
    vague_specificity: 25,
    general_specificity: 50,
    specific_specificity: 75,
    precise_specificity: 100,
};

pub fn build_registry() -> Registry {
    let mut builder = RegistryBuilder::default();
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
    builder
        .register_information_quality(INFORMATION_QUALITY)
        .unwrap_or_else(|error| panic!("invalid information-quality registry: {error}"));

    recruitment::register_recruitment(&mut builder);
    legal::register_legal(&mut builder);
    governance::register_policies(&mut builder);
    operations::register_operations(&mut builder);
    legal::register_investigation_work(&mut builder);
    enterprises::register_enterprises(&mut builder);
    businesses::register_businesses(&mut builder);
    businesses::register_business_disruption(&mut builder);
    businesses::register_laundering(&mut builder);
    governance::register_reputation(&mut builder);
    governance::register_executive_brief(&mut builder);
    governance::register_upkeep(&mut builder);

    builder
        .build(CURRENT_CONTENT_REVISION)
        .unwrap_or_else(|error| panic!("invalid content registry: {error}"))
}
