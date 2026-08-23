//! Canonical patrol deployment validation, lifecycle transitions, and time-of-day presence queries.

use crate::core::id::{IdExhaustionError, NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::{
    DayMinute, PatrolDeploymentDraft, PatrolDeploymentRecord, PatrolDeploymentStatus, PatrolWindow,
};
use crate::world::{OrganizationKind, Rating};
use std::collections::BTreeMap;
use thiserror::Error;

const MINUTES_PER_DAY: u16 = 1_440;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatrolDeploymentTransition {
    Suspend,
    Resume,
    Retire,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PatrolError {
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} cannot deploy law-enforcement patrols")]
    InvalidAuthorityKind(OrganizationId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("organization {0} has no legal jurisdiction record")]
    MissingJurisdiction(OrganizationId),
    #[error("organization {organization} has no jurisdiction over neighborhood {neighborhood}")]
    OutsideJurisdiction {
        organization: OrganizationId,
        neighborhood: NeighborhoodId,
    },
    #[error("patrol deployment must contain at least one daily patrol window")]
    EmptySchedule,
    #[error("patrol windows overlap at minute {minute:?} of the simulation day")]
    OverlappingWindow { minute: DayMinute },
    #[error(
    "organization {organization} already has active patrol deployment {existing} in neighborhood {neighborhood}"
  )]
    DuplicateActiveDeployment {
        organization: OrganizationId,
        neighborhood: NeighborhoodId,
        existing: PatrolDeploymentId,
    },
    #[error("patrol deployment {0} does not exist")]
    MissingDeployment(PatrolDeploymentId),
    #[error("retired patrol deployment {0} cannot be revised")]
    RetiredDeployment(PatrolDeploymentId),
    #[error(
    "patrol deployment {deployment} in status {status:?} cannot apply transition {transition:?}"
  )]
    InvalidTransition {
        deployment: PatrolDeploymentId,
        status: PatrolDeploymentStatus,
        transition: PatrolDeploymentTransition,
    },
    #[error(
    "patrol deployment {deployment} changed after validation; expected version {expected}, found {found}"
  )]
    StaleDeployment {
        deployment: PatrolDeploymentId,
        expected: u32,
        found: u32,
    },
    #[error(
    "jurisdiction for organization {organization} changed after patrol validation; expected version {expected}, found {found:?}"
  )]
    StaleJurisdiction {
        organization: OrganizationId,
        expected: u32,
        found: Option<u32>,
    },
    #[error("patrol validation occurred at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PatrolPresenceSnapshot {
    deployment_versions: BTreeMap<PatrolDeploymentId, u32>,
    presence: Option<Rating>,
}

impl PatrolPresenceSnapshot {
    pub(crate) fn presence(&self) -> Option<Rating> {
        self.presence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthorityPatrolPresenceSnapshot {
    pub(crate) deployment: Option<(PatrolDeploymentId, u32)>,
    pub(crate) presence: Rating,
}

#[derive(Debug)]
pub struct ValidatedPatrolDeployment {
    draft: PatrolDeploymentDraft,
    expected_jurisdiction_version: u32,
    validated_at: SimTime,
}

impl ValidatedPatrolDeployment {
    pub fn commit(self, state: &mut AppState) -> Result<PatrolDeploymentId, PatrolError> {
        validate_time(state, self.validated_at)?;
        validate_jurisdiction_version(
            state,
            self.draft.organization,
            self.expected_jurisdiction_version,
        )?;
        validate_active_dependencies(state, self.draft.organization, self.draft.neighborhood)?;
        ensure_no_active_duplicate(state, self.draft.organization, self.draft.neighborhood)?;
        let id = state.ids.next_patrol_deployment()?;
        state
            .legal
            .insert_patrol_deployment(PatrolDeploymentRecord {
                id,
                organization: self.draft.organization,
                neighborhood: self.draft.neighborhood,
                windows: self.draft.windows,
                status: PatrolDeploymentStatus::Active,
                established_at: self.validated_at,
                last_changed_at: self.validated_at,
                version: 1,
            });
        Ok(id)
    }
}

pub fn validate_establish_patrol_deployment(
    state: &AppState,
    mut draft: PatrolDeploymentDraft,
) -> Result<ValidatedPatrolDeployment, PatrolError> {
    validate_active_dependencies(state, draft.organization, draft.neighborhood)?;
    ensure_no_active_duplicate(state, draft.organization, draft.neighborhood)?;
    draft.windows = normalize_schedule(draft.windows)?;
    let expected_jurisdiction_version = state
        .legal
        .get_jurisdiction(draft.organization)
        .expect("validated patrol authority must have a jurisdiction record")
        .version();
    Ok(ValidatedPatrolDeployment {
        draft,
        expected_jurisdiction_version,
        validated_at: state.now(),
    })
}

#[derive(Debug)]
pub struct ValidatedPatrolRevision {
    deployment: PatrolDeploymentId,
    windows: Vec<PatrolWindow>,
    expected_version: u32,
    expected_jurisdiction_version: Option<u32>,
    validated_at: SimTime,
}

impl ValidatedPatrolRevision {
    pub fn commit(self, state: &mut AppState) -> Result<PatrolDeploymentId, PatrolError> {
        validate_time(state, self.validated_at)?;
        let record = state
            .legal
            .get_patrol_deployment(self.deployment)
            .ok_or(PatrolError::MissingDeployment(self.deployment))?;
        if record.version() != self.expected_version {
            return Err(PatrolError::StaleDeployment {
                deployment: self.deployment,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() == PatrolDeploymentStatus::Retired {
            return Err(PatrolError::RetiredDeployment(self.deployment));
        }
        validate_record_references(state, record.organization(), record.neighborhood())?;
        if record.status() == PatrolDeploymentStatus::Active {
            let expected_jurisdiction_version = self
                .expected_jurisdiction_version
                .expect("active patrol revision must snapshot jurisdiction version");
            validate_jurisdiction_version(
                state,
                record.organization(),
                expected_jurisdiction_version,
            )?;
            validate_active_dependencies(state, record.organization(), record.neighborhood())?;
        }
        state
            .legal
            .revise_patrol_deployment(self.deployment, self.windows, self.validated_at);
        Ok(self.deployment)
    }
}

pub fn validate_revise_patrol_deployment(
    state: &AppState,
    deployment: PatrolDeploymentId,
    windows: Vec<PatrolWindow>,
) -> Result<ValidatedPatrolRevision, PatrolError> {
    let record = state
        .legal
        .get_patrol_deployment(deployment)
        .ok_or(PatrolError::MissingDeployment(deployment))?;
    if record.status() == PatrolDeploymentStatus::Retired {
        return Err(PatrolError::RetiredDeployment(deployment));
    }
    validate_record_references(state, record.organization(), record.neighborhood())?;
    let expected_jurisdiction_version = if record.status() == PatrolDeploymentStatus::Active {
        validate_active_dependencies(state, record.organization(), record.neighborhood())?;
        Some(
            state
                .legal
                .get_jurisdiction(record.organization())
                .expect("validated active patrol must have jurisdiction")
                .version(),
        )
    } else {
        None
    };
    Ok(ValidatedPatrolRevision {
        deployment,
        windows: normalize_schedule(windows)?,
        expected_version: record.version(),
        expected_jurisdiction_version,
        validated_at: state.now(),
    })
}

#[derive(Debug)]
pub struct ValidatedPatrolTransition {
    deployment: PatrolDeploymentId,
    target_status: PatrolDeploymentStatus,
    expected_version: u32,
    expected_jurisdiction_version: Option<u32>,
    validated_at: SimTime,
}

impl ValidatedPatrolTransition {
    pub fn commit(self, state: &mut AppState) -> Result<PatrolDeploymentId, PatrolError> {
        validate_time(state, self.validated_at)?;
        let record = state
            .legal
            .get_patrol_deployment(self.deployment)
            .ok_or(PatrolError::MissingDeployment(self.deployment))?;
        if record.version() != self.expected_version {
            return Err(PatrolError::StaleDeployment {
                deployment: self.deployment,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if self.target_status == PatrolDeploymentStatus::Active {
            let expected_jurisdiction_version = self
                .expected_jurisdiction_version
                .expect("patrol resume must snapshot jurisdiction version");
            validate_jurisdiction_version(
                state,
                record.organization(),
                expected_jurisdiction_version,
            )?;
            validate_active_dependencies(state, record.organization(), record.neighborhood())?;
            ensure_no_active_duplicate(state, record.organization(), record.neighborhood())?;
        } else {
            validate_record_references(state, record.organization(), record.neighborhood())?;
        }
        state.legal.set_patrol_deployment_status(
            self.deployment,
            self.target_status,
            self.validated_at,
        );
        Ok(self.deployment)
    }
}

pub fn validate_patrol_transition(
    state: &AppState,
    deployment: PatrolDeploymentId,
    transition: PatrolDeploymentTransition,
) -> Result<ValidatedPatrolTransition, PatrolError> {
    let record = state
        .legal
        .get_patrol_deployment(deployment)
        .ok_or(PatrolError::MissingDeployment(deployment))?;
    let target_status = match (record.status(), transition) {
        (PatrolDeploymentStatus::Active, PatrolDeploymentTransition::Suspend) => {
            PatrolDeploymentStatus::Suspended
        }
        (PatrolDeploymentStatus::Active, PatrolDeploymentTransition::Retire)
        | (PatrolDeploymentStatus::Suspended, PatrolDeploymentTransition::Retire) => {
            PatrolDeploymentStatus::Retired
        }
        (PatrolDeploymentStatus::Suspended, PatrolDeploymentTransition::Resume) => {
            PatrolDeploymentStatus::Active
        }
        (status, transition) => {
            return Err(PatrolError::InvalidTransition {
                deployment,
                status,
                transition,
            });
        }
    };
    validate_record_references(state, record.organization(), record.neighborhood())?;
    let expected_jurisdiction_version = if target_status == PatrolDeploymentStatus::Active {
        validate_active_dependencies(state, record.organization(), record.neighborhood())?;
        ensure_no_active_duplicate(state, record.organization(), record.neighborhood())?;
        Some(
            state
                .legal
                .get_jurisdiction(record.organization())
                .expect("validated patrol resume must have jurisdiction")
                .version(),
        )
    } else {
        None
    };
    Ok(ValidatedPatrolTransition {
        deployment,
        target_status,
        expected_version: record.version(),
        expected_jurisdiction_version,
        validated_at: state.now(),
    })
}

pub fn resolve_patrol_presence(
    state: &AppState,
    neighborhood: NeighborhoodId,
    at: SimTime,
) -> Option<Rating> {
    resolve_patrol_presence_snapshot(state, neighborhood, at).presence()
}

pub(crate) fn resolve_patrol_presence_snapshot(
    state: &AppState,
    neighborhood: NeighborhoodId,
    at: SimTime,
) -> PatrolPresenceSnapshot {
    let minute = u16::try_from(at.as_minutes() % u64::from(MINUTES_PER_DAY))
        .expect("minute-of-day remainder must fit u16");
    // An explicit patrol schedule is authoritative: once an authority models deployments in a
    // neighborhood, its windows define street presence there and a coverage gap means no one is
    // on beat (presence zero). The neighborhood's ambient `police_presence` profile is only the
    // estimate for districts with no modeled schedule at all — consumers fall back to it when
    // this snapshot reports None. Crews exploit exactly this by scheduling work inside gaps.
    let mut deployment_versions = BTreeMap::new();
    let mut presence: Option<Rating> = None;
    for deployment in state
        .legal
        .active_patrol_deployments_for_neighborhood(neighborhood)
    {
        deployment_versions.insert(deployment.id(), deployment.version());
        let deployment_presence = deployment
            .windows()
            .iter()
            .copied()
            .filter(|window| window_contains_minute(*window, minute))
            .map(|window| window.presence())
            .max_by_key(|rating| rating.value())
            .unwrap_or_else(zero_rating);
        presence = Some(match presence {
            Some(current) if current.value() >= deployment_presence.value() => current,
            Some(_) | None => deployment_presence,
        });
    }
    PatrolPresenceSnapshot {
        deployment_versions,
        presence,
    }
}

pub(crate) fn resolve_patrol_presence_interval_snapshot(
    state: &AppState,
    neighborhood: NeighborhoodId,
    start: SimTime,
    end: SimTime,
) -> PatrolPresenceSnapshot {
    if end <= start {
        return resolve_patrol_presence_snapshot(state, neighborhood, end);
    }

    let mut deployment_versions = BTreeMap::new();
    let mut daily_presence = [0_u8; MINUTES_PER_DAY as usize];
    let mut has_deployment = false;
    for deployment in state
        .legal
        .active_patrol_deployments_for_neighborhood(neighborhood)
    {
        has_deployment = true;
        deployment_versions.insert(deployment.id(), deployment.version());
        for window in deployment.windows() {
            let start_minute = usize::from(window.start().value());
            let presence = window.presence().value();
            for offset in 0..usize::from(window.duration_minutes()) {
                let minute = (start_minute + offset) % usize::from(MINUTES_PER_DAY);
                daily_presence[minute] = daily_presence[minute].max(presence);
            }
        }
    }
    if !has_deployment {
        return PatrolPresenceSnapshot {
            deployment_versions,
            presence: None,
        };
    }

    let duration = end.as_minutes().saturating_sub(start.as_minutes());
    let day_minutes = u64::from(MINUTES_PER_DAY);
    let daily_total: u64 = daily_presence.iter().map(|value| u64::from(*value)).sum();
    let full_days = duration / day_minutes;
    let remainder = duration % day_minutes;
    let mut total_presence = daily_total.saturating_mul(full_days);
    let start_minute = start.as_minutes() % day_minutes;
    for offset in 0..remainder {
        let minute = usize::try_from((start_minute + offset) % day_minutes)
            .expect("minute-of-day remainder must fit usize");
        total_presence = total_presence.saturating_add(u64::from(daily_presence[minute]));
    }
    let average = total_presence
        .saturating_add(duration / 2)
        .checked_div(duration)
        .expect("positive patrol interval duration must divide");
    let average = u8::try_from(average).expect("average patrol presence must fit u8");
    PatrolPresenceSnapshot {
        deployment_versions,
        presence: Some(
            Rating::try_new(average).expect("average patrol presence must remain within bounds"),
        ),
    }
}

pub(crate) fn resolve_authority_patrol_presence_snapshot(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    at: SimTime,
) -> AuthorityPatrolPresenceSnapshot {
    let fallback = state
        .world
        .get_neighborhood(neighborhood)
        .expect("validated police response neighborhood must exist")
        .profile()
        .institutions
        .police_presence;
    let Some(deployment) = state.legal.active_patrol_for(organization, neighborhood) else {
        return AuthorityPatrolPresenceSnapshot {
            deployment: None,
            presence: fallback,
        };
    };
    let minute = u16::try_from(at.as_minutes() % u64::from(MINUTES_PER_DAY))
        .expect("minute-of-day remainder must fit u16");
    // Same authoritative-schedule contract as `resolve_patrol_presence_snapshot`: an off-window
    // minute inside a modeled deployment is a real coverage gap (zero presence, slowest allowed
    // response), not a reason to fall back to the ambient estimate.
    let presence = deployment
        .windows()
        .iter()
        .copied()
        .filter(|window| window_contains_minute(*window, minute))
        .map(PatrolWindow::presence)
        .max_by_key(|rating| rating.value())
        .unwrap_or_else(zero_rating);
    AuthorityPatrolPresenceSnapshot {
        deployment: Some((deployment.id(), deployment.version())),
        presence,
    }
}

pub(crate) fn is_canonical_patrol_schedule(windows: &[PatrolWindow]) -> bool {
    if windows.is_empty() || !schedule_has_no_overlap(windows) {
        return false;
    }
    windows
        .windows(2)
        .all(|pair| patrol_window_sort_key(pair[0]) <= patrol_window_sort_key(pair[1]))
}

fn validate_time(state: &AppState, expected: SimTime) -> Result<(), PatrolError> {
    crate::core::time::ensure_time_current(state.now(), expected)
        .map_err(|(expected, found)| PatrolError::StaleTime { expected, found })
}

fn validate_jurisdiction_version(
    state: &AppState,
    organization: OrganizationId,
    expected: u32,
) -> Result<(), PatrolError> {
    let found = state
        .legal
        .get_jurisdiction(organization)
        .map(|jurisdiction| jurisdiction.version());
    if found == Some(expected) {
        Ok(())
    } else {
        Err(PatrolError::StaleJurisdiction {
            organization,
            expected,
            found,
        })
    }
}

fn validate_record_references(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
) -> Result<(), PatrolError> {
    if state.world.get_organization(organization).is_none() {
        return Err(PatrolError::MissingOrganization(organization));
    }
    if state.world.get_neighborhood(neighborhood).is_none() {
        return Err(PatrolError::MissingNeighborhood(neighborhood));
    }
    Ok(())
}

fn validate_active_dependencies(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
) -> Result<(), PatrolError> {
    let authority = state
        .world
        .get_organization(organization)
        .ok_or(PatrolError::MissingOrganization(organization))?;
    if authority.kind() != OrganizationKind::LawEnforcement {
        return Err(PatrolError::InvalidAuthorityKind(organization));
    }
    let _ = state
        .world
        .get_neighborhood(neighborhood)
        .ok_or(PatrolError::MissingNeighborhood(neighborhood))?;
    let jurisdiction = state
        .legal
        .get_jurisdiction(organization)
        .ok_or(PatrolError::MissingJurisdiction(organization))?;
    if !jurisdiction.neighborhoods().contains(&neighborhood) {
        return Err(PatrolError::OutsideJurisdiction {
            organization,
            neighborhood,
        });
    }
    Ok(())
}

fn ensure_no_active_duplicate(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
) -> Result<(), PatrolError> {
    if let Some(existing) = state.legal.active_patrol_for(organization, neighborhood) {
        return Err(PatrolError::DuplicateActiveDeployment {
            organization,
            neighborhood,
            existing: existing.id(),
        });
    }
    Ok(())
}

fn normalize_schedule(mut windows: Vec<PatrolWindow>) -> Result<Vec<PatrolWindow>, PatrolError> {
    if windows.is_empty() {
        return Err(PatrolError::EmptySchedule);
    }
    windows.sort_by_key(|window| patrol_window_sort_key(*window));
    if let Some(minute) = first_overlapping_minute(&windows) {
        return Err(PatrolError::OverlappingWindow { minute });
    }
    Ok(windows)
}

fn schedule_has_no_overlap(windows: &[PatrolWindow]) -> bool {
    first_overlapping_minute(windows).is_none()
}

fn first_overlapping_minute(windows: &[PatrolWindow]) -> Option<DayMinute> {
    let mut occupied = [false; MINUTES_PER_DAY as usize];
    for window in windows {
        for offset in 0..window.duration_minutes() {
            let minute = (u32::from(window.start().value()) + u32::from(offset))
                % u32::from(MINUTES_PER_DAY);
            let index = usize::try_from(minute).expect("minute-of-day must fit usize");
            if occupied[index] {
                return Some(
                    DayMinute::try_new(u16::try_from(minute).expect("minute-of-day must fit u16"))
                        .expect("wrapped patrol minute must be valid"),
                );
            }
            occupied[index] = true;
        }
    }
    None
}

fn window_contains_minute(window: PatrolWindow, minute: u16) -> bool {
    let elapsed = (u32::from(minute) + u32::from(MINUTES_PER_DAY)
        - u32::from(window.start().value()))
        % u32::from(MINUTES_PER_DAY);
    elapsed < u32::from(window.duration_minutes())
}

fn patrol_window_sort_key(window: PatrolWindow) -> (u16, u16, u8) {
    (
        window.start().value(),
        window.duration_minutes(),
        window.presence().value(),
    )
}

fn zero_rating() -> Rating {
    Rating::try_new(0).expect("zero is a valid rating")
}

#[cfg(test)]
mod tests;
