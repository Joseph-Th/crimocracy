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
    // Payroll is the standing carrying cost of headcount: any session that crossed a
    // campaign-day boundary must have met its wages from organizational cash.
    if require_financials && metrics.payroll_paid_cents <= 0 {
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "a session crossing a campaign-day boundary must meet payroll through the canonical ledger path",
        });
    }
    // The money state contract: whatever the organization routed through its front's books
    // must reconcile exactly. Accounted funds are laundering gross minus the front's authored
    // fee, minus legitimate acquisition spend, minus the exact wage debits that payroll's
    // ledger took from accounted-funds accounts. Payroll is organization-level and can draw
    // any spendable liquidity, so subtracting the whole wage bill would be just as wrong as
    // pretending clean money can never fund wages. A branch that liquidated stolen property
    // must also have laundered those proceeds rather than leaving all organizational value in
    // exposed street cash.
    if let Some(balance) = metrics.accounted_balance_cents
        && balance
            != metrics.laundered_gross_cents
                - metrics.launder_fee_cents
                - metrics.acquisition_spent_cents
                - metrics.payroll_accounted_spent_cents
    {
        return Err(HarnessContractError::InconsistentLaunderingEvidence {
            strategy,
            gross: metrics.laundered_gross_cents,
            fee: metrics.launder_fee_cents,
            acquisition: metrics.acquisition_spent_cents,
            payroll: metrics.payroll_accounted_spent_cents,
            balance: Some(balance),
        });
    }
    if metrics.property_realized_cash_cents.is_some() && metrics.laundered_gross_cents == 0 {
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "liquidated proceeds must be laundered through an owned cash-intensive front before they count as spendable organizational money",
        });
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
                Some(
                    "pre-entry police arrival triggers the standing abort contingency and the crew's direct observations are debriefed into organization knowledge",
                )
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
                Some(
                    "police-arrival decision, surfaced field/legal information, and a counter-surveillance follow-up that reads whether the case is still active",
                )
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
                Some(
                    "surveillance information must carry both patrol and venue-access facts into the burglary plan",
                )
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
        // RUSH owns a standing pre-entry abort contingency, but whether police physically reach
        // the target before entry is a world outcome, not a policy guarantee. When that event
        // occurs, validate the whole abort/debrief chain. When it does not, ordinary terminal
        // validity is already covered by `validate_run_metrics`; batch reachability is checked
        // separately across sufficiently broad samples.
        Strategy::Rush
            if profile != ScenarioProfile::LatePatrol
                && matches!(
                    metrics.abort_cause,
                    Some(OperationAbortCause::PoliceArrival(_))
                ) =>
        {
            validate_night_trap_evidence(metrics)
        }
        Strategy::Press if metrics.police_arrived => validate_night_trap_evidence(metrics),
        Strategy::Recon => validate_night_trap_evidence(metrics),
        Strategy::Rush | Strategy::Press => Ok(()),
    }
}

/// Statistical batches validate event reachability at the aggregate level instead of requiring
/// one stochastic/world-dependent consequence in every run. Small batches are valid bounded
/// samples but cannot establish coverage, so only the same three-variation threshold used by the
/// fixture contract upgrades absence into a regression failure.
pub fn validate_batch_strategy_coverage(
    profile: ScenarioProfile,
    samples: u64,
    rush: &Aggregate,
) -> Result<(), HarnessContractError> {
    if samples < MIN_SAMPLES_FOR_VARIATION_CONTRACT || profile == ScenarioProfile::LatePatrol {
        return Ok(());
    }
    if rush.standing_contingency_aborts == 0 {
        return Err(HarnessContractError::MissingBatchEvidence {
            profile,
            evidence: "RUSH never encountered a pre-entry police arrival across the covered fixture variations, so the standing abort/debrief consequence was not demonstrated",
        });
    }
    Ok(())
}

/// Full-mode Press narrative must complete the whole consequence arc: the player follows up,
/// reads that the case is hot, then polls its standing police contact until the channel itself
/// carries the cooled read - the contact's knowledge being production investigator state, not
/// hidden case access. The institution may end the case either way production rules allow:
/// quiet cases decay cold, and cases escalated into custody can close outright.
pub fn validate_press_consequence_arc(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    if metrics.followup_case_active == Some(true)
        && metrics.cold_case_confirmed == Some(true)
        && metrics.case_cold_minute.is_some()
        && metrics.case_cold_minute.unwrap_or_default()
            > metrics.burglary_terminal_minute.unwrap_or_default()
        && metrics.contact_reads > 0
    {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the surfaced case must end through an institutional ending (cold shelf or closure) and the player's standing police contact must confirm it through a canonical disclosure",
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

/// Full-mode narrative sessions must close the personnel loop end to end: whenever a departure
/// actually happened and the defector trail confirmed a landing spot, leadership must make the
/// one canonical executive re-approach. A refused re-approach must surface its intelligence
/// cost (the production loyalty report the rival receives naming our recruiter). A session
/// without a departure must not fabricate a win-back.
pub fn validate_win_back_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    let strategy = metrics
        .strategy
        .ok_or(HarnessContractError::MissingStrategy)?;
    if metrics.player_personnel_departures == 0 {
        if !metrics.win_back_attempted {
            return Ok(());
        }
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "a session without an autonomous departure must not attempt a win-back",
        });
    }
    if !metrics.win_back_attempted || metrics.win_back_accepted.is_none() {
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "after a confirmed defector trail, leadership must attempt the canonical executive win-back re-approach",
        });
    }
    match metrics.win_back_accepted {
        Some(true) => Ok(()),
        Some(false) if metrics.win_back_refusal_leaked_to_rival == Some(true) => Ok(()),
        Some(false) => Err(HarnessContractError::MissingStrategyEvidence {
            strategy,
            evidence: "a refused win-back must surface its intelligence cost through the production loyalty report delivered to the recruiting organization",
        }),
        None => unreachable!("an attempted win-back always records its outcome"),
    }
}

/// Full-mode narrative sessions must close the second-wind arc through canonical production paths:
/// every branch discovers the reopened second score at the same minute. RUSH rebuilds and works
/// it, PRESS deliberately lets it lapse, and RECON acts on what its fresh casing actually reveals:
/// work the patrol-safe window when the casing stays clean or its case is explicitly shelved;
/// otherwise query its standing police contact and stand down when that casing opens a case that
/// the channel cannot affirmatively clear.
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
                Some(
                    "the RUSH second act must discover the reopened score, debrief the aborted crew's police observation into organizational knowledge, rebuild through the canonical executive path, move the retry away from the known-hot overnight hour, and carry that debriefed PoliceActivity information in the plan without pretending it is a full patrol pattern",
                )
            }
        }
        Strategy::Recon => {
            let recovered_when_clear = metrics.second_opportunity_discovered
                && metrics.second_burglary.is_some()
                && metrics.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                && metrics.second_act_recon_information > 0
                && metrics.second_burglary_terminal_minute.is_some()
                && (!metrics.self_heat_case_opened || metrics.self_heat_case_active == Some(false));
            let stood_down_on_self_heat = metrics.second_opportunity_discovered
                && metrics.second_act_recon_information > 0
                && metrics.self_heat_case_opened
                && metrics.self_heat_case_active != Some(false)
                && metrics.second_burglary.is_none()
                && metrics.second_opportunity_expired;
            if recovered_when_clear || stood_down_on_self_heat {
                None
            } else {
                Some(
                    "the RECON second act must discover the reopened score and re-run surveillance; a clean or explicitly shelved casing case permits the patrol-safe burglary, while a casing case that the police contact confirms active or cannot dependably clear must make the branch stand down until the opportunity expires",
                )
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
                Some(
                    "the PRESS second act must discover the reopened score and deliberately let it lapse while standing down, without recruiting or working the second score",
                )
            }
        }
    };
    evidence.map_or(Ok(()), |evidence| {
        Err(HarnessContractError::MissingStrategyEvidence { strategy, evidence })
    })
}

/// Full-mode narrative PRESS must complete the standing-down arc. The primary comparison
/// set anchors the strict legitimate-wealth demonstration deterministically: launder day by
/// day until the accounted books cover the venue's authored price, buy the independent
/// harbor club outright through the canonical acquisition path (a short book must first
/// surface as a canonical rejection), revise the mandate, capitalize a second-district
/// float, and open a diversified enterprise the hot home case cannot tax, with positive
/// harbor earnings by session end. Rotated sets run the same production paths on worlds
/// whose authored economics may legitimately stall conversion (a concealed till, or a
/// young racket whose heat-taxed float never accumulates); there the contract requires an
/// honest ending - the cooled read confirmed through the contact channel, and if a purchase
/// did happen, the full chain behind it.
pub fn validate_press_expansion_evidence(metrics: &RunMetrics) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    let concealed_till = metrics.enterprise_till_concealed == Some(true);
    let acquisition_complete = metrics.front_acquired
        && metrics.acquisition_price_cents.is_some_and(|price| price > 0)
        && metrics.acquisition_spent_cents == metrics.acquisition_price_cents.unwrap_or_default()
        // The legitimacy gate must be visible: the branch attempted the purchase before
        // its accounted books could cover the authored price at least once.
        && metrics.acquisition_rejections > 0;
    let diversified = acquisition_complete
        && metrics.expansion_established
        && metrics.expansion_net_cents.is_some_and(|net| net > 0)
        && metrics.expansion_heat_cents == Some(0);
    let clean_expansion_started = metrics.front_acquired
        && metrics.expansion_established
        && metrics.expansion_heat_cents == Some(0);
    let survived = metrics.cold_case_confirmed == Some(true) && !metrics.front_acquired;
    if !metrics.primary_narrative_set {
        if (diversified || survived || clean_expansion_started)
            && metrics.cold_case_confirmed == Some(true)
        {
            return Ok(());
        }
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the rotated standing-down wait must end honestly: the cooled read confirmed, with either the survival ending or a consistent diversification chain",
        });
    }
    if concealed_till {
        if survived {
            return Ok(());
        }
        return Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "a concealed-till standing-down wait must end in survival: the shelved read confirmed through the contact channel and no purchase attempted, because hidden money cannot be cleaned",
        });
    }
    if diversified {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "the standing-down wait must end in legitimate wealth and real district diversification: a canonical accounted-funds acquisition of the harbor venue (after a visible short-book rejection), a revised mandate, a capitalized second-district enterprise, positive harbor earnings, and zero harbor surcharge from the unrelated home-district case",
        })
    }
}

/// Full-mode narrative PRESS must answer a witnessed job with the canonical counter-play:
/// the case names its on-scene witness at intake and leadership runs one WitnessPressure
/// operation against that person. Both terminal shapes are honest evidence:
/// a landed pressure degrades the witness's registered cooperation, while an abort under the
/// police-arrival contingency proves quiet counter-play in a watched district carries real
/// risk and that discipline contains it (an InProgress abort leaves no second case).
pub fn validate_press_witness_counterplay(
    metrics: &RunMetrics,
) -> Result<(), HarnessContractError> {
    if metrics.strategy != Some(Strategy::Press) {
        return Ok(());
    }
    if !metrics.investigation_created {
        return Ok(());
    }
    let attempted = metrics.witness_pressure_attempted;
    let landed = metrics
        .witness_pressure_outcome
        .is_some_and(|outcome| outcome != OperationObjectiveOutcome::Failed)
        && metrics.witness_cooperation_degraded;
    let disciplined_abort =
        metrics.witness_pressure_aborted && !metrics.witness_cooperation_degraded;
    if metrics.case_witness_registered && attempted && (landed || disciplined_abort) {
        Ok(())
    } else {
        Err(HarnessContractError::MissingStrategyEvidence {
            strategy: Strategy::Press,
            evidence: "a witnessed job on a character-owned business must name its case witness at intake and draw one canonical WitnessPressure operation that either lands (degrading the witness's cooperation) or aborts under its police-arrival contingency without opening a second case",
        })
    }
}

pub fn validate_harness_state(registry: &Registry, state: &AppState) -> Result<(), Box<dyn Error>> {
    validate_state(state)?;
    validate_state_against_registry(registry, state)?;
    Ok(())
}
