//! Deterministic operation resolution planning and atomic persistence of causal outcomes.

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, NeighborhoodId, OperationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::history::history_system::{validate_record_event, HistoryError, ValidatedHistoryEvent};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::investigation_system::{
    validate_incident_intake, InvestigationError, ValidatedIncidentIntake,
};
use crate::legal::jurisdiction_system::resolve_case_intake_authority;
use crate::legal::{
    Admissibility, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft,
};
use crate::operations::{
    OperationExposureFactors, OperationExposureLevel, OperationExposureRecord,
    OperationObjectiveOutcome, OperationResolutionFactors, OperationResolutionRecord,
    OperationStatus,
};
use crate::registry::{OperationExecutionDefinition, Registry};
use crate::world::{CapabilityKind, QualitativeBand, Rating};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum OperationResolutionError {
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} is not in progress")]
    OperationNotInProgress(OperationId),
    #[error("operation {operation} is not due for resolution until {due_at:?}")]
    ResolutionNotDue {
        operation: OperationId,
        due_at: SimTime,
    },
    #[error("operation resolution variance {variance} exceeds authored limit {limit}")]
    VarianceOutOfRange { variance: i8, limit: u8 },
    #[error("operation exposure variance {variance} exceeds authored limit {limit}")]
    ExposureVarianceOutOfRange { variance: i8, limit: u8 },
    #[error("operation {operation} changed after resolution planning; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error("operation resolution plan was resolved at {expected:?}, but simulation time is now {found:?}")]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error(
        "operation incident routing changed for neighborhood {neighborhood}; expected authority {expected:?}, found {found:?}"
    )]
    StaleIncidentRouting {
        neighborhood: NeighborhoodId,
        expected: Option<crate::core::id::OrganizationId>,
        found: Option<crate::core::id::OrganizationId>,
    },
    #[error(
        "operation incident jurisdiction changed for neighborhood {neighborhood}; organization {organization} expected version {expected_version}, found {found_version:?}"
    )]
    StaleIncidentJurisdictionVersion {
        neighborhood: NeighborhoodId,
        organization: crate::core::id::OrganizationId,
        expected_version: u32,
        found_version: Option<u32>,
    },
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Investigation(#[from] InvestigationError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationResolutionRandomness {
    execution_variance: i8,
    exposure_variance: i8,
}

impl OperationResolutionRandomness {
    pub(crate) fn new(execution_variance: i8, exposure_variance: i8) -> Self {
        Self {
            execution_variance,
            exposure_variance,
        }
    }

    pub fn execution_variance(self) -> i8 {
        self.execution_variance
    }

    pub fn exposure_variance(self) -> i8 {
        self.exposure_variance
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationExposurePlan {
    level: OperationExposureLevel,
    score: i16,
    factors: OperationExposureFactors,
    neighborhood: Option<NeighborhoodId>,
    identified_character: Option<CharacterId>,
}

impl OperationExposurePlan {
    pub fn level(&self) -> OperationExposureLevel {
        self.level
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationResolutionPlan {
    operation: OperationId,
    expected_operation_version: u32,
    resolved_at: SimTime,
    objective_outcome: OperationObjectiveOutcome,
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposurePlan,
    summary: String,
    history_entities: BTreeSet<EntityRef>,
}

pub(crate) fn decide_operation_resolution(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
    randomness: OperationResolutionRandomness,
) -> Result<OperationResolutionPlan, OperationResolutionError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationResolutionError::MissingOperation(operation))?;
    if record.status() != OperationStatus::InProgress {
        return Err(OperationResolutionError::OperationNotInProgress(operation));
    }
    let due_at = record
        .resolution_due_at()
        .expect("in-progress operation must have a resolution due time");
    if state.now() < due_at {
        return Err(OperationResolutionError::ResolutionNotDue { operation, due_at });
    }

    let definition = registry.get_operation(record.kind());
    let execution = definition.execution();
    if randomness.execution_variance().unsigned_abs() > execution.variance_limit() {
        return Err(OperationResolutionError::VarianceOutOfRange {
            variance: randomness.execution_variance(),
            limit: execution.variance_limit(),
        });
    }
    if randomness.exposure_variance().unsigned_abs() > execution.exposure_variance_limit() {
        return Err(OperationResolutionError::ExposureVarianceOutOfRange {
            variance: randomness.exposure_variance(),
            limit: execution.exposure_variance_limit(),
        });
    }

    let role_capability_average = resolve_role_capability_average(registry, state, operation);
    let leader_management = state
        .world
        .get_character(record.leader())
        .and_then(|leader| leader.capability(CapabilityKind::Management));
    let (intelligence_quality, intelligence_adjustment) =
        calculate_intelligence_factors(registry, state, operation);
    let target_police_presence =
        resolve_target_police_presence(state, record.objective().referenced_entities());
    let approach_adjustment = execution
        .approach_difficulty_adjustment(record.approach())
        .expect("validated operation approach must have an authored execution adjustment");
    let time_pressure = resolve_time_pressure(
        record
            .started_at()
            .expect("in-progress operation must have a start time"),
        due_at,
        execution.duration().as_minutes(),
    );

    let factors = OperationResolutionFactors {
        role_capability_average,
        leader_management,
        intelligence_quality,
        intelligence_adjustment,
        target_police_presence,
        approach_adjustment,
        time_pressure,
        variance: randomness.execution_variance(),
    };
    let execution_margin = calculate_execution_margin(execution, factors);
    let objective_outcome = classify_objective_outcome(execution, execution_margin);
    let exposure = calculate_exposure_plan(
        registry,
        state,
        operation,
        randomness.exposure_variance(),
        intelligence_quality,
    );
    let summary = build_after_action_summary(objective_outcome, factors, exposure.level());
    let mut history_entities = BTreeSet::from([
        EntityRef::Operation(operation),
        EntityRef::Organization(record.responsible_organization()),
        EntityRef::Character(record.leader()),
    ]);
    history_entities.extend(record.objective().referenced_entities());
    history_entities.extend(record.roles().values().copied().map(EntityRef::Character));

    Ok(OperationResolutionPlan {
        operation,
        expected_operation_version: record.version(),
        resolved_at: state.now(),
        objective_outcome,
        execution_margin,
        factors,
        exposure,
        summary,
        history_entities,
    })
}

pub(crate) struct ValidatedOperationResolution {
    plan: OperationResolutionPlan,
    incident: Option<ValidatedIncidentIntake>,
    incident_authority: Option<IncidentAuthoritySnapshot>,
    information: ValidatedInformation,
    history: ValidatedHistoryEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IncidentAuthoritySnapshot {
    neighborhood: NeighborhoodId,
    organization: Option<crate::core::id::OrganizationId>,
    jurisdiction_version: Option<u32>,
}

impl ValidatedOperationResolution {
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<OperationId, OperationResolutionError> {
        validate_plan_snapshot(state, &self.plan)?;
        if let Some(snapshot) = self.incident_authority {
            let found = resolve_case_intake_authority(state, snapshot.neighborhood);
            if found != snapshot.organization {
                return Err(OperationResolutionError::StaleIncidentRouting {
                    neighborhood: snapshot.neighborhood,
                    expected: snapshot.organization,
                    found,
                });
            }
            if let Some(organization) = snapshot.organization {
                let found_version = state
                    .legal
                    .get_jurisdiction(organization)
                    .map(|jurisdiction| jurisdiction.version());
                let expected_version = snapshot
                    .jurisdiction_version
                    .expect("routed incident snapshot must contain a jurisdiction version");
                if found_version != Some(expected_version) {
                    return Err(OperationResolutionError::StaleIncidentJurisdictionVersion {
                        neighborhood: snapshot.neighborhood,
                        organization,
                        expected_version,
                        found_version,
                    });
                }
            }
        }
        let incident = self
            .incident
            .map(|validated| validated.commit(state))
            .transpose()?;
        let investigation = incident.as_ref().map(|outcome| outcome.investigation);
        let evidence = incident
            .map(|outcome| outcome.evidence.into_iter().collect())
            .unwrap_or_default();
        let exposure = OperationExposureRecord {
            level: self.plan.exposure.level,
            score: self.plan.exposure.score,
            factors: self.plan.exposure.factors,
            neighborhood: self.plan.exposure.neighborhood,
            identified_character: self.plan.exposure.identified_character,
            investigation,
            evidence,
        };
        let after_action_information = self.information.commit(state);
        let history_event = self.history.commit(state);
        state.operations.complete(
            self.plan.operation,
            OperationResolutionRecord {
                resolved_at: self.plan.resolved_at,
                objective_outcome: self.plan.objective_outcome,
                execution_margin: self.plan.execution_margin,
                factors: self.plan.factors,
                exposure,
                after_action_information,
                history_event,
            },
        );
        Ok(self.plan.operation)
    }
}

pub(crate) fn validate_operation_resolution_plan(
    registry: &Registry,
    state: &AppState,
    plan: OperationResolutionPlan,
) -> Result<ValidatedOperationResolution, OperationResolutionError> {
    validate_plan_snapshot(state, &plan)?;
    let record = state
        .operations
        .get_operation(plan.operation)
        .expect("validated resolution operation must exist");
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(record.responsible_organization()),
            source_kind: InformationSourceKind::AfterAction,
            topic: crate::intelligence::InformationTopic::OperationalOutcome,
            source_entity: Some(EntityRef::Character(record.leader())),
            subject: EntityRef::Operation(record.id()),
            observed_at: plan.resolved_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: plan.summary.clone(),
        },
    )?;
    let history = validate_record_event(
        state,
        HistoryEventDraft {
            occurred_at: plan.resolved_at,
            kind: HistoryEventKind::Operation,
            summary: format!(
                "{} ended with objective {}.",
                record.title(),
                outcome_label(plan.objective_outcome)
            ),
            entities: plan.history_entities.clone(),
        },
    )?;
    let (incident, incident_authority) =
        validate_exposure_incident(registry, state, record, &plan.exposure, plan.resolved_at)?;
    Ok(ValidatedOperationResolution {
        plan,
        incident,
        incident_authority,
        information,
        history,
    })
}

fn validate_exposure_incident(
    registry: &Registry,
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    exposure: &OperationExposurePlan,
    discovered_at: SimTime,
) -> Result<
    (
        Option<ValidatedIncidentIntake>,
        Option<IncidentAuthoritySnapshot>,
    ),
    OperationResolutionError,
> {
    if exposure.level == OperationExposureLevel::None {
        return Ok((None, None));
    }
    let Some(neighborhood) = exposure.neighborhood else {
        return Ok((None, None));
    };
    let Some(owner) = resolve_case_intake_authority(state, neighborhood) else {
        return Ok((
            None,
            Some(IncidentAuthoritySnapshot {
                neighborhood,
                organization: None,
                jurisdiction_version: None,
            }),
        ));
    };
    let jurisdiction_version = state
        .legal
        .get_jurisdiction(owner)
        .expect("resolved legal intake authority must have a jurisdiction record")
        .version();
    let subject = exposure
        .identified_character
        .map(EntityRef::Character)
        .unwrap_or(EntityRef::Operation(operation.id()));
    let strength = match exposure.level {
        OperationExposureLevel::None => unreachable!("non-exposure cannot create an incident"),
        OperationExposureLevel::Trace => EvidenceStrength::Weak,
        OperationExposureLevel::Witnessed => EvidenceStrength::Corroborating,
        OperationExposureLevel::Identifying => EvidenceStrength::Strong,
    };
    let reliability = match exposure.level {
        OperationExposureLevel::None => unreachable!("non-exposure cannot create an incident"),
        OperationExposureLevel::Trace => EvidenceReliability::Questionable,
        OperationExposureLevel::Witnessed => EvidenceReliability::Credible,
        OperationExposureLevel::Identifying => EvidenceReliability::HighlyReliable,
    };
    let mut subjects = BTreeSet::from([EntityRef::Operation(operation.id())]);
    if let Some(character) = exposure.identified_character {
        subjects.insert(EntityRef::Character(character));
    }
    let kind = registry
        .get_operation(operation.kind())
        .execution()
        .exposure_evidence_kind();
    let incident = validate_incident_intake(
        state,
        IncidentIntakeDraft {
            owner,
            title: format!("Incident linked to {}", operation.title()),
            subjects,
            evidence: vec![IncidentEvidenceDraft {
                subject,
                origin: Some(EntityRef::Operation(operation.id())),
                kind,
                strength,
                reliability,
                admissibility: Admissibility::Unknown,
                discovered_at,
            }],
        },
    )?;
    Ok((
        Some(incident),
        Some(IncidentAuthoritySnapshot {
            neighborhood,
            organization: Some(owner),
            jurisdiction_version: Some(jurisdiction_version),
        }),
    ))
}

pub(crate) fn due_in_progress_operations(state: &AppState) -> Vec<OperationId> {
    state.operations.due_in_progress_at_or_before(state.now())
}

fn validate_plan_snapshot(
    state: &AppState,
    plan: &OperationResolutionPlan,
) -> Result<(), OperationResolutionError> {
    let record = state
        .operations
        .get_operation(plan.operation)
        .ok_or(OperationResolutionError::MissingOperation(plan.operation))?;
    if record.version() != plan.expected_operation_version {
        return Err(OperationResolutionError::StaleOperation {
            operation: plan.operation,
            expected: plan.expected_operation_version,
            found: record.version(),
        });
    }
    if record.status() != OperationStatus::InProgress {
        return Err(OperationResolutionError::OperationNotInProgress(
            plan.operation,
        ));
    }
    let due_at = record
        .resolution_due_at()
        .expect("in-progress operation must have a resolution due time");
    if plan.resolved_at < due_at {
        return Err(OperationResolutionError::ResolutionNotDue {
            operation: plan.operation,
            due_at,
        });
    }
    if state.now() != plan.resolved_at {
        return Err(OperationResolutionError::StaleResolutionTime {
            expected: plan.resolved_at,
            found: state.now(),
        });
    }
    Ok(())
}

fn resolve_role_capability_average(
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
                    .and_then(|record| record.capability(capability))
                    .map(|rating| u32::from(rating.value()))
                    .unwrap_or(0);
                (total + value, count + 1)
            });
    let average = total.checked_div(count).unwrap_or(0);
    Rating::try_new(u8::try_from(average).expect("rating average must fit u8"))
        .expect("rating average must remain within rating bounds")
}

fn resolve_target_police_presence(state: &AppState, entities: Vec<EntityRef>) -> Option<Rating> {
    entities
        .into_iter()
        .filter_map(|entity| match entity {
            EntityRef::Neighborhood(id) => state
                .world
                .get_neighborhood(id)
                .map(|record| record.profile().institutions.police_presence),
            EntityRef::Business(id) => state.world.get_business(id).and_then(|business| {
                state
                    .world
                    .get_neighborhood(business.neighborhood())
                    .map(|record| record.profile().institutions.police_presence)
            }),
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => None,
        })
        .max_by_key(|rating| rating.value())
}

fn calculate_exposure_plan(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
    variance: i8,
    intelligence_quality: Rating,
) -> OperationExposurePlan {
    let record = state
        .operations
        .get_operation(operation)
        .expect("operation exposure must reference an existing operation");
    let execution = registry.get_operation(record.kind()).execution();
    let neighborhood =
        resolve_exposure_neighborhood(state, record.objective().referenced_entities());
    let target_police_presence = neighborhood.and_then(|id| {
        state
            .world
            .get_neighborhood(id)
            .map(|record| record.profile().institutions.police_presence)
    });
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
        approach_adjustment,
        intelligence_mitigation: u8::try_from(intelligence_mitigation)
            .expect("bounded intelligence exposure mitigation must fit u8"),
        variance,
    };
    let score = calculate_exposure_score(execution, factors);
    let level = classify_exposure_level(execution, score);
    let identified_character = if level == OperationExposureLevel::Identifying {
        most_exposed_participant(state, record)
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

pub(crate) fn calculate_exposure_score(
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
        + i16::from(factors.approach_adjustment())
        - stealth_mitigation
        - i16::from(factors.intelligence_mitigation())
        + i16::from(factors.variance())
}

pub(crate) fn classify_exposure_level(
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

fn resolve_exposure_neighborhood(
    state: &AppState,
    entities: Vec<EntityRef>,
) -> Option<NeighborhoodId> {
    let mut neighborhoods = BTreeSet::new();
    for entity in entities {
        match entity {
            EntityRef::Neighborhood(id) => {
                neighborhoods.insert(id);
            }
            EntityRef::Business(id) => {
                if let Some(business) = state.world.get_business(id) {
                    neighborhoods.insert(business.neighborhood());
                }
            }
            EntityRef::Organization(_)
            | EntityRef::Character(_)
            | EntityRef::Operation(_)
            | EntityRef::Investigation(_)
            | EntityRef::Evidence(_)
            | EntityRef::FinancialAccount(_)
            | EntityRef::DecisionRequest(_)
            | EntityRef::Mandate(_)
            | EntityRef::Enterprise(_) => {}
        }
    }
    neighborhoods
        .into_iter()
        .fold(None, |best, neighborhood| {
            let police = state
                .world
                .get_neighborhood(neighborhood)
                .map(|record| record.profile().institutions.police_presence.value())
                .unwrap_or(0);
            match best {
                None => Some((neighborhood, police)),
                Some((_current, current_police)) if police > current_police => {
                    Some((neighborhood, police))
                }
                Some(current) => Some(current),
            }
        })
        .map(|(neighborhood, _)| neighborhood)
}

fn operation_participants(record: &crate::operations::OperationRecord) -> BTreeSet<CharacterId> {
    let mut participants = BTreeSet::from([record.leader()]);
    participants.extend(record.roles().values().copied());
    participants
}

fn resolve_stealth_average(
    state: &AppState,
    record: &crate::operations::OperationRecord,
) -> Rating {
    let participants = operation_participants(record);
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

fn most_exposed_participant(
    state: &AppState,
    record: &crate::operations::OperationRecord,
) -> Option<CharacterId> {
    operation_participants(record)
        .into_iter()
        .min_by_key(|character| {
            let stealth = state
                .world
                .get_character(*character)
                .and_then(|record| record.capability(CapabilityKind::Stealth))
                .map(Rating::value)
                .unwrap_or(0);
            (stealth, *character)
        })
}

fn resolve_time_pressure(started_at: SimTime, due_at: SimTime, base_duration: u32) -> u8 {
    let available = due_at.as_minutes().saturating_sub(started_at.as_minutes());
    let base = u64::from(base_duration);
    if available >= base {
        return 0;
    }
    let shortfall = base - available;
    let pressure = shortfall.saturating_mul(30).div_ceil(base);
    u8::try_from(pressure.min(30)).expect("bounded time pressure must fit u8")
}

fn weighted_ability(role_average: Rating, leader_management: Option<Rating>) -> i16 {
    let role = i16::from(role_average.value());
    let management = leader_management
        .map(|rating| i16::from(rating.value()))
        .unwrap_or(role);
    (role * 3 + management) / 4
}

pub(crate) fn calculate_intelligence_factors(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> (Rating, i8) {
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
        let score = information_score(information, planning_at, max_age);
        best_by_topic
            .entry(information.topic())
            .and_modify(|best| *best = (*best).max(score))
            .or_insert(score);
    }

    let relevant_topics = execution.relevant_intelligence_topics();
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
    (quality, adjustment)
}

fn information_score(
    information: &crate::intelligence::InformationRecord,
    planning_at: SimTime,
    max_age: u64,
) -> u8 {
    let reliability = u32::from(reliability_score(information.reliability()));
    let specificity = u32::from(specificity_score(information.specificity()));
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

fn reliability_score(reliability: Reliability) -> u8 {
    match reliability {
        Reliability::Unknown => 10,
        Reliability::Unreliable => 20,
        Reliability::Mixed => 40,
        Reliability::GenerallyReliable => 70,
        Reliability::DirectAccess => 100,
    }
}

fn specificity_score(specificity: Specificity) -> u8 {
    match specificity {
        Specificity::Vague => 25,
        Specificity::General => 50,
        Specificity::Specific => 75,
        Specificity::Precise => 100,
    }
}

pub(crate) fn calculate_execution_margin(
    execution: &OperationExecutionDefinition,
    factors: OperationResolutionFactors,
) -> i16 {
    let ability = weighted_ability(
        factors.role_capability_average(),
        factors.leader_management(),
    );
    let police_pressure = factors
        .target_police_presence()
        .map(|rating| {
            i16::from(rating.value()) * i16::from(execution.police_pressure_weight()) / 100
        })
        .unwrap_or(0);
    let difficulty = i16::from(execution.base_difficulty())
        + police_pressure
        + i16::from(factors.intelligence_adjustment())
        + i16::from(factors.approach_adjustment())
        + i16::from(factors.time_pressure());
    ability - difficulty + i16::from(factors.variance())
}

pub(crate) fn classify_objective_outcome(
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

fn build_after_action_summary(
    outcome: OperationObjectiveOutcome,
    factors: OperationResolutionFactors,
    exposure: OperationExposureLevel,
) -> String {
    let management = factors
        .leader_management()
        .map(|rating| {
            format!(
                "Leadership coordination was {}.",
                band_label(rating.qualitative_band())
            )
        })
        .unwrap_or_else(|| {
            "Leadership had no demonstrated management capability for the execution.".to_owned()
        });
    let police = match factors.target_police_presence() {
        Some(rating) if rating.value() >= 65 => {
            "High local police presence materially increased execution pressure."
        }
        Some(rating) if rating.value() >= 35 => {
            "Local police presence added meaningful execution pressure."
        }
        Some(_) => "Local police presence created limited execution pressure.",
        None => "No location-based police pressure could be established from the operation target.",
    };
    let intelligence = match factors.intelligence_adjustment() {
        value if value < 0 => format!(
            "Planning intelligence was {} and reduced execution uncertainty.",
            band_label(factors.intelligence_quality().qualitative_band())
        ),
        0 => format!(
            "Planning intelligence was {} and provided no material execution advantage.",
            band_label(factors.intelligence_quality().qualitative_band())
        ),
        _ => unreachable!("operation intelligence never increases authored difficulty"),
    };
    let approach = match factors.approach_adjustment() {
        value if value < 0 => "The selected approach reduced execution difficulty.",
        0 => "The selected approach was neutral to execution difficulty.",
        _ => "The selected approach increased execution difficulty.",
    };
    let deadline = if factors.time_pressure() == 0 {
        "The plan had its normal execution window."
    } else {
        "The completion deadline compressed the execution window."
    };
    let circumstances = match factors.variance() {
        value if value < 0 => "Unplanned circumstances were adverse.",
        0 => "Unplanned circumstances were neutral.",
        _ => "Unplanned circumstances were favorable.",
    };
    let exposure = match exposure {
        OperationExposureLevel::None => "No material operational exposure was observed.",
        OperationExposureLevel::Trace => "The crew observed limited trace exposure.",
        OperationExposureLevel::Witnessed => {
            "The operation appears to have been witnessed or otherwise clearly observed."
        }
        OperationExposureLevel::Identifying => {
            "The crew believes at least one participant may have been identifiable."
        }
    };
    format!(
        "Objective {}. Assigned-role competence was {}. {} {} {} {} {} {} {}",
        outcome_label(outcome),
        band_label(factors.role_capability_average().qualitative_band()),
        management,
        intelligence,
        police,
        approach,
        deadline,
        circumstances,
        exposure,
    )
}

fn outcome_label(outcome: OperationObjectiveOutcome) -> &'static str {
    match outcome {
        OperationObjectiveOutcome::Achieved => "achieved",
        OperationObjectiveOutcome::Partial => "partially achieved",
        OperationObjectiveOutcome::Failed => "failed",
    }
}

fn band_label(band: QualitativeBand) -> &'static str {
    match band {
        QualitativeBand::Poor => "poor",
        QualitativeBand::Competent => "competent",
        QualitativeBand::Skilled => "skilled",
        QualitativeBand::Excellent => "excellent",
        QualitativeBand::Exceptional => "exceptional",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::AttentionClass;
    use crate::core::id::OrganizationId;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::core::time::SimDuration;
    use crate::decisions::decision_system::{validate_request_decision, validate_resolve_decision};
    use crate::decisions::{
        DecisionContext, DecisionRequestDraft, DecisionResponse, OperationExceptionReason,
    };
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{InformationDraft, InformationTopic};
    use crate::legal::jurisdiction_system::validate_set_jurisdiction;
    use crate::legal::JurisdictionDraft;
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{
        OperationApproach, OperationContingency, OperationDraft, OperationKind, OperationObjective,
        RoleKind,
    };
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
        validate_reassign_character,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn make_operation_fixture() -> (Registry, AppState, OrganizationId, OperationId) {
        let registry = build_registry();
        let mut state = AppState::new(0x0A19_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Operation Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("operation organization fixture should validate");
        let target = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Operation Test Target".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("operation target fixture should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Operation Test Leader".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Management,
                    Rating::try_new(82).expect("fixture rating should be valid"),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("operation leader fixture should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Operation execution fixture".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: organization,
                leader,
                objective: OperationObjective::Frighten {
                    target: EntityRef::Organization(target),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
                scheduled_for: SimTime::from_minutes(1),
            },
        )
        .expect("operation fixture should validate")
        .commit(&mut state)
        .expect("validated operation fixture should commit");
        (registry, state, organization, operation)
    }

    fn make_intelligence_operation_fixture() -> (Registry, AppState, OperationId) {
        let registry = build_registry();
        let mut state = AppState::new(0x1A7E_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Intelligence Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("intelligence operation organization should validate");
        let target = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Intelligence Test Target".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("intelligence target should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Prepared Crew Leader".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (
                        CapabilityKind::Management,
                        Rating::try_new(82).expect("fixture management should validate"),
                    ),
                    (
                        CapabilityKind::Stealth,
                        Rating::try_new(0).expect("fixture stealth should validate"),
                    ),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("prepared leader should validate");
        let mut intelligence = BTreeSet::new();
        for topic in [
            InformationTopic::Personnel,
            InformationTopic::Relationship,
            InformationTopic::PoliceActivity,
        ] {
            let information = validate_record_information(
                &state,
                InformationDraft {
                    holder: KnowledgeHolder::Organization(organization),
                    source_kind: InformationSourceKind::DirectObservation,
                    topic,
                    source_entity: None,
                    subject: EntityRef::Organization(target),
                    observed_at: state.now(),
                    reliability: Reliability::DirectAccess,
                    specificity: Specificity::Precise,
                    summary: format!("Fresh precise planning information for {topic:?}."),
                },
            )
            .expect("planning information should validate")
            .commit(&mut state);
            intelligence.insert(information);
        }
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Prepared intimidation".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: organization,
                leader,
                objective: OperationObjective::Frighten {
                    target: EntityRef::Organization(target),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
                intelligence,
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: SimTime::from_minutes(1),
            },
        )
        .expect("prepared operation should validate")
        .commit(&mut state)
        .expect("prepared operation should commit");
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(20));
        (registry, state, operation)
    }

    fn make_exposed_business_operation_fixture(
        assign_jurisdiction: bool,
    ) -> (
        Registry,
        AppState,
        OrganizationId,
        NeighborhoodId,
        OperationId,
    ) {
        let registry = build_registry();
        let mut state = AppState::new(0xE710_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Exposure Test Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("exposure crew should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Exposure Test Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("exposure precinct should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Observed Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).expect("fixture wealth should validate"),
                        commercial_activity: Rating::try_new(60)
                            .expect("fixture commerce should validate"),
                        illicit_demand: Rating::try_new(50)
                            .expect("fixture demand should validate"),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(90)
                            .expect("fixture police presence should validate"),
                        political_influence: Rating::try_new(50)
                            .expect("fixture influence should validate"),
                        social_cohesion: Rating::try_new(50)
                            .expect("fixture cohesion should validate"),
                        visible_violence_tolerance: Rating::try_new(30)
                            .expect("fixture violence tolerance should validate"),
                    },
                },
            },
        )
        .expect("exposure neighborhood should validate");
        if assign_jurisdiction {
            validate_set_jurisdiction(
                &state,
                JurisdictionDraft {
                    organization: police,
                    neighborhoods: BTreeSet::from([neighborhood]),
                    case_intake_priority: Rating::try_new(80)
                        .expect("fixture case priority should validate"),
                },
            )
            .expect("precinct jurisdiction should validate")
            .commit(&mut state)
            .expect("precinct jurisdiction should commit");
        }
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Observed Retail Target".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("exposure business should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Exposure Crew Leader".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (
                        CapabilityKind::Management,
                        Rating::try_new(80).expect("fixture management should validate"),
                    ),
                    (
                        CapabilityKind::Stealth,
                        Rating::try_new(0).expect("fixture stealth should validate"),
                    ),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("exposure leader should validate");
        let specialist = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Exposure Entry Specialist".to_owned(),
                organization: Some(organization),
                supervisor: Some(leader),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([
                    (
                        CapabilityKind::Burglary,
                        Rating::try_new(80).expect("fixture burglary should validate"),
                    ),
                    (
                        CapabilityKind::Stealth,
                        Rating::try_new(0).expect("fixture stealth should validate"),
                    ),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("exposure specialist should validate");
        let operation = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Observed burglary".to_owned(),
                kind: OperationKind::Burglary,
                responsible_organization: organization,
                leader,
                objective: OperationObjective::AcquireProperty {
                    target: EntityRef::Business(business),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([
                    (RoleKind::Coordinator, leader),
                    (RoleKind::EntrySpecialist, specialist),
                ]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: SimTime::from_minutes(1),
            },
        )
        .expect("exposure operation should validate")
        .commit(&mut state)
        .expect("exposure operation should commit");
        (registry, state, police, neighborhood, operation)
    }

    #[test]
    fn scheduled_operation_resolves_into_persisted_after_action_and_history() {
        let (registry, mut state, organization, operation) = make_operation_fixture();
        for minute in 1..=20_u64 {
            let outcome = run_tick(&registry, &mut state);
            assert_eq!(outcome.now, SimTime::from_minutes(minute));
            if minute == 1 {
                assert_eq!(outcome.started_operations, vec![operation]);
            }
            assert!(outcome.resolved_operations.is_empty());
        }

        let outcome = run_tick(&registry, &mut state);
        assert_eq!(outcome.now, SimTime::from_minutes(21));
        assert_eq!(outcome.resolved_operations, vec![operation]);
        let record = state
            .operations()
            .get_operation(operation)
            .expect("resolved operation should remain recorded");
        assert_eq!(record.status(), OperationStatus::Completed);
        let resolution = record
            .resolution()
            .expect("completed operation should persist its resolution");
        let information = state
            .intelligence()
            .get_information(resolution.after_action_information())
            .expect("operation resolution should create after-action information");
        assert_eq!(
            information.holder(),
            KnowledgeHolder::Organization(organization)
        );
        assert_eq!(
            information.source_kind(),
            InformationSourceKind::AfterAction
        );
        assert_eq!(information.subject(), EntityRef::Operation(operation));
        assert!(information.summary().contains("Assigned-role competence"));
        let history = state
            .history()
            .get_event(resolution.history_event())
            .expect("operation resolution should create campaign history");
        assert_eq!(history.kind(), HistoryEventKind::Operation);
        assert!(history
            .entities()
            .contains(&EntityRef::Operation(operation)));
        validate_state(&state).expect("resolved operation state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn authority_exception_pauses_and_shifts_operation_resolution_schedule() {
        let (registry, mut state, organization, operation) = make_operation_fixture();
        for _ in 0..5 {
            run_tick(&registry, &mut state);
        }
        let due_before_pause = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .resolution_due_at()
            .expect("in-progress operation should be scheduled for resolution");
        assert_eq!(due_before_pause, SimTime::from_minutes(21));
        let leader = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .leader();
        let decision = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: "Execution encountered a condition outside standing authority.".to_owned(),
            },
        )
        .expect("operation exception should validate")
        .commit(&mut state)
        .expect("validated operation exception should commit");
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("operation should exist")
                .awaiting_decision_since(),
            Some(SimTime::from_minutes(5))
        );

        for _ in 0..10 {
            let outcome = run_tick(&registry, &mut state);
            assert!(outcome.resolved_operations.is_empty());
        }
        validate_resolve_decision(
            &state,
            decision.decision,
            organization,
            DecisionResponse::Continue,
        )
        .expect("continue response should validate")
        .commit(&mut state)
        .expect("validated continue response should commit");
        let resumed = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist after resume");
        assert_eq!(resumed.status(), OperationStatus::InProgress);
        assert_eq!(resumed.awaiting_decision_since(), None);
        assert_eq!(resumed.resolution_due_at(), Some(SimTime::from_minutes(31)));

        for _ in 0..15 {
            let outcome = run_tick(&registry, &mut state);
            assert!(outcome.resolved_operations.is_empty());
        }
        let outcome = run_tick(&registry, &mut state);
        assert_eq!(outcome.now, SimTime::from_minutes(31));
        assert_eq!(outcome.resolved_operations, vec![operation]);
        validate_state(&state).expect("resumed operation state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn save_round_trip_preserves_deterministic_operation_resolution() {
        let (registry, mut original, _organization, operation) = make_operation_fixture();
        for _ in 0..20 {
            run_tick(&registry, &mut original);
        }
        assert_eq!(
            original
                .operations()
                .get_operation(operation)
                .expect("operation should exist")
                .resolution_due_at(),
            Some(SimTime::from_minutes(21))
        );
        let envelope =
            build_save(&registry, &original).expect("pre-resolution operation state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored =
            restore_save(&registry, decoded).expect("pre-resolution operation save should restore");

        let original_tick = run_tick(&registry, &mut original);
        let restored_tick = run_tick(&registry, &mut restored);
        assert_eq!(original_tick, restored_tick);
        assert_eq!(original_tick.resolved_operations, vec![operation]);
        let original_resolution = original
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("original operation should resolve");
        let restored_resolution = restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("restored operation should resolve");
        assert_eq!(
            original_resolution.objective_outcome(),
            restored_resolution.objective_outcome()
        );
        assert_eq!(
            original_resolution.execution_margin(),
            restored_resolution.execution_margin()
        );
        assert_eq!(original_resolution.factors(), restored_resolution.factors());
        assert_eq!(
            original_resolution.exposure().level(),
            restored_resolution.exposure().level()
        );
        assert_eq!(
            original_resolution.exposure().score(),
            restored_resolution.exposure().score()
        );
        assert_eq!(
            original_resolution.exposure().factors(),
            restored_resolution.exposure().factors()
        );
        validate_state(&restored).expect("deterministically restored resolution should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn completed_operation_remains_valid_after_leader_leaves_organization() {
        let (registry, mut state, _organization, operation) = make_operation_fixture();
        for _ in 0..21 {
            run_tick(&registry, &mut state);
        }
        let leader = state
            .operations()
            .get_operation(operation)
            .expect("completed operation should persist")
            .leader();
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("completed operation should persist")
                .status(),
            OperationStatus::Completed
        );

        validate_reassign_character(&state, leader, None, None)
            .expect("completed operation should no longer bind leader membership")
            .commit(&mut state)
            .expect("leader reassignment should commit after operation completion");
        validate_state(&state).expect("historical operation should survive leader reassignment");
        validate_invariants(&state);

        let envelope = build_save(&registry, &state)
            .expect("historical operation with reassigned leader should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&registry, decoded)
            .expect("historical operation with reassigned leader should restore");
        assert_eq!(
            restored
                .operations()
                .get_operation(operation)
                .expect("restored historical operation should persist")
                .status(),
            OperationStatus::Completed
        );
        validate_invariants(&restored);
    }

    #[test]
    fn fresh_complete_intelligence_improves_execution_and_reduces_exposure() {
        let (registry, mut state, operation) = make_intelligence_operation_fixture();
        let plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due prepared operation should resolve deterministically");
        assert_eq!(plan.factors.intelligence_quality().value(), 99);
        assert_eq!(plan.factors.intelligence_adjustment(), -13);
        assert_eq!(plan.execution_margin, 50);
        assert_eq!(plan.exposure.factors.intelligence_mitigation(), 19);
        assert_eq!(plan.exposure.score, 33);
        assert_eq!(plan.exposure.level, OperationExposureLevel::Trace);

        validate_operation_resolution_plan(&registry, &state, plan)
            .expect("fresh causal resolution plan should validate")
            .commit(&mut state)
            .expect("prepared causal resolution should commit");
        validate_state(&state).expect("intelligence-backed operation state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn neighborhood_exposure_opens_jurisdiction_case_and_survives_save_round_trip() {
        let (registry, mut original, police, _neighborhood, operation) =
            make_exposed_business_operation_fixture(true);
        for _ in 0..45 {
            let tick = run_tick(&registry, &mut original);
            assert!(tick.resolved_operations.is_empty());
        }
        assert_eq!(original.now(), SimTime::from_minutes(45));
        let envelope = build_save(&registry, &original)
            .expect("pre-exposure-resolution operation state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored = restore_save(&registry, decoded)
            .expect("pre-exposure-resolution operation save should restore");

        let original_tick = run_tick(&registry, &mut original);
        let restored_tick = run_tick(&registry, &mut restored);
        assert_eq!(original_tick, restored_tick);
        assert_eq!(original_tick.resolved_operations, vec![operation]);
        for state in [&original, &restored] {
            let resolution = state
                .operations()
                .get_operation(operation)
                .and_then(|record| record.resolution())
                .expect("exposed operation should resolve");
            assert!(matches!(
                resolution.exposure().level(),
                OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
            ));
            let investigation_id = resolution
                .exposure()
                .investigation()
                .expect("jurisdictional exposure should open an investigation");
            let investigation = state
                .legal()
                .get_investigation(investigation_id)
                .expect("operation investigation should persist");
            assert_eq!(investigation.owner(), police);
            assert_eq!(resolution.exposure().evidence().len(), 1);
            let evidence_id = *resolution
                .exposure()
                .evidence()
                .iter()
                .next()
                .expect("operation exposure should persist one evidence record");
            let evidence = state
                .legal()
                .get_evidence(evidence_id)
                .expect("operation evidence should persist");
            assert_eq!(evidence.origin(), Some(EntityRef::Operation(operation)));
            assert_eq!(
                state
                    .legal()
                    .evidence_from_origin(EntityRef::Operation(operation))
                    .map(|record| record.id())
                    .collect::<Vec<_>>(),
                vec![evidence_id]
            );
            validate_state(state).expect("exposure-linked legal state should validate");
            validate_invariants(state);
        }
        let original_exposure = original
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("original exposure should resolve")
            .exposure();
        let restored_exposure = restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("restored exposure should resolve")
            .exposure();
        assert_eq!(original_exposure.level(), restored_exposure.level());
        assert_eq!(original_exposure.score(), restored_exposure.score());
        assert_eq!(original_exposure.factors(), restored_exposure.factors());
        assert_eq!(
            original_exposure.investigation(),
            restored_exposure.investigation()
        );
        assert_eq!(original_exposure.evidence(), restored_exposure.evidence());
    }

    #[test]
    fn exposed_operation_without_jurisdiction_creates_no_implicit_case() {
        let (registry, mut state, _police, _neighborhood, operation) =
            make_exposed_business_operation_fixture(false);
        for _ in 0..46 {
            run_tick(&registry, &mut state);
        }
        let exposure = state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("exposed operation should resolve")
            .exposure();
        assert!(matches!(
            exposure.level(),
            OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
        ));
        assert_eq!(exposure.investigation(), None);
        assert!(exposure.evidence().is_empty());
        assert_eq!(
            state
                .legal()
                .evidence_from_origin(EntityRef::Operation(operation))
                .count(),
            0
        );
        validate_state(&state).expect("unrouted exposure should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn resolution_token_rejects_changed_incident_jurisdiction() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture(true);
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due exposure operation should resolve a plan");
        assert!(matches!(
            plan.exposure.level,
            OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
        ));
        let validated = validate_operation_resolution_plan(&registry, &state, plan)
            .expect("operation incident should validate against jurisdiction version one");

        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(90)
                    .expect("updated case priority should validate"),
            },
        )
        .expect("jurisdiction update should validate")
        .commit(&mut state)
        .expect("jurisdiction update should commit");

        let error = validated
            .commit(&mut state)
            .expect_err("stale incident authority snapshot must reject commit");
        assert_eq!(
            error,
            OperationResolutionError::StaleIncidentJurisdictionVersion {
                neighborhood,
                organization: police,
                expected_version: 1,
                found_version: Some(2),
            }
        );
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("stale resolution must leave operation intact")
                .status(),
            OperationStatus::InProgress
        );
        assert_eq!(
            state
                .legal()
                .evidence_from_origin(EntityRef::Operation(operation))
                .count(),
            0
        );
        validate_state(&state).expect("stale resolution rejection should not corrupt state");
        validate_invariants(&state);
    }

    #[test]
    fn resolution_token_rejects_new_jurisdiction_after_unrouted_validation() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture(false);
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due exposed operation should resolve a plan");
        assert!(matches!(
            plan.exposure.level,
            OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
        ));
        let validated = validate_operation_resolution_plan(&registry, &state, plan)
            .expect("unrouted exposure should validate against absence of jurisdiction");

        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(80)
                    .expect("fixture case priority should validate"),
            },
        )
        .expect("new jurisdiction should validate")
        .commit(&mut state)
        .expect("new jurisdiction should commit");

        let error = validated
            .commit(&mut state)
            .expect_err("new incident authority must stale an unrouted resolution token");
        assert_eq!(
            error,
            OperationResolutionError::StaleIncidentRouting {
                neighborhood,
                expected: None,
                found: Some(police),
            }
        );
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("stale resolution must leave operation intact")
                .status(),
            OperationStatus::InProgress
        );
        assert_eq!(
            state
                .legal()
                .evidence_from_origin(EntityRef::Operation(operation))
                .count(),
            0
        );
        validate_state(&state).expect("stale unrouted resolution must leave valid state");
        validate_invariants(&state);
    }
}
