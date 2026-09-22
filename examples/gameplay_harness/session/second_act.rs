//! Strategy-specific second-score policy and settlement for full harness sessions.

use super::*;

/// Executes the act-2 beat per branch. RUSH rebuilds the crew and moves the second score away
/// from the overnight hour its own debrief identified as hot; RECON re-invests in planning and
/// works inside a fresh lower-risk window outside known patrol concentrations; PRESS deliberately takes nothing and lets the
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
    if strategy == Strategy::Recon && metrics.opening_stood_down {
        if narrative {
            println!(
                "[DECIDE]  Opening casing was not cleared. Keep the reopened score unworked rather than treating a new opportunity as permission to ignore that risk."
            );
        }
        return Ok(());
    }
    let roster_current = scenario
        .state
        .world()
        .characters_in_organization(scenario.player)
        .any(|record| record.id() == scenario.burglar);
    if strategy == Strategy::Rush {
        if roster_current {
            if narrative {
                println!(
                    "[DECIDE]  The entry crew is whole again after the win-back; no replacement hire is needed."
                );
            }
        } else {
            recruit_replacement(scenario, narrative, metrics)?;
        }
    }
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
    let replacement = if metrics.replacement_recruited {
        metrics
            .replacement
            .expect("a recorded replacement must persist")
    } else {
        scenario.burglar
    };
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
        let crew_note = if metrics.replacement_recruited {
            "The rebuilt crew"
        } else {
            "The returned crew"
        };
        println!(
            "[DECIDE]  {crew_note} is ready. Shift the second score on {} to {}, away from the overnight hour the crew now knows drew a response. Carry that debriefed police read into the plan rather than pretending it revealed a full patrol schedule.",
            scenario.variation.alternate_target_name(),
            format_day_minute(scheduled_for.as_minutes()),
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
    // The policy clock is an earliest readiness time, not permission to ignore yesterday's
    // patrol observation. Protect the scout as well as the eventual burglary, using only
    // organization-held information about this district (never live patrol schedules).
    let patrol = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::PoliceActivity,
        )
        .filter(|record| {
            record.subject() == EntityRef::Neighborhood(scenario.neighborhood)
                && matches!(
                    record.signal(),
                    Some(InformationSignal::PatrolPattern { .. })
                )
        })
        .max_by_key(|record| (record.observed_at(), record.recorded_at(), record.id()))
        .ok_or("RECON has no held district patrol pattern for planning its second scout")?;
    let duration = scenario
        .registry
        .get_operation(OperationKind::Surveillance)
        .execution()
        .duration();
    let ready_at = scenario
        .state
        .now()
        .max(scenario.timeline.recon_second_act_surveillance_at);
    let scout_at = choose_lower_risk_start_from_patrol_signal(
        ready_at,
        patrol
            .signal()
            .expect("selected patrol observation has semantics"),
        duration,
        SimDuration::from_minutes(60),
        scenario.timeline.second_opportunity_valid_until,
    )?;
    if narrative {
        println!(
            "[REUSE INTEL] Patrol observation from {}: {} Scout ready at {}; schedule {}m of surveillance at {} with a 60m patrol buffer. Known patrol windows constrain looking as well as taking; this reduces risk, not guarantees safety.",
            format_day_minute(patrol.observed_at().as_minutes()),
            patrol.summary(),
            format_day_minute(ready_at.as_minutes()),
            duration.as_minutes(),
            format_day_minute(scout_at.as_minutes()),
        );
    }
    metrics.second_scout_patrol_observed_minute = Some(patrol.observed_at().as_minutes());
    let patrol_id = patrol.id();
    let scout_intelligence = BTreeSet::from([patrol_id]);
    let recon = authorize_surveillance_target(
        scenario,
        EntityRef::Business(scenario.alternate_target),
        &title,
        scout_at,
        scout_intelligence,
    )?;
    metrics.second_scout_attached_patrol = scenario
        .state
        .operations()
        .get_operation(recon)
        .expect("authorized scout persists")
        .intelligence()
        .contains(&patrol_id);
    metrics.second_scout_scheduled_minute = Some(
        scenario
            .state
            .operations()
            .get_operation(recon)
            .expect("authorized scout persists")
            .scheduled_for()
            .as_minutes(),
    );
    metrics.second_scout = Some(recon);
    run_until_operation_terminal(scenario, recon, narrative, metrics)?;
    let assessment = assess_casing(scenario, recon, narrative, metrics)?;
    metrics.second_casing_assessment = Some(assessment);
    metrics.self_heat_check_required = matches!(
        assessment,
        CasingAssessment::Active | CasingAssessment::Unknown | CasingAssessment::Shelved
    );
    metrics.self_heat_case_active = match assessment {
        CasingAssessment::Active => Some(true),
        CasingAssessment::Shelved => Some(false),
        CasingAssessment::Clean | CasingAssessment::Unknown | CasingAssessment::Aborted => None,
    };
    metrics.self_heat_case_opened = metrics.self_heat_case_active.is_some();
    if assessment == CasingAssessment::Aborted {
        return Ok(());
    }
    let resolution = scenario
        .state
        .operations()
        .get_operation(recon)
        .expect("second-score surveillance must persist")
        .resolution()
        .expect("completed second-score surveillance must have a resolution");
    metrics.second_scout_topics_covered = Some(resolution.factors().intelligence_topics_covered());
    let discovered_information = resolution.discovered_information().clone();
    metrics.second_act_recon_information = discovered_information.len();
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

    if !assessment.permits_burglary() {
        return Ok(());
    }
    let patrol_information = learned_patrol_information.ok_or(
        "second-score recon did not produce a patrol-pattern observation; the harness will not infer lower-risk timing from hidden state",
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
    let scheduled_for = choose_lower_risk_start_from_patrol_signal(
        scenario.state.now(),
        &patrol_signal,
        duration,
        SimDuration::from_minutes(60),
        scenario.timeline.second_opportunity_valid_until,
    )?;
    if narrative {
        let windows = crate::observe::patrol_intervals_from_signal(&patrol_signal);
        println!(
            "[INTERPRET] Patrol report \"{}\" -> heavy/regular windows {}, burglary {}m +60m buffer -> chose {}, the window avoids the known concentrations, but ambient district policing still remains.",
            patrol_record.summary(),
            crate::readout::format_patrol_windows(&windows),
            duration.as_minutes(),
            crate::readout::stamp(scheduled_for.as_minutes())
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
