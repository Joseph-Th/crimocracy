//! Geographic legal authority assignments and deterministic incident intake routing.

use crate::core::id::{NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::legal::{JurisdictionDraft, JurisdictionRecord, JurisdictionRevision};
use crate::world::OrganizationKind;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum JurisdictionError {
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} cannot hold law-enforcement jurisdiction")]
    InvalidAuthorityKind(OrganizationId),
    #[error("jurisdiction must contain at least one neighborhood")]
    EmptyJurisdiction,
    #[error("jurisdiction for organization {0} already has the requested assignment")]
    JurisdictionUnchanged(OrganizationId),
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
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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
        let previous_version = self.expected_version.unwrap_or(0);
        ensure_version_can_advance(previous_version, "jurisdiction")?;
        let organization = self.draft.organization;
        state.legal.set_jurisdiction(
            organization,
            self.draft.neighborhoods,
            self.draft.case_intake_priority,
            state.now(),
        );
        Ok(organization)
    }
}

pub fn validate_set_jurisdiction(
    state: &AppState,
    draft: JurisdictionDraft,
) -> Result<ValidatedJurisdiction, JurisdictionError> {
    validate_jurisdiction_dependencies(state, &draft)?;
    let current = state.legal.get_jurisdiction(draft.organization);
    if current.is_some_and(|record| {
        record.neighborhoods() == &draft.neighborhoods
            && record.case_intake_priority() == draft.case_intake_priority
    }) {
        return Err(JurisdictionError::JurisdictionUnchanged(draft.organization));
    }
    let expected_version = current.map(JurisdictionRecord::version);
    ensure_version_can_advance(expected_version.unwrap_or(0), "jurisdiction")?;
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
                .is_some_and(|organization| kinds.contains(&organization.kind()))
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

/// The authority that originates casework in a neighborhood. Case ownership is deliberately
/// law-enforcement-only: every downstream lifecycle gate (autonomous evidence arrests,
/// cold-case closure, lead-knowledge recording) is defined against police institutions, so
/// letting another authority kind take intake would strand its cases outside every one of
/// those paths.
pub fn resolve_case_intake_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    resolve_jurisdiction_priority(state, neighborhood, &[OrganizationKind::LawEnforcement])
}

/// Versioned snapshot of deterministic police case-intake routing. Systems that plan an
/// incident and commit it later must pin both the winning authority and that authority's
/// jurisdiction version: priority can change because another jurisdiction appears, while an
/// unchanged winner can still have its own jurisdiction record edited between phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaseIntakeAuthoritySnapshot {
    pub(crate) neighborhood: NeighborhoodId,
    pub(crate) organization: Option<OrganizationId>,
    pub(crate) jurisdiction_version: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseIntakeAuthoritySnapshotError {
    Routing {
        neighborhood: NeighborhoodId,
        expected: Option<OrganizationId>,
        found: Option<OrganizationId>,
    },
    JurisdictionVersion {
        neighborhood: NeighborhoodId,
        organization: OrganizationId,
        expected_version: u32,
        found_version: Option<u32>,
    },
}

pub(crate) fn resolve_case_intake_authority_snapshot(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> CaseIntakeAuthoritySnapshot {
    let organization = resolve_case_intake_authority(state, neighborhood);
    let jurisdiction_version = organization.map(|organization| {
        state
            .legal
            .get_jurisdiction(organization)
            .expect("resolved case-intake authority must have a jurisdiction record")
            .version()
    });
    CaseIntakeAuthoritySnapshot {
        neighborhood,
        organization,
        jurisdiction_version,
    }
}

pub(crate) fn validate_case_intake_authority_snapshot(
    state: &AppState,
    snapshot: CaseIntakeAuthoritySnapshot,
) -> Result<(), CaseIntakeAuthoritySnapshotError> {
    let found = resolve_case_intake_authority(state, snapshot.neighborhood);
    if found != snapshot.organization {
        return Err(CaseIntakeAuthoritySnapshotError::Routing {
            neighborhood: snapshot.neighborhood,
            expected: snapshot.organization,
            found,
        });
    }
    if let Some(organization) = snapshot.organization {
        let found_version = state
            .legal
            .get_jurisdiction(organization)
            .map(JurisdictionRecord::version);
        let expected_version = snapshot
            .jurisdiction_version
            .expect("routed case-intake snapshot must contain a jurisdiction version");
        if found_version != Some(expected_version) {
            return Err(CaseIntakeAuthoritySnapshotError::JurisdictionVersion {
                neighborhood: snapshot.neighborhood,
                organization,
                expected_version,
                found_version,
            });
        }
    }
    Ok(())
}

pub fn resolve_police_response_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    resolve_jurisdiction_priority(state, neighborhood, &[OrganizationKind::LawEnforcement])
}

/// Whether a persisted police-response jurisdiction snapshot could have been the winning
/// law-enforcement route at `at`. Jurisdiction revisions within one organization are ordered,
/// but cross-organization mutation order inside one `SimTime` is not persisted. The response is
/// therefore valid when its exact revision is reachable at that minute and every competing
/// authority has at least one same-minute-reachable state that does not outrank it.
pub(crate) fn police_response_jurisdiction_snapshot_is_possible(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    at: crate::core::time::SimTime,
    version: u32,
) -> bool {
    let Some(record) = state.legal.get_jurisdiction(organization) else {
        return false;
    };
    let Some(target) = record.revision_by_version(version) else {
        return false;
    };
    if !jurisdiction_revision_is_possible_at(record, target, at)
        || !target.neighborhoods().contains(&neighborhood)
    {
        return false;
    }
    let target_priority = target.case_intake_priority();

    state.legal.jurisdictions().all(|other| {
        if other.organization() == organization
            || !state
                .world
                .get_organization(other.organization())
                .is_some_and(|authority| authority.kind() == OrganizationKind::LawEnforcement)
        {
            return true;
        }
        reachable_jurisdiction_revisions_at(other, at).any(|candidate| {
            candidate.is_none_or(|candidate| {
                !candidate.neighborhoods().contains(&neighborhood)
                    || candidate.case_intake_priority().value() < target_priority.value()
                    || (candidate.case_intake_priority() == target_priority
                        && other.organization() > organization)
            })
        })
    })
}

fn jurisdiction_revision_is_possible_at(
    record: &JurisdictionRecord,
    revision: &JurisdictionRevision,
    at: crate::core::time::SimTime,
) -> bool {
    revision.changed_at() <= at
        && record
            .revisions()
            .iter()
            .filter(|later| later.version() > revision.version())
            .all(|later| later.changed_at() >= at)
}

fn reachable_jurisdiction_revisions_at(
    record: &JurisdictionRecord,
    at: crate::core::time::SimTime,
) -> impl Iterator<Item = Option<&JurisdictionRevision>> {
    let absent = record
        .revisions()
        .first()
        .is_some_and(|first| first.changed_at() >= at)
        .then_some(None);
    let before = record
        .revisions()
        .iter()
        .rev()
        .find(|revision| revision.changed_at() < at)
        .map(Some);
    absent.into_iter().chain(before).chain(
        record
            .revisions()
            .iter()
            .filter(move |revision| revision.changed_at() == at)
            .map(Some),
    )
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
    if draft.neighborhoods.is_empty() {
        return Err(JurisdictionError::EmptyJurisdiction);
    }
    for neighborhood in &draft.neighborhoods {
        if state.world.get_neighborhood(*neighborhood).is_none() {
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
