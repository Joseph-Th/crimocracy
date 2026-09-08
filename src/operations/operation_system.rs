//! Operation validation and lifecycle execution; sibling records contain no resolution logic.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    ArrestId, CharacterId, IdExhaustionError, InformationId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::enterprises::EnterpriseStatus;
use crate::history::history_system::HistoryError;
use crate::intelligence::KnowledgeHolder;
use crate::intelligence::intelligence_system::IntelligenceError;
use crate::operations::operation_abort::validate_authority_abort_operation;
use crate::operations::operation_state::{pause_duration_minutes, shift_past_pause};
use crate::operations::police_response_integration::{
    OperationPoliceResponseStartPlan, PoliceResponseIntegrationError,
    decide_operation_police_response_start,
};
use crate::operations::surveillance_integration::validate_surveillance_request;
use crate::operations::{
    ACTIVE_ASSIGNMENT_STATUSES, OperationAbortCause, OperationCommand, OperationDraft,
    OperationIdentity, OperationKind, OperationObjective, OperationObjectiveKind, OperationRecord,
    OperationRuntime, OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::reports::report_system::ReportError;
use crate::world::BusinessOwner;
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
    #[error("character {character} is already committed to overlapping operation {operation}")]
    ParticipantBusy {
        character: CharacterId,
        operation: OperationId,
    },
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
    #[error("extraction target character {0} is not currently detained")]
    TargetNotDetained(crate::core::id::CharacterId),
    #[error(
        "detainee {character} is already the extraction target of non-terminal operation {operation}"
    )]
    DetaineeAlreadyTargeted {
        character: crate::core::id::CharacterId,
        operation: OperationId,
    },
    #[error("character {0} is not a named witness on any active case")]
    TargetNotCaseWitness(crate::core::id::CharacterId),
    #[error("objective {objective:?} cannot target administrative entity {target:?}")]
    InvalidObjectiveTarget {
        objective: OperationObjectiveKind,
        target: EntityRef,
    },
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
    #[error("plan lacks required {0:?} intelligence")]
    MissingRequiredIntelligenceTopic(crate::intelligence::InformationTopic),
    #[error("business {0} has no active operating economy to disrupt")]
    TargetWithoutOperatingEconomy(crate::core::id::BusinessId),
    #[error("business {business} is owned by the sponsoring organization")]
    SelfTargetedBusiness {
        business: crate::core::id::BusinessId,
    },
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} cannot begin before its scheduled time")]
    StartBeforeScheduled(OperationId),
    #[error(
        "operation {operation} changed after begin validation; expected version {expected}, found {found}"
    )]
    StaleBeginOperation {
        operation: OperationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "operation begin plan was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleBeginTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    PoliceResponseDispatch(#[from] crate::legal::police_response_system::PoliceResponseError),
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
    #[error(
        "operation {operation} missed its completion deadline at {deadline:?} before it could begin at {now:?}"
    )]
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
pub struct ValidatedOperation<'registry> {
    draft: OperationDraft,
    expected_participant_versions: BTreeMap<CharacterId, u32>,
    registry: &'registry Registry,
}

impl<'registry> ValidatedOperation<'registry> {
    pub fn commit(self, state: &mut AppState) -> Result<OperationId, OperationError> {
        if state.now() > self.draft.scheduled_for {
            return Err(OperationError::AuthorizationExpired {
                scheduled_for: self.draft.scheduled_for.as_minutes(),
                now: state.now().as_minutes(),
            });
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
        let mut participants = BTreeSet::from([self.draft.leader]);
        participants.extend(self.draft.roles.values().copied());
        if let Some((character, operation)) = find_busy_participant(
            self.registry,
            state,
            &participants,
            self.draft.kind,
            self.draft.scheduled_for,
            &self.draft.constraints,
        ) {
            return Err(OperationError::ParticipantBusy {
                character,
                operation,
            });
        }

        // Revalidate plan dependencies that can change independently of participant versions: a
        // target business or neighborhood lifecycle change, or intelligence transferred out of the
        // organization between validation and commit, must stale the authorization. Topic and
        // objective-shape relevance are invariant here because intelligence topics are immutable
        // and the authored operation definition is static. Entity existence needs no separate
        // re-check: entity records are append-only, so anything validated at authorization still
        // exists at commit.
        validate_operation_objective(
            state,
            self.draft.kind,
            self.draft.responsible_organization,
            &self.draft.objective,
        )?;
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
                cash_disposition: None,
                abort: None,
                version: 1,
            },
        });
        Ok(id)
    }
}

pub fn validate_authorize_operation<'registry>(
    registry: &'registry Registry,
    state: &AppState,
    draft: OperationDraft,
) -> Result<ValidatedOperation<'registry>, OperationError> {
    if draft.title.trim().is_empty() {
        return Err(OperationError::EmptyTitle);
    }
    let _ = state
        .world
        .get_organization(draft.responsible_organization)
        .ok_or(OperationError::MissingOrganization(
            draft.responsible_organization,
        ))?;
    let leader = state
        .world
        .get_character(draft.leader)
        .ok_or(OperationError::MissingCharacter(draft.leader))?;
    if leader.organization() != Some(draft.responsible_organization) {
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
    validate_operation_objective(
        state,
        draft.kind,
        draft.responsible_organization,
        &draft.objective,
    )?;
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
    let mut participants = BTreeSet::from([draft.leader]);
    participants.extend(role_participants.keys().copied());
    if let Some((character, operation)) = find_busy_participant(
        registry,
        state,
        &participants,
        draft.kind,
        draft.scheduled_for,
        &draft.constraints,
    ) {
        return Err(OperationError::ParticipantBusy {
            character,
            operation,
        });
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
        match constraint {
            crate::operations::OperationConstraint::CompleteBefore(deadline) => {
                // The deadline must leave room for the crew to reach the entry milestone from
                // the earliest minute the operation can actually begin. A plan scheduled for a
                // future minute begins exactly on its schedule; a plan scheduled for the current
                // minute can only begin on the next canonical tick, so anchoring on
                // `scheduled_for` alone would admit deadlines that make every begin transition
                // reject as missed.
                let begin_at = if draft.scheduled_for > state.now() {
                    draft.scheduled_for
                } else {
                    state.now() + SimDuration::ONE_MINUTE
                };
                let entry_offset = definition
                    .execution()
                    .operation_entry_offset()
                    .unwrap_or(SimDuration::from_minutes(0));
                if *deadline <= begin_at + entry_offset {
                    return Err(OperationError::DeadlineBeforeStart);
                }
            }
            crate::operations::OperationConstraint::RequireIntelligenceTopic(topic) => {
                // Reconnaissance prerequisite: organization-held intelligence of exactly this
                // topic, already validated for objective relevance below, must back the plan.
                let covered = draft.intelligence.iter().any(|information| {
                    state
                        .intelligence
                        .get_information(*information)
                        .is_some_and(|record| record.topic() == *topic)
                });
                if !covered {
                    return Err(OperationError::MissingRequiredIntelligenceTopic(*topic));
                }
            }
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
            | crate::operations::OperationContingency::RequestDecisionOnPoliceArrival => {}
        }
    }

    Ok(ValidatedOperation {
        draft,
        expected_participant_versions,
        registry,
    })
}

fn find_busy_participant(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    requested_kind: OperationKind,
    requested_start: SimTime,
    constraints: &[crate::operations::OperationConstraint],
) -> Option<(CharacterId, OperationId)> {
    // Mirror the begin-time window clamp: a binding deadline compresses the modeled occupancy
    // exactly as it will compress the running operation, so a participant is not falsely
    // reported busy over minutes the clamped operation would never hold.
    let mut requested_end = requested_start
        + registry
            .get_operation(requested_kind)
            .execution()
            .duration();
    for constraint in constraints {
        let crate::operations::OperationConstraint::CompleteBefore(deadline) = constraint else {
            continue;
        };
        if *deadline < requested_end {
            requested_end = *deadline;
        }
    }
    participants.iter().find_map(|participant| {
        state
            .operations
            .active_operations_for_participant(*participant)
            .find(|operation| {
                has_overlapping_operation_window(
                    registry,
                    operation,
                    state.now(),
                    requested_start,
                    requested_end,
                )
            })
            .map(|operation| (*participant, operation.id()))
    })
}

/// Validates that resuming a decision-blocked operation at `resumed_at` does not double-book any
/// of its participants. Resuming shifts the resolution deadline forward by the pause duration, so
/// the post-resume window can collide with operations authorized while this one was paused.
///
/// Operations that have not yet begun keep no persisted end time, so an authorized operation whose
/// start falls inside the resumed window is treated as a conflict; its duration cannot shorten the
/// overlap because a start inside the window always overlaps it.
pub(crate) fn validate_operation_resume_participants(
    state: &AppState,
    operation_id: OperationId,
    resumed_at: SimTime,
) -> Result<(), OperationError> {
    let record = state
        .operations
        .get_operation(operation_id)
        .ok_or(OperationError::MissingOperation(operation_id))?;
    let paused_at = record
        .awaiting_decision_since()
        .expect("resume validation requires a decision-blocked operation");
    let due_at = record
        .resolution_due_at()
        .expect("decision-blocked operation must retain its resolution due time");
    let paused_minutes = pause_duration_minutes(paused_at, resumed_at);
    let shifted_due_at = shift_past_pause(due_at, paused_minutes, "resolution time");
    let window_start = record.started_at().unwrap_or(record.scheduled_for());
    for participant in record.participants() {
        let conflict = state
            .operations
            .active_operations_for_participant(participant)
            .find(|other| {
                other.id() != operation_id
                    && match projected_operation_window(other, resumed_at) {
                        Some((start, end)) => window_start < end && start < shifted_due_at,
                        // Authorized and not yet begun: any start inside the resumed window
                        // conflicts because the operation's duration keeps it running past it.
                        None => other.scheduled_for() < shifted_due_at,
                    }
            })
            .map(|other| other.id());
        if let Some(conflicting_operation) = conflict {
            return Err(OperationError::ParticipantBusy {
                character: participant,
                operation: conflicting_operation,
            });
        }
    }
    Ok(())
}

/// Effective occupancy window of a non-terminal operation, projecting the deadline shift a
/// decision-blocked operation will experience if it resumes at `now`. Returns `None` for terminal
/// operations (never occupy) and for authorized operations that have not begun (no persisted end).
fn projected_operation_window(
    existing: &OperationRecord,
    now: SimTime,
) -> Option<(SimTime, SimTime)> {
    if matches!(
        existing.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return None;
    }
    let start = existing.started_at().unwrap_or(existing.scheduled_for());
    let mut end = existing.resolution_due_at()?;
    if existing.status() == OperationStatus::AwaitingDecision
        && let Some(paused_at) = existing.awaiting_decision_since()
    {
        end = shift_past_pause(
            end,
            pause_duration_minutes(paused_at, now),
            "projected resolution time",
        );
    }
    Some((start, end))
}

fn has_overlapping_operation_window(
    registry: &Registry,
    existing: &OperationRecord,
    now: SimTime,
    requested_start: SimTime,
    requested_end: SimTime,
) -> bool {
    if matches!(
        existing.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return false;
    }
    if let Some((existing_start, existing_end)) = projected_operation_window(existing, now) {
        return requested_start < existing_end && existing_start < requested_end;
    }
    // Authorized and not yet begun: the window runs from the scheduled start for the authored
    // duration until `begin` persists the actual resolution deadline.
    let existing_start = existing.scheduled_for();
    let existing_end = existing_start
        + registry
            .get_operation(existing.kind())
            .execution()
            .duration();
    requested_start < existing_end && existing_start < requested_end
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

/// Whether the objective's target has a shape the objective could ever act on, independent of
/// operation kind. Used only to choose the authorization error variant.
fn is_plausible_objective_target(objective: &OperationObjective) -> bool {
    match objective {
        OperationObjective::AcquireProperty { target }
        | OperationObjective::ObtainCash { target }
        | OperationObjective::DisruptBusiness { target } => {
            matches!(target, EntityRef::Business(_))
        }
        OperationObjective::Frighten { target } => matches!(target, EntityRef::Character(_)),
        // Surveillance targets and extraction detainees are validated by their own gates.
        OperationObjective::GatherInformation { .. } | OperationObjective::FreeDetainee { .. } => {
            true
        }
    }
}

pub(crate) fn is_valid_operation_objective(
    kind: OperationKind,
    objective: &OperationObjective,
) -> bool {
    match objective {
        OperationObjective::AcquireProperty { target } => {
            kind.can_acquire_property() && matches!(target, EntityRef::Business(_))
        }
        OperationObjective::GatherInformation { target } => {
            kind == OperationKind::Surveillance
                && crate::operations::surveillance_integration::is_supported_surveillance_target(
                    *target,
                )
        }
        OperationObjective::ObtainCash { target } => {
            kind.can_take_cash() && matches!(target, EntityRef::Business(_))
        }
        OperationObjective::Frighten { target } => {
            kind == OperationKind::WitnessPressure && matches!(target, EntityRef::Character(_))
        }
        OperationObjective::FreeDetainee { .. } => kind == OperationKind::Extraction,
        OperationObjective::DisruptBusiness { target } => {
            matches!(kind, OperationKind::Sabotage | OperationKind::Arson)
                && matches!(target, EntityRef::Business(_))
        }
    }
}

// Field actions may reference concrete world subjects and locations, never control-plane
// records such as operations, investigations, evidence, accounts, decisions, or mandates.
// Historical intelligence and after-action records may refer to inactive entities, so the
// general `is_entity_present` check remains existence-only. Action objectives have a stricter
// contract: their concrete world subjects must still be actionable when the operation is
// authorized.
fn validate_active_field_objective_targets(
    state: &AppState,
    responsible_organization: OrganizationId,
    objective: &OperationObjective,
) -> Result<(), OperationError> {
    match objective {
        OperationObjective::ObtainCash { target } => {
            validate_active_field_objective_target(state, *target)?;
            reject_self_owned_target(state, responsible_organization, *target)
        }
        // Witness pressure is only meaningful against a character who is actually a named
        // witness on an active case run by another authority; anything else would resolve
        // with nothing to coerce.
        OperationObjective::Frighten { target } => {
            validate_active_field_objective_target(state, *target)?;
            let EntityRef::Character(character) = *target else {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: OperationObjectiveKind::Frighten,
                    target: *target,
                });
            };
            // The by-character witness index scopes this probe to the target's own
            // registrations instead of the full ever-growing witness history.
            let is_case_witness =
                state
                    .legal
                    .case_witnesses_for_character(character)
                    .any(|witness| {
                        witness.witness() == character
                            && state
                                .legal
                                .get_investigation(witness.investigation())
                                .is_some_and(|investigation| {
                                    investigation.status()
                                        == crate::legal::InvestigationStatus::Active
                                        && investigation.owner() != responsible_organization
                                })
                    });
            if !is_case_witness {
                return Err(OperationError::TargetNotCaseWitness(character));
            }
            Ok(())
        }
        // Property and sabotage targets are businesses whose premises the crew acts on.
        // Taking value out of, or disrupting, the organization's own premises would enrich
        // or disrupt itself — victim and sponsor the same ledger entity — so authorization
        // rejects self-targeting up front instead of resolving into an incoherent haul.
        OperationObjective::AcquireProperty { target } => {
            let EntityRef::Business(_) = *target else {
                return Ok(());
            };
            validate_active_field_objective_target(state, *target)?;
            reject_self_owned_target(state, responsible_organization, *target)
        }
        OperationObjective::GatherInformation { .. } => Ok(()),
        // Sabotage targets a business whose premises the crew must physically reach, and
        // one that actually operates: disrupting a shuttered or economy-less storefront
        // would resolve into nothing, so authorization rejects it up front.
        OperationObjective::DisruptBusiness { target } => {
            validate_active_field_objective_target(state, *target)?;
            reject_self_owned_target(state, responsible_organization, *target)?;
            let EntityRef::Business(business) = *target else {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: objective.kind(),
                    target: *target,
                });
            };
            let economy = state
                .economy
                .get_business_economy(business)
                .ok_or(OperationError::TargetWithoutOperatingEconomy(business))?;
            if economy.status() != crate::economy::BusinessOperatingStatus::Active {
                return Err(OperationError::TargetWithoutOperatingEconomy(business));
            }
            Ok(())
        }
        // Extraction targets a person who may legitimately be in custody, so the usual
        // active-field-target check does not apply; custody state is validated separately.
        OperationObjective::FreeDetainee { target } => {
            let _ = state
                .world
                .get_character(*target)
                .ok_or(OperationError::MissingEntity(EntityRef::Character(*target)))?;
            if state.legal.active_arrest_for_character(*target).is_none() {
                return Err(OperationError::TargetNotDetained(*target));
            }
            // One detainee, one live extraction plan: a second non-terminal extraction against
            // the same custody could resolve only after the first freed the target, and would
            // then be unable to commit. The target becomes plannable again once the prior
            // operation reaches a terminal state.
            if let Some(operation) = find_non_terminal_extraction_targeting(state, *target) {
                return Err(OperationError::DetaineeAlreadyTargeted {
                    character: *target,
                    operation,
                });
            }
            Ok(())
        }
    }
}

/// Smallest non-terminal operation whose objective extracts the given detainee, if any.
/// The status buckets are scanned in fixed order and ties break on operation id, so the
/// reported operation is deterministic.
fn find_non_terminal_extraction_targeting(
    state: &AppState,
    target_character: CharacterId,
) -> Option<OperationId> {
    ACTIVE_ASSIGNMENT_STATUSES
        .iter()
        .flat_map(|status| state.operations.operations_with_status(*status))
        .filter(|operation| {
            matches!(
              operation.objective(),
              OperationObjective::FreeDetainee { target } if *target == target_character
            )
        })
        .map(|operation| operation.id())
        .min()
}

/// Rejects take/disrupt objectives aimed at a business the sponsoring organization owns.
fn reject_self_owned_target(
    state: &AppState,
    organization: OrganizationId,
    target: EntityRef,
) -> Result<(), OperationError> {
    let EntityRef::Business(business) = target else {
        return Ok(());
    };
    let owner = state
        .world
        .get_business(business)
        .ok_or(OperationError::MissingEntity(target))?
        .owner();
    if owner == BusinessOwner::Organization(organization) {
        return Err(OperationError::SelfTargetedBusiness { business });
    }
    Ok(())
}

fn validate_active_field_objective_target(
    state: &AppState,
    target: EntityRef,
) -> Result<(), OperationError> {
    match target {
        EntityRef::Organization(id) => {
            state
                .world
                .get_organization(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Character(id) => {
            state
                .world
                .get_character(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Neighborhood(id) => {
            state
                .world
                .get_neighborhood(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Business(id) => {
            state
                .world
                .get_business(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Enterprise(id) => {
            let active = state
                .enterprises
                .get_enterprise(id)
                .ok_or(OperationError::MissingEntity(target))?
                .status()
                == EnterpriseStatus::Active;
            if !active {
                return Err(OperationError::InactiveObjectiveTarget(target));
            }
        }
        // Control-plane records never reach this match: `is_valid_operation_objective` rejects
        // them before activation checks run, so reaching this arm means a caller skipped that gate.
        EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => {
            unreachable!(
                "administrative objective targets are rejected by is_valid_operation_objective"
            )
        }
    }
    Ok(())
}

fn validate_operation_objective(
    state: &AppState,
    kind: OperationKind,
    responsible_organization: OrganizationId,
    objective: &OperationObjective,
) -> Result<(), OperationError> {
    if let OperationObjective::AcquireProperty { target } = objective {
        if !kind.can_acquire_property() {
            return Err(OperationError::InvalidObjectiveForKind {
                kind,
                objective: OperationObjectiveKind::AcquireProperty,
            });
        }
        let EntityRef::Business(business) = target else {
            return Err(OperationError::InvalidPropertyTarget(*target));
        };
        let business_record = state
            .world
            .get_business(*business)
            .ok_or(OperationError::MissingEntity(*target))?;
        state
            .world
            .get_neighborhood(business_record.neighborhood())
            .ok_or(OperationError::MissingEntity(EntityRef::Neighborhood(
                business_record.neighborhood(),
            )))?;
    }
    if !is_valid_operation_objective(kind, objective) {
        // A rejection here is either a kind/objective mismatch or a target shape the
        // objective can never act on. Only an implausible target shape reports the target;
        // a well-formed target against an unsupported kind is a kind mismatch, so a normal
        // business or character is never mislabeled as an administrative entity.
        if !is_plausible_objective_target(objective) {
            let invalid_target = match objective {
                OperationObjective::ObtainCash { target }
                | OperationObjective::Frighten { target } => Some(*target),
                OperationObjective::DisruptBusiness { target } => Some(*target),
                OperationObjective::AcquireProperty { .. }
                | OperationObjective::GatherInformation { .. }
                | OperationObjective::FreeDetainee { .. } => None,
            };
            if let Some(target) = invalid_target {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: objective.kind(),
                    target,
                });
            }
        }
        return Err(OperationError::InvalidObjectiveForKind {
            kind,
            objective: objective.kind(),
        });
    }
    validate_active_field_objective_targets(state, responsible_organization, objective)?;
    Ok(())
}

pub(crate) fn find_due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state.operations.find_due_authorized(state.now())
}

/// True once the operation's earliest completion deadline minute has arrived or passed. Begin
/// gating and deadline-abort validation use this: an operation cannot start at its deadline,
/// and a deadline reached without resolution justifies the `DeadlineMissed` abort artifacts.
pub(crate) fn has_missed_operation_deadline(state: &AppState, operation: OperationId) -> bool {
    state
        .operations
        .get_operation(operation)
        .and_then(resolve_earliest_operation_deadline)
        .is_some_and(|deadline| state.now() >= deadline)
}

/// True once the operation's completion deadline has fully passed, using the same strict
/// comparison as the overdue-abort scan. An explicit leadership abort resolved on the deadline
/// minute itself is an active choice rather than a missed deadline, so the decision path uses
/// this form and records its `Decision` cause; the automatic scan still produces
/// `DeadlineMissed` artifacts because it only fires strictly after the deadline.
pub(crate) fn has_operation_deadline_fully_passed(
    state: &AppState,
    operation: OperationId,
) -> bool {
    state
        .operations
        .get_operation(operation)
        .and_then(resolve_earliest_operation_deadline)
        .is_some_and(|deadline| state.now() > deadline)
}

/// Operations whose completion deadline has fully passed without resolution. Begin clamps
/// `resolution_due_at` to a binding deadline, so an in-progress operation is due to resolve on
/// the deadline minute itself; the strict comparison lets that same-minute resolution win over
/// the abort scan. Only work that failed to resolve by its deadline — reachable when a decision
/// request pauses the operation across the deadline — is reported here for a hard abort.
pub(crate) fn find_due_operations_with_missed_deadlines(state: &AppState) -> Vec<OperationId> {
    let mut due = state
        .operations
        .operations_with_status(OperationStatus::InProgress)
        .chain(
            state
                .operations
                .operations_with_status(OperationStatus::AwaitingDecision),
        )
        .filter(|operation| {
            resolve_earliest_operation_deadline(operation)
                .is_some_and(|deadline| state.now() > deadline)
        })
        .map(|operation| operation.id())
        .collect::<Vec<_>>();
    // The status indexes are separate, so restore one global stable order before any
    // deadline abort creates IDs or reports that later work can observe.
    due.sort_unstable();
    due
}

pub(crate) fn resolve_earliest_operation_deadline(record: &OperationRecord) -> Option<SimTime> {
    record
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            crate::operations::OperationConstraint::CompleteBefore(deadline) => Some(*deadline),
            crate::operations::OperationConstraint::RequireIntelligenceTopic(_) => None,
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
        if record.version() != self.expected_version {
            return Err(OperationError::StaleBeginOperation {
                operation: self.operation,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != OperationStatus::Authorized {
            return Err(OperationError::InvalidTransition {
                status: record.status(),
                transition: OperationTransition::Begin,
            });
        }
        if state.now() != self.started_at {
            return Err(OperationError::StaleBeginTime {
                expected: self.started_at,
                found: state.now(),
            });
        }
        let entry_at = self.police_response.entry_at();
        let response = self.police_response.commit_dispatch(state)?;
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

/// Start planning resolves only the operation record and dispatch validation; its decision
/// and intelligence failure modes belong exclusively to the response-arrival pass.
fn map_police_start_planning_error(error: PoliceResponseIntegrationError) -> OperationError {
    match error {
        PoliceResponseIntegrationError::MissingOperation(operation) => {
            OperationError::MissingOperation(operation)
        }
        PoliceResponseIntegrationError::PoliceResponse(dispatch) => dispatch.into(),
        PoliceResponseIntegrationError::Decision(_)
        | PoliceResponseIntegrationError::Intelligence(_)
        | PoliceResponseIntegrationError::IdExhaustion(_) => {
            unreachable!(
                "police response start planning does not allocate IDs or validate arrival-only decisions or intelligence"
            )
        }
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
    if let Some(deadline) = resolve_earliest_operation_deadline(record)
        && state.now() >= deadline
    {
        return Err(OperationError::DeadlineMissed {
            operation,
            deadline,
            now: state.now(),
        });
    }
    let duration = registry.get_operation(record.kind()).execution().duration();
    // A binding deadline compresses the modeled window: the operation resolves on the deadline
    // minute under time pressure (`resolve_time_pressure`), and only a deadline that passes
    // without resolution — a decision-paused operation — is hard-aborted afterwards.
    let mut resolution_due_at = state.now() + duration;
    for constraint in record.constraints() {
        let crate::operations::OperationConstraint::CompleteBefore(deadline) = constraint else {
            continue;
        };
        if *deadline < resolution_due_at {
            resolution_due_at = *deadline;
        }
    }
    // Begin-time re-check of the authorization rule that a deadline must leave room for the
    // crew to reach the entry milestone: a begin issued later than `scheduled_for` shrinks
    // the approach window, and an entry milestone at or after resolution would resolve the
    // operation before its modeled approach begins.
    let earliest_resolution = state.now()
        + registry
            .get_operation(record.kind())
            .execution()
            .operation_entry_offset()
            .unwrap_or(SimDuration::from_minutes(0));
    if resolution_due_at <= earliest_resolution {
        return Err(OperationError::DeadlineMissed {
            operation,
            deadline: resolution_due_at,
            now: state.now(),
        });
    }
    let police_response = decide_operation_police_response_start(registry, state, operation)
        .map_err(map_police_start_planning_error)?;
    Ok(ValidatedOperationStart {
        operation,
        expected_version: record.version(),
        started_at: state.now(),
        resolution_due_at,
        police_response,
    })
}

#[cfg(test)]
mod tests;
