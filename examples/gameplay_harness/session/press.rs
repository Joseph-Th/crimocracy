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
        let cold_window_for_heat_check =
            scenario.registry.legal().cold_case_window().as_minutes() as u64;
        // Heat check lands well inside the authored cold window (about 1/36th of it, bounded
        // to [30,90] minutes) so the read always precedes any possible shelf no matter how
        // authors tune the window.
        let heat_check_delay = (cold_window_for_heat_check / 36).clamp(30, 90);
        let heat_check_at = SimTime::from_minutes(case_open_minute + heat_check_delay);
        if narrative {
            println!(
                "[DECIDE]  A case is open and the crew's field report is back. Hold back on further street work in {neighborhood_name} until leadership knows whether {police_name} is still developing it."
            );
            println!(
                "[DECIDE]  Watch {police_name} itself at {}, {} minutes after the case opened, to read whether detectives are still actively working the matter.",
                format_minute_of_day(heat_check_at.as_minutes()),
                heat_check_delay
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
                    "[VERIFY]  Detectives around {police_name} are still actively developing the case. Keep the district dark."
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
        if narrative && police_activity_information.is_some() {
            println!(
                "[INTERPRET] Quiet-word timing chosen from crew's patrol report to land inside the morning lull at {}.",
                format_minute_of_day(pressure_at.as_minutes())
            );
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

struct StandDownState {
    laundry_days: u32,
    last_absorbed: Option<i64>,
    final_purchase_beat: bool,
    till_concealed: bool,
}

impl StandDownState {
    fn new(till_concealed: bool) -> Self {
        Self {
            laundry_days: 0,
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
        .max(0);
    if launderable <= 0 {
        return Ok(None);
    }
    launder_through_front(scenario, narrative, metrics, cash_account, launderable)
}

fn record_daily_laundering(
    absorbed: Option<i64>,
    narrative: bool,
    stand_down: &mut StandDownState,
) {
    stand_down.laundry_days += 1;
    if narrative && absorbed != stand_down.last_absorbed {
        match absorbed {
            Some(gross) => println!(
                "[LAUNDER] The front's daily absorbable volume moved to {}.",
                format_cents(gross)
            ),
            None => println!(
                "[LAUNDER] The front's books absorbed nothing today; the volume waits as street cash."
            ),
        }
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
    let first_laundry = stand_down.laundry_days == 0;
    if stand_down.till_concealed {
        if metrics.front_acquired && !metrics.expansion_established {
            // Concealed-till worlds normally cannot acquire, but a pre-owned venue still needs
            // its canonical capitalization attempt.
            establish_harbor_expansion(scenario, narrative, metrics)?;
        }
        return Ok(());
    }

    if metrics.front_acquired && !metrics.expansion_established {
        // Capitalize before sweeping fresh income. Otherwise the same till funds laundering
        // first and can permanently starve the newly acquired book.
        establish_harbor_expansion(scenario, narrative, metrics)?;
        let absorbed = launder_enterprise_till(scenario, false, metrics)?;
        record_daily_laundering(absorbed, narrative, stand_down);
        return Ok(());
    }

    if narrative && first_laundry {
        println!(
            "[DECIDE]  Quiet streets are for the books: each day, put the whole till through the ledgers until they can carry the second-district purchase."
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
    let case_open_minute = metrics
        .case_open_minute
        .expect("press consequence arc requires the surfaced case-open minute");
    let cold_case_window = scenario.registry.legal().cold_case_window();
    // The shelf cannot land before the authored inactivity window plus the initial
    // evidence review that extends the case's activity instant; start daily polling
    // from there and keep polling until the channel carries the shelved read.
    let longest_work = scenario
        .registry
        .get_investigation_work(InvestigationWorkKind::EvidenceReview)
        .duration();
    let poll_at = SimTime::from_minutes(
        case_open_minute
            + u64::from(cold_case_window.as_minutes())
            + u64::from(longest_work.as_minutes()),
    );
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
                "[DECIDE]  The second score is real, but {police_name} is still developing the case. Leadership holds the district dark and takes nothing; the opportunity will be allowed to lapse."
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
    // Bounded daily loop: the authored cold-case decay guarantees a deterministic
    // shelf, and laundering accumulates accounted funds at the front's authored
    // pace, so both waits terminate. Every campaign day launders the racket's till
    // (keeping a working-capital floor) and retries the purchase once the books can
    // cover it; the precinct channel is only asked once the shelf could have landed.
    // A till authored as concealed cash stays exactly that: hidden money cannot
    // route through the front's ledgers without exposing it, so the beat leaves
    // it parked and launders only what sits in street cash. Narration follows
    // report discipline: full detail on the first beat and on every change, a
    // one-line heartbeat otherwise.
    let mut day_at = scenario.state.now();
    let mut stand_down = StandDownState::new(till_is_concealed(scenario));
    metrics.enterprise_till_concealed = Some(stand_down.till_concealed);
    if narrative && stand_down.till_concealed {
        println!(
            "[DECIDE]  The racket's till sits in concealed cash, and hidden money cannot touch {}'s ledgers without exposing it. With no clean money to build on, leadership holds to one job this arc: outlast the case.",
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
        let read = poll_case_activity(scenario, burglary, poll_at, narrative, metrics)?;
        narrate_stand_down_heartbeat(scenario, poll_at, &read, narrative, metrics, &stand_down);
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
    poll_at: SimTime,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<(bool, String)>, Box<dyn Error>> {
    if scenario.state.now() < poll_at {
        return Ok(None);
    }
    let read = read_police_contact(scenario, EntityRef::Operation(burglary), narrative, metrics)?;
    if matches!(read, Some((false, _))) {
        metrics.cold_case_confirmed = Some(true);
        if narrative {
            println!(
                "[CONSEQUENCE RESOLVED] The channel confirms the precinct shelved the case. The standing-down worked: the organization absorbed the exposure, kept the district quiet, and outlasted the investigation without touching hidden case state."
            );
        }
    }
    Ok(read)
}

fn narrate_stand_down_heartbeat(
    scenario: &Scenario,
    poll_at: SimTime,
    read: &Option<(bool, String)>,
    narrative: bool,
    metrics: &RunMetrics,
    stand_down: &StandDownState,
) {
    let should_heartbeat = narrative
        && (stand_down.laundry_days > 1 || stand_down.till_concealed)
        && metrics.cold_case_confirmed.is_none()
        && (read.is_some() || stand_down.laundry_days.is_multiple_of(2));
    if !should_heartbeat {
        return;
    }
    let accounted = scenario
        .state
        .finance()
        .get_account(scenario.accounted_funds)
        .expect("accounted-funds account must persist")
        .balance();
    let channel_line = match read {
        Some((true, _)) => "the channel still reads the case as actively developing",
        Some((false, _)) => "the channel confirms the case has cooled",
        None if scenario.state.now() < poll_at => {
            "the channel cannot yet know - the case cannot have shelved"
        }
        _ => "the channel has nothing fresh to share yet",
    };
    println!(
        "[WAIT] {}: {}; {} laundry day(s) so far, accounted books at {}.",
        stamp(scenario.state.now().as_minutes()),
        channel_line,
        stand_down.laundry_days,
        format_cents(accounted.cents()),
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
    if metrics.front_acquired || stand_down.till_concealed {
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
