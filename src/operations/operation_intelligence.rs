//! Shared operation-planning intelligence valuation.
//!
//! Authorization, opportunity discovery, save validation, and eventual execution must agree on
//! whether an information record still has planning value. Keeping the age/quality calculation
//! here avoids making planning consumers depend on the execution subsystem and prevents drift
//! between pre-operation validation and resolution-time scoring.

use crate::core::time::SimTime;
use crate::intelligence::InformationRecord;
use crate::registry::InformationQualityDefinition;

pub(crate) fn resolve_information_score(
    quality: InformationQualityDefinition,
    information: &InformationRecord,
    planning_at: SimTime,
    max_age: u64,
) -> u8 {
    let reliability = u32::from(quality.reliability_score(information.reliability()));
    let specificity = u32::from(quality.specificity_score(information.specificity()));
    let age = planning_at
        .as_minutes()
        .saturating_sub(information.observed_at().as_minutes());
    let freshness = if age >= max_age {
        0_u32
    } else {
        u32::try_from((max_age - age).saturating_mul(100) / max_age)
            .expect("bounded intelligence freshness must fit u32")
    };
    let score = reliability
        .saturating_mul(specificity)
        .saturating_mul(freshness)
        / 10_000;
    u8::try_from(score).expect("bounded information score must fit u8")
}
