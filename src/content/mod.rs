//! Code-owned authored definitions assembled into the immutable startup registry.

use crate::operations::{OperationKind, RoleKind, ALL_OPERATION_APPROACHES, ALL_OPERATION_KINDS};
use crate::registry::{Registry, RegistryBuilder};
use crate::world::{
    ApprovalPolicy, CapabilityKind, CasualtyPolicy, ForcePolicy, LegalSupportPolicy, PolicyKind,
    PolicySetting, TraitKind, ALL_CAPABILITY_KINDS, ALL_TRAIT_KINDS,
};
use std::collections::BTreeSet;

pub const CURRENT_CONTENT_REVISION: u32 = 1;

pub fn build_registry() -> Registry {
    let mut builder = RegistryBuilder::new();
    for kind in ALL_CAPABILITY_KINDS {
        builder
            .register_capability(kind, capability_name(kind))
            .unwrap_or_else(|error| panic!("invalid capability registry: {error}"));
    }
    for kind in ALL_TRAIT_KINDS {
        builder
            .register_trait(kind, trait_name(kind))
            .unwrap_or_else(|error| panic!("invalid trait registry: {error}"));
    }
    register_policies(&mut builder);
    for kind in ALL_OPERATION_KINDS {
        let approaches: BTreeSet<_> = ALL_OPERATION_APPROACHES.into_iter().collect();
        builder
            .register_operation(kind, operation_name(kind), approaches, required_roles(kind))
            .unwrap_or_else(|error| panic!("invalid operation registry: {error}"));
    }
    builder
        .build(CURRENT_CONTENT_REVISION)
        .unwrap_or_else(|error| panic!("invalid content registry: {error}"))
}

fn register_policies(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            PolicyKind::CollectionForce,
            "Collection force",
            PolicySetting::CollectionForce(ForcePolicy::ThreatsOnly),
        ),
        (
            PolicyKind::PatrolBribery,
            "Patrol bribery",
            PolicySetting::PatrolBribery(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::IndependentRecruitment,
            "Independent recruitment",
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::CasualtyResponse,
            "Casualty response",
            PolicySetting::CasualtyResponse(CasualtyPolicy::RequestDecision),
        ),
        (
            PolicyKind::AssociateLegalSupport,
            "Associate legal support",
            PolicySetting::AssociateLegalSupport(LegalSupportPolicy::CaseByCase),
        ),
    ];
    for (kind, name, default) in definitions {
        builder
            .register_policy(kind, name, default)
            .unwrap_or_else(|error| panic!("invalid policy registry: {error}"));
    }
}

fn capability_name(kind: CapabilityKind) -> &'static str {
    match kind {
        CapabilityKind::Violence => "Violence",
        CapabilityKind::Intimidation => "Intimidation",
        CapabilityKind::Stealth => "Stealth",
        CapabilityKind::Burglary => "Burglary",
        CapabilityKind::Driving => "Driving",
        CapabilityKind::Surveillance => "Surveillance",
        CapabilityKind::Investigation => "Investigation",
        CapabilityKind::Accounting => "Accounting",
        CapabilityKind::Negotiation => "Negotiation",
        CapabilityKind::Management => "Management",
        CapabilityKind::PoliticalInfluence => "Political influence",
        CapabilityKind::LegalKnowledge => "Legal knowledge",
        CapabilityKind::SocialAccess => "Social access",
    }
}
fn trait_name(kind: TraitKind) -> &'static str {
    match kind {
        TraitKind::Cautious => "Cautious",
        TraitKind::Impulsive => "Impulsive",
        TraitKind::Greedy => "Greedy",
        TraitKind::Proud => "Proud",
        TraitKind::Patient => "Patient",
        TraitKind::Cruel => "Cruel",
        TraitKind::Charismatic => "Charismatic",
        TraitKind::Vindictive => "Vindictive",
        TraitKind::Secretive => "Secretive",
        TraitKind::Ambitious => "Ambitious",
        TraitKind::LoyalToFamily => "Loyal to family",
        TraitKind::EasilyFrightened => "Easily frightened",
    }
}
fn operation_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Burglary => "Burglary",
        OperationKind::Robbery => "Robbery",
        OperationKind::Hijacking => "Hijacking",
        OperationKind::Smuggling => "Smuggling",
        OperationKind::Intimidation => "Intimidation",
        OperationKind::Kidnapping => "Kidnapping",
        OperationKind::Surveillance => "Surveillance",
        OperationKind::Sabotage => "Sabotage",
        OperationKind::Bribery => "Bribery",
        OperationKind::WitnessPressure => "Witness pressure",
        OperationKind::DocumentTheft => "Document theft",
        OperationKind::GamblingEvent => "Gambling event",
        OperationKind::CovertTransfer => "Covert transfer",
        OperationKind::Extraction => "Extraction",
        OperationKind::RivalInfiltration => "Rival infiltration",
    }
}
fn required_roles(kind: OperationKind) -> BTreeSet<RoleKind> {
    let roles: &[RoleKind] = match kind {
        OperationKind::Burglary => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::Robbery => &[RoleKind::Coordinator, RoleKind::Muscle],
        OperationKind::Hijacking => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Smuggling => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Intimidation => &[RoleKind::Coordinator],
        OperationKind::Kidnapping => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::Surveillance => &[RoleKind::Surveillance],
        OperationKind::Sabotage => &[RoleKind::Coordinator],
        OperationKind::Bribery => &[RoleKind::Negotiator],
        OperationKind::WitnessPressure => &[RoleKind::Coordinator],
        OperationKind::DocumentTheft => &[RoleKind::Coordinator, RoleKind::EntrySpecialist],
        OperationKind::GamblingEvent => &[RoleKind::Coordinator],
        OperationKind::CovertTransfer => &[RoleKind::Coordinator],
        OperationKind::Extraction => &[RoleKind::Coordinator, RoleKind::Driver],
        OperationKind::RivalInfiltration => &[RoleKind::Coordinator],
    };
    roles.iter().copied().collect()
}
