//! Deterministic recruitment scoring: relationship and drive factors, trait adjustments,
//! the margin/outcome rule, and perceived legal-pressure scoring. Pure derivations over
//! snapshots - no randomness and no state mutation, so decide, commit, and the invariant
//! re-derivation all share one arithmetic core.

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, InformationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::{InformationRecord, InformationTopic, KnowledgeHolder};
use crate::recruitment::recruitment_system::RecruitmentFactorContext;
use crate::recruitment::{
    RecruitmentApproach, RecruitmentFactorComponents, RecruitmentFactors, RecruitmentOutcome,
    build_recruitment_factors,
};
use crate::registry::RecruitmentDefinition;
use crate::social::RelationshipDimensions;
use crate::world::{DriveKind, TraitKind};
use std::collections::BTreeSet;

pub(crate) fn resolve_recruitment_factors_from_context(
    context: RecruitmentFactorContext<'_>,
) -> Option<RecruitmentFactors> {
    let RecruitmentFactorContext {
        definition,
        candidate,
        recruiter,
        approach,
        recruiter_relationship,
        incumbent_relationship,
        perceived_legal_pressure,
        organization_competence,
        had_previous_organization,
    } = context;
    let relationship = recruiter_relationship.dimensions()?;

    let base_influence = definition
        .recruiter_capabilities()
        .iter()
        .filter_map(|kind| recruiter.capability(*kind))
        .map(|rating| rating.value())
        .max()
        .unwrap_or(0);
    let recruiter_influence = base_influence
        .saturating_add(
            u8::from(recruiter.has_trait(TraitKind::Charismatic))
                .saturating_mul(definition.charismatic_recruiter_bonus()),
        )
        .min(100);

    let drive_alignment = definition
        .drives_for_approach(approach)
        .iter()
        .map(|kind| drive_value(candidate, *kind))
        .max()
        .unwrap_or(0);

    let relationship_support = recruitment_relationship_support(definition, relationship);
    let (incumbent_attachment, incumbent_resentment) = incumbent_relationship
        .and_then(|snapshot| snapshot.dimensions())
        .map(|dimensions| recruitment_incumbent_factors(definition, dimensions))
        .unwrap_or((0, 0));

    let membership_resistance = if had_previous_organization {
        definition.existing_membership_resistance()
    } else {
        0
    };
    let trait_adjustment =
        recruitment_trait_adjustment(definition, candidate, approach, incumbent_resentment);

    Some(build_recruitment_factors(RecruitmentFactorComponents {
        recruiter_influence,
        drive_alignment,
        relationship_support,
        incumbent_attachment,
        incumbent_resentment,
        perceived_legal_pressure,
        membership_resistance,
        organization_competence,
        trait_adjustment,
    }))
}

pub(crate) fn recruitment_relationship_support(
    definition: &RecruitmentDefinition,
    dimensions: RelationshipDimensions,
) -> u8 {
    let weights = definition.relationships().recruiter_support;
    let positive_relationship = u16::from(dimensions.trust.value())
        .saturating_mul(u16::from(weights.trust_weight))
        + u16::from(dimensions.respect.value()).saturating_mul(u16::from(weights.respect_weight))
        + u16::from(dimensions.affection.value())
            .saturating_mul(u16::from(weights.affection_weight))
        + u16::from(dimensions.debt.value()).saturating_mul(u16::from(weights.debt_weight));
    let positive = u8::try_from(positive_relationship / u16::from(weights.divisor))
        .expect("bounded relationship support must fit u8")
        .min(100);
    let fear_penalty = u8::try_from(
        u16::from(dimensions.fear.value()).saturating_mul(u16::from(weights.fear_penalty_weight))
            / u16::from(weights.fear_penalty_divisor),
    )
    .expect("bounded relationship fear penalty must fit u8");
    positive.saturating_sub(fear_penalty)
}

pub(crate) fn recruitment_incumbent_factors(
    definition: &RecruitmentDefinition,
    dimensions: RelationshipDimensions,
) -> (u8, u8) {
    let weights = definition.relationships().incumbent_attachment;
    let attachment = (u16::from(dimensions.trust.value())
        .saturating_mul(u16::from(weights.trust_weight))
        + u16::from(dimensions.respect.value()).saturating_mul(u16::from(weights.respect_weight))
        + u16::from(dimensions.affection.value())
            .saturating_mul(u16::from(weights.affection_weight))
        + u16::from(dimensions.dependence.value())
            .saturating_mul(u16::from(weights.dependence_weight)))
        / u16::from(weights.divisor);
    (
        u8::try_from(attachment).expect("bounded incumbent attachment must fit u8"),
        dimensions.resentment.value(),
    )
}

fn drive_value(character: &crate::world::CharacterRecord, kind: DriveKind) -> u8 {
    character.drive(kind).map_or(0, |rating| rating.value())
}

fn recruitment_trait_adjustment(
    definition: &RecruitmentDefinition,
    candidate: &crate::world::CharacterRecord,
    approach: RecruitmentApproach,
    incumbent_resentment: u8,
) -> i16 {
    definition
        .trait_rules()
        .iter()
        .filter(|rule| candidate.has_trait(rule.trait_kind))
        .filter(|rule| {
            rule.approach
                .is_none_or(|rule_approach| rule_approach == approach)
        })
        .filter(|rule| {
            rule.minimum_incumbent_resentment
                .is_none_or(|minimum| incumbent_resentment >= minimum)
        })
        .try_fold(0_i16, |total, rule| total.checked_add(rule.adjustment))
        .expect("validated authored recruitment trait adjustments must fit i16")
}

pub(crate) fn resolve_recruitment_margin(
    definition: &RecruitmentDefinition,
    factors: RecruitmentFactors,
    approach: RecruitmentApproach,
) -> i16 {
    let weights = definition.weights();
    // Legal pressure is approach-sensitive: only Protection offers genuinely
    // leverage fear of prosecution. Financial/Advancement pitches do not benefit
    // from a target being "wanted", which would otherwise reward poaching the
    // most-investigated characters with money alone.
    let legal_weight = if approach == RecruitmentApproach::Protection {
        i16::from(weights.perceived_legal_pressure)
    } else {
        0
    };
    let score = definition.base_willingness()
        + weighted(
            factors.recruiter_influence(),
            i16::from(weights.recruiter_influence),
        )
        + weighted(
            factors.drive_alignment(),
            i16::from(weights.drive_alignment),
        )
        + weighted(
            factors.relationship_support(),
            i16::from(weights.relationship_support),
        )
        + weighted(
            factors.incumbent_resentment(),
            i16::from(weights.incumbent_resentment),
        )
        + weighted(factors.perceived_legal_pressure(), legal_weight)
        + weighted(
            factors.organization_competence(),
            i16::from(weights.organization_competence),
        )
        - weighted(
            factors.incumbent_attachment(),
            i16::from(weights.incumbent_attachment),
        )
        - i16::from(factors.membership_resistance())
        + factors.trait_adjustment();
    score - definition.acceptance_score()
}

fn weighted(value: u8, weight: i16) -> i16 {
    i16::from(value) * weight / 100
}

pub(crate) fn resolve_recruitment_outcome(margin: i16) -> RecruitmentOutcome {
    if margin >= 0 {
        RecruitmentOutcome::Accepted
    } else {
        RecruitmentOutcome::Refused
    }
}

pub(crate) fn resolve_perceived_legal_pressure_at(
    definition: &RecruitmentDefinition,
    state: &AppState,
    candidate: CharacterId,
    at: SimTime,
) -> (Option<InformationId>, u8) {
    let ids = candidate_pressure_information_ids(state, candidate, at);
    resolve_perceived_legal_pressure_from_ids(definition, state, &ids, at)
}

/// Selection runs over exactly the ID set the staleness token captures, so a fresh plan can
/// never spuriously fail with `StalePressureKnowledge`. Decide passes the set it already
/// collected for the token; validation recomputes it and must reach the same selection.
pub(crate) fn resolve_perceived_legal_pressure_from_ids(
    definition: &RecruitmentDefinition,
    state: &AppState,
    pressure_information_ids: &BTreeSet<InformationId>,
    at: SimTime,
) -> (Option<InformationId>, u8) {
    pressure_information_ids
        .iter()
        .filter_map(|id| state.intelligence.get_information(*id))
        .map(|information| {
            (
                information.id(),
                perceived_legal_pressure_score(definition, information, at),
                information.observed_at(),
            )
        })
        .filter(|(_, score, _)| *score > 0)
        .max_by_key(|(id, score, observed_at)| (*score, *observed_at, *id))
        .map_or((None, 0), |(id, score, _)| (Some(id), score))
}
fn perceived_legal_pressure_score(
    definition: &RecruitmentDefinition,
    information: &InformationRecord,
    at: SimTime,
) -> u8 {
    let quality = definition.information_quality();
    let reliability = u16::from(quality.reliability_score(information.reliability()));
    let specificity = u16::from(quality.specificity_score(information.specificity()));
    let base = (reliability + specificity) / 2;
    let age = at
        .as_minutes()
        .saturating_sub(information.observed_at().as_minutes());
    let max_age = u64::from(definition.perceived_legal_pressure_max_age().as_minutes());
    let remaining = max_age.saturating_sub(age);
    u8::try_from(u64::from(base) * remaining / max_age)
        .expect("bounded perceived legal pressure must fit u8")
}

/// The single staleness predicate for candidate pressure knowledge: both the plan's snapshot
/// and the commit-time revalidation must agree through this one derivation.
pub(crate) fn candidate_pressure_information_ids(
    state: &AppState,
    candidate: CharacterId,
    at: SimTime,
) -> BTreeSet<InformationId> {
    state
        .intelligence
        .information_for_holder_by_topic(
            KnowledgeHolder::Character(candidate),
            InformationTopic::PoliceActivity,
        )
        .filter(|information| {
            information.subject() == EntityRef::Character(candidate)
                && information.recorded_at() <= at
                && information.observed_at() <= at
        })
        .map(InformationRecord::id)
        .collect()
}
