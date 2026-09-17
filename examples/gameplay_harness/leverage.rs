//! Same-window street-work leverage against leaving the existing organization running.
//! All observations are own books, own crews, or player-delivered reports; no case oracle.

use std::{error::Error, path::Path};

use crimocracy::{core::time::SimTime, registry::Registry};
use serde::Serialize;

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, build_scenario, format_cents,
    resolve_financial_view, run_until,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WindowEvidence {
    pub minute: u64,
    pub front_net_cents: i64,
    pub racket_net_cents: i64,
    pub realized_take_cents: i64,
    pub held_property_cents: i64,
    pub wages_paid_cents: i64,
    pub wages_short_cents: i64,
    /// Consolidated earned flow, not treasury balance or asset valuation. Laundry fees and
    /// owner draws move money between our books and therefore are not deducted again.
    pub earned_after_paid_wages_cents: i64,
    pub street_surcharge_cents: i64,
    pub operations: usize,
    pub crew_minutes: u64,
    pub departures: u32,
    pub decisions: u32,
    pub members: usize,
}

impl WindowEvidence {
    pub fn snapshot(scenario: &Scenario, metrics: &RunMetrics) -> Result<Self, Box<dyn Error>> {
        let view = resolve_financial_view(scenario, metrics)?;
        let racket_net_cents = view.enterprise_lines.iter().try_fold(0_i64, |sum, line| {
            sum.checked_add(line.net_cents)
                .ok_or("window racket flow overflow")
        })?;
        let street_surcharge_cents =
            view.enterprise_lines.iter().try_fold(0_i64, |sum, line| {
                sum.checked_add(line.heat_cents)
                    .ok_or("window surcharge overflow")
            })?;
        let earned = view
            .legitimate_net_cents
            .checked_add(racket_net_cents)
            .and_then(|sum| sum.checked_add(view.liquidated_property_cash_cents))
            .and_then(|sum| sum.checked_sub(metrics.payroll_paid_cents))
            .ok_or("window earned flow overflow")?;
        let mut operations = 0;
        let mut crew_minutes = 0_u64;
        for operation in scenario
            .state
            .operations()
            .operations_for_organization(scenario.player)
        {
            operations += 1;
            if let Some(start) = operation.started_at() {
                let end = operation
                    .resolution()
                    .map(|r| r.resolved_at())
                    .or_else(|| operation.abort_record().map(|a| a.aborted_at()))
                    .unwrap_or(scenario.state.now());
                let crew = operation.participants();
                let elapsed = end
                    .as_minutes()
                    .checked_sub(start.as_minutes())
                    .ok_or("operation ended before it started")?;
                crew_minutes = crew_minutes
                    .checked_add(
                        elapsed
                            .checked_mul(crew.len() as u64)
                            .ok_or("window crew time overflow")?,
                    )
                    .ok_or("window crew time overflow")?;
            }
        }
        Ok(Self {
            minute: scenario.state.now().as_minutes(),
            front_net_cents: view.legitimate_net_cents,
            racket_net_cents,
            realized_take_cents: view.liquidated_property_cash_cents,
            held_property_cents: view.held_property_value_cents,
            wages_paid_cents: metrics.payroll_paid_cents,
            wages_short_cents: metrics.payroll_short_cents,
            earned_after_paid_wages_cents: earned,
            street_surcharge_cents,
            operations,
            crew_minutes,
            departures: metrics.player_personnel_departures,
            decisions: metrics.decision_requests,
            members: scenario
                .state
                .world()
                .characters_in_organization(scenario.player)
                .count(),
        })
    }
}

/// The baseline declines the same initial opportunity, rather than receiving no opportunity.
/// Rackets, wages, rivals and legitimate trade continue through the same canonical ticks.
pub fn run_quiet_baseline(
    registry: &Registry,
    seeds: EvaluationSeeds,
    until: u64,
) -> Result<WindowEvidence, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    crate::session::discover_initial_opportunity(&mut scenario, false)?;
    let mut metrics = RunMetrics::default();
    run_until(
        &mut scenario,
        SimTime::from_minutes(until),
        false,
        &mut metrics,
    )?;
    WindowEvidence::snapshot(&scenario, &metrics)
}

pub fn print_and_write_comparison(
    registry: &Registry,
    seeds: EvaluationSeeds,
    branches: [&RunMetrics; 3],
    artifact_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let windows: Vec<_> = branches
        .iter()
        .map(|run| {
            run.matched_player_window
                .as_ref()
                .ok_or("missing matched player window")
        })
        .collect::<Result<_, _>>()?;
    let minute = windows[0].minute;
    if windows.iter().any(|window| window.minute != minute) {
        return Err("street-work comparison has unequal observation windows".into());
    }
    let baseline = run_quiet_baseline(registry, seeds, minute)?;
    println!(
        "\n--- WHAT DID STREET WORK BUY? (same first {} minutes) ---",
        minute
    );
    println!(
        "[BASELINE] No street jobs: existing fronts and racket earn {} after {} paid wages; {} unpaid. {} members remain. Opening capital excluded.",
        format_cents(baseline.earned_after_paid_wages_cents),
        format_cents(baseline.wages_paid_cents),
        format_cents(baseline.wages_short_cents),
        baseline.members
    );
    for (run, window) in branches.iter().zip(&windows) {
        let delta = window
            .earned_after_paid_wages_cents
            .checked_sub(baseline.earned_after_paid_wages_cents)
            .ok_or("window comparison overflow")?;
        println!(
            "[LEVERAGE] {}: earned {} ({:+} cents vs no jobs); take {}, street surcharge {}, held property {}. {} jobs / {} crew-minutes; {} departures, {} leadership exceptions, {} unpaid wages.",
            run.strategy.ok_or("comparison strategy missing")?.label(),
            format_cents(window.earned_after_paid_wages_cents),
            delta,
            format_cents(window.realized_take_cents),
            format_cents(window.street_surcharge_cents),
            format_cents(window.held_property_cents),
            window.operations,
            window.crew_minutes,
            window.departures,
            window.decisions,
            format_cents(window.wages_short_cents)
        );
    }
    println!(
        "[READ] Earned = legitimate net + racket net + realized property receipts - paid wages. Held property is separate; laundry fees and owner draws are internal transfers, not new earnings. Crew-minutes count people during execution, not player clicks. PRESS's later acquisition and recovery are outside this window."
    );
    if baseline.earned_after_paid_wages_cents > 0 && baseline.wages_short_cents == 0 {
        println!(
            "[CHOICE] These starting assets already support the crew without street jobs. Taking a score is optional acceleration, not survival; judge its extra earnings against lost people, exposure and follow-up work."
        );
    }
    std::fs::create_dir_all(artifact_dir)?;
    let path = artifact_dir.join(format!(
        "leverage-w{:016x}-p{:016x}.json",
        seeds.world, seeds.policy
    ));
    let treatments: Vec<_> = branches.iter().zip(windows).map(|(run, window)| {
        serde_json::json!({"strategy": run.strategy.expect("checked above").label(), "window": window})
    }).collect();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!({
            "world_seed": seeds.world, "policy_seed": seeds.policy,
            "player_visible": {"no_street_jobs": baseline, "treatments": treatments}
        }))?,
    )?;
    println!("[ARTIFACT] wrote {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionRunMode, Strategy, play_session};

    #[test]
    fn quiet_baseline_and_recon_compare_earned_flow_at_the_same_minute() {
        let registry = crimocracy::build_registry();
        let seeds = EvaluationSeeds::new(0x19330514, 0x19330514);
        let baseline = run_quiet_baseline(&registry, seeds, 2880).unwrap();
        let run = play_session(
            &registry,
            Strategy::Recon,
            ScenarioProfile::NightTrap,
            seeds,
            SessionRunMode::FullQuiet,
        )
        .unwrap();
        let window = run.matched_player_window.unwrap();
        assert_eq!(window.minute, baseline.minute);
        assert_eq!(baseline.operations, 0);
        assert_eq!(baseline.crew_minutes, 0);
        assert_eq!(baseline.realized_take_cents, 0);
        assert_eq!(window.front_net_cents, baseline.front_net_cents);
        assert_eq!(window.wages_paid_cents, baseline.wages_paid_cents);
        assert_eq!(window.operations, 4);
        assert!(window.crew_minutes > 0);
        assert_eq!(
            window.earned_after_paid_wages_cents - baseline.earned_after_paid_wages_cents,
            window.realized_take_cents + window.racket_net_cents - baseline.racket_net_cents
        );
        assert_eq!(
            baseline,
            run_quiet_baseline(&registry, seeds, 2880).unwrap()
        );
    }
}
