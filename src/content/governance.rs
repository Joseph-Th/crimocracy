//! Authored standing policies, reputation tuning, executive-brief cadence, and organization upkeep.

use crate::core::time::SimDuration;
use crate::finance::Money;
use crate::registry::{
    ExecutiveBriefDefinitionSpec, RegistryBuilder, ReputationConfigSpec, UpkeepConfigSpec,
};
use crate::world::{ApprovalPolicy, LegalSupportPolicy, PolicyKind, PolicySetting};

pub(super) fn register_reputation(builder: &mut RegistryBuilder) {
    builder
        .register_reputation(ReputationConfigSpec {
            baseline: 40,
            // One point per day: a witnessed job stays in an audience's memory for weeks,
            // not forever, and never manufactures impressions that were never touched.
            daily_decay_step: 1,
            // A witnessed exposure plus a vice inquiry can visibly throttle delegated
            // expansion, making police posture a strategic constraint rather than decoration.
            expansion_police_fear_ceiling: 50,
            witnessed_exposure_police_fear: 8,
            identifying_exposure_police_fear: 10,
            vice_inquiry_police_fear: 6,
            achieved_underworld_competence: 3,
            partial_underworld_competence: 1,
            violent_businesses_fear: 3,
        })
        .unwrap_or_else(|error| panic!("invalid reputation registry: {error}"));
}

pub(super) fn register_executive_brief(builder: &mut RegistryBuilder) {
    builder
        .register_executive_brief(ExecutiveBriefDefinitionSpec {
            cadence: SimDuration::from_minutes(1_440),
            minimum_source_attention: crate::core::attention::AttentionClass::Notable,
            max_source_entries: 8,
        })
        .unwrap_or_else(|error| panic!("invalid executive brief registry: {error}"));
}

pub(super) fn register_upkeep(builder: &mut RegistryBuilder) {
    builder
        .register_upkeep(UpkeepConfigSpec {
            // Daily street wage per member is visible next to one enterprise cycle, so an
            // idle organization feels carrying costs and headcount remains a real decision.
            // District heat can tighten the resulting surplus without making a small crew
            // immediately insolvent.
            per_member_daily: Money::from_cents(32_00),
            shortfall_resentment: 12,
        })
        .unwrap_or_else(|error| panic!("invalid upkeep registry: {error}"));
}

pub(super) fn register_policies(builder: &mut RegistryBuilder) {
    let definitions = [
        (
            PolicyKind::IndependentRecruitment,
            PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
        ),
        (
            PolicyKind::AssociateLegalSupport,
            PolicySetting::AssociateLegalSupport(LegalSupportPolicy::CaseByCase),
        ),
    ];
    for (kind, default) in definitions {
        builder
            .register_policy(kind, default)
            .unwrap_or_else(|error| panic!("invalid policy registry: {error}"));
    }
}
