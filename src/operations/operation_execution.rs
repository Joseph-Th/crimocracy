//! Deterministic operation resolution planning and atomic persistence of causal outcomes.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, IdExhaustionError, IdKind, NeighborhoodId, OperationId, PoliceResponseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::economy::business_economy_system::{
    validate_disrupt_business_economy, BusinessEconomyError, ValidatedBusinessDisruption,
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
    IncidentIntakeDraft, IncidentWitnessDraft, WitnessCooperation,
};
use crate::operations::operation_economics::{
    resolve_cash_proceeds, resolve_property_proceeds, undeposited_cash_clause,
    unliquidated_property_clause, CashProceedsPlan, PropertyProceedsPlan, DEPLETED_TAKE_CLAUSE,
    SABOTAGE_DISRUPTION_CLAUSE,
};
use crate::operations::surveillance_integration::{
    decide_surveillance_intelligence, surveillance_after_action_clause,
    validate_surveillance_information, validate_surveillance_plan_snapshot, SurveillanceError,
    SurveillanceIntelligencePlan,
};
use crate::operations::{
    OperationExposureFactors, OperationExposureLevel, OperationExposureRecord, OperationKind,
    OperationObjective, OperationObjectiveOutcome, OperationRecord, OperationResolutionFactors,
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
    #[error("operation {operation} cash-proceeds arithmetic overflowed")]
    CashProceedsOverflow { operation: OperationId },
    #[error("operation {operation} cash-proceeds context changed after resolution planning")]
    StaleCashProceedsContext { operation: OperationId },
    #[error("sabotage target economy for operation {operation} changed after resolution planning")]
    StaleSabotageContext { operation: OperationId },
    #[error(
        "extraction operation {operation} targets character {character}, who is not in custody"
    )]
    MissingDetaineeArrest {
        operation: OperationId,
        character: CharacterId,
    },
    #[error("extraction operation {operation} cannot release character {character}: {error}")]
    DetaineeRelease {
        operation: OperationId,
        character: CharacterId,
        error: String,
    },
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
    Witness(#[from] crate::legal::witness_system::WitnessError),
    #[error(transparent)]
    Arrest(#[from] crate::legal::arrest_system::ArrestError),
    #[error(transparent)]
    BusinessEconomy(#[from] BusinessEconomyError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposurePlan,
    property_proceeds_plan: PropertyProceedsPlan,
    cash_proceeds_plan: CashProceedsPlan,
    surveillance: Option<SurveillanceIntelligencePlan>,
    /// Whether a sabotage objective faces an operating economy to damage. Decided once here
    /// so the after-action narrative and the validated disruption effect cannot disagree.
    targets_operating_economy: bool,
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
    let objective_outcome = resolve_objective_outcome(execution, execution_margin);
    // The sabotage narrative must describe only disruption that will actually be committed:
    // a target whose economy went suspended between authorization and now has nothing
    // operating to damage, so both the summary clause and the validated effect key off this
    // one decision.
    let sabotage_target = match (record.kind(), record.objective()) {
        (
            OperationKind::Sabotage,
            OperationObjective::DisruptBusiness {
                target: EntityRef::Business(business),
            },
        ) => Some(*business),
        _ => None,
    };
    let targets_operating_economy = sabotage_target.is_some_and(|business| {
        state
            .economy
            .get_business_economy(business)
            .is_some_and(|economy| {
                economy.status() == crate::economy::BusinessOperatingStatus::Active
            })
    });
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
        factors,
        exposure.level(),
    ));
    // A depleted haul must narrate even when recent scores left nothing to carry home:
    // silencing the clause would make an Achieved outcome look like an ordinary score.
    let mut depleted_clause_written = false;
    if let Some(proceeds) = property_proceeds_plan.proceeds.as_ref() {
        summary.push(' ');
        summary.push_str(&unliquidated_property_clause(
            proceeds.estimated_value().cents(),
        ));
    }
    if property_proceeds_plan.depleted_by_recent_take && !depleted_clause_written {
        summary.push(' ');
        summary.push_str(DEPLETED_TAKE_CLAUSE);
        depleted_clause_written = true;
    }
    if let Some(proceeds) = cash_proceeds_plan.proceeds.as_ref() {
        summary.push(' ');
        summary.push_str(&undeposited_cash_clause(proceeds.amount().cents()));
    }
    if cash_proceeds_plan.depleted_by_recent_take && !depleted_clause_written {
        summary.push(' ');
        summary.push_str(DEPLETED_TAKE_CLAUSE);
    }
    if let Some(clause) = surveillance_after_action_clause(surveillance.as_ref(), objective_outcome)
    {
        summary.push(' ');
        summary.push_str(&clause);
    }
    if objective_outcome != OperationObjectiveOutcome::Failed
        && targets_operating_economy
        && matches!(
            (record.kind(), record.objective()),
            (
                OperationKind::Sabotage,
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
            property_proceeds_plan,
            cash_proceeds_plan,
            surveillance,
            targets_operating_economy,
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
    detainee_release: Option<crate::legal::arrest_system::ValidatedRelease>,
    witness_intimidation: Vec<crate::legal::witness_system::ValidatedWitnessCooperation>,
    business_disruption: Option<ValidatedBusinessDisruption>,
    participant_information: Vec<ValidatedInformation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IncidentAuthoritySnapshot {
    neighborhood: NeighborhoodId,
    organization: Option<crate::core::id::OrganizationId>,
    jurisdiction_version: Option<u32>,
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
            budget.push((IdKind::Investigation, 1));
            budget.push((IdKind::Evidence, incident_evidence_count));
            budget.push((IdKind::CaseWitness, u32::from(incident.has_witness())));
        }
        state.ids.reserve_many(&budget)?;
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
        let legal_activity_information = match self.legal_activity_information {
            Some(information) => Some(information.commit(state)?),
            None => None,
        };
        let discovered_information = self
            .surveillance_information
            .into_iter()
            .map(|information| information.commit(state))
            .collect::<Result<BTreeSet<_>, _>>()?;
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
        let after_action_information = self.information.commit(state)?;
        let history_event = self.history.commit(state)?;
        let after_action_report = self.report.commit(state)?;
        state.operations.complete(
            self.plan.snapshot.operation,
            OperationResolutionRecord {
                resolved_at: self.plan.snapshot.resolved_at,
                objective_outcome: self.plan.outcome.objective_outcome,
                execution_margin: self.plan.outcome.execution_margin,
                factors: self.plan.outcome.factors,
                exposure,
                property_proceeds: self.plan.outcome.property_proceeds_plan.proceeds,
                cash_proceeds: self.plan.outcome.cash_proceeds_plan.proceeds,
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
            release.commit(state)?;
        }
        for intimidation in self.witness_intimidation {
            intimidation.commit(state)?;
        }
        // Sabotage damage runs last so the target's economy degrades only after the
        // operation itself has reached its terminal record.
        if let Some(disruption) = self.business_disruption {
            disruption.commit(state)?;
        }
        // Personal after-action knowledge for each participant: the crew knows what went
        // down even though the organization's own record is the org-held after-action.
        // Every draft was validated before the first mutation above.
        for information in self.participant_information {
            information.commit(state)?;
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
    // Extraction success frees the target through the canonical arrest-release path; the
    // release is validated here so commit re-checks only staleness. The nested match keeps
    // every objective and outcome variant explicit, so a new objective can never silently
    // skip the extraction-release effect.
    let detainee_release = match record.objective() {
        crate::operations::OperationObjective::FreeDetainee { target } => {
            match plan.outcome.objective_outcome {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial => {
                    let arrest = state.legal.active_arrest_for_character(*target).ok_or(
                        OperationResolutionError::MissingDetaineeArrest {
                            operation: plan.snapshot.operation,
                            character: *target,
                        },
                    )?;
                    Some(
                        crate::legal::arrest_system::validate_release_arrest(state, arrest.id())
                            .map_err(|error| OperationResolutionError::DetaineeRelease {
                                operation: plan.snapshot.operation,
                                character: *target,
                                error: error.to_string(),
                            })?,
                    )
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
    // Witness pressure degrades the target's cooperation on every active case where they
    // are the named witness and the case is run by another authority — the same contract
    // authorization enforces. Each degradation is validated here so commit re-checks only
    // staleness.
    let mut witness_intimidation = Vec::new();
    if plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed {
        if let (
            crate::operations::OperationKind::WitnessPressure,
            crate::operations::OperationObjective::Frighten {
                target: EntityRef::Character(character),
            },
        ) = (record.kind(), record.objective())
        {
            let responsible_organization = record.responsible_organization();
            // The by-character witness index scopes this to the target's own registrations;
            // scanning every witness ever registered would grow with campaign length.
            let targets: Vec<_> = state
                .legal
                .case_witnesses_for_character(*character)
                .filter(|witness| witness.witness() == *character)
                .filter(|witness| {
                    state
                        .legal
                        .get_investigation(witness.investigation())
                        .is_some_and(|investigation| {
                            investigation.status() == crate::legal::InvestigationStatus::Active
                                && investigation.owner() != responsible_organization
                        })
                })
                .map(|witness| (witness.id(), witness.cooperation()))
                .collect();
            for (case_witness, cooperation) in targets {
                let degraded = match cooperation {
                    WitnessCooperation::Cooperative => WitnessCooperation::Reluctant,
                    WitnessCooperation::Reluctant => WitnessCooperation::Hostile,
                    WitnessCooperation::Hostile => continue,
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
    }
    // Sabotage damage lands through the canonical economy disruption path; the disruption is
    // validated here so commit re-checks only staleness. A target whose economy went
    // suspended (or disappeared) between authorization and resolution has nothing operating
    // to disrupt: the resolution proceeds without a damage effect rather than failing the
    // whole validated settlement — a modeled degenerate outcome, never an aborted tick. The
    // plan's own decision is re-derived so the after-action narrative can never claim
    // disruption the committed state will not carry.
    let mut business_disruption = None;
    if plan.outcome.objective_outcome != OperationObjectiveOutcome::Failed {
        if let (
            crate::operations::OperationKind::Sabotage,
            crate::operations::OperationObjective::DisruptBusiness {
                target: EntityRef::Business(business),
            },
        ) = (record.kind(), record.objective())
        {
            let economy_active =
                state
                    .economy
                    .get_business_economy(*business)
                    .is_some_and(|economy| {
                        economy.status() == crate::economy::BusinessOperatingStatus::Active
                    });
            if economy_active != plan.outcome.targets_operating_economy {
                return Err(OperationResolutionError::StaleSabotageContext {
                    operation: plan.snapshot.operation,
                });
            }
            if economy_active {
                business_disruption = Some(validate_disrupt_business_economy(
                    registry, state, *business,
                )?);
            }
        }
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
    Ok((
        Some(incident),
        Some(IncidentAuthoritySnapshot {
            neighborhood,
            organization: Some(owner),
            jurisdiction_version: Some(jurisdiction_version),
        }),
    ))
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
    resolve_target_police_snapshot_from(state, entities, |state, neighborhood| {
        resolve_patrol_presence_snapshot(state, neighborhood, at)
    })
}

/// Venue entities for police-presence and exposure attribution. Extraction is the one
/// objective whose referenced character stands for a place the crew acts while that person
/// is elsewhere: custody. The venue proxies to the detaining authority's footprint instead
/// of the detainee's organization assets, so an organization's own legitimate businesses
/// never raise its own extraction difficulty or host its exposure incident.
fn resolve_operation_venue_entities(state: &AppState, record: &OperationRecord) -> Vec<EntityRef> {
    match record.objective() {
        OperationObjective::FreeDetainee { target } => state
            .legal
            .active_arrest_for_character(*target)
            .and_then(|arrest| state.legal.get_investigation(arrest.investigation()))
            .map(|investigation| vec![EntityRef::Organization(investigation.owner())])
            .unwrap_or_else(|| vec![EntityRef::Character(*target)]),
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::DisruptBusiness { .. } => record.objective().referenced_entities(),
    }
}

fn resolve_target_police_interval_snapshot(
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
fn resolve_target_neighborhoods(
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
                if let Some(character) = state.world.get_character(id) {
                    if let Some(org) = character.organization() {
                        for business in state.world.businesses_owned_by_organization(org) {
                            neighborhoods.insert(business.neighborhood());
                        }
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
/// operation-originated cases, the originating operation's objective entities. Read-only
/// derivation used by district-scoped consumers such as enterprise heat surcharges.
pub(crate) fn resolve_investigation_target_neighborhoods(
    state: &AppState,
    investigation: &crate::legal::InvestigationRecord,
) -> BTreeSet<NeighborhoodId> {
    let mut entities: Vec<EntityRef> = investigation.subjects().iter().copied().collect();
    // Operation-originated cases also target their objective's entities. Enterprise-originated
    // cases already carry the racket as a subject, whose location maps to a neighborhood below.
    if let Some(EntityRef::Operation(origin)) = investigation.origin() {
        if let Some(operation) = state.operations.get_operation(origin) {
            entities.extend(operation.objective().referenced_entities());
        }
    }
    resolve_target_neighborhoods(state, entities)
}

fn resolve_exposure_plan(
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

/// Maximum normalized time pressure; the producer clamps to this bound and the persisted-state
/// validator rejects factors above it, so both must reference one shared constant.
pub(crate) const MAX_TIME_PRESSURE: u8 = 30;

fn resolve_time_pressure(started_at: SimTime, due_at: SimTime, base_duration: u32) -> u8 {
    let available = due_at.as_minutes().saturating_sub(started_at.as_minutes());
    let base = u64::from(base_duration);
    if available >= base {
        return 0;
    }
    let shortfall = base - available;
    let pressure = shortfall
        .saturating_mul(u64::from(MAX_TIME_PRESSURE))
        .div_ceil(base);
    u8::try_from(pressure.min(u64::from(MAX_TIME_PRESSURE)))
        .expect("bounded time pressure must fit u8")
}

fn weighted_ability(role_average: Rating, leader_capability: Option<Rating>) -> i16 {
    let role = i16::from(role_average.value());
    // A leader without the authored leadership capability contributes nothing to coordination
    // rather than being silently averaged up to the roster's role skill: the after-action summary
    // reports "no demonstrated capability", and the arithmetic should match.
    let leadership = leader_capability
        .map(|rating| i16::from(rating.value()))
        .unwrap_or(0);
    (role * 3 + leadership) / 4
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
        let score = information_score(information, planning_at, max_age);
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
    // Known-unreliable information scores worst (actively misleading); unverified/unknown
    // reliability scores slightly better than known-unreliable. This matches the authored
    // recruitment information-quality table, so the same Reliability enum cannot contribute
    // contradictory values in different subsystems.
    match reliability {
        Reliability::Unknown => 20,
        Reliability::Unreliable => 10,
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

pub(crate) fn resolve_execution_margin(
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

/// Composes the after-action narrative from the resolution factors. The report leads with the
/// outcome and the factors that actually moved it. Neutral lines (normal execution window, no
/// exposure, negligible police presence) and strong-but-expected crew quality on a clean job are
/// omitted rather than recited, so attention goes to what deviates from a routine job: weak
/// capability bands, non-achieved outcomes that deserve explanation, adverse pressure, and thin
/// planning intelligence. Luck commentary is kept only when it explains a degraded result; on an
/// achieved job the variance already shows in the outcome, so reciting it would be noise.
fn build_after_action_summary(
    outcome: OperationObjectiveOutcome,
    factors: OperationResolutionFactors,
    exposure: OperationExposureLevel,
) -> String {
    let mut parts = vec![format!("Objective {}.", outcome_label(outcome))];
    // Crew quality is worth a sentence only when it explains the result: a weak band is a risk
    // factor, and a partial or failed job should say what the crew brought to it.
    if outcome != OperationObjectiveOutcome::Achieved
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
            if outcome != OperationObjectiveOutcome::Achieved
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
    if outcome != OperationObjectiveOutcome::Achieved {
        match factors.variance() {
            value if value < 0 => parts.push(match outcome {
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
