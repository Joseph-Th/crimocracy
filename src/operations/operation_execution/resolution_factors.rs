//! Pure operation-resolution factor, venue, intelligence, and police-pressure derivation.

use super::{OperationExposurePlan, OperationPoliceAlertContext, TargetPoliceSnapshot};
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, NeighborhoodId, OperationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::InformationTopic;
use crate::legal::patrol_system::{
    PatrolPresenceSnapshot, resolve_patrol_presence_interval_snapshot,
    resolve_patrol_presence_snapshot,
};
use crate::operations::operation_objective::pressureable_witness_targets;
use crate::operations::{
    OperationExposureFactors, OperationExposureLevel, OperationKind, OperationObjective,
    OperationObjectiveOutcome, OperationRecord, OperationResolutionFactors,
};
use crate::registry::{OperationExecutionDefinition, Registry};
use crate::world::{CapabilityKind, Rating};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn resolve_role_capability_average(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> Rating {
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation resolution must reference an existing operation");
    let execution = registry.get_operation(record.kind()).execution();
    let (total, count) =
        record
            .roles()
            .iter()
            .fold((0_u32, 0_u32), |(total, count), (role, character)| {
                let capability = execution
                    .capability_for_role(*role)
                    .expect("assigned operation role must have an authored capability mapping");
                let value = state
                    .world
                    .get_character(*character)
                    .expect("operation role must reference a persisted participant")
                    .capability(capability)
                    .map(|rating| u32::from(rating.value()))
                    .unwrap_or(0);
                (total + value, count + 1)
            });
    let average = total.checked_div(count).unwrap_or(0);
    Rating::try_new(u8::try_from(average).expect("rating average must fit u8"))
        .expect("rating average must remain within rating bounds")
}

fn resolve_target_police_snapshot(
    state: &AppState,
    entities: Vec<EntityRef>,
    at: SimTime,
) -> TargetPoliceSnapshot {
    resolve_target_police_snapshot_from(state, entities, |state, neighborhood| {
        resolve_patrol_presence_snapshot(state, neighborhood, at)
    })
}

/// Venue entities for police-presence and exposure attribution. Extraction is the one
/// objective whose referenced character stands for a place the crew acts while that person
/// is elsewhere: custody. The venue stays pinned to the authority behind the exact arrest the
/// plan targeted; later release or re-arrest must not move an in-flight job to another custody
/// event or fall back to the detainee's organization assets.
pub(super) fn resolve_operation_venue_entities(
    state: &AppState,
    record: &OperationRecord,
) -> Vec<EntityRef> {
    match record.objective() {
        OperationObjective::FreeDetainee { target } => record
            .extraction_arrest()
            .and_then(|arrest| state.legal.get_arrest(arrest))
            .and_then(|arrest| state.legal.get_investigation(arrest.investigation()))
            .map(|investigation| vec![EntityRef::Organization(investigation.owner())])
            .unwrap_or_else(|| vec![EntityRef::Character(*target)]),
        OperationObjective::Frighten {
            target: EntityRef::Character(target),
        } if record.kind() == OperationKind::WitnessPressure => {
            let character = EntityRef::Character(*target);
            // A witness can be registered in several live cases, but one pressure encounter
            // cannot occur across every district where that character or their organization has
            // assets. Anchor the venue to the most-recent case whose cooperation this job can
            // actually affect. If pressureability disappears while the job is in flight, retain
            // the most-recent foreign registration as a durable physical proxy.
            let pressureable =
                pressureable_witness_targets(state, record.responsible_organization(), *target)
                    .into_iter()
                    .map(|(case_witness, _)| {
                        state.legal.get_case_witness(case_witness).expect(
                            "pressureable witness target must reference a persisted registration",
                        )
                    })
                    .max_by_key(|case_witness| (case_witness.registered_at(), case_witness.id()));
            let selected = pressureable.or_else(|| {
                state
                    .legal
                    .case_witnesses_for_character(*target)
                    .filter(|case_witness| {
                        state
                            .legal
                            .get_investigation(case_witness.investigation())
                            .expect("case-witness index must reference a persisted investigation")
                            .owner()
                            != record.responsible_organization()
                    })
                    .max_by_key(|case_witness| (case_witness.registered_at(), case_witness.id()))
            });
            let Some(case_witness) = selected else {
                return vec![character];
            };
            let investigation = state
                .legal
                .get_investigation(case_witness.investigation())
                .expect("selected witness registration must reference a persisted investigation");
            let case_neighborhoods =
                resolve_investigation_target_neighborhoods(state, investigation);
            if case_neighborhoods.is_empty() {
                // A manually authored case can have no geographically resolvable subject. Its
                // authority jurisdiction remains a better proxy than treating the encounter as
                // occurring nowhere.
                vec![character, EntityRef::Organization(investigation.owner())]
            } else {
                // Do not also return the character here. Character resolution expands through
                // their current organization and personally owned businesses, which would
                // reintroduce unrelated districts and defeat the case-scene anchor above.
                case_neighborhoods
                    .into_iter()
                    .map(EntityRef::Neighborhood)
                    .collect()
            }
        }
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => record.objective().referenced_entities(),
    }
}

pub(super) fn resolve_target_police_interval_snapshot(
    state: &AppState,
    entities: Vec<EntityRef>,
    start: SimTime,
    end: SimTime,
) -> TargetPoliceSnapshot {
    resolve_target_police_snapshot_from(state, entities, |state, neighborhood| {
        resolve_patrol_presence_interval_snapshot(state, neighborhood, start, end)
    })
}

fn resolve_target_police_snapshot_from(
    state: &AppState,
    entities: Vec<EntityRef>,
    patrol_for: impl Fn(&AppState, NeighborhoodId) -> PatrolPresenceSnapshot,
) -> TargetPoliceSnapshot {
    let neighborhoods = resolve_target_neighborhoods(state, entities);
    let mut patrol_by_neighborhood = BTreeMap::new();
    let mut strongest: Option<(NeighborhoodId, Rating)> = None;
    for neighborhood in neighborhoods {
        let patrol = patrol_for(state, neighborhood);
        let effective_presence = patrol.presence().or_else(|| {
            state
                .world
                .get_neighborhood(neighborhood)
                .map(|record| record.profile().institutions.police_presence)
        });
        patrol_by_neighborhood.insert(neighborhood, patrol);
        let Some(effective_presence) = effective_presence else {
            continue;
        };
        match strongest {
            None => strongest = Some((neighborhood, effective_presence)),
            Some((_current_neighborhood, current_presence))
                if effective_presence.value() > current_presence.value() =>
            {
                strongest = Some((neighborhood, effective_presence));
            }
            Some(_) => {}
        }
    }
    TargetPoliceSnapshot {
        patrol_by_neighborhood,
        target_presence: strongest.map(|(_, presence)| presence),
        exposure_neighborhood: strongest.map(|(neighborhood, _)| neighborhood),
    }
}

/// Venue proxy for operations against entities with no modeled meeting point: every
/// neighborhood an objective entity occupies, including the full asset footprint of
/// organization and character targets. Exposure incidents, patrol snapshots, and
/// investigation heat all attribute through this one derivation so the three consumers
/// agree on where an operation "happened".
pub(super) fn resolve_target_neighborhoods(
    state: &AppState,
    entities: Vec<EntityRef>,
) -> BTreeSet<NeighborhoodId> {
    let mut neighborhoods = BTreeSet::new();
    // Control-plane targets proxy to the world footprint of their owner, mirroring
    // FreeDetainee custody proxying: surveilling an active case or another crew's plan
    // happens where that authority operates, not nowhere. Without the proxy, exposure and
    // police response for such operations could never attribute to a neighborhood.
    let mut queue: Vec<EntityRef> = entities;
    while let Some(entity) = queue.pop() {
        match entity {
            EntityRef::Neighborhood(id) => {
                neighborhoods.insert(id);
            }
            EntityRef::Business(id) => {
                if let Some(business) = state.world.get_business(id) {
                    neighborhoods.insert(business.neighborhood());
                }
            }
            EntityRef::Organization(id) => {
                for business in state.world.businesses_owned_by_organization(id) {
                    neighborhoods.insert(business.neighborhood());
                }
                if let Some(jurisdiction) = state.legal.get_jurisdiction(id) {
                    for neighborhood in jurisdiction.neighborhoods() {
                        neighborhoods.insert(*neighborhood);
                    }
                }
            }
            EntityRef::Character(id) => {
                if let Some(character) = state.world.get_character(id)
                    && let Some(org) = character.organization()
                {
                    for business in state.world.businesses_owned_by_organization(org) {
                        neighborhoods.insert(business.neighborhood());
                    }
                }
                for business in state.world.businesses_owned_by_character(id) {
                    neighborhoods.insert(business.neighborhood());
                }
            }
            EntityRef::Enterprise(id) => {
                if let Some(enterprise) = state.enterprises.get_enterprise(id) {
                    match enterprise.location() {
                        crate::enterprises::EnterpriseLocation::Neighborhood(n) => {
                            neighborhoods.insert(n);
                        }
                        crate::enterprises::EnterpriseLocation::Business(b) => {
                            if let Some(business) = state.world.get_business(b) {
                                neighborhoods.insert(business.neighborhood());
                            }
                        }
                    }
                }
            }
            EntityRef::Operation(id) => {
                if let Some(operation) = state.operations.get_operation(id) {
                    queue.push(EntityRef::Organization(
                        operation.responsible_organization(),
                    ));
                }
            }
            EntityRef::Investigation(id) => {
                if let Some(investigation) = state.legal.get_investigation(id) {
                    queue.push(EntityRef::Organization(investigation.owner()));
                }
            }
            // Unsupported surveillance targets can never reach this derivation validated.
            EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_) => {}
        }
    }
    neighborhoods
}

/// Neighborhoods an investigation targets, derived from its subjects and, for
/// originated cases, the place where the originating event actually happened. Read-only
/// derivation used by district-scoped consumers such as enterprise heat surcharges.
///
/// Origin geography takes precedence over live subject ownership. An old burglary case must not
/// begin taxing a district merely because the investigated crew later buys a business there; the
/// case belongs to the incident scene, not to every asset the organization may acquire in the
/// future. Generic control-plane surveillance still uses `resolve_target_neighborhoods` and may
/// intentionally proxy an organization or case to its current operating footprint.
pub(crate) fn resolve_investigation_target_neighborhoods(
    state: &AppState,
    investigation: &crate::legal::InvestigationRecord,
) -> BTreeSet<NeighborhoodId> {
    match investigation.origin() {
        Some(EntityRef::Operation(origin)) => {
            let Some(operation) = state.operations.get_operation(origin) else {
                return BTreeSet::new();
            };
            if let Some(neighborhood) = operation
                .resolution()
                .and_then(|resolution| resolution.exposure().neighborhood())
            {
                return BTreeSet::from([neighborhood]);
            }
            // A manually-created or still-unresolved operation-origin case has no persisted
            // exposure scene yet. Its objective is the narrowest stable geographic proxy; using
            // the responsible organization's footprint here would make future acquisitions
            // retroactively broaden the case.
            resolve_target_neighborhoods(state, operation.objective().referenced_entities())
        }
        Some(EntityRef::Enterprise(origin)) => {
            // Vice attention belongs to the racket's own location. Do not let organization or
            // manager subjects make one enterprise inquiry tax unrelated districts.
            resolve_target_neighborhoods(state, vec![EntityRef::Enterprise(origin)])
        }
        _ => {
            resolve_target_neighborhoods(state, investigation.subjects().iter().copied().collect())
        }
    }
}

pub(super) fn resolve_exposure_plan(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
    variance: i8,
    intelligence_quality: Rating,
    police_snapshot: &TargetPoliceSnapshot,
    police_response_arrived: bool,
) -> OperationExposurePlan {
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation exposure must reference an existing operation");
    let execution = registry.get_operation(record.kind()).execution();
    let neighborhood = police_snapshot.exposure_neighborhood;
    let target_police_presence = police_snapshot.target_presence;
    let stealth_average = resolve_stealth_average(state, record);
    let approach_adjustment = execution
        .exposure_approach_adjustment(record.approach())
        .expect("validated operation approach must have an authored exposure adjustment");
    let intelligence_mitigation = u16::from(intelligence_quality.value())
        .saturating_mul(u16::from(execution.intelligence_mitigation_weight()))
        / 100;
    let factors = OperationExposureFactors {
        stealth_average,
        target_police_presence,
        police_response_arrived,
        approach_adjustment,
        intelligence_mitigation: u8::try_from(intelligence_mitigation)
            .expect("bounded intelligence exposure mitigation must fit u8"),
        variance,
    };
    let score = resolve_exposure_score(execution, factors);
    let level = resolve_exposure_level(execution, score);
    let identified_character = if level == OperationExposureLevel::Identifying {
        find_most_exposed_participant(state, record)
    } else {
        None
    };
    OperationExposurePlan {
        level,
        score,
        factors,
        neighborhood,
        identified_character,
    }
}

pub(crate) fn resolve_exposure_score(
    execution: &OperationExecutionDefinition,
    factors: OperationExposureFactors,
) -> i16 {
    let police_observation = factors
        .target_police_presence()
        .map(|rating| {
            i16::from(rating.value()) * i16::from(execution.police_observation_weight()) / 100
        })
        .unwrap_or(0);
    let stealth_mitigation = i16::from(factors.stealth_average().value())
        * i16::from(execution.stealth_mitigation_weight())
        / 100;
    i16::from(execution.base_exposure())
        + police_observation
        + if factors.police_response_arrived() {
            i16::from(execution.police_arrival_exposure_penalty())
        } else {
            0
        }
        + i16::from(factors.approach_adjustment())
        - stealth_mitigation
        - i16::from(factors.intelligence_mitigation())
        + i16::from(factors.variance())
}

pub(crate) fn resolve_exposure_level(
    execution: &OperationExecutionDefinition,
    score: i16,
) -> OperationExposureLevel {
    if score >= execution.identifying_exposure_threshold() {
        OperationExposureLevel::Identifying
    } else if score >= execution.witnessed_exposure_threshold() {
        OperationExposureLevel::Witnessed
    } else if score >= execution.trace_exposure_threshold() {
        OperationExposureLevel::Trace
    } else {
        OperationExposureLevel::None
    }
}

fn resolve_stealth_average(
    state: &AppState,
    record: &crate::operations::OperationRecord,
) -> Rating {
    let participants = record.participants();
    let total = participants.iter().fold(0_u32, |total, character| {
        total
            + state
                .world
                .get_character(*character)
                .and_then(|record| record.capability(CapabilityKind::Stealth))
                .map(|rating| u32::from(rating.value()))
                .unwrap_or(0)
    });
    let count =
        u32::try_from(participants.len()).expect("operation participant count must fit u32");
    let average = total
        .checked_div(count)
        .expect("operation always has at least its leader as a participant");
    Rating::try_new(u8::try_from(average).expect("stealth average must fit u8"))
        .expect("stealth average must remain within rating bounds")
}

pub(crate) fn resolve_operation_police_alert_context(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
    at: SimTime,
) -> OperationPoliceAlertContext {
    let record = state
        .operations
        .get_operation(operation)
        .expect("police alert planning must reference an existing operation");
    let execution = registry.get_operation(record.kind()).execution();
    let police_snapshot =
        resolve_target_police_snapshot(state, resolve_operation_venue_entities(state, record), at);
    let stealth_average = resolve_stealth_average(state, record);
    let (intelligence_quality, _, _, _) = resolve_intelligence_factors(registry, state, operation);
    let intelligence_mitigation = u16::from(intelligence_quality.value())
        .saturating_mul(u16::from(execution.intelligence_mitigation_weight()))
        / 100;
    let factors = OperationExposureFactors {
        stealth_average,
        target_police_presence: police_snapshot.target_presence,
        police_response_arrived: false,
        approach_adjustment: execution
            .exposure_approach_adjustment(record.approach())
            .expect("validated operation approach must have an authored exposure adjustment"),
        intelligence_mitigation: u8::try_from(intelligence_mitigation)
            .expect("bounded intelligence exposure mitigation must fit u8"),
        variance: 0,
    };
    OperationPoliceAlertContext {
        score: resolve_exposure_score(execution, factors),
        neighborhood: police_snapshot.exposure_neighborhood,
    }
}

pub(crate) fn has_police_response_arrived_by(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    at: SimTime,
) -> bool {
    operation
        .police_response()
        .and_then(|response| state.legal.get_police_response(response))
        .and_then(|response| response.arrived_at())
        .is_some_and(|arrived_at| arrived_at <= at)
}

fn find_most_exposed_participant(
    state: &AppState,
    record: &crate::operations::OperationRecord,
) -> Option<CharacterId> {
    record.participants().into_iter().min_by_key(|character| {
        let stealth = state
            .world
            .get_character(*character)
            .and_then(|record| record.capability(CapabilityKind::Stealth))
            .map(Rating::value)
            .unwrap_or(0);
        (stealth, *character)
    })
}

pub(crate) fn resolve_time_pressure(
    started_at: SimTime,
    due_at: SimTime,
    base_duration: u32,
    maximum: u8,
) -> u8 {
    let available = due_at.as_minutes().saturating_sub(started_at.as_minutes());
    let base = u64::from(base_duration);
    if available >= base {
        return 0;
    }
    let shortfall = base - available;
    let pressure = shortfall.saturating_mul(u64::from(maximum)).div_ceil(base);
    u8::try_from(pressure.min(u64::from(maximum))).expect("bounded time pressure must fit u8")
}

fn weighted_ability(
    execution: &OperationExecutionDefinition,
    role_average: Rating,
    leader_capability: Option<Rating>,
) -> i16 {
    let role = u32::from(role_average.value());
    // A leader without the authored leadership capability contributes nothing to coordination
    // rather than being silently averaged up to the roster's role skill: the after-action summary
    // reports "no demonstrated capability", and the arithmetic should match.
    let leadership = leader_capability
        .map(|rating| u32::from(rating.value()))
        .unwrap_or(0);
    let role_weight = u32::from(execution.role_capability_weight());
    let leader_weight = u32::from(execution.leader_capability_weight());
    let total_weight = role_weight + leader_weight;
    let weighted = role
        .saturating_mul(role_weight)
        .saturating_add(leadership.saturating_mul(leader_weight))
        / total_weight;
    i16::try_from(weighted).expect("weighted operation ability must fit i16")
}

pub(crate) fn resolve_intelligence_factors(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> (Rating, i8, u8, u8) {
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation intelligence must reference an existing operation");
    let planning_at = record
        .started_at()
        .unwrap_or_else(|| record.scheduled_for());
    let execution = registry.get_operation(record.kind()).execution();
    let max_age = u64::from(execution.max_intelligence_age().as_minutes());
    let mut best_by_topic = std::collections::BTreeMap::<InformationTopic, u8>::new();
    for information in record.intelligence() {
        let information = state
            .intelligence
            .get_information(*information)
            .expect("validated operation intelligence record must exist");
        let score = information_score(
            registry.information_quality(),
            information,
            planning_at,
            max_age,
        );
        best_by_topic
            .entry(information.topic())
            .and_modify(|best| *best = (*best).max(score))
            .or_insert(score);
    }

    let relevant_topics = execution.relevant_intelligence_topics();
    let covered = relevant_topics
        .iter()
        .filter(|topic| best_by_topic.get(topic).is_some_and(|score| *score > 0))
        .count();
    let total = relevant_topics.iter().fold(0_u32, |total, topic| {
        total + u32::from(best_by_topic.get(topic).copied().unwrap_or(0))
    });
    let count = u32::try_from(relevant_topics.len())
        .expect("authored operation intelligence topic count must fit u32");
    let average = total
        .checked_div(count)
        .expect("operation definitions always contain relevant intelligence topics");
    let quality = Rating::try_new(
        u8::try_from(average).expect("bounded intelligence quality average must fit u8"),
    )
    .expect("intelligence quality must remain within rating bounds");
    let reduction = u16::from(quality.value())
        .saturating_mul(u16::from(execution.max_intelligence_difficulty_reduction()))
        / 100;
    let adjustment =
        -i8::try_from(reduction).expect("authored intelligence difficulty reduction must fit i8");
    (
        quality,
        adjustment,
        u8::try_from(covered).expect("authored intelligence topic count must fit u8"),
        u8::try_from(relevant_topics.len()).expect("authored intelligence topic count must fit u8"),
    )
}

fn information_score(
    quality: crate::registry::InformationQualityDefinition,
    information: &crate::intelligence::InformationRecord,
    planning_at: SimTime,
    max_age: u64,
) -> u8 {
    let reliability = u32::from(quality.reliability_score(information.reliability()));
    let specificity = u32::from(quality.specificity_score(information.specificity()));
    let age = planning_at
        .as_minutes()
        .saturating_sub(information.observed_at().as_minutes());
    let freshness = if age >= max_age {
        0_u32
    } else {
        u32::try_from((max_age - age).saturating_mul(100) / max_age)
            .expect("bounded intelligence freshness must fit u32")
    };
    let score = reliability
        .saturating_mul(specificity)
        .saturating_mul(freshness)
        / 10_000;
    u8::try_from(score).expect("bounded information score must fit u8")
}

pub(crate) fn resolve_execution_margin(
    execution: &OperationExecutionDefinition,
    factors: OperationResolutionFactors,
) -> i16 {
    let ability = weighted_ability(
        execution,
        factors.role_capability_average(),
        factors.leader_capability(),
    );
    let police_pressure = factors
        .target_police_presence()
        .map(|rating| {
            i16::from(rating.value()) * i16::from(execution.police_pressure_weight()) / 100
        })
        .unwrap_or(0);
    let difficulty = i16::from(execution.base_difficulty())
        + police_pressure
        + if factors.police_response_arrived() {
            i16::from(execution.police_arrival_difficulty_penalty())
        } else {
            0
        }
        + i16::from(factors.intelligence_adjustment())
        + i16::from(factors.approach_adjustment())
        + i16::from(factors.time_pressure());
    ability - difficulty + i16::from(factors.variance())
}

pub(crate) fn resolve_objective_outcome(
    execution: &OperationExecutionDefinition,
    execution_margin: i16,
) -> OperationObjectiveOutcome {
    if execution_margin >= execution.achieved_margin() {
        OperationObjectiveOutcome::Achieved
    } else if execution_margin >= execution.partial_margin() {
        OperationObjectiveOutcome::Partial
    } else {
        OperationObjectiveOutcome::Failed
    }
}
