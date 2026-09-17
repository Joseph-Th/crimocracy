//! Matched keep-open versus suspend/resume policy after a real reported street surcharge.
//!
//! React to an organization-held surcharge report, not hidden case state. Both branches
//! share the initial PRESS job; only the subsequent racket posture differs. The fixed
//! eight-day comparison excludes the common trigger settlement and the resume tail.

use std::{error::Error, path::Path};

use crimocracy::core::time::{DAY_MINUTES, SimDuration, SimTime};
use crimocracy::enterprises::enterprise_execution::{
    validate_resume_enterprise, validate_suspend_enterprise,
};
use crimocracy::registry::Registry;
use serde::Serialize;

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, Strategy, build_scenario,
    enterprise_label, format_cents, resolve_financial_view, run_initial_burglary, run_until,
    validate_harness_state,
};

/// Campaign days in the matched window: long enough to cover several racket cycles and
/// every daily payroll between the trigger and the resumed tail.
const POSTURE_WINDOW_DAYS: u64 = 8;

fn manager_report<'a>(
    scenario: &'a Scenario,
    cycle: &crimocracy::enterprises::EnterpriseCycleRecord,
) -> Result<&'a str, Box<dyn Error>> {
    let information = cycle
        .information()
        .and_then(|id| scenario.state.intelligence().get_information(id))
        .ok_or("reportable posture cycle has no manager information")?;
    if information.holder()
        != crimocracy::intelligence::KnowledgeHolder::Organization(scenario.player)
        || information.subject()
            != crimocracy::core::entity::EntityRef::Enterprise(scenario.enterprise)
    {
        return Err("posture report is not organization-held home-racket knowledge".into());
    }
    Ok(information.summary())
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PostureMoney {
    front_cycles: u32,
    front_net_cents: i64,
    enterprise_cycles: usize,
    enterprise_net_cents: i64,
    home_heat_paid_cents: i64,
    home_vice_warnings: u32,
    payroll_paid_cents: i64,
    payroll_short_cents: i64,
}

impl PostureMoney {
    /// Cumulative organization books so far; `since` isolates the matched window.
    fn snapshot(scenario: &Scenario, metrics: &RunMetrics) -> Result<Self, Box<dyn Error>> {
        let view = resolve_financial_view(scenario, metrics)?;
        for cycle in scenario
            .state
            .enterprises()
            .cycles_for(scenario.enterprise)
            .filter(|cycle| cycle.drew_vice_attention())
        {
            // Only count warnings whose production observation actually reached leadership.
            manager_report(scenario, cycle)?;
        }
        let (cycles, net, heat, warnings) = scenario
            .state
            .enterprises()
            .cycles_for(scenario.enterprise)
            .fold(
                (0_usize, 0_i64, 0_i64, 0_u32),
                |(cycles, net, heat, warnings), cycle| {
                    (
                        cycles + 1,
                        net + cycle.net_cash().cents(),
                        heat + cycle.investigation_heat().cents(),
                        warnings + u32::from(cycle.drew_vice_attention()),
                    )
                },
            );
        Ok(Self {
            front_cycles: view.legitimate_cycle_count,
            front_net_cents: view.legitimate_net_cents,
            enterprise_cycles: cycles,
            enterprise_net_cents: net,
            home_heat_paid_cents: heat,
            home_vice_warnings: warnings,
            payroll_paid_cents: metrics.payroll_paid_cents,
            payroll_short_cents: metrics.payroll_short_cents,
        })
    }

    fn since(self, baseline: &Self) -> Self {
        Self {
            front_cycles: self.front_cycles - baseline.front_cycles,
            front_net_cents: self.front_net_cents - baseline.front_net_cents,
            enterprise_cycles: self.enterprise_cycles - baseline.enterprise_cycles,
            enterprise_net_cents: self.enterprise_net_cents - baseline.enterprise_net_cents,
            home_heat_paid_cents: self.home_heat_paid_cents - baseline.home_heat_paid_cents,
            home_vice_warnings: self.home_vice_warnings - baseline.home_vice_warnings,
            payroll_paid_cents: self.payroll_paid_cents - baseline.payroll_paid_cents,
            payroll_short_cents: self.payroll_short_cents - baseline.payroll_short_cents,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PostureTrigger {
    minute: u64,
    venue: String,
    surcharge_cents: i64,
    home_net_cents: i64,
    vice_warning_observed: bool,
    manager_report: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PostureEvidence {
    world_seed: u64,
    policy_seed: u64,
    trigger: PostureTrigger,
    window_start_minute: u64,
    window_end_minute: u64,
    keep_open: PostureMoney,
    suspended: PostureMoney,
    keep_open_final_status: crimocracy::enterprises::EnterpriseStatus,
    resumed_due_minute: u64,
    resumed_cycle_net_cents: i64,
    resumed_cycle_heat_cents: i64,
    case_clearance_claimed: bool,
}

/// Fixed policy, not a reaction to hidden case state. The trigger is the organization's own
/// manager report on a hot cycle; no incident is forced and no hidden investigation is read.
/// The eight-day comparison excludes the shared trigger cycle and the resume tail.
pub fn run_enterprise_posture_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    artifact_dir: Option<&Path>,
) -> Result<Option<PostureEvidence>, Box<dyn Error>> {
    let mut open = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut paused = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut open_metrics = RunMetrics {
        strategy: Some(Strategy::Press),
        ..RunMetrics::default()
    };
    let mut paused_metrics = RunMetrics {
        strategy: Some(Strategy::Press),
        ..RunMetrics::default()
    };
    // Both branches play the identical opening burglary arc so the only later difference
    // is the posture decision itself.
    run_initial_burglary(&mut open, Strategy::Press, false, &mut open_metrics)?;
    run_initial_burglary(&mut paused, Strategy::Press, false, &mut paused_metrics)?;
    if bincode::serialize(&open.state)? != bincode::serialize(&paused.state)? {
        return Err("posture branches did not reach identical state after the opening arc".into());
    }

    // Bounded search for the player-visible trigger: the first home cycle whose manager
    // report carries a street surcharge. An absent trigger is recorded without injecting cases.
    let enterprise = paused.enterprise;
    let first_due = open
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .and_then(|record| record.next_cycle_at())
        .ok_or("posture first cycle missing")?;
    let mut trigger_time = first_due;
    let trigger_cycle = loop {
        run_until(&mut open, trigger_time, false, &mut open_metrics)?;
        run_until(&mut paused, trigger_time, false, &mut paused_metrics)?;
        let latest = open
            .state
            .enterprises()
            .latest_cycle(enterprise)
            .filter(|cycle| cycle.occurred_at() == trigger_time);
        if let Some(cycle) = latest
            && cycle.investigation_heat().cents() > 0
        {
            let report = manager_report(&open, cycle)?;
            break PostureTrigger {
                minute: trigger_time.as_minutes(),
                venue: enterprise_label(&open, enterprise),
                surcharge_cents: cycle.investigation_heat().cents(),
                home_net_cents: cycle.net_cash().cents(),
                vice_warning_observed: cycle.drew_vice_attention(),
                manager_report: report.to_owned(),
            };
        }
        if trigger_time.as_minutes() >= first_due.as_minutes() + 4 * DAY_MINUTES {
            println!(
                "[POSTURE ABSENT] No new home-racket surcharge report within four days of the first due cycle; no posture comparison was performed. Due work may be blocked by personnel availability; no case was injected."
            );
            return Ok(None);
        }
        // Due work can be blocked by custody. Poll the next canonical minute, not the
        // unchanged overdue schedule; absence of a settlement is not a zero-cost cycle.
        trigger_time = trigger_time + SimDuration::ONE_MINUTE;
    };
    if bincode::serialize(&open.state)? != bincode::serialize(&paused.state)? {
        return Err("posture branches diverged before the trigger".into());
    }
    let trigger = trigger_cycle;
    let baseline = PostureMoney::snapshot(&open, &open_metrics)?;
    if baseline.enterprise_cycles == 0 || baseline.home_heat_paid_cents <= 0 {
        return Err("posture baseline must contain the reported hot cycle".into());
    }

    // The one policy difference: the suspended branch closes the home racket after the
    // same manager report. Keep-open branch changes nothing.
    let cycle_duration = registry
        .get_enterprise(
            paused
                .state
                .enterprises()
                .get_enterprise(enterprise)
                .ok_or("posture enterprise missing")?
                .kind(),
        )
        .economics()
        .cycle();
    validate_suspend_enterprise(&paused.state, enterprise)?.commit(&mut paused.state)?;
    let window_duration = SimDuration::from_minutes((POSTURE_WINDOW_DAYS * DAY_MINUTES) as u32);
    let window_start = trigger_time;
    let window_end = trigger_time + window_duration;
    run_until(&mut open, window_end, false, &mut open_metrics)?;
    run_until(&mut paused, window_end, false, &mut paused_metrics)?;
    let keep_open = PostureMoney::snapshot(&open, &open_metrics)?.since(&baseline);
    let suspended = PostureMoney::snapshot(&paused, &paused_metrics)?.since(&baseline);
    let keep_open_final_status = open
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .map(crimocracy::enterprises::EnterpriseRecord::status)
        .ok_or("posture enterprise missing at readout")?;
    if keep_open.enterprise_cycles == 0
        || suspended.enterprise_cycles != 0
        || suspended.enterprise_net_cents != 0
        || suspended.home_heat_paid_cents != 0
        || suspended.home_vice_warnings != 0
        || keep_open.front_cycles == 0
        || keep_open.front_cycles != suspended.front_cycles
        || keep_open.front_net_cents != suspended.front_net_cents
        || keep_open.payroll_paid_cents <= 0
        || suspended.payroll_paid_cents <= 0
    {
        return Err(format!(
            "posture continuity failed: open {keep_open:?}, suspended {suspended:?}"
        )
        .into());
    }
    validate_harness_state(registry, &open.state)?;
    validate_harness_state(registry, &paused.state)?;

    // Resume tail, outside the comparison: reopening schedules a full new cycle and the
    // reopened books may still report district pressure.
    validate_resume_enterprise(registry, &paused.state, enterprise)?.commit(&mut paused.state)?;
    let resumed_due = paused
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .and_then(|record| record.next_cycle_at())
        .ok_or("resumed cycle missing")?;
    if resumed_due != window_end + cycle_duration {
        return Err("resume did not schedule a full new cycle".into());
    }
    run_until(
        &mut paused,
        SimTime::from_minutes(resumed_due.as_minutes() - 1),
        false,
        &mut paused_metrics,
    )?;
    let baseline_cycles = baseline.enterprise_cycles;
    if paused.state.enterprises().cycles_for(enterprise).count() != baseline_cycles {
        return Err("suspended time paid out before the resumed cycle was due".into());
    }
    run_until(&mut paused, resumed_due, false, &mut paused_metrics)?;
    if paused.state.enterprises().cycles_for(enterprise).count() != baseline_cycles + 1 {
        return Err("resume must settle exactly one new cycle, not a backlog".into());
    }
    let resumed_cycle = paused
        .state
        .enterprises()
        .cycles_for(enterprise)
        .last()
        .ok_or("resumed settlement missing")?;
    if resumed_cycle.occurred_at() != resumed_due {
        return Err("resumed settlement timestamp does not match the new schedule".into());
    }
    let evidence = PostureEvidence {
        world_seed: seeds.world,
        policy_seed: seeds.policy,
        trigger,
        window_start_minute: window_start.as_minutes(),
        window_end_minute: window_end.as_minutes(),
        keep_open,
        suspended,
        keep_open_final_status,
        resumed_due_minute: resumed_due.as_minutes(),
        resumed_cycle_net_cents: resumed_cycle.net_cash().cents(),
        resumed_cycle_heat_cents: resumed_cycle.investigation_heat().cents(),
        case_clearance_claimed: false,
    };
    validate_harness_state(registry, &paused.state)?;
    print_posture_readout(&evidence);
    if let Some(dir) = artifact_dir {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "posture-w{:016x}-p{:016x}.json",
            seeds.world, seeds.policy
        ));
        let payload = serde_json::json!({ "player_visible": &evidence });
        std::fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
        println!("[ARTIFACT] wrote {}", path.display());
    }
    Ok(Some(evidence))
}

fn print_posture_readout(evidence: &PostureEvidence) {
    println!(
        "[POSTURE TRIGGER] minute {}, the {} racket paid a {} street surcharge (net {} that cycle{}). Leadership decides what the racket itself should do.",
        evidence.trigger.minute,
        evidence.trigger.venue,
        format_cents(evidence.trigger.surcharge_cents),
        format_cents(evidence.trigger.home_net_cents),
        if evidence.trigger.vice_warning_observed {
            "; the manager also reported vice officers watching the venue"
        } else {
            ""
        },
    );
    println!("[POSTURE REPORT] {}", evidence.trigger.manager_report);
    println!(
        "[POSTURE] Keep-open, next {} days: {} cycle(s) netting {}, street surcharge paid {}, manager-observed vice warning(s) {}, final status {:?}.",
        (evidence.window_end_minute - evidence.window_start_minute) / DAY_MINUTES,
        evidence.keep_open.enterprise_cycles,
        format_cents(evidence.keep_open.enterprise_net_cents),
        format_cents(evidence.keep_open.home_heat_paid_cents),
        evidence.keep_open.home_vice_warnings,
        evidence.keep_open_final_status,
    );
    println!(
        "[POSTURE] Suspended, same window: no settlements and no surcharge; income forgone {}. Fronts traded identically in both branches ({} net each); wages were paid in both ({} open / {} suspended, short {} / {}). Suspension stops this racket's settlements and new cycle-based vice draws, not wages or front trade; district pressure can persist independently.",
        format_cents(evidence.keep_open.enterprise_net_cents),
        format_cents(evidence.keep_open.front_net_cents),
        format_cents(evidence.keep_open.payroll_paid_cents),
        format_cents(evidence.suspended.payroll_paid_cents),
        format_cents(evidence.keep_open.payroll_short_cents),
        format_cents(evidence.suspended.payroll_short_cents),
    );
    println!(
        "[POSTURE RESUME] Reopening waited a full cycle, then settled {} once (street surcharge {}); no backlog payout. This is not evidence that suspension clears cases.",
        format_cents(evidence.resumed_cycle_net_cents),
        format_cents(evidence.resumed_cycle_heat_cents),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_report_then_suspend_preserves_trade_and_full_cycle_restart() {
        let registry = crimocracy::build_registry();
        let evidence = run_enterprise_posture_probe(&registry, EvaluationSeeds::defaults(), None)
            .expect("matched posture must preserve continuity and restart without backlog")
            .expect("primary PRESS world must produce a reported surcharge");
        assert_eq!(
            evidence.window_end_minute - evidence.window_start_minute,
            POSTURE_WINDOW_DAYS * DAY_MINUTES
        );
        assert!(evidence.trigger.surcharge_cents > 0);
        assert!(evidence.keep_open.enterprise_cycles > 0);
        assert_eq!(evidence.suspended.enterprise_cycles, 0);
        assert_eq!(evidence.suspended.enterprise_net_cents, 0);
        assert_eq!(evidence.suspended.home_heat_paid_cents, 0);
        assert_eq!(evidence.suspended.home_vice_warnings, 0);
        assert_eq!(
            evidence.keep_open.front_cycles, evidence.suspended.front_cycles,
            "front trade must continue identically under both policies"
        );
        assert_eq!(
            evidence.keep_open.front_net_cents,
            evidence.suspended.front_net_cents,
        );
        assert!(evidence.keep_open.payroll_paid_cents > 0);
        assert!(evidence.suspended.payroll_paid_cents > 0);
        assert!(!evidence.case_clearance_claimed);
        assert!(serde_json::to_value(&evidence).is_ok());
    }
}
