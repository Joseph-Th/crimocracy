//! Tick observation: metrics capture, narration, and typed patrol-sightline interpretation.

use crimocracy::core::attention::AttentionClass;
use crimocracy::core::simulation::TickOutcome;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::decisions::{DecisionContext, DecisionResponse};
use crimocracy::finance::{AccountKind, FinancialOwner};
use crimocracy::intelligence::intelligence_system::validate_information_transfer;
use crimocracy::intelligence::{
    InformationSignal, InformationTopic, InformationTransferDraft, KnowledgeHolder,
};
use std::error::Error;

use crate::*;

pub fn observe_tick(
    scenario: &mut Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    observe_staffing_and_payroll(scenario, outcome, narrative, metrics);
    observe_enterprise_evidence(scenario, outcome, metrics);
    narrate_started_operations(scenario, outcome, narrative);
    observe_cold_case_and_opportunity_cost(scenario, outcome, narrative, metrics);
    observe_burglary_police_response(scenario, outcome, metrics);
    resolve_decision_requests(scenario, outcome, narrative, metrics)?;
    transfer_press_police_observations(scenario, outcome, narrative, metrics)?;
    observe_recruitment(scenario, outcome, narrative, metrics);
    observe_investigation_and_arrests(scenario, outcome, narrative, metrics);
    narrate_resolutions_and_enterprise_cycles(scenario, outcome, narrative);

    if tick_changed_observable_state(outcome) {
        validate_harness_state(scenario.registry, &scenario.state)?;
    }
    Ok(())
}

fn observe_staffing_and_payroll(
    scenario: &Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) {
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
        if let Some(transaction) = payroll.transaction() {
            let transaction = scenario
                .state
                .finance()
                .get_transaction(transaction)
                .expect("payroll outcome transaction must persist");
            let accounted_debit = transaction
                .postings()
                .iter()
                .filter_map(|posting| {
                    let account = scenario
                        .state
                        .finance()
                        .get_account(posting.account)
                        .expect("payroll posting account must persist");
                    (posting.amount.cents() < 0
                        && account.owner() == FinancialOwner::Organization(scenario.player)
                        && account.kind() == AccountKind::AccountedFunds)
                        .then(|| posting.amount.cents().checked_neg())
                        .flatten()
                })
                .try_fold(0_i64, i64::checked_add)
                .expect("accounted payroll debit total must fit money range");
            metrics.payroll_accounted_spent_cents = metrics
                .payroll_accounted_spent_cents
                .checked_add(accounted_debit)
                .expect("session accounted payroll total must fit money range");
        }
        if narrative && payroll.short().cents() > 0 {
            println!(
                "[PAYROLL]  {}: the day's wages went unpaid ({} owed). The crew will remember.",
                stamp(outcome.now.as_minutes()),
                format_cents(payroll.owed().cents())
            );
        }
    }
}

fn observe_enterprise_evidence(
    scenario: &Scenario,
    outcome: &TickOutcome,
    metrics: &mut RunMetrics,
) {
    // Vice-attention evidence is counted for every session regardless of narration: sustained
    // district casework converting into an inquiry on a player-owned racket is a core
    // consequence-loop signal, and batch aggregates track how often it lands.
    for cycle_id in &outcome.enterprise_cycles {
        let drew_vice = scenario
            .state
            .enterprises()
            .get_cycle(*cycle_id)
            .is_some_and(|cycle| {
                cycle.drew_vice_attention()
                    && scenario
                        .state
                        .enterprises()
                        .get_enterprise(cycle.enterprise())
                        .is_some_and(|record| record.organization() == scenario.player)
            });
        if drew_vice {
            metrics.vice_inquiries_drawn += 1;
        }
    }
}

fn narrate_started_operations(scenario: &Scenario, outcome: &TickOutcome, narrative: bool) {
    if !narrative {
        return;
    }
    for operation in &outcome.started_operations {
        let record = scenario
            .state
            .operations()
            .get_operation(*operation)
            .expect("started operation must exist");
        if record.responsible_organization() != scenario.player {
            continue;
        }
        println!(
            "[START]   {}: {} started.",
            stamp(outcome.now.as_minutes()),
            record.title()
        );
    }
}

fn observe_cold_case_and_opportunity_cost(
    scenario: &Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) {
    // A cold-case shelf or closure is an institutional beat, not player-visible news. Capture it
    // only as contract evidence; the narrative waits until the organization learns the change
    // through its own surveillance/contact channels.
    if let Some(case) = scenario.investigation {
        let shelved = outcome.cold_case_suspensions.contains(&case);
        let closed = outcome.cold_case_closures.contains(&case);
        if shelved || closed {
            metrics.case_cold_minute = Some(outcome.now.as_minutes());
        }
    }

    // The second score's lapse is a deliberate, observed consequence: PRESS stands down while the
    // case is hot and the opportunity expires through the canonical lifecycle, generating its own
    // report instead of any hidden-state read.
    if let Some(opportunity) = metrics.second_opportunity
        && outcome.expired_opportunities.contains(&opportunity)
    {
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

fn observe_burglary_police_response(
    scenario: &Scenario,
    outcome: &TickOutcome,
    metrics: &mut RunMetrics,
) {
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
}

fn resolve_decision_requests(
    scenario: &mut Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
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
            // PRESS's defining choice is to press on through a police response on the score
            // itself. It is not a blanket suicide pact: any later operation's police-arrival
            // exception gets the standing abort, because the branch is standing down.
            DecisionContext::OperationPoliceArrival { operation, .. }
                if metrics.strategy == Some(Strategy::Press)
                    && Some(operation) == metrics.burglary =>
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
    Ok(())
}

fn transfer_press_police_observations(
    scenario: &mut Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
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
    Ok(())
}

fn observe_recruitment(
    scenario: &Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) {
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
        if narrative && attempt.previous_organization() == Some(scenario.player) {
            // Recruitment attempts against other organizations are hidden world activity. For
            // our own member, narrate only the production report delivered to leadership: an
            // accepted defection reveals the departure but not the rival destination, while a
            // refusal names the outside recruiter through the loyalty-report path.
            let report = scenario
                .state
                .reports()
                .get_report(attempt.member_report().expect(
                    "recruitment involving a current player member must link its personnel report",
                ))
                .expect("linked recruitment member report must persist");
            let label = match attempt.outcome() {
                crimocracy::recruitment::RecruitmentOutcome::Accepted => "PERSONNEL",
                crimocracy::recruitment::RecruitmentOutcome::Refused => "POACH WARNING",
            };
            for entry in report.entries() {
                println!(
                    "[{label}] {}: {}",
                    stamp(outcome.now.as_minutes()),
                    entry.summary
                );
            }
        }
    }
}

fn observe_investigation_and_arrests(
    scenario: &Scenario,
    outcome: &TickOutcome,
    narrative: bool,
    metrics: &mut RunMetrics,
) {
    metrics.investigation_work_scheduled = metrics.investigation_work_scheduled.saturating_add(
        u32::try_from(outcome.scheduled_investigation_work.len()).unwrap_or(u32::MAX),
    );
    metrics.investigation_work_resolved = metrics.investigation_work_resolved.saturating_add(
        u32::try_from(outcome.resolved_investigation_work.len()).unwrap_or(u32::MAX),
    );

    // Witness interviews are institutional work against the case's named witness: hidden
    // from the organization until its channels read the case, but counted as audit evidence
    // of the testimony chain.
    metrics.witness_interviews_scheduled = metrics.witness_interviews_scheduled.saturating_add(
        u32::try_from(outcome.scheduled_witness_interviews.len()).unwrap_or(u32::MAX),
    );

    // An autonomous evidence-threshold arrest is a production custody event. The organization
    // learns of an arrested member through custody and representation channels; here it is
    // observed for what it is and counted per side so the consequence stays legible.
    for arrest_id in &outcome.evidence_arrests {
        let Some(arrest) = scenario.state.legal().get_arrest(*arrest_id) else {
            continue;
        };
        let member_of_player = scenario
            .state
            .world()
            .get_character(arrest.character())
            .and_then(|character| character.organization())
            .is_some_and(|organization| organization == scenario.player);
        if !member_of_player {
            continue;
        }
        metrics.player_member_arrests = metrics.player_member_arrests.saturating_add(1);
        if narrative {
            let name = scenario
                .state
                .world()
                .get_character(arrest.character())
                .map(|character| character.name().to_owned())
                .unwrap_or_default();
            println!(
                "[ARREST]    {}: {} was taken into custody on the strength of the case file.",
                stamp(outcome.now.as_minutes()),
                name,
            );
        }
    }
}

fn narrate_resolutions_and_enterprise_cycles(
    scenario: &Scenario,
    outcome: &TickOutcome,
    narrative: bool,
) {
    if !narrative {
        return;
    }

    for operation in &outcome.resolved_operations {
        let record = scenario
            .state
            .operations()
            .get_operation(*operation)
            .expect("resolved operation must persist");
        if record.responsible_organization() != scenario.player {
            continue;
        }
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

    narrate_routine_cycle_summary(scenario, outcome);
    narrate_notable_enterprise_cycles(scenario, outcome);
}

fn narrate_routine_cycle_summary(scenario: &Scenario, outcome: &TickOutcome) {
    let player_business_cycles = outcome
        .business_cycles
        .iter()
        .filter(|cycle_id| {
            scenario
                .state
                .economy()
                .get_cycle(**cycle_id)
                .is_some_and(|cycle| {
                    cycle.owner() == crimocracy::world::BusinessOwner::Organization(scenario.player)
                })
        })
        .count();
    let player_enterprise_cycles = outcome
        .enterprise_cycles
        .iter()
        .filter(|cycle_id| {
            scenario
                .state
                .enterprises()
                .get_cycle(**cycle_id)
                .and_then(|cycle| {
                    scenario
                        .state
                        .enterprises()
                        .get_enterprise(cycle.enterprise())
                })
                .is_some_and(|enterprise| enterprise.organization() == scenario.player)
        })
        .count();
    if player_business_cycles == 0
        && player_enterprise_cycles == 0
        && outcome.executive_brief.is_none()
    {
        return;
    }

    // Routine world beats stay quiet unless something above routine attention happened:
    // repeating identical all-quiet lines every simulated day buries the actual story beats.
    // Notable cycle reports print below; brief contents appear in the closing recap.
    let player_notable_cycle = outcome.enterprise_cycles.iter().any(|cycle_id| {
        scenario
            .state
            .enterprises()
            .get_cycle(*cycle_id)
            .is_some_and(|cycle| {
                cycle.attention() == AttentionClass::Notable
                    && scenario
                        .state
                        .enterprises()
                        .get_enterprise(cycle.enterprise())
                        .is_some_and(|record| record.organization() == scenario.player)
            })
    });
    let brief_deserves_attention = outcome
        .executive_brief
        .and_then(|report| scenario.state.reports().get_report(report))
        .is_some_and(|report| {
            report
                .entries()
                .iter()
                .any(|entry| !matches!(entry.attention, AttentionClass::Routine))
        });
    if player_notable_cycle || brief_deserves_attention {
        println!(
            "[ROUTINE] {}: {} legitimate business cycle(s), {} delegated enterprise cycle(s); daily brief delivered.",
            stamp(outcome.now.as_minutes()),
            player_business_cycles,
            player_enterprise_cycles,
        );
    }
}

fn narrate_notable_enterprise_cycles(scenario: &Scenario, outcome: &TickOutcome) {
    // A notable cycle carries its manager's report as organization-held information; the
    // narrative surfaces it so heat-driven cost pressure is legible when it happens. Only this
    // organization's cycles are player-visible: another organization's manager report is
    // information they hold, not something our leadership can read.
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

pub fn choose_safe_start_from_patrol_signal(
    now: SimTime,
    signal: &InformationSignal,
    operation_duration: SimDuration,
    uncertainty_buffer: SimDuration,
    latest_start: SimTime,
) -> Result<SimTime, HarnessContractError> {
    let windows = patrol_intervals_from_signal(signal);
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

pub fn patrol_intervals_from_signal(signal: &InformationSignal) -> Vec<(u64, u64)> {
    let InformationSignal::PatrolPattern { intervals } = signal else {
        return Vec::new();
    };
    intervals
        .iter()
        .map(|interval| {
            (
                u64::from(interval.start_minute()),
                u64::from(interval.end_minute()),
            )
        })
        .collect()
}

pub fn intervals_overlap(start_a: u64, end_a: u64, start_b: u64, end_b: u64) -> bool {
    start_a < end_b && start_b < end_a
}
