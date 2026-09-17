//! PRESS strategy response arc built only from player-visible information and canonical game APIs.

use super::*;

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
        // days of inactivity to go cold, so a next-morning read always precedes any possible
        // shelf; the check never overlaps earlier scout work because it follows the clock.
        let heat_check_delay = 90_u64;
        let heat_check_at = SimTime::from_minutes(case_open_minute + heat_check_delay)
            .max(scenario.state.now() + SimDuration::from_minutes(1));
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
                    "[VERIFY]  Detectives around {police_name} are still actively developing the case. Stop new street jobs; the home racket remains open, earning income and risking further vice attention."
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
    metrics: &RunMetrics,
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
        if narrative {
            println!(
                "[DECIDE]  The after-action says the job was witnessed. Everyone knows who keeps {}: do not follow the failed score immediately, then send Carlo with one quiet word and accept that the follow-up carries its own exposure risk.",
                scenario
                    .state
                    .world()
                    .get_business(scenario.target)
                    .expect("target business must persist")
                    .name(),
            );
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
            // Witness interviews typically land 2-3h after intake. When the organization has a
            // typed patrol pattern, that evidence is binding: if no safe window exists before the
            // interview horizon, fail the treatment instead of discarding known risk and inventing
            // a convenient fallback time.
            let latest_start = SimTime::from_minutes(case_open_minute + 180);
            choose_safe_start_from_patrol_signal(
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
                    "[INTERPRET] No patrol pattern is held, so the quiet-word time at {} is a blind guess inside a watched district, not a patrol-safe plan. If a response arrives, the abort is the lesson: blind counter-play in a hot district gambles.",
                    format_day_minute(pressure_at.as_minutes())
                );
            }
        }
        let witness = scenario.target_owner;
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

    #[test]
    fn laundering_preserves_reserve_even_when_capacity_exceeds_surplus() {
        use crimocracy::finance::finance_system::validate_record_transaction;
        use crimocracy::finance::{LedgerPosting, LedgerTransactionDraft};
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
            SimTime::from_minutes(1_440),
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
            SimTime::from_minutes(1_440),
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
    fn concealed_reserve_does_not_prevent_legitimate_profit_financed_expansion() {
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
        assert_eq!(metrics.laundered_gross_cents, 0);
        assert!(metrics.business_profits_swept_cents >= metrics.acquisition_spent_cents);
        assert!(metrics.acquisition_rejections > 0);
        assert!(metrics.front_acquired && metrics.expansion_established);
        metrics.primary_narrative_set = true;
        validate_run_metrics(&metrics, true).unwrap();
        validate_press_expansion_evidence(&metrics).unwrap();
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
        // Actual daily settlement and the live launder -> buy -> capitalize policy,
        // not a synthetic balance or a separate test implementation of that policy.
        for day in 1..=10 {
            run_until(
                &mut scenario,
                SimTime::from_minutes(day * 1_440),
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
        panic!("settled books never funded the purchase");
    }
}

struct StandDownState {
    capital_review_days: u32,
    last_absorbed: Option<i64>,
    final_purchase_beat: bool,
    till_concealed: bool,
}

impl StandDownState {
    fn new(till_concealed: bool) -> Self {
        Self {
            capital_review_days: 0,
            last_absorbed: None,
            final_purchase_beat: false,
            till_concealed,
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

fn launder_enterprise_till(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<i64>, Box<dyn Error>> {
    if till_is_concealed(scenario) {
        return Ok(None);
    }
    let cash_account = scenario
        .state
        .enterprises()
        .get_enterprise(scenario.enterprise)
        .expect("canal enterprise must persist")
        .cash_account();
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
    launder_through_front(scenario, narrative, metrics, cash_account, launderable)
}

/// Withdraw only settled legitimate earnings and front fees, never the opening
/// capital. The production sweep checks real liquidity and ownership; this policy
/// caps the draw to earned surplus so profitable books remain operating assets.
fn sweep_front_profits(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    use crimocracy::economy::business_economy_system::{
        BusinessProfitSweepDraft, validate_sweep_business_profits,
    };
    let economy = scenario
        .state
        .economy()
        .get_business_economy(scenario.front)
        .expect("owned front economy persists");
    let earned: i64 = scenario
        .state
        .economy()
        .cycles_for(scenario.front)
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
    let amount = (earned + metrics.launder_fee_cents - metrics.business_profits_swept_cents)
        .max(0)
        .min(available);
    if amount == 0 {
        return Ok(());
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
            business: scenario.front,
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
    metrics.business_profits_swept_cents += after - before;
    if narrative {
        println!(
            "[OWNER DRAW] Withdraw {} of earned front profits into accounted funds. Legitimate trade can finance the purchase too; opening capital stays in the business.",
            format_cents(after - before)
        );
    }
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
    sweep_front_profits(scenario, narrative && first_laundry, metrics)?;

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
    let absorbed = launder_enterprise_till(scenario, narrative && first_laundry, metrics)?;
    record_daily_laundering(absorbed, narrative && !first_laundry, stand_down);

    if !metrics.front_acquired && acquire_harbor_front(scenario, narrative, metrics)? {
        establish_harbor_expansion(scenario, narrative, metrics)?;
        if metrics.expansion_established && narrative {
            println!(
                "[DECIDE]  Standing down does not mean going deaf: once a day, {police_name}-channel asks only - has anything moved on the case?"
            );
        }
    }
    Ok(())
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
    // shelved read. The loop is bounded (40 days) well past any authored cold window,
    // so both waits terminate through production disclosures.
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
    for _ in 0..40 {
        if scenario.state.now() < day_at {
            run_until(scenario, day_at, narrative, metrics)?;
        }
        run_daily_capital_management(scenario, police_name, narrative, metrics, &mut stand_down)?;
        let read = poll_case_activity(scenario, burglary, narrative, metrics)?;
        narrate_stand_down_heartbeat(scenario, &read, narrative, metrics, &stand_down);
        if cold_case_wait_is_complete(narrative, metrics, &mut stand_down) {
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
                "[CASE UPDATE] The channel confirms the burglary file is shelved. This clears that file only, not the district: any racket surcharge or manager warning about vice attention remains a separate reason for caution."
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
    stand_down: &StandDownState,
) {
    let should_heartbeat = narrative
        && (stand_down.capital_review_days > 1 || stand_down.till_concealed)
        && metrics.cold_case_confirmed.is_none()
        && (read.is_some() || stand_down.capital_review_days.is_multiple_of(2));
    if !should_heartbeat {
        return;
    }
    let accounted = scenario
        .state
        .finance()
        .get_account(scenario.accounted_funds)
        .expect("accounted-funds account must persist")
        .balance();
    // Both sides of the money loop, from books leadership actually holds: washed
    // money accumulating toward the harbor price, and street cash still waiting.
    let till_cents = scenario
        .state
        .enterprises()
        .get_enterprise(scenario.enterprise)
        .and_then(|record| scenario.state.finance().get_account(record.cash_account()))
        .map(|account| account.balance().cents())
        .unwrap_or_default()
        .max(0);
    let channel_line = match read {
        Some((true, _)) => "the case is still developing - no new street jobs, racket still open",
        Some((false, _)) => "the channel confirms the case has cooled",
        // Leadership cannot know when the file will go cold; until the channel says
        // otherwise the last confirmed read stands and new street jobs stay on hold.
        None => "no fresh word - no new street jobs, racket still open",
    };
    println!(
        "[WAIT] {}: {}; {} capital review(s) so far, accounted books at {}, racket reserve at {}.",
        stamp(scenario.state.now().as_minutes()),
        channel_line,
        stand_down.capital_review_days,
        format_cents(accounted.cents()),
        format_cents(till_cents),
    );
}

fn cold_case_wait_is_complete(
    narrative: bool,
    metrics: &RunMetrics,
    stand_down: &mut StandDownState,
) -> bool {
    if metrics.cold_case_confirmed != Some(true) {
        return false;
    }
    if metrics.front_acquired {
        return true;
    }
    if stand_down.final_purchase_beat {
        if narrative {
            println!(
                "[DECIDE]  The case is cooled but the books never carried the price; diversification waits for another season."
            );
        }
        return true;
    }
    stand_down.final_purchase_beat = true;
    false
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
        if scenario.state.enterprises().cycles_for(expansion).count() >= 2 {
            break;
        }
        run_until(scenario, scenario.state.now() + day, narrative, metrics)?;
    }
    Ok(())
}
