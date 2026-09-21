//! Deterministic operation after-action narrative rendering.

use super::OperationResolutionFactors;
use crate::intelligence::InformationTopic;
use crate::operations::{OperationExposureLevel, OperationObjectiveOutcome};
use crate::world::QualitativeBand;

/// Composes the after-action narrative from the resolution factors. The report leads with the
/// outcome and the factors that actually moved it. Neutral lines (normal execution window, no
/// exposure, negligible police presence) and strong-but-expected crew quality on a clean job are
/// omitted rather than recited, so attention goes to what deviates from a routine job: weak
/// capability bands, tactically degraded outcomes that deserve explanation, adverse pressure, and
/// thin planning intelligence. Practical blockers may make the effective objective fail even after
/// tactical success, so execution commentary follows the tactical outcome while the headline keeps
/// the effective objective result. Luck commentary is kept only when it explains tactical loss.
pub(super) fn build_after_action_summary(
    outcome: OperationObjectiveOutcome,
    tactical_outcome: OperationObjectiveOutcome,
    factors: OperationResolutionFactors,
    exposure: OperationExposureLevel,
    high_police_presence_threshold: u8,
    missing_intelligence_topics: &[InformationTopic],
) -> String {
    let mut parts = vec![format!("Objective {}.", outcome_label(outcome))];
    // Practical blockers can turn a tactically successful execution into an objective failure.
    // Crew-quality and luck commentary therefore keys off the tactical result rather than
    // falsely implying that strong execution caused a target-availability failure.
    if tactical_outcome != OperationObjectiveOutcome::Achieved
        || matches!(
            factors.role_capability_average().qualitative_band(),
            QualitativeBand::Poor | QualitativeBand::Competent
        )
    {
        parts.push(format!(
            "Assigned-role competence was {}.",
            band_label(factors.role_capability_average().qualitative_band())
        ));
    }
    match factors.leader_capability() {
        Some(rating)
            if tactical_outcome != OperationObjectiveOutcome::Achieved
                || matches!(
                    rating.qualitative_band(),
                    QualitativeBand::Poor | QualitativeBand::Competent
                ) =>
        {
            parts.push(format!(
                "Leadership coordination was {}.",
                band_label(rating.qualitative_band())
            ));
        }
        Some(_) => {}
        None => {
            parts.push("Leadership had no demonstrated capability for the execution.".to_owned())
        }
    }
    // Police pressure is reported when it materially shaped the job or when the organization
    // could not establish it at all; light presence was not worth the crew's attention.
    match (
        factors.target_police_presence(),
        factors.police_response_arrived(),
    ) {
        (presence, true) => {
            if presence.is_some_and(|rating| rating.value() >= high_police_presence_threshold) {
                parts.push(
                    "High local police presence materially increased execution pressure."
                        .to_owned(),
                );
            }
            parts.push(
                "Law-enforcement response reached the target before the operation ended."
                    .to_owned(),
            );
        }
        (Some(rating), false) if rating.value() >= high_police_presence_threshold => parts
            .push("High local police presence materially increased execution pressure.".to_owned()),
        (None, false) => parts.push(
            "No location-based police pressure could be established from the operation target."
                .to_owned(),
        ),
        (Some(_), false) => {}
    }
    if factors.intelligence_topics_relevant() > 0 {
        let covered = factors.intelligence_topics_covered();
        let relevant = factors.intelligence_topics_relevant();
        let coverage = if covered == relevant {
            format!("Planning intelligence covered all {relevant} relevant areas")
        } else {
            format!("Planning intelligence covered {covered} of {relevant} relevant areas")
        };
        // Thin coverage is actionable uncertainty the boss should see, not reassurance.
        let confidence = if covered * 2 >= relevant {
            "; the available reports reduced execution uncertainty."
        } else {
            "; large gaps remained in the plan's information."
        };
        parts.push(format!("Before departure: {coverage}{confidence}"));
        if !missing_intelligence_topics.is_empty() {
            let missing = missing_intelligence_topics
                .iter()
                .map(|topic| information_topic_label(*topic))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("Missing usable planning intelligence: {missing}."));
        }
    }
    // A chosen approach that reduced difficulty is the expected case, not news; only an
    // approach that hurt execution earns a sentence.
    if factors.approach_adjustment() > 0 {
        parts.push("The selected approach increased execution difficulty.".to_owned());
    }
    match factors.business_fear_adjustment().cmp(&0) {
        std::cmp::Ordering::Less => {
            parts.push("Business owners' fear of the organization reduced resistance.".to_owned())
        }
        std::cmp::Ordering::Greater => parts
            .push("Business owners were unusually willing to resist the organization.".to_owned()),
        std::cmp::Ordering::Equal => {}
    }
    if factors.time_pressure() > 0 {
        parts.push("The completion deadline compressed the execution window.".to_owned());
    }
    if tactical_outcome != OperationObjectiveOutcome::Achieved {
        match factors.variance() {
            value if value < 0 => parts.push(match tactical_outcome {
                OperationObjectiveOutcome::Partial => {
                    "Adverse unplanned circumstances reduced the result.".to_owned()
                }
                OperationObjectiveOutcome::Failed => {
                    "Adverse unplanned circumstances contributed to the failure.".to_owned()
                }
                OperationObjectiveOutcome::Achieved => unreachable!("excluded above"),
            }),
            0 => {}
            _ => parts.push("Favorable unplanned circumstances improved the result.".to_owned()),
        }
    }
    match exposure {
        OperationExposureLevel::None => {}
        OperationExposureLevel::Trace => {
            parts.push("The crew observed limited trace exposure.".to_owned())
        }
        OperationExposureLevel::Witnessed => parts.push(
            "The operation appears to have been witnessed or otherwise clearly observed."
                .to_owned(),
        ),
        OperationExposureLevel::Identifying => parts.push(
            "The crew believes at least one participant may have been identifiable.".to_owned(),
        ),
    }
    parts.join(" ")
}

pub(super) fn outcome_label(outcome: OperationObjectiveOutcome) -> &'static str {
    match outcome {
        OperationObjectiveOutcome::Achieved => "achieved",
        OperationObjectiveOutcome::Partial => "partially achieved",
        OperationObjectiveOutcome::Failed => "failed",
    }
}

fn information_topic_label(topic: InformationTopic) -> &'static str {
    match topic {
        InformationTopic::General => "general information",
        InformationTopic::TargetSecurity => "target security",
        InformationTopic::Personnel => "personnel",
        InformationTopic::Schedule => "schedule",
        InformationTopic::PoliceActivity => "police activity",
        InformationTopic::Route => "route",
        InformationTopic::FinancialPerformance => "financial performance",
        InformationTopic::Relationship => "relationships",
        InformationTopic::LegalActivity => "legal activity",
        InformationTopic::MarketAccess => "market access",
        InformationTopic::OperationalOutcome => "operational outcomes",
    }
}

fn band_label(band: QualitativeBand) -> &'static str {
    match band {
        QualitativeBand::Poor => "poor",
        QualitativeBand::Competent => "competent",
        QualitativeBand::Skilled => "skilled",
        QualitativeBand::Excellent => "excellent",
        QualitativeBand::Exceptional => "exceptional",
    }
}
