//! Matched own-book earnings under keep-open versus suspend/resume policy.

use std::{error::Error, path::Path};

use crimocracy::core::time::{DAY_DURATION, SimTime};
use crimocracy::enterprises::enterprise_execution::{
    validate_resume_enterprise, validate_suspend_enterprise,
};
use crimocracy::registry::Registry;
use serde::Serialize;

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, build_scenario, format_cents,
    resolve_financial_view, run_until, validate_harness_state,
};

#[derive(Debug, PartialEq, Serialize)]
pub struct PostureMoney {
    front_cycles: u32,
    front_net_cents: i64,
    enterprise_cycles: usize,
    enterprise_net_cents: i64,
    payroll_paid_cents: i64,
    payroll_short_cents: i64,
}

impl PostureMoney {
    fn snapshot(scenario: &Scenario, metrics: &RunMetrics) -> Result<Self, Box<dyn Error>> {
        let view = resolve_financial_view(scenario, metrics)?;
        Ok(Self {
            front_cycles: view.legitimate_cycle_count,
            front_net_cents: view.legitimate_net_cents,
            enterprise_cycles: view.enterprise_cycle_count,
            enterprise_net_cents: view.enterprise_net_cents,
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
            payroll_paid_cents: self.payroll_paid_cents - baseline.payroll_paid_cents,
            payroll_short_cents: self.payroll_short_cents - baseline.payroll_short_cents,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PostureEvidence {
    world_seed: u64,
    policy_seed: u64,
    window_start_minute: u64,
    window_end_minute: u64,
    keep_open: PostureMoney,
    suspended: PostureMoney,
    resumed_due_minute: u64,
    resumed_cycle_net_cents: i64,
    case_clearance_claimed: bool,
}

/// Fixed policy, not a reaction to hidden case state. No incidents or outcomes are forced.
/// The two-day financial comparison excludes both the shared first cycle and resume tail.
pub fn run_enterprise_posture_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    artifact_dir: Option<&Path>,
) -> Result<PostureEvidence, Box<dyn Error>> {
    let mut open = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut paused = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut open_metrics = RunMetrics::default();
    let mut paused_metrics = RunMetrics::default();
    let first_due = open
        .state
        .enterprises()
        .get_enterprise(open.enterprise)
        .and_then(|record| record.next_cycle_at())
        .ok_or("posture first cycle missing")?;
    // Scenario is not Clone: recreate with identical explicit seeds and advance both.
    run_until(&mut open, first_due, false, &mut open_metrics)?;
    run_until(&mut paused, first_due, false, &mut paused_metrics)?;
    if bincode::serialize(&open.state)? != bincode::serialize(&paused.state)? {
        return Err("posture branches did not reach identical starting state".into());
    }
    let baseline = PostureMoney::snapshot(&open, &open_metrics)?;
    if baseline.enterprise_cycles != 1 {
        return Err("posture baseline must contain exactly one settled home cycle".into());
    }
    let enterprise = paused.enterprise;
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
    let window_end = first_due + DAY_DURATION + DAY_DURATION;
    run_until(&mut open, window_end, false, &mut open_metrics)?;
    run_until(&mut paused, window_end, false, &mut paused_metrics)?;
    let keep_open = PostureMoney::snapshot(&open, &open_metrics)?.since(&baseline);
    let suspended = PostureMoney::snapshot(&paused, &paused_metrics)?.since(&baseline);
    if keep_open.enterprise_cycles == 0
        || suspended.enterprise_cycles != 0
        || suspended.enterprise_net_cents != 0
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
    if paused.state.enterprises().cycles_for(enterprise).count() != baseline.enterprise_cycles {
        return Err("suspended time paid out before the resumed cycle was due".into());
    }
    run_until(&mut paused, resumed_due, false, &mut paused_metrics)?;
    if paused.state.enterprises().cycles_for(enterprise).count() != baseline.enterprise_cycles + 1 {
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
        window_start_minute: first_due.as_minutes(),
        window_end_minute: window_end.as_minutes(),
        keep_open,
        suspended,
        resumed_due_minute: resumed_due.as_minutes(),
        resumed_cycle_net_cents: resumed_cycle.net_cash().cents(),
        case_clearance_claimed: false,
    };
    validate_harness_state(registry, &paused.state)?;
    println!(
        "[POSTURE] Same two days: open racket net {}, suspended {}. Front net {} in each; wages paid open {} / suspended {} (short {} / {}).",
        format_cents(evidence.keep_open.enterprise_net_cents),
        format_cents(evidence.suspended.enterprise_net_cents),
        format_cents(evidence.keep_open.front_net_cents),
        format_cents(evidence.keep_open.payroll_paid_cents),
        format_cents(evidence.suspended.payroll_paid_cents),
        format_cents(evidence.keep_open.payroll_short_cents),
        format_cents(evidence.suspended.payroll_short_cents),
    );
    println!(
        "[POSTURE] Resume waited a full cycle, then settled {} once; no backlog payout. Suspension stops racket settlements, not front trading or wages. This is not evidence that suspension clears cases.",
        format_cents(evidence.resumed_cycle_net_cents),
    );
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
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_and_wages_continue_when_home_racket_is_suspended_then_resumed() {
        let registry = crimocracy::build_registry();
        let evidence = run_enterprise_posture_probe(
            &registry,
            EvaluationSeeds::new(0x1933, 0x504F_4C49_4359),
            None,
        )
        .expect("matched posture must preserve continuity and restart without backlog");
        assert_eq!(
            evidence.window_end_minute - evidence.window_start_minute,
            2_880
        );
        assert_eq!(evidence.suspended.enterprise_cycles, 0);
        assert_eq!(evidence.suspended.enterprise_net_cents, 0);
        assert!(evidence.keep_open.enterprise_cycles > 0);
        assert!(!evidence.case_clearance_claimed);
        assert!(serde_json::to_value(&evidence).is_ok());
    }
}
