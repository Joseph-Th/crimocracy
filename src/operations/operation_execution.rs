//! Deterministic operation resolution planning and atomic persistence of causal outcomes.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, NeighborhoodId, OperationId, PoliceResponseId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::economy::business_economy_system::{
    estimate_business_gross_potential, BusinessEconomyError,
};
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
use crate::legal::patrol_system::{
    resolve_patrol_presence_interval_snapshot, resolve_patrol_presence_snapshot,
    PatrolPresenceSnapshot,
};
use crate::legal::{
    Admissibility, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft,
};
use crate::operations::surveillance_integration::{
    decide_surveillance_intelligence, surveillance_after_action_clause,
    validate_surveillance_information, validate_surveillance_plan_snapshot, SurveillanceError,
    SurveillanceIntelligencePlan,
};
use crate::operations::{
    OperationExposureFactors, OperationExposureLevel, OperationExposureRecord, OperationObjective,
    OperationObjectiveOutcome, OperationPropertyProceedsRecord, OperationResolutionFactors,
    OperationResolutionRecord, OperationStatus,
};
use crate::registry::{OperationExecutionDefinition, Registry};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, QualitativeBand, Rating};
use std::collections::{BTreeMap, BTreeSet};
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
    #[error("operation {operation} property-proceeds arithmetic overflowed")]
    PropertyProceedsOverflow { operation: OperationId },
    #[error("operation {operation} property-proceeds context changed after resolution planning")]
    StalePropertyProceedsContext { operation: OperationId },
    #[error("operation {operation} changed after resolution planning; expected version {expected}, found {found}")]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error("operation resolution plan was resolved at {expected:?}, but simulation time is now {found:?}")]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error("police deployment context affecting operation {operation} changed after resolution planning")]
    StalePoliceDeploymentContext { operation: OperationId },
    #[error(
        "police response context affecting operation {operation} changed after resolution planning"
    )]
    StalePoliceResponseContext { operation: OperationId },
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
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    Surveillance(#[from] SurveillanceError),
    #[error(transparent)]
    BusinessEconomy(#[from] BusinessEconomyError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetPoliceSnapshot {
    patrol_by_neighborhood: BTreeMap<NeighborhoodId, PatrolPresenceSnapshot>,
    target_presence: Option<Rating>,
    exposure_neighborhood: Option<NeighborhoodId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoliceResponseResolutionSnapshot {
    response: PoliceResponseId,
    version: u32,
    arrived_at: Option<SimTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationPoliceAlertContext {
    score: i16,
    neighborhood: Option<NeighborhoodId>,
}

impl OperationPoliceAlertContext {
    pub(crate) fn score(self) -> i16 {
        self.score
    }

    pub(crate) fn neighborhood(self) -> Option<NeighborhoodId> {
        self.neighborhood
    }
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
struct OperationResolutionSnapshot {
    operation: OperationId,
    expected_operation_version: u32,
    resolved_at: SimTime,
    police_snapshot: TargetPoliceSnapshot,
    police_response: Option<PoliceResponseResolutionSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationResolutionOutcomePlan {
    objective_outcome: OperationObjectiveOutcome,
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposurePlan,
    property_proceeds: Option<OperationPropertyProceedsRecord>,
    surveillance: Option<SurveillanceIntelligencePlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationResolutionNarrative {
    summary: String,
    history_entities: BTreeSet<EntityRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationResolutionPlan {
    snapshot: OperationResolutionSnapshot,
    outcome: OperationResolutionOutcomePlan,
    narrative: OperationResolutionNarrative,
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
    let leader_capability = state
        .world
        .get_character(record.leader())
        .and_then(|leader| leader.capability(execution.leader_capability()));
    let (
        intelligence_quality,
        intelligence_adjustment,
        intelligence_topics_covered,
        intelligence_topics_relevant,
    ) = calculate_intelligence_factors(registry, state, operation);
    let started_at = record
        .started_at()
        .expect("in-progress operation must have a start time");
    let police_snapshot = resolve_target_police_interval_snapshot(
        state,
        record.objective().referenced_entities(),
        started_at,
        state.now(),
    );
    let target_police_presence = police_snapshot.target_presence;
    let police_response_arrived = did_police_response_arrive_by(state, record, state.now());
    let police_response = record.police_response().map(|response_id| {
        let response = state
            .legal
            .get_police_response(response_id)
            .expect("operation police-response link must reference a persisted response");
        PoliceResponseResolutionSnapshot {
            response: response_id,
            version: response.version(),
            arrived_at: response.arrived_at(),
        }
    });
    let approach_adjustment = execution
        .approach_difficulty_adjustment(record.approach())
        .expect("validated operation approach must have an authored execution adjustment");
    let time_pressure =
        resolve_time_pressure(started_at, due_at, execution.duration().as_minutes());

    let factors = OperationResolutionFactors {
        role_capability_average,
        leader_capability,
        intelligence_quality,
        intelligence_adjustment,
        intelligence_topics_covered,
        intelligence_topics_relevant,
        target_police_presence,
        police_response_arrived,
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
        &police_snapshot,
        police_response_arrived,
    );
    let property_proceeds =
        calculate_property_proceeds(registry, state, record, objective_outcome)?;
    let surveillance = decide_surveillance_intelligence(state, record, objective_outcome)?;
    let mut summary = build_after_action_summary(objective_outcome, factors, exposure.level());
    if let Some(proceeds) = property_proceeds {
        summary.push(' ');
        summary.push_str(&format!(
            "The crew secured property with an estimated held value of {} cents; it remains unliquidated.",
            proceeds.estimated_value().cents()
        ));
    }
    if let Some(clause) = surveillance_after_action_clause(surveillance.as_ref(), objective_outcome)
    {
        summary.push(' ');
        summary.push_str(&clause);
    }
    let mut history_entities = BTreeSet::from([
        EntityRef::Operation(operation),
        EntityRef::Organization(record.responsible_organization()),
        EntityRef::Character(record.leader()),
    ]);
    history_entities.extend(record.objective().referenced_entities());
    history_entities.extend(record.roles().values().copied().map(EntityRef::Character));
    if police_response_arrived {
        if let Some(response) = record
            .police_response()
            .and_then(|id| state.legal.get_police_response(id))
        {
            history_entities.insert(EntityRef::Organization(response.authority()));
            history_entities.insert(EntityRef::Neighborhood(response.neighborhood()));
        }
    }

    Ok(OperationResolutionPlan {
        snapshot: OperationResolutionSnapshot {
            operation,
            expected_operation_version: record.version(),
            resolved_at: state.now(),
            police_snapshot,
            police_response,
        },
        outcome: OperationResolutionOutcomePlan {
            objective_outcome,
            execution_margin,
            factors,
            exposure,
            property_proceeds,
            surveillance,
        },
        narrative: OperationResolutionNarrative {
            summary,
            history_entities,
        },
    })
}

pub(crate) struct ValidatedOperationResolution {
    plan: OperationResolutionPlan,
    incident: Option<ValidatedIncidentIntake>,
    incident_authority: Option<IncidentAuthoritySnapshot>,
    surveillance_information: Vec<ValidatedInformation>,
    legal_activity_information: Option<ValidatedInformation>,
    information: ValidatedInformation,
    history: ValidatedHistoryEvent,
    report: ValidatedReport,
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
            level: self.plan.outcome.exposure.level,
            score: self.plan.outcome.exposure.score,
            factors: self.plan.outcome.exposure.factors,
            neighborhood: self.plan.outcome.exposure.neighborhood,
            identified_character: self.plan.outcome.exposure.identified_character,
            investigation,
            evidence,
        };
        let legal_activity_information = self
            .legal_activity_information
            .map(|information| information.commit(state));
        let discovered_information = self
            .surveillance_information
            .into_iter()
            .map(|information| information.commit(state))
            .collect();
        let after_action_information = self.information.commit(state);
        let history_event = self.history.commit(state);
        let after_action_report = self.report.commit(state);
        state.operations.complete(
            self.plan.snapshot.operation,
            OperationResolutionRecord {
                resolved_at: self.plan.snapshot.resolved_at,
                objective_outcome: self.plan.outcome.objective_outcome,
                execution_margin: self.plan.outcome.execution_margin,
                factors: self.plan.outcome.factors,
                exposure,
                property_proceeds: self.plan.outcome.property_proceeds,
                discovered_information,
                legal_activity_information,
                after_action_information,
                after_action_report,
                history_event,
            },
        );
        Ok(self.plan.snapshot.operation)
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
        .get_operation(plan.snapshot.operation)
        .expect("validated resolution operation must exist");
    let expected_property_proceeds =
        calculate_property_proceeds(registry, state, record, plan.outcome.objective_outcome)?;
    if plan.outcome.property_proceeds != expected_property_proceeds {
        return Err(OperationResolutionError::StalePropertyProceedsContext {
            operation: plan.snapshot.operation,
        });
    }
    let surveillance_information = match &plan.outcome.surveillance {
        Some(surveillance) => validate_surveillance_information(
            state,
            record.responsible_organization(),
            record.id(),
            surveillance,
        )?,
        None => Vec::new(),
    };
    let (incident, incident_authority) = validate_exposure_incident(
        registry,
        state,
        record,
        &plan.outcome.exposure,
        plan.snapshot.resolved_at,
    )?;
    let legal_activity_summary = if incident.is_some() {
        let snapshot = incident_authority.expect("a validated incident must have a snapshot");
        Some(build_legal_activity_summary(
            state,
            record,
            snapshot
                .organization
                .expect("a validated incident must have an intake authority"),
        ))
    } else {
        None
    };
    let legal_activity_information = legal_activity_summary.as_ref().map(|summary| {
        validate_record_information(
            state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(record.responsible_organization()),
                source_kind: InformationSourceKind::AfterAction,
                topic: crate::intelligence::InformationTopic::LegalActivity,
                source_entity: Some(EntityRef::Character(record.leader())),
                subject: EntityRef::Operation(record.id()),
                observed_at: plan.snapshot.resolved_at,
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: summary.clone(),
            },
        )
    });
    let legal_activity_information = legal_activity_information.transpose()?;
    let after_action_summary = legal_activity_summary.map_or_else(
        || plan.narrative.summary.clone(),
        |summary| format!("{} {}", plan.narrative.summary, summary),
    );
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(record.responsible_organization()),
            source_kind: InformationSourceKind::AfterAction,
            topic: crate::intelligence::InformationTopic::OperationalOutcome,
            source_entity: Some(EntityRef::Character(record.leader())),
            subject: EntityRef::Operation(record.id()),
            observed_at: plan.snapshot.resolved_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: after_action_summary.clone(),
        },
    )?;
    let history = validate_record_event(
        state,
        HistoryEventDraft {
            occurred_at: plan.snapshot.resolved_at,
            kind: HistoryEventKind::Operation,
            summary: format!(
                "{} ended with objective {}.",
                record.title(),
                outcome_label(plan.outcome.objective_outcome)
            ),
            entities: plan.narrative.history_entities.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: record.responsible_organization(),
            kind: ReportKind::AfterAction,
            title: format!("{} after-action report", record.title()),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: after_action_summary,
                sources: Vec::new(),
                entities: plan.narrative.history_entities.clone(),
                decision: None,
            }],
        },
    )?;
    Ok(ValidatedOperationResolution {
        plan,
        incident,
        incident_authority,
        surveillance_information,
        legal_activity_information,
        information,
        history,
        report,
    })
}

pub(crate) fn build_legal_activity_summary(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    authority: crate::core::id::OrganizationId,
) -> String {
    let authority_name = state
        .world
        .get_organization(authority)
        .expect("validated incident authority must exist")
        .name();
    format!(
        "The exposure from {} produced a police investigation opened by {}. The organization does not know the case's evidence, lead, or detective work.",
        operation.title(),
        authority_name
    )
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
        .get_operation(plan.snapshot.operation)
        .ok_or(OperationResolutionError::MissingOperation(
            plan.snapshot.operation,
        ))?;
    if record.version() != plan.snapshot.expected_operation_version {
        return Err(OperationResolutionError::StaleOperation {
            operation: plan.snapshot.operation,
            expected: plan.snapshot.expected_operation_version,
            found: record.version(),
        });
    }
    if record.status() != OperationStatus::InProgress {
        return Err(OperationResolutionError::OperationNotInProgress(
            plan.snapshot.operation,
        ));
    }
    let due_at = record
        .resolution_due_at()
        .expect("in-progress operation must have a resolution due time");
    if plan.snapshot.resolved_at < due_at {
        return Err(OperationResolutionError::ResolutionNotDue {
            operation: plan.snapshot.operation,
            due_at,
        });
    }
    if state.now() != plan.snapshot.resolved_at {
        return Err(OperationResolutionError::StaleResolutionTime {
            expected: plan.snapshot.resolved_at,
            found: state.now(),
        });
    }
    let current_police_snapshot = resolve_target_police_interval_snapshot(
        state,
        record.objective().referenced_entities(),
        record
            .started_at()
            .expect("in-progress operation must have a start time"),
        plan.snapshot.resolved_at,
    );
    if current_police_snapshot != plan.snapshot.police_snapshot
        || plan.outcome.factors.target_police_presence()
            != plan.snapshot.police_snapshot.target_presence
        || plan.outcome.exposure.neighborhood != plan.snapshot.police_snapshot.exposure_neighborhood
        || plan.outcome.exposure.factors.target_police_presence()
            != plan.snapshot.police_snapshot.target_presence
    {
        return Err(OperationResolutionError::StalePoliceDeploymentContext {
            operation: plan.snapshot.operation,
        });
    }
    let current_police_response = record.police_response().map(|response_id| {
        let response = state
            .legal
            .get_police_response(response_id)
            .expect("operation police-response link must reference a persisted response");
        PoliceResponseResolutionSnapshot {
            response: response_id,
            version: response.version(),
            arrived_at: response.arrived_at(),
        }
    });
    if current_police_response != plan.snapshot.police_response
        || did_police_response_arrive_by(state, record, plan.snapshot.resolved_at)
            != plan.outcome.factors.police_response_arrived()
        || plan.outcome.exposure.factors.police_response_arrived()
            != plan.outcome.factors.police_response_arrived()
    {
        return Err(OperationResolutionError::StalePoliceResponseContext {
            operation: plan.snapshot.operation,
        });
    }
    if let Some(surveillance) = &plan.outcome.surveillance {
        validate_surveillance_plan_snapshot(state, surveillance)?;
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

fn resolve_target_police_snapshot(
    state: &AppState,
    entities: Vec<EntityRef>,
    at: SimTime,
) -> TargetPoliceSnapshot {
    let neighborhoods = resolve_target_neighborhoods(state, entities);
    let mut patrol_by_neighborhood = BTreeMap::new();
    let mut strongest: Option<(NeighborhoodId, Rating)> = None;
    for neighborhood in neighborhoods {
        let patrol = resolve_patrol_presence_snapshot(state, neighborhood, at);
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

fn resolve_target_police_interval_snapshot(
    state: &AppState,
    entities: Vec<EntityRef>,
    start: SimTime,
    end: SimTime,
) -> TargetPoliceSnapshot {
    let neighborhoods = resolve_target_neighborhoods(state, entities);
    let mut patrol_by_neighborhood = BTreeMap::new();
    let mut strongest: Option<(NeighborhoodId, Rating)> = None;
    for neighborhood in neighborhoods {
        let patrol = resolve_patrol_presence_interval_snapshot(state, neighborhood, start, end);
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

fn resolve_target_neighborhoods(
    state: &AppState,
    entities: Vec<EntityRef>,
) -> BTreeSet<NeighborhoodId> {
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
}

fn calculate_exposure_plan(
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

pub(crate) fn calculate_operation_police_alert_context(
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
        resolve_target_police_snapshot(state, record.objective().referenced_entities(), at);
    let stealth_average = resolve_stealth_average(state, record);
    let (intelligence_quality, _, _, _) =
        calculate_intelligence_factors(registry, state, operation);
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
        score: calculate_exposure_score(execution, factors),
        neighborhood: police_snapshot.exposure_neighborhood,
    }
}

pub(crate) fn did_police_response_arrive_by(
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

fn weighted_ability(role_average: Rating, leader_capability: Option<Rating>) -> i16 {
    let role = i16::from(role_average.value());
    let leadership = leader_capability
        .map(|rating| i16::from(rating.value()))
        .unwrap_or(role);
    (role * 3 + leadership) / 4
}

pub(crate) fn calculate_intelligence_factors(
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
        let score = information_score(information, planning_at, max_age);
        best_by_topic
            .entry(information.topic())
            .and_modify(|best| *best = (*best).max(score))
            .or_insert(score);
    }

    let relevant_topics = execution.relevant_intelligence_topics();
    let covered = relevant_topics
        .iter()
        .filter(|topic| best_by_topic.contains_key(topic))
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

pub(crate) fn calculate_property_proceeds(
    registry: &Registry,
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<Option<OperationPropertyProceedsRecord>, OperationResolutionError> {
    let Some(definition) = registry
        .get_operation(operation.kind())
        .execution()
        .property_proceeds()
    else {
        return Ok(None);
    };
    let OperationObjective::AcquireProperty {
        target: EntityRef::Business(business),
    } = operation.objective()
    else {
        return Ok(None);
    };
    if outcome == OperationObjectiveOutcome::Failed {
        return Ok(None);
    }

    let gross = estimate_business_gross_potential(registry, state, *business)?;
    let full_value = i128::from(gross.cents())
        .checked_mul(i128::from(definition.business_gross_basis_points()))
        .ok_or(OperationResolutionError::PropertyProceedsOverflow {
            operation: operation.id(),
        })?
        / 10_000_i128;
    let value = match outcome {
        OperationObjectiveOutcome::Achieved => full_value,
        OperationObjectiveOutcome::Partial => {
            full_value
                .checked_mul(i128::from(definition.partial_recovery_basis_points()))
                .ok_or(OperationResolutionError::PropertyProceedsOverflow {
                    operation: operation.id(),
                })?
                / 10_000_i128
        }
        OperationObjectiveOutcome::Failed => {
            unreachable!("failed property operations return early")
        }
    };
    let cents =
        i64::try_from(value).map_err(|_| OperationResolutionError::PropertyProceedsOverflow {
            operation: operation.id(),
        })?;
    if cents <= 0 {
        return Ok(None);
    }
    Ok(Some(OperationPropertyProceedsRecord::new(
        EntityRef::Business(*business),
        crate::finance::Money::from_cents(cents),
    )))
}

fn build_after_action_summary(
    outcome: OperationObjectiveOutcome,
    factors: OperationResolutionFactors,
    exposure: OperationExposureLevel,
) -> String {
    let leadership = factors
        .leader_capability()
        .map(|rating| {
            format!(
                "Leadership coordination was {}.",
                band_label(rating.qualitative_band())
            )
        })
        .unwrap_or_else(|| {
            "Leadership had no demonstrated capability for the execution.".to_owned()
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
    let response = if factors.police_response_arrived() {
        "Law-enforcement response reached the target before the operation ended."
    } else {
        "No law-enforcement response reached the target before the operation ended."
    };
    let covered = factors.intelligence_topics_covered();
    let relevant = factors.intelligence_topics_relevant();
    let coverage = if covered == 0 {
        format!("Planning intelligence covered 0 of {relevant} relevant areas")
    } else if covered == relevant {
        format!("Planning intelligence covered all {relevant} relevant areas")
    } else {
        format!("Planning intelligence covered {covered} of {relevant} relevant areas")
    };
    let intelligence = match factors.intelligence_adjustment() {
        value if value < 0 => {
            format!("{coverage}; the available reports reduced execution uncertainty.")
        }
        0 => format!("{coverage} and provided no material execution advantage."),
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
    let circumstances = match (outcome, factors.variance()) {
        (OperationObjectiveOutcome::Achieved, value) if value < 0 => {
            "Unplanned circumstances were adverse, but the crew overcame them."
        }
        (OperationObjectiveOutcome::Partial, value) if value < 0 => {
            "Adverse unplanned circumstances reduced the result."
        }
        (OperationObjectiveOutcome::Failed, value) if value < 0 => {
            "Adverse unplanned circumstances contributed to the failure."
        }
        (_, 0) => "Unplanned circumstances were neutral.",
        (_, _) => "Favorable unplanned circumstances improved the result.",
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
        "Objective {}. Assigned-role competence was {}. {} {} {} {} {} {} {} {}",
        outcome_label(outcome),
        band_label(factors.role_capability_average().qualitative_band()),
        leadership,
        intelligence,
        police,
        response,
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
    use crate::core::id::{BusinessId, FinancialAccountId, OrganizationId};
    use crate::core::invariants::{
        validate_invariants, validate_state, validate_state_against_registry,
    };
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::core::time::SimDuration;
    use crate::decisions::decision_system::{
        validate_request_decision, validate_resolve_decision, DecisionError,
    };
    use crate::decisions::{
        DecisionContext, DecisionRequestDraft, DecisionResponse, OperationExceptionReason,
    };
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{InformationDraft, InformationTopic};
    use crate::legal::jurisdiction_system::validate_set_jurisdiction;
    use crate::legal::patrol_system::{
        validate_establish_patrol_deployment, validate_revise_patrol_deployment,
    };
    use crate::legal::{DayMinute, JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow};
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::property_disposition::{
        validate_dispose_property, PropertyDispositionDraft, PropertyDispositionError,
    };
    use crate::operations::{
        OperationAbortCause, OperationAbortPhase, OperationApproach, OperationContingency,
        OperationDraft, OperationKind, OperationObjective, OperationStatus, RoleKind,
    };
    use crate::reports::organization_financial_report::validate_organization_financial_report;
    use crate::world::world_system::{
        designate_player_organization, insert_business, insert_character, insert_neighborhood,
        insert_organization, validate_reassign_character,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn insert_property_disposition_fixture(
        registry: &Registry,
        state: &mut AppState,
        neighborhood: NeighborhoodId,
        organization: OrganizationId,
    ) -> (BusinessId, FinancialAccountId, FinancialAccountId) {
        let resale_venue = insert_business(
            registry,
            state,
            BusinessDraft {
                name: "Fixture Pawn Exchange".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::ResaleMarket,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(organization),
            },
        )
        .expect("resale venue should validate");
        let cash_account = insert_account(
            state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
                label: "Fixture liquidation cash".to_owned(),
            },
        )
        .expect("liquidation cash account should validate");
        let settlement_account = insert_account(
            state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
                label: "Fixture liquidation settlement".to_owned(),
            },
        )
        .expect("liquidation settlement account should validate");
        (resale_venue, cash_account, settlement_account)
    }

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
        make_exposed_business_operation_fixture_with_contingencies(assign_jurisdiction, Vec::new())
    }

    fn make_exposed_business_operation_fixture_with_contingencies(
        assign_jurisdiction: bool,
        contingencies: Vec<OperationContingency>,
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
                contingencies,
                scheduled_for: SimTime::from_minutes(1),
            },
        )
        .expect("exposure operation should validate")
        .commit(&mut state)
        .expect("exposure operation should commit");
        (registry, state, police, neighborhood, operation)
    }

    #[test]
    fn scheduled_operation_resolves_into_persisted_after_action_report_information_and_history() {
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
        let report = state
            .reports()
            .get_report(resolution.after_action_report())
            .expect("operation resolution should create an after-action report");
        assert_eq!(report.kind(), ReportKind::AfterAction);
        assert_eq!(report.recipient(), organization);
        assert_eq!(report.generated_at(), resolution.resolved_at());
        assert_eq!(report.entries().len(), 1);
        assert_eq!(report.entries()[0].attention, AttentionClass::Notable);
        assert_eq!(report.entries()[0].summary, information.summary());
        assert!(report.entries()[0].sources.is_empty());
        assert!(report.entries()[0].decision.is_none());
        assert!(report.entries()[0]
            .entities
            .contains(&EntityRef::Operation(operation)));
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
    fn after_action_summary_contextualizes_adverse_variance() {
        let factors = OperationResolutionFactors {
            role_capability_average: Rating::try_new(80).expect("fixture rating should be valid"),
            leader_capability: Some(Rating::try_new(80).expect("fixture rating should be valid")),
            intelligence_quality: Rating::try_new(0).expect("fixture rating should be valid"),
            intelligence_adjustment: 0,
            intelligence_topics_covered: 0,
            intelligence_topics_relevant: 1,
            target_police_presence: None,
            police_response_arrived: false,
            approach_adjustment: 0,
            time_pressure: 0,
            variance: -1,
        };

        let achieved = build_after_action_summary(
            OperationObjectiveOutcome::Achieved,
            factors,
            OperationExposureLevel::None,
        );
        assert!(achieved.contains("but the crew overcame them"));

        let failed = build_after_action_summary(
            OperationObjectiveOutcome::Failed,
            factors,
            OperationExposureLevel::None,
        );
        assert!(failed.contains("contributed to the failure"));
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
            &registry,
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
    fn police_response_arrives_during_decision_pause_and_continue_honors_standing_abort() {
        let (registry, mut state, _police, _neighborhood, operation) =
            make_exposed_business_operation_fixture_with_contingencies(
                true,
                vec![
                    OperationContingency::AbortOnPoliceArrivalBeforeEntry,
                    OperationContingency::RequestDecisionOnUnexpectedCondition,
                ],
            );
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("started operation should persist");
        let response_id = operation_record
            .police_response()
            .expect("observable burglary should dispatch police response");
        let organization = operation_record.responsible_organization();
        let leader = operation_record.leader();

        let second_tick = run_tick(&registry, &mut state);
        assert!(second_tick.arrived_police_responses.is_empty());
        let decision = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: "Entry team encountered an unexpected security condition.".to_owned(),
            },
        )
        .expect("operation exception should validate")
        .commit(&mut state)
        .expect("operation exception should commit");
        let paused_at = state.now();
        assert_eq!(paused_at, SimTime::from_minutes(2));

        let response_due = state
            .legal()
            .get_police_response(response_id)
            .expect("response should persist")
            .arrival_due_at();
        while state.now() < response_due {
            let outcome = run_tick(&registry, &mut state);
            assert_eq!(
                state
                    .operations()
                    .get_operation(operation)
                    .expect("decision-blocked operation should persist")
                    .status(),
                OperationStatus::AwaitingDecision
            );
            if outcome.now < response_due {
                assert!(outcome.arrived_police_responses.is_empty());
            }
        }
        assert_eq!(
            state
                .legal()
                .get_police_response(response_id)
                .and_then(|response| response.arrived_at()),
            Some(response_due)
        );
        assert!(state
            .decisions()
            .get_decision(decision.decision)
            .expect("pending decision should persist")
            .resolution()
            .is_none());

        for _ in 0..2 {
            let outcome = run_tick(&registry, &mut state);
            assert!(outcome.resolved_operations.is_empty());
        }
        let resolved_at = state.now();
        validate_resolve_decision(
            &registry,
            &state,
            decision.decision,
            organization,
            DecisionResponse::Continue,
        )
        .expect("continue should validate while preserving standing contingencies")
        .commit(&mut state)
        .expect("continue resolution should atomically honor the police contingency");

        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("aborted operation should persist");
        assert_eq!(operation_record.status(), OperationStatus::Aborted);
        assert_eq!(operation_record.awaiting_decision_since(), Some(paused_at));
        let abort = operation_record
            .abort_record()
            .expect("standing contingency should persist abort history");
        assert_eq!(abort.phase(), OperationAbortPhase::AwaitingDecision);
        assert_eq!(
            abort.cause(),
            OperationAbortCause::PoliceArrival(response_id)
        );
        assert_eq!(abort.aborted_at(), resolved_at);
        let decision_resolution = state
            .decisions()
            .get_decision(decision.decision)
            .and_then(|decision| decision.resolution())
            .expect("continue decision should remain historical");
        assert_eq!(decision_resolution.response(), DecisionResponse::Continue);
        assert_eq!(decision_resolution.resolved_at(), resolved_at);
        validate_state(&state).expect("continuous-time police response state should validate");
        validate_state_against_registry(&registry, &state)
            .expect("continuous-time response state should match authored content");
        validate_invariants(&state);
    }

    #[test]
    fn authority_exception_abort_persists_decision_provenance_and_after_action_artifacts() {
        let (registry, mut state, organization, operation) = make_operation_fixture();
        for _ in 0..5 {
            run_tick(&registry, &mut state);
        }
        let leader = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .leader();
        let decision_summary =
            "Alarm hardware differs from the intelligence and exceeds standing authority.";
        let decision = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: decision_summary.to_owned(),
            },
        )
        .expect("operation exception should validate")
        .commit(&mut state)
        .expect("operation exception should commit");

        let outcome = validate_resolve_decision(
            &registry,
            &state,
            decision.decision,
            organization,
            DecisionResponse::Abort,
        )
        .expect("abort response should validate")
        .commit(&mut state)
        .expect("abort response should atomically terminate the operation");
        assert!(outcome.recruitment_attempt.is_none());

        let record = state
            .operations()
            .get_operation(operation)
            .expect("aborted operation should persist");
        assert_eq!(record.status(), OperationStatus::Aborted);
        assert!(record.resolution().is_none());
        let abort = record
            .abort_record()
            .expect("decision abort should persist abort provenance");
        assert_eq!(abort.aborted_at(), SimTime::from_minutes(5));
        assert_eq!(abort.phase(), OperationAbortPhase::AwaitingDecision);
        assert_eq!(
            abort.cause(),
            OperationAbortCause::Decision(decision.decision)
        );
        let decision_record = state
            .decisions()
            .get_decision(decision.decision)
            .expect("resolved decision should persist");
        let resolution = decision_record
            .resolution()
            .expect("abort decision should be resolved");
        assert_eq!(resolution.response(), DecisionResponse::Abort);
        assert_eq!(resolution.resolved_at(), abort.aborted_at());

        let artifacts = abort
            .artifacts()
            .expect("abort after execution began should create after-action artifacts");
        let information = state
            .intelligence()
            .get_information(artifacts.information())
            .expect("abort information should persist");
        assert!(information.summary().contains(decision_summary));
        let report = state
            .reports()
            .get_report(artifacts.report())
            .expect("abort report should persist");
        assert_eq!(report.entries()[0].summary, information.summary());
        assert!(report.entries()[0]
            .entities
            .contains(&EntityRef::DecisionRequest(decision.decision)));
        let history = state
            .history()
            .get_event(artifacts.history_event())
            .expect("abort history should persist");
        assert_eq!(history.summary(), information.summary());
        assert!(history
            .entities()
            .contains(&EntityRef::DecisionRequest(decision.decision)));

        for _ in 0..30 {
            let tick = run_tick(&registry, &mut state);
            assert!(!tick.resolved_operations.contains(&operation));
        }
        validate_state(&state).expect("decision-aborted operation state should validate");
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
        assert_eq!(
            original_resolution.after_action_report(),
            restored_resolution.after_action_report()
        );
        let original_report = original
            .reports()
            .get_report(original_resolution.after_action_report())
            .expect("original after-action report should persist");
        let restored_report = restored
            .reports()
            .get_report(restored_resolution.after_action_report())
            .expect("restored after-action report should persist");
        assert_eq!(original_report.title(), restored_report.title());
        assert_eq!(
            original_report.entries()[0].summary,
            restored_report.entries()[0].summary
        );
        validate_state(&restored).expect("deterministically restored resolution should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn same_minute_operation_after_action_is_included_in_due_executive_brief() {
        let (registry, mut state, organization, operation) = make_operation_fixture();
        designate_player_organization(&mut state, organization)
            .expect("operation organization should be eligible as the player organization");

        state.advance_clock(SimDuration::from_minutes(1_419));
        let start_tick = run_tick(&registry, &mut state);
        assert_eq!(start_tick.now, SimTime::from_minutes(1_420));
        assert_eq!(start_tick.started_operations, vec![operation]);
        assert!(start_tick.executive_brief.is_none());
        for _ in 0..19 {
            let tick = run_tick(&registry, &mut state);
            assert!(tick.resolved_operations.is_empty());
            assert!(tick.executive_brief.is_none());
        }

        let boundary_tick = run_tick(&registry, &mut state);
        assert_eq!(boundary_tick.now, SimTime::from_minutes(1_440));
        assert_eq!(boundary_tick.resolved_operations, vec![operation]);
        let executive_brief = boundary_tick
            .executive_brief
            .expect("daily boundary should synthesize an executive brief");
        let resolution = state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("operation should resolve at the daily boundary");
        assert!(resolution.after_action_report() < executive_brief);
        let after_action = state
            .reports()
            .get_report(resolution.after_action_report())
            .expect("same-minute after-action report should persist");
        let executive = state
            .reports()
            .get_report(executive_brief)
            .expect("same-minute executive brief should persist");
        assert!(executive.entries().iter().any(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == after_action.entries()[0].summary
                && entry.entities.contains(&EntityRef::Operation(operation))
        }));
        validate_state(&state).expect("same-minute synthesis state should validate");
        validate_invariants(&state);
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
        assert_eq!(plan.outcome.factors.intelligence_quality().value(), 99);
        assert_eq!(plan.outcome.factors.intelligence_adjustment(), -13);
        assert_eq!(plan.outcome.execution_margin, 50);
        assert_eq!(plan.outcome.exposure.factors.intelligence_mitigation(), 19);
        assert_eq!(plan.outcome.exposure.score, 33);
        assert_eq!(plan.outcome.exposure.level, OperationExposureLevel::Trace);

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
            let legal_activity_information = resolution
                .legal_activity_information()
                .expect("jurisdictional exposure should create player legal-activity knowledge");
            let legal_activity = state
                .intelligence()
                .get_information(legal_activity_information)
                .expect("player legal-activity information should persist");
            assert_eq!(legal_activity.topic(), InformationTopic::LegalActivity);
            assert_eq!(legal_activity.subject(), EntityRef::Operation(operation));
            assert!(legal_activity
                .summary()
                .contains("produced a police investigation"));
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
        assert_eq!(
            original
                .operations()
                .get_operation(operation)
                .and_then(|record| record.resolution())
                .and_then(|resolution| resolution.legal_activity_information()),
            restored
                .operations()
                .get_operation(operation)
                .and_then(|record| record.resolution())
                .and_then(|resolution| resolution.legal_activity_information())
        );
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
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .and_then(|record| record.resolution())
                .and_then(|resolution| resolution.legal_activity_information()),
            None
        );
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
    fn patrol_presence_controls_persisted_police_response_delay() {
        let (low_registry, mut low_state, low_police, low_neighborhood, low_operation) =
            make_exposed_business_operation_fixture(true);
        validate_establish_patrol_deployment(
            &low_state,
            PatrolDeploymentDraft {
                organization: low_police,
                neighborhood: low_neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("zero-presence patrol should validate")
        .commit(&mut low_state)
        .expect("zero-presence patrol should commit");
        let low_start = run_tick(&low_registry, &mut low_state);
        assert_eq!(low_start.started_operations, vec![low_operation]);
        let low_response_id = low_state
            .operations()
            .get_operation(low_operation)
            .and_then(|record| record.police_response())
            .expect("observable burglary should dispatch a response");
        let low_response = low_state
            .legal()
            .get_police_response(low_response_id)
            .expect("low-presence response should persist");
        assert_eq!(low_response.response_presence().value(), 0);
        assert_eq!(
            low_response.arrival_due_at().as_minutes() - low_response.dispatched_at().as_minutes(),
            12
        );

        let (high_registry, mut high_state, high_police, high_neighborhood, high_operation) =
            make_exposed_business_operation_fixture(true);
        validate_establish_patrol_deployment(
            &high_state,
            PatrolDeploymentDraft {
                organization: high_police,
                neighborhood: high_neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(100).expect("full patrol presence should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("full-presence patrol should validate")
        .commit(&mut high_state)
        .expect("full-presence patrol should commit");
        let high_start = run_tick(&high_registry, &mut high_state);
        assert_eq!(high_start.started_operations, vec![high_operation]);
        let high_response_id = high_state
            .operations()
            .get_operation(high_operation)
            .and_then(|record| record.police_response())
            .expect("observable burglary should dispatch a response");
        let high_response = high_state
            .legal()
            .get_police_response(high_response_id)
            .expect("high-presence response should persist");
        assert_eq!(high_response.response_presence().value(), 100);
        assert_eq!(
            high_response.arrival_due_at().as_minutes()
                - high_response.dispatched_at().as_minutes(),
            3
        );

        validate_state_against_registry(&low_registry, &low_state)
            .expect("low-presence response state should match authored content");
        validate_state_against_registry(&high_registry, &high_state)
            .expect("high-presence response state should match authored content");
        validate_invariants(&low_state);
        validate_invariants(&high_state);
    }

    #[test]
    fn police_arrival_before_entry_executes_standing_abort_contingency() {
        let (registry, mut state, police, _neighborhood, operation) =
            make_exposed_business_operation_fixture_with_contingencies(
                true,
                vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
            );
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("started operation should persist");
        let response_id = operation_record
            .police_response()
            .expect("high-observation burglary should dispatch police response");
        let entry_at = operation_record
            .entry_at()
            .expect("burglary should have an authored entry milestone");

        let mut arrival_tick = None;
        while state.now() < entry_at {
            let outcome = run_tick(&registry, &mut state);
            if outcome.arrived_police_responses.contains(&response_id) {
                arrival_tick = Some(outcome.now);
                break;
            }
        }
        let arrived_at = arrival_tick.expect("police response should arrive before burglary entry");
        assert!(arrived_at < entry_at);
        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("aborted operation should persist");
        assert_eq!(operation_record.status(), OperationStatus::Aborted);
        let abort = operation_record
            .abort_record()
            .expect("standing police contingency should create abort history");
        assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
        assert_eq!(
            abort.cause(),
            OperationAbortCause::PoliceArrival(response_id)
        );
        assert!(operation_record.resolution().is_none());
        assert_eq!(
            state
                .legal()
                .get_police_response(response_id)
                .and_then(|response| response.arrived_at()),
            Some(arrived_at)
        );
        let mut participants = operation_record
            .roles()
            .values()
            .copied()
            .collect::<BTreeSet<_>>();
        participants.insert(operation_record.leader());
        for participant in participants {
            let pressure: Vec<_> = state
                .intelligence()
                .information_for_holder_by_topic(
                    KnowledgeHolder::Character(participant),
                    InformationTopic::PoliceActivity,
                )
                .collect();
            assert_eq!(pressure.len(), 1);
            assert_eq!(
                pressure[0].source_kind(),
                InformationSourceKind::DirectObservation
            );
            assert_eq!(
                pressure[0].source_entity(),
                Some(EntityRef::Organization(police))
            );
            assert_eq!(pressure[0].subject(), EntityRef::Character(participant));
            assert_eq!(pressure[0].observed_at(), arrived_at);
            assert_eq!(pressure[0].reliability(), Reliability::DirectAccess);
            assert_eq!(pressure[0].specificity(), Specificity::Precise);
        }
        validate_state(&state).expect("police-contingency abort state should remain valid");
        validate_state_against_registry(&registry, &state)
            .expect("police-contingency abort should match authored content");
        validate_invariants(&state);
    }

    #[test]
    fn post_entry_police_arrival_raises_provenance_backed_decision() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture_with_contingencies(
                true,
                vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
            );
        validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("zero-presence patrol should validate")
        .commit(&mut state)
        .expect("zero-presence patrol should commit");

        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("started operation should persist");
        let response_id = operation_record
            .police_response()
            .expect("observable burglary should dispatch police response");
        let entry_at = operation_record
            .entry_at()
            .expect("burglary should have an authored entry milestone");
        let response_due = state
            .legal()
            .get_police_response(response_id)
            .expect("response should persist")
            .arrival_due_at();
        assert!(response_due > entry_at);

        let arrival_outcome = loop {
            let outcome = run_tick(&registry, &mut state);
            if outcome.arrived_police_responses.contains(&response_id) {
                break outcome;
            }
        };
        assert_eq!(arrival_outcome.now, response_due);
        assert_eq!(arrival_outcome.arrived_police_responses, vec![response_id]);
        assert_eq!(arrival_outcome.decision_requests.len(), 1);
        assert!(arrival_outcome.resolved_operations.is_empty());

        let decision_id = arrival_outcome.decision_requests[0].decision;
        let decision = state
            .decisions()
            .get_decision(decision_id)
            .expect("response decision should persist");
        assert_eq!(decision.requested_at(), response_due);
        assert!(matches!(
            decision.context(),
            DecisionContext::OperationException {
                operation: decision_operation,
                reason: OperationExceptionReason::PoliceArrival(decision_response),
            } if decision_operation == operation && decision_response == response_id
        ));
        assert!(decision.summary().contains("response reached"));
        let operation_record = state
            .operations()
            .get_operation(operation)
            .expect("decision-blocked operation should persist");
        assert_eq!(operation_record.status(), OperationStatus::AwaitingDecision);
        assert_eq!(
            operation_record.awaiting_decision_since(),
            Some(response_due)
        );

        let organization = operation_record.responsible_organization();
        let envelope = build_save(&registry, &state)
            .expect("pending police-arrival decision should survive save validation");
        let bytes =
            bincode::serialize(&envelope).expect("police-arrival decision save should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("police-arrival decision save should deserialize");
        state = restore_save(&registry, decoded)
            .expect("pending police-arrival decision should restore with provenance indexes");
        assert_eq!(
            state
                .decisions()
                .decisions_for_operation(operation)
                .filter(|candidate| candidate.id() == decision_id)
                .count(),
            1
        );
        validate_resolve_decision(
            &registry,
            &state,
            decision_id,
            organization,
            DecisionResponse::Continue,
        )
        .expect("post-entry police response should allow leadership to continue")
        .commit(&mut state)
        .expect("post-entry continue should resume operation");
        let resumed = state
            .operations()
            .get_operation(operation)
            .expect("resumed operation should persist");
        assert_eq!(resumed.status(), OperationStatus::InProgress);
        assert_eq!(resumed.awaiting_decision_since(), None);
        assert_eq!(
            state
                .legal()
                .get_police_response(response_id)
                .and_then(|response| response.arrived_at()),
            Some(response_due)
        );
        let duplicate = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: resumed.leader(),
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::PoliceArrival(response_id),
                },
                attention: AttentionClass::Exception,
                summary: "Police arrival should not be raised twice.".to_owned(),
            },
        )
        .expect_err("one police response must not create duplicate leadership decisions");
        assert_eq!(
            duplicate,
            DecisionError::DuplicatePoliceResponseDecision {
                response: response_id,
                decision: decision_id,
            }
        );
        validate_state(&state).expect("post-entry response decision state should validate");
        validate_state_against_registry(&registry, &state)
            .expect("post-entry response decision should match authored content");
        validate_invariants(&state);
    }

    #[test]
    fn police_arrival_during_another_decision_becomes_deferred_follow_up() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture_with_contingencies(
                true,
                vec![OperationContingency::RequestDecisionOnUnexpectedCondition],
            );
        validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(0).expect("zero patrol presence should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("zero-presence patrol should validate")
        .commit(&mut state)
        .expect("zero-presence patrol should commit");

        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        let response_id = state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.police_response())
            .expect("observable burglary should dispatch response");
        let response_due = state
            .legal()
            .get_police_response(response_id)
            .expect("response should persist")
            .arrival_due_at();
        while state.now() < SimTime::from_minutes(5) {
            run_tick(&registry, &mut state);
        }

        let leader = state
            .operations()
            .get_operation(operation)
            .expect("operation should persist")
            .leader();
        let first = validate_request_decision(
            &state,
            DecisionRequestDraft {
                requester: leader,
                context: DecisionContext::OperationException {
                    operation,
                    reason: OperationExceptionReason::UnexpectedCondition,
                },
                attention: AttentionClass::Exception,
                summary: "A separate execution exception requires leadership direction.".to_owned(),
            },
        )
        .expect("initial exception decision should validate")
        .commit(&mut state)
        .expect("initial exception decision should commit");

        let arrival_outcome = loop {
            let outcome = run_tick(&registry, &mut state);
            if outcome.arrived_police_responses.contains(&response_id) {
                break outcome;
            }
        };
        assert_eq!(arrival_outcome.now, response_due);
        assert!(arrival_outcome.decision_requests.is_empty());
        assert_eq!(
            state.decisions().pending_for_operation(operation),
            Some(first.decision)
        );

        let organization = state
            .operations()
            .get_operation(operation)
            .expect("decision-blocked operation should persist")
            .responsible_organization();
        let resolution = validate_resolve_decision(
            &registry,
            &state,
            first.decision,
            organization,
            DecisionResponse::Continue,
        )
        .expect("continuing the first exception should validate")
        .commit(&mut state)
        .expect("continuing the first exception should atomically create any deferred work");
        let follow_up = resolution
            .decision_request
            .expect("arrived police response should become the next leadership decision");
        assert!(follow_up.requests_pause);
        let follow_up_record = state
            .decisions()
            .get_decision(follow_up.decision)
            .expect("deferred response decision should persist");
        assert!(matches!(
            follow_up_record.context(),
            DecisionContext::OperationException {
                operation: decision_operation,
                reason: OperationExceptionReason::PoliceArrival(decision_response),
            } if decision_operation == operation && decision_response == response_id
        ));
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("follow-up blocked operation should persist")
                .status(),
            OperationStatus::AwaitingDecision
        );

        let final_resolution = validate_resolve_decision(
            &registry,
            &state,
            follow_up.decision,
            organization,
            DecisionResponse::Continue,
        )
        .expect("deferred response decision should be resolvable")
        .commit(&mut state)
        .expect("deferred response continue should resume operation");
        assert!(final_resolution.decision_request.is_none());
        assert_eq!(
            state
                .operations()
                .get_operation(operation)
                .expect("resumed operation should persist")
                .status(),
            OperationStatus::InProgress
        );
        assert_eq!(
            state
                .decisions()
                .decisions_for_operation(operation)
                .filter(|decision| matches!(
                    decision.context(),
                    DecisionContext::OperationException {
                        reason: OperationExceptionReason::PoliceArrival(candidate),
                        ..
                    } if candidate == response_id
                ))
                .count(),
            1
        );
        validate_state(&state).expect("deferred response-decision state should validate");
        validate_state_against_registry(&registry, &state)
            .expect("deferred response decision should match authored content");
        validate_invariants(&state);
    }

    #[test]
    fn arrived_response_penalizes_continuing_operation_and_stales_prearrival_plan() {
        let (registry, mut response_state, _police, _neighborhood, response_operation) =
            make_exposed_business_operation_fixture(true);
        let (_, mut control_state, _control_police, _control_neighborhood, control_operation) =
            make_exposed_business_operation_fixture(false);
        run_tick(&registry, &mut response_state);
        run_tick(&registry, &mut control_state);

        let response_id = response_state
            .operations()
            .get_operation(response_operation)
            .and_then(|record| record.police_response())
            .expect("jurisdictional burglary should dispatch response");
        response_state.advance_clock(SimDuration::from_minutes(45));
        control_state.advance_clock(SimDuration::from_minutes(45));
        let stale_plan = decide_operation_resolution(
            &registry,
            &response_state,
            response_operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due operation should be plannable before response processing");
        assert!(!stale_plan.outcome.factors.police_response_arrived());
        let response_outcome =
            crate::operations::police_response_integration::process_due_police_responses(
                &mut response_state,
            )
            .expect("due response should process");
        assert_eq!(response_outcome.arrived, vec![response_id]);
        assert!(response_outcome.decisions.is_empty());
        let stale_error =
            match validate_operation_resolution_plan(&registry, &response_state, stale_plan) {
                Ok(_) => panic!("response arrival must invalidate a pre-arrival resolution plan"),
                Err(error) => error,
            };
        assert_eq!(
            stale_error,
            OperationResolutionError::StalePoliceResponseContext {
                operation: response_operation,
            }
        );

        let response_plan = decide_operation_resolution(
            &registry,
            &response_state,
            response_operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("arrived-response operation should re-plan");
        let control_plan = decide_operation_resolution(
            &registry,
            &control_state,
            control_operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("unrouted control operation should plan");
        assert!(response_plan.outcome.factors.police_response_arrived());
        assert!(!control_plan.outcome.factors.police_response_arrived());
        let execution = registry.get_operation(OperationKind::Burglary).execution();
        assert_eq!(
            control_plan.outcome.execution_margin - response_plan.outcome.execution_margin,
            i16::from(execution.police_arrival_difficulty_penalty())
        );
        assert_eq!(
            response_plan.outcome.exposure.score - control_plan.outcome.exposure.score,
            i16::from(execution.police_arrival_exposure_penalty())
        );
        validate_operation_resolution_plan(&registry, &response_state, response_plan)
            .expect("response-aware resolution should validate")
            .commit(&mut response_state)
            .expect("response-aware resolution should commit");
        validate_state_against_registry(&registry, &response_state)
            .expect("response-aware completion should validate against registry");
        validate_invariants(&response_state);
    }

    #[test]
    fn police_response_arrival_is_deterministic_across_save_round_trip() {
        let (registry, mut original, _police, _neighborhood, operation) =
            make_exposed_business_operation_fixture(true);
        run_tick(&registry, &mut original);
        let response_id = original
            .operations()
            .get_operation(operation)
            .and_then(|record| record.police_response())
            .expect("jurisdictional burglary should dispatch response");
        let due_at = original
            .legal()
            .get_police_response(response_id)
            .expect("response should persist")
            .arrival_due_at();
        while original.now() + SimDuration::ONE_MINUTE < due_at {
            let outcome = run_tick(&registry, &mut original);
            assert!(outcome.arrived_police_responses.is_empty());
        }
        let envelope = build_save(&registry, &original)
            .expect("pre-arrival police response state should save");
        let bytes = bincode::serialize(&envelope).expect("response save should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("response save should deserialize");
        let mut restored = restore_save(&registry, decoded).expect("response save should restore");

        let original_tick = run_tick(&registry, &mut original);
        let restored_tick = run_tick(&registry, &mut restored);
        assert_eq!(original_tick.arrived_police_responses, vec![response_id]);
        assert_eq!(restored_tick.arrived_police_responses, vec![response_id]);
        assert_eq!(
            original
                .legal()
                .get_police_response(response_id)
                .and_then(|record| record.arrived_at()),
            restored
                .legal()
                .get_police_response(response_id)
                .and_then(|record| record.arrived_at())
        );
        validate_state(&restored).expect("restored police-response state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn resolution_plan_snapshots_patrol_versions_and_uses_explicit_schedule_gaps() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture(true);
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(45));
        let deployment = validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(0).expect("fixture minute should validate"),
                    1_440,
                    Rating::try_new(70).expect("fixture patrol rating should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");
        let stale_plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due operation should resolve against active patrol state");
        assert_eq!(
            stale_plan
                .outcome
                .factors
                .target_police_presence()
                .map(Rating::value),
            Some(70)
        );

        validate_revise_patrol_deployment(
            &state,
            deployment,
            vec![PatrolWindow::try_new(
                DayMinute::try_new(600).expect("fixture minute should validate"),
                120,
                Rating::try_new(80).expect("fixture patrol rating should validate"),
            )
            .expect("fixture patrol window should validate")],
        )
        .expect("patrol revision should validate")
        .commit(&mut state)
        .expect("patrol revision should commit");

        let error = validate_operation_resolution_plan(&registry, &state, stale_plan)
            .err()
            .expect("patrol revision must stale an operation resolution plan");
        assert_eq!(
            error,
            OperationResolutionError::StalePoliceDeploymentContext { operation }
        );

        let fresh_plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("operation should re-plan against revised patrol schedule");
        assert_eq!(
            fresh_plan
                .outcome
                .factors
                .target_police_presence()
                .map(Rating::value),
            Some(0)
        );
        assert_eq!(
            fresh_plan
                .outcome
                .exposure
                .factors
                .target_police_presence()
                .map(Rating::value),
            Some(0)
        );
        validate_operation_resolution_plan(&registry, &state, fresh_plan)
            .expect("fresh patrol-aware resolution plan should validate")
            .commit(&mut state)
            .expect("fresh patrol-aware resolution should commit");
        validate_state(&state).expect("patrol-aware operation resolution should remain valid");
        validate_invariants(&state);
    }

    #[test]
    fn operation_resolution_uses_time_weighted_patrol_presence_across_execution_window() {
        let (registry, mut state, police, neighborhood, operation) =
            make_exposed_business_operation_fixture(true);
        validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![PatrolWindow::try_new(
                    DayMinute::try_new(45).expect("fixture patrol minute should validate"),
                    60,
                    Rating::try_new(90).expect("fixture patrol rating should validate"),
                )
                .expect("fixture patrol window should validate")],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");

        let start = run_tick(&registry, &mut state);
        assert_eq!(start.now, SimTime::from_minutes(1));
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(45));

        let plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due operation should resolve across its whole execution window");
        assert_eq!(
            plan.outcome
                .factors
                .target_police_presence()
                .map(Rating::value),
            Some(2)
        );
        assert_eq!(
            plan.outcome
                .exposure
                .factors
                .target_police_presence()
                .map(Rating::value),
            Some(2)
        );
        assert!(!plan
            .narrative
            .summary
            .contains("High local police presence materially increased execution pressure."));
        validate_operation_resolution_plan(&registry, &state, plan)
            .expect("time-weighted patrol plan should validate")
            .commit(&mut state)
            .expect("time-weighted patrol resolution should commit");
        validate_state(&state).expect("time-weighted patrol state should validate");
        validate_invariants(&state);
    }

    #[test]
    fn property_acquisition_persists_estimated_held_value_with_partial_recovery() {
        let (registry, mut achieved_state, _police, neighborhood, operation) =
            make_exposed_business_operation_fixture(false);
        let start = run_tick(&registry, &mut achieved_state);
        assert_eq!(start.started_operations, vec![operation]);
        achieved_state.advance_clock(SimDuration::from_minutes(45));
        let mut partial_state = achieved_state.clone();

        let achieved_plan = decide_operation_resolution(
            &registry,
            &achieved_state,
            operation,
            OperationResolutionRandomness::new(12, 0),
        )
        .expect("favorable property operation should resolve");
        assert_eq!(
            achieved_plan.outcome.objective_outcome,
            OperationObjectiveOutcome::Achieved
        );
        let achieved_proceeds = achieved_plan
            .outcome
            .property_proceeds
            .expect("achieved property acquisition should create held proceeds");
        assert_eq!(achieved_proceeds.estimated_value().cents(), 56_400);
        assert!(achieved_plan
            .narrative
            .summary
            .contains("estimated held value of 56400 cents"));
        assert!(achieved_plan
            .narrative
            .summary
            .contains("remains unliquidated"));
        validate_operation_resolution_plan(&registry, &achieved_state, achieved_plan)
            .expect("achieved property proceeds should validate")
            .commit(&mut achieved_state)
            .expect("achieved property proceeds should commit");
        assert_eq!(
            achieved_state
                .operations()
                .get_operation(operation)
                .and_then(|record| record.resolution())
                .and_then(|resolution| resolution.property_proceeds())
                .map(|proceeds| proceeds.estimated_value().cents()),
            Some(56_400)
        );
        let organization = achieved_state
            .operations()
            .get_operation(operation)
            .expect("completed property operation should persist")
            .responsible_organization();
        let financial_report = validate_organization_financial_report(
            &achieved_state,
            organization,
            SimTime::ZERO,
            achieved_state.now(),
        )
        .expect("held property should integrate into organization financial reporting")
        .commit(&mut achieved_state);
        let report = achieved_state
            .reports()
            .get_report(financial_report)
            .expect("organization financial report should persist");
        assert!(report.entries()[0].summary.contains(
            "Held operation property at period end: 1 operation(s), estimated value 56400 cents"
        ));
        assert!(report.entries().iter().any(|entry| {
            entry.entities.contains(&EntityRef::Operation(operation))
                && entry
                    .summary
                    .contains("estimated held value of 56400 cents")
        }));

        let (resale_venue, cash_account, settlement_account) = insert_property_disposition_fixture(
            &registry,
            &mut achieved_state,
            neighborhood,
            organization,
        );
        let disposition = validate_dispose_property(
            &registry,
            &achieved_state,
            PropertyDispositionDraft {
                operation,
                venue: resale_venue,
                cash_account,
                settlement_account,
            },
        )
        .expect("held burglary property should be disposable through a resale venue");
        assert_eq!(disposition.realized_value().cents(), 36_660);
        let disposition_outcome = disposition
            .commit(&mut achieved_state)
            .expect("property disposition should commit atomically");
        assert_eq!(disposition_outcome.realized_value.cents(), 36_660);
        assert_eq!(
            achieved_state
                .finance()
                .get_account(cash_account)
                .expect("cash account should persist")
                .balance()
                .cents(),
            36_660
        );
        assert_eq!(
            achieved_state
                .finance()
                .get_account(settlement_account)
                .expect("settlement account should persist")
                .balance()
                .cents(),
            -36_660
        );
        assert!(matches!(
            validate_dispose_property(
                &registry,
                &achieved_state,
                PropertyDispositionDraft {
                    operation,
                    venue: resale_venue,
                    cash_account,
                    settlement_account,
                },
            ),
            Err(PropertyDispositionError::AlreadyDisposed(found)) if found == operation
        ));
        let liquidated_report = validate_organization_financial_report(
            &achieved_state,
            organization,
            SimTime::ZERO,
            achieved_state.now(),
        )
        .expect("liquidated property should integrate into organization financial reporting")
        .commit(&mut achieved_state);
        let liquidated_report = achieved_state
            .reports()
            .get_report(liquidated_report)
            .expect("liquidation financial report should persist");
        assert!(liquidated_report.entries()[0].summary.contains(
            "Held operation property at period end: 0 operation(s), estimated value 0 cents"
        ));
        assert!(liquidated_report.entries()[0]
            .summary
            .contains("Liquidated operation property during period: 1 disposition(s), realized cash 36660 cents"));
        assert!(liquidated_report.entries().iter().any(|entry| {
            entry.entities.contains(&EntityRef::Operation(operation))
                && entry
                    .summary
                    .contains("liquidated through Fixture Pawn Exchange")
                && entry.summary.contains("36660 cents")
        }));
        let restored = restore_save(
            &registry,
            build_save(&registry, &achieved_state).expect("property disposition state should save"),
        )
        .expect("property disposition state should restore");
        let restored_disposition = restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.property_disposition())
            .expect("property disposition should survive save restoration");
        assert_eq!(restored_disposition.realized_value().cents(), 36_660);
        assert_eq!(restored_disposition.venue(), resale_venue);
        validate_state_against_registry(&registry, &restored)
            .expect("restored property disposition should remain registry-valid");
        validate_invariants(&restored);

        let partial_plan = decide_operation_resolution(
            &registry,
            &partial_state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("neutral property operation should resolve");
        assert_eq!(
            partial_plan.outcome.objective_outcome,
            OperationObjectiveOutcome::Partial
        );
        assert_eq!(
            partial_plan
                .outcome
                .property_proceeds
                .expect("partial property acquisition should create reduced held proceeds")
                .estimated_value()
                .cents(),
            22_560
        );
        validate_operation_resolution_plan(&registry, &partial_state, partial_plan)
            .expect("partial property proceeds should validate")
            .commit(&mut partial_state)
            .expect("partial property proceeds should commit");
        validate_state_against_registry(&registry, &achieved_state)
            .expect("achieved property proceeds should remain registry-valid");
        validate_state_against_registry(&registry, &partial_state)
            .expect("partial property proceeds should remain registry-valid");
        validate_invariants(&achieved_state);
        validate_invariants(&partial_state);
    }

    #[test]
    fn property_disposition_reporting_respects_executive_brief_window() {
        let (registry, mut state, _police, neighborhood, operation) =
            make_exposed_business_operation_fixture(false);
        let organization = state
            .operations()
            .get_operation(operation)
            .expect("authorized operation should persist")
            .responsible_organization();
        designate_player_organization(&mut state, organization)
            .expect("test organization should be designatable as player");
        let start = run_tick(&registry, &mut state);
        assert_eq!(start.started_operations, vec![operation]);
        state.advance_clock(SimDuration::from_minutes(45));
        let plan = decide_operation_resolution(
            &registry,
            &state,
            operation,
            OperationResolutionRandomness::new(12, 0),
        )
        .expect("favorable property operation should resolve");
        assert_eq!(
            plan.outcome.objective_outcome,
            OperationObjectiveOutcome::Achieved
        );
        validate_operation_resolution_plan(&registry, &state, plan)
            .expect("property acquisition should validate")
            .commit(&mut state)
            .expect("property acquisition should commit");

        let (venue, cash_account, settlement_account) =
            insert_property_disposition_fixture(&registry, &mut state, neighborhood, organization);
        let mut same_window = state.clone();
        let mut later_window = state;

        validate_dispose_property(
            &registry,
            &same_window,
            PropertyDispositionDraft {
                operation,
                venue,
                cash_account,
                settlement_account,
            },
        )
        .expect("same-window property disposition should validate")
        .commit(&mut same_window)
        .expect("same-window property disposition should commit");
        let delta = 1_439_u64
            .checked_sub(same_window.now().as_minutes())
            .expect("fixture should resolve before first daily brief");
        same_window.advance_clock(SimDuration::from_minutes(
            u32::try_from(delta).expect("first brief delta should fit SimDuration"),
        ));
        let same_window_tick = run_tick(&registry, &mut same_window);
        let same_window_brief = same_window_tick
            .executive_brief
            .expect("first daily brief should be generated");
        let same_window_report = same_window
            .reports()
            .get_report(same_window_brief)
            .expect("same-window executive brief should persist");
        let operation_entries = same_window_report
            .entries()
            .iter()
            .filter(|entry| entry.entities.contains(&EntityRef::Operation(operation)))
            .collect::<Vec<_>>();
        assert_eq!(operation_entries.len(), 1);
        assert!(operation_entries[0]
            .summary
            .contains("it was later liquidated through Fixture Pawn Exchange for 36660 cents"));
        assert!(!same_window_report
            .entries()
            .iter()
            .any(|entry| entry.summary.starts_with("Property from ")));
        assert!(!same_window_report
            .entries()
            .iter()
            .any(|entry| entry.summary.contains("remains unliquidated")));

        let delta = 1_439_u64
            .checked_sub(later_window.now().as_minutes())
            .expect("fixture should resolve before first daily brief");
        later_window.advance_clock(SimDuration::from_minutes(
            u32::try_from(delta).expect("first brief delta should fit SimDuration"),
        ));
        let first_tick = run_tick(&registry, &mut later_window);
        let first_brief = first_tick
            .executive_brief
            .expect("first daily brief should be generated");
        let first_report = later_window
            .reports()
            .get_report(first_brief)
            .expect("first executive brief should persist");
        assert!(first_report
            .entries()
            .iter()
            .any(|entry| entry.summary.contains("remains unliquidated")));

        validate_dispose_property(
            &registry,
            &later_window,
            PropertyDispositionDraft {
                operation,
                venue,
                cash_account,
                settlement_account,
            },
        )
        .expect("later-window property disposition should validate")
        .commit(&mut later_window)
        .expect("later-window property disposition should commit");
        let delta = 2_879_u64
            .checked_sub(later_window.now().as_minutes())
            .expect("disposition should precede the second daily brief");
        later_window.advance_clock(SimDuration::from_minutes(
            u32::try_from(delta).expect("second brief delta should fit SimDuration"),
        ));
        let second_tick = run_tick(&registry, &mut later_window);
        let second_brief = second_tick
            .executive_brief
            .expect("second daily brief should be generated");
        let second_report = later_window
            .reports()
            .get_report(second_brief)
            .expect("second executive brief should persist");
        assert!(second_report.entries().iter().any(|entry| {
            entry.summary.starts_with("Property from ")
                && entry
                    .summary
                    .contains("liquidated through Fixture Pawn Exchange for 36660 cents")
        }));
        assert!(!second_report
            .entries()
            .iter()
            .any(|entry| entry.summary.contains("remains unliquidated")));

        validate_state_against_registry(&registry, &same_window)
            .expect("same-window brief state should remain registry-valid");
        validate_state_against_registry(&registry, &later_window)
            .expect("later-window brief state should remain registry-valid");
        validate_invariants(&same_window);
        validate_invariants(&later_window);
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
            plan.outcome.exposure.level,
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
            plan.outcome.exposure.level,
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
