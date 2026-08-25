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

    /// The (topic, subject) pairs this plan will persist — the frozen signature set recorded
    /// on the operation's resolution.
    pub(crate) fn surveillance_signatures(&self) -> BTreeSet<(InformationTopic, EntityRef)> {
        self.observations
            .iter()
            .map(|observation| (observation.topic, observation.subject))
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
    let snapshot = resolve_target_snapshot(state, *target, observed_at, surveiller)?;
    let observations = build_observations(&snapshot, outcome, observed_at);
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
    let findings = plan.observation_findings().collect::<Vec<_>>().join("; ");
    let clause = match outcome {
    OperationObjectiveOutcome::Achieved => format!(
      "Surveillance produced {} usable target observation{}{}.",
      plan.observation_count(),
      if plan.observation_count() == 1 { "" } else { "s" },
      if findings.is_empty() {
        String::new()
      } else {
        format!(": {findings}")
      }
    ),
    OperationObjectiveOutcome::Partial => format!(
      "Surveillance produced {} limited target observation{}; important details remain unresolved.{}",
      plan.observation_count(),
      if plan.observation_count() == 1 { "" } else { "s" },
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
    // One source of truth for the target→(topic, subject) table: the resolution record froze
    // the signatures this operation produced, so persisted surveillance intelligence is valid
    // exactly when its signature is in that set. Re-deriving the expectation from current state
    // would let later changes (for example a case notified to the surveiller after resolution)
    // silently invalidate honestly-produced intelligence.
    resolution
        .surveillance_signatures()
        .contains(&(information.topic(), information.subject()))
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
                .map(|character| (character.id(), character.name().to_owned()))
                .collect();
            let law_enforcement_sightline = if is_law_enforcement_authority(organization.kind()) {
                // A sightline exists only once this authority has surfaced an operation-
                // originated case to the surveiller; before that there is nothing to re-read,
                // so surveillance falls back to ordinary personnel observation instead of
                // fabricating a "shelved" read about a case that never touched this organization.
                state
                    .legal
                    .investigations_for_owner(id)
                    .any(|case| case.notified_organizations().contains(&surveiller))
                    .then(|| LawEnforcementCaseSightline {
                        // "Still being worked" while any known case is active: that is the
                        // player-relevant heat signal, and it never reveals evidence, subjects,
                        // or internal case details.
                        active_case_against_surveiller: state
                            .legal
                            .investigations_for_owner(id)
                            .any(|case| {
                                case.status() == InvestigationStatus::Active
                                    && case.notified_organizations().contains(&surveiller)
                            }),
                    })
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
            // evidence graph and named subjects are never read here. This deliberately
            // differs from the organization sightline above, which summarizes an authority's
            // whole (mostly hidden) caseload and therefore requires prior notification
            // before it may claim any case-activity read at all.
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
                summary: patrol_summary(name, patrol, outcome, observed_at),
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
                summary: patrol_summary(neighborhood_name, patrol, outcome, observed_at),
                finding: format!("police activity around {neighborhood_name}"),
            }];
            if outcome == OperationObjectiveOutcome::Achieved {
                observations.push(SurveillanceObservation {
                    topic: InformationTopic::MarketAccess,
                    subject: EntityRef::Business(*id),
                    reliability,
                    specificity,
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
            summary: character_summary(name, organization.as_ref(), supervisor.as_ref()),
            finding: format!("the movements of {name}"),
        }],
        SurveillanceTargetSnapshot::Organization {
            id,
            name,
            active_members,
            law_enforcement_sightline,
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
                finding: format!("case activity at {name}"),
            }],
            None => vec![SurveillanceObservation {
                topic: InformationTopic::Personnel,
                subject: EntityRef::Organization(*id),
                reliability,
                specificity,
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
    use crate::legal::case_knowledge::CaseActivityStatus;
    // The observation reports only visible authority activity tied to a case the surveilling
    // organization already knows exists; it never reveals evidence, subjects, or case internals.
    if outcome == OperationObjectiveOutcome::Partial {
        return format!(
      "Visible activity around {name} remained difficult to judge; a dependable read on whether the case is still being actively developed was not established."
    );
    }
    // Dependable reads lead with the shared anchored activity marker so player-facing
    // parsers read the sightline without hidden state and free text cannot spoof the parse.
    let (status, prose) = if active_case_against_surveiller {
        (
            CaseActivityStatus::Active,
            format!(
        "Detectives around {name} appear to be actively developing the case connected to your recent activity. The matter has not gone quiet."
      ),
        )
    } else {
        (
            CaseActivityStatus::Shelved,
            format!(
        "No active case machinery connected to your recent activity was observed around {name}; the matter appears to have been shelved and routine police functions continue."
      ),
        )
    };
    format!("{} {prose}", status.marker())
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
