//! Institutional-enforcement validation: jurisdictions, patrol deployments, and dispatched responses.

//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::legal::patrol_system::is_canonical_patrol_schedule;
use crate::legal::{PatrolDeploymentStatus, PoliceResponseStatus};
use crate::world::OrganizationKind;

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
        ) || jurisdiction.neighborhoods().is_empty()
            || jurisdiction.version() == 0
            || jurisdiction
                .neighborhoods()
                .iter()
                .any(|neighborhood| state.world.get_neighborhood(*neighborhood).is_none())
        {
            return Err(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            });
        }
    }

    Ok(())
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
            || deployment.version() == 0
            || deployment.established_at() > deployment.last_changed_at()
            || deployment.last_changed_at() > state.now()
            || !is_canonical_patrol_schedule(deployment.windows())
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

    Ok(())
}

pub(super) fn validate_police_responses(state: &AppState) -> Result<(), StateValidationError> {
    for response in state.legal.police_responses() {
        let authority = state.world.get_organization(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || state
                .world
                .get_neighborhood(response.neighborhood())
                .is_none()
            || response.version() == 0
            || response.dispatched_at() >= response.arrival_due_at()
            || response.dispatched_at() > state.now()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        let operation = state
            .operations
            .get_operation(response.source_operation())
            .ok_or(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            })?;
        let jurisdiction = state.legal.get_jurisdiction(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if operation.police_response() != Some(response.id())
            || operation.started_at() != Some(response.dispatched_at())
            || response.jurisdiction_version() == 0
            || response.jurisdiction_version() > jurisdiction.version()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        if let Some(patrol) = response.patrol() {
            let deployment = state
                .legal
                .get_patrol_deployment(patrol.deployment())
                .ok_or(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                })?;
            if patrol.version() == 0
                || patrol.version() > deployment.version()
                || deployment.organization() != response.authority()
                || deployment.neighborhood() != response.neighborhood()
            {
                return Err(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                });
            }
        }
        match response.status() {
            PoliceResponseStatus::Dispatched => {
                if response.arrived_at().is_some() || response.version() != 1 {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
            PoliceResponseStatus::Arrived => {
                if response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at < response.arrival_due_at() || arrived_at > state.now()
                }) || response.version() < 2
                {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
        }
    }

    Ok(())
}
