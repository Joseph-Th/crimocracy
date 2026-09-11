//! Controlled/calibration harness for deterministic strategy and integration evidence.
//!
//! RUSH, PRESS, and RECON use canonical production operations and player-visible information.
//! The default narrative stays on that player-visible boundary; hidden structural evidence is
//! retained in contracts/artifacts instead of being interleaved with the playable story.
//! `[NARRATION]` lines are the harness's documentary voice: they
//! explain world causality from player-visible facts and never feed action selection either.
//! Narrative sessions also run a player-earned defector watch after an accepted defection: the
//! organization watches every known rival through canonical surveillance and confirms where the
//! departed member resurfaces, instead of the departure report leaking the recruiting organization.
//! Timeline anchors are derived from the authored registry (operation duration, autonomous
//! recruitment cadence, and cold-case window) so session timing tracks the game instead of a
//! second hard-coded ruleset. World/simulation and evaluation-policy seeds are independent; scenario
//! sensitivity varies the world while matched branches keep one fixed policy treatment.

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
        HarnessMode::Smoke => run_smoke(
            EvaluationSeeds::new(options.world_seed, options.policy_seed),
            options.strategy,
        ),
        HarnessMode::Full => run_full(options),
    }
}

fn run_smoke(
    seeds: EvaluationSeeds,
    selected_strategy: Option<Strategy>,
) -> Result<(), Box<dyn Error>> {
    let registry = build_registry();
    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!(
        "mode: smoke | world seed {:#x} | policy seed {:#x}",
        seeds.world, seeds.policy
    );
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
    let mut smoke_metrics: Vec<RunMetrics> = Vec::new();
    for strategy in [Strategy::Rush, Strategy::Press, Strategy::Recon] {
        if selected_strategy.is_some_and(|selected| selected != strategy) {
            continue;
        }
        let metrics = play_session(
            &registry,
            strategy,
            ScenarioProfile::NightTrap,
            seeds,
            SessionRunMode::Batch,
        )?;
        validate_run_metrics(&metrics, false)?;
        validate_strategy_evidence(ScenarioProfile::NightTrap, &metrics)?;
        println!(
            "[SMOKE] {:<5} terminal {:>4}m | {} | police {} | evidence {} | intel legal {} / police {} / burglary {} | counter-intel {} | follow-up case {} | cold case {} | recruitment {} attempts / {} departures",
            strategy.label(),
            metrics.burglary_terminal_minute.unwrap_or_default(),
            terminal_label(&metrics),
            if metrics.police_arrived {
                "arrived"
            } else {
                "none"
            },
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
        smoke_metrics.push(metrics);
    }
    if selected_strategy.is_none() && smoke_metrics.len() == 3 {
        let rush = &smoke_metrics[0];
        let press = &smoke_metrics[1];
        let recon = &smoke_metrics[2];
        let info_leverage = recon.planning_information_count > rush.planning_information_count
            && recon.burglary_information_quality.unwrap_or_default()
                > rush.burglary_information_quality.unwrap_or_default();
        let consequence =
            press.investigation_created || recon.property_realized_cash_cents.is_some();
        println!(
            "\n[SMOKE LOOP] Observe→Plan→Delegate→Consequence: info leverage {} | consequence {} | personnel departures {}/{}/{}.",
            if info_leverage {
                "PASS (RECON 3 intel > RUSH 1)"
            } else {
                "fail"
            },
            if consequence { "PASS" } else { "fail" },
            rush.player_personnel_departures,
            press.player_personnel_departures,
            recon.player_personnel_departures,
        );
        println!(
            "[SMOKE READOUT] Run `cargo harness-full --samples 2` for the full player narrative, financial view, and structured diagnostic artifacts."
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
        world_seed,
        policy_seed,
        strategy,
        artifact_dir,
    } = options;
    debug_assert_eq!(mode, HarnessMode::Full);
    debug_assert!(strategy.is_none());
    let registry = build_registry();
    // Distinct from the [profile.harness] build directory `target/harness/`, so
    // `cargo clean` can never delete persisted run evidence.
    let artifact_dir = artifact_dir.unwrap_or_else(|| PathBuf::from("target/harness-runs"));

    println!("CRIMOCRACY GAMEPLAY HARNESS");
    println!("===========================\n");
    println!("Mode: controlled/calibration strategy comparison with bounded scenario sensitivity.");
    println!(
        "Evidence boundary: synthetic setup through production paths; narrated policy inputs and consequences are player-visible. Hidden structural evidence stays in contracts/artifacts.\n"
    );
    println!(
        "Observation windows: full sessions capture the shared financial comparison at two simulated days; consequence arcs may continue beyond that boundary when player policy keeps waiting. Matched batches run for one day to keep sensitivity evidence bounded.\n"
    );
    println!(
        "Narrative comparisons rotate across {NARRATIVE_SEED_ROTATION} adjacent world seeds so every authored fixture variation gets exercised while policy seed {policy_seed:#x} stays fixed; matched branches inside one world share one treatment.\n"
    );

    let mut narrative_sets: Vec<(EvaluationSeeds, RunMetrics, RunMetrics, RunMetrics)> =
        Vec::with_capacity(NARRATIVE_SEED_ROTATION as usize);
    for offset in 0..NARRATIVE_SEED_ROTATION {
        let narrative_seeds = EvaluationSeeds::new(world_seed.wrapping_add(offset), policy_seed);
        let deep_readout = offset == 0;
        println!(
            "\n=== NARRATIVE COMPARISON SET {} of {NARRATIVE_SEED_ROTATION}: world {:#x}, policy {:#x} ===",
            offset + 1,
            narrative_seeds.world,
            narrative_seeds.policy,
        );
        if deep_readout {
            println!("\n--- CONTROLLED SESSION: RUSH ---");
        }
        let mut rush = play_session(
            &registry,
            Strategy::Rush,
            ScenarioProfile::NightTrap,
            narrative_seeds,
            if deep_readout {
                SessionRunMode::FullNarrative
            } else {
                SessionRunMode::FullQuiet
            },
        )?;
        if deep_readout {
            println!("\n--- CONTROLLED SESSION: PRESS (same fixture and world) ---");
        }
        let mut press = play_session_with_fixture_view(
            &registry,
            Strategy::Press,
            ScenarioProfile::NightTrap,
            narrative_seeds,
            if deep_readout {
                SessionRunMode::FullNarrative
            } else {
                SessionRunMode::FullQuiet
            },
            false,
        )?;
        if deep_readout {
            println!("\n--- CONTROLLED SESSION: RECON (same fixture and world) ---");
        }
        let mut recon = play_session_with_fixture_view(
            &registry,
            Strategy::Recon,
            ScenarioProfile::NightTrap,
            narrative_seeds,
            if deep_readout {
                SessionRunMode::FullNarrative
            } else {
                SessionRunMode::FullQuiet
            },
            false,
        )?;
        if deep_readout {
            rush.primary_narrative_set = true;
            press.primary_narrative_set = true;
            recon.primary_narrative_set = true;
        }

        println!(
            "\n--- {} (world seed {:#x}, policy seed {:#x}) ---",
            if deep_readout {
                "SAME-SCENARIO DIAGNOSTIC METRICS"
            } else {
                "ROTATED-VARIATION SUMMARY"
            },
            narrative_seeds.world,
            narrative_seeds.policy,
        );
        validate_run_metrics(&rush, true)?;
        validate_run_metrics(&press, true)?;
        validate_run_metrics(&recon, true)?;
        validate_night_trap_evidence(&rush)?;
        validate_night_trap_evidence(&press)?;
        validate_night_trap_evidence(&recon)?;
        validate_press_consequence_arc(&press)?;
        validate_press_witness_counterplay(&press)?;
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
        validate_branch_financial_isolation(&rush, &press, &recon)?;
        println!(
            "[HARNESS CHECK] Legitimate cashflow stayed identical across branches; delegated enterprise cashflow diverged only by district-scoped effects."
        );
        if deep_readout {
            print_metrics(&rush);
            print_metrics(&press);
            print_metrics(&recon);
        } else {
            for metrics in [&rush, &press, &recon] {
                println!(
                    "[SET SUMMARY] {:<5}: {}, police arrival {}, case staffed {}, departures {}, win-back {:?}, act-2 burglary {}",
                    metrics.strategy.expect("strategy must be set").label(),
                    terminal_label(metrics),
                    metrics.police_arrived,
                    metrics.session_case_staffed,
                    metrics.player_personnel_departures,
                    metrics.win_back_accepted,
                    metrics.second_burglary.is_some(),
                );
            }
        }
        narrative_sets.push((narrative_seeds, rush, press, recon));
    }

    let (primary_seeds, rush, press, recon) = narrative_sets
        .first()
        .expect("at least one narrative set must run");
    let primary_seeds = *primary_seeds;
    let rush = rush.clone();
    let press = press.clone();
    let recon = recon.clone();

    println!("\n--- VICE HEAT PROBE ---");
    // Reaching the readout proves the vice-attention chain: the probe fails the run otherwise.
    run_vice_attention_probe(&registry, primary_seeds)?;
    print_experience_readout(&rush, &press, &recon, true);

    println!("\n--- OPPORTUNITY PORTFOLIO PROBE ---");
    run_opportunity_portfolio_probe(&registry, primary_seeds)?;

    println!("\n--- ORGANIZATIONAL CAPACITY PROBE ---");
    run_organizational_capacity_probe(&registry, primary_seeds)?;

    println!("\n--- REPEAT-TAKE PROBE ---");
    run_repeat_take_probe(&registry, primary_seeds)?;

    println!("\n--- LEGAL FOUNDATION CHECK ---");
    run_legal_foundation_check(&registry)?;

    println!("\n--- NIGHT-TRAP BATCH ({samples} world seeds per strategy) ---");
    println!("[BATCH] Running matched world seeds with policy {policy_seed:#x} for NIGHT TRAP...");
    let (rush_aggregate, press_aggregate, recon_aggregate) = run_strategy_batch(
        &registry,
        ScenarioProfile::NightTrap,
        samples,
        EvaluationSeeds::new(world_seed, policy_seed),
        Some(&artifact_dir),
    )?;
    println!("[BATCH PASS] NIGHT TRAP matched-world checks passed.");
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

    println!(
        "\n--- SCENARIO SENSITIVITY ({samples} world seeds per strategy/profile, fixed policy {policy_seed:#x}) ---"
    );
    for profile in ScenarioProfile::SENSITIVITY_SET {
        println!(
            "[BATCH] Running matched world seeds for {}...",
            profile.label()
        );
        let (rush, press, recon) = run_strategy_batch(
            &registry,
            profile,
            samples,
            EvaluationSeeds::new(world_seed, policy_seed),
            Some(&artifact_dir),
        )?;
        println!("\n[{}]", profile.label());
        rush.print("RUSH");
        press.print("PRESS");
        recon.print("RECON");
        print_convergence_observation(profile, &rush, &press, &recon);
        println!(
            "[BATCH PASS] {} matched-world checks passed.",
            profile.label()
        );
    }

    // Persist per-run seeds and raw metrics beneath aggregate diagnostics.
    // Full mode always writes artifacts; the directory defaults to target/harness-runs.
    println!("\n--- ARTIFACTS ---");
    let narrative_runs = [&rush, &press, &recon];
    for metrics in narrative_runs {
        if let Ok(path) = persist_run_artifact(
            &artifact_dir,
            primary_seeds,
            ScenarioProfile::NightTrap,
            metrics,
        ) {
            println!("[ARTIFACT] wrote {}", path.display());
        }
    }
    // Also capture the batch aggregate summary.
    {
        fs::create_dir_all(&artifact_dir)?;
        let summary = serde_json::json!({
            "mode": "full",
            "world_seed": format!("{world_seed:#x}"),
            "world_seed_dec": world_seed,
            "policy_seed": format!("{policy_seed:#x}"),
            "policy_seed_dec": policy_seed,
            "samples": samples,
            "elapsed_secs": wall_start.elapsed().as_secs_f64(),
            "note": "scenario-sensitivity samples vary world/simulation seed while the evaluation-policy seed remains fixed; per-run JSON retains both"
        });
        let path = artifact_dir.join(format!(
            "summary-w{world_seed:016x}-p{policy_seed:016x}.json"
        ));
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
        DEFAULT_POLICY_SEED, DEFAULT_WORLD_SEED, EvaluationSeeds, FixtureVariation,
        HarnessCliError, HarnessContractError, HarnessMode, HarnessOptions,
        NARRATIVE_SEED_ROTATION, RunMetrics, ScenarioProfile, ScenarioTimeline, SessionRunMode,
        Strategy, bounded_policy_choice, choose_safe_start_from_patrol_signal, parse_options,
        patrol_intervals_from_signal, play_session, run_opportunity_portfolio_probe, run_smoke,
        run_vice_attention_probe, validate_batch_strategy_coverage,
        validate_branch_financial_isolation, validate_press_witness_counterplay,
        validate_second_act_evidence,
    };
    use crimocracy::core::time::{SimDuration, SimTime};
    use crimocracy::intelligence::{CaseActivitySignal, InformationSignal, PatrolIntervalSignal};
    use crimocracy::operations::OperationObjectiveOutcome;

    fn patrol_signal(intervals: &[(u16, u16)]) -> InformationSignal {
        InformationSignal::PatrolPattern {
            intervals: intervals
                .iter()
                .map(|(start, end)| {
                    PatrolIntervalSignal::try_new(*start, *end)
                        .expect("harness patrol fixture interval must validate")
                })
                .collect(),
        }
    }

    #[test]
    fn full_quiet_mode_changes_only_presentation() {
        let registry = crimocracy::build_registry();
        for strategy in [Strategy::Rush, Strategy::Press, Strategy::Recon] {
            let narrative = play_session(
                &registry,
                strategy,
                ScenarioProfile::NightTrap,
                EvaluationSeeds::defaults(),
                SessionRunMode::FullNarrative,
            )
            .expect("full narrative session should complete");
            let quiet = play_session(
                &registry,
                strategy,
                ScenarioProfile::NightTrap,
                EvaluationSeeds::defaults(),
                SessionRunMode::FullQuiet,
            )
            .expect("full quiet session should complete");
            assert_eq!(
                narrative, quiet,
                "presentation mode must not change {strategy:?} gameplay metrics"
            );
        }
    }

    #[test]
    fn parses_explicit_smoke_mode_and_independent_hex_seeds() {
        let options = parse_options(
            [
                "--mode",
                "smoke",
                "--samples",
                "1",
                "--world-seed",
                "0x2a",
                "--policy-seed",
                "0x2b",
            ]
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
                world_seed: 42,
                policy_seed: 43,
                strategy: None,
                artifact_dir: None,
            }
        );
    }

    #[test]
    fn accepts_uppercase_hex_prefix() {
        let options = parse_options(["--world-seed", "0X2A"].into_iter().map(str::to_owned))
            .expect("uppercase hexadecimal prefixes should parse")
            .expect("non-help arguments should request a run");

        assert_eq!(options.world_seed, 42);
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
        assert_eq!(options.world_seed, DEFAULT_WORLD_SEED);
        assert_eq!(options.policy_seed, DEFAULT_POLICY_SEED);
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
    fn tolerates_end_of_options_markers_from_cargo_aliases() {
        // `cargo harness-full -- --samples 8` appends after the alias's own `--`,
        // so the parser receives a second marker; it must stay inert.
        let options = parse_options(
            ["--", "--mode", "full", "--samples", "2", "--"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("end-of-options markers should not reject valid arguments")
        .expect("non-help arguments should request a run");

        assert_eq!(options.mode, HarnessMode::Full);
        assert_eq!(options.samples, 2);
    }

    #[test]
    fn reads_normalized_windows_from_typed_patrol_signal() {
        let signal = patrol_signal(&[(120, 240), (1_320, 1_440)]);
        assert_eq!(
            patrol_intervals_from_signal(&signal),
            vec![(120, 240), (1_320, 1_440)]
        );
    }

    #[test]
    fn chooses_a_buffered_window_from_player_visible_patrol_signal() {
        let signal = patrol_signal(&[(120, 240), (1_320, 1_440)]);
        let chosen = choose_safe_start_from_patrol_signal(
            SimTime::from_minutes(1),
            &signal,
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(720),
        )
        .expect("actionable patrol text should produce a safe candidate");

        assert_eq!(chosen, SimTime::from_minutes(300));
    }

    #[test]
    fn rejects_information_without_actionable_patrol_semantics() {
        let signal = InformationSignal::CaseActivity(CaseActivitySignal::Active);
        let error = choose_safe_start_from_patrol_signal(
            SimTime::ZERO,
            &signal,
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
        let signal = patrol_signal(&[(120, 240), (1_320, 1_440)]);
        let error = choose_safe_start_from_patrol_signal(
            SimTime::from_minutes(1),
            &signal,
            SimDuration::from_minutes(45),
            SimDuration::from_minutes(60),
            SimTime::from_minutes(200),
        )
        .expect_err("planning must respect the player-visible opportunity deadline");

        assert!(matches!(error, HarnessContractError::NoSafeOperationWindow));
    }

    #[test]
    fn portfolio_probe_requires_explicit_opportunity_prioritization() {
        run_opportunity_portfolio_probe(&crimocracy::build_registry(), EvaluationSeeds::defaults())
            .expect("portfolio probe should preserve selected and expired opportunities");
    }

    #[test]
    fn vice_heat_probe_proves_clean_districts_stay_clean_and_casework_converts() {
        run_vice_attention_probe(&crimocracy::build_registry(), EvaluationSeeds::defaults())
            .expect("vice-attention probe should prove the sustained-casework conversion chain");
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
    fn narrative_seed_rotation_covers_every_authored_variation() {
        let variations: std::collections::BTreeSet<_> = (0..NARRATIVE_SEED_ROTATION)
            .map(|offset| FixtureVariation::from_seed(DEFAULT_WORLD_SEED.wrapping_add(offset)))
            .collect();
        assert_eq!(
            variations.len() as u64,
            NARRATIVE_SEED_ROTATION,
            "the narrative rotation must exercise every authored fixture variation"
        );
    }

    #[test]
    fn policy_choice_avalanches_across_adjacent_policy_seeds_and_salts() {
        // Adjacent policy seeds must not replay one decision sequence: over a window of eight
        // values both binary outcomes must appear, and different salts must not agree.
        let outcomes: Vec<u64> = (0..8)
            .map(|offset| bounded_policy_choice(DEFAULT_POLICY_SEED + offset, 0x5EED, 2))
            .collect();
        assert!(outcomes.contains(&0) && outcomes.contains(&1));
        let other_salt: Vec<u64> = (0..8)
            .map(|offset| bounded_policy_choice(DEFAULT_POLICY_SEED + offset, 0x0DEF, 2))
            .collect();
        assert_ne!(outcomes, other_salt);
    }

    #[test]
    fn scenario_timeline_is_policy_seed_varied_and_registry_anchored() {
        let registry = crimocracy::build_registry();
        let first = ScenarioTimeline::for_policy(&registry, 0);
        let matched = ScenarioTimeline::for_policy(&registry, 0);
        let varied = ScenarioTimeline::for_policy(&registry, 3);
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
    fn world_seed_variation_does_not_change_fixed_policy_timeline() {
        let registry = crimocracy::build_registry();
        let first = EvaluationSeeds::new(0, 7);
        let second = EvaluationSeeds::new(1, 7);

        assert_ne!(
            FixtureVariation::from_seed(first.world),
            FixtureVariation::from_seed(second.world),
            "world sensitivity must actually perturb the authored fixture"
        );
        assert_eq!(
            ScenarioTimeline::for_policy(&registry, first.policy),
            ScenarioTimeline::for_policy(&registry, second.policy),
            "holding policy seed fixed must hold evaluation timing fixed while world varies"
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

    #[test]
    fn press_witness_counterplay_requires_named_witness_and_pressure() {
        let mut metrics = branch_metrics(Strategy::Press, true, None);
        let error = validate_press_witness_counterplay(&metrics)
            .expect_err("a witnessed case must name its witness and draw pressure");
        assert!(matches!(
            error,
            HarnessContractError::MissingStrategyEvidence { .. }
        ));

        metrics.case_witness_registered = true;
        metrics.witness_pressure_attempted = true;
        metrics.witness_pressure_outcome = Some(OperationObjectiveOutcome::Achieved);
        metrics.witness_cooperation_degraded = true;
        validate_press_witness_counterplay(&metrics)
            .expect("a landed pressure that degrades cooperation must pass");

        let mut aborted = branch_metrics(Strategy::Press, true, None);
        aborted.case_witness_registered = true;
        aborted.witness_pressure_attempted = true;
        aborted.witness_pressure_aborted = true;
        validate_press_witness_counterplay(&aborted)
            .expect("a disciplined police-arrival abort without degradation must pass");

        let mut botched = branch_metrics(Strategy::Press, true, None);
        botched.case_witness_registered = true;
        botched.witness_pressure_attempted = true;
        botched.witness_pressure_outcome = Some(OperationObjectiveOutcome::Failed);
        let error = validate_press_witness_counterplay(&botched)
            .expect_err("a failed pressure with no degradation is neither shape");
        assert!(matches!(
            error,
            HarnessContractError::MissingStrategyEvidence { .. }
        ));
    }

    #[test]
    fn witness_counterplay_contract_ignores_other_strategies_and_uncased_sessions() {
        let mut rush = branch_metrics(Strategy::Rush, false, None);
        rush.case_witness_registered = true;
        validate_press_witness_counterplay(&rush)
            .expect("non-press branches carry no counter-play contract");
        let uncased = branch_metrics(Strategy::Press, false, None);
        validate_press_witness_counterplay(&uncased)
            .expect("a session without a case has nothing to counter");
    }

    #[test]
    fn batch_coverage_allows_per_run_absence_but_requires_reachability_when_sampled_broadly() {
        let empty = super::Aggregate::default();
        validate_batch_strategy_coverage(ScenarioProfile::NightTrap, 1, &empty)
            .expect("one bounded sample cannot prove a stochastic event unreachable");

        let error = validate_batch_strategy_coverage(
            ScenarioProfile::NightTrap,
            super::MIN_SAMPLES_FOR_VARIATION_CONTRACT,
            &empty,
        )
        .expect_err("covered fixture variations should expose the authored night-trap abort path");
        assert!(matches!(
            error,
            HarnessContractError::MissingBatchEvidence {
                profile: ScenarioProfile::NightTrap,
                ..
            }
        ));

        let mut covered = super::Aggregate::default();
        covered.standing_contingency_aborts = 1;
        validate_batch_strategy_coverage(
            ScenarioProfile::NightTrap,
            super::MIN_SAMPLES_FOR_VARIATION_CONTRACT,
            &covered,
        )
        .expect("one observed abort proves aggregate reachability without forcing every seed");
    }

    fn persisted_operation_id(raw: u32) -> crimocracy::core::id::OperationId {
        serde_json::from_value(serde_json::Value::from(raw))
            .expect("persistent operation IDs deserialize from their raw integer representation")
    }

    #[test]
    fn recon_second_act_accepts_clean_recovery_or_uncleared_case_stand_down() {
        let mut clean = branch_metrics(Strategy::Recon, false, None);
        clean.second_opportunity_discovered = true;
        clean.second_act_recon_information = 2;
        clean.second_burglary = Some(persisted_operation_id(77));
        clean.second_burglary_outcome = Some(OperationObjectiveOutcome::Achieved);
        clean.second_burglary_terminal_minute = Some(2_095);
        validate_second_act_evidence(&clean)
            .expect("clean fresh recon may proceed into a successful second score");

        let mut hot = branch_metrics(Strategy::Recon, false, None);
        hot.second_opportunity_discovered = true;
        hot.second_opportunity_expired = true;
        hot.second_act_recon_information = 2;
        hot.self_heat_case_opened = true;
        hot.self_heat_case_active = Some(true);
        validate_second_act_evidence(&hot)
            .expect("a confirmed hot casing case must make cautious recon stand down");

        let mut inconclusive = branch_metrics(Strategy::Recon, false, None);
        inconclusive.second_opportunity_discovered = true;
        inconclusive.second_opportunity_expired = true;
        inconclusive.second_act_recon_information = 2;
        inconclusive.self_heat_case_opened = true;
        inconclusive.self_heat_case_active = None;
        validate_second_act_evidence(&inconclusive)
            .expect("an uncleared casing case must also make cautious recon stand down");
    }

    #[test]
    fn recon_second_act_rejects_compounding_a_confirmed_hot_case() {
        let mut metrics = branch_metrics(Strategy::Recon, false, None);
        metrics.second_opportunity_discovered = true;
        metrics.second_act_recon_information = 2;
        metrics.self_heat_case_opened = true;
        metrics.self_heat_case_active = Some(true);
        metrics.second_burglary = Some(persisted_operation_id(78));
        metrics.second_burglary_outcome = Some(OperationObjectiveOutcome::Achieved);
        metrics.second_burglary_terminal_minute = Some(2_095);

        let error = validate_second_act_evidence(&metrics).expect_err(
            "RECON must not work another burglary after confirming its casing case is hot",
        );
        assert!(matches!(
            error,
            HarnessContractError::MissingStrategyEvidence { .. }
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
        run_smoke(EvaluationSeeds::defaults(), None)
            .expect("smoke harness should pass its canonical-path contract");
    }
}
