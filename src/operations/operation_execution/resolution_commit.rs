//! Resolution artifact validation and atomic commit.
//!
//! The parent module owns outcome planning. This module turns a frozen plan into validated
//! cross-domain artifacts, then commits that transaction after all freshness checks succeed.

use super::incident_intake::validate_exposure_incident;
use super::resolution_effects::validate_resolution_effects;
use super::{
    OperationResolutionError, OperationResolutionPlan, completion_history_summary,
    participant_after_action_summary, validate_plan_snapshot,
};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, IdKind, OperationId};
use crate::core::state::AppState;
use crate::core::version::ensure_version_can_advance;
use crate::economy::business_economy_system::ValidatedBusinessDisruption;
use crate::history::history_system::{ValidatedHistoryEvent, validate_record_event};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_system_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::investigation_system::ValidatedIncidentIntake;
use crate::legal::jurisdiction_system::{
    CaseIntakeAuthoritySnapshot, CaseIntakeAuthoritySnapshotError,
    validate_case_intake_authority_snapshot,
};
use crate::operations::surveillance_integration::{
    SurveillanceIntelligencePlan, validate_surveillance_information,
};
use crate::operations::{OperationExposureRecord, OperationObjective, OperationResolutionRecord};
use crate::registry::Registry;
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ValidatedOperationResolution {
    plan: OperationResolutionPlan,
    off_window_patrol_presence_percent: u8,
    incident: Option<ValidatedIncidentIntake>,
    incident_authority: Option<CaseIntakeAuthoritySnapshot>,
    surveillance_information: Vec<ValidatedInformation>,
    information: ValidatedInformation,
    history: ValidatedHistoryEvent,
    report: ValidatedReport,
    detainee_release: Option<crate::legal::arrest_system::ValidatedRelease>,
    witness_intimidation: Vec<crate::legal::witness_system::ValidatedWitnessCooperation>,
    business_disruption: Option<ValidatedBusinessDisruption>,
    participant_information: Vec<(CharacterId, ValidatedInformation)>,
}

impl ValidatedOperationResolution {
    /// Commits the whole resolution atomically. Every fallible effect is validated before the
    /// first mutation and re-checks only its freshness token here.
    pub(crate) fn commit(
        self,
        state: &mut AppState,
    ) -> Result<OperationId, OperationResolutionError> {
        let surveillance_information_count = u32::try_from(self.surveillance_information.len())
            .expect("surveillance information count must fit u32");
        let mut budget = vec![
            (IdKind::Information, 1 + surveillance_information_count),
            (IdKind::HistoryEvent, 1),
            (IdKind::Report, 1),
        ];
        let operation = state
            .operations
            .get_operation(self.plan.snapshot.operation)
            .expect("resolution plan operation must exist");
        let participant_count =
            u32::try_from(operation.participants().len()).expect("participant count must fit u32");
        budget.push((IdKind::Information, participant_count));
        if let Some(incident) = self.incident.as_ref() {
            budget.extend(incident.id_budget()?);
        }
        state.ids.reserve_many(&budget)?;
        validate_plan_snapshot(state, &self.plan, self.off_window_patrol_presence_percent)?;
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
        let discovered_information = self
            .surveillance_information
            .into_iter()
            .map(|information| {
                information
                    .commit(state)
                    .expect("resolution information IDs were preflighted before mutation")
            })
            .collect::<BTreeSet<_>>();
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
        let participant_information = self
            .participant_information
            .into_iter()
            .map(|(participant, information)| {
                (
                    participant,
                    information
                        .commit(state)
                        .expect("participant information IDs were preflighted before resolution"),
                )
            })
            .collect::<BTreeMap<_, _>>();
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
                participant_information,
                surveillance_signatures,
                after_action_information,
                after_action_report,
                history_event,
            },
        );
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
        if let Some(disruption) = self.business_disruption {
            disruption
                .commit(state)
                .expect("preflighted business disruption must remain current during resolution");
        }
        Ok(self.plan.snapshot.operation)
    }
}

pub(crate) fn validate_operation_resolution_plan(
    registry: &Registry,
    state: &AppState,
    plan: OperationResolutionPlan,
) -> Result<ValidatedOperationResolution, OperationResolutionError> {
    let off_window_patrol_presence_percent = registry.legal().off_window_patrol_presence_percent();
    validate_plan_snapshot(state, &plan, off_window_patrol_presence_percent)?;
    let record = state
        .operations
        .get_operation(plan.snapshot.operation)
        .expect("validated resolution operation must exist");
    ensure_version_can_advance(record.version(), "operation")?;
    let effects = validate_resolution_effects(registry, state, record, &plan.outcome)?;
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
    let after_action_summary = plan.narrative.summary.clone();
    let information = validate_record_system_information(
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
            kind: HistoryEventKind::Operation,
            summary: completion_history_summary(record, plan.outcome.objective_outcome),
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
    let participant_information = record
        .participants()
        .into_iter()
        .map(|participant| {
            validate_record_system_information(
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
                    summary: participant_after_action_summary(
                        record,
                        plan.outcome.objective_outcome,
                    ),
                },
            )
            .map(|information| (participant, information))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ValidatedOperationResolution {
        plan,
        off_window_patrol_presence_percent,
        incident,
        incident_authority,
        surveillance_information,
        information,
        history,
        report,
        detainee_release: effects.detainee_release,
        witness_intimidation: effects.witness_intimidation,
        business_disruption: effects.business_disruption,
        participant_information,
    })
}
