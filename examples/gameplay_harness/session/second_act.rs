//! Strategy-specific second-score policy and settlement for full harness sessions.

use super::*;

/// Executes the act-2 beat per branch. RUSH rebuilds the crew and moves the second score away
/// from the overnight hour its own debrief identified as hot; RECON re-invests in planning and
/// works inside a fresh patrol-safe window; PRESS deliberately takes nothing and lets the
/// discovered opportunity lapse.
pub(super) fn run_second_act(
    scenario: &mut Scenario,
    strategy: Strategy,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(opportunity) = metrics.second_opportunity else {
        return Err("act 2 cannot run before the second opportunity is discovered".into());
    };
    match strategy {
        Strategy::Rush => run_rush_second_act(scenario, opportunity, narrative, metrics),
        Strategy::Recon => run_recon_second_act(scenario, opportunity, narrative, metrics),
        Strategy::Press => {
            run_press_second_act(scenario, narrative, metrics);
            Ok(())
        }
    }
}

fn run_rush_second_act(
    scenario: &mut Scenario,
    opportunity: OpportunityId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let replacement = recruit_replacement(scenario, narrative, metrics)?;
    let scheduled_for = scenario.timeline.rush_second_act_at;
    let title = format!(
        "{} second-score burglary",
        scenario.variation.alternate_target_name()
    );
    let mut intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
    intelligence.extend(metrics.debrief_police_activity_information.iter().copied());
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
            "[DECIDE]  Rebuild is in hand. Shift the second score on {} to {}, away from the overnight hour the crew now knows drew a response. Carry that debriefed police read into the rebuilt crew's plan rather than pretending it revealed a full patrol schedule.",
            scenario.variation.alternate_target_name(),
            format_minute_of_day(scheduled_for.as_minutes()),
        );
    }
    let burglary = authorize_burglary(
        scenario,
        Strategy::Rush,
        scenario.alternate_target,
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
    liquidate_second_act_property(scenario, burglary, narrative, metrics)
}

fn run_recon_second_act(
    scenario: &mut Scenario,
    opportunity: OpportunityId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
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
        EntityRef::Business(scenario.alternate_target),
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
    let discovered_information = resolution.discovered_information().clone();
    metrics.second_act_recon_information = discovered_information.len();
    metrics.self_heat_case_opened = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::LegalActivity,
        )
        .any(|information| information.subject() == EntityRef::Operation(recon));
    let mut burglary_intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
    let mut learned_patrol_information = None;
    for information in &discovered_information {
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
        if record.topic() == InformationTopic::PoliceActivity
            && matches!(
                record.signal(),
                Some(InformationSignal::PatrolPattern { .. })
            )
        {
            learned_patrol_information = Some(*information);
        }
        burglary_intelligence.insert(*information);
    }

    if metrics.self_heat_case_opened
        && !recon_case_is_cool_enough_to_continue(scenario, recon, narrative, metrics)?
    {
        return Ok(());
    }
    let patrol_information = learned_patrol_information.ok_or(
        "second-score recon did not produce a patrol-pattern observation; the harness will not infer a safe time from hidden state",
    )?;
    let patrol_record = scenario
        .state
        .intelligence()
        .get_information(patrol_information)
        .expect("second-score patrol-pattern information must persist");
    let patrol_signal = patrol_record
        .signal()
        .cloned()
        .ok_or("second-score patrol-pattern information lost its typed semantics")?;
    let duration = scenario
        .registry
        .get_operation(OperationKind::Burglary)
        .execution()
        .duration();
    let scheduled_for = choose_safe_start_from_patrol_signal(
        scenario.state.now(),
        &patrol_signal,
        duration,
        SimDuration::from_minutes(60),
        scenario.timeline.second_opportunity_valid_until,
    )?;
    if narrative {
        let windows = crate::observe::patrol_intervals_from_signal(&patrol_signal);
        println!(
            "[INTERPRET] Patrol report \"{}\" -> windows {:?} (minutes), burglary {}m +60m buffer -> chose {} ({}), window stays outside heavy presence.",
            patrol_record.summary(),
            windows,
            duration.as_minutes(),
            scheduled_for.as_minutes(),
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
        scenario.alternate_target,
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
    liquidate_second_act_property(scenario, burglary, narrative, metrics)
}

fn recon_case_is_cool_enough_to_continue(
    scenario: &mut Scenario,
    recon: OperationId,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<bool, Box<dyn Error>> {
    let police_name = scenario
        .state
        .world()
        .get_organization(scenario.police)
        .expect("police organization must persist")
        .name()
        .to_owned();
    let neighborhood_name = scenario
        .state
        .world()
        .get_neighborhood(scenario.neighborhood)
        .expect("neighborhood must persist")
        .name()
        .to_owned();
    if narrative {
        println!(
            "[DECIDE]  The after-action on our own casing says it drew a case. Before another job touches {neighborhood_name}, leadership uses its channel inside {police_name}."
        );
    }
    metrics.self_heat_case_active =
        read_police_contact(scenario, EntityRef::Operation(recon), narrative, metrics)?
            .map(|(sightline, _)| sightline);
    match metrics.self_heat_case_active {
        Some(false) => {
            if narrative {
                println!(
                    "[VERIFY]  The channel says {police_name} has already shelved the casing case. RECON can keep evaluating the score from the information it gathered."
                );
            }
            Ok(true)
        }
        Some(true) => {
            if narrative {
                println!(
                    "[VERIFY]  Detectives are actively developing the case our casing opened. RECON stands down; the second score will lapse rather than compound fresh heat."
                );
            }
            Ok(false)
        }
        None => {
            if narrative {
                println!(
                    "[VERIFY]  The channel gave no dependable read on the casing case. RECON treats uncertainty as risk and stands down; the second score will lapse."
                );
            }
            Ok(false)
        }
    }
}

fn run_press_second_act(scenario: &Scenario, narrative: bool, metrics: &RunMetrics) {
    if !narrative {
        return;
    }
    if metrics.second_opportunity_expired {
        println!(
            "[DECIDE]  Standing down has a price: the second score on {} lapsed without action while the case stayed protected. The discipline that outlasted the investigation also gave up real value.",
            scenario.variation.alternate_target_name()
        );
    } else {
        let neighborhood_name = scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("neighborhood must persist")
            .name();
        println!(
            "[DECIDE]  The second score is still on the table, but {neighborhood_name} stays dark until leadership confirms the case is shelved."
        );
    }
}

fn record_second_act_burglary_terminal(
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

fn liquidate_second_act_property(
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
    launder_through_front(
        scenario,
        narrative,
        metrics,
        scenario.liquidation_cash,
        disposition.realized_value.cents(),
    )?;
    Ok(())
}
