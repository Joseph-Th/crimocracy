//! Full-mode rival discovery and follow-up through player-held surveillance observations only.

use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, InformationId, OperationId};
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::economy::BusinessEconomyDraft;
use crimocracy::economy::business_economy_system::validate_establish_business_economy;
use crimocracy::finance::finance_system::insert_account;
use crimocracy::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
use crimocracy::intelligence::{
    EnterpriseLocationSignal, InformationSignal, InformationSourceKind, InformationTopic,
    KnowledgeHolder, Reliability, Specificity,
};
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::{
    OperationApproach, OperationConstraint, OperationContingency, OperationDraft,
    OperationExposureLevel, OperationKind, OperationObjective, OperationObjectiveOutcome,
    OperationStatus, RoleKind,
};
use crimocracy::registry::Registry;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, authorize_surveillance_target,
    build_scenario, format_cents, run_until, run_until_operation_terminal, stamp,
    validate_harness_state,
};

#[derive(Debug, Serialize)]
struct Observation {
    information: InformationId,
    holder: KnowledgeHolder,
    subject: EntityRef,
    topic: InformationTopic,
    observed_minute: u64,
    recorded_minute: u64,
    source_kind: InformationSourceKind,
    source_entity: Option<EntityRef>,
    derived_from: BTreeSet<InformationId>,
    reliability: Reliability,
    specificity: Specificity,
    signal: Option<InformationSignal>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct WatchEvidence {
    operation: OperationId,
    target: EntityRef,
    scheduled_minute: u64,
    terminal_minute: u64,
    status: OperationStatus,
    outcome: Option<OperationObjectiveOutcome>,
    exposure: Option<OperationExposureLevel>,
    source_information: BTreeSet<InformationId>,
    after_action: Option<Observation>,
    observations: Vec<Observation>,
}

#[derive(Debug, Serialize)]
struct BusinessCycleEvidence {
    occurred_minute: u64,
    disrupted: bool,
    gross_revenue_cents: i64,
    net_cash_cents: i64,
}

#[derive(Debug, Serialize)]
struct InterventionEvidence {
    /// The player-held enterprise observation whose typed location chose this venue. It is
    /// decision provenance, not attached operation intelligence because its subject is the
    /// enterprise rather than the business objective.
    location_source: InformationId,
    target: BusinessId,
    operation: OperationId,
    scheduled_minute: u64,
    terminal_minute: u64,
    status: OperationStatus,
    outcome: Option<OperationObjectiveOutcome>,
    exposure: Option<OperationExposureLevel>,
    planning_information: BTreeSet<InformationId>,
    constraints: Vec<OperationConstraint>,
    contingencies: Vec<OperationContingency>,
    after_action: Option<Observation>,
}

#[derive(Debug, Serialize)]
struct InterventionEvaluation {
    /// Structural verification only. These rival books are not automatically visible to the
    /// player and must never drive the acting policy above.
    disruption_active_after_operation: bool,
    actual_next_cycle: Option<BusinessCycleEvidence>,
    matched_no_intervention_cycle: Option<BusinessCycleEvidence>,
    gross_reduction_cents: Option<i64>,
    net_reduction_cents: Option<i64>,
}

#[derive(Debug, Serialize)]
struct PlayerVisibleEvidence {
    discovery: WatchEvidence,
    selected_source: Option<InformationId>,
    followup: Option<WatchEvidence>,
    intervention: Option<InterventionEvidence>,
    absence: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ProbeEvidence {
    world_seed: u64,
    policy_seed: u64,
    expansion_boundary_minute: u64,
    timing_policy: &'static str,
    player_visible: PlayerVisibleEvidence,
    evaluation: Option<InterventionEvaluation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RivalProbeSummary {
    pub actionable_intervention: bool,
    pub material_economic_impact: bool,
}

/// One organization watch, then at most one exact-subject enterprise watch. Failed or partial
/// discovery is evidence of absence, not permission to enumerate the rival's hidden rackets.
pub fn run_rival_intelligence_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
    artifact_dir: Option<&Path>,
) -> Result<RivalProbeSummary, Box<dyn Error>> {
    let evidence = collect_probe(registry, seeds)?;
    let summary = RivalProbeSummary {
        actionable_intervention: evidence.player_visible.intervention.is_some(),
        material_economic_impact: evidence.evaluation.as_ref().is_some_and(|evaluation| {
            evaluation
                .gross_reduction_cents
                .is_some_and(|reduction| reduction > 0)
                && evaluation
                    .net_reduction_cents
                    .is_some_and(|reduction| reduction > 0)
        }),
    };
    if detail {
        println!(
            "[RIVAL INTELLIGENCE] world {:#x}, policy {:#x}; canonical NightTrap fixture, one daily expansion boundary. {}",
            seeds.world, seeds.policy, evidence.timing_policy,
        );
        print_watch("DISCOVERY", &evidence.player_visible.discovery);
        if let Some(followup) = &evidence.player_visible.followup {
            println!(
                "[DECIDE] Follow the first observed enterprise, attaching source {:?}; no hidden racket inventory was consulted.",
                evidence.player_visible.selected_source,
            );
            print_watch("FOLLOW-UP", followup);
        }
        if let Some(intervention) = &evidence.player_visible.intervention {
            println!(
                "[DECIDE] Typed enterprise-location fact {:?} identifies business {:?}. Use only focused player-held local intelligence {:?} to plan sabotage.",
                intervention.location_source,
                intervention.target,
                intervention.planning_information,
            );
            println!(
                "[INTERVENE] {:?} -> Business({:?}), {:?}, outcome {:?}, exposure {:?}; scheduled {}, terminal {}; plan requires local police intelligence and aborts on a pre-entry police arrival.",
                intervention.operation,
                intervention.target,
                intervention.status,
                intervention.outcome,
                intervention.exposure,
                stamp(intervention.scheduled_minute),
                stamp(intervention.terminal_minute),
            );
            if let Some(after_action) = &intervention.after_action {
                println!(
                    "[LEARN] {:?}: {}",
                    after_action.information, after_action.summary
                );
            }
        }
        if let Some(evaluation) = &evidence.evaluation
            && let (Some(actual), Some(control), Some(gross_reduction), Some(net_reduction)) = (
                &evaluation.actual_next_cycle,
                &evaluation.matched_no_intervention_cycle,
                evaluation.gross_reduction_cents,
                evaluation.net_reduction_cents,
            )
        {
            println!(
                "[EVALUATE] Matched no-intervention rival cycle at {}: gross {}, net {}. After intervention: gross {}, net {}, disrupted={}. Economic impact: gross {}, net {}. This counterfactual is harness-only evaluation and did not inform player policy.",
                stamp(control.occurred_minute),
                format_cents(control.gross_revenue_cents),
                format_cents(control.net_cash_cents),
                format_cents(actual.gross_revenue_cents),
                format_cents(actual.net_cash_cents),
                actual.disrupted,
                format_cents(-gross_reduction),
                format_cents(-net_reduction),
            );
        }
        if let Some(absence) = evidence.player_visible.absence {
            println!("[OBSERVED ABSENCE] {absence}");
        }
    }
    if let Some(directory) = artifact_dir {
        let path = persist_evidence(directory, &evidence)?;
        if detail {
            println!("[ARTIFACT] wrote {}", path.display());
        }
    }
    Ok(summary)
}

fn collect_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
) -> Result<ProbeEvidence, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut metrics = RunMetrics::default();
    establish_rival_venue_economy(registry, &mut scenario)?;
    let boundary = u64::from(
        registry
            .recruitment()
            .autonomous_attempt_cadence()
            .as_minutes(),
    );
    run_until(
        &mut scenario,
        SimTime::from_minutes(boundary),
        false,
        &mut metrics,
    )?;

    // The organization identity belongs to the known fixture. No enterprise identity does.
    let rival = EntityRef::Organization(scenario.rival);
    let discovery = run_watch(
        &mut scenario,
        rival,
        "Known rival personnel watch",
        BTreeSet::new(),
        &mut metrics,
    )?;
    let discovered: BTreeSet<_> = discovery
        .observations
        .iter()
        .map(|item| item.information)
        .collect();
    let selected = select_observed_enterprise(&scenario, &discovered);
    let followup = match selected {
        Some((target, source)) => Some(run_watch(
            &mut scenario,
            target,
            "Follow observed rival racket",
            BTreeSet::from([source]),
            &mut metrics,
        )?),
        None => None,
    };
    let intervention_target = selected.and_then(|(_, source)| {
        select_observed_business_location(&scenario, source).map(|target| (target, source))
    });
    let matched_no_intervention_cycle = intervention_target
        .map(|(target, _)| matched_next_business_cycle_without_intervention(&scenario, target))
        .transpose()?
        .flatten();
    let intervention = match (intervention_target, followup.as_ref()) {
        (Some((target, location_source)), Some(followup)) => Some(run_intervention(
            &mut scenario,
            target,
            location_source,
            followup,
            &mut metrics,
        )?),
        _ => None,
    };
    let evaluation = intervention
        .as_ref()
        .map(|intervention| {
            evaluate_intervention(
                &mut scenario,
                intervention.target,
                matched_no_intervention_cycle,
                &mut metrics,
            )
        })
        .transpose()?;
    validate_harness_state(registry, &scenario.state)?;
    Ok(ProbeEvidence {
        world_seed: seeds.world,
        policy_seed: seeds.policy,
        expansion_boundary_minute: boundary,
        timing_policy: "Each watch starts 30 minutes after readiness; this is bounded evaluation timing, not a claim of patrol safety. Policy seed is retained but does not vary this fixed treatment.",
        player_visible: PlayerVisibleEvidence {
            discovery,
            selected_source: selected.map(|(_, source)| source),
            followup,
            intervention,
            absence: selected.is_none().then_some(
                "The organization watch produced no player-held EnterpriseActivity observation with an Enterprise subject. No follow-up was authorized; this does not establish that the rival has no rackets.",
            ),
        },
        evaluation,
    })
}

fn establish_rival_venue_economy(
    registry: &Registry,
    scenario: &mut Scenario,
) -> Result<(), Box<dyn Error>> {
    let operating_account = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(scenario.rival_venue),
            kind: AccountKind::LegitimateOperating,
        },
    )?;
    let settlement_account = insert_account(
        &mut scenario.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Business(scenario.rival_venue),
            kind: AccountKind::Settlement,
        },
    )?;
    validate_establish_business_economy(
        registry,
        &scenario.state,
        BusinessEconomyDraft {
            business: scenario.rival_venue,
            operating_account,
            settlement_account,
        },
    )?
    .commit(&mut scenario.state)?;
    Ok(())
}

/// Stable first information ID among this watch's discoveries, never among hidden enterprises.
fn select_observed_enterprise(
    scenario: &Scenario,
    discovered: &BTreeSet<InformationId>,
) -> Option<(EntityRef, InformationId)> {
    discovered.iter().find_map(|id| {
        let information = scenario
            .state
            .intelligence()
            .get_information(*id)
            .expect("discovered information must persist");
        (information.holder() == KnowledgeHolder::Organization(scenario.player)
            && information.topic() == InformationTopic::EnterpriseActivity
            && matches!(information.subject(), EntityRef::Enterprise(_)))
        .then_some((information.subject(), *id))
    })
}

fn select_observed_business_location(
    scenario: &Scenario,
    source: InformationId,
) -> Option<BusinessId> {
    let information = scenario.state.intelligence().get_information(source)?;
    if information.holder() != KnowledgeHolder::Organization(scenario.player)
        || information.topic() != InformationTopic::EnterpriseActivity
        || !matches!(information.subject(), EntityRef::Enterprise(_))
    {
        return None;
    }
    match information.signal() {
        Some(InformationSignal::EnterpriseLocation(EnterpriseLocationSignal::Business(
            business,
        ))) => Some(*business),
        _ => None,
    }
}

fn observe_information(
    scenario: &Scenario,
    id: InformationId,
) -> Result<Observation, Box<dyn Error>> {
    let information = scenario
        .state
        .intelligence()
        .get_information(id)
        .ok_or("watch information did not persist")?;
    if information.holder() != KnowledgeHolder::Organization(scenario.player) {
        return Err("watch evidence must be held by the player organization".into());
    }
    Ok(Observation {
        information: id,
        holder: information.holder(),
        subject: information.subject(),
        topic: information.topic(),
        observed_minute: information.observed_at().as_minutes(),
        recorded_minute: information.recorded_at().as_minutes(),
        source_kind: information.source_kind(),
        source_entity: information.source_entity(),
        derived_from: information.derived_from().clone(),
        reliability: information.reliability(),
        specificity: information.specificity(),
        signal: information.signal().cloned(),
        // Quote the persisted observation exactly. Do not supplement a missing racket kind,
        // location, manager, or financial detail from the foreign enterprise's hidden record.
        summary: information.summary().to_owned(),
    })
}

fn matched_next_business_cycle_without_intervention(
    scenario: &Scenario,
    target: BusinessId,
) -> Result<Option<BusinessCycleEvidence>, Box<dyn Error>> {
    let mut control = scenario.clone();
    let Some(next_cycle_at) = control
        .state
        .economy()
        .get_business_economy(target)
        .and_then(|economy| economy.next_cycle_at())
    else {
        return Ok(None);
    };
    let mut metrics = RunMetrics::default();
    run_until(&mut control, next_cycle_at, false, &mut metrics)?;
    Ok(control
        .state
        .economy()
        .latest_cycle(target)
        .map(business_cycle_evidence))
}

fn business_cycle_evidence(
    cycle: &crimocracy::economy::BusinessCycleRecord,
) -> BusinessCycleEvidence {
    BusinessCycleEvidence {
        occurred_minute: cycle.occurred_at().as_minutes(),
        disrupted: cycle.disrupted(),
        gross_revenue_cents: cycle.gross_revenue().cents(),
        net_cash_cents: cycle.net_cash().cents(),
    }
}

fn run_intervention(
    scenario: &mut Scenario,
    target: BusinessId,
    location_source: InformationId,
    followup: &WatchEvidence,
    metrics: &mut RunMetrics,
) -> Result<InterventionEvidence, Box<dyn Error>> {
    let planning_information = followup
        .observations
        .iter()
        .filter(|observation| observation.topic == InformationTopic::PoliceActivity)
        .map(|observation| observation.information)
        .collect::<BTreeSet<_>>();
    let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
    let operation = validate_authorize_operation(
        scenario.registry,
        &scenario.state,
        OperationDraft {
            title: "Disrupt observed rival venue".to_owned(),
            kind: OperationKind::Sabotage,
            responsible_organization: scenario.player,
            leader: scenario.lieutenant,
            objective: OperationObjective::DisruptBusiness {
                target: EntityRef::Business(target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.lieutenant),
                (RoleKind::EntrySpecialist, scenario.burglar),
            ]),
            intelligence: planning_information.clone(),
            constraints: vec![OperationConstraint::RequireIntelligenceTopic(
                InformationTopic::PoliceActivity,
            )],
            contingencies: vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
            scheduled_for,
        },
    )?
    .commit(&mut scenario.state)?;
    run_until_operation_terminal(scenario, operation, false, metrics)?;

    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .ok_or("authorized intervention did not persist")?;
    let resolution = record.resolution();
    let after_action = resolution
        .map(|result| observe_information(scenario, result.after_action_information()))
        .transpose()?;
    let status = record.status();
    let outcome = resolution.map(|result| result.objective_outcome());
    let exposure = resolution.map(|result| result.exposure().level());
    let terminal_minute = scenario.state.now().as_minutes();
    let constraints = record.constraints().to_vec();
    let contingencies = record.contingencies().to_vec();

    Ok(InterventionEvidence {
        location_source,
        target,
        operation,
        scheduled_minute: scheduled_for.as_minutes(),
        terminal_minute,
        status,
        outcome,
        exposure,
        planning_information,
        constraints,
        contingencies,
        after_action,
    })
}

fn evaluate_intervention(
    scenario: &mut Scenario,
    target: BusinessId,
    matched_no_intervention_cycle: Option<BusinessCycleEvidence>,
    metrics: &mut RunMetrics,
) -> Result<InterventionEvaluation, Box<dyn Error>> {
    let disruption_active_after_operation = scenario
        .state
        .economy()
        .get_business_economy(target)
        .is_some_and(|economy| economy.is_disrupted(scenario.state.now()));
    let next_cycle_at = scenario
        .state
        .economy()
        .get_business_economy(target)
        .and_then(|economy| economy.next_cycle_at());
    let actual_next_cycle = if let Some(next_cycle_at) = next_cycle_at {
        run_until(scenario, next_cycle_at, false, metrics)?;
        scenario
            .state
            .economy()
            .latest_cycle(target)
            .map(business_cycle_evidence)
    } else {
        None
    };
    let gross_reduction_cents = actual_next_cycle
        .as_ref()
        .zip(matched_no_intervention_cycle.as_ref())
        .map(|(actual, control)| control.gross_revenue_cents - actual.gross_revenue_cents);
    let net_reduction_cents = actual_next_cycle
        .as_ref()
        .zip(matched_no_intervention_cycle.as_ref())
        .map(|(actual, control)| control.net_cash_cents - actual.net_cash_cents);
    Ok(InterventionEvaluation {
        disruption_active_after_operation,
        actual_next_cycle,
        matched_no_intervention_cycle,
        gross_reduction_cents,
        net_reduction_cents,
    })
}

fn run_watch(
    scenario: &mut Scenario,
    target: EntityRef,
    title: &str,
    intelligence: BTreeSet<InformationId>,
    metrics: &mut RunMetrics,
) -> Result<WatchEvidence, Box<dyn Error>> {
    let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
    let operation =
        authorize_surveillance_target(scenario, target, title, scheduled_for, intelligence)?;
    run_until_operation_terminal(scenario, operation, false, metrics)?;
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .ok_or("authorized watch did not persist")?;
    let resolution = record.resolution();
    let observations = resolution
        .map(|result| {
            result
                .discovered_information()
                .iter()
                .map(|id| observe_information(scenario, *id))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let after_action = resolution
        .map(|result| observe_information(scenario, result.after_action_information()))
        .transpose()?;
    Ok(WatchEvidence {
        operation,
        target,
        scheduled_minute: record.scheduled_for().as_minutes(),
        terminal_minute: scenario.state.now().as_minutes(),
        status: record.status(),
        outcome: resolution.map(|result| result.objective_outcome()),
        // Deliberately project only crew-observed exposure, never the raw exposure record's
        // institutional investigation/evidence IDs or hidden resolution factors.
        exposure: resolution.map(|result| result.exposure().level()),
        source_information: record.intelligence().clone(),
        after_action,
        observations,
    })
}

fn print_watch(label: &str, watch: &WatchEvidence) {
    println!(
        "[{label}] {:?} -> {:?}, {:?}, outcome {:?}, exposure {:?}; scheduled {}, terminal {}; attached sources {:?}.",
        watch.operation,
        watch.target,
        watch.status,
        watch.outcome,
        watch.exposure,
        stamp(watch.scheduled_minute),
        stamp(watch.terminal_minute),
        watch.source_information,
    );
    for observation in watch.after_action.iter().chain(&watch.observations) {
        println!(
            "[LEARN] {:?}: {:?} / {:?}, {:?} / {:?}, observed {}, recorded {}; source {:?} {:?}, lineage {:?}: {}",
            observation.information,
            observation.topic,
            observation.subject,
            observation.reliability,
            observation.specificity,
            stamp(observation.observed_minute),
            stamp(observation.recorded_minute),
            observation.source_kind,
            observation.source_entity,
            observation.derived_from,
            observation.summary,
        );
    }
}

fn persist_evidence(directory: &Path, evidence: &ProbeEvidence) -> Result<PathBuf, Box<dyn Error>> {
    let bytes = serde_json::to_vec_pretty(evidence)?;
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!(
        "rival-intelligence-w{:016x}-p{:016x}.json",
        evidence.world_seed, evidence.policy_seed,
    ));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_chain_follows_only_the_first_discovered_enterprise() {
        let registry = crimocracy::build_registry();
        let evidence =
            collect_probe(&registry, EvaluationSeeds::defaults()).expect("probe completes");
        let visible = &evidence.player_visible;
        assert_eq!(
            visible.discovery.outcome,
            Some(OperationObjectiveOutcome::Achieved)
        );
        let first = visible
            .discovery
            .observations
            .iter()
            .find(|item| {
                item.topic == InformationTopic::EnterpriseActivity
                    && matches!(item.subject, EntityRef::Enterprise(_))
            })
            .expect("default watch discovers a rival racket");
        assert_eq!(visible.selected_source, Some(first.information));
        assert_eq!(
            first.source_entity,
            Some(EntityRef::Operation(visible.discovery.operation))
        );
        let followup = visible
            .followup
            .as_ref()
            .expect("discovery enables canonical follow-up");
        assert_eq!(followup.target, first.subject);
        assert_eq!(
            followup.source_information,
            BTreeSet::from([first.information])
        );
        assert_eq!(followup.outcome, Some(OperationObjectiveOutcome::Achieved));
        assert!(followup.observations.iter().any(|item| {
            item.subject == first.subject
                && item.source_entity == Some(EntityRef::Operation(followup.operation))
        }));
        let patrol = followup
            .observations
            .iter()
            .find(|item| item.topic == InformationTopic::PoliceActivity)
            .expect("a focused racket watch adds local police intelligence");
        assert!(matches!(patrol.subject, EntityRef::Neighborhood(_)));
        assert_eq!(
            patrol.source_entity,
            Some(EntityRef::Operation(followup.operation))
        );
        assert!(matches!(
            patrol.signal,
            Some(InformationSignal::PatrolPattern { .. })
        ));
        assert!(
            visible
                .discovery
                .observations
                .iter()
                .all(|item| { item.topic != InformationTopic::PoliceActivity }),
            "broad discovery must not disclose patrols at every rival location"
        );
        assert!(first.observed_minute <= followup.scheduled_minute);
        let learned_business = match first.signal {
            Some(InformationSignal::EnterpriseLocation(EnterpriseLocationSignal::Business(
                business,
            ))) => business,
            ref other => {
                panic!("rival racket must carry an actionable business location: {other:?}")
            }
        };
        let intervention = visible
            .intervention
            .as_ref()
            .expect("typed rival location plus focused patrol intelligence enables intervention");
        assert_eq!(intervention.target, learned_business);
        assert_eq!(intervention.location_source, first.information);
        assert!(
            !intervention
                .planning_information
                .contains(&first.information),
            "enterprise-location knowledge chooses the venue but must not be smuggled into business-target operation scoring"
        );
        assert_eq!(
            intervention.planning_information,
            BTreeSet::from([patrol.information]),
            "focused local police intelligence is the only attached sabotage planning fact"
        );
        assert_eq!(
            intervention.constraints,
            vec![OperationConstraint::RequireIntelligenceTopic(
                InformationTopic::PoliceActivity
            )],
            "the demonstration must make the observed local police pattern a real authorization prerequisite"
        );
        assert_eq!(
            intervention.contingencies,
            vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
            "the intervention must preserve a standing safety reaction rather than silently absorbing a pre-entry police arrival"
        );
        assert_eq!(intervention.status, OperationStatus::Completed);
        assert_ne!(
            intervention.outcome,
            Some(OperationObjectiveOutcome::Failed)
        );
        let evaluation = evidence
            .evaluation
            .as_ref()
            .expect("a successful intervention must be evaluated against a matched continuation");
        assert!(evaluation.disruption_active_after_operation);
        let disrupted_cycle = evaluation
            .actual_next_cycle
            .as_ref()
            .expect("intervention branch must reach the rival venue's next operating cycle");
        assert!(disrupted_cycle.disrupted);
        let control_cycle = evaluation
            .matched_no_intervention_cycle
            .as_ref()
            .expect("matched no-intervention continuation must reach the same business cycle");
        assert!(!control_cycle.disrupted);
        assert_eq!(
            disrupted_cycle.occurred_minute,
            control_cycle.occurred_minute
        );
        assert!(
            evaluation
                .gross_reduction_cents
                .is_some_and(|reduction| reduction > 0),
            "sabotage must materially reduce the rival venue's next-cycle gross"
        );
        assert!(
            evaluation
                .net_reduction_cents
                .is_some_and(|reduction| reduction > 0),
            "sabotage must materially reduce the rival venue's next-cycle net"
        );
        assert_eq!(visible.absence, None);

        // The artifact's semantic projection retains the actual discovery/source chain.
        let json = serde_json::to_value(&evidence).expect("serialize player evidence");
        assert_eq!(json["world_seed"], evidence.world_seed);
        assert_eq!(json["policy_seed"], evidence.policy_seed);
        assert_eq!(
            json["player_visible"]["selected_source"],
            serde_json::to_value(first.information).unwrap()
        );
        assert_eq!(
            json["player_visible"]["followup"]["source_information"],
            serde_json::to_value(&followup.source_information).unwrap()
        );
        assert_eq!(
            json["player_visible"]["intervention"]["target"],
            serde_json::to_value(intervention.target).unwrap()
        );
        assert!(
            json["player_visible"]["intervention"]
                .get("next_business_cycle")
                .is_none(),
            "hidden rival books must not be serialized under player-visible evidence"
        );
        assert!(
            json["evaluation"]["matched_no_intervention_cycle"].is_object(),
            "counterfactual economics belong in explicit harness evaluation"
        );
        assert_eq!(
            json["player_visible"]["discovery"]["observations"],
            serde_json::to_value(&visible.discovery.observations).unwrap()
        );
        assert!(json.get("diagnostic").is_none());
        assert!(
            visible.discovery.observations.iter().any(|item| {
                item.topic == InformationTopic::EnterpriseActivity
                    && item.information == first.information
                    && item.subject == first.subject
            }),
            "organization surveillance must retain the enterprise-activity observation selected for the follow-up"
        );

        // Filesystem identity is adapter-only and never participates in simulation policy.
        struct TestDirectory(PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).expect("remove task-owned test artifacts");
            }
        }
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/agent-output");
        std::fs::create_dir_all(&parent).unwrap();
        let directory = parent.join(format!(
            "rival-intelligence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&directory).expect("create unique artifact directory");
        let directory = TestDirectory(directory);
        let path = persist_evidence(&directory.0, &evidence).expect("persist probe artifact");
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read probe artifact")).unwrap();
        assert_eq!(persisted, json);
        let blocked = directory.0.join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let error = persist_evidence(&blocked, &evidence)
            .expect_err("artifact directory failure must reach the caller");
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn selection_does_not_invent_a_target_when_discovery_is_absent() {
        let registry = crimocracy::build_registry();
        let scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .expect("canonical scenario builds");
        assert_eq!(
            select_observed_enterprise(&scenario, &BTreeSet::new()),
            None
        );
        assert_eq!(
            select_observed_enterprise(
                &scenario,
                &BTreeSet::from([
                    scenario.opportunity_information,
                    scenario.alternate_opportunity_information,
                ])
            ),
            None,
            "fixture business rumors cannot manufacture an enterprise discovery"
        );
    }
}
