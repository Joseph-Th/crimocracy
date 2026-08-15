//! Semantic operation plans, execution state, and outcomes; sibling systems own authorization and resolution.

pub(crate) mod operation_execution;
pub mod operation_system;
pub(crate) mod police_response_integration;
pub(crate) mod surveillance_integration;

use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, DecisionRequestId, EvidenceId, HistoryEventId, InformationId, InvestigationId,
    NeighborhoodId, OperationId, OrganizationId, PoliceResponseId, ReportId,
};
use crate::core::time::SimTime;
use crate::world::Rating;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationKind {
    Burglary,
    Robbery,
    Hijacking,
    Smuggling,
    Intimidation,
    Kidnapping,
    Surveillance,
    Sabotage,
    Bribery,
    WitnessPressure,
    DocumentTheft,
    GamblingEvent,
    CovertTransfer,
    Extraction,
    RivalInfiltration,
}

pub const ALL_OPERATION_KINDS: [OperationKind; 15] = [
    OperationKind::Burglary,
    OperationKind::Robbery,
    OperationKind::Hijacking,
    OperationKind::Smuggling,
    OperationKind::Intimidation,
    OperationKind::Kidnapping,
    OperationKind::Surveillance,
    OperationKind::Sabotage,
    OperationKind::Bribery,
    OperationKind::WitnessPressure,
    OperationKind::DocumentTheft,
    OperationKind::GamblingEvent,
    OperationKind::CovertTransfer,
    OperationKind::Extraction,
    OperationKind::RivalInfiltration,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationApproach {
    Covert,
    Deceptive,
    Intimidating,
    Violent,
    InsideAssistance,
    Opportunistic,
}

pub const ALL_OPERATION_APPROACHES: [OperationApproach; 6] = [
    OperationApproach::Covert,
    OperationApproach::Deceptive,
    OperationApproach::Intimidating,
    OperationApproach::Violent,
    OperationApproach::InsideAssistance,
    OperationApproach::Opportunistic,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoleKind {
    Driver,
    Lookout,
    EntrySpecialist,
    SafeSpecialist,
    Muscle,
    InsideContact,
    Coordinator,
    Surveillance,
    Negotiator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationObjective {
    AcquireProperty {
        target: EntityRef,
    },
    ObtainCash {
        target: EntityRef,
    },
    Frighten {
        target: EntityRef,
    },
    GatherInformation {
        target: EntityRef,
    },
    DestroyEquipment {
        target: EntityRef,
    },
    MoveContraband {
        origin: EntityRef,
        destination: EntityRef,
    },
    RemovePerson {
        target: CharacterId,
    },
}

impl OperationObjective {
    pub(crate) fn referenced_entities(&self) -> Vec<EntityRef> {
        match self {
            Self::AcquireProperty { target }
            | Self::ObtainCash { target }
            | Self::Frighten { target }
            | Self::GatherInformation { target }
            | Self::DestroyEquipment { target } => vec![*target],
            Self::MoveContraband {
                origin,
                destination,
            } => vec![*origin, *destination],
            Self::RemovePerson { target } => vec![EntityRef::Character(*target)],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationConstraint {
    AvoidCasualties,
    DoNotHarmEmployees,
    AvoidFirearms,
    ProtectLeadershipIdentity,
    PreserveMerchandise,
    CompleteBefore(SimTime),
    ExcludeCharacter(CharacterId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationContingency {
    AbortOnPoliceArrivalBeforeEntry,
    UseForceOnResistance,
    UseSecondaryExitIfBlocked,
    ContactIfDetained(CharacterId),
    RequestDecisionOnUnexpectedCondition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OperationStatus {
    Authorized,
    InProgress,
    AwaitingDecision,
    Completed,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationAbortPhase {
    BeforeStart,
    InProgress,
    AwaitingDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationAbortCause {
    AuthorityOrder,
    Decision(DecisionRequestId),
    PoliceArrival(PoliceResponseId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAbortArtifacts {
    information: InformationId,
    report: ReportId,
    history_event: HistoryEventId,
}

impl OperationAbortArtifacts {
    pub fn information(self) -> InformationId {
        self.information
    }

    pub fn report(self) -> ReportId {
        self.report
    }

    pub fn history_event(self) -> HistoryEventId {
        self.history_event
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAbortRecord {
    aborted_at: SimTime,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    artifacts: Option<OperationAbortArtifacts>,
}

impl OperationAbortRecord {
    pub fn aborted_at(self) -> SimTime {
        self.aborted_at
    }

    pub fn phase(self) -> OperationAbortPhase {
        self.phase
    }

    pub fn cause(self) -> OperationAbortCause {
        self.cause
    }

    pub fn artifacts(self) -> Option<OperationAbortArtifacts> {
        self.artifacts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationObjectiveOutcome {
    Achieved,
    Partial,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationExposureLevel {
    None,
    Trace,
    Witnessed,
    Identifying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationExposureFactors {
    stealth_average: Rating,
    target_police_presence: Option<Rating>,
    police_response_arrived: bool,
    approach_adjustment: i8,
    intelligence_mitigation: u8,
    variance: i8,
}

impl OperationExposureFactors {
    pub fn stealth_average(self) -> Rating {
        self.stealth_average
    }

    pub fn target_police_presence(self) -> Option<Rating> {
        self.target_police_presence
    }

    pub fn police_response_arrived(self) -> bool {
        self.police_response_arrived
    }

    pub fn approach_adjustment(self) -> i8 {
        self.approach_adjustment
    }

    pub fn intelligence_mitigation(self) -> u8 {
        self.intelligence_mitigation
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationExposureRecord {
    level: OperationExposureLevel,
    score: i16,
    factors: OperationExposureFactors,
    neighborhood: Option<NeighborhoodId>,
    identified_character: Option<CharacterId>,
    investigation: Option<InvestigationId>,
    evidence: BTreeSet<EvidenceId>,
}

impl OperationExposureRecord {
    pub fn level(&self) -> OperationExposureLevel {
        self.level
    }

    pub fn score(&self) -> i16 {
        self.score
    }

    pub fn factors(&self) -> OperationExposureFactors {
        self.factors
    }

    pub fn neighborhood(&self) -> Option<NeighborhoodId> {
        self.neighborhood
    }

    pub fn identified_character(&self) -> Option<CharacterId> {
        self.identified_character
    }

    pub fn investigation(&self) -> Option<InvestigationId> {
        self.investigation
    }

    pub fn evidence(&self) -> &BTreeSet<EvidenceId> {
        &self.evidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResolutionFactors {
    role_capability_average: Rating,
    leader_management: Option<Rating>,
    intelligence_quality: Rating,
    intelligence_adjustment: i8,
    target_police_presence: Option<Rating>,
    police_response_arrived: bool,
    approach_adjustment: i8,
    time_pressure: u8,
    variance: i8,
}

impl OperationResolutionFactors {
    pub fn role_capability_average(self) -> Rating {
        self.role_capability_average
    }

    pub fn leader_management(self) -> Option<Rating> {
        self.leader_management
    }

    pub fn intelligence_quality(self) -> Rating {
        self.intelligence_quality
    }

    pub fn intelligence_adjustment(self) -> i8 {
        self.intelligence_adjustment
    }

    pub fn target_police_presence(self) -> Option<Rating> {
        self.target_police_presence
    }

    pub fn police_response_arrived(self) -> bool {
        self.police_response_arrived
    }

    pub fn approach_adjustment(self) -> i8 {
        self.approach_adjustment
    }

    pub fn time_pressure(self) -> u8 {
        self.time_pressure
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationResolutionRecord {
    resolved_at: SimTime,
    objective_outcome: OperationObjectiveOutcome,
    execution_margin: i16,
    factors: OperationResolutionFactors,
    exposure: OperationExposureRecord,
    discovered_information: BTreeSet<InformationId>,
    after_action_information: InformationId,
    after_action_report: ReportId,
    history_event: HistoryEventId,
}

impl OperationResolutionRecord {
    pub fn resolved_at(&self) -> SimTime {
        self.resolved_at
    }

    pub fn objective_outcome(&self) -> OperationObjectiveOutcome {
        self.objective_outcome
    }

    pub fn execution_margin(&self) -> i16 {
        self.execution_margin
    }

    pub fn factors(&self) -> OperationResolutionFactors {
        self.factors
    }

    pub fn exposure(&self) -> &OperationExposureRecord {
        &self.exposure
    }

    pub fn discovered_information(&self) -> &BTreeSet<InformationId> {
        &self.discovered_information
    }

    pub fn after_action_information(&self) -> InformationId {
        self.after_action_information
    }

    pub fn after_action_report(&self) -> ReportId {
        self.after_action_report
    }

    pub fn history_event(&self) -> HistoryEventId {
        self.history_event
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationIdentity {
    id: OperationId,
    title: String,
    kind: OperationKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationCommand {
    responsible_organization: OrganizationId,
    leader: CharacterId,
    objective: OperationObjective,
    approach: OperationApproach,
    roles: BTreeMap<RoleKind, CharacterId>,
    intelligence: BTreeSet<InformationId>,
    constraints: Vec<OperationConstraint>,
    contingencies: Vec<OperationContingency>,
    scheduled_for: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationRuntime {
    status: OperationStatus,
    started_at: Option<SimTime>,
    resolution_due_at: Option<SimTime>,
    entry_at: Option<SimTime>,
    police_response: Option<PoliceResponseId>,
    awaiting_decision_since: Option<SimTime>,
    resolution: Option<OperationResolutionRecord>,
    abort: Option<OperationAbortRecord>,
    version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationRecord {
    identity: OperationIdentity,
    command: OperationCommand,
    runtime: OperationRuntime,
}

impl OperationRecord {
    pub fn id(&self) -> OperationId {
        self.identity.id
    }

    pub fn title(&self) -> &str {
        &self.identity.title
    }

    pub fn kind(&self) -> OperationKind {
        self.identity.kind
    }

    pub fn responsible_organization(&self) -> OrganizationId {
        self.command.responsible_organization
    }

    pub fn leader(&self) -> CharacterId {
        self.command.leader
    }

    pub fn objective(&self) -> &OperationObjective {
        &self.command.objective
    }

    pub fn approach(&self) -> OperationApproach {
        self.command.approach
    }

    pub fn roles(&self) -> &BTreeMap<RoleKind, CharacterId> {
        &self.command.roles
    }

    pub fn intelligence(&self) -> &BTreeSet<InformationId> {
        &self.command.intelligence
    }

    pub fn constraints(&self) -> &[OperationConstraint] {
        &self.command.constraints
    }

    pub fn contingencies(&self) -> &[OperationContingency] {
        &self.command.contingencies
    }

    pub fn scheduled_for(&self) -> SimTime {
        self.command.scheduled_for
    }

    pub fn status(&self) -> OperationStatus {
        self.runtime.status
    }

    pub fn started_at(&self) -> Option<SimTime> {
        self.runtime.started_at
    }

    pub fn resolution_due_at(&self) -> Option<SimTime> {
        self.runtime.resolution_due_at
    }

    pub fn entry_at(&self) -> Option<SimTime> {
        self.runtime.entry_at
    }

    pub fn police_response(&self) -> Option<PoliceResponseId> {
        self.runtime.police_response
    }

    pub fn awaiting_decision_since(&self) -> Option<SimTime> {
        self.runtime.awaiting_decision_since
    }

    pub fn resolution(&self) -> Option<&OperationResolutionRecord> {
        self.runtime.resolution.as_ref()
    }

    pub fn abort_record(&self) -> Option<OperationAbortRecord> {
        self.runtime.abort
    }

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OperationState {
    records: BTreeMap<OperationId, OperationRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<OperationId>>,
    by_status: BTreeMap<OperationStatus, BTreeSet<OperationId>>,
    by_discovered_information: BTreeMap<InformationId, OperationId>,
    authorized_by_start: BTreeMap<SimTime, BTreeSet<OperationId>>,
    in_progress_by_resolution_due: BTreeMap<SimTime, BTreeSet<OperationId>>,
}

impl OperationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_operation(&self, id: OperationId) -> Option<&OperationRecord> {
        self.records.get(&id)
    }

    pub fn operations_for_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.by_organization
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|operation_id| self.records.get(operation_id))
    }

    pub fn operation_for_discovered_information(
        &self,
        information: InformationId,
    ) -> Option<&OperationRecord> {
        self.by_discovered_information
            .get(&information)
            .and_then(|operation| self.records.get(operation))
    }

    pub fn operations_with_status(
        &self,
        status: OperationStatus,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.by_status
            .get(&status)
            .into_iter()
            .flatten()
            .filter_map(|operation_id| self.records.get(operation_id))
    }

    pub(crate) fn operations(&self) -> impl Iterator<Item = &OperationRecord> {
        self.records.values()
    }

    pub(crate) fn due_authorized_at_or_before(&self, now: SimTime) -> Vec<OperationId> {
        self.authorized_by_start
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn due_in_progress_at_or_before(&self, now: SimTime) -> Vec<OperationId> {
        self.in_progress_by_resolution_due
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn insert(&mut self, record: OperationRecord) {
        let id = record.id();
        debug_assert_eq!(
            record.status(),
            OperationStatus::Authorized,
            "new operations must enter state as authorized"
        );
        self.by_organization
            .entry(record.responsible_organization())
            .or_default()
            .insert(id);
        self.by_status
            .entry(record.status())
            .or_default()
            .insert(id);
        self.authorized_by_start
            .entry(record.scheduled_for())
            .or_default()
            .insert(id);
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate operation ID inserted"
        );
    }

    pub(crate) fn begin(
        &mut self,
        id: OperationId,
        started_at: SimTime,
        resolution_due_at: SimTime,
        entry_at: Option<SimTime>,
        police_response: Option<PoliceResponseId>,
    ) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before begin commit");
        assert_eq!(
            record.status(),
            OperationStatus::Authorized,
            "only authorized operations may begin"
        );
        let scheduled_for = record.scheduled_for();
        Self::remove_schedule_index(&mut self.authorized_by_start, scheduled_for, id);
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before begin commit");
            record.runtime.started_at = Some(started_at);
            record.runtime.resolution_due_at = Some(resolution_due_at);
            record.runtime.entry_at = entry_at;
            record.runtime.police_response = police_response;
            record.runtime.awaiting_decision_since = None;
        }
        self.change_status(id, OperationStatus::InProgress);
        self.in_progress_by_resolution_due
            .entry(resolution_due_at)
            .or_default()
            .insert(id);
    }

    pub(crate) fn set_awaiting_decision(&mut self, id: OperationId, paused_at: SimTime) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before decision wait commit");
        assert_eq!(
            record.status(),
            OperationStatus::InProgress,
            "only in-progress operations may await a decision"
        );
        let due_at = record
            .resolution_due_at()
            .expect("in-progress operation must have a resolution due time");
        Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
        self.records
            .get_mut(&id)
            .expect("validated operation disappeared before decision wait commit")
            .runtime
            .awaiting_decision_since = Some(paused_at);
        self.change_status(id, OperationStatus::AwaitingDecision);
    }

    pub(crate) fn resume(&mut self, id: OperationId, resumed_at: SimTime) {
        let (due_at, entry_at, paused_at) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before resume commit");
            assert_eq!(
                record.status(),
                OperationStatus::AwaitingDecision,
                "only decision-blocked operations may resume"
            );
            (
                record
                    .resolution_due_at()
                    .expect("awaiting operation must retain its resolution due time"),
                record.entry_at(),
                record
                    .awaiting_decision_since()
                    .expect("awaiting operation must retain its pause time"),
            )
        };
        let paused_minutes = resumed_at
            .as_minutes()
            .checked_sub(paused_at.as_minutes())
            .expect("operation cannot resume before its decision pause began");
        let shifted_due_at = SimTime::from_minutes(
            due_at
                .as_minutes()
                .checked_add(paused_minutes)
                .expect("operation resolution time overflowed u64 minutes"),
        );
        let shifted_entry_at = entry_at.map(|entry_at| {
            if entry_at > paused_at {
                SimTime::from_minutes(
                    entry_at
                        .as_minutes()
                        .checked_add(paused_minutes)
                        .expect("operation entry time overflowed u64 minutes"),
                )
            } else {
                entry_at
            }
        });
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before resume commit");
            record.runtime.resolution_due_at = Some(shifted_due_at);
            record.runtime.entry_at = shifted_entry_at;
            record.runtime.awaiting_decision_since = None;
        }
        self.change_status(id, OperationStatus::InProgress);
        self.in_progress_by_resolution_due
            .entry(shifted_due_at)
            .or_default()
            .insert(id);
    }

    pub(crate) fn abort(&mut self, id: OperationId, abort: OperationAbortRecord) {
        let (status, scheduled_for, due_at) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before abort commit");
            (
                record.status(),
                record.scheduled_for(),
                record.resolution_due_at(),
            )
        };
        assert!(
            matches!(
                status,
                OperationStatus::Authorized
                    | OperationStatus::InProgress
                    | OperationStatus::AwaitingDecision
            ),
            "only active operations may abort"
        );
        match status {
            OperationStatus::Authorized => {
                Self::remove_schedule_index(&mut self.authorized_by_start, scheduled_for, id);
            }
            OperationStatus::InProgress => {
                let due_at = due_at.expect("in-progress operation must have a resolution due time");
                Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
            }
            OperationStatus::AwaitingDecision
            | OperationStatus::Completed
            | OperationStatus::Aborted => {}
        }
        if abort.phase() != OperationAbortPhase::AwaitingDecision {
            self.records
                .get_mut(&id)
                .expect("validated operation disappeared before abort commit")
                .runtime
                .awaiting_decision_since = None;
        }
        self.records
            .get_mut(&id)
            .expect("validated operation disappeared before abort commit")
            .runtime
            .abort = Some(abort);
        self.change_status(id, OperationStatus::Aborted);
    }

    pub(crate) fn complete(&mut self, id: OperationId, resolution: OperationResolutionRecord) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before completion commit");
        assert_eq!(
            record.status(),
            OperationStatus::InProgress,
            "only in-progress operations may complete"
        );
        assert!(
            record.abort_record().is_none(),
            "completed operations cannot retain an abort record"
        );
        let due_at = record
            .resolution_due_at()
            .expect("in-progress operation must have a resolution due time");
        for information in resolution.discovered_information() {
            let previous = self.by_discovered_information.insert(*information, id);
            debug_assert!(
                previous.is_none(),
                "Ownership Exclusivity: discovered information is linked to multiple operations"
            );
        }
        Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before completion commit");
            record.runtime.resolution = Some(resolution);
            record.runtime.awaiting_decision_since = None;
        }
        self.change_status(id, OperationStatus::Completed);
    }

    fn change_status(&mut self, id: OperationId, next: OperationStatus) {
        let previous = self
            .records
            .get(&id)
            .expect("validated operation disappeared before status commit")
            .status();
        if let Some(ids) = self.by_status.get_mut(&previous) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_status.remove(&previous);
            }
        }
        let record = self
            .records
            .get_mut(&id)
            .expect("validated operation disappeared before status commit");
        record.runtime.status = next;
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("operation version counter exhausted");
        self.by_status.entry(next).or_default().insert(id);
    }

    fn remove_schedule_index(
        index: &mut BTreeMap<SimTime, BTreeSet<OperationId>>,
        time: SimTime,
        id: OperationId,
    ) {
        if let Some(ids) = index.get_mut(&time) {
            ids.remove(&id);
            if ids.is_empty() {
                index.remove(&time);
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            if !self
                .by_organization
                .get(&record.responsible_organization())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            if !self
                .by_status
                .get(&record.status())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            let authorized_indexed = self
                .authorized_by_start
                .get(&record.scheduled_for())
                .is_some_and(|ids| ids.contains(&record.id()));
            if authorized_indexed != (record.status() == OperationStatus::Authorized) {
                return false;
            }
            let resolution_indexed = record.resolution_due_at().is_some_and(|due_at| {
                self.in_progress_by_resolution_due
                    .get(&due_at)
                    .is_some_and(|ids| ids.contains(&record.id()))
            });
            if resolution_indexed != (record.status() == OperationStatus::InProgress) {
                return false;
            }
            if let Some(resolution) = record.resolution() {
                for information in resolution.discovered_information() {
                    if self.by_discovered_information.get(information) != Some(&record.id()) {
                        return false;
                    }
                }
            }
        }
        for (organization, ids) in &self.by_organization {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.responsible_organization() == *organization)
                {
                    return false;
                }
            }
        }
        for (information, operation) in &self.by_discovered_information {
            if !self.records.get(operation).is_some_and(|record| {
                record.resolution().is_some_and(|resolution| {
                    resolution.discovered_information().contains(information)
                })
            }) {
                return false;
            }
        }
        for (status, ids) in &self.by_status {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.status() == *status)
                {
                    return false;
                }
            }
        }
        for (time, ids) in &self.authorized_by_start {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == OperationStatus::Authorized
                        && record.scheduled_for() == *time
                }) {
                    return false;
                }
            }
        }
        for (time, ids) in &self.in_progress_by_resolution_due {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == OperationStatus::InProgress
                        && record.resolution_due_at() == Some(*time)
                }) {
                    return false;
                }
            }
        }
        true
    }

    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: operation indexes disagree with source records"
        );
        for record in self.records.values() {
            debug_assert!(
                self.by_organization
                    .get(&record.responsible_organization())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: operation organization index is missing an operation"
            );
            debug_assert!(
                self.by_status
                    .get(&record.status())
                    .is_some_and(|ids| ids.contains(&record.id())),
                "Index Completeness: operation status index is missing an operation"
            );
            if let Some(resolution) = record.resolution() {
                for information in resolution.discovered_information() {
                    debug_assert_eq!(
                        self.by_discovered_information.get(information),
                        Some(&record.id()),
                        "Index Completeness: operation discovery index is missing information"
                    );
                }
            }
        }
        for (status, ids) in &self.by_status {
            for id in ids {
                let record = self
                    .records
                    .get(id)
                    .expect("Index Completeness: operation status index points to missing record");
                debug_assert_eq!(
                    record.status(),
                    *status,
                    "Derived Data Consistency: operation status index disagrees with record"
                );
            }
        }
        for record in self.records.values() {
            debug_assert_eq!(
                self.authorized_by_start
                    .get(&record.scheduled_for())
                    .is_some_and(|ids| ids.contains(&record.id())),
                record.status() == OperationStatus::Authorized,
                "Derived Data Consistency: operation authorization schedule disagrees with lifecycle"
            );
            debug_assert_eq!(
                record.resolution_due_at().is_some_and(|due_at| {
                    self.in_progress_by_resolution_due
                        .get(&due_at)
                        .is_some_and(|ids| ids.contains(&record.id()))
                }),
                record.status() == OperationStatus::InProgress,
                "Derived Data Consistency: operation resolution schedule disagrees with lifecycle"
            );
        }
    }
}

#[derive(Clone, Debug)]
pub struct OperationDraft {
    pub title: String,
    pub kind: OperationKind,
    pub responsible_organization: OrganizationId,
    pub leader: CharacterId,
    pub objective: OperationObjective,
    pub approach: OperationApproach,
    pub roles: BTreeMap<RoleKind, CharacterId>,
    pub intelligence: BTreeSet<InformationId>,
    pub constraints: Vec<OperationConstraint>,
    pub contingencies: Vec<OperationContingency>,
    pub scheduled_for: SimTime,
}
