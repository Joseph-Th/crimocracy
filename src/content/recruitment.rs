//! Authored recruitment timing, scoring, relationships, drives, and trait rules.

use crate::core::time::SimDuration;
use crate::recruitment::RecruitmentApproach;
use crate::registry::{
    RecruitmentDefinitionSpec, RecruitmentIncumbentRelationshipDefinition,
    RecruitmentRelationshipDefinition, RecruitmentRelationshipSupportDefinition,
    RecruitmentScoringDefinition, RecruitmentTimingDefinition, RecruitmentTraitRuleDefinition,
    RecruitmentWeightsDefinition, RegistryBuilder,
};
use crate::world::{CapabilityKind, DriveKind, TraitKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn register_recruitment(builder: &mut RegistryBuilder) {
    builder
        .register_recruitment(RecruitmentDefinitionSpec {
            timing: RecruitmentTimingDefinition {
                cooldown: SimDuration::from_minutes(10_080),
                autonomous_attempt_cadence: SimDuration::from_minutes(1_440),
                perceived_legal_pressure_max_age: SimDuration::from_minutes(20_160),
            },
            scoring: RecruitmentScoringDefinition {
                base_willingness: 20,
                acceptance_score: 45,
                existing_membership_resistance: 15,
                charismatic_recruiter_bonus: 10,
                weights: RecruitmentWeightsDefinition {
                    recruiter_influence: 30,
                    drive_alignment: 25,
                    relationship_support: 25,
                    incumbent_resentment: 15,
                    perceived_legal_pressure: 15,
                    incumbent_attachment: 25,
                    organization_competence: 15,
                },
            },
            recruiter_capabilities: BTreeSet::from([
                CapabilityKind::Negotiation,
                CapabilityKind::SocialAccess,
            ]),
            relationships: RecruitmentRelationshipDefinition {
                recruiter_support: RecruitmentRelationshipSupportDefinition {
                    trust_weight: 2,
                    respect_weight: 1,
                    affection_weight: 1,
                    debt_weight: 1,
                    divisor: 5,
                    fear_penalty_weight: 1,
                    fear_penalty_divisor: 3,
                },
                incumbent_attachment: RecruitmentIncumbentRelationshipDefinition {
                    trust_weight: 1,
                    respect_weight: 1,
                    affection_weight: 1,
                    dependence_weight: 1,
                    divisor: 4,
                },
            },
            approach_drives: BTreeMap::from([
                (
                    RecruitmentApproach::FinancialOpportunity,
                    BTreeSet::from([DriveKind::Money]),
                ),
                (
                    RecruitmentApproach::Advancement,
                    BTreeSet::from([DriveKind::Status, DriveKind::Independence]),
                ),
                (
                    RecruitmentApproach::Protection,
                    BTreeSet::from([DriveKind::Safety, DriveKind::FamilySecurity]),
                ),
                (
                    RecruitmentApproach::PersonalAppeal,
                    BTreeSet::from([DriveKind::Respect]),
                ),
            ]),
            trait_rules: vec![
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Secretive,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: -8,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Cautious,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: -4,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Impulsive,
                    approach: None,
                    minimum_incumbent_resentment: None,
                    adjustment: 3,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Vindictive,
                    approach: None,
                    minimum_incumbent_resentment: Some(50),
                    adjustment: 8,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Greedy,
                    approach: Some(RecruitmentApproach::FinancialOpportunity),
                    minimum_incumbent_resentment: None,
                    adjustment: 12,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Ambitious,
                    approach: Some(RecruitmentApproach::FinancialOpportunity),
                    minimum_incumbent_resentment: None,
                    adjustment: 3,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Ambitious,
                    approach: Some(RecruitmentApproach::Advancement),
                    minimum_incumbent_resentment: None,
                    adjustment: 12,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::Advancement),
                    minimum_incumbent_resentment: None,
                    adjustment: 5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::EasilyFrightened,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 15,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Cautious,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 6,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::LoyalToFamily,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: 5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::Protection),
                    minimum_incumbent_resentment: None,
                    adjustment: -5,
                },
                RecruitmentTraitRuleDefinition {
                    trait_kind: TraitKind::Proud,
                    approach: Some(RecruitmentApproach::PersonalAppeal),
                    minimum_incumbent_resentment: None,
                    adjustment: 4,
                },
            ],
        })
        .unwrap_or_else(|error| panic!("invalid recruitment registry: {error}"));
}
