//! Deterministic operation resolution planning and atomic persistence of causal outcomes.

mod resolution_factors;

pub(crate) use resolution_factors::{
    MAX_TIME_PRESSURE, has_police_response_arrived_by, resolve_execution_margin,
    resolve_exposure_level, resolve_exposure_score, resolve_intelligence_factors,
    resolve_investigation_target_neighborhoods, resolve_objective_outcome,
    resolve_operation_police_alert_context,
};
use resolution_factors::{
    resolve_exposure_plan, resolve_operation_venue_entities, resolve_role_capability_average,
    resolve_target_police_interval_snapshot, resolve_time_pressure,
};

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, IdExhaustionError, IdKind, NeighborhoodId, OperationId, PoliceResponseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::economy::business_economy_system::{
    BusinessEconomyError, ValidatedBusinessDisruption, validate_disrupt_business_economy,
};
use crate::history::history_system::{HistoryError, ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::investigation_system::{
    InvestigationError, ValidatedIncidentIntake, validate_incident_intake,
};
use crate::legal::jurisdiction_system::{
    CaseIntakeAuthoritySnapshot, CaseIntakeAuthoritySnapshotError,
    resolve_case_intake_authority_snapshot, validate_case_intake_authority_snapshot,
};
use crate::legal::patrol_system::PatrolPresenceSnapshot;
use crate::legal::{
    Admissibility, EvidenceReliability, EvidenceStrength, IncidentEvidenceDraft,
    IncidentIntakeDraft, IncidentWitnessDraft, WitnessCooperation,
};
use crate::operations::operation_economics::{
    CashProceedsPlan, PropertyProceedsPlan, SABOTAGE_DISRUPTION_CLAUSE, depleted_take_clause,
    held_cash_clause, held_property_clause, resolve_cash_proceeds, resolve_property_proceeds,
};
use crate::operations::operation_objective::{
    blocker_clause, effective_objective_outcome, pressureable_witness_targets,
    resolve_objective_blocker,
};
use crate::operations::surveillance_integration::{
    SurveillanceError, SurveillanceIntelligencePlan, decide_surveillance_intelligence,
    surveillance_after_action_clause, validate_surveillance_information,
    validate_surveillance_plan_snapshot,
};
use crate::operations::{
    OperationExposureFactors, OperationExposureLevel, OperationExposureRecord, OperationKind,
    OperationObjective, OperationObjectiveBlocker, OperationObjectiveOutcome, OperationRecord,
    OperationResolutionFactors, OperationResolutionRecord, OperationStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{QualitativeBand, Rating};
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
    #[error("operation {operation} cash-proceeds arithmetic overflowed")]
    CashProceedsOverflow { operation: OperationId },
    #[error("operation {operation} cash-proceeds context changed after resolution planning")]
    StaleCashProceedsContext { operation: OperationId },
    #[error(
        "practical objective context for operation {operation} changed after resolution planning"
    )]
    StaleObjectiveContext { operation: OperationId },
    #[error(
        "extraction custody context for operation {operation} changed after resolution planning"
    )]
    StaleExtractionContext { operation: OperationId },
    #[error("extraction operation {operation} cannot release character {character}: {error}")]
    DetaineeRelease {
        operation: OperationId,
        character: CharacterId,
        #[source]
        error: crate::legal::arrest_system::ArrestError,
    },
    #[error(
        "operation {operation} changed after resolution planning; expected version {expected}, found {found}"
    )]
    StaleOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "operation resolution plan was resolved at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error(
        "police deployment context affecting operation {operation} changed after resolution planning"
    )]
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
    Witness(#[from] crate::legal::witness_system::WitnessError),
    #[error(transparent)]
    Arrest(#[from] crate::legal::arrest_system::ArrestError),
    #[error(transparent)]
    BusinessEconomy(#[from] BusinessEconomyError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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

    pub(crate) fn execution_variance(self) -> i8 {
        self.execution_variance
    }

    pub(crate) fn exposure_variance(self) -> i8 {
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
    objective_blocker: Option<OperationObjectiveBlocker>,
    /// Exact case-witness registrations a tactically viable witness-pressure objective could
    /// still affect when resolution was decided. Freezing the whole set prevents a validated
    /// resolution from silently omitting a newly registered case or applying to a different
    /// cooperation state if legal context changes before commit.
    witness_pressure_targets: Vec<(crate::core::id::CaseWitnessId, WitnessCooperation)>,
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposurePlan,
    property_proceeds_plan: PropertyProceedsPlan,
    cash_proceeds_plan: CashProceedsPlan,
    /// Active custody relationship observed at resolution planning. `None` on an extraction is
    /// meaningful: the target left custody before the crew reached the objective, which forces
    /// the objective to fail instead of making the simulation tick uncommittable.
    extraction_arrest: Option<ArrestId>,
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
    ) = resolve_intelligence_factors(registry, state, operation);
    let started_at = record
        .started_at()
        .expect("in-progress operation must have a start time");
    let police_snapshot = resolve_target_police_interval_snapshot(
        state,
        resolve_operation_venue_entities(state, record),
        started_at,
        state.now(),
    );
    let target_police_presence = police_snapshot.target_presence;
    let police_response_arrived = has_police_response_arrived_by(state, record, state.now());
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
    let execution_margin = resolve_execution_margin(execution, factors);
    let base_objective_outcome = resolve_objective_outcome(execution, execution_margin);
    let extraction_arrest = resolve_extraction_arrest_snapshot(state, record);
    let witness_pressure_targets = if base_objective_outcome != OperationObjectiveOutcome::Failed
        && let (
            OperationKind::WitnessPressure,
            OperationObjective::Frighten {
                target: EntityRef::Character(character),
            },
        ) = (record.kind(), record.objective())
    {
        pressureable_witness_targets(state, record.responsible_organization(), *character)
    } else {
        Vec::new()
    };
    let objective_blocker = if base_objective_outcome == OperationObjectiveOutcome::Failed {
        None
    } else {
        resolve_objective_blocker(state, record)
    };
    let objective_outcome = effective_objective_outcome(base_objective_outcome, objective_blocker);
    let exposure = resolve_exposure_plan(
        registry,
        state,
        operation,
        randomness.exposure_variance(),
        intelligence_quality,
        &police_snapshot,
        police_response_arrived,
    );
    let property_proceeds_plan =
        resolve_property_proceeds(registry, state, record, objective_outcome)?;
    let cash_proceeds_plan = resolve_cash_proceeds(registry, state, record, objective_outcome)?;
    let surveillance = decide_surveillance_intelligence(state, record, objective_outcome)?;
    // Every after-action summary leads with the operation title so executive-brief entries stay
    // identifiable when several operations resolve into the same brief window.
    let mut summary = format!("{}: ", record.title());
    summary.push_str(&build_after_action_summary(
        objective_outcome,
        base_objective_outcome,
        factors,
        exposure.level(),
    ));
    // A depleted haul must narrate even when recent scores left nothing to carry home:
    // silencing the clause would make an Achieved outcome look like an ordinary score.
    let mut depleted_clause_written = false;
    if let Some(proceeds) = property_proceeds_plan.proceeds.as_ref() {
        summary.push(' ');
        summary.push_str(&held_property_clause(proceeds.estimated_value().cents()));
    }
    if property_proceeds_plan.depleted_by_recent_take && !depleted_clause_written {
        summary.push(' ');
        summary.push_str(depleted_take_clause(record.kind()));
        depleted_clause_written = true;
    }
    if let Some(proceeds) = cash_proceeds_plan.proceeds.as_ref() {
        summary.push(' ');
        summary.push_str(&held_cash_clause(proceeds.amount().cents()));
    }
    if cash_proceeds_plan.depleted_by_recent_take && !depleted_clause_written {
        summary.push(' ');
        summary.push_str(depleted_take_clause(record.kind()));
    }
    if let Some(clause) = surveillance_after_action_clause(surveillance.as_ref(), objective_outcome)
    {
        summary.push(' ');
        summary.push_str(&clause);
    }
    if let Some(blocker) = objective_blocker {
        summary.push(' ');
        summary.push_str(blocker_clause(blocker));
    }
    if objective_outcome != OperationObjectiveOutcome::Failed
        && matches!(
            (record.kind(), record.objective()),
            (
                OperationKind::Sabotage | OperationKind::Arson,
                OperationObjective::DisruptBusiness {
                    target: EntityRef::Business(_)
                }
            )
        )
    {
        summary.push(' ');
        summary.push_str(SABOTAGE_DISRUPTION_CLAUSE);
    }
    let mut history_entities = BTreeSet::from([
        EntityRef::Operation(operation),
        EntityRef::Organization(record.responsible_organization()),
        EntityRef::Character(record.leader()),
    ]);
    history_entities.extend(record.objective().referenced_entities());
    history_entities.extend(record.roles().values().copied().map(EntityRef::Character));
    if police_response_arrived {
        let response_id = record
            .police_response()
            .expect("arrived operation police response must remain linked from the operation");
        let response = state
            .legal
            .get_police_response(response_id)
            .expect("operation police-response link must reference a persisted response");
        history_entities.insert(EntityRef::Organization(response.authority()));
        history_entities.insert(EntityRef::Neighborhood(response.neighborhood()));
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
            objective_blocker,
            witness_pressure_targets,
            execution_margin,
            factors,
            exposure,
            property_proceeds_plan,
            cash_proceeds_plan,
            extraction_arrest,
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
    incident_authority: Option<CaseIntakeAuthoritySnapshot>,
    surveillance_information: Vec<ValidatedInformation>,
    legal_activity_information: Option<ValidatedInformation>,
    information: ValidatedInformation,
    history: ValidatedHistoryEvent,
    report: ValidatedReport,
    detainee_release: Option<crate::legal::arrest_system::ValidatedRelease>,
    witness_intimidation: Vec<crate::legal::witness_system::ValidatedWitnessCooperation>,
    business_disruption: Option<ValidatedBusinessDisruption>,
    participant_information: Vec<ValidatedInformation>,
}

impl ValidatedOperationResolution {
    /// Commits the whole resolution atomically. Every fallible effect (custody release,
    /// witness intimidation, sabotage disruption) is validated inside
    /// [`validate_operation_resolution_plan`] and re-checks only its version token at commit;
    /// canonical callers validate and commit within the same tick minute, so no intervening
    /// mutation can invalidate those tokens. A tail-effect failure after the terminal record
    /// would therefore signal caller misuse (holding a validated plan across ticks), not a
    /// reachable pipeline state.
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<OperationId, OperationResolutionError> {
        let incident_evidence_count = self
            .incident
            .as_ref()
            .map(ValidatedIncidentIntake::evidence_count)
            .transpose()?
            .unwrap_or(0);
        let surveillance_information_count = u32::try_from(self.surveillance_information.len())
            .expect("surveillance information count must fit u32");
        let mut budget = vec![
            (
                IdKind::Information,
                1 + u32::from(self.legal_activity_information.is_some())
                    + surveillance_information_count,
            ),
            (IdKind::HistoryEvent, 1),
            (IdKind::Report, 1),
        ];
        // Every participant personally knows how the job they were part of ended; that
        // private knowledge is what an arrested participant can later trade as an informant.
        let operation = state
            .operations
            .get_operation(self.plan.snapshot.operation)
            .expect("resolution plan operation must exist");
        let participant_count =
            u32::try_from(operation.participants().len()).expect("participant count must fit u32");
        budget.push((IdKind::Information, participant_count));
        if let Some(incident) = self.incident.as_ref() {
            budget.push((
                IdKind::Investigation,
                u32::from(incident.requires_new_investigation()),
            ));
            budget.push((IdKind::Evidence, incident_evidence_count));
            budget.push((IdKind::CaseWitness, u32::from(incident.has_witness())));
        }
        state.ids.reserve_many(&budget)?;
        validate_plan_snapshot(state, &self.plan)?;
        let operation = state
            .operations
            .get_operation(self.plan.snapshot.operation)
            .expect("validated resolution operation must still exist");
        ensure_version_can_advance(operation.version(), "operation")?;
        if let Some(snapshot) = self.incident_authority {
            validate_case_intake_authority_snapshot(state, snapshot).map_err(
                |error| match error {
                    CaseIntakeAuthoritySnapshotError::Routing {
                        neighborhood,
                        expected,
                        found,
                    } => OperationResolutionError::StaleIncidentRouting {
                        neighborhood,
                        expected,
                        found,
                    },
                    CaseIntakeAuthoritySnapshotError::JurisdictionVersion {
                        neighborhood,
                        organization,
                        expected_version,
                        found_version,
                    } => OperationResolutionError::StaleIncidentJurisdictionVersion {
                        neighborhood,
                        organization,
                        expected_version,
                        found_version,
                    },
                },
            )?;
        }
        // Tail effects own other domains and can stale independently of the operation record.
        // Re-check all of them before the incident or any artifact can mutate state. Nothing
        // between this preflight and the tail commits mutates arrests, existing active witness
        // cases, or business economies, so successful checks remain current for this commit.
        if let Some(release) = &self.detainee_release {
            let character = match state
                .operations
                .get_operation(self.plan.snapshot.operation)
                .expect("revalidated resolution operation must still exist")
                .objective()
            {
                OperationObjective::FreeDetainee { target } => *target,
                OperationObjective::AcquireProperty { .. }
                | OperationObjective::ObtainCash { .. }
                | OperationObjective::Frighten { .. }
                | OperationObjective::GatherInformation { .. }
                | OperationObjective::DisruptBusiness { .. } => {
                    unreachable!("only extraction resolutions carry a detainee release")
                }
            };
            release.ensure_current(state).map_err(|error| {
                OperationResolutionError::DetaineeRelease {
                    operation: self.plan.snapshot.operation,
                    character,
                    error,
                }
            })?;
        }
        for intimidation in &self.witness_intimidation {
            intimidation.ensure_current(state)?;
        }
        if let Some(disruption) = &self.business_disruption {
            disruption.ensure_current(state)?;
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
        let legal_activity_information = self.legal_activity_information.map(|information| {
            information
                .commit(state)
                .expect("resolution information IDs were preflighted before mutation")
        });
        let discovered_information = self
            .surveillance_information
            .into_iter()
            .map(|information| {
                information
                    .commit(state)
                    .expect("resolution information IDs were preflighted before mutation")
            })
            .collect::<BTreeSet<_>>();
        // The signature set is frozen from the validated plan's observations: what this
        // operation actually saw is authoritative for later validation, not a re-derivation
        // that later notification changes could silently contradict.
        let surveillance_signatures = self
            .plan
            .outcome
            .surveillance
            .as_ref()
            .map(SurveillanceIntelligencePlan::surveillance_signatures)
            .unwrap_or_default();
        let after_action_information = self
            .information
            .commit(state)
            .expect("resolution information IDs were preflighted before mutation");
        let history_event = self
            .history
            .commit(state)
            .expect("resolution history ID was preflighted before mutation");
        let after_action_report = self
            .report
            .commit(state)
            .expect("resolution report ID was preflighted before mutation");
        let extraction_arrest = self.plan.outcome.extraction_arrest;
        state.operations.complete(
            self.plan.snapshot.operation,
            OperationResolutionRecord {
                resolved_at: self.plan.snapshot.resolved_at,
                objective_outcome: self.plan.outcome.objective_outcome,
                objective_blocker: self.plan.outcome.objective_blocker,
                execution_margin: self.plan.outcome.execution_margin,
                factors: self.plan.outcome.factors,
                exposure,
                property_proceeds: self.plan.outcome.property_proceeds_plan.proceeds,
                cash_proceeds: self.plan.outcome.cash_proceeds_plan.proceeds,
                extraction_arrest,
                discovered_information,
                surveillance_signatures,
                legal_activity_information,
                after_action_information,
                after_action_report,
                history_event,
            },
        );
        // Extraction releases run last so custody ownership changes only after the operation
        // itself has reached its terminal record; the validated release was checked against
        // the arrest version seen during plan validation.
        if let Some(release) = self.detainee_release {
            release
                .commit(state)
                .expect("preflighted detainee release must remain current during resolution");
        }
        for intimidation in self.witness_intimidation {
            intimidation
                .commit(state)
                .expect("preflighted witness pressure must remain current during resolution");
        }
        // Sabotage damage runs last so the target's economy degrades only after the
        // operation itself has reached its terminal record.
        if let Some(disruption) = self.business_disruption {
            disruption
                .commit(state)
                .expect("preflighted business disruption must remain current during resolution");
        }
        // Personal after-action knowledge for each participant: the crew knows what went
        // down even though the organization's own record is the org-held after-action.
        // Every draft was validated before the first mutation above.
        for information in self.participant_information {
            information
                .commit(state)
                .expect("participant information IDs were preflighted before resolution mutation");
        }
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
    ensure_version_can_advance(record.version(), "operation")?;
    let expected_property_proceeds =
        resolve_property_proceeds(registry, state, record, plan.outcome.objective_outcome)?;
    if plan.outcome.property_proceeds_plan != expected_property_proceeds {
        return Err(OperationResolutionError::StalePropertyProceedsContext {
            operation: plan.snapshot.operation,
        });
    }
    let expected_cash_proceeds =
        resolve_cash_proceeds(registry, state, record, plan.outcome.objective_outcome)?;
    if plan.outcome.cash_proceeds_plan != expected_cash_proceeds {
        return Err(OperationResolutionError::StaleCashProceedsContext {
            operation: plan.snapshot.operation,
        });
    }
    // Extraction success frees the exact arrest observed in the resolution snapshot. If custody
    // already ended, resolution remains valid but the effective objective outcome is Failed and
    // there is no release effect. This turns a mutable legal dependency into an explicit causal
    // outcome instead of a due-tick panic.
    let detainee_release = match record.objective() {
        crate::operations::OperationObjective::FreeDetainee { target } => {
            match plan.outcome.objective_outcome {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial => {
                    let arrest = plan.outcome.extraction_arrest.ok_or(
                        OperationResolutionError::StaleExtractionContext {
                            operation: plan.snapshot.operation,
                        },
                    )?;
                    let release =
                        crate::legal::arrest_system::validate_release_arrest(state, arrest)
                            .map_err(|error| OperationResolutionError::DetaineeRelease {
                                operation: plan.snapshot.operation,
                                character: *target,
                                error,
                            })?;
                    debug_assert_eq!(release.arrest(), arrest);
                    Some(release)
                }
                OperationObjectiveOutcome::Failed => None,
            }
        }
        crate::operations::OperationObjective::AcquireProperty { .. }
        | crate::operations::OperationObjective::ObtainCash { .. }
        | crate::operations::OperationObjective::Frighten { .. }
        | crate::operations::OperationObjective::GatherInformation { .. }
        | crate::operations::OperationObjective::DisruptBusiness { .. } => None,
    };
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
        plan.outcome.factors.target_police_presence(),
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
    // Witness pressure degrades every registration that can still influence future testimony.
    // Statemented or already-hostile witnesses have no remaining modeled cooperation effect and
    // are therefore objective blockers rather than fake successful intimidation.
    let mut witness_intimidation = Vec::new();
    if plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed
        && let (
            crate::operations::OperationKind::WitnessPressure,
            crate::operations::OperationObjective::Frighten {
                target: EntityRef::Character(_),
            },
        ) = (record.kind(), record.objective())
    {
        for &(case_witness, cooperation) in &plan.outcome.witness_pressure_targets {
            let degraded = match cooperation {
                WitnessCooperation::Cooperative => WitnessCooperation::Reluctant,
                WitnessCooperation::Reluctant => WitnessCooperation::Hostile,
                WitnessCooperation::Hostile => {
                    unreachable!("pressureable witness targets exclude hostile cooperation")
                }
            };
            witness_intimidation.push(
                crate::legal::witness_system::validate_set_witness_cooperation(
                    state,
                    case_witness,
                    degraded,
                )?,
            );
        }
    }
    // Sabotage damage lands through the canonical economy disruption path. A suspended target is
    // converted to a practical objective failure during planning, so every non-failed sabotage
    // reaching this point must have a real disruption effect to commit.
    let mut business_disruption = None;
    if plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed
        && let (
            crate::operations::OperationKind::Sabotage | crate::operations::OperationKind::Arson,
            crate::operations::OperationObjective::DisruptBusiness {
                target: EntityRef::Business(business),
            },
        ) = (record.kind(), record.objective())
    {
        business_disruption = Some(validate_disrupt_business_economy(
            registry, state, *business,
        )?);
    }
    // Personal after-action knowledge for each participant: the crew knows what went down
    // even though the organization's own record is the org-held after-action. Validating
    // here keeps commit free of fallible content checks after terminal mutation.
    let participant_information = record
        .participants()
        .into_iter()
        .map(|participant| {
            validate_record_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Character(participant),
                    source_kind: InformationSourceKind::AfterAction,
                    topic: crate::intelligence::InformationTopic::OperationalOutcome,
                    source_entity: Some(EntityRef::Character(record.leader())),
                    subject: EntityRef::Operation(record.id()),
                    observed_at: plan.snapshot.resolved_at,
                    reliability: Reliability::DirectAccess,
                    specificity: Specificity::Precise,
                    summary: format!(
                        "You took part in {}, which ended with objective {}.",
                        record.title(),
                        outcome_label(plan.outcome.objective_outcome)
                    ),
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ValidatedOperationResolution {
        plan,
        incident,
        incident_authority,
        surveillance_information,
        legal_activity_information,
        information,
        history,
        report,
        detainee_release,
        witness_intimidation,
        business_disruption,
        participant_information,
    })
}

/// Renders the canonical legal-activity summary text. One template source: the commit path
/// builds its persisted copy from this writer and the per-tick invariant pass re-renders the
/// text into a reused buffer, so the two can never drift while staying allocation-free on
/// the validation side.
pub(crate) fn write_legal_activity_summary(
    out: &mut impl std::fmt::Write,
    operation_title: &str,
    authority_name: &str,
) -> std::fmt::Result {
    write!(
        out,
        "The exposure from {operation_title} produced a police investigation opened by {authority_name}. \
         The organization does not know the case's evidence, lead, or detective work."
    )
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
    let mut summary = String::new();
    write_legal_activity_summary(&mut summary, operation.title(), authority_name)
        .expect("String buffer writes are infallible");
    summary
}

fn resolve_incident_witness(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    exposure: &OperationExposurePlan,
    target_police_presence: Option<Rating>,
) -> Option<IncidentWitnessDraft> {
    if !matches!(
        exposure.level,
        OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
    ) {
        return None;
    }
    // Only business targets have an identifiable on-scene witness today: the owner.
    let target = match operation.objective() {
        OperationObjective::AcquireProperty { target }
        | OperationObjective::ObtainCash { target }
        | OperationObjective::DisruptBusiness { target } => Some(*target),
        OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::FreeDetainee { .. } => None,
    }?;
    let EntityRef::Business(business) = target else {
        return None;
    };
    let record = state.world.get_business(business)?;
    let crate::world::BusinessOwner::Character(character) = record.owner() else {
        return None;
    };
    let witness = state.world.get_character(character)?;
    // The identified participant cannot witness their own crime, and an organization's own
    // member is not treated as the case's named witness against it.
    if Some(character) == exposure.identified_character
        || witness.organization() == Some(operation.responsible_organization())
    {
        return None;
    }
    // Patrol presence shapes whether a witness is willing to stand behind an account (§31).
    let cooperation = match target_police_presence.map(Rating::value) {
        Some(presence) if presence >= 60 => WitnessCooperation::Cooperative,
        Some(presence) if presence >= 30 => WitnessCooperation::Reluctant,
        _ => WitnessCooperation::Hostile,
    };
    Some(IncidentWitnessDraft {
        character,
        cooperation,
    })
}

fn validate_exposure_incident(
    registry: &Registry,
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    exposure: &OperationExposurePlan,
    target_police_presence: Option<Rating>,
    discovered_at: SimTime,
) -> Result<
    (
        Option<ValidatedIncidentIntake>,
        Option<CaseIntakeAuthoritySnapshot>,
    ),
    OperationResolutionError,
> {
    if exposure.level == OperationExposureLevel::None {
        return Ok((None, None));
    }
    let Some(neighborhood) = exposure.neighborhood else {
        return Ok((None, None));
    };
    let authority_snapshot = resolve_case_intake_authority_snapshot(state, neighborhood);
    let Some(owner) = authority_snapshot.organization else {
        return Ok((None, Some(authority_snapshot)));
    };
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
    // A witnessed or identifying exposure leaves a named witness when the target is a
    // character-owned business: the owner saw it happen. Members of the responsible
    // organization and the identified participant never count as the case's witness.
    let witness = resolve_incident_witness(state, operation, exposure, target_police_presence);
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
            origin: Some(EntityRef::Operation(operation.id())),
            notified_organizations: BTreeSet::from([operation.responsible_organization()]),
            witness,
        },
    )?;
    Ok((Some(incident), Some(authority_snapshot)))
}

pub(crate) fn find_due_in_progress_operations(state: &AppState) -> Vec<OperationId> {
    state.operations.find_due_in_progress(state.now())
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
        resolve_operation_venue_entities(state, record),
        record
            .started_at()
            .expect("in-progress operation must have a start time"),
        plan.snapshot.resolved_at,
    );
    // The real staleness signal is the recomputed snapshots: patrol deployments or the police
    // response may have changed since planning. The plan's outcome factors were derived from the
    // plan snapshot itself, so no re-derivation is needed (and comparing them would be tautological).
    if current_police_snapshot != plan.snapshot.police_snapshot {
        return Err(OperationResolutionError::StalePoliceDeploymentContext {
            operation: plan.snapshot.operation,
        });
    }
    let practical_context_matters = plan.outcome.objective_blocker.is_some()
        || plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed;
    let current_witness_pressure_targets = if practical_context_matters
        && let (
            OperationKind::WitnessPressure,
            OperationObjective::Frighten {
                target: EntityRef::Character(character),
            },
        ) = (record.kind(), record.objective())
    {
        pressureable_witness_targets(state, record.responsible_organization(), *character)
    } else {
        Vec::new()
    };
    if current_witness_pressure_targets != plan.outcome.witness_pressure_targets {
        return Err(OperationResolutionError::StaleObjectiveContext {
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
        || has_police_response_arrived_by(state, record, plan.snapshot.resolved_at)
            != plan.outcome.factors.police_response_arrived()
    {
        return Err(OperationResolutionError::StalePoliceResponseContext {
            operation: plan.snapshot.operation,
        });
    }
    if resolve_extraction_arrest_snapshot(state, record) != plan.outcome.extraction_arrest {
        return Err(OperationResolutionError::StaleExtractionContext {
            operation: plan.snapshot.operation,
        });
    }
    // Tactical failures do not depend on practical availability because no objective effect is
    // committed. For every tactically viable plan, the blocker is part of the validated snapshot
    // and must remain exactly the same through commit.
    if (plan.outcome.objective_blocker.is_some()
        || plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed)
        && resolve_objective_blocker(state, record) != plan.outcome.objective_blocker
    {
        return Err(OperationResolutionError::StaleObjectiveContext {
            operation: plan.snapshot.operation,
        });
    }
    if let Some(surveillance) = &plan.outcome.surveillance {
        validate_surveillance_plan_snapshot(state, surveillance)?;
    }
    Ok(())
}

fn resolve_extraction_arrest_snapshot(
    state: &AppState,
    record: &OperationRecord,
) -> Option<ArrestId> {
    match record.objective() {
        OperationObjective::FreeDetainee { target } => {
            let arrest_id = record.extraction_arrest()?;
            state
                .legal
                .get_arrest(arrest_id)
                .filter(|arrest| {
                    arrest.character() == *target
                        && arrest.status() == crate::legal::ArrestStatus::Detained
                })
                .map(|_| arrest_id)
        }
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => None,
    }
}

/// Composes the after-action narrative from the resolution factors. The report leads with the
/// outcome and the factors that actually moved it. Neutral lines (normal execution window, no
/// exposure, negligible police presence) and strong-but-expected crew quality on a clean job are
/// omitted rather than recited, so attention goes to what deviates from a routine job: weak
/// capability bands, tactically degraded outcomes that deserve explanation, adverse pressure, and
/// thin planning intelligence. Practical blockers may make the effective objective fail even after
/// tactical success, so execution commentary follows the tactical outcome while the headline keeps
/// the effective objective result. Luck commentary is kept only when it explains tactical loss.
fn build_after_action_summary(
    outcome: OperationObjectiveOutcome,
    tactical_outcome: OperationObjectiveOutcome,
    factors: OperationResolutionFactors,
    exposure: OperationExposureLevel,
) -> String {
    let mut parts = vec![format!("Objective {}.", outcome_label(outcome))];
    // Practical blockers can turn a tactically successful execution into an objective failure.
    // Crew-quality and luck commentary therefore keys off the tactical result rather than
    // falsely implying that strong execution caused a target-availability failure.
    if tactical_outcome != OperationObjectiveOutcome::Achieved
        || matches!(
            factors.role_capability_average().qualitative_band(),
            QualitativeBand::Poor | QualitativeBand::Competent
        )
    {
        parts.push(format!(
            "Assigned-role competence was {}.",
            band_label(factors.role_capability_average().qualitative_band())
        ));
    }
    match factors.leader_capability() {
        Some(rating)
            if tactical_outcome != OperationObjectiveOutcome::Achieved
                || matches!(
                    rating.qualitative_band(),
                    QualitativeBand::Poor | QualitativeBand::Competent
                ) =>
        {
            parts.push(format!(
                "Leadership coordination was {}.",
                band_label(rating.qualitative_band())
            ));
        }
        Some(_) => {}
        None => {
            parts.push("Leadership had no demonstrated capability for the execution.".to_owned())
        }
    }
    // Police pressure is reported when it materially shaped the job or when the organization
    // could not establish it at all; light presence was not worth the crew's attention.
    match (
        factors.target_police_presence(),
        factors.police_response_arrived(),
    ) {
        (presence, true) => {
            if presence.is_some_and(|rating| rating.value() >= 65) {
                parts.push(
                    "High local police presence materially increased execution pressure."
                        .to_owned(),
                );
            }
            parts.push(
                "Law-enforcement response reached the target before the operation ended."
                    .to_owned(),
            );
        }
        (Some(rating), false) if rating.value() >= 65 => parts
            .push("High local police presence materially increased execution pressure.".to_owned()),
        (None, false) => parts.push(
            "No location-based police pressure could be established from the operation target."
                .to_owned(),
        ),
        (Some(_), false) => {}
    }
    if factors.intelligence_topics_covered() > 0 {
        let covered = factors.intelligence_topics_covered();
        let relevant = factors.intelligence_topics_relevant();
        let coverage = if covered == relevant {
            format!("Planning intelligence covered all {relevant} relevant areas")
        } else {
            format!("Planning intelligence covered {covered} of {relevant} relevant areas")
        };
        // Thin coverage is actionable uncertainty the boss should see, not reassurance.
        let confidence = if covered * 2 >= relevant {
            "; the available reports reduced execution uncertainty."
        } else {
            "; large gaps remained in the plan's information."
        };
        parts.push(format!("{coverage}{confidence}"));
    }
    // A chosen approach that reduced difficulty is the expected case, not news; only an
    // approach that hurt execution earns a sentence.
    if factors.approach_adjustment() > 0 {
        parts.push("The selected approach increased execution difficulty.".to_owned());
    }
    if factors.time_pressure() > 0 {
        parts.push("The completion deadline compressed the execution window.".to_owned());
    }
    if tactical_outcome != OperationObjectiveOutcome::Achieved {
        match factors.variance() {
            value if value < 0 => parts.push(match tactical_outcome {
                OperationObjectiveOutcome::Partial => {
                    "Adverse unplanned circumstances reduced the result.".to_owned()
                }
                OperationObjectiveOutcome::Failed => {
                    "Adverse unplanned circumstances contributed to the failure.".to_owned()
                }
                OperationObjectiveOutcome::Achieved => unreachable!("excluded above"),
            }),
            0 => {}
            _ => parts.push("Favorable unplanned circumstances improved the result.".to_owned()),
        }
    }
    match exposure {
        OperationExposureLevel::None => {}
        OperationExposureLevel::Trace => {
            parts.push("The crew observed limited trace exposure.".to_owned())
        }
        OperationExposureLevel::Witnessed => parts.push(
            "The operation appears to have been witnessed or otherwise clearly observed."
                .to_owned(),
        ),
        OperationExposureLevel::Identifying => parts.push(
            "The crew believes at least one participant may have been identifiable.".to_owned(),
        ),
    }
    parts.join(" ")
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
mod tests;
