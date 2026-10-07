//! PRESS strategy response arc built only from player-visible information and canonical game APIs.

use super::*;
use crimocracy::core::id::{BusinessId, EnterpriseId};
use crimocracy::core::time::DAY_MINUTES;
use crimocracy::delegation::{MandateAuthority, ResponsibilityScope};
use crimocracy::enterprises::enterprise_execution::validate_establish_enterprise;
use crimocracy::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crimocracy::finance::finance_system::validate_record_transaction;
use crimocracy::finance::{LedgerPosting, LedgerTransactionDraft};
use crimocracy::legal::ALL_INVESTIGATION_WORK_KINDS;
use std::collections::BTreeMap;

pub(super) fn run_press_response(
    scenario: &mut Scenario,
    burglary: OperationId,
    full_arc: bool,
    narrative: bool,
    campaign_day_minutes: u64,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    learn_initial_case_through_contact(scenario, burglary, narrative, metrics)?;
    let mut pending_witness_pressure = schedule_witness_pressure(scenario, narrative, metrics)?;

    // The Press branch exercises a real player follow-up: the organization uses only the
    // case-activity fact disclosed through its standing police contact and the crew's field report
    // to authorize counter-surveillance of the precinct itself. The investigation's evidence,
    // lead, and internal ID stay hidden; the follow-up reads only whether the authority is still
    // visibly developing the known case.
    if metrics.player_legal_activity_information > 0
        && metrics.player_police_activity_information > 0
    {
        let neighborhood_name = scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("counter-surveillance neighborhood must persist")
            .name()
            .to_owned();
        let police_name = scenario
            .state
            .world()
            .get_organization(scenario.police)
            .expect("police organization must persist")
            .name()
            .to_owned();
        let case_open_minute = metrics
            .case_open_minute
            .expect("press consequence arc requires the surfaced case-open minute");
        // Player tradecraft, not institutional math: look at the precinct itself the next
        // morning, about 90 minutes after the case opened. An originated street case takes
        // the authored cold-case window of inactivity to go cold, so a next-morning read
        // always precedes any possible shelf; the check never overlaps earlier scout work
        // because it follows the clock.
        let heat_check_delay = 90_u64;
        let heat_check_at = SimTime::from_minutes(case_open_minute + heat_check_delay)
            .max(scenario.state.now() + SimDuration::from_minutes(1));
        debug_assert!(
            heat_check_at.as_minutes()
                < case_open_minute
                    + u64::from(scenario.registry.legal().cold_case_window().as_minutes()),
            "the next-morning heat check must precede any possible cold-case shelf"
        );
        let heat_check_lag = heat_check_at.as_minutes().saturating_sub(case_open_minute);
        if narrative {
            println!(
                "[DECIDE]  A case is open and the crew's field report is back. Hold back on further street work in {neighborhood_name} until leadership knows whether {police_name} is still developing it."
            );
            println!(
                "[DECIDE]  Watch {police_name} itself at {}, {} minutes after the case opened, to read whether detectives are still actively working the matter.",
                format_day_minute(heat_check_at.as_minutes()),
                heat_check_lag
            );
        }
        metrics.counterintelligence_scheduled_at = Some(heat_check_at.as_minutes());
        let counterintelligence_title = format!("{police_name} case-heat check");
        let police = scenario.police;
        let counterintelligence = authorize_surveillance_target(
            scenario,
            EntityRef::Organization(police),
            &counterintelligence_title,
            heat_check_at,
            BTreeSet::new(),
        )?;
        run_until_operation_terminal(scenario, counterintelligence, narrative, metrics)?;
        // The quiet word was scheduled into the same morning gap and is already terminal by
        // the time the precinct watch closes; capture here so its narration stays
        // chronological with the rest of the night.
        if let Some(pressure) = pending_witness_pressure.take() {
            capture_witness_pressure_outcome(scenario, pressure, narrative, metrics)?;
        }
        let operation = scenario
            .state
            .operations()
            .get_operation(counterintelligence)
            .expect("counterintelligence operation must persist");
        if let Some(resolution) = operation.resolution() {
            metrics.counterintelligence_outcome = Some(resolution.objective_outcome());
            metrics.counterintelligence_information = resolution.discovered_information().len();
            metrics.followup_case_active = observe_authority_case_sightline(scenario, resolution);
        }
        if narrative {
            match metrics.followup_case_active {
                Some(true) => println!(
                    "[VERIFY]  Detectives around {police_name} are still actively developing the case. Stop new street jobs; the home racket remains open, earning income and risking further enforcement attention."
                ),
                Some(false) => println!(
                    "[VERIFY]  No active case machinery around {police_name}; the matter appears shelved."
                ),
                None => println!(
                    "[VERIFY]  The check did not produce a dependable read on the case's activity."
                ),
            }
        }
        if full_arc {
            run_stand_down_and_diversify(
                scenario,
                burglary,
                &police_name,
                campaign_day_minutes,
                narrative,
                metrics,
            )?;
        }
    }
    // The quiet word resolved during the same advance of the clock; capture its consequence
    // from production records whether or not the narrative polling loop above ran.
    if let Some(pressure) = pending_witness_pressure.take() {
        capture_witness_pressure_outcome(scenario, pressure, narrative, metrics)?;
    }

    Ok(())
}

fn learn_initial_case_through_contact(
    scenario: &mut Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    if !matches!(
        metrics.exposure_level,
        Some(
            crimocracy::operations::OperationExposureLevel::Witnessed
                | crimocracy::operations::OperationExposureLevel::Identifying
        )
    ) {
        return Ok(());
    }
    // The after-action only tells leadership what the crew observed. A visibly exposed job is
    // enough reason to ask an existing police contact whether the precinct has opened a case,
    // but the answer itself must come through the contact's canonical disclosure surface.
    let disclosed =
        read_police_contact(scenario, EntityRef::Operation(burglary), narrative, metrics)?;
    if disclosed.is_some() {
        refresh_player_case_information(scenario, burglary, metrics);
    }
    Ok(())
}

fn schedule_witness_pressure(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<OperationId>, Box<dyn Error>> {
    // The Press answer to a witnessed job is not only patience: leadership leans once on the
    // shop's owner - public knowledge who that is. Leadership does not send the follow-up
    // immediately after the failed score. If organization-held information contains a typed
    // patrol pattern, that pattern selects the timing; otherwise the harness uses a bounded
    // treatment delay without narrating it as knowledge. The follow-up carries its own exposure
    // risk like any street work.
    let mut pending_witness_pressure: Option<OperationId> = None;
    if metrics.player_legal_activity_information > 0
        && matches!(
            metrics.exposure_level,
            Some(
                crimocracy::operations::OperationExposureLevel::Witnessed
                    | crimocracy::operations::OperationExposureLevel::Identifying
            )
        )
    {
        let witness = scenario.target_owner;
        let witness_name = match scenario
            .state
            .world()
            .get_business(scenario.target)
            .expect("target business must persist")
            .owner()
        {
            crimocracy::world::BusinessOwner::Character(owner) => scenario
                .state
                .world()
                .get_character(owner)
                .map(|record| record.name().to_owned())
                .unwrap_or_else(|| "the owner".to_owned()),
            crimocracy::world::BusinessOwner::Independent
            | crimocracy::world::BusinessOwner::Organization(_) => "the owner".to_owned(),
        };
        let witness_source =
            find_pending_disclosure_sources(&scenario.state, scenario.police_contact)
                .into_iter()
                .find(|source| {
                    scenario
                        .state
                        .intelligence()
                        .get_information(*source)
                        .is_some_and(|information| {
                            information.topic() == InformationTopic::LegalActivity
                                && information.subject() == EntityRef::Character(witness)
                                && matches!(
                            information.signal(),
                            Some(InformationSignal::LegalPersonStatus(
                                crimocracy::intelligence::LegalPersonStatusSignal::CaseWitness {
                                    ..
                                }
                            ))
                        )
                        })
                });
        let Some(witness_source) = witness_source else {
            if narrative {
                println!(
                    "[VERIFY]  The contact has not confirmed that {witness_name} is actually on the case as a witness. Do not authorize pressure from ownership or street visibility alone."
                );
            }
            return Ok(None);
        };
        let disclosure =
            validate_contact_disclosure(&scenario.state, scenario.police_contact, witness_source)?
                .commit(&mut scenario.state)?;
        metrics.contact_reads += 1;
        let disclosed_information = scenario
            .state
            .contacts()
            .get_disclosure(disclosure)
            .expect("witness-status disclosure must persist")
            .disclosed_information();
        if narrative {
            let information = scenario
                .state
                .intelligence()
                .get_information(disclosed_information)
                .expect("disclosed witness-status information must persist");
            println!(
                "[LEARN]   {}: {}",
                format_information_grade(information.reliability(), information.specificity()),
                information.summary()
            );
        }
        if narrative {
            println!(
                "[DECIDE]  The after-action says the job was witnessed and the police contact confirms {witness_name} is on the case. Do not follow the failed score immediately; send Carlo with one quiet word only from that learned legal status, and accept that the follow-up carries its own exposure risk."
            );
            match metrics.exposure_level {
                Some(crimocracy::operations::OperationExposureLevel::Identifying) => println!(
                    "[STAKES]  The crew may have been identified: testimony plus the incident record can corroborate toward custody. The quiet word risks a second case, but leaving a cooperative witness unanswered risks the file growing teeth."
                ),
                _ => println!(
                    "[STAKES]  The job was witnessed but nobody was identified: testimony names the score, not a member, so no arrest follows from this file alone. The quiet word still risks opening a second case to deny the file future corroboration — a gamble, not a rescue."
                ),
            }
        }
        // The lull anchor is player-visible reasoning: the crew's own field report places the
        // heavy enforcement in the small hours, so leadership schedules the quiet word inside
        // the first morning gap after the case opened and still before an institutional
        // interview would typically be worked. When the crew's debrief produced a patrol
        // report, use the same patrol-aware window selection RECON uses; otherwise fall
        // back to a bounded evaluation-owned offset.
        let case_open_minute = metrics
            .case_open_minute
            .expect("witness-pressure arc requires the surfaced case-open minute");
        let debrief_patrol_pattern = metrics
            .debrief_police_activity_information
            .iter()
            .copied()
            .find(|information| {
                scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .is_some_and(|record| {
                        matches!(
                            record.signal(),
                            Some(InformationSignal::PatrolPattern { .. })
                        )
                    })
            });
        // Fallback: any organization-held police-activity observation that actually carries a
        // dependable recurring pattern. A debrief saying only "police were active at that hour"
        // is useful planning information, but it is not enough to manufacture a daily schedule.
        let police_activity_information = debrief_patrol_pattern.or_else(|| {
            scenario
                .state
                .intelligence()
                .information_for_holder_by_topic(
                    KnowledgeHolder::Organization(scenario.player),
                    InformationTopic::PoliceActivity,
                )
                .find(|information| {
                    matches!(
                        information.signal(),
                        Some(InformationSignal::PatrolPattern { .. })
                    )
                })
                .map(|information| information.id())
        });
        let pressure_at = if let Some(information) = police_activity_information {
            let patrol_signal = scenario
                .state
                .intelligence()
                .get_information(information)
                .and_then(|record| record.signal())
                .cloned()
                .expect("selected police-activity information must retain patrol semantics");
            let duration = scenario
                .registry
                .get_operation(OperationKind::WitnessPressure)
                .execution()
                .duration();
            // Institutional interviews land within the longest authored investigation-work
            // duration after intake. When the organization has a typed patrol pattern, that
            // evidence is binding: if no lower-risk window exists before the interview
            // horizon, fail the treatment instead of discarding known risk and inventing
            // a convenient fallback time.
            let latest_start =
                SimTime::from_minutes(case_open_minute + interview_horizon_minutes(scenario));
            choose_lower_risk_start_from_patrol_signal(
                SimTime::from_minutes(case_open_minute),
                &patrol_signal,
                duration,
                SimDuration::from_minutes(30),
                latest_start,
            )?
        } else {
            let delay = 50 + bounded_policy_choice(scenario.seeds.policy, 0xA11CE, 30);
            SimTime::from_minutes(case_open_minute + delay)
        };
        if narrative {
            if police_activity_information.is_some() {
                println!(
                    "[INTERPRET] Quiet-word timing chosen from crew's patrol report to land inside the morning lull at {}.",
                    format_day_minute(pressure_at.as_minutes())
                );
            } else {
                println!(
                    "[INTERPRET] No patrol pattern is held, so the quiet-word time at {} is a blind guess inside a watched district, not an information-reduced plan. If a response arrives, the abort is the lesson: blind counter-play in a hot district gambles.",
                    format_day_minute(pressure_at.as_minutes())
                );
            }
        }
        pending_witness_pressure = Some(authorize_witness_pressure(
            scenario,
            witness,
            &format!("Quiet word to {witness_name}"),
            pressure_at,
        )?);
    }

    Ok(pending_witness_pressure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crimocracy::core::time::DAY_MINUTES;

    #[test]
    fn laundering_preserves_reserve_even_when_capacity_exceeds_surplus() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let mut metrics = RunMetrics::default();
        run_until(
            &mut scenario,
            SimTime::from_minutes(DAY_MINUTES),
            false,
            &mut metrics,
        )
        .unwrap();
        let till = scenario
            .state
            .enterprises()
            .get_enterprise(scenario.enterprise)
            .unwrap()
            .cash_account();
        let balance = scenario
            .state
            .finance()
            .get_account(till)
            .unwrap()
            .balance()
            .cents();
        // A controlled lean till, made by moving earned cash into the other owned
        // reserve through the same ledger path as player capitalization.
        validate_record_transaction(
            &scenario.state,
            LedgerTransactionDraft {
                occurred_at: scenario.state.now(),
                memo: "Reserve cash outside the laundry till".to_owned(),
                postings: vec![
                    LedgerPosting {
                        account: till,
                        amount: Money::from_cents(6_000 - balance),
                    },
                    LedgerPosting {
                        account: scenario.expansion_cash,
                        amount: Money::from_cents(balance - 6_000),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut scenario.state)
        .unwrap();
        assert_eq!(
            launder_enterprise_till(&mut scenario, false, &mut metrics).unwrap(),
            Some(1_000)
        );
        assert_eq!(
            scenario
                .state
                .finance()
                .get_account(till)
                .unwrap()
                .balance()
                .cents(),
            5_000
        );
        let before = scenario.state.clone();
        assert_eq!(
            launder_enterprise_till(&mut scenario, false, &mut metrics).unwrap(),
            None
        );
        assert_eq!(
            bincode::serialize(&scenario.state).unwrap(),
            bincode::serialize(&before).unwrap(),
            "the reserve is not available to subsequent laundry calls"
        );
    }

    #[test]
    fn owner_draw_uses_earnings_once_and_leaves_opening_capital() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let mut metrics = RunMetrics::default();
        let operating = scenario
            .state
            .economy()
            .get_business_economy(scenario.front)
            .unwrap()
            .operating_account();
        let opening = scenario
            .state
            .finance()
            .get_account(operating)
            .unwrap()
            .balance();
        let before = scenario.state.clone();
        sweep_front_profits(&mut scenario, false, &mut metrics).unwrap();
        assert_eq!(
            bincode::serialize(&scenario.state).unwrap(),
            bincode::serialize(&before).unwrap()
        );
        run_until(
            &mut scenario,
            SimTime::from_minutes(DAY_MINUTES),
            false,
            &mut metrics,
        )
        .unwrap();
        let earned: i64 = scenario
            .state
            .economy()
            .cycles_for(scenario.front)
            .map(|cycle| cycle.net_cash().cents())
            .sum();
        sweep_front_profits(&mut scenario, false, &mut metrics).unwrap();
        assert_eq!(metrics.business_profits_swept_cents, earned);
        assert_eq!(
            scenario
                .state
                .finance()
                .get_account(operating)
                .unwrap()
                .balance(),
            opening
        );
        let before = scenario.state.clone();
        sweep_front_profits(&mut scenario, false, &mut metrics).unwrap();
        assert_eq!(
            bincode::serialize(&scenario.state).unwrap(),
            bincode::serialize(&before).unwrap(),
            "earnings cannot be withdrawn twice"
        );
    }

    #[test]
    #[ignore = "scenario-scale rotated-world PRESS arc; run cargo test-harness-deep or cargo harness-full"]
    fn deep_harness_concealed_reserve_does_not_prevent_legitimate_profit_financed_expansion() {
        let registry = crimocracy::build_registry();
        let mut metrics = play_session(
            &registry,
            Strategy::Press,
            ScenarioProfile::NightTrap,
            EvaluationSeeds::new(DEFAULT_WORLD_SEED + 1, DEFAULT_POLICY_SEED),
            SessionRunMode::FullQuiet,
        )
        .unwrap();
        assert_eq!(metrics.enterprise_till_concealed, Some(true));
        // The home racket's concealed reserve is never laundered directly - and it is
        // never spent as float either, so it cannot reach the wash through the lending
        // book or the off-track book. Any laundering that does happen must be
        // post-diversification flow: the harbor racket's own street till through the
        // harbor club, or a later home book's street take through the home front -
        // bounded by those books' honest earnings, never by the concealed home reserve.
        let launderable_book_earnings = metrics.expansion_net_cents.unwrap_or(0)
            + metrics.loan_net_cents.unwrap_or(0)
            + metrics.bookmaking_net_cents.unwrap_or(0);
        assert!(
            metrics.laundered_gross_cents <= launderable_book_earnings,
            "concealed home reserves must stay concealed: {} laundered against launderable book earnings of {}",
            metrics.laundered_gross_cents,
            launderable_book_earnings
        );
        assert!(metrics.business_profits_swept_cents >= metrics.acquisition_spent_cents);
        assert!(metrics.acquisition_rejections > 0);
        assert!(metrics.front_acquired && metrics.expansion_established);
        metrics.primary_narrative_set = true;
        validate_run_metrics(&metrics, true).unwrap();
        validate_press_expansion_evidence(&metrics).unwrap();
    }

    #[test]
    fn posture_suspends_only_a_losing_trailing_book() {
        assert_eq!(
            super::choose_racket_posture(-1),
            super::RacketPosture::Suspend
        );
        assert_eq!(
            super::choose_racket_posture(0),
            super::RacketPosture::KeepOpen
        );
        assert_eq!(
            super::choose_racket_posture(16_746),
            super::RacketPosture::KeepOpen
        );
    }

    #[test]
    fn press_stand_down_governs_home_book_and_surplus_capital() {
        let registry = crimocracy::build_registry();
        let mut metrics = play_session(
            &registry,
            Strategy::Press,
            ScenarioProfile::NightTrap,
            EvaluationSeeds::defaults(),
            SessionRunMode::FullQuiet,
        )
        .expect("full quiet press session should complete");
        assert!(
            metrics.posture_evaluations > 0,
            "the stand-down must actually review the home book from settled cycles"
        );
        // Over a war-chest-scale stand-down the district can carry heat the burglary-file
        // channel never sees (vice inquiries on our own racket, rivals' files), so a
        // losing book may legitimately suspend. The honest posture contract is one
        // recovery-interval reopen probe per suspension - no daily whipsaw - with at most
        // one probe still pending when the session ends.
        assert!(
            metrics.posture_suspensions == metrics.posture_resumptions
                || metrics.posture_suspensions == metrics.posture_resumptions + 1,
            "each suspension must pair with exactly one recovery-interval reopen probe: {} suspensions vs {} resumptions",
            metrics.posture_suspensions,
            metrics.posture_resumptions
        );
        assert!(
            metrics.front_acquired && metrics.expansion_established,
            "the harbor escape must open before surplus allocation"
        );
        assert!(
            metrics.annex_acquired,
            "surplus accounted funds must convert the lapsed score into a second front"
        );
        assert!(metrics.annex_price_cents.is_some_and(|price| price > 0));
        assert_eq!(
            metrics.annex_spent_cents,
            metrics.annex_price_cents.unwrap_or_default()
        );
        assert!(metrics.second_opportunity_expired);
        metrics.primary_narrative_set = true;
        validate_press_second_front_evidence(&metrics)
            .expect("the primary set must complete the second-front chain");
    }

    #[test]
    fn annex_short_purchase_is_a_canonical_rejection_without_state_mutation() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let mut metrics = RunMetrics {
            second_opportunity_expired: true,
            ..RunMetrics::default()
        };
        let before = scenario.state.clone();
        assert!(!acquire_annex_front(&mut scenario, false, &mut metrics).unwrap());
        assert_eq!(metrics.annex_rejections, 1);
        assert!(!metrics.annex_acquired);
        assert_eq!(
            bincode::serialize(&scenario.state).unwrap(),
            bincode::serialize(&before).unwrap()
        );
    }

    #[test]
    fn short_purchase_is_a_canonical_rejection_without_state_mutation() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let mut metrics = RunMetrics::default();
        let before = scenario.state.clone();
        assert!(!acquire_harbor_front(&mut scenario, false, &mut metrics).unwrap());
        assert_eq!(metrics.acquisition_rejections, 1);
        assert_eq!(
            bincode::serialize(&scenario.state).unwrap(),
            bincode::serialize(&before).unwrap()
        );
    }

    #[test]
    fn purchase_day_preserves_fifty_dollar_opening_float() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .expect("scenario builds");
        let mut metrics = RunMetrics::default();
        let mut stand_down = StandDownState::new(false);
        // The deadline is the same price-derived accumulation horizon the arc uses, not a
        // fixed day count, so authored price changes move the test bound with them.
        let deadline = stand_down_day_bound(&scenario);
        // Actual daily settlement and the live launder -> buy -> capitalize policy,
        // not a synthetic balance or a separate test implementation of that policy.
        for day in 1..=deadline {
            run_until(
                &mut scenario,
                SimTime::from_minutes(day * DAY_MINUTES),
                false,
                &mut metrics,
            )
            .expect("daily books settle");
            run_daily_capital_management(
                &mut scenario,
                "Central Precinct",
                false,
                &mut metrics,
                &mut stand_down,
            )
            .expect("daily capital management completes");
            if metrics.front_acquired {
                assert!(
                    metrics.expansion_established,
                    "purchase must open the book that day"
                );
                assert_eq!(
                    scenario
                        .state
                        .finance()
                        .get_account(scenario.expansion_cash)
                        .expect("expansion account persists")
                        .balance()
                        .cents(),
                    5_000,
                    "laundering must not consume the intended $50 opening float",
                );
                return;
            }
        }
        panic!("settled books never funded the purchase inside the price-derived horizon");
    }
}

struct StandDownState {
    capital_review_days: u32,
    last_absorbed: Option<i64>,
    till_concealed: bool,
    /// Per-front cap on owner draws: each venue's earned surplus can be withdrawn exactly
    /// once, so a multi-front organization tracks its draw ledger per business rather than
    /// double-counting a single session total across venues.
    swept_per_front: BTreeMap<BusinessId, i64>,
    /// Pace window for the war-chest heartbeat: the last review's accounted balance, plus
    /// the acquisition spend at that moment so a purchase resets the projection instead of
    /// reading as a negative income day.
    last_heartbeat: Option<(u32, i64, i64)>,
    /// One-time narration for the day a second front's own books start adding volume.
    harbor_volume_narrated: bool,
    /// Whether the defector hunt already ran inside this stand-down. The case-cooling
    /// beat triggers it once; later reviews must not repeat the watches or the appeal.
    personnel_recovered: bool,
    /// One-time narration for the first laundering cap the lending book hits: two
    /// home books share one front's plausible-volume window, which is exactly why
    /// the harbor club's separate books matter to more than district heat.
    loan_cap_narrated: bool,
    /// One-time narration while the off-track book's float waits for idle cash.
    book_wait_narrated: bool,
    /// Per-book posture governance state: the gambling book and the later lending
    /// book are each judged on their own trailing settled cycles, with their own
    /// suspension clock and resume-probe floor.
    gambling_posture: BookPostureState,
    loan_posture: BookPostureState,
    bookmaking_posture: BookPostureState,
}

impl StandDownState {
    fn new(till_concealed: bool) -> Self {
        Self {
            capital_review_days: 0,
            last_absorbed: None,
            till_concealed,
            swept_per_front: BTreeMap::new(),
            last_heartbeat: None,
            harbor_volume_narrated: false,
            personnel_recovered: false,
            loan_cap_narrated: false,
            book_wait_narrated: false,
            gambling_posture: BookPostureState::default(),
            loan_posture: BookPostureState::default(),
            bookmaking_posture: BookPostureState::default(),
        }
    }
}

fn till_is_concealed(scenario: &Scenario) -> bool {
    !scenario
        .state
        .finance()
        .get_account(
            scenario
                .state
                .enterprises()
                .get_enterprise(scenario.enterprise)
                .expect("canal enterprise must persist")
                .cash_account(),
        )
        .is_some_and(|account| account.kind() == crimocracy::finance::AccountKind::StreetCash)
}

/// Wash one racket's street till through a chosen owned front, keeping the standing float
/// reserve in the till. Concealed reserves cannot be laundered directly - that is the
/// production rule the home branch narrates - so a concealed till returns nothing rather
/// than fabricating a conversion path.
fn launder_enterprise_till_through(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    enterprise: EnterpriseId,
    front: BusinessId,
) -> Result<Option<i64>, Box<dyn Error>> {
    let cash_account = scenario
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("enterprise must persist")
        .cash_account();
    if !scenario
        .state
        .finance()
        .get_account(cash_account)
        .is_some_and(|account| account.kind() == AccountKind::StreetCash)
    {
        return Ok(None);
    }
    let launderable = scenario
        .state
        .finance()
        .get_account(cash_account)
        .map(|account| account.balance().cents())
        .unwrap_or_default()
        .saturating_sub(LAUNDERING_FLOAT_FLOOR_CENTS)
        .max(0);
    if launderable <= 0 {
        return Ok(None);
    }
    launder_through_owned_front(
        scenario,
        narrative,
        metrics,
        front,
        cash_account,
        launderable,
    )
}

/// The home racket's till still washes through the original front.
fn launder_enterprise_till(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<i64>, Box<dyn Error>> {
    launder_enterprise_till_through(
        scenario,
        narrative,
        metrics,
        scenario.enterprise,
        scenario.front,
    )
}

/// Wash one of the later home books' street tills (the lending book, then the off-track
/// book) through the home front's shared plausible-volume window. Returns the absorbed
/// amount and whether the front's window was too full to carry any of the till's
/// launderable surplus this cycle. Books wash only after settling real earnings; the
/// float is working capital, not a sum to be washed straight back out the day it was
/// raised. Concealed tills never reach this path - their reserve stays buried.
fn wash_auxiliary_home_till(
    scenario: &mut Scenario,
    metrics: &mut RunMetrics,
    enterprise: Option<EnterpriseId>,
    till: FinancialAccountId,
) -> Result<(Option<i64>, bool), Box<dyn Error>> {
    let has_settled_earnings = enterprise
        .is_some_and(|enterprise| scenario.state.enterprises().cycles_for(enterprise).count() >= 1);
    if !has_settled_earnings {
        return Ok((None, false));
    }
    let account = scenario
        .state
        .finance()
        .get_account(till)
        .expect("auxiliary home book till must persist");
    if account.kind() != AccountKind::StreetCash {
        return Ok((None, false));
    }
    let launderable = account
        .balance()
        .cents()
        .saturating_sub(LAUNDERING_FLOAT_FLOOR_CENTS)
        .max(0);
    if launderable <= 0 {
        return Ok((None, false));
    }
    let absorbed =
        launder_through_owned_front(scenario, false, metrics, scenario.front, till, launderable)?;
    let capped = absorbed.is_none();
    Ok((absorbed, capped))
}

/// Withdraw only settled legitimate earnings, never laundering fees or opening capital. The
/// production sweep checks real liquidity and ownership; this policy caps the draw to each
/// venue's earned surplus so profitable books remain operating assets. The cap is tracked
/// per business by the caller, so a multi-front organization draws each front's profit
/// exactly once. Returns the cents actually swept.
pub(crate) fn sweep_front_profits_of(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    business: BusinessId,
    already_swept_cents: i64,
) -> Result<i64, Box<dyn Error>> {
    use crimocracy::economy::business_economy_system::{
        BusinessProfitSweepDraft, validate_sweep_business_profits,
    };
    let Some(economy) = scenario.state.economy().get_business_economy(business) else {
        return Ok(0);
    };
    let earned: i64 = scenario
        .state
        .economy()
        .cycles_for(business)
        .map(|cycle| cycle.net_cash().cents())
        .sum();
    let available = scenario
        .state
        .finance()
        .get_account(economy.operating_account())
        .expect("front till persists")
        .balance()
        .cents()
        .max(0);
    let amount = (earned - already_swept_cents).max(0).min(available);
    if amount == 0 {
        return Ok(0);
    }
    let before = scenario
        .state
        .finance()
        .get_account(scenario.accounted_funds)
        .expect("accounted books persist")
        .balance()
        .cents();
    validate_sweep_business_profits(
        &scenario.state,
        BusinessProfitSweepDraft {
            organization: scenario.player,
            business,
            destination: scenario.accounted_funds,
            amount: Money::from_cents(amount),
        },
    )?
    .commit(&mut scenario.state)?;
    let after = scenario
        .state
        .finance()
        .get_account(scenario.accounted_funds)
        .expect("accounted books persist")
        .balance()
        .cents();
    let swept = after - before;
    debug_assert_eq!(
        swept, amount,
        "the committed owner draw must move exactly the validated amount"
    );
    metrics.business_profits_swept_cents = metrics
        .business_profits_swept_cents
        .checked_add(swept)
        .expect("session owner-draw total must fit money range");
    if narrative {
        println!(
            "[OWNER DRAW] Withdraw {} of earned front profits into accounted funds. Legitimate trade can finance the purchase too; opening capital stays in the business.",
            format_cents(swept)
        );
    }
    Ok(swept)
}

/// Single-front convenience for callers that keep their draw ledger in the shared session
/// total (the retention probe): sweeps the original home front with the accumulated
/// metrics counter as its per-front cap.
pub(crate) fn sweep_front_profits(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    sweep_front_profits_of(
        scenario,
        narrative,
        metrics,
        scenario.front,
        metrics.business_profits_swept_cents,
    )?;
    Ok(())
}

fn record_daily_laundering(
    absorbed: Option<i64>,
    narrative: bool,
    stand_down: &mut StandDownState,
) {
    stand_down.capital_review_days += 1;
    // Daily amounts move with the till balance, not just the books' ceiling, so printing every
    // small change buries the story. The [WAIT] heartbeat already quotes the accounted total;
    // say something here only when the wash stalls and street cash starts pooling.
    if narrative && absorbed.is_none() && stand_down.last_absorbed.is_some() {
        println!(
            "[LAUNDER] The front's books absorbed nothing today; the volume waits as street cash."
        );
    }
    stand_down.last_absorbed = absorbed;
}

fn run_daily_capital_management(
    scenario: &mut Scenario,
    police_name: &str,
    narrative: bool,
    metrics: &mut RunMetrics,
    stand_down: &mut StandDownState,
) -> Result<(), Box<dyn Error>> {
    let first_laundry = stand_down.capital_review_days == 0;
    // Inspect the real purchase gate before raising capital, once. A short book is
    // validator evidence, not a scripted balance comparison masquerading as rejection.
    if !metrics.front_acquired && metrics.acquisition_rejections == 0 {
        acquire_harbor_front(scenario, narrative, metrics)?;
    }
    // The stand-down's one in-district growth move, once the channel confirms the file
    // shelved and the crew is whole: idle cash from the day's unwashed take goes to
    // work as a lending book at the owned home front. This fills the quiet stretch
    // between the cold read and the harbor purchase with a real capital-allocation
    // decision - the same accumulation the [GOAL] line promises, earned by governance
    // rather than street risk.
    if metrics.cold_case_confirmed == Some(true)
        && !metrics.loan_established
        && (stand_down.personnel_recovered || metrics.defector.is_none())
    {
        establish_home_lending_book(scenario, narrative, metrics)?;
    }
    // Owner draws from every front the organization owns: each venue's earned surplus
    // enters the war chest exactly once, tracked per business so a growing portfolio
    // cannot double-draw the same profit.
    sweep_owned_front_profits(scenario, narrative && first_laundry, metrics, stand_down)?;

    if metrics.front_acquired && !metrics.expansion_established {
        // Capitalize before sweeping fresh income. Otherwise the same till funds laundering
        // first and can permanently starve the newly acquired book.
        establish_harbor_expansion(scenario, narrative, metrics)?;
        let absorbed = launder_enterprise_till(scenario, false, metrics)?;
        record_daily_laundering(absorbed, narrative, stand_down);
        return Ok(());
    }

    if narrative && first_laundry && !stand_down.till_concealed {
        println!(
            "[DECIDE]  Keep a $50 street reserve for the new book; wash only the surplus. Withdraw earned front profits as well: legitimate income and washed money both buy the harbor venue."
        );
    }
    // A second cash-intensive front multiplies laundering volume the way ownership should:
    // the harbor racket's own till washes through the harbor club's books while the home
    // book keeps its original channel. Each front's plausible volume is its own.
    let home_absorbed = launder_enterprise_till(scenario, narrative && first_laundry, metrics)?;
    // The later home books (lending, then the off-track book) share the home front's
    // plausible-volume window with the gambling take, so a full window queues their
    // surplus as street cash - the honest bottleneck that makes a second set of front
    // books worth buying. Wash only after a book has settled real earnings; the float
    // is working capital, not a sum to be washed straight back out on the day it was
    // raised.
    let (loan_absorbed, loan_capped) = wash_auxiliary_home_till(
        scenario,
        metrics,
        metrics.loan_enterprise,
        scenario.loan_cash,
    )?;
    let (book_absorbed, book_capped) = wash_auxiliary_home_till(
        scenario,
        metrics,
        metrics.bookmaking_enterprise,
        scenario.bookmaking_cash,
    )?;
    if (loan_capped || book_capped) && narrative && !stand_down.loan_cap_narrated {
        stand_down.loan_cap_narrated = true;
        let front_name = scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("home front must persist")
            .name()
            .to_owned();
        println!(
            "[LAUNDER] {front_name}'s books are carrying all the volume they plausibly can this cycle; the surplus waits as street cash. Every racket on one front shares its window - more fronts, or a second set of front books, would carry more per day."
        );
    }
    let auxiliary_absorbed = match (loan_absorbed, book_absorbed) {
        (Some(loan), Some(book)) => Some(loan + book),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    };
    let harbor_absorbed = match metrics.expansion_enterprise {
        Some(expansion) if metrics.expansion_established => {
            let absorbed = launder_enterprise_till_through(
                scenario,
                false,
                metrics,
                expansion,
                scenario.expansion_front,
            )?;
            if absorbed.is_some() && narrative && !stand_down.harbor_volume_narrated {
                stand_down.harbor_volume_narrated = true;
                println!(
                    "[LAUNDER] The harbor club's own books now wash the harbor racket's take as well; a second front roughly doubles how fast dirty money turns clean."
                );
            }
            absorbed
        }
        _ => None,
    };
    let absorbed = match (home_absorbed, auxiliary_absorbed, harbor_absorbed) {
        (Some(home), Some(auxiliary), Some(harbor)) => Some(home + auxiliary + harbor),
        (Some(home), Some(auxiliary), None) => Some(home + auxiliary),
        (Some(home), None, Some(harbor)) => Some(home + harbor),
        (None, Some(auxiliary), Some(harbor)) => Some(auxiliary + harbor),
        (Some(home), None, None) => Some(home),
        (None, Some(auxiliary), None) => Some(auxiliary),
        (None, None, Some(harbor)) => Some(harbor),
        (None, None, None) => None,
    };
    record_daily_laundering(absorbed, narrative && !first_laundry, stand_down);

    if !metrics.front_acquired && acquire_harbor_front(scenario, narrative, metrics)? {
        establish_harbor_expansion(scenario, narrative, metrics)?;
        if metrics.expansion_established && narrative {
            println!(
                "[DECIDE]  Standing down does not mean going deaf: once a day, {police_name}-channel asks only - has anything moved on the case?"
            );
        }
    }
    // Second-front allocation with surplus only: the harbor escape keeps priority, so
    // this runs once the harbor book is open and the lapsed score is honestly history.
    if metrics.front_acquired
        && metrics.expansion_established
        && !metrics.annex_acquired
        && metrics.second_opportunity_expired
    {
        acquire_annex_front(scenario, narrative, metrics)?;
    }
    // Empire ladder: with income property secured, surplus clean money buys the wire
    // office - the capability unlock for off-track bookmaking - and the book opens as
    // soon as idle cash covers its float. The purchase and the book opening are separate
    // gates: the establishment retries on later reviews until idle street cash actually
    // covers the float.
    if metrics.annex_acquired && !metrics.wire_acquired {
        acquire_wire_service(scenario, narrative, metrics)?;
    }
    if metrics.wire_acquired && !metrics.bookmaking_established {
        establish_bookmaking_book(scenario, narrative, metrics, stand_down)?;
    }
    Ok(())
}

/// Authored working-capital target for the stand-down's lending book, and the floor
/// below which opening the book would just be an empty gesture. The float is small on
/// purpose: it is idle cash put to work, not a second treasury.
const LENDING_FLOAT_TARGET_CENTS: i64 = 10_000;
const LENDING_FLOAT_MINIMUM_CENTS: i64 = 2_500;

/// Idle street cash a boss can actually move into a new book's float: the general
/// street treasury in full, plus each street-cash racket till's surplus above its
/// standing laundering float. Concealed tills are skipped entirely - a buried hoard
/// funds nothing - and a destination account is never a source for its own float.
/// Returns balanced debit postings plus the destination credit, up to the target.
fn plan_idle_street_float(
    scenario: &Scenario,
    target_cents: i64,
    destination: FinancialAccountId,
) -> (Vec<LedgerPosting>, i64) {
    let mut postings = Vec::new();
    let mut remaining = target_cents.max(0);
    // The general treasury has no float obligation: every cent of it is idle.
    let mut sources = vec![(scenario.liquidation_cash, 0_i64)];
    // Racket tills keep their standing laundering float; only the surplus above it is
    // idle. Ordered home books first, then the second-district book, so the funding
    // choice is deterministic across runs.
    sources.extend([
        (
            scenario
                .state
                .enterprises()
                .get_enterprise(scenario.enterprise)
                .expect("home gambling book must persist")
                .cash_account(),
            LAUNDERING_FLOAT_FLOOR_CENTS,
        ),
        (scenario.loan_cash, LAUNDERING_FLOAT_FLOOR_CENTS),
        (scenario.bookmaking_cash, LAUNDERING_FLOAT_FLOOR_CENTS),
        (scenario.expansion_cash, LAUNDERING_FLOAT_FLOOR_CENTS),
    ]);
    for (account_id, floor) in sources {
        if remaining <= 0 {
            break;
        }
        if account_id == destination {
            continue;
        }
        let Some(account) = scenario.state.finance().get_account(account_id) else {
            continue;
        };
        if account.kind() != AccountKind::StreetCash {
            continue;
        }
        let surplus = (account.balance().cents() - floor).max(0);
        let draw = remaining.min(surplus);
        if draw > 0 {
            postings.push(LedgerPosting {
                account: account_id,
                amount: Money::from_cents(-draw),
            });
            remaining -= draw;
        }
    }
    let funded = target_cents.max(0) - remaining;
    postings.push(LedgerPosting {
        account: destination,
        amount: Money::from_cents(funded),
    });
    (postings, funded)
}

/// The stand-down's in-district growth play. Once the channel confirms the burglary file
/// shelved and the crew is whole, idle street cash stops sitting still: leadership opens a
/// quiet lending book at the home front. LoanSharking needs only a cash-intensive venue,
/// and the club already hosts the gambling book, so one front carries two rackets. The
/// float comes from cash leadership can actually reach - the general street treasury
/// first, then the gambling till's surplus above its standing float - and establishment
/// runs through the canonical enterprise path under the existing home-district mandate.
/// No new authority, no invented conversion: the same mandate that governs the gambling
/// book admits a second kind at the same venue, and the new book's take is still dirty
/// street cash that must wash through the same front's plausible-volume window.
fn establish_home_lending_book(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    if metrics.loan_established {
        return Ok(());
    }
    let (float_postings, float_cents) =
        plan_idle_street_float(scenario, LENDING_FLOAT_TARGET_CENTS, scenario.loan_cash);
    // Concealed reserves stay concealed: a buried hoard is not working capital for a new
    // book, because every dollar that cycles through the loan till's daily wash would
    // effectively launder it. Only genuinely launderable street cash may fund the float,
    // which `plan_idle_street_float` enforces by skipping concealed tills.
    let gambling_cash = scenario
        .state
        .enterprises()
        .get_enterprise(scenario.enterprise)
        .expect("home gambling book must persist")
        .cash_account();
    let gambling_account = scenario
        .state
        .finance()
        .get_account(gambling_cash)
        .expect("gambling till must persist");
    let till_concealed = gambling_account.kind() != AccountKind::StreetCash;
    if float_cents < LENDING_FLOAT_MINIMUM_CENTS {
        metrics.loan_funding_deferred_days = metrics.loan_funding_deferred_days.saturating_add(1);
        if narrative && metrics.loan_funding_deferred_days == 1 {
            if till_concealed {
                println!(
                    "[EXPAND]   The lending book waits for idle street cash: the gambling take sits concealed where it cannot fund or wash anything. Legitimate front profits keep building the war chest in the meantime."
                );
            } else {
                println!(
                    "[EXPAND]   The lending book waits for idle cash; the next settled cycle funds the float."
                );
            }
        }
        return Ok(());
    }
    let front_name = scenario
        .state
        .world()
        .get_business(scenario.front)
        .expect("home front must persist")
        .name()
        .to_owned();
    let lieutenant_name = scenario
        .state
        .world()
        .get_character(scenario.lieutenant)
        .expect("lieutenant must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[DECIDE]  The file is shelved and the crew is whole. Standing down from street work does not mean idle cash sits idle: the club's back room can run a quiet lending book under {lieutenant_name}'s standing home-district authority."
        );
    }
    // The posting plan was derived from live balances above and nothing mutated state
    // in between (only read-only name lookups and narration), so commit it directly.
    validate_record_transaction(
        &scenario.state,
        LedgerTransactionDraft {
            occurred_at: scenario.state.now(),
            memo: "Capitalize the lending book".to_owned(),
            postings: float_postings,
            authorization: None,
        },
    )?
    .commit(&mut scenario.state)?;
    let loan = validate_establish_enterprise(
        scenario.registry,
        &scenario.state,
        EnterpriseDraft {
            kind: EnterpriseKind::LoanSharking,
            organization: scenario.player,
            authority: MandateAuthority {
                mandate: scenario.lieutenant_mandate,
                manager: scenario.lieutenant,
                scope: ResponsibilityScope::Neighborhood(scenario.neighborhood),
            },
            location: EnterpriseLocation::Business(scenario.front),
            supporting_businesses: BTreeSet::new(),
            cash_account: scenario.loan_cash,
            settlement_account: scenario.loan_settlement,
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.loan_enterprise = Some(loan);
    metrics.loan_established = true;
    metrics.loan_open_minute = Some(scenario.state.now().as_minutes());
    if narrative {
        println!(
            "[EXPAND]   Lending book established at {front_name} with a {} float: two rackets now share one front, and their combined take still has to wash through the same set of books.",
            format_cents(float_cents),
        );
    }
    Ok(())
}

/// Sweeps earned surplus from every business the organization currently owns that has a
/// live operating economy - the home front, the acquired harbor club, and the annex
/// front once it is bought - using the per-front draw ledger in `stand_down`.
fn sweep_owned_front_profits(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    stand_down: &mut StandDownState,
) -> Result<(), Box<dyn Error>> {
    let owned: Vec<BusinessId> = scenario
        .state
        .world()
        .businesses_owned_by_organization(scenario.player)
        .map(|record| record.id())
        .collect();
    for business in owned {
        let already_swept = stand_down
            .swept_per_front
            .get(&business)
            .copied()
            .unwrap_or(0);
        let swept = sweep_front_profits_of(scenario, narrative, metrics, business, already_swept)?;
        if swept > 0 {
            stand_down
                .swept_per_front
                .insert(business, already_swept + swept);
        }
    }
    Ok(())
}

/// A boss's home-racket posture from the trailing settled book alone: while the
/// observable trailing net is negative, suspending stops the settlements,
/// surcharges, and new vice draws; otherwise the book stays open and earning.
/// An exactly break-even book stays open: it costs nothing to hold while its
/// manager keeps watching the district.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RacketPosture {
    KeepOpen,
    Suspend,
}

fn choose_racket_posture(trailing_net_cents: i64) -> RacketPosture {
    if trailing_net_cents < 0 {
        RacketPosture::Suspend
    } else {
        RacketPosture::KeepOpen
    }
}

/// Per-book posture memory: when the book was suspended (so the reopen probe waits a
/// full authored recovery interval instead of whipsawing on a latched cold read) and
/// when it was last resumed (cycles settled before the resume are stale for posture
/// judgments; only the probe's own results decide the next move).
#[derive(Clone, Copy, Debug, Default)]
struct BookPostureState {
    suspended_since: Option<u64>,
    resumed_at: Option<u64>,
}

/// Daily player-visible posture governance during the stand-down, applied to every home
/// book the organization runs: read the last two settled cycles from production state
/// (the same manager reports leadership holds), suspend through the canonical path while
/// the trailing book loses money, and probe the district with a single reopen only after
/// an authored recovery interval has passed. The interval equals the production cold-case
/// window because that is how long a boss knows an institutional file takes to shelf on
/// its own; district heat can come from vice inquiries and rivals' files the contact
/// channel never sees, so a shelved burglary file alone is not proof the books will pay
/// again. A freshly reopened book is judged only on cycles settled since the resume, so
/// stale pre-suspension losses cannot whipsaw it straight back down. Runs identically
/// with or without narration; prints are gated.
fn govern_home_book_posture(
    scenario: &mut Scenario,
    enterprise: EnterpriseId,
    label: &str,
    narrative: bool,
    metrics: &mut RunMetrics,
    posture: &mut BookPostureState,
) -> Result<(), Box<dyn Error>> {
    use crimocracy::enterprises::EnterpriseStatus;
    use crimocracy::enterprises::enterprise_execution::{
        validate_resume_enterprise, validate_suspend_enterprise,
    };
    let now = scenario.state.now().as_minutes();
    if scenario
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("home enterprise must persist")
        .status()
        != EnterpriseStatus::Active
    {
        // Suspended: no fresh cycles settle, so trailing numbers are stale. Wait a full
        // recovery interval (the authored cold-case window) before spending one reopen
        // probe on the district; the probe's own settled cycle then reports honestly
        // whether the heat has cleared.
        if let Some(suspended_at) = posture.suspended_since
            && now
                >= suspended_at
                    + u64::from(scenario.registry.legal().cold_case_window().as_minutes())
        {
            validate_resume_enterprise(scenario.registry, &scenario.state, enterprise)?
                .commit(&mut scenario.state)?;
            metrics.posture_resumptions = metrics.posture_resumptions.saturating_add(1);
            posture.suspended_since = None;
            posture.resumed_at = Some(now);
            if narrative {
                println!(
                    "[POSTURE] A recovery interval has passed since the {label} closed; leadership probes the district with one reopened cycle. The next settled book reports whether the heat has actually cleared."
                );
            }
        }
        return Ok(());
    }
    // Judge only cycles settled since the last resume probe, so a freshly reopened
    // book is measured on its own results rather than on pre-suspension losses.
    let resumed_at = posture.resumed_at.unwrap_or(0);
    let trailing: Vec<(i64, i64)> = scenario
        .state
        .enterprises()
        .cycles_for(enterprise)
        .rev()
        .take(2)
        .filter(|cycle| cycle.occurred_at().as_minutes() >= resumed_at)
        .map(|cycle| (cycle.net_cash().cents(), cycle.investigation_heat().cents()))
        .collect();
    if trailing.len() < 2 {
        return Ok(());
    }
    metrics.posture_evaluations = metrics.posture_evaluations.saturating_add(1);
    let first_review = metrics.posture_evaluations == 1;
    let trailing_net: i64 = trailing.iter().map(|(net, _)| net).sum();
    let trailing_heat: i64 = trailing.iter().map(|(_, heat)| heat).sum();
    if choose_racket_posture(trailing_net) == RacketPosture::Suspend {
        validate_suspend_enterprise(&scenario.state, enterprise)?.commit(&mut scenario.state)?;
        metrics.posture_suspensions = metrics.posture_suspensions.saturating_add(1);
        posture.suspended_since = Some(now);
        if narrative {
            println!(
                "[POSTURE] {label} trailing {} over the last {} settled cycle(s) with {} of heat: suspending the racket until the file cools. Wages still come due and the front keeps trading; the bleeding stops.",
                format_cents(trailing_net),
                trailing.len(),
                format_cents(trailing_heat),
            );
        }
    } else if narrative && first_review {
        println!(
            "[POSTURE] {label} trailing {} over the last {} settled cycle(s) with {} of heat: keeping it open while it pays. Leadership re-checks daily and suspends if heat pushes the book negative.",
            format_cents(trailing_net),
            trailing.len(),
            format_cents(trailing_heat),
        );
    }
    Ok(())
}

/// Posture governance for every home book: the gambling book the fixture opens with and
/// the lending book the stand-down adds. Each book is judged on its own trailing
/// results; district heat taxes all of them, and a boss suspends whichever books bleed.
fn govern_home_racket_posture(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    stand_down: &mut StandDownState,
) -> Result<(), Box<dyn Error>> {
    let gambling = scenario.enterprise;
    govern_home_book_posture(
        scenario,
        gambling,
        "gambling book",
        narrative,
        metrics,
        &mut stand_down.gambling_posture,
    )?;
    if let Some(loan) = metrics.loan_enterprise.filter(|_| metrics.loan_established) {
        govern_home_book_posture(
            scenario,
            loan,
            "lending book",
            narrative,
            metrics,
            &mut stand_down.loan_posture,
        )?;
    }
    if let Some(book) = metrics
        .bookmaking_enterprise
        .filter(|_| metrics.bookmaking_established)
    {
        govern_home_book_posture(
            scenario,
            book,
            "off-track book",
            narrative,
            metrics,
            &mut stand_down.bookmaking_posture,
        )?;
    }
    Ok(())
}

/// The PRESS second purchase: with the harbor escape secured, surplus accounted funds
/// buy the lapsed annex score as pure legitimate infrastructure. It earns real front
/// income the case cannot tax and hosts no racket, so it adds no heat surface. The
/// purchase is gated on the lapsed opportunity so leadership never scores property it
/// owns, and on the open harbor book so heat escape keeps priority over marginal income.
/// Harbor accounting stays exclusive: this path records its own price, spend, and
/// short-book rejections.
pub fn acquire_annex_front(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<bool, Box<dyn Error>> {
    use crimocracy::economy::business_acquisition::{
        BusinessAcquisitionDraft, validate_acquire_business,
    };
    use crimocracy::finance::{AccountKind, FinancialOwner};
    if !metrics.second_opportunity_expired {
        return Ok(false);
    }
    let price = scenario
        .registry
        .get_business(
            scenario
                .state
                .world()
                .get_business(scenario.alternate_target)
                .expect("annex target must persist")
                .kind(),
        )
        .economics()
        .acquisition_cost();
    let funding_accounts: BTreeSet<_> = scenario
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(scenario.player))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect();
    if narrative && metrics.annex_rejections == 0 && !metrics.annex_acquired {
        println!(
            "[DECIDE]  Harbor first: escaping the Canal case dominates marginal income. The annex - the score we refused - waits for surplus; with clean money to spare it becomes infrastructure instead of temptation."
        );
    }
    let purchase = validate_acquire_business(
        scenario.registry,
        &scenario.state,
        BusinessAcquisitionDraft {
            organization: scenario.player,
            business: scenario.alternate_target,
            funding_accounts,
        },
    );
    let purchase = match purchase {
        Ok(purchase) => purchase,
        Err(
            crimocracy::economy::business_acquisition::BusinessAcquisitionError::InsufficientFunds {
                available_cents,
                price_cents,
            },
        ) => {
            if metrics.annex_rejections == 0 && narrative {
                println!(
                    "[ACQUIRE] The annex seller wants {}; our accounted books hold only {} after the harbor purchase. Income property waits for surplus.",
                    format_cents(price_cents),
                    format_cents(available_cents),
                );
            }
            metrics.annex_rejections = metrics.annex_rejections.saturating_add(1);
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    let venue_name = scenario
        .state
        .world()
        .get_business(scenario.alternate_target)
        .expect("annex target must persist")
        .name()
        .to_owned();
    purchase.commit(&mut scenario.state)?;
    assert_eq!(
        scenario
            .state
            .world()
            .get_business(scenario.alternate_target)
            .expect("acquired annex must persist")
            .owner(),
        crimocracy::world::BusinessOwner::Organization(scenario.player),
        "a committed acquisition must transfer venue ownership"
    );
    assert!(
        scenario
            .state
            .economy()
            .get_business_economy(scenario.alternate_target)
            .is_some(),
        "a committed acquisition must open the venue's operating economy"
    );
    metrics.annex_acquired = true;
    metrics.annex_price_cents = Some(price.cents());
    metrics.annex_spent_cents = price.cents();
    // The clean-money identity in `validate_run_metrics` reconciles every accounted-funds
    // purchase against the final balance, so the second front joins the same spend total
    // rather than living in a separate ledger the contract cannot see.
    metrics.acquisition_spent_cents = metrics
        .acquisition_spent_cents
        .checked_add(price.cents())
        .expect("session acquisition spend must fit money range");
    if narrative {
        println!(
            "[ACQUIRE] {}: {venue_name} purchased outright for {} from accounted surplus: the score we refused becomes infrastructure - legitimate income the case cannot tax, run clean with no book to heat.",
            stamp(scenario.state.now().as_minutes()),
            format_cents(price.cents()),
        );
    }
    Ok(true)
}

/// The empire-ladder purchase: once the annex is owned and the books keep compounding,
/// clean money buys the independent racing-wire office. This is not one more venue - it
/// is the only way into off-track bookmaking, because the wire is a network dependency
/// and cash at the club cannot substitute for it. Ownership transfer, the live operating
/// economy, and the accounted-funds price run through the same canonical acquisition
/// path as the harbor club and the annex; the wire keeps its own price/spend/rejection
/// accounting the same way the annex does.
pub fn acquire_wire_service(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<bool, Box<dyn Error>> {
    use crimocracy::economy::business_acquisition::{
        BusinessAcquisitionDraft, validate_acquire_business,
    };
    use crimocracy::finance::{AccountKind, FinancialOwner};
    if !metrics.annex_acquired || metrics.wire_acquired {
        return Ok(false);
    }
    let price = scenario
        .registry
        .get_business(
            scenario
                .state
                .world()
                .get_business(scenario.wire_front)
                .expect("wire office must persist")
                .kind(),
        )
        .economics()
        .acquisition_cost();
    let funding_accounts: BTreeSet<_> = scenario
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(scenario.player))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| account.id())
        .collect();
    if narrative && metrics.wire_rejections == 0 {
        println!(
            "[DECIDE]  The books compound faster than the club can spend them. The next purchase is not another venue: the racing-wire office is the only door into off-track bookmaking, and no amount of cash at the club substitutes for its track reports."
        );
    }
    let purchase = validate_acquire_business(
        scenario.registry,
        &scenario.state,
        BusinessAcquisitionDraft {
            organization: scenario.player,
            business: scenario.wire_front,
            funding_accounts,
        },
    );
    let purchase = match purchase {
        Ok(purchase) => purchase,
        Err(
            crimocracy::economy::business_acquisition::BusinessAcquisitionError::InsufficientFunds {
                available_cents,
                price_cents,
            },
        ) => {
            if metrics.wire_rejections == 0 && narrative {
                println!(
                    "[ACQUIRE] The wire office wants {}; our accounted books hold only {}. The track reports wait for surplus.",
                    format_cents(price_cents),
                    format_cents(available_cents),
                );
            }
            metrics.wire_rejections = metrics.wire_rejections.saturating_add(1);
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    let wire_name = scenario
        .state
        .world()
        .get_business(scenario.wire_front)
        .expect("wire office must persist")
        .name()
        .to_owned();
    purchase.commit(&mut scenario.state)?;
    assert_eq!(
        scenario
            .state
            .world()
            .get_business(scenario.wire_front)
            .expect("acquired wire must persist")
            .owner(),
        crimocracy::world::BusinessOwner::Organization(scenario.player),
        "a committed acquisition must transfer venue ownership"
    );
    assert!(
        scenario
            .state
            .economy()
            .get_business_economy(scenario.wire_front)
            .is_some(),
        "a committed acquisition must open the venue's operating economy"
    );
    metrics.wire_acquired = true;
    metrics.wire_price_cents = Some(price.cents());
    metrics.wire_spent_cents = price.cents();
    // The clean-money identity in `validate_run_metrics` reconciles every accounted-funds
    // purchase against the final balance, so the wire joins the same spend total.
    metrics.acquisition_spent_cents = metrics
        .acquisition_spent_cents
        .checked_add(price.cents())
        .expect("session acquisition spend must fit money range");
    if narrative {
        println!(
            "[ACQUIRE] {}: {wire_name} purchased outright for {} from accounted surplus: a legitimate newsroom by day, and by night the only thing standing between this organization and the off-track betting market.",
            stamp(scenario.state.now().as_minutes()),
            format_cents(price.cents()),
        );
    }
    Ok(true)
}

/// The empire-ladder racket the wire unlock makes possible: an off-track book at the
/// home front, with the owned wire office as its network dependency. The float comes
/// from idle street cash the same way the lending book's does (concealed reserves stay
/// buried), and establishment runs under the standing two-district mandate - the wire
/// sits in the home district, so the home-district scope covers the dependency too.
fn establish_bookmaking_book(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    stand_down: &mut StandDownState,
) -> Result<(), Box<dyn Error>> {
    if metrics.bookmaking_established {
        return Ok(());
    }
    // The float draws on every idle street till the organization holds - treasury plus
    // each racket till's surplus above its standing float - because the organization's
    // idle cash is spread across its books while the home laundering window caps daily
    // washing. Concealed tills stay buried by the same rule as the lending book.
    let (float_postings, float_cents) = plan_idle_street_float(
        scenario,
        LENDING_FLOAT_TARGET_CENTS,
        scenario.bookmaking_cash,
    );
    if float_cents < LENDING_FLOAT_MINIMUM_CENTS {
        if narrative && !stand_down.book_wait_narrated {
            stand_down.book_wait_narrated = true;
            println!(
                "[EXPAND]   The off-track book waits for idle street cash to cover the float; the wire reports keep coming either way."
            );
        }
        return Ok(());
    }
    let front_name = scenario
        .state
        .world()
        .get_business(scenario.front)
        .expect("home front must persist")
        .name()
        .to_owned();
    let wire_name = scenario
        .state
        .world()
        .get_business(scenario.wire_front)
        .expect("wire office must persist")
        .name()
        .to_owned();
    // The posting plan was derived from live balances above and nothing mutated state
    // in between (only read-only name lookups), so commit it directly.
    validate_record_transaction(
        &scenario.state,
        LedgerTransactionDraft {
            occurred_at: scenario.state.now(),
            memo: "Capitalize the off-track book".to_owned(),
            postings: float_postings,
            authorization: None,
        },
    )?
    .commit(&mut scenario.state)?;
    let book = validate_establish_enterprise(
        scenario.registry,
        &scenario.state,
        EnterpriseDraft {
            kind: EnterpriseKind::Bookmaking,
            organization: scenario.player,
            authority: MandateAuthority {
                mandate: scenario.lieutenant_mandate,
                manager: scenario.lieutenant,
                scope: ResponsibilityScope::Neighborhood(scenario.neighborhood),
            },
            location: EnterpriseLocation::Business(scenario.front),
            supporting_businesses: BTreeSet::from([scenario.wire_front]),
            cash_account: scenario.bookmaking_cash,
            settlement_account: scenario.bookmaking_settlement,
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.bookmaking_enterprise = Some(book);
    metrics.bookmaking_established = true;
    metrics.bookmaking_open_minute = Some(scenario.state.now().as_minutes());
    if narrative {
        println!(
            "[EXPAND]   Off-track book established at {front_name} with a {} float, taking {wire_name}'s track reports as its information dependency: a market the organization could not enter at any price before owning the wire.",
            format_cents(float_cents),
        );
        println!(
            "[NARRATION] The organization that started with one club and one card game now runs three books - cards, loans, and the track - across two districts, with its own wire service feeding the third. None of it needed a single new street job; it needed clean money, owned venues, and the discipline to keep taking them."
        );
    }
    Ok(())
}

/// Bounded daily reviews derived from production timing and authored prices: the
/// cold-case window for the file to shelve, plus a war-chest horizon long enough for
/// accounted funds to cover the harbor venue and the later annex at their authored
/// prices at a conservative floor of clean accumulation per review. A fixed day count
/// would go stale whenever authored prices or economics change.
fn stand_down_day_bound(scenario: &Scenario) -> u64 {
    let cold_window_minutes = u64::from(scenario.registry.legal().cold_case_window().as_minutes());
    let cold_days = cold_window_minutes.div_ceil(DAY_MINUTES);
    let harbor_price = scenario
        .registry
        .get_business(crimocracy::world::BusinessKind::Hospitality)
        .economics()
        .acquisition_cost()
        .cents();
    let annex_price = scenario
        .registry
        .get_business(crimocracy::world::BusinessKind::Retail)
        .economics()
        .acquisition_cost()
        .cents();
    let wire_price = scenario
        .registry
        .get_business(crimocracy::world::BusinessKind::NewsService)
        .economics()
        .acquisition_cost()
        .cents();
    // Conservative clean-accumulation floor per review day: roughly one small front's
    // plausible laundering net plus a modest owner draw. Observed pacing on the authored
    // fixture runs well above this floor, so honest worlds finish early; the floor keeps
    // slow worlds from being cut off mid-accumulation instead of ending their arcs early.
    const CLEAN_ACCUMULATION_FLOOR_CENTS: i64 = 15_000;
    let war_chest_target = harbor_price + annex_price + wire_price;
    let war_chest_days = u64::try_from(
        (war_chest_target + CLEAN_ACCUMULATION_FLOOR_CENTS - 1) / CLEAN_ACCUMULATION_FLOOR_CENTS,
    )
    .expect("authored prices must fit the review-day bound");
    // Margin covers daily polling cadence, custody-deferred case decay, and the final
    // purchase/settlement beats after the last review.
    cold_days + war_chest_days + 8
}

/// Latest start for a patrol-informed quiet word, derived from production: institutional
/// interviews land within the longest authored investigation-work duration after intake.
fn interview_horizon_minutes(scenario: &Scenario) -> u64 {
    ALL_INVESTIGATION_WORK_KINDS
        .iter()
        .map(|kind| {
            u64::from(
                scenario
                    .registry
                    .get_investigation_work(*kind)
                    .duration()
                    .as_minutes(),
            )
        })
        .max()
        .unwrap_or_default()
}

fn run_stand_down_and_diversify(
    scenario: &mut Scenario,
    burglary: OperationId,
    police_name: &str,
    campaign_day_minutes: u64,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    // Every campaign day the organization launders the racket's till through its
    // front's books and asks its standing precinct contact whether anything moved on
    // the case - daily tradecraft, not calendar math: leadership cannot know when the
    // file will go cold, so it keeps asking until the channel itself carries the
    // shelved read. Once the file is shelved the war chest becomes the only remaining
    // gate: reviews continue while clean books accumulate toward the harbor venue and
    // the later annex at their authored prices. The loop is bounded by production
    // timing plus a price-derived accumulation horizon, so every world ends either in
    // the completed diversification chain or an honest "another season" ending.
    // PRESS notices the reopened second score at the same canonical minute every narrative
    // branch does, while it is still standing down. The branch then deliberately schedules
    // nothing on it: the discipline that protects the open case is also an opportunity cost.
    if !metrics.second_opportunity_discovered {
        let discovery_at = scenario.timeline.second_opportunity_discovery_at;
        if scenario.state.now() < discovery_at {
            run_until(scenario, discovery_at, narrative, metrics)?;
        }
        discover_second_opportunity(scenario, narrative, metrics)?;
        if narrative {
            println!(
                "[DECIDE]  The second score is real, but {police_name} is still developing the case. Leadership declines another street job and lets the opportunity lapse. The home racket stays open: we accept its ongoing vice risk to keep earning."
            );
        }
    }
    // Once the matched observation window has closed, standing down no longer means
    // sitting on idle capital. Each day the organization launders the racket's take
    // through its front's books, and as soon as those accounted funds cover the
    // venue's authored price it buys the independent harbor club outright through
    // the canonical acquisition path - dirty money cannot buy legitimacy, so the
    // clean-money war chest gates the diversification. Owning the venue is what
    // lets the delegated expansion establish a second racket there, outside Central
    // Precinct's jurisdiction. Real agency during the wait, not a time skip.
    let matched_boundary = SimTime::from_minutes(
        metrics
            .matched_financial_boundary_minute
            .expect("narrative sessions always record their matched financial boundary"),
    );
    if scenario.state.now() < matched_boundary {
        run_until(scenario, matched_boundary, narrative, metrics)?;
    }
    if narrative {
        println!(
            "[DECIDE]  Standing down does not mean standing still: build clean money day by day, then buy the harbor club and open a second book the home case cannot touch."
        );
    }
    // Bounded daily capital reviews combine legitimate owner draws with surplus
    // street-cash laundering. Concealed reserves stay concealed until capitalization;
    // no fake conversion is needed to make legitimate trade useful. Case status still
    // comes only from the contact. Routine accounting is summarized by the heartbeat.
    let mut day_at = scenario.state.now();
    let mut stand_down = StandDownState::new(till_is_concealed(scenario));
    metrics.enterprise_till_concealed = Some(stand_down.till_concealed);
    if narrative && stand_down.till_concealed {
        println!(
            "[DECIDE]  The racket's concealed reserve cannot be laundered directly through {}. Leave it concealed; withdraw the front's legitimate profits to buy the harbor venue instead. The reserve can still capitalize its racket.",
            scenario
                .state
                .world()
                .get_business(scenario.front)
                .expect("laundering front must persist")
                .name(),
        );
    }
    let stand_down_days = stand_down_day_bound(scenario);
    for _ in 0..stand_down_days {
        if scenario.state.now() < day_at {
            run_until(scenario, day_at, narrative, metrics)?;
        }
        run_daily_capital_management(scenario, police_name, narrative, metrics, &mut stand_down)?;
        let read = poll_case_activity(scenario, burglary, narrative, metrics)?;
        // The file is shelved: street work is safe again, and finding the missing
        // specialist outranks real estate. A boss hunts for his man the moment the
        // district quiets, not after the war chest is finished.
        if metrics.cold_case_confirmed == Some(true)
            && !stand_down.personnel_recovered
            && metrics.defector.is_some()
        {
            stand_down.personnel_recovered = true;
            let member = metrics
                .defector
                .expect("a departure record names the missing member");
            if narrative {
                let member_name = scenario
                    .state
                    .world()
                    .get_character(member)
                    .expect("departed member must persist")
                    .name();
                println!(
                    "[DECIDE]  The file is shelved and street work is safe again. Before the books absorb leadership's whole attention: find where {member_name} landed, and make the one appeal his history with the family earns."
                );
            }
            super::run_personnel_recovery(scenario, narrative, metrics)?;
            super::restore_defector_reporting_line(scenario, narrative, metrics)?;
        }
        govern_home_racket_posture(scenario, narrative, metrics, &mut stand_down)?;
        narrate_stand_down_heartbeat(scenario, &read, narrative, metrics, &mut stand_down);
        if stand_down_wait_is_complete(metrics, &stand_down) {
            break;
        }
        day_at = day_at
            + SimDuration::from_minutes(
                u32::try_from(campaign_day_minutes)
                    .expect("authored campaign day must fit the duration type"),
            );
    }
    if narrative && metrics.cold_case_confirmed.is_none() {
        println!("[VERIFY]  The channel never produced a dependable read on the case's activity.");
    }
    if narrative
        && metrics.cold_case_confirmed == Some(true)
        && !stand_down_wait_is_complete(metrics, &stand_down)
    {
        println!(
            "[DECIDE]  The file cooled but the war chest never carried the full chain of prices; diversification waits for another season of clean books."
        );
    }
    settle_expansion_proof(scenario, campaign_day_minutes, narrative, metrics)
}

fn poll_case_activity(
    scenario: &mut Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<(bool, String)>, Box<dyn Error>> {
    // Leadership asks its standing contact every day, the way a player checks a live
    // threat: through the channel, not the calendar. The query only produces a fresh
    // disclosure when the institution actually has new word (an active read early, the
    // shelved read once the file goes cold); otherwise it returns nothing and the
    // organization keeps new street jobs on hold based on the last thing it heard.
    let read = read_police_contact(scenario, EntityRef::Operation(burglary), narrative, metrics)?;
    if matches!(read, Some((false, _))) {
        metrics.cold_case_confirmed = Some(true);
        if narrative {
            println!(
                "[CASE UPDATE] The channel confirms the burglary file is shelved. This clears that file only, not the district: any racket surcharge or manager warning about enforcement attention remains a separate reason for caution."
            );
        }
    }
    Ok(read)
}

fn narrate_stand_down_heartbeat(
    scenario: &Scenario,
    read: &Option<(bool, String)>,
    narrative: bool,
    metrics: &RunMetrics,
    stand_down: &mut StandDownState,
) {
    let should_heartbeat = narrative
        && stand_down.capital_review_days > 1
        // Heartbeat is a governance summary, not a daily log: report only fresh channel
        // news, the purchase/expansion beats (narrated at their own sites), or a periodic
        // pulse every fourth review so a multi-week wait reads as stewardship rather than
        // spam. Routine enterprise/front settlements already narrate above. The pulse
        // continues after the file cools because the war chest, not the case, is then the
        // only remaining gate.
        && (read.is_some() || stand_down.capital_review_days.is_multiple_of(4));
    if !should_heartbeat {
        return;
    }
    let accounted = scenario
        .state
        .finance()
        .get_account(scenario.accounted_funds)
        .expect("accounted-funds account must persist")
        .balance()
        .cents();
    // Both sides of the money loop, from books leadership actually holds: washed money
    // accumulating toward the next purchase, and street cash still waiting its turn.
    let street_cents: i64 = scenario
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(scenario.player))
        .filter(|account| {
            matches!(
                account.kind(),
                AccountKind::StreetCash | AccountKind::ConcealedCash
            )
        })
        .map(|account| account.balance().cents().max(0))
        .sum();
    let channel_line = match (read, metrics.cold_case_confirmed) {
        (Some((true, _)), _) => {
            "the case is still developing - no new street jobs, racket still open"
        }
        (Some((false, _)), _) => "the channel confirms the case has cooled",
        (None, Some(true)) => "the file is shelved; the war chest is the only gate now",
        // Leadership cannot know when the file will go cold; until the channel says
        // otherwise the last confirmed read stands and new street jobs stay on hold.
        (None, _) => "no fresh word - no new street jobs, racket still open",
    };
    // The next capital target on the empire ladder: the harbor club until it is owned,
    // then the lapsed annex score as income property, then the racing-wire office whose
    // ownership unlocks off-track bookmaking. Once the whole ladder is owned there is no
    // next purchase and the heartbeat says so. Prices come from the authored registry.
    let next_purchase = if !metrics.front_acquired {
        let price = scenario
            .registry
            .get_business(crimocracy::world::BusinessKind::Hospitality)
            .economics()
            .acquisition_cost()
            .cents();
        Some(("the harbor club", price))
    } else if !metrics.annex_acquired {
        let price = scenario
            .registry
            .get_business(
                scenario
                    .state
                    .world()
                    .get_business(scenario.alternate_target)
                    .expect("annex target must persist")
                    .kind(),
            )
            .economics()
            .acquisition_cost()
            .cents();
        Some(("the annex front", price))
    } else if !metrics.wire_acquired {
        let price = scenario
            .registry
            .get_business(
                scenario
                    .state
                    .world()
                    .get_business(scenario.wire_front)
                    .expect("wire office must persist")
                    .kind(),
            )
            .economics()
            .acquisition_cost()
            .cents();
        Some(("the racing-wire office", price))
    } else {
        None
    };
    let Some((target_name, target_price)) = next_purchase else {
        println!(
            "[WAIT] {}: {}; {} capital review(s) so far - clean books at {}, street liquidity {}. The purchase ladder is complete: harbor club, annex, and wire office all owned; the books now earn faster than the organization spends.",
            stamp(scenario.state.now().as_minutes()),
            channel_line,
            stand_down.capital_review_days,
            format_cents(accounted),
            format_cents(street_cents),
        );
        stand_down.last_heartbeat = Some((
            stand_down.capital_review_days,
            accounted,
            metrics.acquisition_spent_cents,
        ));
        return;
    };
    let gap = (target_price - accounted).max(0);
    // Pace from the last pulse, reset by any purchase so a spend never reads as a
    // negative income day. This is the same arithmetic a boss does on the books.
    let mut pace_note = String::new();
    if let Some((last_reviews, last_accounted, last_spend)) = stand_down.last_heartbeat
        && last_spend == metrics.acquisition_spent_cents
        && stand_down.capital_review_days > last_reviews
    {
        let reviews_elapsed = stand_down.capital_review_days - last_reviews;
        let pace = (accounted - last_accounted) / i64::from(reviews_elapsed);
        if pace > 0 {
            let days_out = (gap + pace - 1) / pace;
            pace_note = format!(
                "; clean books grow about {} per review, roughly {} review(s) away at this pace",
                format_cents(pace),
                days_out,
            );
        } else if gap > 0 {
            pace_note = format!(
                "; the books are not gaining on {} at this pace",
                target_name
            );
        }
    }
    stand_down.last_heartbeat = Some((
        stand_down.capital_review_days,
        accounted,
        metrics.acquisition_spent_cents,
    ));
    println!(
        "[WAIT] {}: {}; {} capital review(s) so far - clean books at {}, street liquidity {}. Next purchase: {} at {}, {} to go{}.",
        stamp(scenario.state.now().as_minutes()),
        channel_line,
        stand_down.capital_review_days,
        format_cents(accounted),
        format_cents(street_cents),
        target_name,
        format_cents(target_price),
        format_cents(gap),
        pace_note,
    );
}

/// The stand-down's diversification chain is complete when the harbor venue is owned,
/// its second-district book is open, and the lapsed annex score has been converted (it
/// stops being a live conversion target only once bought, because the opportunity
/// lapsed long before any purchase could happen). The empire ladder extends the same
/// logic: once the annex is owned, the chain is complete when the wire office is bought
/// AND the off-track book it unlocks is actually open - owning the wire without the book
/// it feeds is a half-finished rung, and the loop keeps reviewing until idle cash covers
/// the float. Completion still requires the cooled read: a boss keeps asking the channel
/// until the file is actually shelved, even when the money arrived first. When the war
/// chest cannot carry an authored price inside the bounded horizon, the loop simply
/// exhausts and the caller narrates the honest "another season" ending instead of
/// inventing a purchase.
fn stand_down_wait_is_complete(metrics: &RunMetrics, stand_down: &StandDownState) -> bool {
    let annex_resolved = metrics.annex_acquired || !metrics.second_opportunity_expired;
    let empire_resolved =
        !metrics.annex_acquired || (metrics.wire_acquired && metrics.bookmaking_established);
    metrics.front_acquired
        && metrics.expansion_established
        && annex_resolved
        && empire_resolved
        && stand_down.capital_review_days > 0
        && metrics.cold_case_confirmed == Some(true)
}

fn settle_expansion_proof(
    scenario: &mut Scenario,
    campaign_day_minutes: u64,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(expansion) = metrics
        .expansion_enterprise
        .filter(|_| metrics.front_acquired)
    else {
        return Ok(());
    };
    let day = SimDuration::from_minutes(
        u32::try_from(campaign_day_minutes)
            .expect("authored campaign day must fit the duration type"),
    );
    for _ in 0..10 {
        // The harbor book needs two settled cycles to show real ongoing earnings, a
        // purchased annex front needs its first clean settlement, and the off-track
        // book needs two of its own cycles so the closing view reports a wire-fed
        // market that actually earns, not a just-opened shell.
        let harbor_ready = scenario.state.enterprises().cycles_for(expansion).count() >= 2;
        let annex_ready = !metrics.annex_acquired
            || scenario
                .state
                .economy()
                .cycles_for(scenario.alternate_target)
                .count()
                >= 1;
        let book_ready = !metrics.bookmaking_established
            || metrics
                .bookmaking_enterprise
                .is_some_and(|book| scenario.state.enterprises().cycles_for(book).count() >= 2);
        if harbor_ready && annex_ready && book_ready {
            break;
        }
        run_until(scenario, scenario.state.now() + day, narrative, metrics)?;
    }
    Ok(())
}
