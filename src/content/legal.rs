//! Authored legal-system thresholds and investigation-work definitions.

use crate::core::time::SimDuration;
use crate::finance::Money;
use crate::legal::InvestigationWorkKind;
use crate::registry::{
    InvestigationInterviewOutcomeDefinition, InvestigationSourceSupportDefinition,
    InvestigationWorkDefinitionSpec, LegalConfigSpec, RegistryBuilder, WitnessTestimonyDefinition,
};

pub(super) fn register_legal(builder: &mut RegistryBuilder) {
    builder
        .register_legal(LegalConfigSpec {
            // Seven campaign days (one week) of institutional inactivity before an
            // origin-linked street case is deterministically shelved.
            cold_case_window: SimDuration::from_minutes(10_080),
            // Three statementless interviews and investigators stop retrying a witness:
            // enough for a reluctant witness to open up, few enough that a hostile one
            // cannot stall a case forever.
            witness_interview_attempt_limit: 3,
            witness_testimony: WitnessTestimonyDefinition {
                strength_corroborating_min_confidence: 35,
                strength_strong_min_confidence: 60,
                strength_direct_min_confidence: 85,
                reliability_mixed_min_confidence: 25,
                reliability_credible_min_confidence: 50,
                reliability_highly_reliable_min_confidence: 80,
                reluctant_band_discount: 1,
                hostile_band_discount: 2,
            },
            // One custody day before a detainee faces their informant-recruitment decision.
            informant_decision_delay: SimDuration::from_minutes(1_440),
            // Custody requires corroboration from two independent qualifying sources, including
            // at least one Strong or Direct source. Direct and autonomous arrest use the same bar.
            minimum_arrest_qualifying_evidence: 2,
            // A detainee starts at 25 percent cooperation risk. A maximum Safety drive adds
            // 50 points; active counsel removes 25 points and can fully suppress the modeled
            // custodial-pressure risk for a detainee with no Safety-driven pressure.
            informant_base_flip_chance_percent: 25,
            informant_safety_bonus_percent: 50,
            represented_informant_reduction_percent: 25,
            automatic_support_retainer: Money::from_cents(5_000),
            // Two custody days is long enough for the one-day informant decision and legal
            // support response to matter, but custody cannot become de facto permanent while
            // charging, bail, and trial remain outside the modeled foundation.
            maximum_detention: SimDuration::from_minutes(2_880),
        })
        .unwrap_or_else(|error| panic!("invalid legal registry: {error}"));
}

pub(super) fn register_investigation_work(builder: &mut RegistryBuilder) {
    let source_support = InvestigationSourceSupportDefinition {
        witness_hostile: 20,
        witness_reluctant: 50,
        witness_cooperative: 85,
        evidence_weak: 20,
        evidence_corroborating: 45,
        evidence_strong: 70,
        evidence_direct: 95,
        reliability_questionable: 15,
        reliability_mixed: 40,
        reliability_credible: 70,
        reliability_highly_reliable: 95,
        admissibility_unknown: 35,
        admissibility_inadmissible: 0,
        admissibility_disputed: 50,
        admissibility_admissible: 90,
    };
    builder
        .register_investigation_work(
            InvestigationWorkKind::EvidenceReview,
            InvestigationWorkDefinitionSpec {
                duration: SimDuration::from_minutes(180),
                base_difficulty: 45,
                source_support_weight: 35,
                variance_limit: 12,
                connected_margin: 0,
                source_support,
                interview_outcome: None,
            },
        )
        .unwrap_or_else(|error| panic!("invalid investigation work registry: {error}"));
    // Interview support is the witness's cooperation; a hostile witness can deny the
    // detective a usable statement entirely.
    builder
        .register_investigation_work(
            InvestigationWorkKind::WitnessInterview,
            InvestigationWorkDefinitionSpec {
                duration: SimDuration::from_minutes(120),
                base_difficulty: 30,
                source_support_weight: 45,
                variance_limit: 10,
                connected_margin: 0,
                source_support,
                interview_outcome: Some(InvestigationInterviewOutcomeDefinition {
                    medium_margin: 10,
                    high_margin: 20,
                    low_confidence: 40,
                    medium_confidence: 65,
                    high_confidence: 85,
                }),
            },
        )
        .unwrap_or_else(|error| panic!("invalid investigation work registry: {error}"));
}
