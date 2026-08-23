//! Session flow: play_session, the second act, defector trail, and terminal-state loop helpers.

use crimocracy::contacts::contact_system::{
    find_pending_disclosure_sources, validate_contact_disclosure,
};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{OperationId, OrganizationId};
use crimocracy::core::simulation::run_tick;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::finance::Money;
use crimocracy::intelligence::{InformationTopic, KnowledgeHolder};
use crimocracy::legal::InvestigationWorkKind;
use crimocracy::operations::property_disposition::{
    validate_dispose_property, PropertyDispositionDraft,
};
use crimocracy::operations::{OperationAbortCause, OperationKind, OperationStatus};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
};
use crimocracy::opportunities::OperationOpportunityDraft;
use crimocracy::recruitment::recruitment_system::validate_recruitment_attempt;
use crimocracy::recruitment::{RecruitmentApproach, RecruitmentDraft, RecruitmentOutcome};
use crimocracy::registry::Registry;
use crimocracy::reports::ReportKind;
use crimocracy::world::territory_influence::resolve_neighborhood_influence;
use std::collections::BTreeSet;
use std::error::Error;

use crate::*;

/// The standing police-contact channel, used the way a player uses it: ask the handler what the
/// contact can tell us, then hear one fresh item through the canonical disclosure path. Returns
/// the parsed case-activity sightline when the disclosure carried one. The acting policy never
/// enumerates hidden knowledge — `find_pending_disclosure_sources` exposes only what the channel
/// itself offers, and everything the organization learns arrives as a derived information record.
pub fn read_police_contact(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<bool>, Box<dyn Error>> {
    let sources = find_pending_disclosure_sources(&scenario.state, scenario.police_contact);
    let Some(source) = sources.into_iter().find(|source| {
        scenario
            .state
            .intelligence()
            .get_information(*source)
            .is_some_and(|information| information.topic() == InformationTopic::LegalActivity)
    }) else {
        return Ok(None);
    };
    let boss_name = scenario
        .state
        .world()
        .get_character(scenario.boss)
        .expect("boss must persist")
        .name()
        .to_owned();
    let detective_name = scenario
        .state
        .world()
        .get_character(scenario.detective)
        .expect("detective must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[CONTACT] {boss_name} quietly asks {detective_name} what the precinct is doing about it."
        );
    }
    let disclosure = validate_contact_disclosure(&scenario.state, scenario.police_contact, source)?
        .commit(&mut scenario.state)?;
    metrics.contact_reads += 1;
    let disclosed = scenario
        .state
        .contacts()
        .get_disclosure(disclosure)
        .expect("committed contact disclosure must be queryable")
        .disclosed_information();
    let record = scenario
        .state
        .intelligence()
        .get_information(disclosed)
        .expect("disclosed contact information must persist");
    let read = observe_authority_case_sightline_summary(record.summary());
    if narrative {
        println!(
            "[LEARN]   {:?} / {:?}: {}",
            record.reliability(),
            record.specificity(),
            record.summary()
        );
    }
    Ok(read)
}

pub fn play_session(
    registry: &Registry,
    strategy: Strategy,
    profile: ScenarioProfile,
    seed: u64,
    narrative: bool,
    continue_for_financial_day: bool,
) -> Result<RunMetrics, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, profile)?;
    let mut metrics = RunMetrics {
        strategy: Some(strategy),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };
    // Matched financial boundary: narrative sessions observe two campaign days and batch
    // sessions observe one (mirroring the observation windows below). Every branch crosses
    // this minute before its arc extends, so a snapshot here compares identical windows.
    let campaign_day_minutes = u64::from(
        registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes(),
    );
    metrics.matched_financial_boundary_minute =
        Some(campaign_day_minutes * if narrative { 2 } else { 1 });

    if narrative {
        println!(
            "[FIXTURE] {} authored variation selected by simulation seed.",
            scenario.variation.label(),
        );
        print_starting_player_view(&scenario);
    }

    let opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: scenario.variation.opportunity_summary().to_owned(),
            valid_until: Some(scenario.timeline.initial_opportunity_valid_until),
        },
    )?
    .commit(&mut scenario.state)?;

    if narrative {
        let record = scenario
            .state
            .opportunities()
            .get_opportunity(opportunity)
            .expect("committed opportunity must be queryable");
        println!("\n[OBSERVE] Opportunity: {}", record.summary());
        println!(
            "          Source: {}",
            scenario
                .state
                .intelligence()
                .get_information(scenario.opportunity_information)
                .expect("starting information must exist")
                .summary()
        );
    }

    let mut burglary_intelligence = BTreeSet::from([scenario.opportunity_information]);
    let mut learned_patrol_summary = None;
    if strategy == Strategy::Recon {
        if narrative {
            println!(
                "[DECIDE]  Order surveillance before committing the burglary. The goal is to learn venue access and police rhythm."
            );
        }
        let surveillance = authorize_surveillance(&mut scenario)?;
        run_until_operation_terminal(&mut scenario, surveillance, narrative, &mut metrics)?;
        let resolution = scenario
            .state
            .operations()
            .get_operation(surveillance)
            .expect("surveillance must remain queryable")
            .resolution()
            .expect("completed surveillance must have a resolution");
        metrics.discovered_surveillance_information = resolution.discovered_information().len();
        for information in resolution.discovered_information() {
            let record = scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("surveillance information must persist");
            if narrative {
                println!(
                    "[LEARN]   {:?} / {:?}: {}",
                    record.reliability(),
                    record.specificity(),
                    record.summary()
                );
            }
            if record.topic() == InformationTopic::PoliceActivity {
                learned_patrol_summary = Some(record.summary().to_owned());
            }
            // Every discovered record is already organization-held and target-relevant by the
            // surveillance contract. Carry all of it into the next plan so the harness tests
            // the same information-selection boundary a player would use.
            burglary_intelligence.insert(*information);
        }
    }

    let scheduled_for = match strategy {
        Strategy::Rush | Strategy::Press => scenario.timeline.initial_burglary_at,
        Strategy::Recon => {
            let patrol_summary = learned_patrol_summary.as_deref().ok_or(
                "recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
            )?;
            let duration = scenario
                .registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration();
            let chosen = choose_safe_start_from_patrol_report(
                scenario.state.now(),
                patrol_summary,
                duration,
                SimDuration::from_minutes(60),
                scenario.timeline.initial_opportunity_valid_until,
            )?;
            if narrative {
                println!(
                    "[INTERPRET] Parsed the reported recurring patrol windows and chose minute {} so the authored burglary window stays outside them with a one-hour uncertainty buffer.",
                    chosen.as_minutes()
                );
            }
            chosen
        }
    };
    if narrative && matches!(strategy, Strategy::Rush | Strategy::Press) {
        let clock = format_minute_of_day(scheduled_for.as_minutes());
        match strategy {
            Strategy::Rush => println!(
                "[DECIDE]  Move immediately on the opportunity at {clock}, using only the original street information."
            ),
            Strategy::Press => println!(
                "[DECIDE]  Hit {} at {clock} and press on through a police response unless leadership later orders otherwise.",
                scenario.variation.target_name(),
            ),
            Strategy::Recon => unreachable!("recon narrates its own planning decision"),
        }
    }
    if scenario.state.now() >= scheduled_for {
        return Err(format!(
            "scenario preparation reached minute {} before burglary schedule {}",
            scenario.state.now().as_minutes(),
            scheduled_for.as_minutes()
        )
        .into());
    }
    let target = scenario.target;
    let entry_specialist = scenario.burglar;
    let title = format!("{} burglary", scenario.variation.target_name());
    let burglary = authorize_burglary(
        &mut scenario,
        strategy,
        target,
        &title,
        scheduled_for,
        burglary_intelligence,
        entry_specialist,
    )?;
    validate_convert_opportunity(&scenario.state, opportunity, burglary)?
        .commit(&mut scenario.state)?;
    metrics.burglary = Some(burglary);

    if narrative {
        println!(
            "[COMMIT]  Burglary authorized for minute {} with {:?} approach and {} planning information item(s).",
            scheduled_for.as_minutes(),
            scenario
                .state
                .operations()
                .get_operation(burglary)
                .expect("burglary must exist")
                .approach(),
            scenario
                .state
                .operations()
                .get_operation(burglary)
                .expect("burglary must exist")
                .intelligence()
                .len(),
        );
    }
    metrics.planning_information_count = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must exist")
        .intelligence()
        .len();
    metrics.planning_information_topics = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must exist")
        .intelligence()
        .iter()
        .map(|information| {
            scenario
                .state
                .intelligence()
                .get_information(*information)
                .expect("selected planning information must persist")
                .topic()
        })
        .collect();
    if narrative {
        print_planning_inputs(&scenario, burglary);
    }

    run_until_operation_terminal(&mut scenario, burglary, narrative, &mut metrics)?;
    metrics.burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    let burglary_record = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must remain queryable");
    metrics.aborted = burglary_record.status() == OperationStatus::Aborted;
    if metrics.aborted {
        let abort = burglary_record
            .abort_record()
            .expect("aborted burglary must persist its abort provenance");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    }
    if let Some(resolution) = burglary_record.resolution() {
        metrics.outcome = Some(resolution.objective_outcome());
        metrics.exposure_score = Some(resolution.exposure().score());
        metrics.exposure_level = Some(resolution.exposure().level());
        metrics.investigation_created = resolution.exposure().investigation().is_some();
        metrics.evidence_count = resolution.exposure().evidence().len();
        // The case ID itself is developer-audit-only; the player only ever sees the surfaced
        // legal-activity knowledge and their own later surveillance observations.
        scenario.investigation = resolution.exposure().investigation();
        metrics.case_open_minute = scenario
            .investigation
            .map(|_| scenario.state.now().as_minutes());
        metrics.burglary_information_quality =
            Some(resolution.factors().intelligence_quality().value());
        metrics.property_acquired_value_cents = resolution
            .property_proceeds()
            .map(|proceeds| proceeds.estimated_value().cents());

        if narrative {
            let report = scenario
                .state
                .reports()
                .get_report(resolution.after_action_report())
                .expect("after-action report must persist");
            print_report("AFTER-ACTION", report, &scenario);
            println!(
                "[CONSEQUENCE] Exposure {:?} (score {}); police case created: {}; evidence records: {}.",
                resolution.exposure().level(),
                resolution.exposure().score(),
                resolution.exposure().investigation().is_some(),
                resolution.exposure().evidence().len(),
            );
            print_resolution_factors(resolution);
            if let Some(proceeds) = resolution.property_proceeds() {
                println!(
                    "[PROCEEDS] Held property estimated at {}. This is organizational value, not liquid cash.",
                    format_cents(proceeds.estimated_value().cents())
                );
            }
        }
    } else if narrative {
        let abort = burglary_record
            .abort_record()
            .expect("aborted burglary must persist its abort provenance");
        println!(
            "[ABORT] phase {:?}, cause {:?}; objective resolution was not completed.",
            abort.phase(),
            abort.cause(),
        );
        if let Some(artifacts) = abort.artifacts() {
            let report = scenario
                .state
                .reports()
                .get_report(artifacts.report())
                .expect("started abort must persist its after-action report");
            print_report("ABORT REPORT", report, &scenario);
        }
        if strategy == Strategy::Rush && narrative {
            println!(
                "[DECIDE]  The standing abort protected the crew. Walk away from {} tonight; the police rhythm there is not beaten by speed alone.",
                scenario
                    .state
                    .world()
                    .get_neighborhood(scenario.neighborhood)
                    .expect("neighborhood must persist")
                    .name()
            );
        }
    }

    // A standing abort is not a wasted night: the abort artifacts carry the organization's
    // own debrief-derived read on how the responding authority moved in this district. That
    // record is what lets act 2 plan against the patrol rhythm without fresh surveillance or
    // hidden-state reads.
    if metrics.aborted {
        if let Some(OperationAbortCause::PoliceArrival(_)) = metrics.abort_cause {
            let debrief_information = burglary_record
                .abort_record()
                .and_then(|abort| abort.artifacts())
                .and_then(|artifacts| artifacts.police_activity_information());
            if let Some(information) = debrief_information {
                metrics.player_police_activity_information =
                    metrics.player_police_activity_information.saturating_add(1);
                metrics.debrief_patrol_information.push(information);
                if narrative {
                    let record = scenario
                        .state
                        .intelligence()
                        .get_information(information)
                        .expect("debrief police-activity knowledge must persist");
                    println!(
                        "[DECIDE]  Debrief the crew before anyone plans around that response; what they saw becomes organizational knowledge."
                    );
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
            }
        }
    }

    let acquired_property_value = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.property_proceeds())
        .map(|proceeds| proceeds.estimated_value());
    if let Some(estimated_value) = acquired_property_value {
        if narrative {
            println!(
                "[DECIDE]  Move the acquired property through {} rather than leave it as held inventory.",
                scenario
                    .state
                    .world()
                    .get_business(scenario.resale_venue)
                    .expect("resale venue must persist")
                    .name(),
            );
        }
        let disposition = validate_dispose_property(
            scenario.registry,
            &scenario.state,
            PropertyDispositionDraft {
                operation: burglary,
                venue: scenario.resale_venue,
                cash_account: scenario.liquidation_cash,
                settlement_account: scenario.liquidation_settlement,
            },
        )?
        .commit(&mut scenario.state)?;
        metrics.property_realized_cash_cents = Some(disposition.realized_value.cents());
        metrics.liquidation_minute = Some(scenario.state.now().as_minutes());
        if narrative {
            println!(
                "[LIQUIDATE] {} estimated property -> {} realized resale cash.",
                format_cents(estimated_value.cents()),
                format_cents(disposition.realized_value.cents())
            );
        }
    }

    metrics.player_legal_activity_information = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::LegalActivity,
        )
        .filter(|information| information.subject() == EntityRef::Operation(burglary))
        .count();
    if narrative {
        print_player_knowledge_gap(&scenario, burglary);
    }

    // The Press branch exercises a real player follow-up: the organization uses only the
    // surfaced legal-activity report and the crew's field report to authorize counter-surveillance
    // of the precinct itself. The investigation's evidence, lead, and internal ID stay hidden; the
    // follow-up reads only whether the authority is still visibly developing the known case.
    if strategy == Strategy::Press
        && metrics.player_legal_activity_information > 0
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
        // Heat check lands well inside the authored cold window: ~1/36th of the window
        // (â‰ˆ60m for the current 2160m window) bounded to [30,90] so it never drifts outside
        // if authors tune the window.
        let heat_check_delay = (cold_window_for_heat_check / 36).clamp(30, 90);
        let heat_check_at = SimTime::from_minutes(case_open_minute + heat_check_delay);
        if narrative {
            println!(
                "[DECIDE]  A case is open and the crew's field report is back. Hold back on further street work in {neighborhood_name} until leadership knows whether {police_name} is still developing it."
            );
            println!(
                "[DECIDE]  Watch {police_name} itself at {}, about one hour after the case opened, to read whether detectives are still actively working the matter.",
                format_minute_of_day(heat_check_at.as_minutes())
            );
        }
        metrics.counterintelligence_scheduled_at = Some(heat_check_at.as_minutes());
        let counterintelligence_title = format!("{police_name} case-heat check");
        let police = scenario.police;
        let counterintelligence = authorize_surveillance_target(
            &mut scenario,
            EntityRef::Organization(police),
            &counterintelligence_title,
            heat_check_at,
        )?;
        run_until_operation_terminal(&mut scenario, counterintelligence, narrative, &mut metrics)?;
        let operation = scenario
            .state
            .operations()
            .get_operation(counterintelligence)
            .expect("counterintelligence operation must persist");
        if let Some(resolution) = operation.resolution() {
            metrics.counterintelligence_outcome = Some(resolution.objective_outcome());
            metrics.counterintelligence_information = resolution.discovered_information().len();
            metrics.followup_case_active = observe_authority_case_sightline(&scenario, resolution);
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
        // The narrative session stands down but does not go deaf: once per campaign day the
        // organization asks its precinct contact what the institution knows. The lead
        // detective's own knowledge is production state — recorded when he took the case and
        // refreshed when the authority shelves it — so each new development arrives as a fresh,
        // disclosable record through the canonical channel. Batch sessions observe one day and
        // stop while the case is still hot, keeping the matched financial window intact.
        if narrative {
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
            let mut poll_at = SimTime::from_minutes(
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
                    run_until(&mut scenario, discovery_at, narrative, &mut metrics)?;
                }
                discover_second_opportunity(&mut scenario, narrative, &mut metrics)?;
                if narrative {
                    println!(
                        "[DECIDE]  The second score is real, but {police_name} is still developing the case. Leadership holds the district dark and takes nothing; the opportunity will be allowed to lapse."
                    );
                }
            }
            // Once the matched observation window has closed, standing down no longer means
            // sitting on idle capital: the organization diversifies into the quiet harbor
            // district, whose rackets pay no heat surcharge because Central Precinct's case
            // never touched them. This is real agency during the wait, not a time skip.
            let matched_boundary = SimTime::from_minutes(
                metrics
                    .matched_financial_boundary_minute
                    .expect("narrative sessions always record their matched financial boundary"),
            );
            if scenario.state.now() < matched_boundary {
                run_until(&mut scenario, matched_boundary, narrative, &mut metrics)?;
            }
            establish_harbor_expansion(&mut scenario, narrative, &mut metrics)?;
            if narrative {
                println!(
                    "[DECIDE]  Standing down does not mean going deaf: once a day, {police_name}-channel asks only — has anything moved on the case?"
                );
            }
            // Bounded polling loop: the authored cold-case decay guarantees a deterministic shelf,
            // so the poll terminates when the refreshed investigator knowledge becomes disclosable.
            for _ in 0..40 {
                if scenario.state.now() < poll_at {
                    run_until(&mut scenario, poll_at, narrative, &mut metrics)?;
                }
                let read = read_police_contact(&mut scenario, narrative, &mut metrics)?;
                match read {
                    Some(false) => {
                        metrics.cold_case_confirmed = Some(true);
                        println!(
                            "[CONSEQUENCE RESOLVED] The channel confirms the precinct shelved the case. The standing-down worked: the organization absorbed the exposure, kept the district quiet, and outlasted the investigation without touching hidden case state."
                        );
                        break;
                    }
                    Some(true) => {
                        println!(
                            "[VERIFY]  The channel says detectives are still actively developing the case; keep the district dark."
                        );
                    }
                    None => {}
                }
                poll_at = poll_at
                    + SimDuration::from_minutes(
                        u32::try_from(campaign_day_minutes)
                            .expect("authored campaign day must fit the duration type"),
                    );
            }
            if metrics.cold_case_confirmed.is_none() {
                println!(
                    "[VERIFY]  The channel never produced a dependable read on the case's activity."
                );
            }
        }
    }

    if continue_for_financial_day {
        // The authored autonomous-recruitment cadence defines the rivals' campaign day. Sessions
        // observe a whole number of those days so financial windows stay matched while session
        // timing tracks the authored content instead of hard-coded minutes.
        let campaign_day_minutes = u64::from(
            scenario
                .registry
                .recruitment()
                .autonomous_attempt_cadence()
                .as_minutes(),
        );
        let observation_end = if narrative {
            SimTime::from_minutes(campaign_day_minutes * 2)
        } else {
            SimTime::from_minutes(campaign_day_minutes)
        };
        // The rival autonomous-recruitment cadence fires once per campaign day. Narrative sessions
        // pause just past the first day boundary so an accepted defection is observable, let the
        // organization run its own defector watch, then finish the observation window.
        let recruitment_boundary = SimTime::from_minutes(campaign_day_minutes + 1);
        if narrative && observation_end > recruitment_boundary {
            run_until(&mut scenario, recruitment_boundary, narrative, &mut metrics)?;
        } else {
            run_until(&mut scenario, observation_end, narrative, &mut metrics)?;
        }
        // All narrative branches notice the reopened second score at the same canonical minute and
        // then each branch either works it (RUSH rebuild, RECON re-recon) or deliberately lets it
        // lapse as the price of standing down (PRESS, which already discovered it during the
        // cold-case wait above). Batch sessions keep the single-act window for performance.
        if narrative && !metrics.second_opportunity_discovered {
            let discovery_at = scenario.timeline.second_opportunity_discovery_at;
            if scenario.state.now() < discovery_at {
                run_until(&mut scenario, discovery_at, narrative, &mut metrics)?;
            }
            discover_second_opportunity(&mut scenario, narrative, &mut metrics)?;
        }
        if narrative && metrics.defector.is_some() && metrics.defector_trail_confirmed.is_none() {
            run_defector_trail(&mut scenario, narrative, &mut metrics)?;
        }
        // The trail's answer invites one more player move: a personal re-approach to the
        // defector through the canonical executive recruitment path. Its outcome is production
        // scoring, not authoring, and a refusal leaks the approach to the rival.
        if narrative && metrics.defector_trail_confirmed.is_some() {
            run_win_back_attempt(&mut scenario, narrative, &mut metrics)?;
        }
        if narrative {
            run_second_act(&mut scenario, strategy, narrative, &mut metrics)?;
        }
        if scenario.state.now() < observation_end {
            run_until(&mut scenario, observation_end, narrative, &mut metrics)?;
        }
        let financials = resolve_financial_view(&scenario)?;
        let mut financials = financials;
        financials.payroll_paid_cents = metrics.payroll_paid_cents;
        financials.payroll_short_cents = metrics.payroll_short_cents;
        metrics.legitimate_net_cents = Some(financials.legitimate_net_cents);
        metrics.enterprise_net_cents = Some(financials.enterprise_net_cents);
        // Raw audit evidence of delegated rival growth: active rackets each non-player
        // organization operates in the home district, derived through the canonical
        // territory-influence surface (acting policy never reads this).
        metrics.rival_home_enterprises =
            resolve_neighborhood_influence(&scenario.state, scenario.neighborhood)
                .expect("home-district influence should resolve")
                .standings
                .into_iter()
                .filter(|standing| standing.organization != scenario.player)
                .map(|standing| standing.active_enterprises)
                .sum();
        if let Some(expansion) = metrics.expansion_enterprise {
            metrics.expansion_net_cents = Some(
                scenario
                    .state
                    .enterprises()
                    .cycles_for(expansion)
                    .try_fold(Money::ZERO, |sum, cycle| sum.checked_add(cycle.net_cash()))
                    .expect("expansion enterprise totals must fit money range")
                    .cents(),
            );
        }
        if narrative {
            print_final_case_audit(&scenario, burglary);
            print_second_act_recap(&scenario, strategy, &metrics);
            print_financial_view(&scenario, financials);
            println!("\n[EXECUTIVE BRIEFS]");
            for report in scenario
                .state
                .reports()
                .reports_for(scenario.player)
                .filter(|report| report.kind() == ReportKind::ExecutiveBrief)
            {
                print_report_condensed("BRIEF", report);
            }
        }
    }

    metrics.player_report_count = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.kind() != ReportKind::ExecutiveBrief)
        .count();
    metrics.executive_brief_count = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.kind() == ReportKind::ExecutiveBrief)
        .count();

    Ok(metrics)
}

/// Executes the narrative act-2 beat per branch. RUSH rebuilds the crew and works the second score
/// in the morning lull; RECON re-invests in planning and works it inside a fresh patrol-safe
/// window; PRESS deliberately takes nothing and lets the discovered opportunity lapse.
pub fn run_second_act(
    scenario: &mut Scenario,
    strategy: Strategy,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(opportunity) = metrics.second_opportunity else {
        return Err("act 2 cannot run before the second opportunity is discovered".into());
    };
    let alternate_target = scenario.alternate_target;
    let neighborhood_name = scenario
        .state
        .world()
        .get_neighborhood(scenario.neighborhood)
        .expect("neighborhood must persist")
        .name()
        .to_owned();

    match strategy {
        Strategy::Rush => {
            let replacement = recruit_replacement(scenario, narrative, metrics)?;
            let scheduled_for = scenario.timeline.rush_second_act_at;
            let title = format!(
                "{} second-score burglary",
                scenario.variation.alternate_target_name()
            );
            // The rebuilt crew plans from the original street observation plus what the
            // debrief taught the organization about the district's police response.
            let mut intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
            intelligence.extend(metrics.debrief_patrol_information.iter().copied());
            metrics.second_act_planning_topics = intelligence
                .iter()
                .map(|information| {
                    scenario
                        .state
                        .intelligence()
                        .get_information(*information)
                        .expect("second-score planning information must persist")
                        .topic()
                })
                .collect();
            if narrative {
                println!(
                    "[DECIDE]  Rebuild is in hand. Work the second score on {} during the morning lull at {}, with the rebuilt crew, the original street observation, and the debriefed read on the district's response.",
                    scenario.variation.alternate_target_name(),
                    format_minute_of_day(scheduled_for.as_minutes()),
                );
            }
            let burglary = authorize_burglary(
                scenario,
                Strategy::Rush,
                alternate_target,
                &title,
                scheduled_for,
                intelligence,
                replacement,
            )?;
            validate_convert_opportunity(&scenario.state, opportunity, burglary)?
                .commit(&mut scenario.state)?;
            metrics.second_burglary = Some(burglary);
            run_until_operation_terminal(scenario, burglary, narrative, metrics)?;
            record_second_act_burglary_terminal(scenario, burglary, metrics);
            liquidate_second_act_property(scenario, burglary, narrative, metrics)?;
        }
        Strategy::Recon => {
            let title = format!(
                "{} second-score surveillance",
                scenario.variation.alternate_target_name()
            );
            if narrative {
                println!(
                    "[DECIDE]  Re-invest in planning: run fresh surveillance on {} before committing the second score, and pick the protected window from the new report.",
                    scenario.variation.alternate_target_name()
                );
            }
            let recon = authorize_surveillance_target(
                scenario,
                EntityRef::Business(alternate_target),
                &title,
                scenario.timeline.recon_second_act_surveillance_at,
            )?;
            run_until_operation_terminal(scenario, recon, narrative, metrics)?;
            let resolution = scenario
                .state
                .operations()
                .get_operation(recon)
                .expect("second-score surveillance must persist")
                .resolution()
                .expect("completed second-score surveillance must have a resolution");
            metrics.second_act_recon_information = resolution.discovered_information().len();
            // Casing carries risk both ways: if the surveillance itself drew a police case, the
            // organization knows it only through the surfaced after-action report. RECON's
            // answer is quieter than PRESS's precinct watch: ask the institutional contact it
            // keeps inside the precinct instead of putting more eyes on the street.
            let self_heat_investigation = resolution.exposure().investigation();
            metrics.self_heat_case_opened = self_heat_investigation.is_some();
            let mut burglary_intelligence =
                BTreeSet::from([scenario.alternate_opportunity_information]);
            let mut learned_patrol_summary = None;
            for information in resolution.discovered_information() {
                let record = scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("second-score surveillance information must persist");
                if narrative {
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
                if record.topic() == InformationTopic::PoliceActivity {
                    learned_patrol_summary = Some(record.summary().to_owned());
                }
                burglary_intelligence.insert(*information);
            }
            let patrol_summary = learned_patrol_summary.as_deref().ok_or(
                "second-score recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
            )?;
            let duration = scenario
                .registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration();
            let scheduled_for = choose_safe_start_from_patrol_report(
                scenario.state.now(),
                patrol_summary,
                duration,
                SimDuration::from_minutes(60),
                scenario.timeline.second_opportunity_valid_until,
            )?;
            if narrative {
                println!(
                    "[INTERPRET] Parsed the reported recurring patrol windows and chose {} so the second-score burglary stays outside them with a one-hour uncertainty buffer.",
                    format_minute_of_day(scheduled_for.as_minutes())
                );
            }
            let title = format!(
                "{} second-score burglary",
                scenario.variation.alternate_target_name()
            );
            let burglary = authorize_burglary(
                scenario,
                Strategy::Recon,
                alternate_target,
                &title,
                scheduled_for,
                burglary_intelligence,
                scenario.burglar,
            )?;
            metrics.second_act_planning_topics = scenario
                .state
                .operations()
                .get_operation(burglary)
                .expect("second-score burglary must remain queryable")
                .intelligence()
                .iter()
                .map(|information| {
                    scenario
                        .state
                        .intelligence()
                        .get_information(*information)
                        .expect("second-score planning information must persist")
                        .topic()
                })
                .collect();
            validate_convert_opportunity(&scenario.state, opportunity, burglary)?
                .commit(&mut scenario.state)?;
            metrics.second_burglary = Some(burglary);
            run_until_operation_terminal(scenario, burglary, narrative, metrics)?;
            record_second_act_burglary_terminal(scenario, burglary, metrics);
            liquidate_second_act_property(scenario, burglary, narrative, metrics)?;
            // Close the self-inflicted-heat loop through the contact channel: the after-action
            // on the casing reported an opened case, so leadership asks its precinct channel
            // what detectives are doing rather than surveilling the precinct again. The
            // detective's knowledge of his own case is production state — recorded when he took
            // the case as lead — and the acting decision, disclosure, and everything the
            // organization learns flow through the canonical contact and information paths.
            if let Some(_investigation) = self_heat_investigation {
                let neighborhood_name = scenario
                    .state
                    .world()
                    .get_neighborhood(scenario.neighborhood)
                    .expect("neighborhood must persist")
                    .name()
                    .to_owned();
                let police_name = scenario
                    .state
                    .world()
                    .get_organization(scenario.police)
                    .expect("police organization must persist")
                    .name()
                    .to_owned();
                if narrative {
                    println!(
                        "[DECIDE]  The after-action on our own casing says it drew attention: {police_name} opened a case out of that surveillance. Before anything else touches {neighborhood_name}, leadership uses the channel it keeps inside the precinct."
                    );
                }
                metrics.self_heat_case_active = read_police_contact(scenario, narrative, metrics)?;
                if narrative {
                    match metrics.self_heat_case_active {
                        Some(true) => println!(
                            "[VERIFY]  The channel confirms detectives are still developing the case our casing opened; {neighborhood_name} stays quiet past this window."
                        ),
                        Some(false) => println!(
                            "[VERIFY]  The channel says {police_name} has already shelved the case our casing opened."
                        ),
                        None => println!(
                            "[VERIFY]  The channel gave no dependable read on the case our casing opened."
                        ),
                    }
                }
            }
        }
        Strategy::Press => {
            // PRESS already discovered the second score during the cold-case wait and deliberately
            // scheduled nothing on it. The lapse is a standing-down cost, narrated when the
            // opportunity expired and confirmed here from the public lifecycle record.
            if metrics.second_opportunity_expired {
                if narrative {
                    println!(
                        "[DECIDE]  Standing down has a price: the second score on {} lapsed without action while the case stayed protected. The discipline that outlasted the investigation also gave up real value.",
                        scenario.variation.alternate_target_name()
                    );
                }
            } else if narrative {
                println!(
                    "[DECIDE]  The second score is still on the table, but {neighborhood_name} stays dark until leadership confirms the case is shelved."
                );
            }
        }
    }
    Ok(())
}

pub fn record_second_act_burglary_terminal(
    scenario: &Scenario,
    burglary: OperationId,
    metrics: &mut RunMetrics,
) {
    metrics.second_burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    let record = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("second-score burglary must remain queryable");
    metrics.second_burglary_aborted = record.status() == OperationStatus::Aborted;
    if let Some(resolution) = record.resolution() {
        metrics.second_burglary_outcome = Some(resolution.objective_outcome());
        metrics.second_act_property_acquired_value_cents = resolution
            .property_proceeds()
            .map(|proceeds| proceeds.estimated_value().cents());
    }
}

pub fn liquidate_second_act_property(
    scenario: &mut Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(estimated_value) = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.property_proceeds())
        .map(|proceeds| proceeds.estimated_value())
    else {
        return Ok(());
    };
    let venue_name = scenario
        .state
        .world()
        .get_business(scenario.resale_venue)
        .expect("resale venue must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[DECIDE]  Move the second-score property through {venue_name} rather than leave it as held inventory."
        );
    }
    let disposition = validate_dispose_property(
        scenario.registry,
        &scenario.state,
        PropertyDispositionDraft {
            operation: burglary,
            venue: scenario.resale_venue,
            cash_account: scenario.liquidation_cash,
            settlement_account: scenario.liquidation_settlement,
        },
    )?
    .commit(&mut scenario.state)?;
    metrics.second_act_property_realized_cash_cents = Some(disposition.realized_value.cents());
    if narrative {
        println!(
            "[LIQUIDATE] {} estimated property -> {} realized resale cash.",
            format_cents(estimated_value.cents()),
            format_cents(disposition.realized_value.cents())
        );
    }
    Ok(())
}

pub fn capture_terminal_status(
    scenario: &Scenario,
    operation: OperationId,
    metrics: &mut RunMetrics,
) {
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("capacity-probe operation must remain queryable");
    metrics.aborted = record.status() == OperationStatus::Aborted;
    metrics.outcome = record
        .resolution()
        .map(|resolution| resolution.objective_outcome());
    if metrics.aborted {
        let abort = record
            .abort_record()
            .expect("aborted capacity-probe operation must retain its cause");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    }
}

pub fn run_until_operation_terminal(
    scenario: &mut Scenario,
    operation: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let started_at = scenario.state.now();
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("authorized operation must remain queryable");
    let operation_kind = record.kind();
    let authored_duration_minutes = scenario
        .registry
        .get_operation(operation_kind)
        .execution()
        .duration()
        .as_minutes();
    // The terminal-wait guard anchors at the later of the loop's current minute and the
    // operation's authored schedule, then adds the authored duration plus a decision and
    // police-arrival slack window, so it covers the wait-to-start and stays synchronized with
    // authored content instead of a hard-coded constant that could go stale.
    let guard_anchor = record.scheduled_for().max(started_at);
    let deadline = guard_anchor
        + SimDuration::from_minutes(authored_duration_minutes + OPERATION_WAIT_SLACK_MINUTES);
    loop {
        let status = scenario
            .state
            .operations()
            .get_operation(operation)
            .expect("authorized operation must remain queryable")
            .status();
        if matches!(
            status,
            OperationStatus::Completed | OperationStatus::Aborted
        ) {
            return Ok(());
        }
        if scenario.state.now() >= deadline {
            return Err(HarnessContractError::OperationDidNotTerminate {
                operation,
                started_at: started_at.as_minutes(),
                deadline: deadline.as_minutes(),
            }
            .into());
        }
        let outcome = run_tick(scenario.registry, &mut scenario.state);
        observe_tick(scenario, &outcome, narrative, metrics)?;
        maybe_capture_matched_financials(scenario, metrics)?;
    }
}

pub fn run_until(
    scenario: &mut Scenario,
    until: SimTime,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    while scenario.state.now() < until {
        let outcome = run_tick(scenario.registry, &mut scenario.state);
        observe_tick(scenario, &outcome, narrative, metrics)?;
        maybe_capture_matched_financials(scenario, metrics)?;
    }
    maybe_capture_matched_financials(scenario, metrics)?;
    Ok(())
}

/// Snapshot cumulative finances the first time a session reaches the shared campaign-day
/// boundary. Branch arcs that extend past it (the PRESS cold-case wait) still capture an
/// identical-window view, so `validate_branch_financial_isolation` never compares totals
/// from different observation lengths.
pub fn maybe_capture_matched_financials(
    scenario: &mut Scenario,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let boundary_reached = match metrics.matched_financial_boundary_minute {
        Some(boundary) => scenario.state.now().as_minutes() >= boundary,
        None => false,
    };
    if !boundary_reached || metrics.matched_legitimate_net_cents.is_some() {
        return Ok(());
    }
    let view = resolve_financial_view(scenario)?;
    metrics.matched_legitimate_net_cents = Some(view.legitimate_net_cents);
    metrics.matched_enterprise_net_cents = Some(view.enterprise_net_cents);
    Ok(())
}

/// Player-earned counter-intelligence after an accepted defection: the organization watches every
/// known rival through canonical surveillance to confirm where the departed member resurfaces. The
/// departure report deliberately never names the recruiting organization; this follow-up is the
/// player-visible channel that closes the knowledge loop without any hidden-state reads.
pub fn run_defector_trail(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(defector) = metrics.defector else {
        return Ok(());
    };
    let defector_name = scenario
        .state
        .world()
        .get_character(defector)
        .expect("departed character must persist")
        .name()
        .to_owned();
    let player_name = scenario
        .state
        .world()
        .get_organization(scenario.player)
        .expect("player organization must persist")
        .name()
        .to_owned();
    let first_rival_name = scenario
        .state
        .world()
        .get_organization(scenario.rival)
        .expect("rival organization must persist")
        .name()
        .to_owned();
    if narrative {
        let departed_at = metrics
            .defection_minute
            .map(|minute| format!(" at minute {minute}"))
            .unwrap_or_default();
        println!(
            "[DECIDE]  {player_name} knows {defector_name} left{departed_at}. Watch the district's known rivals for where a defector resurfaces; start with {first_rival_name}."
        );
    }
    // The fixture has two named rivals; watch each one through the player's own surveillance. At
    // most one rival is the true destination, so the trail confirms where the member landed while
    // still showing absence everywhere else through the same canonical channel.
    let known_rivals = [scenario.rival, scenario.second_rival];
    let mut resurfaced_at: Option<OrganizationId> = None;
    for rival in known_rivals {
        let rival_name = scenario
            .state
            .world()
            .get_organization(rival)
            .expect("known rival must persist")
            .name()
            .to_owned();
        let title = format!("{rival_name} personnel watch");
        let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
        let operation = authorize_surveillance_target(
            scenario,
            EntityRef::Organization(rival),
            &title,
            scheduled_for,
        )?;
        run_until_operation_terminal(scenario, operation, narrative, metrics)?;
        let resolution = scenario
            .state
            .operations()
            .get_operation(operation)
            .expect("personnel watch must persist")
            .resolution()
            .expect("completed personnel watch must have a resolution");
        let found = resolution
            .discovered_information()
            .iter()
            .any(|information| {
                scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .is_some_and(|record| {
                        record.topic() == InformationTopic::Personnel
                            && record.subject() == EntityRef::Organization(rival)
                            && record.summary().contains(&defector_name)
                    })
            });
        if found {
            resurfaced_at = Some(rival);
        }
        if narrative {
            for information in resolution.discovered_information() {
                let record = scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("personnel-watch information must persist");
                if record.topic() == InformationTopic::Personnel
                    && record.subject() == EntityRef::Organization(rival)
                {
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
            }
            if found {
                println!(
                    "[VERIFY DEFECTOR] {defector_name} now appears among {rival_name}'s recurring personnel."
                );
            } else {
                println!(
                    "[VERIFY DEFECTOR] {rival_name}'s watch shows {defector_name} is not working there."
                );
            }
        }
    }
    metrics.defector_trail_confirmed = Some(resurfaced_at.is_some());
    if narrative {
        match resurfaced_at {
            Some(rival) => {
                let rival_name = scenario
                    .state
                    .world()
                    .get_organization(rival)
                    .expect("confirmed rival must persist")
                    .name()
                    .to_owned();
                println!(
                    "[VERIFY DEFECTOR] {defector_name} resurfaces among {rival_name}'s personnel. {player_name} confirmed through its own surveillance where its former member landed."
                );
            }
            None => println!(
                "[VERIFY DEFECTOR] None of the watched rivals showed {defector_name}; the personnel watch did not directly confirm where the member landed."
            ),
        }
    }
    Ok(())
}

/// The player's answer to a confirmed defector: one personal re-approach through the canonical
/// executive recruitment path. Nothing here reads hidden state — the pitch resolves through the
/// same production scoring the rival's poaching used (recruiter bond versus fresh attachment to
/// the new organization, plus membership resistance). A refusal carries a real intelligence cost:
/// production rules deliver a loyalty report to the rival naming our recruiter, so reaching out
/// tells the rival its poach succeeded and who came asking.
pub fn run_win_back_attempt(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(defector) = metrics.defector else {
        return Ok(());
    };
    let defector_name = scenario
        .state
        .world()
        .get_character(defector)
        .expect("departed character must persist")
        .name()
        .to_owned();
    let boss_name = scenario
        .state
        .world()
        .get_character(scenario.boss)
        .expect("boss must persist")
        .name()
        .to_owned();
    let player_name = scenario
        .state
        .world()
        .get_organization(scenario.player)
        .expect("player organization must persist")
        .name()
        .to_owned();
    let rival_name = scenario
        .state
        .world()
        .get_organization(scenario.rival)
        .expect("rival organization must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[DECIDE]  {boss_name} makes one personal appeal to {defector_name}: come home to {player_name}."
        );
    }
    let attempt_at = scenario.state.now();
    let attempt = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            target_organization: scenario.player,
            recruiter: scenario.boss,
            candidate: defector,
            approach: RecruitmentApproach::PersonalAppeal,
        },
    )?
    .commit(&mut scenario.state)?;
    let record = scenario
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("committed win-back attempt must be queryable");
    let accepted = record.outcome() == RecruitmentOutcome::Accepted;
    metrics.win_back_attempted = true;
    metrics.win_back_accepted = Some(accepted);
    metrics.win_back_margin = Some(record.margin());
    if narrative {
        println!(
            "[NARRATION] The {:?} pitch resolved at margin {}: {boss_name}'s old bond pulls one way; {defector_name}'s fresh attachment to {}'s protection and ordinary membership resistance pull the other.",
            record.approach(),
            record.margin(),
            rival_name,
        );
        println!(
            "[DEV AUDIT] Win-back factors: drive alignment {}, relationship support {}, incumbent attachment {}, incumbent resentment {}, membership resistance {}, trait adjustment {}.",
            record.factors().drive_alignment(),
            record.factors().relationship_support(),
            record.factors().incumbent_attachment(),
            record.factors().incumbent_resentment(),
            record.factors().membership_resistance(),
            record.factors().trait_adjustment(),
        );
    }
    if accepted {
        let membership = scenario
            .state
            .world()
            .get_character(defector)
            .expect("defector must persist")
            .organization();
        debug_assert_eq!(
            membership,
            Some(scenario.player),
            "an accepted win-back must move membership back through the canonical reassignment"
        );
        if narrative {
            println!(
                "[WIN BACK]  {defector_name} came home to {player_name}. Membership moved through the production reassignment path; the crew that left in fear is whole again — and both organizations now know exactly how much his loyalty is worth."
            );
        }
        return Ok(());
    }
    // Refusal cost: the production loyalty-report path tells the candidate's current
    // organization who tried to recruit them. Audit-only here — the acting policy never reads
    // rival reports — but the narration may explain the mechanism because it follows from the
    // player-visible refusal itself.
    let leaked = scenario
        .state
        .reports()
        .reports_for(scenario.rival)
        .any(|report| {
            report.title() == "Personnel approach" && report.generated_at() == attempt_at
        });
    metrics.win_back_refusal_leaked_to_rival = Some(leaked);
    if narrative {
        if leaked {
            println!(
                "[DEV AUDIT] Production rules delivered {defector_name}'s loyalty report to {rival_name} leadership naming {boss_name}: the refusal itself told the rival its poach succeeded and who came asking."
            );
        } else {
            println!(
                "[DEV AUDIT] No loyalty report reached {rival_name} for this refusal; the leak contract expects one through the canonical path."
            );
        }
        println!(
            "[WIN BACK]  {defector_name} stayed with {rival_name}. The door closed politely — and {player_name} paid for the knock with a piece of its own cover."
        );
    }
    Ok(())
}
