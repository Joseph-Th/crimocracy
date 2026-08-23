//! Controlled/calibration harness for deterministic strategy and integration evidence.
//!
//! RUSH, PRESS, and RECON use canonical production operations and player-visible information.
//! `[DEV AUDIT]` output may inspect hidden state after decisions for diagnostics, but hidden state
//! never feeds action selection. `[NARRATION]` lines are the harness's documentary voice: they
//! explain world causality from player-visible facts and never feed action selection either.
//! Narrative sessions also run a player-earned defector watch after an accepted defection: the
//! organization watches every known rival through canonical surveillance and confirms where the
//! departed member resurfaces, instead of the departure report leaking the recruiting organization.
//! Timeline anchors are derived from the authored registry (operation duration, autonomous
//! recruitment cadence, and cold-case window) so session timing tracks the game instead of a
//! second hard-coded ruleset. Batch runs vary the simulation seed; each seed rotates the fixture
//! and bounded policy timing while matched branches stay on the same scenario timeline.

mod contracts;
mod model;
mod observe;
mod options;
mod probes;
mod readout;
mod scenario;
mod session;

pub use contracts::*;
pub use model::*;
pub use observe::*;
pub use options::*;
pub use probes::*;
pub use readout::*;
pub use scenario::*;
pub use session::*;

use crimocracy::build_registry;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_options(std::env::args().skip(1))? else {
        return Ok(());
    };

    match options.mode {
        HarnessMode::Smoke => run_smoke(options.seed, options.strategy),
        HarnessMode::Full => run_full(options),
    }
}

fn run_smoke(seed: u64, selected_strategy: Option<Strategy>) -> Result<(), Box<dyn Error>> {
    let registry = build_registry();
    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!("mode: smoke | seed {seed:#x}");
    let contract = match selected_strategy {
        Some(strategy) => format!(
            "contract: {} canonical strategy path (legal foundation skipped)",
            strategy.label()
        ),
        None => "contract: all canonical strategy paths plus legal foundation".to_owned(),
    };
    println!("{contract}");

    if selected_strategy.is_none() {
        run_legal_foundation_check(&registry)?;
    } else {
        println!("legal foundation: skipped for focused strategy iteration");
    }
    for strategy in [Strategy::Rush, Strategy::Press, Strategy::Recon] {
        if selected_strategy.is_some_and(|selected| selected != strategy) {
            continue;
        }
        let metrics = play_session(
            &registry,
            strategy,
            ScenarioProfile::NightTrap,
            seed,
            false,
            // Observe the whole first campaign day, not just the operation: smoke evidence must
            // include the rival's autonomous recruitment pass and its consequences instead of
            // structurally zeroed personnel counters.
            true,
        )?;
        validate_run_metrics(&metrics, false)?;
        validate_strategy_evidence(ScenarioProfile::NightTrap, &metrics)?;
        println!(
            "[SMOKE] {:<5} terminal {:>4}m | {} | police {} | evidence {} | intel legal {} / police {} / burglary {} | counter-intel {} | follow-up case {} | cold case {} | recruitment {} attempts / {} departures",
            strategy.label(),
            metrics.burglary_terminal_minute.unwrap_or_default(),
            terminal_label(&metrics),
            if metrics.police_arrived { "arrived" } else { "none" },
            metrics.evidence_count,
            metrics.player_legal_activity_information,
            metrics.player_police_activity_information,
            optional_scalar(metrics.burglary_information_quality),
            objective_label(metrics.counterintelligence_outcome).unwrap_or("-"),
            tri_state(metrics.followup_case_active),
            tri_state(metrics.cold_case_confirmed),
            metrics.autonomous_recruitment_attempts,
            metrics.player_personnel_departures,
        );
    }
    match selected_strategy {
        Some(strategy) => println!(
            "[SMOKE PASS] {} canonical harness contract passed",
            strategy.label()
        ),
        None => println!("[SMOKE PASS] all canonical harness contracts passed"),
    }
    Ok(())
}

fn run_full(options: HarnessOptions) -> Result<(), Box<dyn Error>> {
    let wall_start = Instant::now();
    let HarnessOptions {
        mode,
        samples,
        seed,
        strategy,
        artifact_dir,
    } = options;
    debug_assert_eq!(mode, HarnessMode::Full);
    debug_assert!(strategy.is_none());
    let registry = build_registry();
    let artifact_dir = artifact_dir.unwrap_or_else(|| PathBuf::from("target/harness"));

    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!("===========================\n");
    println!("Mode: controlled/calibration strategy comparison with bounded scenario sensitivity.");
    println!(
        "Evidence boundary: synthetic setup through production paths; policy inputs are player-visible, while [DEV AUDIT] is diagnostic only.\n"
    );
    println!(
        "Observation windows: narrative sessions run for two simulated days; matched batches run for one day to keep sensitivity evidence bounded.\n"
    );
    println!("Narrative comparison uses seed {seed:#x}.\n");

    println!("--- CONTROLLED SESSION: RUSH ---");
    let rush = play_session(
        &registry,
        Strategy::Rush,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;
    println!("\n--- CONTROLLED SESSION: PRESS ---");
    let press = play_session(
        &registry,
        Strategy::Press,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;
    println!("\n--- CONTROLLED SESSION: RECON ---");
    let recon = play_session(
        &registry,
        Strategy::Recon,
        ScenarioProfile::NightTrap,
        seed,
        true,
        true,
    )?;

    println!("\n--- SAME-SCENARIO READOUT ---");
    validate_run_metrics(&rush, true)?;
    validate_run_metrics(&press, true)?;
    validate_run_metrics(&recon, true)?;
    validate_night_trap_evidence(&rush)?;
    validate_night_trap_evidence(&press)?;
    validate_night_trap_evidence(&recon)?;
    validate_press_consequence_arc(&press)?;
    validate_press_expansion_evidence(&press)?;
    validate_defector_trail_evidence(&rush)?;
    validate_defector_trail_evidence(&press)?;
    validate_defector_trail_evidence(&recon)?;
    validate_win_back_evidence(&rush)?;
    validate_win_back_evidence(&press)?;
    validate_win_back_evidence(&recon)?;
    validate_second_act_evidence(&rush)?;
    validate_second_act_evidence(&press)?;
    validate_second_act_evidence(&recon)?;
    print_metrics(&rush);
    print_metrics(&press);
    print_metrics(&recon);
    print_experience_readout(&rush, &press, &recon);
    validate_branch_financial_isolation(&rush, &press, &recon)?;
    println!(
        "[HARNESS CHECK] Legitimate cashflow stayed identical across branches; delegated enterprise cashflow diverged only by district-scoped effects: PRESS paid the Canal District heat surcharge while hot, and its post-window Harbor District expansion earned surcharge-free income outside Central Precinct's jurisdiction."
    );

    println!("\n--- OPPORTUNITY PORTFOLIO PROBE ---");
    run_opportunity_portfolio_probe(&registry, seed)?;

    println!("\n--- ORGANIZATIONAL CAPACITY PROBE ---");
    run_organizational_capacity_probe(&registry, seed)?;

    println!("\n--- REPEAT-TAKE PROBE ---");
    run_repeat_take_probe(&registry, seed)?;

    println!("\n--- LEGAL FOUNDATION CHECK ---");
    run_legal_foundation_check(&registry)?;

    println!("\n--- NIGHT-TRAP BATCH ({samples} seeds per strategy) ---");
    println!("[BATCH] Running matched seeds for NIGHT TRAP...");
    let (rush_aggregate, press_aggregate, recon_aggregate) = run_strategy_batch(
        &registry,
        ScenarioProfile::NightTrap,
        samples,
        seed,
        Some(&artifact_dir),
    )?;
    println!("[BATCH PASS] NIGHT TRAP matched-seed checks passed.");
    rush_aggregate.print("RUSH");
    press_aggregate.print("PRESS");
    recon_aggregate.print("RECON");
    println!(
        "Decisions surfaced: rush {}, press {}, recon {}. Police arrivals: rush {}, press {}, recon {}.",
        rush_aggregate.decisions,
        press_aggregate.decisions,
        recon_aggregate.decisions,
        rush_aggregate.police_arrived,
        press_aggregate.police_arrived,
        recon_aggregate.police_arrived,
    );

    println!("\n--- SCENARIO SENSITIVITY ({samples} seeds per strategy/profile) ---");
    for profile in ScenarioProfile::SENSITIVITY_SET {
        println!("[BATCH] Running matched seeds for {}...", profile.label());
        let (rush, press, recon) =
            run_strategy_batch(&registry, profile, samples, seed, Some(&artifact_dir))?;
        println!("\n[{}]", profile.label());
        rush.print("RUSH");
        press.print("PRESS");
        recon.print("RECON");
        print_convergence_observation(profile, &rush, &press, &recon);
        println!(
            "[BATCH PASS] {} matched-seed checks passed.",
            profile.label()
        );
    }

    // Persist per-run seeds and raw metrics beneath aggregate diagnostics.
    // Full mode always writes artifacts; the directory defaults to target/harness.
    println!("\n--- ARTIFACTS ---");
    let narrative_runs = [(&rush, seed), (&press, seed), (&recon, seed)];
    for (metrics, run_seed) in narrative_runs {
        if let Ok(path) =
            persist_run_artifact(&artifact_dir, run_seed, ScenarioProfile::NightTrap, metrics)
        {
            println!("[ARTIFACT] wrote {}", path.display());
        }
    }
    // Also capture the batch aggregate summary.
    {
        fs::create_dir_all(&artifact_dir)?;
        let summary = serde_json::json!({
            "mode": "full",
            "seed": format!("{seed:#x}"),
            "samples": samples,
            "elapsed_secs": wall_start.elapsed().as_secs_f64(),
            "note": "per-run JSON files retain per-run seeds and raw metrics beneath derived findings"
        });
        let path = artifact_dir.join(format!("summary-{seed:#x}.json"));
        fs::write(&path, serde_json::to_string_pretty(&summary)?)?;
        println!("[ARTIFACT] wrote {}", path.display());
    }
    println!(
        "\n[HARNESS DONE] full suite in {:.1}s  artifacts: {}",
        wall_start.elapsed().as_secs_f64(),
        artifact_dir.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        choose_safe_start_from_patrol_report, parse_options, parse_patrol_windows,
        run_opportunity_portfolio_probe, run_smoke, validate_branch_financial_isolation,
        FixtureVariation, HarnessCliError, HarnessContractError, HarnessMode, HarnessOptions,
        RunMetrics, ScenarioProfile, ScenarioTimeline, Strategy, DEFAULT_SEED,
    };
    use crimocracy::core::time::{SimDuration, SimTime};

    #[test]
    fn parses_explicit_smoke_mode_and_hex_seed() {
        let options = parse_options(
            ["--mode", "smoke", "--samples", "1", "--seed", "0x2a"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("valid harness arguments should parse")
        .expect("non-help arguments should request a run");

        assert_eq!(
            options,
            HarnessOptions {
                mode: HarnessMode::Smoke,
                samples: 1,
                seed: 42,
                strategy: None,
                artifact_dir: None,
            }
        );
    }

    #[test]
    fn accepts_uppercase_hex_prefix() {
        let options = parse_options(["--seed", "0X2A"].into_iter().map(str::to_owned))
            .expect("uppercase hexadecimal prefixes should parse")
            .expect("non-help arguments should request a run");

        assert_eq!(options.seed, 42);
    }

    #[test]
    fn parses_a_focused_smoke_strategy() {
        let options = parse_options(
            ["--mode", "smoke", "--strategy", "press"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("focused smoke arguments should parse")
        .expect("non-help arguments should request a run");

        assert_eq!(options.strategy, Some(Strategy::Press));
    }

    #[test]
    fn rejects_strategy_selection_in_full_mode() {
        let error = parse_options(
            ["--mode", "full", "--strategy", "press"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("full mode must keep all strategy branches matched");

        assert!(matches!(error, HarnessCliError::StrategyOnlyInSmoke));
    }

    #[test]
    fn uses_fast_smoke_mode_defaults() {
        let options = parse_options(std::iter::empty())
            .expect("default arguments should parse")
            .expect("default arguments should request a run");

        assert_eq!(options.mode, HarnessMode::Smoke);
        assert_eq!(options.samples, 1);
        assert_eq!(options.seed, DEFAULT_SEED);
    }

    #[test]
    fn keeps_full_mode_bounded_by_default() {
        let options = parse_options(["--mode", "full"].into_iter().map(str::to_owned))
            .expect("explicit full mode should parse")
            .expect("non-help arguments should request a run");

        assert_eq!(options.mode, HarnessMode::Full);
        assert_eq!(options.samples, super::DEFAULT_BATCH_SAMPLES);
        assert_eq!(options.samples, super::MIN_SAMPLES_FOR_VARIATION_CONTRACT);
    }

    #[test]
    fn rejects_out_of_range_sample_count() {
        let error = parse_options(["--samples", "0"].into_iter().map(str::to_owned))
            .expect_err("zero samples must be rejected");

        assert!(matches!(
            error,
            HarnessCliError::SampleCountOutOfRange { value: 0 }
        ));
    }

    #[test]
    fn rejects_multi_sample_smoke_mode() {
        let error = parse_options(
            ["--mode", "smoke", "--samples", "2"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("smoke mode must not silently ignore a larger batch request");

        assert!(matches!(
            error,
            HarnessCliError::SmokeSampleCount { value: 2 }
        ));
    }

    #[test]
    fn rejects_unknown_strategy() {
        let error = parse_options(
            ["--mode", "smoke", "--strategy", "reckon"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect_err("unknown strategies must fail clearly");

        assert!(matches!(
            error,
            HarnessCliError::InvalidStrategy { value } if value == "reckon"
        ));
    }

    #[test]
    fn parses_wrapped_patrol_windows_without_empty_intervals() {
        assert_eq!(
            parse_patrol_windows(
                "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy)."
            ),
            vec![(120, 240), (1_320, 1_440)]
        );
    }

    #[test]
    fn chooses_a_buffered_window_from_player_visible_patrol_text() {
        let chosen = choose_safe_start_from_patrol_report(
            SimTime::from_minutes(1),
            "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy).",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(720),
        )
        .expect("actionable patrol text should produce a safe candidate");

        assert_eq!(chosen, SimTime::from_minutes(300));
    }

    #[test]
    fn rejects_patrol_text_without_actionable_windows() {
        let error = choose_safe_start_from_patrol_report(
            SimTime::ZERO,
            "Patrol activity was observed, but no recurring window was established.",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(720),
        )
        .expect_err("the harness must not infer a safe time from vague surveillance");

        assert!(matches!(
            error,
            HarnessContractError::NoActionablePatrolWindows
        ));
    }

    #[test]
    fn refuses_a_patrol_safe_start_after_opportunity_expiry() {
        let error = choose_safe_start_from_patrol_report(
            SimTime::from_minutes(1),
            "roughly 02:00-04:00 (concentrated); roughly 22:00-00:00 (heavy).",
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(200),
        )
        .expect_err("planning must respect the player-visible opportunity deadline");

        assert!(matches!(error, HarnessContractError::NoSafeOperationWindow));
    }

    #[test]
    fn portfolio_probe_requires_explicit_opportunity_prioritization() {
        run_opportunity_portfolio_probe(&crimocracy::build_registry(), DEFAULT_SEED)
            .expect("portfolio probe should preserve selected and expired opportunities");
    }

    #[test]
    fn seed_selects_distinct_authored_fixture_variations() {
        let clockwork = FixtureVariation::from_seed(0);
        let crowded = FixtureVariation::from_seed(1);
        let quiet = FixtureVariation::from_seed(2);

        assert_ne!(clockwork, crowded);
        assert_ne!(crowded, quiet);
        assert_ne!(clockwork, quiet);
        assert_ne!(
            clockwork.patrol_windows(ScenarioProfile::NightTrap),
            crowded.patrol_windows(ScenarioProfile::NightTrap),
        );
        assert_ne!(clockwork.target_name(), crowded.target_name());
        assert_ne!(crowded.source_specificity(), quiet.source_specificity());
        assert_ne!(
            clockwork.neighborhood_economy(),
            quiet.neighborhood_economy(),
        );
    }

    #[test]
    fn scenario_timeline_is_seed_varied_and_registry_anchored() {
        let registry = crimocracy::build_registry();
        let first = ScenarioTimeline::for_scenario(&registry, 0);
        let matched = ScenarioTimeline::for_scenario(&registry, 0);
        let varied = ScenarioTimeline::for_scenario(&registry, 3);
        let burglary_duration = registry
            .get_operation(crimocracy::operations::OperationKind::Burglary)
            .execution()
            .duration();

        assert_eq!(first, matched, "matched branches must share one timeline");
        assert_ne!(
            first, varied,
            "batch policy timing must not replay one exact clock sequence"
        );
        assert!(first.initial_burglary_at < first.initial_opportunity_valid_until);
        assert!(
            first.rush_second_act_at + burglary_duration < first.second_opportunity_valid_until,
            "the authored operation must fit inside the derived second-score window"
        );
        assert!(
            first.recon_second_act_surveillance_at > first.second_opportunity_discovery_at,
            "fresh recon must follow the player-visible opportunity discovery"
        );
    }

    #[test]
    fn matched_window_financials_pass_healthy_branches() {
        let rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, true, Some((19_775, 35_120)));
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 45_120)));

        validate_branch_financial_isolation(&rush, &press, &recon)
            .expect("isolated legitimate income and heat-only enterprise divergence should pass");
    }

    #[test]
    fn rejects_legitimate_income_drift_between_branches() {
        let rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, true, Some((71_979, 35_120)));
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 45_120)));

        let error = validate_branch_financial_isolation(&rush, &press, &recon)
            .expect_err("legitimate income must stay isolated from legal state");
        assert!(matches!(
            error,
            HarnessContractError::FinancialBranchMismatch { .. }
        ));
    }

    #[test]
    fn rejects_unheated_enterprise_divergence() {
        let rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, true, Some((19_775, 35_120)));
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 46_120)));

        let error = validate_branch_financial_isolation(&rush, &press, &recon)
            .expect_err("unheated branches must share identical enterprise economics");
        assert!(matches!(
            error,
            HarnessContractError::FinancialBranchMismatch { .. }
        ));
    }

    #[test]
    fn rejects_heated_branch_out_earning_unheated_branches() {
        let rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, true, Some((19_775, 124_923)));
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 45_120)));

        let error = validate_branch_financial_isolation(&rush, &press, &recon)
            .expect_err("an investigation-active branch pays the heat surcharge and cannot out-earn an unheated one");
        assert!(matches!(
            error,
            HarnessContractError::FinancialBranchMismatch { .. }
        ));
    }

    #[test]
    fn requires_matched_window_snapshots_from_every_branch() {
        let rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, true, None);
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 45_120)));

        let error = validate_branch_financial_isolation(&rush, &press, &recon)
            .expect_err("every branch must snapshot its matched financial window");
        assert!(matches!(
            error,
            HarnessContractError::MissingMatchedFinancialSnapshot {
                strategy: Strategy::Press
            }
        ));
    }

    fn branch_metrics(
        strategy: Strategy,
        investigation_created: bool,
        matched: Option<(i64, i64)>,
    ) -> RunMetrics {
        let mut metrics = RunMetrics {
            strategy: Some(strategy),
            investigation_created,
            // A burglary-originated case is by definition a staffed session case, so the
            // heating signal and the burglary resolution record move together in fixtures.
            session_case_staffed: investigation_created,
            matched_financial_boundary_minute: Some(2_880),
            ..RunMetrics::default()
        };
        if let Some((legitimate, enterprise)) = matched {
            metrics.matched_legitimate_net_cents = Some(legitimate);
            metrics.matched_enterprise_net_cents = Some(enterprise);
        }
        metrics
    }

    #[test]
    fn surveillance_originated_case_heats_only_its_own_branch() {
        let mut rush = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        let press = branch_metrics(Strategy::Press, false, Some((19_775, 45_120)));
        let recon = branch_metrics(Strategy::Recon, false, Some((19_775, 45_120)));
        // A casing case staffed in one branch must carry the same heat guarantee as a
        // burglary case: it may never out-earn an unheated branch over the shared window.
        rush.session_case_staffed = true;
        rush.matched_enterprise_net_cents = Some(46_120);
        assert!(
            validate_branch_financial_isolation(&rush, &press, &recon).is_err(),
            "a cased branch must not out-earn unheated branches on the session-wide signal"
        );
        // A case opened after the boundary flags heating without changing that window's net;
        // equal-to-unheated nets stay within the contract's heat-only-lowers guarantee.
        let mut post_boundary = branch_metrics(Strategy::Rush, false, Some((19_775, 45_120)));
        post_boundary.session_case_staffed = true;
        validate_branch_financial_isolation(&post_boundary, &press, &recon)
            .expect("post-boundary case staffing must not violate the matched-window contract");
    }

    #[test]
    #[ignore = "controlled smoke contract runs in its focused local gate lane"]
    fn smoke_mode_covers_canonical_paths() {
        run_smoke(DEFAULT_SEED, None)
            .expect("smoke harness should pass its canonical-path contract");
    }
}
