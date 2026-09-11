//! Surveillance operation integration that turns observed world state into bounded organization knowledge.

use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, CharacterId, EnterpriseId, InvestigationId, NeighborhoodId, OperationId,
    OrganizationId, PatrolDeploymentId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES_U16, SimTime};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_information, validate_record_information_with_signal,
};
use crate::intelligence::{
    CaseActivitySignal, InformationDraft, InformationRecord, InformationSignal,
    InformationSourceKind, InformationTopic, KnowledgeHolder, PatrolIntervalSignal, Reliability,
    Specificity,
};
use crate::legal::{InvestigationStatus, PatrolWindow};
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationRecord, OperationStatus,
};
use crate::registry::Registry;
use crate::world::{BusinessFunction, OrganizationKind, Rating};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub(crate) enum SurveillanceError {
    #[error("surveillance operations require a gather-information objective")]
    InvalidObjective,
    #[error("entity {0:?} cannot be directly observed by surveillance")]
    UnsupportedTarget(EntityRef),
    #[error("surveillance target {0:?} no longer exists")]
    MissingTarget(EntityRef),
    #[error("surveillance target {0:?} changed after resolution planning")]
    StaleTarget(EntityRef),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SurveillanceIntelligencePlan {
    target: EntityRef,
    observed_at: SimTime,
    surveiller: OrganizationId,
    snapshot: SurveillanceTargetSnapshot,
    observations: Vec<SurveillanceObservation>,
}

impl SurveillanceIntelligencePlan {
    pub(crate) fn observation_count(&self) -> usize {
        self.observations.len()
    }

    /// The topic/subject/semantic triples this plan will persist, frozen on the operation's
    /// resolution so later world changes cannot rewrite what the surveillance actually learned.
    pub(crate) fn surveillance_signatures(
        &self,
    ) -> BTreeSet<(InformationTopic, EntityRef, Option<InformationSignal>)> {
        self.observations
            .iter()
            .map(|observation| {
                (
                    observation.topic,
                    observation.subject,
                    observation.signal.clone(),
                )
            })
            .collect()
    }

    /// Compact phrases naming what each observation covers, in stable observation order.
    pub(crate) fn observation_findings(&self) -> impl Iterator<Item = &str> {
        self.observations
            .iter()
            .map(|observation| observation.finding.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SurveillanceObservation {
    topic: InformationTopic,
    subject: EntityRef,
    reliability: Reliability,
    specificity: Specificity,
    signal: Option<InformationSignal>,
    summary: String,
    /// Compact player-facing phrase naming what this observation covers, quoted by the
    /// operation's after-action clause so the report says what was learned without forcing a
    /// drill-down into each information record.
    finding: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SurveillanceTargetSnapshot {
    Neighborhood {
        id: NeighborhoodId,
        name: String,
        patrol: PatrolPatternSnapshot,
    },
    Business {
        id: BusinessId,
        name: String,
        functions: BTreeSet<BusinessFunction>,
        neighborhood: NeighborhoodId,
        neighborhood_name: String,
        patrol: PatrolPatternSnapshot,
    },
    Character {
        id: CharacterId,
        name: String,
        organization: Option<(OrganizationId, String)>,
        supervisor: Option<(CharacterId, String)>,
    },
    Organization {
        id: OrganizationId,
        name: String,
        active_members: Vec<(CharacterId, String)>,
        // Present for law-enforcement/legal-authority targets when surveillance can tie visible
        // authority activity to a case originated by the surveiller's own prior activity.
        law_enforcement_sightline: Option<CaseActivitySignal>,
    },
    Investigation {
        id: InvestigationId,
        title: String,
        owner: OrganizationId,
        owner_name: String,
        status: InvestigationStatus,
        lead: Option<(CharacterId, String)>,
    },
    Enterprise {
        id: EnterpriseId,
        organization: OrganizationId,
        organization_name: String,
        manager: CharacterId,
        manager_name: String,
        location: EnterpriseLocation,
        location_name: String,
        status: EnterpriseStatus,
    },
    Operation {
        id: OperationId,
        organization: OrganizationId,
        organization_name: String,
        status: OperationStatus,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PatrolPatternSnapshot {
    neighborhood: NeighborhoodId,
    baseline_presence: Rating,
    current_presence: Option<Rating>,
    deployments: Vec<PatrolPatternDeployment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PatrolPatternDeployment {
    id: PatrolDeploymentId,
    version: u32,
    windows: Vec<PatrolWindow>,
}

pub(crate) fn validate_surveillance_request(
    kind: OperationKind,
    objective: &OperationObjective,
) -> Result<(), SurveillanceError> {
    if kind != OperationKind::Surveillance {
        return Ok(());
    }
    let OperationObjective::GatherInformation { target } = objective else {
        return Err(SurveillanceError::InvalidObjective);
    };
    if !is_supported_surveillance_target(*target) {
        return Err(SurveillanceError::UnsupportedTarget(*target));
    }
    Ok(())
}

pub(crate) const fn is_supported_surveillance_target(target: EntityRef) -> bool {
    match target {
        EntityRef::Organization(_)
        | EntityRef::Character(_)
        | EntityRef::Neighborhood(_)
        | EntityRef::Business(_)
        | EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Enterprise(_) => true,
        EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => false,
    }
}

pub(crate) fn decide_surveillance_intelligence(
    registry: &Registry,
    state: &AppState,
    operation: &OperationRecord,
    outcome: OperationObjectiveOutcome,
) -> Result<Option<SurveillanceIntelligencePlan>, SurveillanceError> {
    if operation.kind() != OperationKind::Surveillance {
        return Ok(None);
    }
    let OperationObjective::GatherInformation { target } = operation.objective() else {
        return Err(SurveillanceError::InvalidObjective);
    };
    if !is_supported_surveillance_target(*target) {
        return Err(SurveillanceError::UnsupportedTarget(*target));
    }
    let observed_at = state.now();
    let surveiller = operation.responsible_organization();
    let snapshot = resolve_target_snapshot(state, *target, observed_at, surveiller)?;
    let bucket_minutes = u16::try_from(
        registry
            .get_operation(OperationKind::Surveillance)
            .execution()
            .patrol_observation_bucket()
            .as_minutes(),
    )
    .expect("validated patrol observation bucket must fit one-day minute width");
    let observations = build_observations(&snapshot, outcome, observed_at, bucket_minutes);
    Ok(Some(SurveillanceIntelligencePlan {
        target: *target,
        observed_at,
        surveiller,
        snapshot,
        observations,
    }))
}

pub(crate) fn validate_surveillance_plan_snapshot(
    state: &AppState,
    plan: &SurveillanceIntelligencePlan,
) -> Result<(), SurveillanceError> {
    if state.now() != plan.observed_at {
        return Err(SurveillanceError::StaleTarget(plan.target));
    }
    let current = resolve_target_snapshot(state, plan.target, plan.observed_at, plan.surveiller)?;
    if current != plan.snapshot {
        return Err(SurveillanceError::StaleTarget(plan.target));
    }
    Ok(())
}

pub(crate) fn validate_surveillance_information(
    state: &AppState,
    organization: OrganizationId,
    source_operation: OperationId,
    plan: &SurveillanceIntelligencePlan,
) -> Result<Vec<ValidatedInformation>, crate::intelligence::intelligence_system::IntelligenceError>
{
    plan.observations
        .iter()
        .map(|observation| {
            let draft = InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind: InformationSourceKind::Surveillance,
                topic: observation.topic,
                source_entity: Some(EntityRef::Operation(source_operation)),
                subject: observation.subject,
                observed_at: plan.observed_at,
                reliability: observation.reliability,
                specificity: observation.specificity,
                summary: observation.summary.clone(),
            };
            match &observation.signal {
                Some(signal) => {
                    validate_record_information_with_signal(state, draft, signal.clone())
                }
                None => validate_record_information(state, draft),
            }
        })
        .collect()
}

pub(crate) fn surveillance_after_action_clause(
    plan: Option<&SurveillanceIntelligencePlan>,
    outcome: OperationObjectiveOutcome,
) -> Option<String> {
    let plan = plan?;
    let findings = plan.observation_findings().collect::<Vec<_>>().join("; ");
    let clause = match outcome {
        OperationObjectiveOutcome::Achieved => format!(
            "Surveillance produced {} usable target observation{}{}.",
            plan.observation_count(),
            if plan.observation_count() == 1 {
                ""
            } else {
                "s"
            },
            if findings.is_empty() {
                String::new()
            } else {
                format!(": {findings}")
            }
        ),
        OperationObjectiveOutcome::Partial => format!(
            "Surveillance produced {} limited target observation{}; important details remain unresolved.{}",
            plan.observation_count(),
            if plan.observation_count() == 1 {
                ""
            } else {
                "s"
            },
            if findings.is_empty() {
                String::new()
            } else {
                format!(" Covered: {findings}.")
            }
        ),
        OperationObjectiveOutcome::Failed => {
            "Surveillance produced no target observation reliable enough for planning.".to_owned()
        }
    };
    Some(clause)
}

pub(crate) fn is_valid_persisted_surveillance_information(
    operation: &OperationRecord,
    information: &InformationRecord,
) -> bool {
    let Some(resolution) = operation.resolution() else {
        return false;
    };
    let Some((expected_reliability, expected_specificity)) =
        observation_quality(resolution.objective_outcome())
    else {
        return false;
    };
    if operation.kind() != OperationKind::Surveillance
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::Surveillance
        || information.source_entity() != Some(EntityRef::Operation(operation.id()))
        || information.observed_at() != resolution.resolved_at()
        || information.recorded_at() != resolution.resolved_at()
        || information.reliability() != expected_reliability
        || information.specificity() != expected_specificity
        || !information.derived_from().is_empty()
    {
        return false;
    }
    // One source of truth for the target→observation table: the resolution froze topic, subject,
    // and typed semantics. Re-deriving from current state would let later changes silently
    // invalidate honest intelligence; omitting semantics would let corrupted saves rewrite facts.
    resolution.surveillance_signatures().contains(&(
        information.topic(),
        information.subject(),
        information.signal().cloned(),
    ))
}

fn resolve_target_snapshot(
    state: &AppState,
    target: EntityRef,
    at: SimTime,
    surveiller: OrganizationId,
) -> Result<SurveillanceTargetSnapshot, SurveillanceError> {
    match target {
        EntityRef::Neighborhood(id) => {
            let neighborhood = state
                .world
                .get_neighborhood(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            Ok(SurveillanceTargetSnapshot::Neighborhood {
                id,
                name: neighborhood.name().to_owned(),
                patrol: resolve_patrol_pattern(state, id, at),
            })
        }
        EntityRef::Business(id) => {
            let business = state
                .world
                .get_business(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            let neighborhood = state
                .world
                .get_neighborhood(business.neighborhood())
                .ok_or(SurveillanceError::MissingTarget(target))?;
            Ok(SurveillanceTargetSnapshot::Business {
                id,
                name: business.name().to_owned(),
                functions: business.functions().clone(),
                neighborhood: business.neighborhood(),
                neighborhood_name: neighborhood.name().to_owned(),
                patrol: resolve_patrol_pattern(state, business.neighborhood(), at),
            })
        }
        EntityRef::Character(id) => {
            let character = state
                .world
                .get_character(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            let organization = character.organization().map(|organization| {
                let record = state
                    .world
                    .get_organization(organization)
                    .expect("character organization must exist in valid state");
                (organization, record.name().to_owned())
            });
            let supervisor = character.supervisor().map(|supervisor| {
                let record = state
                    .world
                    .get_character(supervisor)
                    .expect("character supervisor must exist in valid state");
                (supervisor, record.name().to_owned())
            });
            Ok(SurveillanceTargetSnapshot::Character {
                id,
                name: character.name().to_owned(),
                organization,
                supervisor,
            })
        }
        EntityRef::Organization(id) => {
            let organization = state
                .world
                .get_organization(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            let active_members = state
                .world
                .characters_in_organization(id)
                .filter(|character| {
                    state
                        .legal
                        .active_arrest_for_character(character.id())
                        .is_none()
                })
                .map(|character| (character.id(), character.name().to_owned()))
                .collect();
            let law_enforcement_sightline = if is_law_enforcement_authority(organization.kind()) {
                // Watching an authority may reveal whether it is working a case caused by the
                // surveiller's own activity. Case selection uses only durable origin ownership,
                // never evidence or subjects; the surveillance operation itself is what turns
                // that institutional truth into organization-held knowledge.
                resolve_known_authority_case_activity(state, id, surveiller)
            } else {
                None
            };
            Ok(SurveillanceTargetSnapshot::Organization {
                id,
                name: organization.name().to_owned(),
                active_members,
                law_enforcement_sightline,
            })
        }
        EntityRef::Investigation(id) => {
            let investigation = state
                .legal
                .get_investigation(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            // Privacy boundary for watching a specific known case: its public face — title,
            // owning authority, lifecycle status, and visibly assigned personnel — is fair
            // surveillance observation, exactly like watching any business or character. The
            // evidence graph and named subjects are never read here. This deliberately differs
            // from the organization sightline above, which can only associate authority activity
            // with cases whose durable origin belongs to the surveilling organization.
            let owner = state
                .world
                .get_organization(investigation.owner())
                .expect("investigation owner must exist in valid state");
            let lead = investigation.lead_investigator().map(|lead| {
                let character = state
                    .world
                    .get_character(lead)
                    .expect("lead investigator must exist in valid state");
                (lead, character.name().to_owned())
            });
            Ok(SurveillanceTargetSnapshot::Investigation {
                id,
                title: investigation.title().to_owned(),
                owner: investigation.owner(),
                owner_name: owner.name().to_owned(),
                status: investigation.status(),
                lead,
            })
        }
        EntityRef::Enterprise(id) => {
            let enterprise = state
                .enterprises
                .get_enterprise(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            let organization = state
                .world
                .get_organization(enterprise.organization())
                .expect("enterprise organization must exist in valid state");
            let manager = state
                .world
                .get_character(enterprise.manager())
                .expect("enterprise manager must exist in valid state");
            Ok(SurveillanceTargetSnapshot::Enterprise {
                id,
                organization: enterprise.organization(),
                organization_name: organization.name().to_owned(),
                manager: enterprise.manager(),
                manager_name: manager.name().to_owned(),
                location: enterprise.location(),
                location_name: enterprise_location_name(state, enterprise.location()),
                status: enterprise.status(),
            })
        }
        EntityRef::Operation(id) => {
            let operation = state
                .operations
                .get_operation(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
            let organization = state
                .world
                .get_organization(operation.responsible_organization())
                .expect("operation organization must exist in valid state");
            Ok(SurveillanceTargetSnapshot::Operation {
                id,
                organization: operation.responsible_organization(),
                organization_name: organization.name().to_owned(),
                status: operation.status(),
            })
        }
        EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => Err(SurveillanceError::UnsupportedTarget(target)),
    }
}

fn resolve_patrol_pattern(
    state: &AppState,
    neighborhood: NeighborhoodId,
    at: SimTime,
) -> PatrolPatternSnapshot {
    let baseline_presence = state
        .world
        .get_neighborhood(neighborhood)
        .expect("surveillance patrol neighborhood must exist")
        .profile()
        .institutions
        .police_presence;
    let current_presence =
        crate::legal::patrol_system::resolve_patrol_presence(state, neighborhood, at);
    let deployments = state
        .legal
        .active_patrol_deployments_for_neighborhood(neighborhood)
        .map(|deployment| PatrolPatternDeployment {
            id: deployment.id(),
            version: deployment.version(),
            windows: deployment.windows().to_vec(),
        })
        .collect();
    PatrolPatternSnapshot {
        neighborhood,
        baseline_presence,
        current_presence,
        deployments,
    }
}

fn build_observations(
    snapshot: &SurveillanceTargetSnapshot,
    outcome: OperationObjectiveOutcome,
    observed_at: SimTime,
    patrol_bucket_minutes: u16,
) -> Vec<SurveillanceObservation> {
    let Some((reliability, specificity)) = observation_quality(outcome) else {
        return Vec::new();
    };
    match snapshot {
        SurveillanceTargetSnapshot::Neighborhood { id, name, patrol } => {
            vec![SurveillanceObservation {
                topic: InformationTopic::PoliceActivity,
                subject: EntityRef::Neighborhood(*id),
                reliability,
                specificity,
                signal: patrol_pattern_signal(patrol, outcome, patrol_bucket_minutes),
                summary: patrol_summary(name, patrol, outcome, observed_at, patrol_bucket_minutes),
                finding: format!("police activity around {name}"),
            }]
        }
        SurveillanceTargetSnapshot::Business {
            id,
            name,
            functions,
            neighborhood,
            neighborhood_name,
            patrol,
        } => {
            let mut observations = vec![SurveillanceObservation {
                topic: InformationTopic::PoliceActivity,
                subject: EntityRef::Neighborhood(*neighborhood),
                reliability,
                specificity,
                signal: patrol_pattern_signal(patrol, outcome, patrol_bucket_minutes),
                summary: patrol_summary(
                    neighborhood_name,
                    patrol,
                    outcome,
                    observed_at,
                    patrol_bucket_minutes,
                ),
                finding: format!("police activity around {neighborhood_name}"),
            }];
            if outcome == OperationObjectiveOutcome::Achieved {
                observations.push(SurveillanceObservation {
                    topic: InformationTopic::MarketAccess,
                    subject: EntityRef::Business(*id),
                    reliability,
                    specificity,
                    signal: None,
                    summary: business_access_summary(name, functions),
                    finding: format!("access intelligence at {name}"),
                });
            }
            observations
        }
        SurveillanceTargetSnapshot::Character {
            id,
            name,
            organization,
            supervisor,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::Personnel,
            subject: EntityRef::Character(*id),
            reliability,
            specificity,
            signal: None,
            summary: character_summary(name, organization.as_ref(), supervisor.as_ref()),
            finding: format!("the movements of {name}"),
        }],
        SurveillanceTargetSnapshot::Organization {
            id,
            name,
            active_members,
            law_enforcement_sightline,
        } => match law_enforcement_sightline {
            Some(activity) => vec![SurveillanceObservation {
                topic: InformationTopic::LegalActivity,
                subject: EntityRef::Organization(*id),
                reliability,
                specificity,
                signal: authority_sightline_signal(*activity, outcome)
                    .map(InformationSignal::CaseActivity),
                summary: authority_sightline_summary(name, *activity, outcome),
                finding: format!("case activity at {name}"),
            }],
            None => vec![SurveillanceObservation {
                topic: InformationTopic::Personnel,
                subject: EntityRef::Organization(*id),
                reliability,
                specificity,
                signal: organization_personnel_signal(active_members, outcome),
                summary: organization_summary(name, active_members, outcome),
                finding: format!("personnel around {name}"),
            }],
        },
        SurveillanceTargetSnapshot::Investigation {
            id,
            title,
            owner_name,
            status,
            lead,
            owner: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::LegalActivity,
            subject: EntityRef::Investigation(*id),
            reliability,
            specificity,
            signal: Some(InformationSignal::CaseActivity(
                crate::legal::case_knowledge::activity_for_status(*status),
            )),
            summary: investigation_summary(title, owner_name, *status, lead.as_ref(), outcome),
            finding: format!("the status of {title}"),
        }],
        SurveillanceTargetSnapshot::Enterprise {
            id,
            organization_name,
            manager_name,
            location_name,
            status,
            organization: _,
            manager: _,
            location: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::Personnel,
            subject: EntityRef::Enterprise(*id),
            reliability,
            specificity,
            signal: None,
            summary: enterprise_summary(organization_name, manager_name, location_name, *status),
            finding: format!("activity at {location_name}"),
        }],
        SurveillanceTargetSnapshot::Operation {
            id,
            organization_name,
            status,
            organization: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::OperationalOutcome,
            subject: EntityRef::Operation(*id),
            reliability,
            specificity,
            signal: None,
            summary: format!(
                "Observed activity linked to {} appears {}.",
                organization_name,
                operation_status_label(*status)
            ),
            finding: format!("activity linked to {organization_name}"),
        }],
    }
}

fn observation_quality(outcome: OperationObjectiveOutcome) -> Option<(Reliability, Specificity)> {
    match outcome {
        OperationObjectiveOutcome::Achieved => {
            Some((Reliability::GenerallyReliable, Specificity::Specific))
        }
        OperationObjectiveOutcome::Partial => Some((Reliability::Mixed, Specificity::General)),
        OperationObjectiveOutcome::Failed => None,
    }
}

fn patrol_summary(
    neighborhood_name: &str,
    patrol: &PatrolPatternSnapshot,
    outcome: OperationObjectiveOutcome,
    observed_at: SimTime,
    bucket_minutes: u16,
) -> String {
    if outcome == OperationObjectiveOutcome::Partial {
        let presence = patrol.current_presence.unwrap_or(patrol.baseline_presence);
        return format!(
            "Police activity around {neighborhood_name} appeared {} during the observation period; a dependable daily patrol pattern was not established.",
            police_presence_label(presence)
        );
    }
    if patrol.deployments.is_empty() {
        return format!(
            "No stable daily patrol deployment pattern was confirmed around {neighborhood_name}; visible police activity appears {} overall.",
            police_presence_label(patrol.baseline_presence)
        );
    }
    let observed_windows = observed_patrol_windows(patrol);
    let windows = observed_windows
        .iter()
        .copied()
        .map(|window| approximate_patrol_window(window, bucket_minutes))
        .collect::<Vec<_>>();
    let minute = u16::try_from(observed_at.as_minutes() % u64::from(DAY_MINUTES_U16))
        .expect("minute-of-day remainder must fit u16");
    format!(
        "Observed patrol activity around {neighborhood_name} follows a recurring pattern: {}. Around {}, activity was {}.",
        windows.join(", "),
        format_day_minute(rounded_day_minute(minute, bucket_minutes)),
        police_presence_label(patrol.current_presence.unwrap_or(patrol.baseline_presence))
    )
}

fn patrol_pattern_signal(
    patrol: &PatrolPatternSnapshot,
    outcome: OperationObjectiveOutcome,
    bucket_minutes: u16,
) -> Option<InformationSignal> {
    if outcome != OperationObjectiveOutcome::Achieved || patrol.deployments.is_empty() {
        return None;
    }
    let intervals = observed_patrol_windows(patrol)
        .into_iter()
        .flat_map(|window| approximate_patrol_intervals(window, bucket_minutes))
        .collect::<BTreeSet<_>>();
    (!intervals.is_empty()).then_some(InformationSignal::PatrolPattern { intervals })
}

fn observed_patrol_windows(patrol: &PatrolPatternSnapshot) -> Vec<PatrolWindow> {
    patrol
        .deployments
        .iter()
        .flat_map(|deployment| deployment.windows.iter().copied())
        .collect()
}

fn approximate_patrol_intervals(
    window: PatrolWindow,
    bucket_minutes: u16,
) -> Vec<PatrolIntervalSignal> {
    let Some((start, end)) = approximate_patrol_bounds(window, bucket_minutes) else {
        return vec![
            PatrolIntervalSignal::try_new(0, DAY_MINUTES_U16)
                .expect("all-day patrol interval must be valid"),
        ];
    };
    if end > start {
        return vec![
            PatrolIntervalSignal::try_new(start, end)
                .expect("ordered patrol interval must be valid"),
        ];
    }
    let mut intervals = vec![
        PatrolIntervalSignal::try_new(start, DAY_MINUTES_U16)
            .expect("wrapped patrol tail must be valid"),
    ];
    if end > 0 {
        intervals.push(
            PatrolIntervalSignal::try_new(0, end).expect("wrapped patrol head must be valid"),
        );
    }
    intervals
}

fn approximate_patrol_window(window: PatrolWindow, bucket_minutes: u16) -> String {
    let Some((start, end)) = approximate_patrol_bounds(window, bucket_minutes) else {
        return format!("all day ({})", police_presence_label(window.presence()));
    };
    let display_end = if end == DAY_MINUTES_U16 { 0 } else { end };
    format!(
        "roughly {}-{} ({})",
        format_day_minute(start),
        format_day_minute(display_end),
        police_presence_label(window.presence())
    )
}

/// Expands an observed patrol window to containing authored observation-bucket boundaries.
/// Rounding both endpoints independently to the nearest bucket can collapse a real short window;
/// treating that collapse as all-day presence would turn a few observed minutes into twenty-four
/// hours of actionable police coverage. Containing bounds preserve uncertainty without inventing
/// coverage the observation disproves. `None` means the conservative expansion covers the full day.
fn approximate_patrol_bounds(window: PatrolWindow, bucket_minutes: u16) -> Option<(u16, u16)> {
    let bucket_minutes = u32::from(bucket_minutes);
    let day = u32::from(DAY_MINUTES_U16);
    let start = u32::from(window.start().value());
    let end = start + u32::from(window.duration_minutes());
    let approximate_start = start / bucket_minutes * bucket_minutes;
    let approximate_end = end.div_ceil(bucket_minutes) * bucket_minutes;
    if approximate_end - approximate_start >= day {
        return None;
    }
    let start = u16::try_from(approximate_start % day)
        .expect("bucketed patrol start must fit minute-of-day width");
    let end = if approximate_end <= day {
        u16::try_from(approximate_end).expect("same-day patrol end must fit interval width")
    } else {
        u16::try_from(approximate_end % day)
            .expect("wrapped patrol end must fit minute-of-day width")
    };
    debug_assert_ne!(start, end);
    Some((start, end))
}

fn rounded_day_minute(minute: u16, bucket_minutes: u16) -> u16 {
    let bucket = u32::from(bucket_minutes);
    let rounded = (u32::from(minute) + bucket / 2) / bucket * bucket;
    u16::try_from(rounded % u32::from(DAY_MINUTES_U16)).expect("rounded day minute must fit u16")
}

fn format_day_minute(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn police_presence_label(rating: Rating) -> &'static str {
    rating.police_presence_label()
}

fn business_access_summary(name: &str, functions: &BTreeSet<BusinessFunction>) -> String {
    let access = functions
        .iter()
        .map(|function| business_function_label(*function))
        .collect::<Vec<_>>();
    if access.is_empty() {
        format!("Surveillance of {name} identified no specialized operating access.")
    } else {
        format!(
            "Surveillance of {name} confirmed operating access associated with {}.",
            access.join(", ")
        )
    }
}

fn business_function_label(function: BusinessFunction) -> &'static str {
    match function {
        BusinessFunction::CashIntensive => "heavy cash handling",
        BusinessFunction::VehicleFleet => "a vehicle fleet",
        BusinessFunction::Warehousing => "storage space",
        BusinessFunction::MeetingSpace => "private meeting space",
        BusinessFunction::CustomerAccess => "regular customer access",
        BusinessFunction::ResaleMarket => "resale-market access",
        BusinessFunction::UnionAccess => "union access",
        BusinessFunction::DistributionInfrastructure => "distribution infrastructure",
        BusinessFunction::ProfessionalRecords => "professional record handling",
        BusinessFunction::AlcoholProduction => "alcohol production",
        BusinessFunction::Nightlife => "nightlife venue",
    }
}

fn character_summary(
    name: &str,
    organization: Option<&(OrganizationId, String)>,
    supervisor: Option<&(CharacterId, String)>,
) -> String {
    let affiliation = organization
        .map(|(_, organization)| format!("regularly associated with {organization}"))
        .unwrap_or_else(|| "not regularly associated with a known organization".to_owned());
    let reporting = supervisor
        .map(|(_, supervisor)| format!(" An apparent reporting contact is {supervisor}."))
        .unwrap_or_default();
    format!("Surveillance observed {name} {affiliation}.{reporting}")
}

fn is_law_enforcement_authority(kind: OrganizationKind) -> bool {
    matches!(
        kind,
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    )
}

fn resolve_known_authority_case_activity(
    state: &AppState,
    authority: OrganizationId,
    surveiller: OrganizationId,
) -> Option<CaseActivitySignal> {
    state
        .legal
        .investigations_for_owner(authority)
        .filter(|case| {
            case.origin().is_some_and(|origin| {
                crate::legal::investigation_system::case_origin_responsible_organization(
                    state, origin,
                ) == Some(surveiller)
            })
        })
        .map(|case| crate::legal::case_knowledge::activity_for_status(case.status()))
        .fold(None, |aggregate, activity| {
            Some(match (aggregate, activity) {
                (Some(CaseActivitySignal::Active), _) | (_, CaseActivitySignal::Active) => {
                    CaseActivitySignal::Active
                }
                (Some(CaseActivitySignal::Shelved), _) | (_, CaseActivitySignal::Shelved) => {
                    CaseActivitySignal::Shelved
                }
                (None | Some(CaseActivitySignal::Closed), CaseActivitySignal::Closed) => {
                    CaseActivitySignal::Closed
                }
            })
        })
}

fn authority_sightline_summary(
    name: &str,
    activity: CaseActivitySignal,
    outcome: OperationObjectiveOutcome,
) -> String {
    // The observation reports only visible authority activity tied to a case caused by the
    // surveilling organization's own activity; it never reveals evidence, subjects, or internals.
    if outcome == OperationObjectiveOutcome::Partial {
        return format!(
            "Visible activity around {name} remained difficult to judge; a dependable read on whether the case is still being actively developed was not established."
        );
    }
    // Dependable reads share the same display prefix as investigator-held case knowledge so
    // the two player-facing channels describe case activity consistently without parsing prose.
    let prose = match activity {
        CaseActivitySignal::Active => format!(
            "Detectives around {name} appear to be actively developing the case connected to your recent activity. The matter has not gone quiet."
        ),
        CaseActivitySignal::Shelved => format!(
            "No active case machinery connected to your recent activity was observed around {name}; the matter appears to have been shelved and routine police functions continue."
        ),
        CaseActivitySignal::Closed => format!(
            "No active case machinery connected to your recent activity was observed around {name}; the matter appears closed."
        ),
    };
    format!(
        "{} {prose}",
        crate::legal::case_knowledge::case_activity_summary_prefix(activity)
    )
}

fn authority_sightline_signal(
    activity: CaseActivitySignal,
    outcome: OperationObjectiveOutcome,
) -> Option<CaseActivitySignal> {
    (outcome == OperationObjectiveOutcome::Achieved).then_some(activity)
}

fn organization_summary(
    name: &str,
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> String {
    let observed = observed_organization_members(active_members, outcome)
        .iter()
        .map(|(_, member)| member.as_str())
        .collect::<Vec<_>>();
    if observed.is_empty() {
        format!("Surveillance of {name} did not identify a recurring active affiliate.")
    } else {
        format!(
            "Recurring activity around {name} included {}.",
            observed.join(", ")
        )
    }
}

fn organization_personnel_signal(
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> Option<InformationSignal> {
    let characters = observed_organization_members(active_members, outcome)
        .iter()
        .map(|(character, _)| *character)
        .collect::<BTreeSet<_>>();
    (!characters.is_empty()).then_some(InformationSignal::PersonnelPresence { characters })
}

fn observed_organization_members(
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> &[(CharacterId, String)] {
    let limit = if outcome == OperationObjectiveOutcome::Achieved {
        3
    } else {
        1
    };
    &active_members[..active_members.len().min(limit)]
}

fn investigation_summary(
    title: &str,
    owner_name: &str,
    status: InvestigationStatus,
    lead: Option<&(CharacterId, String)>,
    outcome: OperationObjectiveOutcome,
) -> String {
    let lead_clause = if outcome == OperationObjectiveOutcome::Achieved {
        lead.map(|(_, name)| format!(" {name} appears to be directing the visible work."))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "Visible activity around the {title} file indicates the matter is {} under {owner_name}.{lead_clause}",
        investigation_status_label(status)
    )
}

fn enterprise_summary(
    organization_name: &str,
    manager_name: &str,
    location_name: &str,
    status: EnterpriseStatus,
) -> String {
    format!(
        "Activity at {location_name} appears {} under {manager_name} for {organization_name}.",
        enterprise_status_label(status)
    )
}

fn enterprise_location_name(state: &AppState, location: EnterpriseLocation) -> String {
    match location {
        EnterpriseLocation::Neighborhood(neighborhood) => state
            .world
            .get_neighborhood(neighborhood)
            .expect("enterprise surveillance target must reference a persisted neighborhood")
            .name()
            .to_owned(),
        EnterpriseLocation::Business(business) => state
            .world
            .get_business(business)
            .expect("enterprise surveillance target must reference a persisted business")
            .name()
            .to_owned(),
    }
}

fn investigation_status_label(status: InvestigationStatus) -> &'static str {
    match status {
        InvestigationStatus::Active => "active",
        InvestigationStatus::Suspended => "quiet or suspended",
        InvestigationStatus::Closed => "closed",
    }
}

fn enterprise_status_label(status: EnterpriseStatus) -> &'static str {
    match status {
        EnterpriseStatus::Active => "active",
        EnterpriseStatus::Suspended => "inactive or suspended",
        EnterpriseStatus::Retired => "closed and retired",
    }
}

fn operation_status_label(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Authorized => "planned but not yet underway",
        OperationStatus::InProgress => "currently underway",
        OperationStatus::AwaitingDecision => "paused pending direction",
        OperationStatus::Completed => "completed",
        OperationStatus::Aborted => "aborted",
    }
}

#[cfg(test)]
mod tests;
