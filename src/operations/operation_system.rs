//! Operation validation and lifecycle execution; sibling records contain no resolution logic.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    ArrestId, CharacterId, DecisionRequestId, IdExhaustionError, IdKind, InformationId,
    OperationId, OrganizationId, PoliceResponseId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::history::history_system::{validate_record_event, HistoryError, ValidatedHistoryEvent};
use crate::history::{HistoryEventDraft, HistoryEventKind};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::operations::police_response_integration::{
    decide_operation_police_response_start, OperationPoliceResponseStartPlan,
};
use crate::operations::surveillance_integration::validate_surveillance_request;
use crate::operations::{
    OperationAbortArtifacts, OperationAbortCause, OperationAbortPhase, OperationAbortRecord,
    OperationCommand, OperationDraft, OperationIdentity, OperationKind, OperationObjective,
    OperationObjectiveKind, OperationRecord, OperationRuntime, OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::Lifecycle;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationTransition {
    Begin,
    Abort,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OperationError {
    #[error("operation title must not be empty")]
    EmptyTitle,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("information {information} is not held by responsible organization {organization}")]
    InformationUnavailable {
        information: InformationId,
        organization: OrganizationId,
    },
    #[error("information {0} is not relevant to this operation plan")]
    IrrelevantInformation(InformationId),
    #[error(
        "character {leader} is not an active member of responsible organization {organization}"
    )]
    InvalidLeader {
        leader: CharacterId,
        organization: OrganizationId,
    },
    #[error("character {0} assigned to an operation is not active")]
    InactiveParticipant(CharacterId),
    #[error(
        "character {character} assigned to an operation belongs to organization {actual:?}, not {expected}"
    )]
    ForeignParticipant {
        character: CharacterId,
        expected: OrganizationId,
        actual: Option<OrganizationId>,
    },
    #[error("character {character} is detained under arrest {arrest} and cannot participate")]
    DetainedParticipant {
        character: CharacterId,
        arrest: ArrestId,
    },
    #[error(
        "character {character} changed after operation validation; expected version {expected}, found {found}"
    )]
    StaleParticipant {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "operation authorization expired at simulation minute {scheduled_for}; current minute is {now}"
    )]
    AuthorizationExpired { scheduled_for: u64, now: u64 },
    #[error("operation approach is not supported by the operation definition")]
    UnsupportedApproach,
    #[error("surveillance operations require a gather-information objective")]
    InvalidSurveillanceObjective,
    #[error("entity {0:?} cannot be directly observed by a surveillance operation")]
    UnsupportedSurveillanceTarget(EntityRef),
    #[error("operation objective {objective:?} is not supported by operation kind {kind:?}")]
    InvalidObjectiveForKind {
        kind: OperationKind,
        objective: OperationObjectiveKind,
    },
    #[error("property-acquisition objective target {0:?} is not a business")]
    InvalidPropertyTarget(EntityRef),
    #[error("operation objective target {0:?} is inactive")]
    InactiveObjectiveTarget(EntityRef),
    #[error(
        "character {character} is assigned to multiple operation roles: {first_role:?} and {second_role:?}"
    )]
    DuplicateRoleParticipant {
        character: CharacterId,
        first_role: RoleKind,
        second_role: RoleKind,
    },
    #[error("operation is missing required role {0:?}")]
    MissingRequiredRole(RoleKind),
    #[error("operation is scheduled in the past")]
    ScheduledInPast,
    #[error("operation completion deadline is earlier than its scheduled start")]
    DeadlineBeforeStart,
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} cannot begin before its scheduled time")]
    StartBeforeScheduled(OperationId),
    #[error("police response context for operation {0} could not be validated")]
    InvalidPoliceResponseContext(OperationId),
    #[error(
        "operation {0:?} does not define an entry milestone for the police-arrival contingency"
    )]
    UnsupportedPoliceEntryContingency(crate::operations::OperationKind),
    #[error("transition {transition:?} is invalid from status {status:?}")]
    InvalidTransition {
        status: OperationStatus,
        transition: OperationTransition,
    },
    #[error(
        "operation {operation} changed after abort validation; expected version {expected}, found {found}"
    )]
    StaleAbortOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "operation {operation} abort was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleAbortTime {
        operation: OperationId,
        expected: SimTime,
        found: SimTime,
    },
    #[error("operation {operation} missed its completion deadline at {deadline:?} before it could begin at {now:?}")]
    DeadlineMissed {
        operation: OperationId,
        deadline: SimTime,
        now: SimTime,
    },
    #[error("operation {operation} has not missed a completion deadline")]
    DeadlineNotMissed { operation: OperationId },
    #[error("operation {operation} cannot use abort cause {cause:?} from status {status:?}")]
    InvalidAbortCause {
        operation: OperationId,
        status: OperationStatus,
        cause: OperationAbortCause,
    },
    #[error("operation {operation} abort artifacts could not be validated against current state")]
    InvalidAbortArtifacts { operation: OperationId },
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug)]
pub struct ValidatedOperation {
    draft: OperationDraft,
    expected_participant_versions: BTreeMap<CharacterId, u32>,
}

impl ValidatedOperation {
    pub fn commit(self, state: &mut AppState) -> Result<OperationId, OperationError> {
        if state.now() > self.draft.scheduled_for {
            return Err(OperationError::AuthorizationExpired {
                scheduled_for: self.draft.scheduled_for.as_minutes(),
                now: state.now().as_minutes(),
            });
        }
        let organization = state
            .world
            .get_organization(self.draft.responsible_organization)
            .ok_or(OperationError::MissingOrganization(
                self.draft.responsible_organization,
            ))?;
        if organization.lifecycle() != Lifecycle::Active {
            return Err(OperationError::MissingOrganization(
                self.draft.responsible_organization,
            ));
        }
        for (participant, expected) in &self.expected_participant_versions {
            let record = state
                .world
                .get_character(*participant)
                .ok_or(OperationError::MissingCharacter(*participant))?;
            if record.version() != *expected {
                return Err(OperationError::StaleParticipant {
                    character: *participant,
                    expected: *expected,
                    found: record.version(),
                });
            }
            if record.lifecycle() != Lifecycle::Active {
                return Err(OperationError::InactiveParticipant(*participant));
            }
            if record.organization() != Some(self.draft.responsible_organization) {
                return Err(OperationError::ForeignParticipant {
                    character: *participant,
                    expected: self.draft.responsible_organization,
                    actual: record.organization(),
                });
            }
            if let Some(arrest) = state.legal.active_arrest_for_character(*participant) {
                return Err(OperationError::DetainedParticipant {
                    character: *participant,
                    arrest: arrest.id(),
                });
            }
        }
        let leader = state
            .world
            .get_character(self.draft.leader)
            .ok_or(OperationError::MissingCharacter(self.draft.leader))?;
        if leader.organization() != Some(self.draft.responsible_organization) {
            return Err(OperationError::InvalidLeader {
                leader: self.draft.leader,
                organization: self.draft.responsible_organization,
            });
        }

        // Revalidate plan dependencies that can change independently of participant versions: a
        // target business or neighborhood lifecycle change, or intelligence transferred out of the
        // organization between validation and commit, must stale the authorization. Topic and
        // objective-shape relevance are invariant here because intelligence topics are immutable
        // and the authored operation definition is static.
        validate_operation_objective(state, self.draft.kind, &self.draft.objective)?;
        for information in &self.draft.intelligence {
            let record = state
                .intelligence
                .get_information(*information)
                .ok_or(OperationError::MissingInformation(*information))?;
            if record.holder() != KnowledgeHolder::Organization(self.draft.responsible_organization)
            {
                return Err(OperationError::InformationUnavailable {
                    information: *information,
                    organization: self.draft.responsible_organization,
                });
            }
        }

        let OperationDraft {
            title,
            kind,
            responsible_organization,
            leader,
            objective,
            approach,
            roles,
            intelligence,
            constraints,
            contingencies,
            scheduled_for,
        } = self.draft;
        let id = state.ids.next_operation()?;
        state.operations.insert(OperationRecord {
            identity: OperationIdentity { id, title, kind },
            command: OperationCommand {
                responsible_organization,
                leader,
                objective,
                approach,
                roles,
                intelligence,
                constraints,
                contingencies,
                scheduled_for,
            },
            runtime: OperationRuntime {
                status: OperationStatus::Authorized,
                started_at: None,
                resolution_due_at: None,
                entry_at: None,
                police_response: None,
                awaiting_decision_since: None,
                resolution: None,
                property_disposition: None,
                abort: None,
                version: 1,
            },
        });
        Ok(id)
    }
}

pub fn validate_authorize_operation(
    registry: &Registry,
    state: &AppState,
    draft: OperationDraft,
) -> Result<ValidatedOperation, OperationError> {
    if draft.title.trim().is_empty() {
        return Err(OperationError::EmptyTitle);
    }
    let organization = state
        .world
        .get_organization(draft.responsible_organization)
        .ok_or(OperationError::MissingOrganization(
            draft.responsible_organization,
        ))?;
    if organization.lifecycle() != Lifecycle::Active {
        return Err(OperationError::MissingOrganization(
            draft.responsible_organization,
        ));
    }
    let leader = state
        .world
        .get_character(draft.leader)
        .ok_or(OperationError::MissingCharacter(draft.leader))?;
    if leader.lifecycle() != Lifecycle::Active
        || leader.organization() != Some(draft.responsible_organization)
    {
        return Err(OperationError::InvalidLeader {
            leader: draft.leader,
            organization: draft.responsible_organization,
        });
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(draft.leader) {
        return Err(OperationError::DetainedParticipant {
            character: draft.leader,
            arrest: arrest.id(),
        });
    }
    if draft.scheduled_for < state.now() {
        return Err(OperationError::ScheduledInPast);
    }
    validate_surveillance_request(draft.kind, &draft.objective).map_err(|error| match error {
        crate::operations::surveillance_integration::SurveillanceError::InvalidObjective => {
            OperationError::InvalidSurveillanceObjective
        }
        crate::operations::surveillance_integration::SurveillanceError::UnsupportedTarget(
            target,
        ) => OperationError::UnsupportedSurveillanceTarget(target),
        crate::operations::surveillance_integration::SurveillanceError::MissingTarget(_)
        | crate::operations::surveillance_integration::SurveillanceError::StaleTarget(_) => {
            unreachable!("authorization validates target existence through the operation objective")
        }
    })?;
    validate_operation_objective(state, draft.kind, &draft.objective)?;
    let mut expected_participant_versions = BTreeMap::from([(draft.leader, leader.version())]);

    let definition = registry.get_operation(draft.kind);
    if !definition.supported_approaches().contains(&draft.approach) {
        return Err(OperationError::UnsupportedApproach);
    }
    for role in definition.required_roles() {
        if !draft.roles.contains_key(role) {
            return Err(OperationError::MissingRequiredRole(*role));
        }
    }
    let mut role_participants = BTreeMap::new();
    for (role, participant) in &draft.roles {
        if let Some(first_role) = role_participants.insert(*participant, *role) {
            return Err(OperationError::DuplicateRoleParticipant {
                character: *participant,
                first_role,
                second_role: *role,
            });
        }
        let record = state
            .world
            .get_character(*participant)
            .ok_or(OperationError::MissingCharacter(*participant))?;
        if record.lifecycle() != Lifecycle::Active {
            return Err(OperationError::InactiveParticipant(*participant));
        }
        if record.organization() != Some(draft.responsible_organization) {
            return Err(OperationError::ForeignParticipant {
                character: *participant,
                expected: draft.responsible_organization,
                actual: record.organization(),
            });
        }
        if let Some(arrest) = state.legal.active_arrest_for_character(*participant) {
            return Err(OperationError::DetainedParticipant {
                character: *participant,
                arrest: arrest.id(),
            });
        }
        expected_participant_versions.insert(*participant, record.version());
    }
    for information in &draft.intelligence {
        let record = state
            .intelligence
            .get_information(*information)
            .ok_or(OperationError::MissingInformation(*information))?;
        if record.holder() != KnowledgeHolder::Organization(draft.responsible_organization) {
            return Err(OperationError::InformationUnavailable {
                information: *information,
                organization: draft.responsible_organization,
            });
        }
        if !definition
            .execution()
            .relevant_intelligence_topics()
            .contains(&record.topic())
            || !is_information_subject_relevant(state, &draft.objective, record.subject())
        {
            return Err(OperationError::IrrelevantInformation(*information));
        }
    }
    for entity in draft.objective.referenced_entities() {
        if !is_entity_present(state, entity) {
            return Err(OperationError::MissingEntity(entity));
        }
    }
    for constraint in &draft.constraints {
        let crate::operations::OperationConstraint::CompleteBefore(deadline) = constraint;
        if *deadline <= draft.scheduled_for {
            return Err(OperationError::DeadlineBeforeStart);
        }
    }
    for contingency in &draft.contingencies {
        match contingency {
            crate::operations::OperationContingency::AbortOnPoliceArrivalBeforeEntry
                if definition.execution().operation_entry_offset().is_none() =>
            {
                return Err(OperationError::UnsupportedPoliceEntryContingency(
                    draft.kind,
                ));
            }
            crate::operations::OperationContingency::AbortOnPoliceArrivalBeforeEntry
            | crate::operations::OperationContingency::RequestDecisionOnUnexpectedCondition => {}
        }
    }

    Ok(ValidatedOperation {
        draft,
        expected_participant_versions,
    })
}

pub(crate) fn is_information_subject_relevant(
    state: &AppState,
    objective: &crate::operations::OperationObjective,
    subject: EntityRef,
) -> bool {
    let referenced = objective.referenced_entities();
    if referenced.contains(&subject) {
        return true;
    }
    let EntityRef::Neighborhood(subject_neighborhood) = subject else {
        return false;
    };
    referenced.into_iter().any(|entity| match entity {
        EntityRef::Business(business) => state
            .world
            .get_business(business)
            .is_some_and(|record| record.neighborhood() == subject_neighborhood),
        EntityRef::Neighborhood(neighborhood) => neighborhood == subject_neighborhood,
        EntityRef::Organization(_)
        | EntityRef::Character(_)
        | EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_)
        | EntityRef::Enterprise(_) => false,
    })
}

pub(crate) fn is_valid_operation_objective(
    kind: OperationKind,
    objective: &OperationObjective,
) -> bool {
    match objective {
        OperationObjective::AcquireProperty { target } => {
            matches!(target, EntityRef::Business(_))
        }
        OperationObjective::GatherInformation { target } => {
            kind == OperationKind::Surveillance
                && crate::operations::surveillance_integration::is_supported_surveillance_target(
                    *target,
                )
        }
        OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::DestroyEquipment { .. }
        | OperationObjective::MoveContraband { .. }
        | OperationObjective::RemovePerson { .. } => kind != OperationKind::Surveillance,
    }
}

fn validate_operation_objective(
    state: &AppState,
    kind: OperationKind,
    objective: &OperationObjective,
) -> Result<(), OperationError> {
    if let OperationObjective::AcquireProperty { target } = objective {
        let EntityRef::Business(business) = target else {
            return Err(OperationError::InvalidPropertyTarget(*target));
        };
        let business_record = state
            .world
            .get_business(*business)
            .ok_or(OperationError::MissingEntity(*target))?;
        if business_record.lifecycle() != Lifecycle::Active {
            return Err(OperationError::InactiveObjectiveTarget(*target));
        }
        let neighborhood = state
            .world
            .get_neighborhood(business_record.neighborhood())
            .ok_or(OperationError::MissingEntity(EntityRef::Neighborhood(
                business_record.neighborhood(),
            )))?;
        if neighborhood.lifecycle() != Lifecycle::Active {
            return Err(OperationError::InactiveObjectiveTarget(
                EntityRef::Neighborhood(business_record.neighborhood()),
            ));
        }
    }
    if kind != OperationKind::Surveillance
        && objective.kind() == OperationObjectiveKind::GatherInformation
    {
        return Err(OperationError::InvalidObjectiveForKind {
            kind,
            objective: objective.kind(),
        });
    }
    Ok(())
}

pub(crate) fn due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state.operations.due_authorized_at_or_before(state.now())
}

pub(crate) fn has_missed_operation_deadline(state: &AppState, operation: OperationId) -> bool {
    state
        .operations
        .get_operation(operation)
        .and_then(earliest_operation_deadline)
        .is_some_and(|deadline| state.now() >= deadline)
}

pub(crate) fn validate_deadline_missed_operation(
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationAbort, OperationError> {
    if !has_missed_operation_deadline(state, operation) {
        return Err(OperationError::DeadlineNotMissed { operation });
    }
    validate_operation_abort(state, operation, OperationAbortCause::DeadlineMissed)
}

fn earliest_operation_deadline(record: &OperationRecord) -> Option<SimTime> {
    record
        .constraints()
        .iter()
        .map(|constraint| match constraint {
            crate::operations::OperationConstraint::CompleteBefore(deadline) => *deadline,
        })
        .min()
}

pub fn apply_transition(
    registry: &Registry,
    state: &mut AppState,
    operation: OperationId,
    transition: OperationTransition,
) -> Result<(), OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    let status = record.status();
    if transition == OperationTransition::Begin && state.now() < record.scheduled_for() {
        return Err(OperationError::StartBeforeScheduled(operation));
    }
    match (status, transition) {
        (OperationStatus::Authorized, OperationTransition::Begin) => {
            validate_begin_operation(registry, state, operation)?.commit(state)
        }
        (OperationStatus::Authorized, OperationTransition::Abort)
        | (OperationStatus::InProgress, OperationTransition::Abort) => {
            validate_authority_abort_operation(state, operation)?.commit(state)
        }
        (OperationStatus::InProgress, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Abort)
        | (OperationStatus::Completed, OperationTransition::Begin)
        | (OperationStatus::Completed, OperationTransition::Abort)
        | (OperationStatus::Aborted, OperationTransition::Begin)
        | (OperationStatus::Aborted, OperationTransition::Abort) => {
            Err(OperationError::InvalidTransition { status, transition })
        }
    }
}

pub(crate) struct ValidatedOperationStart {
    operation: OperationId,
    expected_version: u32,
    started_at: SimTime,
    resolution_due_at: SimTime,
    police_response: OperationPoliceResponseStartPlan,
}

impl ValidatedOperationStart {
    pub(crate) fn commit(self, state: &mut AppState) -> Result<(), OperationError> {
        let record = state
            .operations
            .get_operation(self.operation)
            .ok_or(OperationError::MissingOperation(self.operation))?;
        if record.version() != self.expected_version
            || record.status() != OperationStatus::Authorized
            || state.now() != self.started_at
        {
            return Err(OperationError::InvalidPoliceResponseContext(self.operation));
        }
        let entry_at = self.police_response.entry_at();
        let response = self
            .police_response
            .commit_dispatch(state)
            .map_err(|_| OperationError::InvalidPoliceResponseContext(self.operation))?;
        state.operations.begin(
            self.operation,
            self.started_at,
            self.resolution_due_at,
            entry_at,
            response,
        );
        Ok(())
    }
}

pub(crate) fn validate_begin_operation(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationStart, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    if record.status() != OperationStatus::Authorized {
        return Err(OperationError::InvalidTransition {
            status: record.status(),
            transition: OperationTransition::Begin,
        });
    }
    if state.now() < record.scheduled_for() {
        return Err(OperationError::StartBeforeScheduled(operation));
    }
    if let Some(deadline) = earliest_operation_deadline(record) {
        if state.now() >= deadline {
            return Err(OperationError::DeadlineMissed {
                operation,
                deadline,
                now: state.now(),
            });
        }
    }
    let duration = registry.get_operation(record.kind()).execution().duration();
    let mut resolution_due_at = state.now() + duration;
    for constraint in record.constraints() {
        let crate::operations::OperationConstraint::CompleteBefore(deadline) = constraint;
        if *deadline < resolution_due_at {
            resolution_due_at = *deadline;
        }
    }
    let police_response = decide_operation_police_response_start(registry, state, operation)
        .map_err(|_| OperationError::InvalidPoliceResponseContext(operation))?;
    Ok(ValidatedOperationStart {
        operation,
        expected_version: record.version(),
        started_at: state.now(),
        resolution_due_at,
        police_response,
    })
}

pub struct ValidatedOperationAbort {
    operation: OperationId,
    expected_operation_version: u32,
    expected_status: OperationStatus,
    aborted_at: SimTime,
    phase: OperationAbortPhase,
    cause: OperationAbortCause,
    information: Option<ValidatedInformation>,
    report: Option<ValidatedReport>,
    history: Option<ValidatedHistoryEvent>,
}

impl ValidatedOperationAbort {
    pub fn commit(self, state: &mut AppState) -> Result<(), OperationError> {
        state.ids.reserve_many(&[
            (IdKind::Information, 1),
            (IdKind::HistoryEvent, 1),
            (IdKind::Report, 1),
        ])?;
        let record = state
            .operations
            .get_operation(self.operation)
            .ok_or(OperationError::MissingOperation(self.operation))?;
        if record.version() != self.expected_operation_version {
            return Err(OperationError::StaleAbortOperation {
                operation: self.operation,
                expected: self.expected_operation_version,
                found: record.version(),
            });
        }
        if record.status() != self.expected_status {
            return Err(OperationError::InvalidAbortCause {
                operation: self.operation,
                status: record.status(),
                cause: self.cause,
            });
        }
        if state.now() != self.aborted_at {
            return Err(OperationError::StaleAbortTime {
                operation: self.operation,
                expected: self.aborted_at,
                found: state.now(),
            });
        }
        if let OperationAbortCause::Decision(decision) = self.cause {
            if state.decisions.pending_for_operation(self.operation) != Some(decision) {
                return Err(OperationError::InvalidAbortCause {
                    operation: self.operation,
                    status: record.status(),
                    cause: self.cause,
                });
            }
        }
        if let OperationAbortCause::PoliceArrival(response) = self.cause {
            if !police_arrival_can_abort(state, record, response) {
                return Err(OperationError::InvalidAbortCause {
                    operation: self.operation,
                    status: record.status(),
                    cause: self.cause,
                });
            }
        }

        let artifacts = match (self.information, self.report, self.history) {
            (None, None, None) => None,
            (Some(information), Some(report), Some(history)) => {
                let information = information.commit(state)?;
                let history_event = history.commit(state)?;
                let report = report.commit(state)?;
                Some(OperationAbortArtifacts {
                    information,
                    report,
                    history_event,
                })
            }
            (None, Some(_), _) | (None, _, Some(_)) | (Some(_), None, _) | (Some(_), _, None) => {
                unreachable!("validated operation abort artifacts are all present or all absent")
            }
        };
        state.operations.abort(
            self.operation,
            OperationAbortRecord {
                aborted_at: self.aborted_at,
                phase: self.phase,
                cause: self.cause,
                artifacts,
            },
        );
        Ok(())
    }
}

pub fn validate_authority_abort_operation(
    state: &AppState,
    operation: OperationId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(state, operation, OperationAbortCause::AuthorityOrder)
}

pub(crate) fn validate_decision_abort_operation(
    state: &AppState,
    operation: OperationId,
    decision: DecisionRequestId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(state, operation, OperationAbortCause::Decision(decision))
}

pub(crate) fn validate_police_arrival_abort_operation(
    state: &AppState,
    operation: OperationId,
    response: PoliceResponseId,
) -> Result<ValidatedOperationAbort, OperationError> {
    validate_operation_abort(
        state,
        operation,
        OperationAbortCause::PoliceArrival(response),
    )
}

pub(crate) fn validate_police_arrival_abort_if_applicable(
    state: &AppState,
    operation: OperationId,
) -> Result<Option<ValidatedOperationAbort>, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    let Some(response) = record.police_response() else {
        return Ok(None);
    };
    if police_arrival_can_abort(state, record, response) {
        validate_police_arrival_abort_operation(state, operation, response).map(Some)
    } else {
        Ok(None)
    }
}

pub(crate) fn projected_resume_entry_at(
    operation: &OperationRecord,
    resumed_at: SimTime,
) -> Option<SimTime> {
    let entry_at = operation.entry_at()?;
    let paused_at = operation.awaiting_decision_since()?;
    if entry_at <= paused_at {
        return Some(entry_at);
    }
    let paused_minutes = resumed_at
        .as_minutes()
        .checked_sub(paused_at.as_minutes())
        .expect("operation cannot resume before its decision pause began");
    Some(SimTime::from_minutes(
        entry_at
            .as_minutes()
            .checked_add(paused_minutes)
            .expect("operation entry time overflowed u64 minutes"),
    ))
}

fn validate_operation_abort(
    state: &AppState,
    operation: OperationId,
    cause: OperationAbortCause,
) -> Result<ValidatedOperationAbort, OperationError> {
    let record = state
        .operations
        .get_operation(operation)
        .ok_or(OperationError::MissingOperation(operation))?;
    let phase = match (record.status(), cause) {
        (OperationStatus::Authorized, OperationAbortCause::AuthorityOrder) => {
            OperationAbortPhase::BeforeStart
        }
        (OperationStatus::Authorized, OperationAbortCause::DeadlineMissed)
            if earliest_operation_deadline(record)
                .is_some_and(|deadline| state.now() >= deadline) =>
        {
            OperationAbortPhase::BeforeStart
        }
        (OperationStatus::InProgress, OperationAbortCause::AuthorityOrder) => {
            OperationAbortPhase::InProgress
        }
        (OperationStatus::AwaitingDecision, OperationAbortCause::Decision(decision))
            if state.decisions.pending_for_operation(operation) == Some(decision) =>
        {
            OperationAbortPhase::AwaitingDecision
        }
        (OperationStatus::InProgress, OperationAbortCause::PoliceArrival(response))
            if police_arrival_can_abort(state, record, response) =>
        {
            OperationAbortPhase::InProgress
        }
        (OperationStatus::AwaitingDecision, OperationAbortCause::PoliceArrival(response))
            if police_arrival_can_abort(state, record, response) =>
        {
            OperationAbortPhase::AwaitingDecision
        }
        (status, cause) => {
            return Err(OperationError::InvalidAbortCause {
                operation,
                status,
                cause,
            });
        }
    };

    let (information, report, history) = match (phase, cause) {
        (OperationAbortPhase::BeforeStart, OperationAbortCause::AuthorityOrder) => {
            (None, None, None)
        }
        (OperationAbortPhase::BeforeStart, OperationAbortCause::DeadlineMissed)
        | (OperationAbortPhase::InProgress, _)
        | (OperationAbortPhase::AwaitingDecision, _) => {
            let summary = build_abort_summary(state, record, cause)?;
            let entities = abort_entities(state, record, cause);
            let information = validate_record_information(
                state,
                InformationDraft {
                    holder: KnowledgeHolder::Organization(record.responsible_organization()),
                    source_kind: InformationSourceKind::AfterAction,
                    topic: InformationTopic::OperationalOutcome,
                    source_entity: Some(EntityRef::Character(record.leader())),
                    subject: EntityRef::Operation(operation),
                    observed_at: state.now(),
                    reliability: Reliability::DirectAccess,
                    specificity: Specificity::Precise,
                    summary: summary.clone(),
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            let report = validate_record_report(
                state,
                ReportDraft {
                    recipient: record.responsible_organization(),
                    kind: ReportKind::AfterAction,
                    title: format!("{} after-action report", record.title()),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Notable,
                        summary: summary.clone(),
                        sources: Vec::new(),
                        entities: entities.clone(),
                        decision: None,
                    }],
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            let history = validate_record_event(
                state,
                HistoryEventDraft {
                    occurred_at: state.now(),
                    kind: HistoryEventKind::Operation,
                    summary,
                    entities,
                },
            )
            .map_err(|_| OperationError::InvalidAbortArtifacts { operation })?;
            (Some(information), Some(report), Some(history))
        }
        (OperationAbortPhase::BeforeStart, OperationAbortCause::Decision(_))
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::PoliceArrival(_)) => {
            unreachable!("pre-start operation aborts cannot use execution-only causes")
        }
    };

    Ok(ValidatedOperationAbort {
        operation,
        expected_operation_version: record.version(),
        expected_status: record.status(),
        aborted_at: state.now(),
        phase,
        cause,
        information,
        report,
        history,
    })
}

fn build_abort_summary(
    state: &AppState,
    operation: &OperationRecord,
    cause: OperationAbortCause,
) -> Result<String, OperationError> {
    match cause {
        OperationAbortCause::AuthorityOrder => Ok(format!(
            "{} was aborted by leadership after execution began. Objective resolution was not completed.",
            operation.title()
        )),
        OperationAbortCause::Decision(decision) => {
            let decision = state
                .decisions
                .get_decision(decision)
                .ok_or(OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                })?;
            Ok(format!(
                "{} was aborted after leadership reviewed an execution exception: {} Objective resolution was not completed.",
                operation.title(),
                decision.summary()
            ))
        }
        OperationAbortCause::PoliceArrival(response) => {
            let response = state
                .legal
                .get_police_response(response)
                .ok_or(OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                })?;
            let authority = state
                .world
                .get_organization(response.authority())
                .ok_or(OperationError::InvalidAbortArtifacts {
                    operation: operation.id(),
                })?;
            Ok(format!(
                "{} was aborted under its standing contingency when {} response reached the target before entry. Objective resolution was not completed.",
                operation.title(),
                authority.name()
            ))
        }
        OperationAbortCause::DeadlineMissed => {
            let deadline = earliest_operation_deadline(operation).expect(
                "validated deadline abort must retain a completion deadline",
            );
            Ok(format!(
                "{} missed its completion deadline at minute {} before execution could begin.",
                operation.title(),
                deadline.as_minutes()
            ))
        }
    }
}

fn abort_entities(
    state: &AppState,
    record: &OperationRecord,
    cause: OperationAbortCause,
) -> BTreeSet<EntityRef> {
    let mut entities = BTreeSet::from([
        EntityRef::Operation(record.id()),
        EntityRef::Organization(record.responsible_organization()),
        EntityRef::Character(record.leader()),
    ]);
    entities.extend(record.objective().referenced_entities());
    entities.extend(record.roles().values().copied().map(EntityRef::Character));
    match cause {
        OperationAbortCause::AuthorityOrder => {}
        OperationAbortCause::Decision(decision) => {
            entities.insert(EntityRef::DecisionRequest(decision));
        }
        OperationAbortCause::PoliceArrival(response) => {
            if let Some(response) = state.legal.get_police_response(response) {
                entities.insert(EntityRef::Organization(response.authority()));
                entities.insert(EntityRef::Neighborhood(response.neighborhood()));
            }
        }
        OperationAbortCause::DeadlineMissed => {}
    }
    entities
}

fn police_arrival_can_abort(
    state: &AppState,
    operation: &OperationRecord,
    response: PoliceResponseId,
) -> bool {
    if operation.police_response() != Some(response)
        || !operation
            .contingencies()
            .contains(&crate::operations::OperationContingency::AbortOnPoliceArrivalBeforeEntry)
    {
        return false;
    }
    let Some(response) = state.legal.get_police_response(response) else {
        return false;
    };
    let effective_arrival = match response.status() {
        crate::legal::PoliceResponseStatus::Dispatched
            if operation.status() == OperationStatus::InProgress
                && response.arrival_due_at() <= state.now() =>
        {
            state.now()
        }
        crate::legal::PoliceResponseStatus::Arrived => match response.arrived_at() {
            Some(arrived_at) => arrived_at,
            None => return false,
        },
        crate::legal::PoliceResponseStatus::Dispatched => return false,
    };
    let entry_at = match operation.status() {
        OperationStatus::InProgress => operation.entry_at(),
        OperationStatus::AwaitingDecision => projected_resume_entry_at(operation, state.now()),
        OperationStatus::Authorized | OperationStatus::Completed | OperationStatus::Aborted => None,
    };
    entry_at.is_some_and(|entry_at| effective_arrival < entry_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::time::SimTime;
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
        Specificity,
    };
    use crate::operations::{
        OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
    };
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
        validate_reassign_character,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn make_test_operation_state() -> (Registry, AppState, OrganizationId, CharacterId, EntityRef) {
        let registry = build_registry();
        let mut state = AppState::new(19);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let leader = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Leader".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("leader fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Test Ward".to_owned(),
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
        .expect("neighborhood fixture should validate");
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Test Business".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("business fixture should validate");
        (
            registry,
            state,
            organization,
            leader,
            EntityRef::Business(business),
        )
    }

    fn make_test_draft(
        organization: OrganizationId,
        leader: CharacterId,
        target: EntityRef,
    ) -> OperationDraft {
        OperationDraft {
            title: "Test intimidation".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::Frighten { target },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::ZERO,
        }
    }

    #[test]
    fn invalid_terminal_transition_leaves_operation_unchanged() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let operation = validate_authorize_operation(
            &registry,
            &state,
            make_test_draft(organization, leader, target),
        )
        .expect("operation fixture should validate")
        .commit(&mut state)
        .expect("validated operation should remain current");

        apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
            .expect("authorized operation should begin");
        apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
            .expect("in-progress operation should abort");
        let before = state
            .operations
            .get_operation(operation)
            .expect("operation should exist")
            .version();

        let error = apply_transition(&registry, &mut state, operation, OperationTransition::Abort)
            .expect_err("terminal operation must reject further transitions");
        assert_eq!(
            error,
            OperationError::InvalidTransition {
                status: OperationStatus::Aborted,
                transition: OperationTransition::Abort,
            }
        );
        let record = state
            .operations
            .get_operation(operation)
            .expect("operation should still exist");
        assert_eq!(record.status(), OperationStatus::Aborted);
        assert_eq!(record.version(), before);
        validate_invariants(&state);
    }

    #[test]
    fn operation_rejects_foreign_crew_members_before_authorization() {
        let (registry, state, organization, leader, target) = make_test_operation_state();
        let mut state = state;
        let foreign_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Foreign Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("foreign organization fixture should validate");
        let foreign_member = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Foreign Member".to_owned(),
                organization: Some(foreign_organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("foreign member fixture should validate");
        let mut draft = make_test_draft(organization, leader, target);
        draft.roles.insert(RoleKind::Coordinator, foreign_member);

        let error = validate_authorize_operation(&registry, &state, draft)
            .expect_err("foreign crew members must not be authorized");
        assert_eq!(
            error,
            OperationError::ForeignParticipant {
                character: foreign_member,
                expected: organization,
                actual: Some(foreign_organization),
            }
        );
        assert_eq!(state.operations().operations().count(), 0);
        validate_invariants(&state);
    }

    #[test]
    fn operation_rejects_one_character_filling_multiple_roles() {
        let (registry, state, organization, leader, target) = make_test_operation_state();
        let draft = OperationDraft {
            title: "Impossible double assignment".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::Frighten { target },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader), (RoleKind::Lookout, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::ZERO,
        };

        let error = validate_authorize_operation(&registry, &state, draft)
            .expect_err("one character must not fill multiple simultaneous roles");
        assert_eq!(
            error,
            OperationError::DuplicateRoleParticipant {
                character: leader,
                first_role: RoleKind::Lookout,
                second_role: RoleKind::Coordinator,
            }
        );
        assert_eq!(state.operations().operations().count(), 0);
        validate_invariants(&state);
    }

    #[test]
    fn property_acquisition_requires_a_business_target() {
        let (registry, state, organization, leader, _) = make_test_operation_state();
        let draft = OperationDraft {
            title: "Invalid property seizure".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: organization,
            leader,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Character(leader),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::EntrySpecialist, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: SimTime::ZERO,
        };

        let error = validate_authorize_operation(&registry, &state, draft)
            .expect_err("property acquisition must identify a business target");
        assert_eq!(
            error,
            OperationError::InvalidPropertyTarget(EntityRef::Character(leader))
        );
    }

    #[test]
    fn expired_planning_information_is_not_reported_as_covered() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: target,
                observed_at: SimTime::ZERO,
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Expired personnel observation".to_owned(),
            },
        )
        .expect("information fixture should validate")
        .commit(&mut state)
        .expect("information fixture should commit");
        let mut draft = make_test_draft(organization, leader, target);
        draft.scheduled_for = SimTime::from_minutes(10_081);
        draft.intelligence.insert(information);
        let operation = validate_authorize_operation(&registry, &state, draft)
            .expect("operation with stale but structurally valid information should authorize")
            .commit(&mut state)
            .expect("operation should commit");

        let (quality, adjustment, covered, relevant) =
            crate::operations::operation_execution::calculate_intelligence_factors(
                &registry, &state, operation,
            );
        assert_eq!(quality.value(), 0);
        assert_eq!(adjustment, 0);
        assert_eq!(covered, 0);
        assert_eq!(relevant, 3);
    }

    #[test]
    fn pre_start_cancellation_records_cause_without_fabricating_execution_artifacts() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let mut draft = make_test_draft(organization, leader, target);
        draft.scheduled_for = SimTime::from_minutes(30);
        let operation = validate_authorize_operation(&registry, &state, draft)
            .expect("future operation should validate")
            .commit(&mut state)
            .expect("future operation should commit");

        validate_authority_abort_operation(&state, operation)
            .expect("authorized operation should accept a leadership cancellation")
            .commit(&mut state)
            .expect("validated pre-start cancellation should commit");

        let record = state
            .operations()
            .get_operation(operation)
            .expect("cancelled operation should persist");
        let abort = record
            .abort_record()
            .expect("cancelled operation should persist its abort record");
        assert_eq!(record.status(), OperationStatus::Aborted);
        assert_eq!(abort.aborted_at(), SimTime::ZERO);
        assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
        assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
        assert!(abort.artifacts().is_none());
        assert!(record.started_at().is_none());
        assert!(record.resolution_due_at().is_none());
        assert_eq!(state.reports().reports_for(organization).count(), 0);
        validate_state(&state).expect("pre-start cancellation should be structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn in_progress_authority_abort_records_causal_artifacts_and_survives_save_round_trip() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let operation = validate_authorize_operation(
            &registry,
            &state,
            make_test_draft(organization, leader, target),
        )
        .expect("operation fixture should validate")
        .commit(&mut state)
        .expect("operation fixture should commit");
        apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
            .expect("operation should begin");
        state.advance_clock(crate::core::time::SimDuration::from_minutes(5));

        validate_authority_abort_operation(&state, operation)
            .expect("in-progress operation should accept a leadership abort")
            .commit(&mut state)
            .expect("validated in-progress abort should commit");

        let record = state
            .operations()
            .get_operation(operation)
            .expect("aborted operation should persist");
        let abort = record
            .abort_record()
            .expect("started abort should persist its provenance");
        assert_eq!(abort.aborted_at(), SimTime::from_minutes(5));
        assert_eq!(abort.phase(), OperationAbortPhase::InProgress);
        assert_eq!(abort.cause(), OperationAbortCause::AuthorityOrder);
        assert!(record.resolution().is_none());
        let artifacts = abort
            .artifacts()
            .expect("started abort should produce after-action artifacts");
        let information = state
            .intelligence()
            .get_information(artifacts.information())
            .expect("abort information should persist");
        assert_eq!(
            information.holder(),
            KnowledgeHolder::Organization(organization)
        );
        assert_eq!(information.topic(), InformationTopic::OperationalOutcome);
        assert_eq!(information.subject(), EntityRef::Operation(operation));
        assert!(information.summary().contains("aborted by leadership"));
        let report = state
            .reports()
            .get_report(artifacts.report())
            .expect("abort report should persist");
        assert_eq!(report.kind(), ReportKind::AfterAction);
        assert_eq!(report.recipient(), organization);
        assert_eq!(report.entries().len(), 1);
        assert_eq!(report.entries()[0].summary, information.summary());
        let history = state
            .history()
            .get_event(artifacts.history_event())
            .expect("abort history should persist");
        assert_eq!(history.summary(), information.summary());

        let envelope = build_save(&registry, &state).expect("aborted operation should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&registry, decoded).expect("aborted operation should restore");
        let restored_abort = restored
            .operations()
            .get_operation(operation)
            .and_then(|record| record.abort_record())
            .expect("restored abort provenance should persist");
        assert_eq!(restored_abort, abort);
        validate_state(&restored).expect("restored abort state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn abort_token_rejects_time_staleness_without_partial_mutation() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let operation = validate_authorize_operation(
            &registry,
            &state,
            make_test_draft(organization, leader, target),
        )
        .expect("operation fixture should validate")
        .commit(&mut state)
        .expect("operation fixture should commit");
        apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
            .expect("operation should begin");
        let abort = validate_authority_abort_operation(&state, operation)
            .expect("fresh abort should validate");
        state.advance_clock(crate::core::time::SimDuration::ONE_MINUTE);

        let error = abort
            .commit(&mut state)
            .expect_err("abort token must expire when simulation time advances");
        assert_eq!(
            error,
            OperationError::StaleAbortTime {
                operation,
                expected: SimTime::ZERO,
                found: SimTime::from_minutes(1),
            }
        );
        let record = state
            .operations()
            .get_operation(operation)
            .expect("stale abort must leave operation present");
        assert_eq!(record.status(), OperationStatus::InProgress);
        assert!(record.abort_record().is_none());
        assert_eq!(state.reports().reports_for(organization).count(), 0);
        assert_eq!(
            state
                .history()
                .events_for(EntityRef::Operation(operation))
                .count(),
            0
        );
        validate_state(&state).expect("stale abort rejection must leave valid state");
        validate_invariants(&state);
    }

    #[test]
    fn missing_required_role_is_rejected_before_id_allocation() {
        let (registry, state, organization, leader, target) = make_test_operation_state();
        let mut draft = make_test_draft(organization, leader, target);
        draft.roles.clear();

        let error = validate_authorize_operation(&registry, &state, draft)
            .expect_err("missing required role must fail validation");
        assert_eq!(
            error,
            OperationError::MissingRequiredRole(RoleKind::Coordinator)
        );
        assert_eq!(
            state
                .operations
                .operations_for_organization(organization)
                .count(),
            0
        );
        validate_invariants(&state);
    }

    #[test]
    fn operation_cannot_begin_before_scheduled_time() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let mut draft = make_test_draft(organization, leader, target);
        draft.scheduled_for = SimTime::from_minutes(30);
        let operation = validate_authorize_operation(&registry, &state, draft)
            .expect("future operation should validate")
            .commit(&mut state)
            .expect("validated operation should remain current");
        let version = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .version();

        let error = apply_transition(&registry, &mut state, operation, OperationTransition::Begin)
            .expect_err("operation must not begin early");
        assert_eq!(error, OperationError::StartBeforeScheduled(operation));
        let record = state
            .operations()
            .get_operation(operation)
            .expect("operation should still exist");
        assert_eq!(record.status(), OperationStatus::Authorized);
        assert_eq!(record.version(), version);
        validate_invariants(&state);
    }

    #[test]
    fn missed_completion_deadline_aborts_before_start_with_visible_provenance() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let mut draft = make_test_draft(organization, leader, target);
        draft.scheduled_for = SimTime::from_minutes(30);
        draft
            .constraints
            .push(crate::operations::OperationConstraint::CompleteBefore(
                SimTime::from_minutes(40),
            ));
        let operation = validate_authorize_operation(&registry, &state, draft)
            .expect("deadline-constrained operation should validate")
            .commit(&mut state)
            .expect("deadline-constrained operation should commit");

        state.advance_clock(crate::core::time::SimDuration::from_minutes(41));
        let outcome = crate::core::simulation::run_tick(&registry, &mut state);

        assert!(outcome.started_operations.is_empty());
        let record = state
            .operations()
            .get_operation(operation)
            .expect("deadline-missed operation should persist");
        assert_eq!(record.status(), OperationStatus::Aborted);
        let abort = record
            .abort_record()
            .expect("deadline miss should persist an abort record");
        assert_eq!(abort.phase(), OperationAbortPhase::BeforeStart);
        assert_eq!(abort.cause(), OperationAbortCause::DeadlineMissed);
        let artifacts = abort
            .artifacts()
            .expect("deadline miss should produce visible provenance");
        let information = state
            .intelligence()
            .get_information(artifacts.information())
            .expect("deadline information should persist");
        assert!(information
            .summary()
            .contains("missed its completion deadline"));
        assert_eq!(state.reports().reports_for(organization).count(), 1);
        validate_state(&state).expect("deadline-missed operation should remain valid");
        validate_invariants(&state);

        let envelope = build_save(&registry, &state).expect("deadline miss should be saveable");
        let bytes = bincode::serialize(&envelope).expect("deadline save should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("deadline save should deserialize");
        let restored = restore_save(&registry, decoded).expect("deadline save should restore");
        assert_eq!(
            restored
                .operations()
                .get_operation(operation)
                .and_then(|record| record.abort_record())
                .map(|abort| abort.cause()),
            Some(OperationAbortCause::DeadlineMissed)
        );
    }

    #[test]
    fn stale_authorization_cannot_commit_after_leader_reassignment() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let validated = validate_authorize_operation(
            &registry,
            &state,
            make_test_draft(organization, leader, target),
        )
        .expect("operation should validate against the original hierarchy");

        validate_reassign_character(&state, leader, None, None)
            .expect("leader should be reassignable before the operation is committed")
            .commit(&mut state)
            .expect("reassignment should commit");

        let error = validated
            .commit(&mut state)
            .expect_err("stale authorization must not create an invalid operation");
        assert_eq!(
            error,
            OperationError::StaleParticipant {
                character: leader,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .operations()
                .operations_for_organization(organization)
                .count(),
            0
        );
        validate_invariants(&state);
    }

    #[test]
    fn authorization_expires_if_scheduled_time_passes_before_commit() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let mut draft = make_test_draft(organization, leader, target);
        draft.scheduled_for = SimTime::from_minutes(2);
        let validated = validate_authorize_operation(&registry, &state, draft)
            .expect("future operation should validate");

        for _ in 0..3 {
            crate::core::simulation::run_tick(&registry, &mut state);
        }

        let error = validated
            .commit(&mut state)
            .expect_err("authorization must expire once its scheduled time is in the past");
        assert_eq!(
            error,
            OperationError::AuthorizationExpired {
                scheduled_for: 2,
                now: 3,
            }
        );
        assert_eq!(
            state
                .operations()
                .operations_for_organization(organization)
                .count(),
            0
        );
        validate_invariants(&state);
    }

    #[test]
    fn operation_intelligence_must_be_owned_and_relevant() {
        let (registry, mut state, organization, leader, target) = make_test_operation_state();
        let irrelevant = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::TargetSecurity,
                source_entity: None,
                subject: target,
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "Detailed security information that does not answer an intimidation planning need."
                    .to_owned(),
            },
        )
        .expect("irrelevant information fixture should still be valid information")
        .commit(&mut state)
        .expect("irrelevant information fixture should commit");
        let unavailable = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Character(leader),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: target,
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "The leader knows the target's personnel pattern personally.".to_owned(),
            },
        )
        .expect("character information fixture should validate")
        .commit(&mut state)
        .expect("character information fixture should commit");

        let mut draft = make_test_draft(organization, leader, target);
        draft.intelligence = BTreeSet::from([irrelevant]);
        let error = validate_authorize_operation(&registry, &state, draft)
            .expect_err("authored operation should reject an irrelevant intelligence topic");
        assert_eq!(error, OperationError::IrrelevantInformation(irrelevant));

        let mut draft = make_test_draft(organization, leader, target);
        draft.intelligence = BTreeSet::from([unavailable]);
        let error = validate_authorize_operation(&registry, &state, draft).expect_err(
            "operation plan should reject intelligence not yet reported to the organization",
        );
        assert_eq!(
            error,
            OperationError::InformationUnavailable {
                information: unavailable,
                organization,
            }
        );
        assert_eq!(
            state
                .operations()
                .operations_for_organization(organization)
                .count(),
            0
        );
        validate_invariants(&state);
    }
}
