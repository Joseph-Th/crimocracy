//! Canonical patrol deployment validation, lifecycle transitions, and time-of-day presence queries.

use crate::core::id::{IdExhaustionError, NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES_U16, SimTime};
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::legal::{
    DayMinute, PatrolDeploymentDraft, PatrolDeploymentRecord, PatrolDeploymentRevision,
    PatrolDeploymentStatus, PatrolWindow, PoliceResponsePatrolSnapshot,
};
use crate::world::{OrganizationKind, Rating};
use std::collections::BTreeMap;
use thiserror::Error;

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
    #[error("patrol deployment {0} already has the requested schedule")]
    ScheduleUnchanged(PatrolDeploymentId),
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
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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
                established_at: self.validated_at,
                revisions: vec![PatrolDeploymentRevision {
                    changed_at: self.validated_at,
                    windows: self.draft.windows,
                    status: PatrolDeploymentStatus::Active,
                    version: 1,
                }],
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
        ensure_version_can_advance(record.version(), "patrol deployment")?;
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
    let windows = normalize_schedule(windows)?;
    if record.windows() == windows.as_slice() {
        return Err(PatrolError::ScheduleUnchanged(deployment));
    }
    ensure_version_can_advance(record.version(), "patrol deployment")?;
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
        windows,
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
        ensure_version_can_advance(record.version(), "patrol deployment")?;
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
    ensure_version_can_advance(record.version(), "patrol deployment")?;
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
    let minute = u16::try_from(at.as_minutes() % u64::from(DAY_MINUTES_U16))
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
        .patrol_deployments_for_neighborhood(neighborhood)
    {
        let Some(revision) = deployment
            .revision_at(at)
            .filter(|revision| revision.status() == PatrolDeploymentStatus::Active)
        else {
            continue;
        };
        deployment_versions.insert(deployment.id(), revision.version());
        let deployment_presence = revision
            .windows()
            .iter()
            .copied()
            .filter(|window| is_minute_within_patrol_window(*window, minute))
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

    let deployments: Vec<_> = state
        .legal
        .patrol_deployments_for_neighborhood(neighborhood)
        .collect();
    let mut boundaries = vec![start, end];
    for deployment in &deployments {
        boundaries.extend(
            deployment
                .revisions()
                .iter()
                .map(PatrolDeploymentRevision::changed_at)
                .filter(|changed_at| *changed_at > start && *changed_at < end),
        );
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let ambient_presence = state
        .world
        .get_neighborhood(neighborhood)
        .map(|record| record.profile().institutions.police_presence.value())
        .unwrap_or(0);
    let mut deployment_versions = BTreeMap::new();
    let mut has_modeled_patrol = false;
    let mut total_presence = 0_u128;
    for segment in boundaries.windows(2) {
        let segment_start = segment[0];
        let segment_end = segment[1];
        let mut daily_presence = [0_u8; DAY_MINUTES_U16 as usize];
        let mut segment_has_patrol = false;
        for deployment in &deployments {
            let Some(revision) = deployment
                .revision_at(segment_start)
                .filter(|revision| revision.status() == PatrolDeploymentStatus::Active)
            else {
                continue;
            };
            segment_has_patrol = true;
            has_modeled_patrol = true;
            deployment_versions.insert(deployment.id(), revision.version());
            for window in revision.windows() {
                let start_minute = usize::from(window.start().value());
                let presence = window.presence().value();
                for offset in 0..usize::from(window.duration_minutes()) {
                    let minute = (start_minute + offset) % usize::from(DAY_MINUTES_U16);
                    daily_presence[minute] = daily_presence[minute].max(presence);
                }
            }
        }
        let segment_duration = segment_end
            .as_minutes()
            .checked_sub(segment_start.as_minutes())
            .expect("ordered patrol segment must have positive duration");
        total_presence += if segment_has_patrol {
            scheduled_presence_sum(&daily_presence, segment_start, segment_duration)
        } else {
            u128::from(ambient_presence) * u128::from(segment_duration)
        };
    }
    if !has_modeled_patrol {
        return PatrolPresenceSnapshot {
            deployment_versions,
            presence: None,
        };
    }
    let duration = end
        .as_minutes()
        .checked_sub(start.as_minutes())
        .expect("ordered patrol interval must have a positive duration");
    let average = (total_presence + u128::from(duration / 2)) / u128::from(duration);
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
    let Some((deployment, revision)) = state
        .legal
        .patrol_deployments_for_neighborhood(neighborhood)
        .filter(|deployment| deployment.organization() == organization)
        .filter_map(|deployment| {
            deployment
                .revision_at(at)
                .filter(|revision| revision.status() == PatrolDeploymentStatus::Active)
                .map(|revision| (deployment, revision))
        })
        .max_by_key(|(deployment, revision)| (revision.changed_at(), deployment.id()))
    else {
        return AuthorityPatrolPresenceSnapshot {
            deployment: None,
            presence: fallback,
        };
    };
    let minute = u16::try_from(at.as_minutes() % u64::from(DAY_MINUTES_U16))
        .expect("minute-of-day remainder must fit u16");
    // Same authoritative-schedule contract as `resolve_patrol_presence_snapshot`: an off-window
    // minute inside a modeled deployment is a real coverage gap (zero presence, slowest allowed
    // response), not a reason to fall back to the ambient estimate.
    let presence = revision
        .windows()
        .iter()
        .copied()
        .filter(|window| is_minute_within_patrol_window(*window, minute))
        .map(PatrolWindow::presence)
        .max_by_key(|rating| rating.value())
        .unwrap_or_else(zero_rating);
    AuthorityPatrolPresenceSnapshot {
        deployment: Some((deployment.id(), revision.version())),
        presence,
    }
}

/// Proves that a persisted police-response patrol snapshot could have been observed at `at`.
///
/// Several patrol revisions may share one simulation minute. Cross-domain ordering within that
/// minute is intentionally not persisted, so a response dispatched at that timestamp may validly
/// precede or follow any same-minute patrol revision while preserving each deployment's revision
/// order. Validation therefore accepts the active revision immediately before `at` and every
/// active revision authored exactly at `at`; it does not collapse history to the final revision of
/// that minute. An ambient (`None`) snapshot is possible only when no deployment was active just
/// before the minute, or when that active deployment has a same-minute suspension/retirement that
/// could have happened before dispatch.
pub(crate) fn police_response_patrol_snapshot_is_possible(
    state: &AppState,
    organization: OrganizationId,
    neighborhood: NeighborhoodId,
    at: SimTime,
    snapshot: Option<PoliceResponsePatrolSnapshot>,
    presence: Rating,
) -> bool {
    let fallback = match state.world.get_neighborhood(neighborhood) {
        Some(record) => record.profile().institutions.police_presence,
        None => return false,
    };
    let deployments: Vec<_> = state
        .legal
        .patrol_deployments_for_neighborhood(neighborhood)
        .filter(|deployment| deployment.organization() == organization)
        .collect();

    if let Some(snapshot) = snapshot {
        return deployments.iter().any(|deployment| {
            if deployment.id() != snapshot.deployment() {
                return false;
            }
            let candidate = deployment.revisions().iter().find(|revision| {
                revision.version() == snapshot.version()
                    && revision.status() == PatrolDeploymentStatus::Active
                    && (revision.changed_at() == at
                        || revision.changed_at() < at
                            && deployment
                                .revisions()
                                .iter()
                                .filter(|later| later.version() > revision.version())
                                .all(|later| later.changed_at() >= at))
            });
            candidate.is_some_and(|revision| patrol_revision_presence(revision, at) == presence)
        });
    }

    if presence != fallback {
        return false;
    }
    let active_before = deployments.iter().find_map(|deployment| {
        deployment
            .revisions()
            .iter()
            .rev()
            .find(|revision| revision.changed_at() < at)
            .filter(|revision| revision.status() == PatrolDeploymentStatus::Active)
            .map(|revision| (deployment, revision))
    });
    let Some((deployment, active_before)) = active_before else {
        return true;
    };
    deployment.revisions().iter().any(|revision| {
        revision.version() > active_before.version()
            && revision.changed_at() == at
            && matches!(
                revision.status(),
                PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired
            )
    })
}

fn patrol_revision_presence(revision: &PatrolDeploymentRevision, at: SimTime) -> Rating {
    let minute = u16::try_from(at.as_minutes() % u64::from(DAY_MINUTES_U16))
        .expect("minute-of-day remainder must fit u16");
    revision
        .windows()
        .iter()
        .copied()
        .filter(|window| is_minute_within_patrol_window(*window, minute))
        .map(PatrolWindow::presence)
        .max_by_key(|rating| rating.value())
        .unwrap_or_else(zero_rating)
}

fn scheduled_presence_sum(
    daily_presence: &[u8; DAY_MINUTES_U16 as usize],
    start: SimTime,
    duration: u64,
) -> u128 {
    let day_minutes = u64::from(DAY_MINUTES_U16);
    // Presence is bounded by 100 for every simulated minute, so a u128 accumulator can represent
    // the exact sum across the entire u64 clock range. Saturating u64 arithmetic would silently
    // flatten sufficiently long intervals and produce a materially wrong average.
    let daily_total: u128 = daily_presence.iter().map(|value| u128::from(*value)).sum();
    let full_days = duration / day_minutes;
    let remainder = duration % day_minutes;
    let mut total_presence = daily_total * u128::from(full_days);
    let start_minute = start.as_minutes() % day_minutes;
    for offset in 0..remainder {
        let minute = usize::try_from((start_minute + offset) % day_minutes)
            .expect("minute-of-day remainder must fit usize");
        total_presence += u128::from(daily_presence[minute]);
    }
    total_presence
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
    let mut occupied = [false; DAY_MINUTES_U16 as usize];
    for window in windows {
        for offset in 0..window.duration_minutes() {
            let minute = (u32::from(window.start().value()) + u32::from(offset))
                % u32::from(DAY_MINUTES_U16);
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

fn is_minute_within_patrol_window(window: PatrolWindow, minute: u16) -> bool {
    let elapsed = (u32::from(minute) + u32::from(DAY_MINUTES_U16)
        - u32::from(window.start().value()))
        % u32::from(DAY_MINUTES_U16);
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
