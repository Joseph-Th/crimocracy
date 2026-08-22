//! Geographic legal authority assignments and deterministic incident intake routing.

use crate::core::id::{NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::legal::{JurisdictionDraft, JurisdictionRecord};
use crate::world::{Lifecycle, OrganizationKind};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum JurisdictionError {
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} cannot hold law-enforcement jurisdiction")]
    InvalidAuthorityKind(OrganizationId),
    #[error("organization {0} is not active")]
    InactiveAuthority(OrganizationId),
    #[error("jurisdiction must contain at least one neighborhood")]
    EmptyJurisdiction,
    #[error("neighborhood {0} does not exist or is not active")]
    MissingNeighborhood(NeighborhoodId),
    #[error(
        "organization {organization} cannot remove neighborhood {neighborhood} from jurisdiction while patrol deployment {deployment} is active"
    )]
    ActivePatrolDeployment {
        organization: OrganizationId,
        neighborhood: NeighborhoodId,
        deployment: PatrolDeploymentId,
    },
    #[error(
        "jurisdiction for organization {organization} changed after validation; expected version {expected:?}, found {found:?}"
    )]
    StaleJurisdiction {
        organization: OrganizationId,
        expected: Option<u32>,
        found: Option<u32>,
    },
}

#[derive(Debug)]
pub struct ValidatedJurisdiction {
    draft: JurisdictionDraft,
    expected_version: Option<u32>,
}

impl ValidatedJurisdiction {
    pub fn commit(self, state: &mut AppState) -> Result<OrganizationId, JurisdictionError> {
        let found_version = state
            .legal
            .get_jurisdiction(self.draft.organization)
            .map(JurisdictionRecord::version);
        if found_version != self.expected_version {
            return Err(JurisdictionError::StaleJurisdiction {
                organization: self.draft.organization,
                expected: self.expected_version,
                found: found_version,
            });
        }
        validate_jurisdiction_dependencies(state, &self.draft)?;
        let version = self
            .expected_version
            .unwrap_or(0)
            .checked_add(1)
            .expect("jurisdiction version counter exhausted");
        let organization = self.draft.organization;
        state.legal.set_jurisdiction(JurisdictionRecord {
            organization,
            neighborhoods: self.draft.neighborhoods,
            case_intake_priority: self.draft.case_intake_priority,
            version,
        });
        Ok(organization)
    }
}

pub fn validate_set_jurisdiction(
    state: &AppState,
    draft: JurisdictionDraft,
) -> Result<ValidatedJurisdiction, JurisdictionError> {
    validate_jurisdiction_dependencies(state, &draft)?;
    let expected_version = state
        .legal
        .get_jurisdiction(draft.organization)
        .map(JurisdictionRecord::version);
    Ok(ValidatedJurisdiction {
        draft,
        expected_version,
    })
}

/// Highest-priority active authority over a neighborhood whose kind is in `kinds`, with a
/// deterministic organization-ID tie-break.
fn resolve_jurisdiction_priority(
    state: &AppState,
    neighborhood: NeighborhoodId,
    kinds: &[OrganizationKind],
) -> Option<OrganizationId> {
    state
        .legal
        .jurisdictions_for_neighborhood(neighborhood)
        .filter(|jurisdiction| {
            state
                .world
                .get_organization(jurisdiction.organization())
                .is_some_and(|organization| {
                    organization.lifecycle() == Lifecycle::Active
                        && kinds.contains(&organization.kind())
                })
        })
        .fold(None, |best, jurisdiction| match best {
            None => Some(jurisdiction),
            Some(current)
                if jurisdiction.case_intake_priority().value()
                    > current.case_intake_priority().value()
                    || (jurisdiction.case_intake_priority() == current.case_intake_priority()
                        && jurisdiction.organization() < current.organization()) =>
            {
                Some(jurisdiction)
            }
            Some(current) => Some(current),
        })
        .map(JurisdictionRecord::organization)
}

pub fn resolve_case_intake_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    resolve_jurisdiction_priority(
        state,
        neighborhood,
        &[
            OrganizationKind::LawEnforcement,
            OrganizationKind::LegalAuthority,
        ],
    )
}

pub fn resolve_police_response_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    resolve_jurisdiction_priority(state, neighborhood, &[OrganizationKind::LawEnforcement])
}

fn validate_jurisdiction_dependencies(
    state: &AppState,
    draft: &JurisdictionDraft,
) -> Result<(), JurisdictionError> {
    let organization = state
        .world
        .get_organization(draft.organization)
        .ok_or(JurisdictionError::MissingOrganization(draft.organization))?;
    match organization.kind() {
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority => {}
        OrganizationKind::Criminal
        | OrganizationKind::LegalServices
        | OrganizationKind::Prosecutor
        | OrganizationKind::Political
        | OrganizationKind::Press
        | OrganizationKind::Labor
        | OrganizationKind::Civic
        | OrganizationKind::Commercial => {
            return Err(JurisdictionError::InvalidAuthorityKind(draft.organization));
        }
    }
    if organization.lifecycle() != Lifecycle::Active {
        return Err(JurisdictionError::InactiveAuthority(draft.organization));
    }
    if draft.neighborhoods.is_empty() {
        return Err(JurisdictionError::EmptyJurisdiction);
    }
    for neighborhood in &draft.neighborhoods {
        if !state
            .world
            .get_neighborhood(*neighborhood)
            .is_some_and(|record| record.lifecycle() == Lifecycle::Active)
        {
            return Err(JurisdictionError::MissingNeighborhood(*neighborhood));
        }
    }
    if let Some(current) = state.legal.get_jurisdiction(draft.organization) {
        for neighborhood in current.neighborhoods().difference(&draft.neighborhoods) {
            if let Some(deployment) = state
                .legal
                .active_patrol_for(draft.organization, *neighborhood)
            {
                return Err(JurisdictionError::ActivePatrolDeployment {
                    organization: draft.organization,
                    neighborhood: *neighborhood,
                    deployment: deployment.id(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
