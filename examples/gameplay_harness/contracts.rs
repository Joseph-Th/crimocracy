//! Harness contract validators over persisted run metrics and registry-aware state.

use crimocracy::core::invariants::{validate_state, validate_state_against_registry};
use crimocracy::core::state::AppState;
use crimocracy::intelligence::InformationTopic;
use crimocracy::operations::{OperationAbortCause, OperationAbortPhase, OperationObjectiveOutcome};
use crimocracy::registry::Registry;
use std::error::Error;

use crate::*;

pub fn validate_run_metrics(
    metrics: &RunMetrics,
    require_financials: bool,
) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    if metrics.burglary.is_none() {
        return Err(HarnessContractError::MissingBurglary { strategy });
    }
    if metrics.burglary_terminal_minute.is_none() {
        return Err(HarnessContractError::MissingTerminalState { strategy });
    }
    if metrics.aborted == metrics.outcome.is_some()
        || metrics.aborted != (metrics.abort_phase.is_some() && metrics.abort_cause.is_some())
        || (!metrics.aborted && (metrics.abort_phase.is_some() || metrics.abort_cause.is_some()))
    {
        return Err(HarnessContractError::InconsistentTerminalState {
            strategy,
            aborted: metrics.aborted,
            outcome: metrics.outcome,
            abort_phase: metrics.abort_phase,
            abort_cause: metrics.abort_cause,
        });
    }
    if require_financials
        && (metrics.legitimate_net_cents.is_none() || metrics.enterprise_net_cents.is_none())
    {
        return Err(HarnessContractError::MissingFinancialObservation { strategy });
    }
    Ok(())
}

pub fn validate_night_trap_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    let evidence = match strategy {
        Strategy::Rush => {
            if matches!(
                metrics.abort_cause,
                Some(OperationAbortCause::PoliceArrival(_))
            ) && metrics.abort_phase == Some(OperationAbortPhase::InProgress)
                && metrics.player_police_activity_information > 0
            {
                None
            } else {
                Some("pre-entry police arrival triggers the standing abort contingency and the crew's direct observations are debriefed into organization knowledge")
            }
        }
        Strategy::Press => {
            if metrics.decision_requests > 0
                && metrics.investigation_created
                && metrics.player_legal_activity_information > 0
                && metrics.player_police_activity_information > 0
                && metrics.counterintelligence_outcome.is_some()
                && metrics.counterintelligence_information > 0
                && metrics.followup_case_active == Some(true)
            {
                None
            } else {
                Some("police-arrival decision, surfaced field/legal information, and a counter-surveillance follow-up that reads whether the case is still active")
            }
        }
        Strategy::Recon => {
            if metrics.discovered_surveillance_information >= 2
                && metrics.planning_information_count >= 3
                && metrics
                    .planning_information_topics
                    .contains(&InformationTopic::MarketAccess)
                && metrics.outcome.is_some()
            {
                None
            } else {
                Some("surveillance information must carry both patrol and venue-access facts into the burglary plan")
            }
        }
    };
    evidence.map_or(Ok(()), |evidence| {
        Err(HarnessContractError::MissingStrategyEvidence { strategy, evidence })
    })
}

pub fn validate_strategy_evidence(
    profile: ScenarioProfile,
    metrics: &RunMetrics,
) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    match strategy {
        Strategy::Rush if profile != ScenarioProfile::LatePatrol => {
            validate_night_trap_evidence(metrics)
        }
        Strategy::Press if metrics.police_arrived => validate_night_trap_evidence(metrics),
        Strategy::Recon => validate_night_trap_evidence(metrics),
        Strategy::Rush | Strategy::Press => Ok(()),
    }
}

/// Full-mode Press narrative must complete the whole consequence arc: the player follows up,
/// reads that the case is hot, waits out the authored cold window, and verifies the case was
/// shelved through their own surveillance rather than hidden case access.
pub fn validate_press_consequence_arc(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    if metrics.followup_case_active == Some(true)
        && metrics.cold_case_confirmed == Some(true)
        && metrics.case_cold_minute.is_some()
        && metrics.case_cold_minute.unwrap_or_default()
            > metrics.burglary_terminal_minute.unwrap_or_default()
    {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the surfaced case must cool through the authored cold window and the player's own re-check must confirm the shelf",
        })
    }
}

/// Full-mode narrative sessions must close the defection loop: whenever an autonomous rival
/// departure actually removed a player member, the player's own surveillance of every known rival
/// must confirm where that member landed. A session without a departure must not fabricate a trail.
pub fn validate_defector_trail_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    if metrics.player_personnel_departures > 0 {
        if metrics.defector_trail_confirmed == Some(true) {
            Ok(())
        } else {
            Err(HarnessContractError::MissingStrategyEvidence {
                strategy,
                evidence: "after an autonomous rival departure removed a player member, the player's own surveillance of every known rival must confirm where the member landed",
            })
        }
    } else if metrics.defector_trail_confirmed.is_none() {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "a session without an autonomous rival departure must not fabricate a defector trail",
        })
    }
}

/// Full-mode narrative sessions must close the second-wind arc through canonical production paths:
/// every branch discovers the reopened second score at the same minute, then either rebuilds and
/// recovers value from it (RUSH via executive recruitment + morning-lull hit, RECON via fresh
/// recon + patrol-safe window) or deliberately lets it lapse as the price of standing down (PRESS).
/// A RECON branch whose own casing drew a police case must also read that case's activity through
/// its standing police contact before the window closes.
pub fn validate_second_act_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    let evidence = match strategy {
        Strategy::Rush => {
            if metrics.second_opportunity_discovered
                && metrics.replacement_recruited
                && metrics.replacement.is_some()
                && metrics.second_burglary.is_some()
                && metrics.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                && metrics.second_act_recon_information == 0
                && metrics.second_burglary_terminal_minute.is_some()
                && metrics.player_personnel_departures > 0
                // The debriefed abort must actually inform the rebuild: the second-score plan
                // carries the organization's debrief-derived police-response observation.
                && metrics.player_police_activity_information > 0
                && metrics
                    .second_act_planning_topics
                    .contains(&InformationTopic::PoliceActivity)
            {
                None
            } else {
                Some("the RUSH second act must discover the reopened score, debrief the aborted crew's police observations into organizational knowledge, rebuild through the canonical executive path, and work the second score in the morning lull with the rebuilt crew, no fresh recon, and the debriefed patrol information in the plan")
            }
        }
        Strategy::Recon => {
            if metrics.second_opportunity_discovered
                && metrics.second_burglary.is_some()
                && metrics.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                && metrics.second_act_recon_information > 0
                && metrics.second_burglary_terminal_minute.is_some()
                // Self-inflicted heat closes its loop: a case drawn by the branch's own
                // surveillance must be read through a player-visible channel (the standing
                // police contact), and a session whose casing drew no case must not fabricate
                // a read.
                && metrics.self_heat_case_active.is_some() == metrics.self_heat_case_opened
            {
                None
            } else {
                Some("the RECON second act must discover the reopened score, re-run surveillance on the alternate target, complete the burglary inside a fresh patrol-safe window, and read any surveillance-drawn case through its police contact")
            }
        }
        Strategy::Press => {
            if metrics.second_opportunity_discovered
                && metrics.second_opportunity_expired
                && metrics.second_burglary.is_none()
                && !metrics.replacement_recruited
            {
                None
            } else {
                Some("the PRESS second act must discover the reopened score and deliberately let it lapse while standing down, without recruiting or working the second score")
            }
        }
    };
    evidence.map_or(Ok(()), |evidence| {
        Err(HarnessContractError::MissingStrategyEvidence { strategy, evidence })
    })
}

/// Full-mode narrative PRESS sessions must convert the standing-down wait into governance:
/// the branch revises its mandate, capitalizes a second-district float from idle street cash,
/// and opens a diversified enterprise that the hot home-district case cannot tax. The harbor
/// book must also have actually earned by session end, proving the expansion is live economy,
/// not a paper establishment.
pub fn validate_press_expansion_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    if metrics.expansion_established && metrics.expansion_net_cents.is_some_and(|net| net > 0) {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the standing-down wait must end in district diversification: a revised mandate, a capitalized second-district enterprise, and positive harbor earnings by session end",
        })
    }
}

pub fn validate_harness_state(registry: &Registry, state: &AppState) -> Result<(), Box<dyn Error>> {
    validate_state(state)?;
    validate_state_against_registry(registry, state)?;
    Ok(())
}
