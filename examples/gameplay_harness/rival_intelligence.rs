//! Full-mode rival discovery and follow-up through player-held surveillance observations only.

use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{InformationId, OperationId};
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crimocracy::operations::{OperationExposureLevel, OperationObjectiveOutcome, OperationStatus};
use crimocracy::registry::Registry;
use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::path::{Path, PathBuf};

use crate::{
    EvaluationSeeds, RunMetrics, Scenario, ScenarioProfile, authorize_surveillance_target,
    build_scenario, run_until, run_until_operation_terminal, stamp, validate_harness_state,
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
struct PlayerVisibleEvidence {
    discovery: WatchEvidence,
    selected_source: Option<InformationId>,
    followup: Option<WatchEvidence>,
    absence: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ProbeEvidence {
    world_seed: u64,
    policy_seed: u64,
    expansion_boundary_minute: u64,
    timing_policy: &'static str,
    player_visible: PlayerVisibleEvidence,
}

/// One organization watch, then at most one exact-subject enterprise watch. Failed or partial
/// discovery is evidence of absence, not permission to enumerate the rival's hidden rackets.
pub fn run_rival_intelligence_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    artifact_dir: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let evidence = collect_probe(registry, seeds)?;
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
    if let Some(absence) = evidence.player_visible.absence {
        println!("[OBSERVED ABSENCE] {absence}");
    }
    if let Some(directory) = artifact_dir {
        let path = persist_evidence(directory, &evidence)?;
        println!("[ARTIFACT] wrote {}", path.display());
    }
    Ok(())
}

fn collect_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
) -> Result<ProbeEvidence, Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let mut metrics = RunMetrics::default();
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
            absence: selected.is_none().then_some(
                "The organization watch produced no player-held Personnel observation with an Enterprise subject. No follow-up was authorized; this does not establish that the rival has no rackets.",
            ),
        },
    })
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
            && information.topic() == InformationTopic::Personnel
            && matches!(information.subject(), EntityRef::Enterprise(_)))
        .then_some((information.subject(), *id))
    })
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
        // Quote the persisted observation exactly. Do not supplement a missing racket kind,
        // location, manager, or financial detail from the foreign enterprise's hidden record.
        summary: information.summary().to_owned(),
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
                item.topic == InformationTopic::Personnel
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
        assert!(first.observed_minute <= followup.scheduled_minute);
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
            json["player_visible"]["discovery"]["observations"],
            serde_json::to_value(&visible.discovery.observations).unwrap()
        );
        assert!(json.get("diagnostic").is_none());
        assert!(
            visible.discovery.observations.iter().any(|item| {
                item.topic == InformationTopic::Personnel
                    && item.subject == visible.discovery.target
            }),
            "racket discoveries must retain the original personnel observation"
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
