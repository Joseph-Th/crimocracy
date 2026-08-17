//! Surveillance operation integration that turns observed world state into bounded organization knowledge.

use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, CharacterId, EnterpriseId, InvestigationId, NeighborhoodId, OperationId,
    OrganizationId, PatrolDeploymentId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::intelligence::intelligence_system::{validate_record_information, ValidatedInformation};
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSourceKind, InformationTopic, KnowledgeHolder,
    Reliability, Specificity,
};
use crate::legal::{InvestigationStatus, PatrolWindow};
use crate::operations::{
    OperationKind, OperationObjective, OperationObjectiveOutcome, OperationRecord, OperationStatus,
};
use crate::world::{BusinessFunction, Lifecycle, OrganizationKind, Rating};
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
    surveiller: Option<OrganizationId>,
    snapshot: SurveillanceTargetSnapshot,
    observations: Vec<SurveillanceObservation>,
}

impl SurveillanceIntelligencePlan {
    pub(crate) fn observation_count(&self) -> usize {
        self.observations.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LawEnforcementCaseSightline {
    /// Whether the surveilling organization has been surfaced an active operation-originated case
    /// owned by the targeted authority. The sightline never reveals evidence, subjects, or internal
    /// case details; it only distinguishes "the case the organization knows about is still being
    /// actively worked" from "the authority has shelved it".
    active_case_against_surveiller: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SurveillanceObservation {
    topic: InformationTopic,
    subject: EntityRef,
    reliability: Reliability,
    specificity: Specificity,
    summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SurveillanceTargetSnapshot {
    Neighborhood {
        id: NeighborhoodId,
        name: String,
        lifecycle: Lifecycle,
        patrol: PatrolPatternSnapshot,
    },
    Business {
        id: BusinessId,
        name: String,
        lifecycle: Lifecycle,
        functions: BTreeSet<BusinessFunction>,
        neighborhood: NeighborhoodId,
        neighborhood_name: String,
        patrol: PatrolPatternSnapshot,
    },
    Character {
        id: CharacterId,
        name: String,
        lifecycle: Lifecycle,
        organization: Option<(OrganizationId, String)>,
        supervisor: Option<(CharacterId, String)>,
    },
    Organization {
        id: OrganizationId,
        name: String,
        lifecycle: Lifecycle,
        active_members: Vec<(CharacterId, String)>,
        // Present for law-enforcement/legal-authority targets so the player's known case can be
        // re-checked through canonical surveillance after standing down.
        law_enforcement_sightline: Option<LawEnforcementCaseSightline>,
    },
    Investigation {
        id: InvestigationId,
        title: String,
        owner: OrganizationId,
        owner_name: String,
        status: InvestigationStatus,
        lead: Option<(CharacterId, String)>,
        assigned_investigators: BTreeSet<CharacterId>,
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
    let snapshot = capture_target_snapshot(state, *target, observed_at, surveiller)?;
    let observations = build_observations(&snapshot, outcome, observed_at);
    Ok(Some(SurveillanceIntelligencePlan {
        target: *target,
        observed_at,
        surveiller: Some(surveiller),
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
    let surveiller = plan
        .surveiller
        .expect("validated surveillance plan must carry its surveiller");
    let current = capture_target_snapshot(state, plan.target, plan.observed_at, surveiller)?;
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
            validate_record_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Organization(organization),
                    source_kind: InformationSourceKind::Surveillance,
                    topic: observation.topic,
                    source_entity: Some(EntityRef::Operation(source_operation)),
                    subject: observation.subject,
                    observed_at: plan.observed_at,
                    reliability: observation.reliability,
                    specificity: observation.specificity,
                    summary: observation.summary.clone(),
                },
            )
        })
        .collect()
}

pub(crate) fn surveillance_after_action_clause(
    plan: Option<&SurveillanceIntelligencePlan>,
    outcome: OperationObjectiveOutcome,
) -> Option<String> {
    let plan = plan?;
    Some(match outcome {
        OperationObjectiveOutcome::Achieved => format!(
            "Surveillance produced {} usable target observation{}.",
            plan.observation_count(),
            if plan.observation_count() == 1 { "" } else { "s" }
        ),
        OperationObjectiveOutcome::Partial => format!(
            "Surveillance produced {} limited target observation{}; important details remain unresolved.",
            plan.observation_count(),
            if plan.observation_count() == 1 { "" } else { "s" }
        ),
        OperationObjectiveOutcome::Failed => {
            "Surveillance produced no target observation reliable enough for planning.".to_owned()
        }
    })
}

pub(crate) fn is_valid_persisted_surveillance_information(
    state: &AppState,
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
    let OperationObjective::GatherInformation { target } = operation.objective() else {
        return false;
    };
    match *target {
        EntityRef::Neighborhood(neighborhood) => {
            information.topic() == InformationTopic::PoliceActivity
                && information.subject() == EntityRef::Neighborhood(neighborhood)
        }
        EntityRef::Business(business) => {
            (resolution.objective_outcome() == OperationObjectiveOutcome::Achieved
                && information.topic() == InformationTopic::MarketAccess
                && information.subject() == EntityRef::Business(business))
                || (information.topic() == InformationTopic::PoliceActivity
                    && state.world.get_business(business).is_some_and(|record| {
                        information.subject() == EntityRef::Neighborhood(record.neighborhood())
                    }))
        }
        EntityRef::Character(character) => {
            information.topic() == InformationTopic::Personnel
                && information.subject() == EntityRef::Character(character)
        }
        EntityRef::Organization(organization) => {
            let is_law_enforcement = state
                .world
                .get_organization(organization)
                .is_some_and(|record| is_law_enforcement_authority(record.kind()));
            if is_law_enforcement {
                information.topic() == InformationTopic::LegalActivity
                    && information.subject() == EntityRef::Organization(organization)
            } else {
                information.topic() == InformationTopic::Personnel
                    && information.subject() == EntityRef::Organization(organization)
            }
        }
        EntityRef::Investigation(investigation) => {
            information.topic() == InformationTopic::LegalActivity
                && information.subject() == EntityRef::Investigation(investigation)
        }
        EntityRef::Enterprise(enterprise) => {
            information.topic() == InformationTopic::Personnel
                && information.subject() == EntityRef::Enterprise(enterprise)
        }
        EntityRef::Operation(target_operation) => {
            information.topic() == InformationTopic::OperationalOutcome
                && information.subject() == EntityRef::Operation(target_operation)
        }
        EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => false,
    }
}

pub(crate) fn expected_persisted_surveillance_signatures(
    state: &AppState,
    operation: &OperationRecord,
) -> Option<BTreeSet<(InformationTopic, EntityRef)>> {
    if operation.kind() != OperationKind::Surveillance {
        return None;
    }
    let resolution = operation.resolution()?;
    if resolution.objective_outcome() == OperationObjectiveOutcome::Failed {
        return Some(BTreeSet::new());
    }
    let OperationObjective::GatherInformation { target } = operation.objective() else {
        return None;
    };
    let mut expected = BTreeSet::new();
    match *target {
        EntityRef::Neighborhood(neighborhood) => {
            expected.insert((
                InformationTopic::PoliceActivity,
                EntityRef::Neighborhood(neighborhood),
            ));
        }
        EntityRef::Business(business) => {
            let record = state.world.get_business(business)?;
            expected.insert((
                InformationTopic::PoliceActivity,
                EntityRef::Neighborhood(record.neighborhood()),
            ));
            if resolution.objective_outcome() == OperationObjectiveOutcome::Achieved {
                expected.insert((
                    InformationTopic::MarketAccess,
                    EntityRef::Business(business),
                ));
            }
        }
        EntityRef::Character(character) => {
            expected.insert((InformationTopic::Personnel, EntityRef::Character(character)));
        }
        EntityRef::Organization(organization) => {
            let is_law_enforcement = state
                .world
                .get_organization(organization)
                .is_some_and(|record| is_law_enforcement_authority(record.kind()));
            if is_law_enforcement {
                expected.insert((
                    InformationTopic::LegalActivity,
                    EntityRef::Organization(organization),
                ));
            } else {
                expected.insert((
                    InformationTopic::Personnel,
                    EntityRef::Organization(organization),
                ));
            }
        }
        EntityRef::Investigation(investigation) => {
            expected.insert((
                InformationTopic::LegalActivity,
                EntityRef::Investigation(investigation),
            ));
        }
        EntityRef::Enterprise(enterprise) => {
            expected.insert((
                InformationTopic::Personnel,
                EntityRef::Enterprise(enterprise),
            ));
        }
        EntityRef::Operation(target_operation) => {
            expected.insert((
                InformationTopic::OperationalOutcome,
                EntityRef::Operation(target_operation),
            ));
        }
        EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => return None,
    }
    Some(expected)
}

fn capture_target_snapshot(
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
                lifecycle: neighborhood.lifecycle(),
                patrol: capture_patrol_pattern(state, id, at),
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
                lifecycle: business.lifecycle(),
                functions: business.functions().clone(),
                neighborhood: business.neighborhood(),
                neighborhood_name: neighborhood.name().to_owned(),
                patrol: capture_patrol_pattern(state, business.neighborhood(), at),
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
                lifecycle: character.lifecycle(),
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
                .filter(|character| character.lifecycle() == Lifecycle::Active)
                .map(|character| (character.id(), character.name().to_owned()))
                .collect();
            let law_enforcement_sightline = if is_law_enforcement_authority(organization.kind()) {
                Some(LawEnforcementCaseSightline {
                    active_case_against_surveiller: state.legal.investigations().any(|case| {
                        case.owner() == id
                            && case.status() == InvestigationStatus::Active
                            && case.notified_organizations().contains(&surveiller)
                    }),
                })
            } else {
                None
            };
            Ok(SurveillanceTargetSnapshot::Organization {
                id,
                name: organization.name().to_owned(),
                lifecycle: organization.lifecycle(),
                active_members,
                law_enforcement_sightline,
            })
        }
        EntityRef::Investigation(id) => {
            let investigation = state
                .legal
                .get_investigation(id)
                .ok_or(SurveillanceError::MissingTarget(target))?;
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
                assigned_investigators: investigation.assigned_investigators().clone(),
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

fn capture_patrol_pattern(
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
) -> Vec<SurveillanceObservation> {
    let Some((reliability, specificity)) = observation_quality(outcome) else {
        return Vec::new();
    };
    match snapshot {
        SurveillanceTargetSnapshot::Neighborhood {
            id,
            name,
            patrol,
            lifecycle: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::PoliceActivity,
            subject: EntityRef::Neighborhood(*id),
            reliability,
            specificity,
            summary: patrol_summary(name, patrol, outcome, observed_at),
        }],
        SurveillanceTargetSnapshot::Business {
            id,
            name,
            functions,
            neighborhood,
            neighborhood_name,
            patrol,
            lifecycle: _,
        } => {
            let mut observations = vec![SurveillanceObservation {
                topic: InformationTopic::PoliceActivity,
                subject: EntityRef::Neighborhood(*neighborhood),
                reliability,
                specificity,
                summary: patrol_summary(neighborhood_name, patrol, outcome, observed_at),
            }];
            if outcome == OperationObjectiveOutcome::Achieved {
                observations.push(SurveillanceObservation {
                    topic: InformationTopic::MarketAccess,
                    subject: EntityRef::Business(*id),
                    reliability,
                    specificity,
                    summary: business_access_summary(name, functions),
                });
            }
            observations
        }
        SurveillanceTargetSnapshot::Character {
            id,
            name,
            organization,
            supervisor,
            lifecycle: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::Personnel,
            subject: EntityRef::Character(*id),
            reliability,
            specificity,
            summary: character_summary(name, organization.as_ref(), supervisor.as_ref()),
        }],
        SurveillanceTargetSnapshot::Organization {
            id,
            name,
            active_members,
            law_enforcement_sightline,
            lifecycle: _,
        } => match law_enforcement_sightline {
            Some(sightline) => vec![SurveillanceObservation {
                topic: InformationTopic::LegalActivity,
                subject: EntityRef::Organization(*id),
                reliability,
                specificity,
                summary: authority_sightline_summary(
                    name,
                    sightline.active_case_against_surveiller,
                    outcome,
                ),
            }],
            None => vec![SurveillanceObservation {
                topic: InformationTopic::Personnel,
                subject: EntityRef::Organization(*id),
                reliability,
                specificity,
                summary: organization_summary(name, active_members, outcome),
            }],
        },
        SurveillanceTargetSnapshot::Investigation {
            id,
            title,
            owner_name,
            status,
            lead,
            assigned_investigators,
            owner: _,
        } => vec![SurveillanceObservation {
            topic: InformationTopic::LegalActivity,
            subject: EntityRef::Investigation(*id),
            reliability,
            specificity,
            summary: investigation_summary(
                title,
                owner_name,
                *status,
                lead.as_ref(),
                assigned_investigators.len(),
                outcome,
            ),
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
            summary: enterprise_summary(organization_name, manager_name, location_name, *status),
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
            summary: format!(
                "Observed activity linked to {} appears {}.",
                organization_name,
                operation_status_label(*status)
            ),
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
    let mut windows = Vec::new();
    for deployment in &patrol.deployments {
        for window in &deployment.windows {
            if windows.len() == 4 {
                break;
            }
            windows.push(approximate_patrol_window(*window));
        }
        if windows.len() == 4 {
            break;
        }
    }
    let extra = patrol
        .deployments
        .iter()
        .map(|deployment| deployment.windows.len())
        .sum::<usize>()
        .saturating_sub(windows.len());
    let extra_clause = if extra == 0 {
        String::new()
    } else {
        format!(
            ", plus {extra} additional recurring window{}",
            if extra == 1 { "" } else { "s" }
        )
    };
    let minute = u16::try_from(observed_at.as_minutes() % 1_440)
        .expect("minute-of-day remainder must fit u16");
    format!(
        "Observed patrol activity around {neighborhood_name} follows a recurring pattern: {}{extra_clause}. Around {}, activity was {}.",
        windows.join(", "),
        format_day_minute(rounded_half_hour(minute)),
        police_presence_label(patrol.current_presence.unwrap_or(patrol.baseline_presence))
    )
}

fn approximate_patrol_window(window: PatrolWindow) -> String {
    if window.duration_minutes() == 1_440 {
        return format!("all day ({})", police_presence_label(window.presence()));
    }
    let start = rounded_half_hour(window.start().value());
    let end = rounded_half_hour(
        u16::try_from(
            (u32::from(window.start().value()) + u32::from(window.duration_minutes())) % 1_440,
        )
        .expect("patrol window minute remainder must fit u16"),
    );
    format!(
        "roughly {}-{} ({})",
        format_day_minute(start),
        format_day_minute(end),
        police_presence_label(window.presence())
    )
}

fn rounded_half_hour(minute: u16) -> u16 {
    let rounded = (u32::from(minute) + 15) / 30 * 30;
    u16::try_from(rounded % 1_440).expect("rounded day minute must fit u16")
}

fn format_day_minute(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn police_presence_label(rating: Rating) -> &'static str {
    match rating.value() {
        0..=19 => "sparse",
        20..=44 => "light",
        45..=69 => "regular",
        70..=89 => "heavy",
        90..=100 => "concentrated",
        _ => unreachable!(),
    }
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

fn authority_sightline_summary(
    name: &str,
    active_case_against_surveiller: bool,
    outcome: OperationObjectiveOutcome,
) -> String {
    // The observation reports only visible authority activity tied to a case the surveilling
    // organization already knows exists; it never reveals evidence, subjects, or case internals.
    if outcome == OperationObjectiveOutcome::Partial {
        return format!(
            "Visible activity around {name} remained difficult to judge; a dependable read on whether the case is still being actively developed was not established."
        );
    }
    if active_case_against_surveiller {
        format!(
            "Detectives around {name} appear to be actively developing the case connected to your recent activity. The matter has not gone quiet."
        )
    } else {
        format!(
            "No active case machinery connected to your recent activity was observed around {name}; the matter appears to have been shelved and routine police functions continue."
        )
    }
}

fn organization_summary(
    name: &str,
    active_members: &[(CharacterId, String)],
    outcome: OperationObjectiveOutcome,
) -> String {
    let limit = if outcome == OperationObjectiveOutcome::Achieved {
        3
    } else {
        1
    };
    let observed = active_members
        .iter()
        .take(limit)
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

fn investigation_summary(
    title: &str,
    owner_name: &str,
    status: InvestigationStatus,
    lead: Option<&(CharacterId, String)>,
    assigned_count: usize,
    outcome: OperationObjectiveOutcome,
) -> String {
    let lead_clause = if outcome == OperationObjectiveOutcome::Achieved {
        lead.map(|(_, name)| format!(" {name} appears to be directing the visible work."))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let staffing_clause = if outcome == OperationObjectiveOutcome::Achieved && assigned_count > 0 {
        format!(
            " At least {assigned_count} investigator{} are visibly assigned.",
            if assigned_count == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    format!(
        "Visible activity around the {title} file indicates the matter is {} under {owner_name}.{lead_clause}{staffing_clause}",
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
            .map(|record| record.name().to_owned())
            .unwrap_or_else(|| format!("neighborhood {neighborhood}")),
        EnterpriseLocation::Business(business) => state
            .world
            .get_business(business)
            .map(|record| record.name().to_owned())
            .unwrap_or_else(|| format!("business {business}")),
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
        EnterpriseStatus::Closed => "closed",
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::id::EvidenceId;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::core::time::SimDuration;
    use crate::legal::investigation_system::{
        process_cold_case_decay, validate_add_evidence, validate_incident_intake,
        validate_open_investigation,
    };
    use crate::legal::jurisdiction_system::validate_set_jurisdiction;
    use crate::legal::patrol_system::validate_establish_patrol_deployment;
    use crate::legal::{
        Admissibility, DayMinute, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft,
        JurisdictionDraft, PatrolDeploymentDraft, PatrolWindow,
    };
    use crate::operations::operation_execution::{
        calculate_intelligence_factors, decide_operation_resolution,
        validate_operation_resolution_plan, OperationResolutionError,
        OperationResolutionRandomness,
    };
    use crate::operations::operation_system::{validate_authorize_operation, OperationError};
    use crate::operations::{
        OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
    };
    use crate::registry::Registry;
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
        validate_reassign_character,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CapabilityKind, CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        registry: Registry,
        state: AppState,
        crew: OrganizationId,
        observer: CharacterId,
        entry_specialist: CharacterId,
        police: OrganizationId,
        neighborhood: NeighborhoodId,
        business: BusinessId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating should validate")
    }

    fn fixture(observer_skill: u8, with_patrol: bool) -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0x5A11_1933);
        let crew = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Northside Observation Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("crew should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Northside Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Northside Market".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(55),
                        commercial_activity: rating(75),
                        illicit_demand: rating(60),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(55),
                        political_influence: rating(45),
                        social_cohesion: rating(50),
                        visible_violence_tolerance: rating(30),
                    },
                },
            },
        )
        .expect("neighborhood should validate");
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: rating(80),
            },
        )
        .expect("jurisdiction should validate")
        .commit(&mut state)
        .expect("jurisdiction should commit");
        if with_patrol {
            validate_establish_patrol_deployment(
                &state,
                PatrolDeploymentDraft {
                    organization: police,
                    neighborhood,
                    windows: vec![
                        PatrolWindow::try_new(
                            DayMinute::try_new(120).expect("fixture minute should validate"),
                            120,
                            rating(80),
                        )
                        .expect("fixture patrol window should validate"),
                        PatrolWindow::try_new(
                            DayMinute::try_new(1_320).expect("fixture minute should validate"),
                            120,
                            rating(60),
                        )
                        .expect("fixture patrol window should validate"),
                    ],
                },
            )
            .expect("patrol should validate")
            .commit(&mut state)
            .expect("patrol should commit");
        }
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Market Social Club".to_owned(),
                kind: BusinessKind::Hospitality,
                functions: BTreeSet::from([
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::MeetingSpace,
                    BusinessFunction::Warehousing,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("business should validate");
        let observer = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Mara Vale".to_owned(),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Surveillance, rating(observer_skill)),
                    (CapabilityKind::Management, rating(observer_skill)),
                    (CapabilityKind::Stealth, rating(observer_skill)),
                    (CapabilityKind::Burglary, rating(observer_skill)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("observer should validate");
        let entry_specialist = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Nora Quill".to_owned(),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Burglary, rating(observer_skill))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("entry specialist should validate");
        Fixture {
            registry,
            state,
            crew,
            observer,
            entry_specialist,
            police,
            neighborhood,
            business,
        }
    }

    fn authorize_surveillance(fixture: &mut Fixture, target: EntityRef) -> OperationId {
        validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Observe target".to_owned(),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.crew,
                leader: fixture.observer,
                objective: OperationObjective::GatherInformation { target },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("surveillance should validate")
        .commit(&mut fixture.state)
        .expect("surveillance should commit")
    }

    fn resolve_with_zero_variance(fixture: &mut Fixture, operation: OperationId) {
        let start = run_tick(&fixture.registry, &mut fixture.state);
        assert_eq!(start.started_operations, vec![operation]);
        fixture.state.advance_clock(SimDuration::from_minutes(120));
        let plan = decide_operation_resolution(
            &fixture.registry,
            &fixture.state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("due surveillance should produce a resolution plan");
        validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
            .expect("fresh surveillance plan should validate")
            .commit(&mut fixture.state)
            .expect("validated surveillance should commit");
    }

    #[test]
    fn achieved_business_surveillance_creates_actionable_patrol_and_access_intelligence() {
        let mut fixture = fixture(100, true);
        let business = fixture.business;
        let surveillance = authorize_surveillance(&mut fixture, EntityRef::Business(business));
        resolve_with_zero_variance(&mut fixture, surveillance);

        let resolution = fixture
            .state
            .operations()
            .get_operation(surveillance)
            .and_then(|record| record.resolution())
            .expect("surveillance should resolve");
        assert_eq!(
            resolution.objective_outcome(),
            OperationObjectiveOutcome::Achieved
        );
        assert_eq!(resolution.discovered_information().len(), 2);

        let discovered = resolution
            .discovered_information()
            .iter()
            .map(|information| {
                fixture
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("discovered information should persist")
            })
            .collect::<Vec<_>>();
        for information in &discovered {
            assert_eq!(
                information.holder(),
                KnowledgeHolder::Organization(fixture.crew)
            );
            assert_eq!(
                information.source_kind(),
                InformationSourceKind::Surveillance
            );
            assert_eq!(
                information.source_entity(),
                Some(EntityRef::Operation(surveillance))
            );
            assert_eq!(information.reliability(), Reliability::GenerallyReliable);
            assert_eq!(information.specificity(), Specificity::Specific);
            assert_eq!(
                fixture
                    .state
                    .operations()
                    .operation_for_discovered_information(information.id())
                    .map(|record| record.id()),
                Some(surveillance)
            );
        }
        let police = discovered
            .iter()
            .find(|information| information.topic() == InformationTopic::PoliceActivity)
            .expect("business surveillance should discover police activity");
        assert_eq!(
            police.subject(),
            EntityRef::Neighborhood(fixture.neighborhood)
        );
        assert!(police.summary().contains("recurring pattern"));
        assert!(police.summary().contains("roughly 02:00-04:00"));
        assert!(!police.summary().contains("patrol-deployment"));

        let access = discovered
            .iter()
            .find(|information| information.topic() == InformationTopic::MarketAccess)
            .expect("achieved business surveillance should discover venue access");
        assert_eq!(access.subject(), EntityRef::Business(fixture.business));
        assert!(access.summary().contains("regular customer access"));
        assert!(access.summary().contains("private meeting space"));
        assert!(access.summary().contains("storage space"));

        let after_action = fixture
            .state
            .intelligence()
            .get_information(resolution.after_action_information())
            .expect("after-action information should persist");
        assert!(after_action
            .summary()
            .contains("Surveillance produced 2 usable target observations."));

        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("surveillance discoveries should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&fixture.registry, decoded)
            .expect("surveillance discoveries should restore");
        for information in resolution.discovered_information() {
            assert_eq!(
                restored
                    .operations()
                    .operation_for_discovered_information(*information)
                    .map(|record| record.id()),
                Some(surveillance)
            );
        }

        let police_information = police.id();
        let access_information = access.id();
        let burglary = validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Use surveillance for entry planning".to_owned(),
                kind: OperationKind::Burglary,
                responsible_organization: fixture.crew,
                leader: fixture.observer,
                objective: OperationObjective::AcquireProperty {
                    target: EntityRef::Business(fixture.business),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([
                    (RoleKind::Coordinator, fixture.observer),
                    (RoleKind::EntrySpecialist, fixture.entry_specialist),
                ]),
                intelligence: BTreeSet::from([police_information, access_information]),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect("fresh surveillance intelligence should be valid burglary planning input")
        .commit(&mut fixture.state)
        .expect("intelligence-backed burglary should commit");
        let start = run_tick(&fixture.registry, &mut fixture.state);
        assert!(start.started_operations.contains(&burglary));
        let (quality, adjustment, covered, relevant) =
            calculate_intelligence_factors(&fixture.registry, &fixture.state, burglary);
        assert!(quality.value() > 0);
        assert!(adjustment < 0);
        assert!(covered >= 2);
        assert!(covered < relevant);
        validate_state(&fixture.state).expect("surveillance-backed planning state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn partial_and_failed_surveillance_degrade_or_withhold_target_knowledge() {
        let rival_registry = build_registry();
        let mut partial = fixture(35, false);
        let rival = insert_organization(
            &rival_registry,
            &mut partial.state,
            OrganizationDraft {
                name: "Dock Rival".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("rival should validate");
        let target = insert_character(
            &rival_registry,
            &mut partial.state,
            CharacterDraft {
                name: "Nico Hart".to_owned(),
                organization: Some(rival),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("target should validate");
        let operation = authorize_surveillance(&mut partial, EntityRef::Character(target));
        resolve_with_zero_variance(&mut partial, operation);
        let resolution = partial
            .state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("partial surveillance should resolve");
        assert_eq!(
            resolution.objective_outcome(),
            OperationObjectiveOutcome::Partial
        );
        assert_eq!(resolution.discovered_information().len(), 1);
        let information = partial
            .state
            .intelligence()
            .get_information(*resolution.discovered_information().iter().next().unwrap())
            .expect("partial surveillance information should persist");
        assert_eq!(information.reliability(), Reliability::Mixed);
        assert_eq!(information.specificity(), Specificity::General);

        let mut failed = fixture(0, false);
        let failed_target = insert_character(
            &failed.registry,
            &mut failed.state,
            CharacterDraft {
                name: "Unresolved Target".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("failed target should validate");
        let failed_operation =
            authorize_surveillance(&mut failed, EntityRef::Character(failed_target));
        resolve_with_zero_variance(&mut failed, failed_operation);
        let failed_resolution = failed
            .state
            .operations()
            .get_operation(failed_operation)
            .and_then(|record| record.resolution())
            .expect("failed surveillance should resolve");
        assert_eq!(
            failed_resolution.objective_outcome(),
            OperationObjectiveOutcome::Failed
        );
        assert!(failed_resolution.discovered_information().is_empty());
        let after_action = failed
            .state
            .intelligence()
            .get_information(failed_resolution.after_action_information())
            .expect("failed surveillance should still produce after-action information");
        assert!(after_action
            .summary()
            .contains("no target observation reliable enough for planning"));
        validate_state(&partial.state).expect("partial surveillance state should validate");
        validate_state(&failed.state).expect("failed surveillance state should validate");
        validate_invariants(&partial.state);
        validate_invariants(&failed.state);
    }

    #[test]
    fn surveillance_resolution_rejects_target_change_after_planning() {
        let mut fixture = fixture(100, false);
        let rival = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Moving Target Group".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("rival should validate");
        let target = insert_character(
            &fixture.registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Changing Subject".to_owned(),
                organization: Some(rival),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("target should validate");
        let operation = authorize_surveillance(&mut fixture, EntityRef::Character(target));
        let start = run_tick(&fixture.registry, &mut fixture.state);
        assert_eq!(start.started_operations, vec![operation]);
        fixture.state.advance_clock(SimDuration::from_minutes(120));
        let plan = decide_operation_resolution(
            &fixture.registry,
            &fixture.state,
            operation,
            OperationResolutionRandomness::new(0, 0),
        )
        .expect("surveillance plan should resolve against current target state");

        validate_reassign_character(&fixture.state, target, None, None)
            .expect("target reassignment should validate")
            .commit(&mut fixture.state)
            .expect("target reassignment should commit");
        let error = validate_operation_resolution_plan(&fixture.registry, &fixture.state, plan)
            .err()
            .expect("target change must stale surveillance resolution");
        assert_eq!(
            error,
            OperationResolutionError::Surveillance(SurveillanceError::StaleTarget(
                EntityRef::Character(target)
            ))
        );
        assert_eq!(
            fixture
                .state
                .operations()
                .get_operation(operation)
                .expect("stale surveillance should remain present")
                .status(),
            OperationStatus::InProgress
        );
        validate_state(&fixture.state).expect("stale surveillance rejection should preserve state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn investigation_surveillance_reports_visible_case_activity_without_evidence_graph_leakage() {
        let mut fixture = fixture(100, false);
        let suspect = insert_character(
            &fixture.registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Hidden Case Subject".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("suspect should validate");
        let investigation = validate_open_investigation(
            &fixture.state,
            InvestigationDraft {
                owner: fixture.police,
                title: "Harbor Ledger Inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(suspect)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut fixture.state)
        .expect("investigation should commit");
        validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation,
                custodian: fixture.police,
                subject: EntityRef::Character(suspect),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("hidden case evidence should validate")
        .commit(&mut fixture.state)
        .expect("hidden case evidence should commit");
        let operation =
            authorize_surveillance(&mut fixture, EntityRef::Investigation(investigation));
        resolve_with_zero_variance(&mut fixture, operation);
        let resolution = fixture
            .state
            .operations()
            .get_operation(operation)
            .and_then(|record| record.resolution())
            .expect("investigation surveillance should resolve");
        assert_eq!(resolution.discovered_information().len(), 1);
        let information = fixture
            .state
            .intelligence()
            .get_information(*resolution.discovered_information().iter().next().unwrap())
            .expect("legal-activity observation should persist");
        assert_eq!(information.topic(), InformationTopic::LegalActivity);
        assert_eq!(
            information.subject(),
            EntityRef::Investigation(investigation)
        );
        assert!(information.summary().contains("Harbor Ledger Inquiry"));
        assert!(information.summary().contains("Northside Precinct"));
        assert!(!information.summary().contains("Hidden Case Subject"));
        assert!(!information.summary().contains("Document"));
        validate_state(&fixture.state).expect("investigation surveillance state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn law_enforcement_org_surveillance_reports_case_heat_and_shelved_close_without_leaks() {
        let mut fixture = fixture(100, false);
        let business = fixture.business;
        let police = fixture.police;
        let incident = authorize_surveillance(&mut fixture, EntityRef::Business(business));
        // Resolve the originating surveillance to terminal state so it does not also start when
        // the later re-check surveillance run_tick fires; the fixture has no patrol deployment, so
        // its resolution creates no exposure case.
        resolve_with_zero_variance(&mut fixture, incident);
        let case = validate_incident_intake(
            &fixture.state,
            IncidentIntakeDraft {
                owner: fixture.police,
                title: "Crew Incident Inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Operation(incident)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Operation(incident),
                    origin: Some(EntityRef::Operation(incident)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: fixture.state.now(),
                }],
                origin_operation: Some(incident),
                notified_organizations: BTreeSet::from([fixture.crew]),
            },
        )
        .expect("incident intake should validate")
        .commit(&mut fixture.state)
        .expect("incident intake should commit")
        .investigation;

        // While the case is active, police-organization surveillance reports the case heat
        // without revealing the evidence graph or internal case details.
        let hot_surveillance =
            authorize_surveillance(&mut fixture, EntityRef::Organization(police));
        resolve_with_zero_variance(&mut fixture, hot_surveillance);
        let hot_resolution = fixture
            .state
            .operations()
            .get_operation(hot_surveillance)
            .and_then(|record| record.resolution())
            .expect("hot surveillance should resolve");
        assert_eq!(hot_resolution.discovered_information().len(), 1);
        let hot_observation = fixture
            .state
            .intelligence()
            .get_information(
                *hot_resolution
                    .discovered_information()
                    .iter()
                    .next()
                    .unwrap(),
            )
            .expect("case-heat observation should persist");
        assert_eq!(hot_observation.topic(), InformationTopic::LegalActivity);
        assert_eq!(hot_observation.subject(), EntityRef::Organization(police));
        assert!(hot_observation
            .summary()
            .contains("actively developing the case"));
        assert!(!hot_observation.summary().contains("Crew Incident Inquiry"));
        assert!(!hot_observation.summary().contains("Surveillance"));

        // A passing of the authored cold window deterministically shelves the case, and a fresh
        // police-organization surveillance then reports the matter has gone quiet.
        fixture.state.advance_clock(SimDuration::from_minutes(121));
        let suspended = process_cold_case_decay(&mut fixture.state, SimDuration::from_minutes(120))
            .expect("cold-case decay should resolve");
        assert_eq!(suspended, vec![case]);
        assert_eq!(
            fixture
                .state
                .legal()
                .get_investigation(case)
                .expect("cold case should persist")
                .status(),
            InvestigationStatus::Suspended
        );
        validate_state(&fixture.state).expect("cold-case decay state should validate");
        validate_invariants(&fixture.state);

        let cold_surveillance =
            authorize_surveillance(&mut fixture, EntityRef::Organization(police));
        resolve_with_zero_variance(&mut fixture, cold_surveillance);
        let cold_resolution = fixture
            .state
            .operations()
            .get_operation(cold_surveillance)
            .and_then(|record| record.resolution())
            .expect("recheck surveillance should resolve");
        let cold_observation = fixture
            .state
            .intelligence()
            .get_information(
                *cold_resolution
                    .discovered_information()
                    .iter()
                    .next()
                    .unwrap(),
            )
            .expect("shelved observation should persist");
        assert!(cold_observation.summary().contains("shelved"));
        assert!(!cold_observation
            .summary()
            .contains("actively developing the case"));
        validate_state(&fixture.state).expect("shelved recheck state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn surveillance_authorization_rejects_semantically_invalid_objectives_and_targets() {
        let fixture = fixture(80, false);
        let invalid_objective = validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Not actually surveillance".to_owned(),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.crew,
                leader: fixture.observer,
                objective: OperationObjective::Frighten {
                    target: EntityRef::Business(fixture.business),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect_err("surveillance must require a gather-information objective");
        assert_eq!(
            invalid_objective,
            OperationError::InvalidSurveillanceObjective
        );

        let evidence = EntityRef::Evidence(EvidenceId::from_raw(9_999));
        let unsupported = validate_authorize_operation(
            &fixture.registry,
            &fixture.state,
            OperationDraft {
                title: "Observe evidence record".to_owned(),
                kind: OperationKind::Surveillance,
                responsible_organization: fixture.crew,
                leader: fixture.observer,
                objective: OperationObjective::GatherInformation { target: evidence },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, fixture.observer)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: fixture.state.now() + SimDuration::ONE_MINUTE,
            },
        )
        .expect_err("evidence records are not directly observable operation targets");
        assert_eq!(
            unsupported,
            OperationError::UnsupportedSurveillanceTarget(evidence)
        );
        assert_eq!(
            fixture
                .state
                .operations()
                .operations_for_organization(fixture.crew)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }
}
