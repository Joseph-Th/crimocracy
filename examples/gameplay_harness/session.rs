//! Shared session orchestration, defector trail, and terminal-state loop helpers.
//! PRESS response policy and strategy-specific second acts live in child modules.

mod defector;
mod press;
mod second_act;

use crimocracy::contacts::contact_system::{
    find_pending_disclosure_sources, validate_contact_disclosure,
};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{
    FinancialAccountId, InformationId, OperationId, OpportunityId, OrganizationId,
};
use crimocracy::core::simulation::run_tick;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::finance::finance_system::{
    LaunderingDraft, LaunderingError, ValidatedLaundering, validate_launder_funds,
};
use crimocracy::finance::{AccountKind, FinancialOwner, Money};
use crimocracy::intelligence::{InformationSignal, InformationTopic, KnowledgeHolder};
use crimocracy::legal::InvestigationWorkKind;
use crimocracy::operations::property_disposition::{
    PropertyDispositionDraft, validate_dispose_property,
};
use crimocracy::operations::{OperationAbortCause, OperationKind, OperationStatus};
use crimocracy::opportunities::OperationOpportunityDraft;
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
};
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
/// the typed case-activity sightline plus the disclosed summary so callers can quote a changed
/// read even on days they suppress full narration. The acting policy never enumerates hidden
/// knowledge - `find_pending_disclosure_sources` exposes only what the channel itself offers,
/// and everything the organization learns arrives as a derived information record.
pub fn read_police_contact(
    scenario: &mut Scenario,
    case_subject: EntityRef,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<Option<(bool, String)>, Box<dyn Error>> {
    let sources = find_pending_disclosure_sources(&scenario.state, scenario.police_contact);
    let Some(source) = sources.into_iter().find(|source| {
        scenario
            .state
            .intelligence()
            .get_information(*source)
            .is_some_and(|information| {
                information.topic() == InformationTopic::LegalActivity
                    && information.subject() == case_subject
            })
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
    let read = observe_case_activity_information(record);
    let summary = record.summary().to_owned();
    if narrative {
        println!(
            "[LEARN]   {:?} / {:?}: {}",
            record.reliability(),
            record.specificity(),
            record.summary()
        );
    }
    Ok(read.map(|sightline| (sightline, summary)))
}

/// Working capital the racket's till keeps while its take is being laundered day by day.
pub const LAUNDERING_FLOAT_FLOOR_CENTS: i64 = 5_000;

/// Runs street cash through an owned cash-intensive front's books via the canonical
/// laundering path. The organization asks for the full amount first; a capacity rejection is
/// player-visible accounting information - the front's books can only plausibly absorb an
/// authored share of its legitimate volume per cycle - so the beat launders what fits and
/// leaves the rest where it sits. Returns the committed gross amount, if any.
pub fn launder_through_front(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
    source_account: FinancialAccountId,
    requested_cents: i64,
) -> Result<Option<i64>, Box<dyn Error>> {
    if requested_cents <= 0 {
        return Ok(None);
    }
    let front_name = scenario
        .state
        .world()
        .get_business(scenario.front)
        .expect("laundering front must persist")
        .name()
        .to_owned();
    let draft = |amount: Money| LaunderingDraft {
        organization: scenario.player,
        street_account: source_account,
        business: scenario.front,
        accounted_account: scenario.accounted_funds,
        amount,
    };
    let validated = match validate_launder_funds(
        scenario.registry,
        &scenario.state,
        draft(Money::from_cents(requested_cents)),
    ) {
        Ok(validated) => Some(validated),
        Err(LaunderingError::CapacityExceeded { capacity_cents, .. }) => {
            metrics.laundering_capacity_rejections =
                metrics.laundering_capacity_rejections.saturating_add(1);
            if capacity_cents <= 0 {
                if narrative {
                    println!(
                        "[LAUNDER] {front_name}'s books already carry this cycle's plausible volume; {} stays street cash.",
                        format_cents(requested_cents),
                    );
                }
                return Ok(None);
            }
            if narrative {
                println!(
                    "[LAUNDER] {front_name}'s books can plausibly absorb only {} of the requested {} this cycle; the rest stays street cash.",
                    format_cents(capacity_cents),
                    format_cents(requested_cents),
                );
            }
            Some(validate_launder_funds(
                scenario.registry,
                &scenario.state,
                draft(Money::from_cents(capacity_cents)),
            )?)
        }
        Err(error) => return Err(error.into()),
    };
    let Some(validated) = validated else {
        return Ok(None);
    };
    let (gross, fee) = commit_laundering(scenario, metrics, validated, source_account)?;
    if narrative {
        println!(
            "[LAUNDER] {front_name} absorbed {}; the house kept {} as booked revenue, and {} now sits as accounted money.",
            format_cents(gross),
            format_cents(fee),
            format_cents(gross - fee),
        );
    }
    Ok(Some(gross))
}

/// Commits a validated laundering transaction and records its gross/fee split in the run
/// metrics from the committed ledger postings, never from recomputed game math.
fn commit_laundering(
    scenario: &mut Scenario,
    metrics: &mut RunMetrics,
    validated: ValidatedLaundering,
    source_account: FinancialAccountId,
) -> Result<(i64, i64), Box<dyn Error>> {
    let transaction = validated.commit(&mut scenario.state)?;
    let record = scenario
        .state
        .finance()
        .get_transaction(transaction)
        .expect("committed laundering transaction must be queryable");
    let gross = record
        .postings()
        .iter()
        .find(|posting| posting.account == source_account)
        .map(|posting| -posting.amount.cents())
        .expect("laundering transaction must debit its street source");
    let credited = record
        .postings()
        .iter()
        .find(|posting| posting.account == scenario.accounted_funds)
        .map(|posting| posting.amount.cents())
        .expect("laundering transaction must credit accounted funds");
    let fee = gross - credited;
    metrics.laundered_gross_cents += gross;
    metrics.launder_fee_cents += fee;
    Ok((gross, fee))
}

/// Records the quiet word's outcome from production records: resolution or abort provenance,
/// and whether the registered witness's cooperation actually moved. The after-action report
/// is the organization's player-visible account; cooperation is audit evidence.
fn capture_witness_pressure_outcome(
    scenario: &mut Scenario,
    pressure: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    metrics.witness_pressure_attempted = true;
    let record = scenario
        .state
        .operations()
        .get_operation(pressure)
        .expect("witness-pressure operation must remain queryable");
    metrics.witness_pressure_aborted = record.status() == OperationStatus::Aborted;
    if let Some(resolution) = record.resolution() {
        metrics.witness_pressure_outcome = Some(resolution.objective_outcome());
        if narrative {
            let report = scenario
                .state
                .reports()
                .get_report(resolution.after_action_report())
                .expect("pressure after-action report must persist");
            print_report("AFTER-ACTION", report, scenario);
        }
    }
    if record.status() == OperationStatus::Aborted && narrative {
        println!(
            "[DECIDE]  A response was coming; the crew walked away. Quiet work in a watched district is not free - leadership leaves the witness alone rather than risk another case."
        );
    }
    if let Some(investigation) = scenario.investigation {
        metrics.witness_cooperation_degraded = scenario
            .state
            .legal()
            .case_witnesses_for_investigation(investigation)
            .filter(|case_witness| case_witness.witness() == scenario.target_owner)
            .any(|case_witness| {
                matches!(
                    case_witness.cooperation(),
                    crimocracy::legal::WitnessCooperation::Reluctant
                        | crimocracy::legal::WitnessCooperation::Hostile
                )
            });
        // Cooperation is intentionally audit-only here. The player-facing after-action above can
        // report what the crew attempted and observed, but no information channel reveals the
        // witness owner's hidden cooperation state after the pressure operation.
    }
    Ok(())
}

fn discover_initial_opportunity(
    scenario: &mut Scenario,
    narrative: bool,
) -> Result<OpportunityId, Box<dyn Error>> {
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
    Ok(opportunity)
}

struct InitialBurglaryPlan {
    scheduled_for: SimTime,
    intelligence: BTreeSet<InformationId>,
}

fn prepare_initial_burglary_plan(
    scenario: &mut Scenario,
    strategy: Strategy,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<InitialBurglaryPlan, Box<dyn Error>> {
    let mut intelligence = BTreeSet::from([scenario.opportunity_information]);
    let mut learned_patrol_information = None;
    if strategy == Strategy::Recon {
        if narrative {
            println!(
                "[DECIDE]  Order surveillance before committing the burglary. The goal is to learn venue access and police rhythm."
            );
        }
        let surveillance = authorize_surveillance(scenario)?;
        run_until_operation_terminal(scenario, surveillance, narrative, metrics)?;
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
            if record.topic() == InformationTopic::PoliceActivity
                && matches!(
                    record.signal(),
                    Some(InformationSignal::PatrolPattern { .. })
                )
            {
                learned_patrol_information = Some(*information);
            }
            intelligence.insert(*information);
        }
    }

    let scheduled_for = match strategy {
        Strategy::Rush | Strategy::Press => scenario.timeline.initial_burglary_at,
        Strategy::Recon => {
            let patrol_information = learned_patrol_information.ok_or(
                "recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
            )?;
            let patrol_record = scenario
                .state
                .intelligence()
                .get_information(patrol_information)
                .expect("selected patrol-pattern information must persist");
            let patrol_signal = patrol_record
                .signal()
                .cloned()
                .ok_or("selected patrol-pattern information lost its typed semantics")?;
            let duration = scenario
                .registry
                .get_operation(OperationKind::Burglary)
                .execution()
                .duration();
            let chosen = choose_safe_start_from_patrol_signal(
                scenario.state.now(),
                &patrol_signal,
                duration,
                SimDuration::from_minutes(60),
                scenario.timeline.initial_opportunity_valid_until,
            )?;
            if narrative {
                let windows = crate::observe::patrol_intervals_from_signal(&patrol_signal);
                println!(
                    "[INTERPRET] Patrol report \"{}\" -> windows {:?} (minutes), burglary {}m +60m buffer -> chose minute {} ({}), window stays outside heavy presence.",
                    patrol_record.summary(),
                    windows,
                    duration.as_minutes(),
                    chosen.as_minutes(),
                    crate::readout::format_minute_of_day(chosen.as_minutes())
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
    Ok(InitialBurglaryPlan {
        scheduled_for,
        intelligence,
    })
}

fn authorize_initial_burglary(
    scenario: &mut Scenario,
    strategy: Strategy,
    opportunity: OpportunityId,
    plan: InitialBurglaryPlan,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<OperationId, Box<dyn Error>> {
    let title = format!("{} burglary", scenario.variation.target_name());
    let burglary = authorize_burglary(
        scenario,
        strategy,
        scenario.target,
        &title,
        plan.scheduled_for,
        plan.intelligence,
        scenario.burglar,
    )?;
    validate_convert_opportunity(&scenario.state, opportunity, burglary)?
        .commit(&mut scenario.state)?;
    metrics.burglary = Some(burglary);

    let record = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must exist");
    metrics.planning_information_count = record.intelligence().len();
    metrics.planning_information_topics = record
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
        println!(
            "[COMMIT]  Burglary authorized for minute {} with {:?} approach and {} planning information item(s).",
            plan.scheduled_for.as_minutes(),
            record.approach(),
            record.intelligence().len(),
        );
        print_planning_inputs(scenario, burglary);
    }
    Ok(burglary)
}

fn resolve_initial_burglary(
    scenario: &mut Scenario,
    strategy: Strategy,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    run_until_operation_terminal(scenario, burglary, narrative, metrics)?;
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
        // The case ID is audit-only. Acting policy learns case existence and timing from its own
        // LegalActivity information after the resolution is surfaced.
        scenario.investigation = resolution.exposure().investigation();
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
            print_report("AFTER-ACTION", report, scenario);
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
            print_report("ABORT REPORT", report, scenario);
        }
        if strategy == Strategy::Rush {
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

    if metrics.aborted
        && matches!(
            metrics.abort_cause,
            Some(OperationAbortCause::PoliceArrival(_))
        )
    {
        let debrief_information = burglary_record
            .abort_record()
            .and_then(|abort| abort.artifacts())
            .and_then(|artifacts| artifacts.police_activity_information());
        if let Some(information) = debrief_information {
            metrics.player_police_activity_information =
                metrics.player_police_activity_information.saturating_add(1);
            metrics
                .debrief_police_activity_information
                .push(information);
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
    Ok(())
}

fn liquidate_initial_property(
    scenario: &mut Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let acquired_property_value = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.property_proceeds())
        .map(|proceeds| proceeds.estimated_value());
    let Some(estimated_value) = acquired_property_value else {
        return Ok(());
    };
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
        let front_name = scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("laundering front must persist")
            .name()
            .to_owned();
        println!(
            "[DECIDE]  Dirty cash buys nothing legitimate. Run the resale proceeds through {front_name}'s books."
        );
    }
    launder_through_front(
        scenario,
        narrative,
        metrics,
        scenario.liquidation_cash,
        disposition.realized_value.cents(),
    )?;
    Ok(())
}

fn refresh_player_case_information(
    scenario: &Scenario,
    burglary: OperationId,
    metrics: &mut RunMetrics,
) {
    let player_case_information = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::LegalActivity,
        )
        .filter(|information| information.subject() == EntityRef::Operation(burglary))
        .collect::<Vec<_>>();
    metrics.player_legal_activity_information = player_case_information.len();
    metrics.case_open_minute = player_case_information
        .iter()
        .map(|information| information.observed_at().as_minutes())
        .min();
}

fn capture_initial_case_diagnostics(
    scenario: &Scenario,
    burglary: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) {
    if narrative {
        print_player_knowledge_gap(scenario, burglary);
    }
    if let Some(investigation) = scenario.investigation {
        metrics.case_witness_registered = scenario
            .state
            .legal()
            .case_witnesses_for_investigation(investigation)
            .next()
            .is_some();
    }
}

fn capture_campaign_audit_metrics(scenario: &Scenario, metrics: &mut RunMetrics) {
    metrics.rival_home_enterprises =
        resolve_neighborhood_influence(&scenario.state, scenario.neighborhood)
            .expect("home-district influence should resolve")
            .standings
            .into_iter()
            .filter(|standing| standing.organization != scenario.player)
            .map(|standing| standing.active_enterprises)
            .sum();
    if let Some(expansion) = metrics.expansion_enterprise {
        let (net, heat) = scenario
            .state
            .enterprises()
            .cycles_for(expansion)
            .try_fold((Money::ZERO, Money::ZERO), |(net, heat), cycle| {
                Some((
                    net.checked_add(cycle.net_cash())?,
                    heat.checked_add(cycle.investigation_heat())?,
                ))
            })
            .expect("expansion enterprise totals must fit money range");
        metrics.expansion_net_cents = Some(net.cents());
        metrics.expansion_heat_cents = Some(heat.cents());
    }
    if let Some(investigation) = scenario.investigation {
        let interview_ran = scenario
            .state
            .legal()
            .work_for_investigation(investigation)
            .any(|work| work.kind() == InvestigationWorkKind::WitnessInterview);
        metrics.witness_testimony_produced = interview_ran
            && scenario
                .state
                .legal()
                .get_investigation(investigation)
                .is_some_and(|case| {
                    case.evidence().iter().any(|evidence_id| {
                        scenario
                            .state
                            .legal()
                            .get_evidence(*evidence_id)
                            .is_some_and(|evidence| {
                                evidence.kind() == crimocracy::legal::EvidenceKind::WitnessTestimony
                            })
                    })
                });
    }
}

fn run_post_burglary_campaign(
    scenario: &mut Scenario,
    strategy: Strategy,
    burglary: OperationId,
    full_arc: bool,
    narrative: bool,
    campaign_day_minutes: u64,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    if strategy == Strategy::Press {
        press::run_press_response(
            scenario,
            burglary,
            full_arc,
            narrative,
            campaign_day_minutes,
            metrics,
        )?;
    }

    let observation_end = if full_arc {
        SimTime::from_minutes(campaign_day_minutes * 2)
    } else {
        SimTime::from_minutes(campaign_day_minutes)
    };
    let recruitment_boundary = SimTime::from_minutes(campaign_day_minutes + 1);
    if full_arc && observation_end > recruitment_boundary {
        run_until(scenario, recruitment_boundary, narrative, metrics)?;
    } else {
        run_until(scenario, observation_end, narrative, metrics)?;
    }

    if full_arc && !metrics.second_opportunity_discovered {
        let discovery_at = scenario.timeline.second_opportunity_discovery_at;
        if scenario.state.now() < discovery_at {
            run_until(scenario, discovery_at, narrative, metrics)?;
        }
        discover_second_opportunity(scenario, narrative, metrics)?;
    }
    if full_arc && metrics.defector.is_some() && metrics.defector_trail_confirmed.is_none() {
        defector::run_defector_trail(scenario, narrative, metrics)?;
    }
    if full_arc && metrics.defector_trail_confirmed.is_some() {
        defector::run_win_back_attempt(scenario, narrative, metrics)?;
    }
    if full_arc {
        second_act::run_second_act(scenario, strategy, narrative, metrics)?;
    }
    if scenario.state.now() < observation_end {
        run_until(scenario, observation_end, narrative, metrics)?;
    }

    let mut financials = resolve_financial_view(scenario, metrics)?;
    financials.payroll_paid_cents = metrics.payroll_paid_cents;
    financials.payroll_short_cents = metrics.payroll_short_cents;
    metrics.legitimate_net_cents = Some(financials.legitimate_net_cents);
    metrics.enterprise_net_cents = Some(financials.enterprise_net_cents);
    capture_campaign_audit_metrics(scenario, metrics);
    if narrative {
        print_organization_closing_view(scenario, metrics);
        print_second_act_recap(scenario, strategy, metrics);
        print_financial_view(scenario, financials);
        print_executive_briefs(
            scenario
                .state
                .reports()
                .reports_for(scenario.player)
                .filter(|report| report.kind() == ReportKind::ExecutiveBrief),
        );
    }
    Ok(())
}

fn capture_final_session_summary(scenario: &Scenario, metrics: &mut RunMetrics) {
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
    metrics.accounted_balance_cents = Some(
        scenario
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(scenario.player))
            .filter(|account| account.kind() == AccountKind::AccountedFunds)
            .try_fold(0_i64, |total, account| {
                total.checked_add(account.balance().cents())
            })
            .expect("organization accounted-funds total must fit money range"),
    );
}

pub fn play_session(
    registry: &Registry,
    strategy: Strategy,
    profile: ScenarioProfile,
    seeds: EvaluationSeeds,
    run_mode: SessionRunMode,
) -> Result<RunMetrics, Box<dyn Error>> {
    play_session_with_fixture_view(
        registry,
        strategy,
        profile,
        seeds,
        run_mode,
        run_mode.narrative(),
    )
}

/// `print_fixture_view` exists so a narrative comparison prints the shared authored fixture
/// once instead of repeating it per strategy: the world is identical across matched branches.
pub fn play_session_with_fixture_view(
    registry: &Registry,
    strategy: Strategy,
    profile: ScenarioProfile,
    seeds: EvaluationSeeds,
    run_mode: SessionRunMode,
    print_fixture_view: bool,
) -> Result<RunMetrics, Box<dyn Error>> {
    let narrative = run_mode.narrative();
    let full_arc = run_mode.full_arc();
    let mut scenario = build_scenario(registry, seeds, profile)?;
    let mut metrics = RunMetrics {
        strategy: Some(strategy),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };
    // Matched financial boundary: full sessions snapshot at two campaign days and batch
    // sessions at one. Every branch crosses this minute before a longer consequence arc extends,
    // so the snapshot compares identical windows instead of unequal waits.
    let campaign_day_minutes = u64::from(
        registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes(),
    );
    metrics.matched_financial_boundary_minute =
        Some(campaign_day_minutes * if full_arc { 2 } else { 1 });

    if narrative && print_fixture_view {
        println!(
            "[FIXTURE] {} authored variation selected by world seed.",
            scenario.variation.label(),
        );
        print_starting_player_view(&scenario);
    }

    let opportunity = discover_initial_opportunity(&mut scenario, narrative)?;
    let plan = prepare_initial_burglary_plan(&mut scenario, strategy, narrative, &mut metrics)?;
    let burglary = authorize_initial_burglary(
        &mut scenario,
        strategy,
        opportunity,
        plan,
        narrative,
        &mut metrics,
    )?;
    resolve_initial_burglary(&mut scenario, strategy, burglary, narrative, &mut metrics)?;
    liquidate_initial_property(&mut scenario, burglary, narrative, &mut metrics)?;
    capture_initial_case_diagnostics(&scenario, burglary, narrative, &mut metrics);

    run_post_burglary_campaign(
        &mut scenario,
        strategy,
        burglary,
        full_arc,
        narrative,
        campaign_day_minutes,
        &mut metrics,
    )?;
    capture_final_session_summary(&scenario, &mut metrics);

    Ok(metrics)
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
        + SimDuration::from_minutes(authored_duration_minutes + scenario.wait_slack_minutes.max(1));
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
    let view = resolve_financial_view(scenario, metrics)?;
    metrics.matched_legitimate_net_cents = Some(view.legitimate_net_cents);
    metrics.matched_enterprise_net_cents = Some(view.enterprise_net_cents);
    Ok(())
}
