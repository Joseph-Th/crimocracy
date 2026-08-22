//! Canonical patrol deployment validation, lifecycle transitions, and time-of-day presence queries.

use crate::core::id::{IdExhaustionError, NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::{
    DayMinute, PatrolDeploymentDraft, PatrolDeploymentRecord, PatrolDeploymentStatus, PatrolWindow,
};
use crate::world::{Lifecycle, OrganizationKind, Rating};
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
    #[error("organization {0} is not active")]
    InactiveAuthority(OrganizationId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("neighborhood {0} is not active")]
    InactiveNeighborhood(NeighborhoodId),
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
    if state.now() == expected {
        Ok(())
    } else {
        Err(PatrolError::StaleTime {
            expected,
            found: state.now(),
        })
    }
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
    if authority.lifecycle() != Lifecycle::Active {
        return Err(PatrolError::InactiveAuthority(organization));
    }
    let neighborhood_record = state
        .world
        .get_neighborhood(neighborhood)
        .ok_or(PatrolError::MissingNeighborhood(neighborhood))?;
    if neighborhood_record.lifecycle() != Lifecycle::Active {
        return Err(PatrolError::InactiveNeighborhood(neighborhood));
    }
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::legal::jurisdiction_system::{validate_set_jurisdiction, JurisdictionError};
    use crate::legal::JurisdictionDraft;
    use crate::world::world_system::{insert_neighborhood, insert_organization};
    use crate::world::{
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, OrganizationDraft,
    };
    use std::collections::BTreeSet;

    fn make_fixture() -> (crate::Registry, AppState, OrganizationId, NeighborhoodId) {
        let registry = build_registry();
        let mut state = AppState::new(0x0A70_1933);
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Patrol Test Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).expect("fixture rating should validate"),
                        commercial_activity: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        illicit_demand: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(60)
                            .expect("fixture rating should validate"),
                        political_influence: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        social_cohesion: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        visible_violence_tolerance: Rating::try_new(40)
                            .expect("fixture rating should validate"),
                    },
                },
            },
        )
        .expect("patrol neighborhood fixture should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Patrol Test Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("patrol authority fixture should validate");
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(70)
                    .expect("fixture priority should validate"),
            },
        )
        .expect("patrol jurisdiction fixture should validate")
        .commit(&mut state)
        .expect("patrol jurisdiction fixture should commit");
        (registry, state, police, neighborhood)
    }

    fn window(start: u16, duration: u16, presence: u8) -> PatrolWindow {
        PatrolWindow::try_new(
            DayMinute::try_new(start).expect("fixture minute should validate"),
            duration,
            Rating::try_new(presence).expect("fixture rating should validate"),
        )
        .expect("fixture patrol window should validate")
    }

    #[test]
    fn patrol_windows_wrap_midnight_and_leave_real_coverage_gaps() {
        let (_registry, mut state, police, neighborhood) = make_fixture();
        validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![window(1_320, 240, 80), window(480, 120, 40)],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");

        assert_eq!(
            resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(1_380))
                .map(Rating::value),
            Some(80)
        );
        assert_eq!(
            resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(60))
                .map(Rating::value),
            Some(80)
        );
        assert_eq!(
            resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(300))
                .map(Rating::value),
            Some(0)
        );
        assert_eq!(
            resolve_patrol_presence(&state, neighborhood, SimTime::from_minutes(540))
                .map(Rating::value),
            Some(40)
        );
        validate_state(&state).expect("patrol state should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn overlapping_patrol_windows_are_rejected_without_mutation() {
        let (_registry, state, police, neighborhood) = make_fixture();
        let error = validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![window(1_380, 120, 70), window(30, 60, 50)],
            },
        )
        .expect_err("overlapping wrapped windows must be rejected");
        assert!(matches!(error, PatrolError::OverlappingWindow { .. }));
        assert_eq!(
            state
                .legal()
                .patrol_deployments()
                .filter(|deployment| deployment.neighborhood() == neighborhood)
                .count(),
            0
        );
        validate_invariants(&state);
    }

    #[test]
    fn active_patrol_blocks_jurisdiction_contraction_until_suspended() {
        let (_registry, mut state, police, neighborhood) = make_fixture();
        let second_neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Second Patrol Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).expect("fixture rating should validate"),
                        commercial_activity: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        illicit_demand: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        political_influence: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        social_cohesion: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        visible_violence_tolerance: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                    },
                },
            },
        )
        .expect("second neighborhood should validate");
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood, second_neighborhood]),
                case_intake_priority: Rating::try_new(70)
                    .expect("fixture priority should validate"),
            },
        )
        .expect("expanded jurisdiction should validate")
        .commit(&mut state)
        .expect("expanded jurisdiction should commit");
        let deployment = validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![window(0, 1_440, 70)],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");

        let contraction = JurisdictionDraft {
            organization: police,
            neighborhoods: BTreeSet::from([second_neighborhood]),
            case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
        };
        let error = validate_set_jurisdiction(&state, contraction.clone())
            .expect_err("active patrol must block removal of its neighborhood");
        assert_eq!(
            error,
            JurisdictionError::ActivePatrolDeployment {
                organization: police,
                neighborhood,
                deployment,
            }
        );

        validate_patrol_transition(&state, deployment, PatrolDeploymentTransition::Suspend)
            .expect("active patrol should suspend")
            .commit(&mut state)
            .expect("patrol suspension should commit");
        validate_set_jurisdiction(&state, contraction)
            .expect("suspended patrol should not block jurisdiction contraction")
            .commit(&mut state)
            .expect("jurisdiction contraction should commit");
        validate_state(&state).expect("suspended patrol may remain outside current jurisdiction");
        validate_invariants(&state);
    }

    #[test]
    fn stale_patrol_revision_cannot_overwrite_lifecycle_change() {
        let (_registry, mut state, police, neighborhood) = make_fixture();
        let deployment = validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![window(0, 1_440, 60)],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");
        let stale =
            validate_revise_patrol_deployment(&state, deployment, vec![window(0, 1_440, 80)])
                .expect("patrol revision should validate");
        validate_patrol_transition(&state, deployment, PatrolDeploymentTransition::Suspend)
            .expect("patrol suspension should validate")
            .commit(&mut state)
            .expect("patrol suspension should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale revision must not overwrite lifecycle change");
        assert_eq!(
            error,
            PatrolError::StaleDeployment {
                deployment,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .legal()
                .get_patrol_deployment(deployment)
                .expect("deployment should remain present")
                .status(),
            PatrolDeploymentStatus::Suspended
        );
        validate_invariants(&state);
    }

    #[test]
    fn patrol_deployment_survives_save_round_trip_with_active_index() {
        let (registry, mut state, police, neighborhood) = make_fixture();
        let deployment = validate_establish_patrol_deployment(
            &state,
            PatrolDeploymentDraft {
                organization: police,
                neighborhood,
                windows: vec![window(600, 120, 75)],
            },
        )
        .expect("patrol deployment should validate")
        .commit(&mut state)
        .expect("patrol deployment should commit");
        let envelope = build_save(&registry, &state).expect("patrol state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&registry, decoded).expect("patrol state should restore");

        assert_eq!(
            restored
                .legal()
                .get_patrol_deployment(deployment)
                .expect("restored deployment should exist")
                .version(),
            1
        );
        assert_eq!(
            resolve_patrol_presence(&restored, neighborhood, SimTime::from_minutes(660))
                .map(Rating::value),
            Some(75)
        );
        validate_state(&restored).expect("restored patrol state should validate");
        validate_invariants(&restored);
    }
}
