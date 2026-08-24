//! Tick observation: metrics capture, narration, and patrol-report sightline parsing.

use crimocracy::core::attention::AttentionClass;
use crimocracy::core::simulation::TickOutcome;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::decisions::{DecisionContext, DecisionResponse};
use crimocracy::intelligence::intelligence_system::validate_information_transfer;
use crimocracy::intelligence::{InformationTopic, InformationTransferDraft, KnowledgeHolder};
use std::error::Error;

use crate::*;

pub fn observe_tick(
    scenario: &mut Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    if !outcome.staffed_investigations.is_empty() {
        metrics.session_case_staffed = true;
    }
    for payroll in outcome
        .payrolls
        .iter()
        .filter(|payroll| payroll.organization() == scenario.player)
    {
        metrics.payroll_paid_cents += payroll.paid().cents();
        metrics.payroll_short_cents += payroll.short().cents();
        if narrative && payroll.short().cents() > 0 {
            println!(
                "[PAYROLL]  {}: the day's wages went unpaid ({} owed). The crew will remember.",
                stamp(outcome.now.as_minutes()),
                format_cents(payroll.owed().cents())
            );
        }
    }
    if narrative {
        for operation in &outcome.started_operations {
            let record = scenario
                .state
                .operations()
                .get_operation(*operation)
                .expect("started operation must exist");
            println!(
                "[START]   {}: {} started.",
                stamp(outcome.now.as_minutes()),
                record.title()
            );
            if let Some(response) = record.police_response() {
                let response = scenario
                    .state
                    .legal()
                    .get_police_response(response)
                    .expect("dispatched response must persist");
                println!(
                    "          Police response dispatched; estimated arrival minute {} based on local deployment.",
                    response.arrival_due_at().as_minutes()
                );
            }
        }
    }

    // A cold-case shelf is an institutional beat, not player-visible news: the organization
    // learns it through its own channels (precinct surveillance or the police contact). The
    // narrative prints it only as a hidden-state audit marker.
    if let Some(case) = scenario.investigation {
        if outcome.cold_case_suspensions.contains(&case) {
            metrics.case_cold_minute = Some(outcome.now.as_minutes());
            if narrative {
                let owner = scenario
                    .state
                    .legal()
                    .get_investigation(case)
                    .expect("shelved case must persist")
                    .owner();
                let owner_name = scenario
                    .state
                    .world()
                    .get_organization(owner)
                    .expect("case owner must persist")
                    .name();
                println!(
                    "[DEV AUDIT] {}: {} shelved the case (hidden institutional beat; the organization learns this through its own channels).",
                    stamp(outcome.now.as_minutes()),
                    owner_name
                );
            }
        }
    }

    // The second score's lapse is a deliberate, observed consequence: PRESS stands down while the
    // case is hot and the opportunity expires through the canonical lifecycle, generating its own
    // report instead of any hidden-state read.
    if let Some(opportunity) = metrics.second_opportunity {
        if outcome.expired_opportunities.contains(&opportunity) {
            metrics.second_opportunity_expired = true;
            if narrative {
                println!(
                    "[OPPORTUNITY COST] {}: the second score on {} lapsed without action. The standing-down discipline that protects the hot case has a real price.",
                    stamp(outcome.now.as_minutes()),
                    scenario.variation.alternate_target_name()
                );
            }
        }
    }

    for operation in &outcome.started_operations {
        if Some(*operation) == metrics.burglary {
            metrics.police_dispatched = scenario
                .state
                .operations()
                .get_operation(*operation)
                .and_then(|record| record.police_response())
                .is_some();
        }
    }
    if let Some(burglary) = metrics.burglary {
        metrics.police_dispatched |= scenario
            .state
            .operations()
            .get_operation(burglary)
            .and_then(|record| record.police_response())
            .is_some();
        if let Some(response) = scenario
            .state
            .operations()
            .get_operation(burglary)
            .and_then(|record| record.police_response())
        {
            metrics.police_arrived |= outcome.arrived_police_responses.contains(&response);
        }
    }

    for request in &outcome.decision_requests {
        metrics.decision_requests += 1;
        let decision = scenario
            .state
            .decisions()
            .get_decision(request.decision)
            .expect("surfaced decision must persist");
        if narrative {
            println!(
                "[EXCEPTION] {}: {}",
                stamp(outcome.now.as_minutes()),
                decision.summary()
            );
        }
        let response = match decision.context() {
            DecisionContext::OperationPoliceArrival { .. }
                if metrics.strategy == Some(Strategy::Press) =>
            {
                DecisionResponse::Continue
            }
            DecisionContext::OperationPoliceArrival { .. } => DecisionResponse::Abort,
            DecisionContext::RecruitmentApproval(_) => DecisionResponse::Reject,
        };
        if narrative {
            println!("[DECIDE]  Leadership response: {response:?}.");
        }
        validate_resolve_decision(
            scenario.registry,
            &scenario.state,
            request.decision,
            decision.recipient(),
            response,
        )?
        .commit(&mut scenario.state)?;
    }

    // Press is the branch where the leader chooses to continue after police arrival. The
    // response also creates direct observations for the participating people; report those
    // observations through the canonical transfer path so the player-facing organization view
    // contains the lived consequence without reading hidden case state.
    if metrics.strategy == Some(Strategy::Press) && metrics.police_arrived {
        let sources: Vec<_> = scenario
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Character(scenario.burglar),
                InformationTopic::PoliceActivity,
            )
            .filter(|information| information.observed_at() == outcome.now)
            .map(|information| information.id())
            .collect();
        for source in sources {
            let already_reported = scenario
                .state
                .intelligence()
                .information_derived_from(source)
                .any(|information| {
                    information.holder() == KnowledgeHolder::Organization(scenario.player)
                });
            if already_reported {
                continue;
            }
            validate_information_transfer(
                &scenario.state,
                InformationTransferDraft {
                    source,
                    recipient: KnowledgeHolder::Organization(scenario.player),
                },
            )?
            .commit(&mut scenario.state)?;
            metrics.player_police_activity_information =
                metrics.player_police_activity_information.saturating_add(1);
            if narrative {
                println!(
                    "[PLAYER ACTION] {}: the crew reported the police response back to Marrow Organization; the organization now knows what the burglar directly experienced.",
                    stamp(outcome.now.as_minutes()),
                );
            }
        }
    }

    metrics.autonomous_recruitment_attempts = metrics
        .autonomous_recruitment_attempts
        .saturating_add(u32::try_from(outcome.recruitment_attempts.len()).unwrap_or(u32::MAX));
    for attempt in &outcome.recruitment_attempts {
        let attempt = scenario
            .state
            .recruitment()
            .get_attempt(*attempt)
            .expect("autonomous recruitment attempt must persist");
        if attempt.previous_organization() == Some(scenario.player) {
            match attempt.outcome() {
                crimocracy::recruitment::RecruitmentOutcome::Accepted => {
                    metrics.player_personnel_departures =
                        metrics.player_personnel_departures.saturating_add(1);
                    metrics.defector = Some(attempt.candidate());
                    metrics.defection_minute = Some(outcome.now.as_minutes());
                }
                // The member stayed loyal and reported the pitch through the production
                // loyalty-report path; the organization now knows it was targeted and by whom.
                crimocracy::recruitment::RecruitmentOutcome::Refused => {
                    metrics.player_poach_warnings = metrics.player_poach_warnings.saturating_add(1);
                }
            }
        }
        if narrative {
            let recruiter = scenario
                .state
                .world()
                .get_character(attempt.recruiter())
                .expect("autonomous recruiter must exist");
            let candidate = scenario
                .state
                .world()
                .get_character(attempt.candidate())
                .expect("autonomous candidate must exist");
            println!(
                "[AUTONOMY] {}: {} independently approached {} using {:?}; pressure {}, margin {}, outcome {:?}.",
                stamp(outcome.now.as_minutes()),
                recruiter.name(),
                candidate.name(),
                attempt.approach(),
                attempt.factors().perceived_legal_pressure(),
                attempt.margin(),
                attempt.outcome(),
            );
            println!(
                "[DEV AUDIT] Recruitment factors: drive alignment {}, relationship support {}, incumbent attachment {}, incumbent resentment {}, perceived legal pressure {}, membership resistance {}, trait adjustment {}.",
                attempt.factors().drive_alignment(),
                attempt.factors().relationship_support(),
                attempt.factors().incumbent_attachment(),
                attempt.factors().incumbent_resentment(),
                attempt.factors().perceived_legal_pressure(),
                attempt.factors().membership_resistance(),
                attempt.factors().trait_adjustment(),
            );
            narrate_recruitment_causality(scenario, metrics, attempt, candidate);
            if attempt.previous_organization() == Some(scenario.player)
                && attempt.outcome() == crimocracy::recruitment::RecruitmentOutcome::Refused
            {
                // Quote the loyalty report the production path delivered to the player, so the
                // narrative shows the organization's own information rather than a reconstruction.
                if let Some(report) =
                    scenario
                        .state
                        .reports()
                        .reports_for(scenario.player)
                        .find(|report| {
                            report.title() == "Personnel approach"
                                && report.generated_at() == outcome.now
                        })
                {
                    for entry in report.entries() {
                        println!(
                            "[POACH WARNING] {}: {}",
                            stamp(outcome.now.as_minutes()),
                            entry.summary
                        );
                    }
                }
            }
        }
    }

    if narrative {
        for (investigation, investigator) in &outcome.staffed_investigations {
            let case = scenario
                .state
                .legal()
                .get_investigation(*investigation)
                .expect("staffed investigation must persist");
            let investigator = scenario
                .state
                .world()
                .get_character(*investigator)
                .expect("staffed investigator must persist");
            println!(
                "[DEV AUDIT] {}: {} assigned {} as lead investigator.",
                stamp(outcome.now.as_minutes()),
                case.title(),
                investigator.name(),
            );
        }
        for work_id in &outcome.scheduled_investigation_work {
            let work = scenario
                .state
                .legal()
                .get_investigation_work(*work_id)
                .expect("scheduled investigation work must persist");
            let source = work
                .focus()
                .evidence_id()
                .and_then(|evidence| scenario.state.legal().get_evidence(evidence));
            println!(
                "[DEV AUDIT] {}: scheduled {:?} due {} using {:?} evidence.",
                stamp(outcome.now.as_minutes()),
                work.kind(),
                stamp(work.due_at().as_minutes()),
                source.map(|evidence| evidence.kind()),
            );
        }
        for work_id in &outcome.resolved_investigation_work {
            let work = scenario
                .state
                .legal()
                .get_investigation_work(*work_id)
                .expect("resolved investigation work must persist");
            let resolution = work
                .resolution()
                .expect("resolved investigation work must have a resolution");
            let derived = resolution
                .derived_evidence()
                .and_then(|evidence| scenario.state.legal().get_evidence(evidence));
            println!(
                "[DEV AUDIT] {}: {:?} resolved {:?} at margin {}; derived {:?}.",
                stamp(outcome.now.as_minutes()),
                work.kind(),
                resolution.outcome(),
                resolution.margin(),
                derived.map(|evidence| evidence.kind()),
            );
        }
    }
    metrics.investigation_work_scheduled = metrics.investigation_work_scheduled.saturating_add(
        u32::try_from(outcome.scheduled_investigation_work.len()).unwrap_or(u32::MAX),
    );
    metrics.investigation_work_resolved = metrics.investigation_work_resolved.saturating_add(
        u32::try_from(outcome.resolved_investigation_work.len()).unwrap_or(u32::MAX),
    );

    if narrative {
        for operation in &outcome.resolved_operations {
            let record = scenario
                .state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must persist");
            let resolution = record
                .resolution()
                .expect("resolved operation must have result");
            println!(
                "[RESULT]  {}: {} -> {:?}, exposure {:?}.",
                stamp(outcome.now.as_minutes()),
                record.title(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            );
        }
        if !outcome.business_cycles.is_empty() || !outcome.enterprise_cycles.is_empty() {
            println!(
                "[ROUTINE] {}: {} legitimate business cycle(s), {} delegated enterprise cycle(s).",
                stamp(outcome.now.as_minutes()),
                outcome.business_cycles.len(),
                outcome.enterprise_cycles.len(),
            );
        }
        // A notable cycle carries its manager's report as organization-held information; the
        // narrative surfaces it so heat-driven cost pressure is legible when it happens. Only
        // this organization's cycles are player-visible: another organization's manager report
        // is information they hold, not something our leadership can read.
        for cycle_id in &outcome.enterprise_cycles {
            let cycle = scenario
                .state
                .enterprises()
                .get_cycle(*cycle_id)
                .expect("settled enterprise cycle must persist");
            if cycle.attention() != AttentionClass::Notable {
                continue;
            }
            let owns_cycle = scenario
                .state
                .enterprises()
                .get_enterprise(cycle.enterprise())
                .is_some_and(|record| record.organization() == scenario.player);
            if !owns_cycle {
                continue;
            }
            let summary = cycle
                .information()
                .and_then(|information| scenario.state.intelligence().get_information(information))
                .map(|record| record.summary().to_owned())
                .unwrap_or_else(|| "cycle report missing".to_owned());
            println!(
                "[ENTERPRISE] {}: {}",
                stamp(outcome.now.as_minutes()),
                summary,
            );
        }
        if let Some(report) = outcome.executive_brief {
            let report = scenario
                .state
                .reports()
                .get_report(report)
                .expect("executive brief must persist");
            println!(
                "[BRIEF GENERATED] {}: executive brief with {} player-visible entr{}; full brief appears in the final recap.",
                stamp(outcome.now.as_minutes()),
                report.entries().len(),
                if report.entries().len() == 1 { "y" } else { "ies" },
            );
        }
    }
    if tick_changed_observable_state(outcome) {
        validate_harness_state(scenario.registry, &scenario.state)?;
    }
    Ok(())
}

/// Explains why an autonomous recruitment attempt landed or failed, connecting it to this
/// session's player-visible events. `[NARRATION]` is the harness's documentary voice: it may
/// explain world causality, but it never feeds action selection and never reads hidden
/// case/evidence state.
pub fn narrate_recruitment_causality(
    scenario: &Scenario,
    metrics: &RunMetrics,
    attempt: &crimocracy::recruitment::RecruitmentAttemptRecord,
    candidate: &crimocracy::world::CharacterRecord,
) {
    let candidate_name = candidate.name();
    let crew_role = metrics
        .burglary
        .and_then(|operation| scenario.state.operations().get_operation(operation))
        .and_then(|operation| {
            operation
                .roles()
                .iter()
                .find_map(|(role, member)| (*member == attempt.candidate()).then_some(*role))
        });
    let operation_title = metrics
        .burglary
        .and_then(|operation| scenario.state.operations().get_operation(operation))
        .map(|operation| operation.title().to_owned());
    let accepted = attempt.outcome() == crimocracy::recruitment::RecruitmentOutcome::Accepted;
    match (
        accepted,
        metrics.police_arrived,
        crew_role,
        operation_title,
    ) {
        (true, true, Some(role), Some(title)) => println!(
            "[NARRATION] {candidate_name} was on the {title} crew when police arrived. That direct contact is the lever a rival's {:?} pitch exploited; the organization loses its {} for burglary work.",
            attempt.approach(),
            role_label(role),
        ),
        (true, true, _, Some(title)) => println!(
            "[NARRATION] Police contact during the {title} operation opened the pressure window a rival's {:?} pitch exploited.",
            attempt.approach(),
        ),
        (true, _, _, _) => println!(
            "[NARRATION] {candidate_name} left the organization even without a revealed police contact this session; the rival's {:?} approach found another opening.",
            attempt.approach(),
        ),
        (false, false, _, _) => println!(
            "[NARRATION] {candidate_name} was not exposed to police this session, so the rival's {:?} pitch carried no immediate legal-pressure lever and it failed. The organization keeps its personnel.",
            attempt.approach(),
        ),
        (false, true, _, _) => println!(
            "[NARRATION] {candidate_name} refused the rival's {:?} pitch despite this session's police contact; the organization keeps its personnel.",
            attempt.approach(),
        ),
    }
}

/// True when the tick produced any transaction a player could observe or that persists state.
/// The harness validates the whole world at these consequential boundaries; skipping fully routine
/// minutes keeps the matched-batch lane fast without losing corruption coverage at any real event.
pub fn tick_changed_observable_state(outcome: &TickOutcome) -> bool {
    !outcome.started_operations.is_empty()
        || !outcome.arrived_police_responses.is_empty()
        || !outcome.decision_requests.is_empty()
        || !outcome.resolved_operations.is_empty()
        || !outcome.staffed_investigations.is_empty()
        || !outcome.scheduled_investigation_work.is_empty()
        || !outcome.scheduled_witness_interviews.is_empty()
        || !outcome.resolved_investigation_work.is_empty()
        || !outcome.evidence_arrests.is_empty()
        || !outcome.informant_recruitments.is_empty()
        || !outcome.informant_disclosures.is_empty()
        || !outcome.automatic_legal_support.is_empty()
        || !outcome.business_cycles.is_empty()
        || !outcome.enterprise_cycles.is_empty()
        || !outcome.recruitment_attempts.is_empty()
        || !outcome.expired_opportunities.is_empty()
        || !outcome.cold_case_suspensions.is_empty()
        || !outcome.cold_case_closures.is_empty()
        || outcome.executive_brief.is_some()
}

pub fn choose_safe_start_from_patrol_report(
    now: SimTime,
    report: &str,
    operation_duration: SimDuration,
    uncertainty_buffer: SimDuration,
    latest_start: SimTime,
) -> Result<SimTime, HarnessContractError> {
    let windows = parse_patrol_windows(report);
    if windows.is_empty() {
        return Err(HarnessContractError::NoActionablePatrolWindows);
    }
    let duration = u64::from(operation_duration.as_minutes());
    let buffer = u64::from(uncertainty_buffer.as_minutes());
    let earliest = now.as_minutes().saturating_add(1);
    let latest = latest_start.as_minutes().saturating_sub(duration);
    let first_candidate = earliest.div_ceil(30).saturating_mul(30);
    for candidate in (first_candidate..=latest)
        .step_by(30)
        .take_while(|candidate| *candidate < first_candidate.saturating_add(2_880))
    {
        let operation_start = candidate % 1_440;
        let operation_end = operation_start.saturating_add(duration);
        if operation_end > 1_440 {
            continue;
        }
        let overlaps_buffered_patrol = windows.iter().any(|(start, end)| {
            let buffered_start = start.saturating_sub(buffer);
            let buffered_end = end.saturating_add(buffer).min(1_440);
            intervals_overlap(operation_start, operation_end, buffered_start, buffered_end)
        });
        if !overlaps_buffered_patrol {
            return Ok(SimTime::from_minutes(candidate));
        }
    }
    Err(HarnessContractError::NoSafeOperationWindow)
}

pub fn parse_patrol_windows(report: &str) -> Vec<(u64, u64)> {
    let mut windows = Vec::new();
    let mut remaining = report;
    while let Some(index) = remaining.find("roughly ") {
        remaining = &remaining[index + "roughly ".len()..];
        let Some(start) = remaining.get(0..5).and_then(parse_clock_minute) else {
            break;
        };
        if remaining.get(5..6) != Some("-") {
            break;
        }
        let Some(end) = remaining.get(6..11).and_then(parse_clock_minute) else {
            break;
        };
        if start < end {
            windows.push((start, end));
        } else if start > end {
            windows.push((start, 1_440));
            if end > 0 {
                windows.push((0, end));
            }
        } else {
            windows.push((0, 1_440));
        }
        remaining = remaining.get(11..).unwrap_or_default();
    }
    windows
}

pub fn parse_clock_minute(value: &str) -> Option<u64> {
    let (hour, minute) = value.split_once(':')?;
    let hour = hour.parse::<u64>().ok()?;
    let minute = minute.parse::<u64>().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

pub fn intervals_overlap(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> bool {
    start_a < end_b && start_b < end_a
}
