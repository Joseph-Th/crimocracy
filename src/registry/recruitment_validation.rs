//! Recruitment-specific registry validation. Keeps authored scoring, relationship math, trait
//! semantics, arithmetic safety, and outcome reachability out of the general registry builder.

use super::builder::RegistryBuildError;
use super::definitions::{
    RecruitmentDefinitionSpec, RecruitmentRelationshipSupportDefinition,
    RecruitmentTraitRuleDefinition,
};
use crate::recruitment::{ALL_RECRUITMENT_APPROACHES, RecruitmentApproach};
use crate::world::TraitKind;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_recruitment_definition(
    spec: &RecruitmentDefinitionSpec,
) -> Result<(), RegistryBuildError> {
    validate_timing(spec)?;
    validate_scoring_bounds(spec)?;
    validate_relationship_math(spec)?;
    validate_approach_drives(spec)?;
    validate_trait_rules(spec)?;
    validate_margin_space(spec)
}

fn validate_timing(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    if spec.timing.cooldown.as_minutes() == 0
        || spec.timing.autonomous_attempt_cadence.as_minutes() == 0
        || spec.timing.perceived_legal_pressure_max_age.as_minutes() == 0
    {
        return Err(RegistryBuildError::InvalidRecruitmentDuration);
    }
    Ok(())
}

fn validate_scoring_bounds(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    let scoring = spec.scoring;
    let weights = scoring.weights;
    if [
        weights.recruiter_influence,
        weights.drive_alignment,
        weights.relationship_support,
        weights.incumbent_resentment,
        weights.perceived_legal_pressure,
        weights.incumbent_attachment,
        weights.organization_competence,
        scoring.existing_membership_resistance,
        scoring.charismatic_recruiter_bonus,
    ]
    .into_iter()
    .any(|value| value > 100)
    {
        return Err(RegistryBuildError::InvalidRecruitmentWeight);
    }
    if !(0..=100).contains(&scoring.base_willingness)
        || !(0..=100).contains(&scoring.acceptance_score)
    {
        return Err(RegistryBuildError::InvalidRecruitmentScoring);
    }
    if spec.recruiter_capabilities.is_empty() {
        return Err(RegistryBuildError::MissingRecruitmentCapabilities);
    }
    Ok(())
}

fn validate_relationship_math(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    let support = spec.relationships.recruiter_support;
    let attachment = spec.relationships.incumbent_attachment;
    let support_weight_total = u16::from(support.trust_weight)
        + u16::from(support.respect_weight)
        + u16::from(support.affection_weight)
        + u16::from(support.debt_weight);
    let attachment_weight_total = u16::from(attachment.trust_weight)
        + u16::from(attachment.respect_weight)
        + u16::from(attachment.affection_weight)
        + u16::from(attachment.dependence_weight);
    if support.divisor == 0
        || support.fear_penalty_divisor == 0
        || attachment.divisor == 0
        || support_weight_total == 0
        || attachment_weight_total == 0
        || u32::from(support_weight_total) * 100 / u32::from(support.divisor) > 100
        || u32::from(attachment_weight_total) * 100 / u32::from(attachment.divisor) > 100
        || u32::from(support.fear_penalty_weight) * 100 / u32::from(support.fear_penalty_divisor)
            > 100
    {
        return Err(RegistryBuildError::InvalidRecruitmentRelationshipWeights);
    }
    Ok(())
}

fn validate_approach_drives(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    for approach in ALL_RECRUITMENT_APPROACHES {
        if spec
            .approach_drives
            .get(&approach)
            .is_none_or(BTreeSet::is_empty)
        {
            return Err(RegistryBuildError::MissingRecruitmentApproachDrives(
                approach,
            ));
        }
    }
    Ok(())
}

fn validate_trait_rules(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    let mut semantic_rules = BTreeSet::new();
    let mut maximum_absolute_trait_adjustment = 0_i32;
    for rule in &spec.trait_rules {
        if rule
            .minimum_incumbent_resentment
            .is_some_and(|minimum| !(1..=100).contains(&minimum))
            || !(-50..=50).contains(&rule.adjustment)
        {
            return Err(RegistryBuildError::InvalidRecruitmentTraitRule(
                rule.trait_kind,
            ));
        }
        if !semantic_rules.insert((
            rule.trait_kind,
            rule.approach,
            rule.minimum_incumbent_resentment,
        )) {
            return Err(RegistryBuildError::DuplicateRecruitmentTraitRule(
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
    Ok(())
}

fn validate_margin_space(spec: &RecruitmentDefinitionSpec) -> Result<(), RegistryBuildError> {
    for approach in ALL_RECRUITMENT_APPROACHES {
        let (minimum_margin, maximum_margin) = margin_bounds(spec, approach);
        if minimum_margin < i32::from(i16::MIN) || maximum_margin > i32::from(i16::MAX) {
            return Err(RegistryBuildError::InvalidRecruitmentArithmeticRange);
        }
        // Runtime accepts at margin >= 0 and refuses below zero. Require both branches to remain
        // theoretically reachable for every authored pitch type rather than shipping an approach
        // whose result is predetermined before candidate state is considered.
        if maximum_margin < 0 || minimum_margin >= 0 {
            return Err(RegistryBuildError::InvalidRecruitmentOutcomeRange(approach));
        }
    }
    Ok(())
}

fn margin_bounds(spec: &RecruitmentDefinitionSpec, approach: RecruitmentApproach) -> (i32, i32) {
    let scoring = spec.scoring;
    let weights = scoring.weights;
    let relationship_support_max = relationship_support_max(spec.relationships.recruiter_support);
    let attachment = spec.relationships.incumbent_attachment;
    let attachment_weight_total = u16::from(attachment.trust_weight)
        + u16::from(attachment.respect_weight)
        + u16::from(attachment.affection_weight)
        + u16::from(attachment.dependence_weight);
    let attachment_max = attachment_weight_total * 100 / u16::from(attachment.divisor);

    let legal_weight = if approach == RecruitmentApproach::Protection {
        i32::from(weights.perceived_legal_pressure)
    } else {
        0
    };
    let positive_max_without_incumbent = i32::from(scoring.base_willingness)
        - i32::from(scoring.acceptance_score)
        + i32::from(weights.recruiter_influence)
        + i32::from(weights.drive_alignment)
        + weighted_i32(relationship_support_max, weights.relationship_support)
        + legal_weight
        + i32::from(weights.organization_competence);
    let baseline = i32::from(scoring.base_willingness) - i32::from(scoring.acceptance_score);

    let (no_incumbent_trait_min, no_incumbent_trait_max) =
        trait_adjustment_bounds(&spec.trait_rules, approach, 0);
    let mut minimum = baseline + no_incumbent_trait_min;
    let mut maximum = positive_max_without_incumbent + no_incumbent_trait_max;

    for resentment in resentment_breakpoints(&spec.trait_rules) {
        let (trait_min, trait_max) =
            trait_adjustment_bounds(&spec.trait_rules, approach, resentment);
        let resentment_gain = weighted_i32(resentment, weights.incumbent_resentment);
        let membership_cost = i32::from(scoring.existing_membership_resistance);
        let incumbent_max =
            positive_max_without_incumbent + resentment_gain - membership_cost + trait_max;
        maximum = maximum.max(incumbent_max);

        let incumbent_min = baseline + resentment_gain
            - weighted_i32(
                u8::try_from(attachment_max).expect("validated attachment maximum fits u8"),
                weights.incumbent_attachment,
            )
            - membership_cost
            + trait_min;
        minimum = minimum.min(incumbent_min);
    }
    (minimum, maximum)
}

fn relationship_support_max(support: RecruitmentRelationshipSupportDefinition) -> u8 {
    let weight_total = u16::from(support.trust_weight)
        + u16::from(support.respect_weight)
        + u16::from(support.affection_weight)
        + u16::from(support.debt_weight);
    u8::try_from(weight_total * 100 / u16::from(support.divisor))
        .expect("validated recruitment support maximum fits u8")
}

fn resentment_breakpoints(rules: &[RecruitmentTraitRuleDefinition]) -> BTreeSet<u8> {
    let mut points = BTreeSet::from([0, 100]);
    for threshold in rules
        .iter()
        .filter_map(|rule| rule.minimum_incumbent_resentment)
    {
        points.insert(threshold);
        points.insert(threshold - 1);
    }
    points
}

fn trait_adjustment_bounds(
    rules: &[RecruitmentTraitRuleDefinition],
    approach: RecruitmentApproach,
    incumbent_resentment: u8,
) -> (i32, i32) {
    // A character either has or lacks each trait, so rules for the same trait are not independent.
    // Aggregate the rules that would fire for one trait first, then choose the set of traits that
    // minimizes or maximizes the total. This mirrors runtime stacking without inventing mutually
    // exclusive trait semantics that the world model does not define.
    let mut by_trait: BTreeMap<TraitKind, i32> = BTreeMap::new();
    for rule in rules.iter().filter(|rule| {
        rule.approach
            .is_none_or(|rule_approach| rule_approach == approach)
            && rule
                .minimum_incumbent_resentment
                .is_none_or(|minimum| incumbent_resentment >= minimum)
    }) {
        *by_trait.entry(rule.trait_kind).or_default() += i32::from(rule.adjustment);
    }
    by_trait
        .values()
        .fold((0, 0), |(minimum, maximum), adjustment| {
            (
                minimum + (*adjustment).min(0),
                maximum + (*adjustment).max(0),
            )
        })
}

fn weighted_i32(value: u8, weight: u8) -> i32 {
    i32::from(value) * i32::from(weight) / 100
}
