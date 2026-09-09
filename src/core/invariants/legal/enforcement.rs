//! Institutional-enforcement validation: jurisdictions, patrol deployments, and dispatched responses.

use crate::core::id::{NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::jurisdiction_system::police_response_jurisdiction_snapshot_is_possible;
use crate::legal::patrol_system::{
    is_canonical_patrol_schedule, police_response_patrol_snapshot_is_possible,
};
use crate::legal::{
    PatrolDeploymentRecord, PatrolDeploymentStatus, PoliceResponseRecord, PoliceResponseStatus,
};
use crate::world::OrganizationKind;
use std::collections::BTreeMap;

type PatrolAssignment = (OrganizationId, NeighborhoodId);
type PatrolActiveInterval = (SimTime, Option<SimTime>, PatrolDeploymentId);

pub(super) fn validate_jurisdictions(state: &AppState) -> Result<(), StateValidationError> {
    for jurisdiction in state.legal.jurisdictions() {
        let organization = state
            .world
            .get_organization(jurisdiction.organization())
            .ok_or(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            })?;
        if !matches!(
            organization.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) || !has_valid_jurisdiction_history(state, jurisdiction)
        {
            return Err(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            });
        }
    }

    Ok(())
}

fn has_valid_jurisdiction_history(
    state: &AppState,
    jurisdiction: &crate::legal::JurisdictionRecord,
) -> bool {
    let revisions = jurisdiction.revisions();
    if revisions.is_empty() {
        return false;
    }
    for (index, revision) in revisions.iter().enumerate() {
        let Ok(expected_version) = u32::try_from(index + 1) else {
            return false;
        };
        if revision.version() != expected_version
            || revision.changed_at() > state.now()
            || revision.neighborhoods().is_empty()
            || revision
                .neighborhoods()
                .iter()
                .any(|neighborhood| state.world.get_neighborhood(*neighborhood).is_none())
        {
            return false;
        }
        let Some(previous) = index.checked_sub(1).and_then(|index| revisions.get(index)) else {
            continue;
        };
        if revision.changed_at() < previous.changed_at()
            || (revision.neighborhoods() == previous.neighborhoods()
                && revision.case_intake_priority() == previous.case_intake_priority())
        {
            return false;
        }
    }
    true
}

pub(super) fn validate_patrol_deployments(state: &AppState) -> Result<(), StateValidationError> {
    for deployment in state.legal.patrol_deployments() {
        let authority = state
            .world
            .get_organization(deployment.organization())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        let _ = state
            .world
            .get_neighborhood(deployment.neighborhood())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || !has_valid_patrol_history(state, deployment)
        {
            return Err(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            });
        }
        match deployment.status() {
            PatrolDeploymentStatus::Active => {
                let jurisdiction = state.legal.get_jurisdiction(deployment.organization());
                if jurisdiction.is_none_or(|record| {
                    !record.neighborhoods().contains(&deployment.neighborhood())
                }) || state
                    .legal
                    .active_patrol_for(deployment.organization(), deployment.neighborhood())
                    .is_none_or(|record| record.id() != deployment.id())
                {
                    return Err(StateValidationError::InvalidPatrolDeployment {
                        deployment: deployment.id(),
                    });
                }
            }
            PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired => {}
        }
    }

    validate_patrol_history_exclusivity(state)?;

    Ok(())
}

fn validate_patrol_history_exclusivity(state: &AppState) -> Result<(), StateValidationError> {
    let mut intervals_by_assignment: BTreeMap<PatrolAssignment, Vec<PatrolActiveInterval>> =
        BTreeMap::new();
    for deployment in state.legal.patrol_deployments() {
        for (start, end) in active_patrol_intervals(deployment) {
            intervals_by_assignment
                .entry((deployment.organization(), deployment.neighborhood()))
                .or_default()
                .push((start, end, deployment.id()));
        }
    }

    for intervals in intervals_by_assignment.values_mut() {
        intervals.sort_by_key(|(start, _, deployment)| (*start, *deployment));
        let mut previous_end: Option<Option<SimTime>> = None;
        for (start, end, deployment) in intervals.iter().copied() {
            if let Some(prior_end) = previous_end
                && prior_end.is_none_or(|prior_end| start < prior_end)
            {
                return Err(StateValidationError::InvalidPatrolDeployment { deployment });
            }
            previous_end = Some(end);
        }
    }
    Ok(())
}

fn active_patrol_intervals(deployment: &PatrolDeploymentRecord) -> Vec<(SimTime, Option<SimTime>)> {
    let mut intervals = Vec::new();
    let mut active_start = None;
    for revision in deployment.revisions() {
        match (active_start, revision.status()) {
            (None, PatrolDeploymentStatus::Active) => {
                active_start = Some(revision.changed_at());
            }
            (Some(start), PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired) => {
                if start < revision.changed_at() {
                    intervals.push((start, Some(revision.changed_at())));
                }
                active_start = None;
            }
            (Some(_), PatrolDeploymentStatus::Active)
            | (None, PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired) => {}
        }
    }
    if let Some(start) = active_start {
        intervals.push((start, None));
    }
    intervals
}

fn has_valid_patrol_history(
    state: &AppState,
    deployment: &crate::legal::PatrolDeploymentRecord,
) -> bool {
    let revisions = deployment.revisions();
    let Some(first) = revisions.first() else {
        return false;
    };
    if first.changed_at() != deployment.established_at()
        || first.status() != PatrolDeploymentStatus::Active
        || first.version() != 1
    {
        return false;
    }
    for (index, revision) in revisions.iter().enumerate() {
        let Ok(expected_version) = u32::try_from(index + 1) else {
            return false;
        };
        if revision.version() != expected_version
            || revision.changed_at() > state.now()
            || !is_canonical_patrol_schedule(revision.windows())
        {
            return false;
        }
        let Some(previous) = index.checked_sub(1).and_then(|index| revisions.get(index)) else {
            continue;
        };
        if revision.changed_at() < previous.changed_at()
            || previous.status() == PatrolDeploymentStatus::Retired
        {
            return false;
        }
        if revision.status() == previous.status() {
            if revision.windows() == previous.windows() {
                return false;
            }
        } else if revision.windows() != previous.windows()
            || !matches!(
                (previous.status(), revision.status()),
                (
                    PatrolDeploymentStatus::Active,
                    PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired
                ) | (
                    PatrolDeploymentStatus::Suspended,
                    PatrolDeploymentStatus::Active | PatrolDeploymentStatus::Retired
                )
            )
        {
            return false;
        }
    }
    true
}

pub(super) fn validate_police_responses(state: &AppState) -> Result<(), StateValidationError> {
    for response in state.legal.police_responses() {
        validate_police_response(state, response)?;
    }

    Ok(())
}

fn validate_police_response(
    state: &AppState,
    response: &PoliceResponseRecord,
) -> Result<(), StateValidationError> {
    validate_police_response_definition(state, response)?;
    validate_police_response_links(state, response)?;
    validate_police_response_patrol(state, response)?;
    validate_police_response_lifecycle(state, response)
}

fn validate_police_response_definition(
    state: &AppState,
    response: &PoliceResponseRecord,
) -> Result<(), StateValidationError> {
    let authority = state
        .world
        .get_organization(response.authority())
        .ok_or_else(|| invalid_police_response(response))?;
    if authority.kind() != OrganizationKind::LawEnforcement
        || state
            .world
            .get_neighborhood(response.neighborhood())
            .is_none()
        || response.version() == 0
        || response.dispatched_at() >= response.arrival_due_at()
        || response.dispatched_at() > state.now()
    {
        return Err(invalid_police_response(response));
    }
    Ok(())
}

fn validate_police_response_links(
    state: &AppState,
    response: &PoliceResponseRecord,
) -> Result<(), StateValidationError> {
    let operation = state
        .operations
        .get_operation(response.source_operation())
        .ok_or_else(|| invalid_police_response(response))?;
    if operation.police_response() != Some(response.id())
        || operation.started_at() != Some(response.dispatched_at())
        || response.jurisdiction_version() == 0
        || !police_response_jurisdiction_snapshot_is_possible(
            state,
            response.authority(),
            response.neighborhood(),
            response.dispatched_at(),
            response.jurisdiction_version(),
        )
    {
        return Err(invalid_police_response(response));
    }
    Ok(())
}

fn validate_police_response_patrol(
    state: &AppState,
    response: &PoliceResponseRecord,
) -> Result<(), StateValidationError> {
    if response
        .patrol()
        .is_some_and(|patrol| patrol.version() == 0)
        || !police_response_patrol_snapshot_is_possible(
            state,
            response.authority(),
            response.neighborhood(),
            response.dispatched_at(),
            response.patrol(),
            response.response_presence(),
        )
    {
        return Err(invalid_police_response(response));
    }
    Ok(())
}

fn validate_police_response_lifecycle(
    state: &AppState,
    response: &PoliceResponseRecord,
) -> Result<(), StateValidationError> {
    let valid = match response.status() {
        PoliceResponseStatus::Dispatched => {
            response.arrived_at().is_none() && response.version() == 1
        }
        PoliceResponseStatus::Arrived => {
            response.arrived_at().is_some_and(|arrived_at| {
                arrived_at >= response.arrival_due_at() && arrived_at <= state.now()
            }) && response.version() == 2
        }
    };
    if !valid {
        return Err(invalid_police_response(response));
    }
    Ok(())
}

fn invalid_police_response(response: &PoliceResponseRecord) -> StateValidationError {
    StateValidationError::InvalidPoliceResponse {
        response: response.id(),
    }
}
