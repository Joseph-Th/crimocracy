//! Surveillance operation integration that turns observed world state into bounded organization knowledge.

mod after_action;
mod observation_text;

pub(crate) use after_action::{
    persisted_surveillance_after_action_clause, surveillance_after_action_clause,
};
use observation_text::*;

use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, CharacterId, EnterpriseId, InvestigationId, NeighborhoodId, OperationId,
    OrganizationId, PatrolDeploymentId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES_U16, SimTime};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_system_information,
    validate_record_system_information_with_signal,
};
use crate::intelligence::{
    CaseActivitySignal, EnterpriseLocationSignal, InformationDraft, InformationRecord,
    InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder,
    PatrolIntervalSignal, Reliability, Specificity,
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
pub(crate) enum SurveillanceRequestError {
    #[error("surveillance operations require a gather-information objective")]
    InvalidObjective,
    #[error("entity {0:?} cannot be directly observed by surveillance")]
    UnsupportedTarget(EntityRef),
}

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
        active_enterprises: Vec<EnterpriseSnapshot>,
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
        enterprise: EnterpriseSnapshot,
        // Direct observation earns local patrol knowledge; broad organization discovery does not.
        neighborhood_name: String,
        patrol: PatrolPatternSnapshot,
    },
    Operation {
        id: OperationId,
        organization: OrganizationId,
        organization_name: String,
        status: OperationStatus,
    },
}

/// Only visibly observable enterprise facts, shared by direct and organization surveillance.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseSnapshot {
    id: EnterpriseId,
    organization: OrganizationId,
    organization_name: String,
    manager: CharacterId,
    manager_name: String,
    location: EnterpriseLocation,
    location_name: String,
    kind: crate::enterprises::EnterpriseKind,
    status: EnterpriseStatus,
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
) -> Result<(), SurveillanceRequestError> {
    if kind != OperationKind::Surveillance {
        return Ok(());
    }
    let OperationObjective::GatherInformation { target } = objective else {
        return Err(SurveillanceRequestError::InvalidObjective);
    };
    if !is_supported_surveillance_target(*target) {
        return Err(SurveillanceRequestError::UnsupportedTarget(*target));
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
    let snapshot = resolve_target_snapshot(
        state,
        *target,
        observed_at,
        surveiller,
        registry.legal().off_window_patrol_presence_percent(),
    )?;
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
    off_window_patrol_presence_percent: u8,
) -> Result<(), SurveillanceError> {
    crate::core::time::ensure_time_current(state.now(), plan.observed_at)
        .map_err(|_| SurveillanceError::StaleTarget(plan.target))?;
    let current = resolve_target_snapshot(
        state,
        plan.target,
        plan.observed_at,
        plan.surveiller,
        off_window_patrol_presence_percent,
    )?;
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
                    validate_record_system_information_with_signal(state, draft, signal.clone())
                }
                None => validate_record_system_information(state, draft),
            }
        })
        .collect()
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
    off_window_patrol_presence_percent: u8,
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
                patrol: resolve_patrol_pattern(state, id, at, off_window_patrol_presence_percent),
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
                patrol: resolve_patrol_pattern(
                    state,
                    business.neighborhood(),
                    at,
                    off_window_patrol_presence_percent,
                ),
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
            // The organization index is EnterpriseId ordered. Snapshot the bounded selection
            // itself so additions, lifecycle changes, and every displayed dependency are
            // re-derived at validation without reading ledgers or institutional case truth.
            let active_enterprises = if organization.kind() == OrganizationKind::Criminal {
                state
                    .enterprises
                    .active_for_organization(id)
                    .take(3)
                    .map(|enterprise| resolve_enterprise_snapshot(state, enterprise))
                    .collect()
            } else {
                Vec::new()
            };
            Ok(SurveillanceTargetSnapshot::Organization {
                id,
                name: organization.name().to_owned(),
                active_members,
                active_enterprises,
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
            let neighborhood =
                crate::enterprises::enterprise_execution::resolve_location_neighborhood(
                    state,
                    enterprise.location(),
                )
                .map_err(|_| SurveillanceError::MissingTarget(target))?;
            let neighborhood_name = state
                .world
                .get_neighborhood(neighborhood)
                .expect("enterprise surveillance neighborhood must exist")
                .name()
                .to_owned();
            Ok(SurveillanceTargetSnapshot::Enterprise {
                enterprise: resolve_enterprise_snapshot(state, enterprise),
                neighborhood_name,
                patrol: resolve_patrol_pattern(
                    state,
                    neighborhood,
                    at,
                    off_window_patrol_presence_percent,
                ),
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

fn resolve_enterprise_snapshot(
    state: &AppState,
    enterprise: &crate::enterprises::EnterpriseRecord,
) -> EnterpriseSnapshot {
    let organization = state
        .world
        .get_organization(enterprise.organization())
        .expect("enterprise organization must exist in valid state");
    let manager = state
        .world
        .get_character(enterprise.manager())
        .expect("enterprise manager must exist in valid state");
    EnterpriseSnapshot {
        id: enterprise.id(),
        organization: enterprise.organization(),
        organization_name: organization.name().to_owned(),
        manager: enterprise.manager(),
        manager_name: manager.name().to_owned(),
        location: enterprise.location(),
        location_name: enterprise_location_name(state, enterprise.location()),
        kind: enterprise.kind(),
        status: enterprise.status(),
    }
}

fn resolve_patrol_pattern(
    state: &AppState,
    neighborhood: NeighborhoodId,
    at: SimTime,
    off_window_patrol_presence_percent: u8,
) -> PatrolPatternSnapshot {
    let baseline_presence = state
        .world
        .get_neighborhood(neighborhood)
        .expect("surveillance patrol neighborhood must exist")
        .profile()
        .institutions
        .police_presence;
    let current_presence =
        crate::legal::patrol_system::resolve_patrol_presence_snapshot_with_percent(
            state,
            neighborhood,
            at,
            off_window_patrol_presence_percent,
        )
        .presence();
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
            active_enterprises,
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
            None => {
                let mut observations = vec![SurveillanceObservation {
                    topic: InformationTopic::Personnel,
                    subject: EntityRef::Organization(*id),
                    reliability,
                    specificity,
                    signal: organization_personnel_signal(active_members, outcome),
                    summary: organization_summary(name, active_members, outcome),
                    finding: format!("personnel around {name}"),
                }];
                if outcome == OperationObjectiveOutcome::Achieved {
                    observations.extend(active_enterprises.iter().map(|enterprise| {
                        enterprise_observation(enterprise, reliability, specificity)
                    }));
                }
                observations
            }
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
            signal: investigation_case_signal(*status, outcome),
            summary: investigation_summary(title, owner_name, *status, lead.as_ref(), outcome),
            finding: format!("the status of {title}"),
        }],
        SurveillanceTargetSnapshot::Enterprise {
            enterprise,
            neighborhood_name,
            patrol,
        } => vec![
            enterprise_observation(enterprise, reliability, specificity),
            SurveillanceObservation {
                topic: InformationTopic::PoliceActivity,
                subject: EntityRef::Neighborhood(patrol.neighborhood),
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
            },
        ],
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

fn enterprise_observation(
    enterprise: &EnterpriseSnapshot,
    reliability: Reliability,
    specificity: Specificity,
) -> SurveillanceObservation {
    let location_signal = match enterprise.location {
        EnterpriseLocation::Business(business) => EnterpriseLocationSignal::Business(business),
        EnterpriseLocation::Neighborhood(neighborhood) => {
            EnterpriseLocationSignal::Neighborhood(neighborhood)
        }
    };
    SurveillanceObservation {
        topic: InformationTopic::EnterpriseActivity,
        subject: EntityRef::Enterprise(enterprise.id),
        reliability,
        specificity,
        signal: Some(InformationSignal::EnterpriseLocation(location_signal)),
        summary: enterprise_summary(
            enterprise.kind,
            &enterprise.organization_name,
            &enterprise.manager_name,
            &enterprise.location_name,
            enterprise.status,
        ),
        finding: format!(
            "{} activity at {}",
            enterprise_kind_label(enterprise.kind),
            enterprise.location_name
        ),
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

#[cfg(test)]
mod tests;
