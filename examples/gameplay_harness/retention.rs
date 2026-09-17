//! Recovery versus retention, compared from one post-win-back state through two retry windows.

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, Strategy, build_scenario,
    run_initial_burglary, run_personnel_recovery, run_until, stamp,
};
use crimocracy::core::{
    id::CharacterId,
    time::{DAY_MINUTES, SimTime},
};
use crimocracy::registry::Registry;
use crimocracy::world::world_system::validate_reassign_character;
use serde::Serialize;
use std::{error::Error, path::Path};

#[derive(Debug, Serialize)]
struct RetentionArm {
    restored_reporting_line: bool,
    retained: bool,
    departures: u32,
    loyalty_warnings: u32,
    wages_paid_cents: i64,
    wages_short_cents: i64,
    reports: Vec<PersonnelReport>,
}

#[derive(Debug, Serialize)]
struct PersonnelReport {
    report: crimocracy::core::id::ReportId,
    minute: u64,
    summary: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RetentionEvidence {
    world_seed: u64,
    policy_seed: u64,
    start_minute: u64,
    end_minute: u64,
    leave_under_recruiter: RetentionArm,
    restore_trusted_supervisor: RetentionArm,
}

/// An explicit management decision, not an invisible change to the recruitment transaction.
/// The fixture's own crew relationships establish the trusted reporting line before play.
pub(crate) fn restore_reporting_line(
    scenario: &mut Scenario,
    member: CharacterId,
) -> Result<(), Box<dyn Error>> {
    validate_reassign_character(
        &scenario.state,
        member,
        Some(scenario.player),
        Some(scenario.lieutenant),
    )?
    .commit(&mut scenario.state)?;
    Ok(())
}

fn run_arm(
    mut scenario: Scenario,
    member: CharacterId,
    restore: bool,
    end: SimTime,
) -> Result<RetentionArm, Box<dyn Error>> {
    let start = scenario.state.now();
    if restore {
        restore_reporting_line(&mut scenario, member)?;
    }
    let mut metrics = RunMetrics::default();
    run_until(&mut scenario, end, false, &mut metrics)?;
    let reports = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.generated_at() > start)
        .filter(|report| report.kind() != crimocracy::reports::ReportKind::ExecutiveBrief)
        .flat_map(|report| {
            report
                .entries()
                .iter()
                .filter(move |entry| {
                    entry
                        .entities
                        .contains(&crimocracy::core::entity::EntityRef::Character(member))
                })
                .map(move |entry| PersonnelReport {
                    report: report.id(),
                    minute: report.generated_at().as_minutes(),
                    summary: entry.summary.clone(),
                })
        })
        .collect();
    Ok(RetentionArm {
        restored_reporting_line: restore,
        retained: scenario
            .state
            .world()
            .get_character(member)
            .unwrap()
            .organization()
            == Some(scenario.player),
        departures: metrics.player_personnel_departures,
        loyalty_warnings: metrics.player_poach_warnings,
        wages_paid_cents: metrics.payroll_paid_cents,
        wages_short_cents: metrics.payroll_short_cents,
        reports,
    })
}

fn collect(
    registry: &Registry,
    seeds: EvaluationSeeds,
) -> Result<Option<RetentionEvidence>, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        ..RunMetrics::default()
    };
    run_initial_burglary(&mut scenario, Strategy::Rush, false, &mut metrics)?;
    run_until(
        &mut scenario,
        SimTime::from_minutes(DAY_MINUTES + 1),
        false,
        &mut metrics,
    )?;
    run_personnel_recovery(&mut scenario, false, &mut metrics)?;
    if metrics.win_back_accepted != Some(true) {
        return Ok(None);
    }
    let member = metrics
        .defector
        .ok_or("accepted recovery must name member")?;
    let start_minute = scenario.state.now().as_minutes();
    // A fixed observation treatment derived from production cadence, not hidden rival attempts.
    let end_minute =
        start_minute + 2 * u64::from(registry.recruitment().cooldown().as_minutes()) + DAY_MINUTES;
    let end = SimTime::from_minutes(end_minute);
    Ok(Some(RetentionEvidence {
        world_seed: seeds.world,
        policy_seed: seeds.policy,
        start_minute,
        end_minute,
        leave_under_recruiter: run_arm(scenario.clone(), member, false, end)?,
        restore_trusted_supervisor: run_arm(scenario, member, true, end)?,
    }))
}

pub(crate) fn run_retention_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    directory: &Path,
) -> Result<(), Box<dyn Error>> {
    let Some(evidence) = collect(registry, seeds)? else {
        println!(
            "[RETENTION ABSENT] No confirmed accepted recovery; no retention comparison manufactured."
        );
        return Ok(());
    };
    println!(
        "[RETENTION] Matched continuation from {} to {}; no new street jobs, routine wages and rivals continue. Only the returned member's supervisor changes.",
        stamp(evidence.start_minute),
        stamp(evidence.end_minute)
    );
    for arm in [
        &evidence.leave_under_recruiter,
        &evidence.restore_trusted_supervisor,
    ] {
        println!(
            "  [{}] retained {}, departures {}, loyalty warnings {}, unpaid wages {} cents",
            if arm.restored_reporting_line {
                "restore trusted lieutenant"
            } else {
                "leave under recruiter"
            },
            arm.retained,
            arm.departures,
            arm.loyalty_warnings,
            arm.wages_short_cents
        );
        for entry in &arm.reports {
            println!("    {} {}", stamp(entry.minute), entry.summary);
        }
    }
    println!(
        "[READ] Getting someone back is not the same as keeping them. Reporting lines use existing personal bonds; no relationship bonus, immunity, or cash was injected. Retention here is bounded, not a lifetime guarantee."
    );
    std::fs::create_dir_all(directory)?;
    std::fs::write(
        directory.join(format!(
            "retention-w{:016x}-p{:016x}.json",
            seeds.world, seeds.policy
        )),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reporting_line_changes_retention_after_rival_retry() {
        let registry = crimocracy::build_registry();
        let evidence = collect(&registry, EvaluationSeeds::defaults())
            .unwrap()
            .unwrap();
        assert!(!evidence.leave_under_recruiter.retained);
        assert_eq!(evidence.leave_under_recruiter.departures, 1);
        assert!(evidence.restore_trusted_supervisor.retained);
        assert_eq!(evidence.restore_trusted_supervisor.departures, 0);
        assert_eq!(evidence.restore_trusted_supervisor.loyalty_warnings, 2);
        assert_eq!(evidence.leave_under_recruiter.loyalty_warnings, 0);
        assert_eq!(
            evidence.restore_trusted_supervisor.reports.len(),
            2,
            "one visible refusal per rival retry proves the retention came from the member's own choice"
        );
        assert_eq!(evidence.restore_trusted_supervisor.wages_short_cents, 0);
        assert_eq!(evidence.leave_under_recruiter.wages_short_cents, 0);
    }
}
