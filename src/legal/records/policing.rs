//! Jurisdiction, patrol-deployment, and police-response record vocabulary and indexes.

use crate::core::id::{
    NeighborhoodId, OperationId, OrganizationId, PatrolDeploymentId, PoliceResponseId,
};
use crate::core::time::{DAY_MINUTES_U16, SimTime};
use crate::world::Rating;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct JurisdictionRevision {
    pub(in crate::legal) changed_at: SimTime,
    pub(in crate::legal) neighborhoods: BTreeSet<NeighborhoodId>,
    pub(in crate::legal) case_intake_priority: Rating,
    pub(in crate::legal) version: u32,
}

impl JurisdictionRevision {
    pub(crate) fn changed_at(&self) -> SimTime {
        self.changed_at
    }

    pub(crate) fn neighborhoods(&self) -> &BTreeSet<NeighborhoodId> {
        &self.neighborhoods
    }

    pub(crate) fn case_intake_priority(&self) -> Rating {
        self.case_intake_priority
    }

    pub(crate) fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JurisdictionRecord {
    pub(in crate::legal) organization: OrganizationId,
    pub(in crate::legal) revisions: Vec<JurisdictionRevision>,
}

impl JurisdictionRecord {
    fn current_revision(&self) -> &JurisdictionRevision {
        self.revisions
            .last()
            .expect("persisted jurisdiction must contain an establishment revision")
    }

    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn neighborhoods(&self) -> &BTreeSet<NeighborhoodId> {
        self.current_revision().neighborhoods()
    }

    pub fn case_intake_priority(&self) -> Rating {
        self.current_revision().case_intake_priority()
    }

    pub fn version(&self) -> u32 {
        self.current_revision().version()
    }

    pub(crate) fn revisions(&self) -> &[JurisdictionRevision] {
        &self.revisions
    }

    pub(crate) fn revision_by_version(&self, version: u32) -> Option<&JurisdictionRevision> {
        version
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.revisions.get(index))
            .filter(|revision| revision.version() == version)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct DayMinute(u16);

impl DayMinute {
    pub const MAX: u16 = DAY_MINUTES_U16 - 1;

    pub fn try_new(value: u16) -> Result<Self, DayMinuteError> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(DayMinuteError { value })
        }
    }

    pub const fn value(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for DayMinute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::try_new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("minute of day {value} is outside the inclusive range 0..=1439")]
pub struct DayMinuteError {
    value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PatrolWindow {
    start: DayMinute,
    duration_minutes: u16,
    presence: Rating,
}

impl PatrolWindow {
    pub const MIN_DURATION_MINUTES: u16 = 1;
    pub const MAX_DURATION_MINUTES: u16 = DAY_MINUTES_U16;

    pub fn try_new(
        start: DayMinute,
        duration_minutes: u16,
        presence: Rating,
    ) -> Result<Self, PatrolWindowError> {
        if !(Self::MIN_DURATION_MINUTES..=Self::MAX_DURATION_MINUTES).contains(&duration_minutes) {
            return Err(PatrolWindowError { duration_minutes });
        }
        Ok(Self {
            start,
            duration_minutes,
            presence,
        })
    }

    pub const fn start(self) -> DayMinute {
        self.start
    }

    pub const fn duration_minutes(self) -> u16 {
        self.duration_minutes
    }

    pub const fn presence(self) -> Rating {
        self.presence
    }
}

impl<'de> Deserialize<'de> for PatrolWindow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedPatrolWindow {
            start: DayMinute,
            duration_minutes: u16,
            presence: Rating,
        }

        let serialized = SerializedPatrolWindow::deserialize(deserializer)?;
        Self::try_new(
            serialized.start,
            serialized.duration_minutes,
            serialized.presence,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("patrol window duration {duration_minutes} is outside the inclusive range 1..=1440")]
pub struct PatrolWindowError {
    duration_minutes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatrolDeploymentStatus {
    Active,
    Suspended,
    Retired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PatrolDeploymentRevision {
    pub(in crate::legal) changed_at: SimTime,
    pub(in crate::legal) windows: Vec<PatrolWindow>,
    pub(in crate::legal) status: PatrolDeploymentStatus,
    pub(in crate::legal) version: u32,
}

impl PatrolDeploymentRevision {
    pub(crate) fn changed_at(&self) -> SimTime {
        self.changed_at
    }

    pub(crate) fn windows(&self) -> &[PatrolWindow] {
        &self.windows
    }

    pub(crate) fn status(&self) -> PatrolDeploymentStatus {
        self.status
    }

    pub(crate) fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatrolDeploymentRecord {
    pub(in crate::legal) id: PatrolDeploymentId,
    pub(in crate::legal) organization: OrganizationId,
    pub(in crate::legal) neighborhood: NeighborhoodId,
    pub(in crate::legal) established_at: SimTime,
    pub(in crate::legal) revisions: Vec<PatrolDeploymentRevision>,
}

impl PatrolDeploymentRecord {
    fn current_revision(&self) -> &PatrolDeploymentRevision {
        self.revisions
            .last()
            .expect("persisted patrol deployment must contain an establishment revision")
    }

    pub fn id(&self) -> PatrolDeploymentId {
        self.id
    }

    pub fn organization(&self) -> OrganizationId {
        self.organization
    }

    pub fn neighborhood(&self) -> NeighborhoodId {
        self.neighborhood
    }

    pub fn windows(&self) -> &[PatrolWindow] {
        self.current_revision().windows()
    }

    pub fn status(&self) -> PatrolDeploymentStatus {
        self.current_revision().status()
    }

    pub fn established_at(&self) -> SimTime {
        self.established_at
    }

    pub fn version(&self) -> u32 {
        self.current_revision().version()
    }

    pub(crate) fn revisions(&self) -> &[PatrolDeploymentRevision] {
        &self.revisions
    }

    pub(crate) fn revision_at(&self, at: SimTime) -> Option<&PatrolDeploymentRevision> {
        self.revisions
            .iter()
            .rev()
            .find(|revision| revision.changed_at() <= at)
    }
}

#[derive(Clone, Debug)]
pub struct PatrolDeploymentDraft {
    pub organization: OrganizationId,
    pub neighborhood: NeighborhoodId,
    pub windows: Vec<PatrolWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoliceResponseStatus {
    Dispatched,
    Arrived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliceResponsePatrolSnapshot {
    deployment: PatrolDeploymentId,
    version: u32,
}

impl PoliceResponsePatrolSnapshot {
    pub(crate) fn new(deployment: PatrolDeploymentId, version: u32) -> Self {
        Self {
            deployment,
            version,
        }
    }

    pub fn deployment(self) -> PatrolDeploymentId {
        self.deployment
    }

    pub fn version(self) -> u32 {
        self.version
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseRouting {
    pub(in crate::legal) authority: OrganizationId,
    pub(in crate::legal) neighborhood: NeighborhoodId,
    pub(in crate::legal) source_operation: OperationId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseTiming {
    pub(in crate::legal) dispatched_at: SimTime,
    pub(in crate::legal) arrival_due_at: SimTime,
    pub(in crate::legal) arrived_at: Option<SimTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PoliceResponseState {
    pub(in crate::legal) alert_score: i16,
    pub(in crate::legal) response_presence: Rating,
    pub(in crate::legal) jurisdiction_version: u32,
    pub(in crate::legal) patrol: Option<PoliceResponsePatrolSnapshot>,
    pub(in crate::legal) status: PoliceResponseStatus,
    pub(in crate::legal) version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoliceResponseRecord {
    pub(in crate::legal) id: PoliceResponseId,
    pub(in crate::legal) routing: PoliceResponseRouting,
    pub(in crate::legal) timing: PoliceResponseTiming,
    pub(in crate::legal) state: PoliceResponseState,
}

impl PoliceResponseRecord {
    pub fn id(&self) -> PoliceResponseId {
        self.id
    }
    pub fn authority(&self) -> OrganizationId {
        self.routing.authority
    }
    pub fn neighborhood(&self) -> NeighborhoodId {
        self.routing.neighborhood
    }
    pub fn source_operation(&self) -> OperationId {
        self.routing.source_operation
    }
    pub fn dispatched_at(&self) -> SimTime {
        self.timing.dispatched_at
    }
    pub fn arrival_due_at(&self) -> SimTime {
        self.timing.arrival_due_at
    }
    pub fn arrived_at(&self) -> Option<SimTime> {
        self.timing.arrived_at
    }
    pub fn alert_score(&self) -> i16 {
        self.state.alert_score
    }
    pub fn response_presence(&self) -> Rating {
        self.state.response_presence
    }
    pub fn jurisdiction_version(&self) -> u32 {
        self.state.jurisdiction_version
    }
    pub fn patrol(&self) -> Option<PoliceResponsePatrolSnapshot> {
        self.state.patrol
    }
    pub fn status(&self) -> PoliceResponseStatus {
        self.state.status
    }
    pub fn version(&self) -> u32 {
        self.state.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct JurisdictionIndexes {
    pub(in crate::legal) jurisdictions_by_neighborhood:
        BTreeMap<NeighborhoodId, BTreeSet<OrganizationId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct PatrolIndexes {
    pub(in crate::legal) by_neighborhood: BTreeMap<NeighborhoodId, BTreeSet<PatrolDeploymentId>>,
    pub(in crate::legal) active_by_organization_neighborhood:
        BTreeMap<(OrganizationId, NeighborhoodId), PatrolDeploymentId>,
    pub(in crate::legal) active_by_neighborhood:
        BTreeMap<NeighborhoodId, BTreeSet<PatrolDeploymentId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::legal) struct PoliceResponseIndexes {
    pub(in crate::legal) by_source_operation: BTreeMap<OperationId, PoliceResponseId>,
    pub(in crate::legal) dispatched_by_arrival_due: BTreeMap<SimTime, BTreeSet<PoliceResponseId>>,
}

#[derive(Clone, Debug)]
pub struct JurisdictionDraft {
    pub organization: OrganizationId,
    pub neighborhoods: BTreeSet<NeighborhoodId>,
    pub case_intake_priority: Rating,
}
