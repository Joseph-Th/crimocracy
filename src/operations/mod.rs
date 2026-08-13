//! Semantic operation plans and lifecycle state; `operation_system` validates and commits all changes.

pub mod operation_system;

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OperationId, OrganizationId};
use crate::core::time::SimTime;
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
    constraints: Vec<OperationConstraint>,
    contingencies: Vec<OperationContingency>,
    scheduled_for: SimTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OperationRuntime {
    status: OperationStatus,
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

    pub fn version(&self) -> u32 {
        self.runtime.version
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OperationState {
    records: BTreeMap<OperationId, OperationRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<OperationId>>,
    by_status: BTreeMap<OperationStatus, BTreeSet<OperationId>>,
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

    pub(crate) fn insert(&mut self, record: OperationRecord) {
        let id = record.id();
        self.by_organization
            .entry(record.responsible_organization())
            .or_default()
            .insert(id);
        self.by_status
            .entry(record.status())
            .or_default()
            .insert(id);
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate operation ID inserted"
        );
    }

    pub(crate) fn transition(&mut self, id: OperationId, next: OperationStatus) {
        let record = self
            .records
            .get_mut(&id)
            .expect("validated operation disappeared before transition commit");
        let previous = record.runtime.status;
        if let Some(ids) = self.by_status.get_mut(&previous) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_status.remove(&previous);
            }
        }
        record.runtime.status = next;
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("operation version counter exhausted");
        self.by_status.entry(next).or_default().insert(id);
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
    pub constraints: Vec<OperationConstraint>,
    pub contingencies: Vec<OperationContingency>,
    pub scheduled_for: SimTime,
}
