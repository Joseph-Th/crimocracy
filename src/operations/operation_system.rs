//! Operation authorization, scheduling, and lifecycle; objective actionability lives in the child validator.

mod objective_validation;

use objective_validation::validate_operation_objective;
pub(crate) use objective_validation::{
    is_actionable_opportunity_target, is_information_subject_relevant, is_valid_operation_objective,
};

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    ArrestId, CharacterId, IdExhaustionError, InformationId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::history::history_system::HistoryError;
use crate::intelligence::KnowledgeHolder;
use crate::intelligence::intelligence_system::IntelligenceError;
use crate::operations::operation_abort::validate_authority_abort_operation;
use crate::operations::operation_intelligence::resolve_information_score;
use crate::operations::operation_state::{checked_shift_past_pause, pause_duration_minutes};
use crate::operations::police_response_integration::{
    OperationPoliceResponseStartPlan, PoliceResponseIntegrationError,
    decide_operation_police_response_start,
};
use crate::operations::surveillance_integration::validate_surveillance_request;
use crate::operations::{
    OperationAbortCause, OperationCommand, OperationDraft, OperationIdentity, OperationKind,
    OperationObjective, OperationObjectiveKind, OperationRecord, OperationRuntime, OperationStatus,
    RoleKind,
};
use crate::registry::{OperationDefinition, Registry};
use crate::reports::report_system::ReportError;
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
    #[error(
        "extraction custody for character {character} changed after authorization validation; expected arrest {expected}, found {found:?}"
    )]
    StaleExtractionCustody {
        character: CharacterId,
        expected: ArrestId,
        found: Option<ArrestId>,
    },
    #[error(
        "extraction for detainee {character} would finish at {planned_end:?}, but custody ends at {custody_ends_at:?} and must still be active through completion"
    )]
    ExtractionMissesCustodyWindow {
        character: CharacterId,
        planned_end: SimTime,
        custody_ends_at: SimTime,
    },
    #[error("character {0} is not a named witness on any active case")]
    TargetNotCaseWitness(crate::core::id::CharacterId),
    #[error("character {0} has no active witness cooperation left for intimidation to reduce")]
    TargetNotPressureableWitness(crate::core::id::CharacterId),
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
    #[error("role {0:?} has no execution function for this operation kind")]
    UnsupportedRole(RoleKind),
    #[error("operation is scheduled in the past")]
    ScheduledInPast,
    #[error("operation timing exceeds the representable simulation clock")]
    SimulationTimeOverflow,
    #[error("operation completion deadline leaves no executable window after begin/entry")]
    DeadlineLeavesNoExecutionWindow,
    #[error("plan lacks usable required {0:?} intelligence at its earliest planned start")]
    MissingRequiredIntelligenceTopic(crate::intelligence::InformationTopic),
    #[error("business {0} has no active operating economy to disrupt")]
    TargetWithoutOperatingEconomy(crate::core::id::BusinessId),
    #[error("business {business} is owned by the sponsoring organization")]
    SelfTargetedBusiness {
        business: crate::core::id::BusinessId,
    },
    #[error("business {business} is not owned by the sponsoring organization")]
    TargetBusinessNotSponsorOwned {
        business: crate::core::id::BusinessId,
    },
    #[error("business {business} lacks required operation function {function:?}")]
    TargetBusinessMissingFunction {
        business: crate::core::id::BusinessId,
        function: crate::world::BusinessFunction,
    },
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {operation} cannot begin before {earliest_start:?}")]
    StartBeforeEarliestStart {
        operation: OperationId,
        earliest_start: SimTime,
    },
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
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub struct ValidatedOperation<'registry> {
    draft: OperationDraft,
    expected_participant_versions: BTreeMap<CharacterId, u32>,
    extraction_arrest: Option<ArrestId>,
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
        validate_deadline_execution_window(
            self.registry.get_operation(self.draft.kind).execution(),
            state.now(),
            self.draft.scheduled_for,
            &self.draft.constraints,
        )?;
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
        validate_extraction_custody_current(state, &self.draft.objective, self.extraction_arrest)?;
        validate_operation_objective(
            self.registry,
            state,
            self.draft.kind,
            self.draft.responsible_organization,
            &self.draft.objective,
        )?;
        validate_extraction_custody_window(
            self.registry,
            state,
            &self.draft,
            self.extraction_arrest,
            state.now(),
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
        validate_authorization_constraints(
            self.registry,
            state,
            self.registry.get_operation(self.draft.kind),
            &self.draft,
        )?;

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
        let authorized_at = state.now();
        let id = state.ids.next_operation()?;
        state.operations.insert(OperationRecord {
            identity: OperationIdentity { id, title, kind },
            command: OperationCommand {
                responsible_organization,
                leader,
                objective,
                extraction_arrest: self.extraction_arrest,
                approach,
                roles,
                intelligence,
                constraints,
                contingencies,
                authorized_at,
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
        registry,
        state,
        draft.kind,
        draft.responsible_organization,
        &draft.objective,
    )?;
    let extraction_arrest = resolve_current_extraction_arrest(state, &draft.objective)?;
    let mut expected_participant_versions = BTreeMap::from([(draft.leader, leader.version())]);

    let definition = registry.get_operation(draft.kind);
    validate_representable_operation_window(
        definition.execution(),
        state.now(),
        draft.scheduled_for,
    )?;
    if !definition.supported_approaches().contains(&draft.approach) {
        return Err(OperationError::UnsupportedApproach);
    }
    let participants = validate_authorization_participants(
        state,
        definition,
        &draft,
        &mut expected_participant_versions,
    )?;
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
    validate_authorization_intelligence(state, definition, &draft)?;
    for entity in draft.objective.referenced_entities() {
        if !is_entity_present(state, entity) {
            return Err(OperationError::MissingEntity(entity));
        }
    }
    validate_deadline_execution_window(
        definition.execution(),
        state.now(),
        draft.scheduled_for,
        &draft.constraints,
    )?;
    validate_extraction_custody_window(registry, state, &draft, extraction_arrest, state.now())?;
    validate_authorization_constraints(registry, state, definition, &draft)?;
    validate_authorization_contingencies(definition, &draft)?;

    Ok(ValidatedOperation {
        draft,
        expected_participant_versions,
        extraction_arrest,
        registry,
    })
}

/// Begin-time availability is stricter than authorization-time interval projection. A future
/// follow-up may be authorized exactly at another operation's projected end because the earlier
/// decision might be resolved before that boundary. Once the follow-up actually tries to begin,
/// however, any still-pending operation decision remains an active personnel commitment even when
/// its unshifted half-open window ends exactly at this minute.
fn find_busy_participant_for_begin(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    operation: OperationId,
    requested_start: SimTime,
    requested_end: SimTime,
) -> Option<(CharacterId, OperationId)> {
    find_busy_participant_in_window(
        registry,
        state,
        participants,
        Some(operation),
        requested_start,
        requested_end,
    )
    .or_else(|| {
        participants.iter().find_map(|participant| {
            state
                .operations
                .active_operations_for_participant(*participant)
                .find(|other| {
                    other.id() != operation && other.status() == OperationStatus::AwaitingDecision
                })
                .map(|other| (*participant, other.id()))
        })
    })
}

/// Validates the operation's required seats and participant availability while collecting the
/// version pins consumed by the authorization token. Keeping this as one concern prevents the
/// public authorization path from interleaving roster validation with plan semantics.
fn validate_authorization_participants(
    state: &AppState,
    definition: &OperationDefinition,
    draft: &OperationDraft,
    expected_participant_versions: &mut BTreeMap<CharacterId, u32>,
) -> Result<BTreeSet<CharacterId>, OperationError> {
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
        if definition.execution().capability_for_role(*role).is_none() {
            return Err(OperationError::UnsupportedRole(*role));
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
    Ok(participants)
}

fn validate_authorization_intelligence(
    state: &AppState,
    definition: &OperationDefinition,
    draft: &OperationDraft,
) -> Result<(), OperationError> {
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
    Ok(())
}

fn validate_authorization_constraints(
    registry: &Registry,
    state: &AppState,
    definition: &OperationDefinition,
    draft: &OperationDraft,
) -> Result<(), OperationError> {
    let planning_at = earliest_operation_start_from_authorization(state.now(), draft.scheduled_for);
    let max_age = u64::from(definition.execution().max_intelligence_age().as_minutes());
    for constraint in &draft.constraints {
        match constraint {
            crate::operations::OperationConstraint::CompleteBy(_) => {}
            crate::operations::OperationConstraint::RequireIntelligenceTopic(topic) => {
                // Reconnaissance prerequisite: organization-held intelligence of exactly this
                // topic, already validated for objective relevance, must still have nonzero
                // canonical planning value when the operation can first begin. A stale record
                // must not satisfy the constraint after the execution model has already reduced
                // that same information to zero usefulness.
                let covered = draft.intelligence.iter().any(|information| {
                    state
                        .intelligence
                        .get_information(*information)
                        .is_some_and(|record| {
                            record.topic() == *topic
                                && resolve_information_score(
                                    registry.information_quality(),
                                    record,
                                    planning_at,
                                    max_age,
                                ) > 0
                        })
                });
                if !covered {
                    return Err(OperationError::MissingRequiredIntelligenceTopic(*topic));
                }
            }
        }
    }
    Ok(())
}

fn validate_authorization_contingencies(
    definition: &OperationDefinition,
    draft: &OperationDraft,
) -> Result<(), OperationError> {
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
    Ok(())
}

/// A completion deadline must leave at least one executable minute after the earliest legal
/// begin/entry boundary. Authorization tokens may be committed later than they were validated,
/// so this rule is deliberately shared by validation and commit using the actual authorization
/// instant each path owns.
fn validate_representable_operation_window(
    execution: &crate::registry::OperationExecutionDefinition,
    authorized_at: SimTime,
    scheduled_for: SimTime,
) -> Result<SimTime, OperationError> {
    let start = checked_earliest_operation_start_from_authorization(authorized_at, scheduled_for)
        .ok_or(OperationError::SimulationTimeOverflow)?;
    start
        .as_minutes()
        .checked_add(u64::from(execution.duration().as_minutes()))
        .ok_or(OperationError::SimulationTimeOverflow)?;
    Ok(start)
}

fn validate_deadline_execution_window(
    execution: &crate::registry::OperationExecutionDefinition,
    authorized_at: SimTime,
    scheduled_for: SimTime,
    constraints: &[crate::operations::OperationConstraint],
) -> Result<(), OperationError> {
    let begin_at =
        validate_representable_operation_window(execution, authorized_at, scheduled_for)?;
    if resolve_deadline_without_execution_window(execution, begin_at, constraints).is_some() {
        return Err(OperationError::DeadlineLeavesNoExecutionWindow);
    }
    Ok(())
}

/// Earliest completion deadline that leaves no executable minute after the operation's authored
/// entry milestone when beginning at `begin_at`. `None` means every deadline still leaves a
/// usable execution window. Checked minute arithmetic makes this safe for restore validation at
/// the edge of the representable simulation clock.
pub(crate) fn resolve_deadline_without_execution_window(
    execution: &crate::registry::OperationExecutionDefinition,
    begin_at: SimTime,
    constraints: &[crate::operations::OperationConstraint],
) -> Option<SimTime> {
    let earliest_deadline = constraints
        .iter()
        .filter_map(|constraint| match constraint {
            crate::operations::OperationConstraint::CompleteBy(deadline) => Some(*deadline),
            crate::operations::OperationConstraint::RequireIntelligenceTopic(_) => None,
        });
    let entry_offset = u64::from(
        execution
            .operation_entry_offset()
            .unwrap_or(SimDuration::from_minutes(0))
            .as_minutes(),
    );
    let first_executable_minute = begin_at.as_minutes().checked_add(entry_offset);
    earliest_deadline
        .filter(|deadline| {
            first_executable_minute
                .is_none_or(|first_executable| deadline.as_minutes() <= first_executable)
        })
        .min()
}

fn find_busy_participant(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    requested_kind: OperationKind,
    requested_start: SimTime,
    constraints: &[crate::operations::OperationConstraint],
) -> Option<(CharacterId, OperationId)> {
    let (requested_start, requested_end) = projected_authorized_operation_window(
        registry,
        state.now(),
        requested_kind,
        requested_start,
        constraints,
    );
    find_busy_participant_in_window(
        registry,
        state,
        participants,
        None,
        requested_start,
        requested_end,
    )
}

fn find_busy_participant_in_window(
    registry: &Registry,
    state: &AppState,
    participants: &BTreeSet<CharacterId>,
    excluded_operation: Option<OperationId>,
    requested_start: SimTime,
    requested_end: SimTime,
) -> Option<(CharacterId, OperationId)> {
    participants.iter().find_map(|participant| {
        state
            .operations
            .active_operations_for_participant(*participant)
            .find(|operation| {
                Some(operation.id()) != excluded_operation
                    && has_overlapping_operation_window(
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

/// Occupancy window for an operation that is still awaiting its begin transition.
///
/// `run_tick` advances the clock before starting due operations. Therefore an operation
/// authorized for the current minute cannot occupy its crew until the next minute, while a
/// future operation begins on its authored schedule. Binding completion deadlines shorten the
/// same window at authorization time and at begin time. Keeping both rules here prevents the
/// booking projection from ending a minute early or reserving time beyond a deadline.
fn projected_authorized_operation_window(
    registry: &Registry,
    authorized_at: SimTime,
    kind: OperationKind,
    scheduled_for: SimTime,
    constraints: &[crate::operations::OperationConstraint],
) -> (SimTime, SimTime) {
    let start = earliest_operation_start_from_authorization(authorized_at, scheduled_for);
    // Authorization rejects an unrepresentable authored window before this projection is used
    // for a new plan. Existing-state overlap checks are read-only, so clamp an impossible
    // historical/malformed end to the finite horizon rather than letting a query panic.
    let mut end = start
        .checked_add(registry.get_operation(kind).execution().duration())
        .unwrap_or(SimTime::from_minutes(u64::MAX));
    for constraint in constraints {
        let crate::operations::OperationConstraint::CompleteBy(deadline) = constraint else {
            continue;
        };
        if *deadline < end {
            end = *deadline;
        }
    }
    (start, end)
}

/// Reject an extraction that cannot finish before the target's currently modeled custody ends.
/// This is a planning gate only: later release, delay, or re-arrest remains mutable simulation
/// state and is handled again by resolution-time custody snapshots.
fn validate_extraction_custody_window(
    registry: &Registry,
    state: &AppState,
    draft: &OperationDraft,
    extraction_arrest: Option<ArrestId>,
    authorized_at: SimTime,
) -> Result<(), OperationError> {
    let OperationObjective::FreeDetainee { target } = draft.objective else {
        debug_assert!(extraction_arrest.is_none());
        return Ok(());
    };
    let arrest_id = extraction_arrest.expect("validated extraction must retain its custody link");
    let arrest = state
        .legal
        .get_arrest(arrest_id)
        .expect("validated extraction custody must remain persisted");
    debug_assert_eq!(arrest.character(), target);
    let (_, planned_end) = projected_authorized_operation_window(
        registry,
        authorized_at,
        draft.kind,
        draft.scheduled_for,
        &draft.constraints,
    );
    let custody_ends_at = crate::legal::arrest_system::custody_release_at(
        arrest.arrested_at(),
        registry.legal().maximum_detention(),
    );
    if planned_end >= custody_ends_at {
        return Err(OperationError::ExtractionMissesCustodyWindow {
            character: target,
            planned_end,
            custody_ends_at,
        });
    }
    Ok(())
}

/// The custody relationship an extraction is being planned against. The ID, not merely the
/// detainee, is part of the plan because release followed by re-arrest creates a different legal
/// situation that an already-authorized operation must not silently adopt.
fn resolve_current_extraction_arrest(
    state: &AppState,
    objective: &OperationObjective,
) -> Result<Option<ArrestId>, OperationError> {
    let OperationObjective::FreeDetainee { target } = objective else {
        return Ok(None);
    };
    state
        .legal
        .active_arrest_for_character(*target)
        .map(|arrest| Some(arrest.id()))
        .ok_or(OperationError::TargetNotDetained(*target))
}

/// Authorization tokens pin an extraction to one custody event. Commit must reject if that
/// detainee was released or re-arrested between validation and commit rather than retargeting the
/// operation to whatever arrest happens to be active later.
fn validate_extraction_custody_current(
    state: &AppState,
    objective: &OperationObjective,
    expected: Option<ArrestId>,
) -> Result<(), OperationError> {
    let OperationObjective::FreeDetainee { target } = objective else {
        debug_assert!(expected.is_none());
        return Ok(());
    };
    let expected = expected.expect("validated extraction must retain its custody link");
    let found = state
        .legal
        .active_arrest_for_character(*target)
        .map(|arrest| arrest.id());
    if found != Some(expected) {
        return Err(OperationError::StaleExtractionCustody {
            character: *target,
            expected,
            found,
        });
    }
    Ok(())
}

fn earliest_operation_start_from_authorization(
    authorized_at: SimTime,
    scheduled_for: SimTime,
) -> SimTime {
    checked_earliest_operation_start_from_authorization(authorized_at, scheduled_for)
        .expect("validated operation must retain a representable earliest start")
}

fn checked_earliest_operation_start_from_authorization(
    authorized_at: SimTime,
    scheduled_for: SimTime,
) -> Option<SimTime> {
    if scheduled_for > authorized_at {
        Some(scheduled_for)
    } else {
        authorized_at
            .as_minutes()
            .checked_add(1)
            .map(SimTime::from_minutes)
    }
}

/// First legal execution minute for a persisted authorized operation. A future plan begins on its
/// authored schedule, while a plan authorized for the current minute cannot retroactively execute
/// inside the minute in which authorization was committed.
pub(crate) fn resolve_operation_earliest_start(record: &OperationRecord) -> SimTime {
    earliest_operation_start_from_authorization(record.authorized_at(), record.scheduled_for())
}

pub(crate) fn try_resolve_operation_earliest_start(record: &OperationRecord) -> Option<SimTime> {
    checked_earliest_operation_start_from_authorization(
        record.authorized_at(),
        record.scheduled_for(),
    )
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
    let shifted_due_at = checked_shift_past_pause(due_at, paused_minutes)
        .ok_or(OperationError::SimulationTimeOverflow)?;
    if let Some(entry_at) = record.entry_at()
        && entry_at > paused_at
        && checked_shift_past_pause(entry_at, paused_minutes).is_none()
    {
        return Err(OperationError::SimulationTimeOverflow);
    }
    let window_start = record.started_at().unwrap_or(record.scheduled_for());
    for participant in record.participants() {
        let conflict = state
            .operations
            .active_operations_for_participant(participant)
            .find(|other| {
                other.id() != operation_id
                    && match projected_operation_window(other, resumed_at) {
                        Some((start, end)) => window_start < end && start < shifted_due_at,
                        // Authorized and not yet begun: compare the first minute it can actually
                        // start. Authorization provenance, not the resume instant, determines
                        // whether its schedule was a same-minute plan or a genuine future start.
                        None => resolve_operation_earliest_start(other) < shifted_due_at,
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

/// Applies the operation-owned half of a validated exception-decision request. The decision
/// system performs the cross-domain freshness checks before inserting its request; keeping the
/// actual lifecycle mutation here prevents peer domains from reaching through to `OperationState`
/// internals directly.
pub(crate) fn apply_decision_pause_preflighted(
    state: &mut AppState,
    operation: OperationId,
    paused_at: SimTime,
) {
    state.operations.set_awaiting_decision(operation, paused_at);
}

/// Applies the operation-owned half of a validated Continue decision. Resume overflow and
/// participant conflicts are preflighted by `validate_operation_resume_participants`; this
/// mutation is deliberately infallible after that validation so the decision record and operation
/// lifecycle can commit as one cross-domain transaction.
pub(crate) fn apply_decision_resume_preflighted(
    state: &mut AppState,
    operation: OperationId,
    resumed_at: SimTime,
) {
    state.operations.resume(operation, resumed_at);
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
        // If this already-paused operation would extend past the representable clock horizon,
        // it cannot safely resume. For booking purposes it occupies the crew through the end
        // of representable time; the actual resume path rejects with SimulationTimeOverflow.
        end = checked_shift_past_pause(end, pause_duration_minutes(paused_at, now))
            .unwrap_or(SimTime::from_minutes(u64::MAX));
    }
    Some((start, end))
}

/// Effective participant-booking window for any non-terminal operation. Authorized work uses
/// its guaranteed first executable minute plus authored/deadline-bounded duration; started work
/// uses the persisted runtime window, with an unresolved decision pause projected through `now`.
/// This is the single booking projection shared by runtime admission and restore validation.
pub(crate) fn resolve_operation_booking_window(
    registry: &Registry,
    operation: &OperationRecord,
    now: SimTime,
) -> Option<(SimTime, SimTime)> {
    if let Some(window) = projected_operation_window(operation, now) {
        return Some(window);
    }
    if operation.status() != OperationStatus::Authorized {
        return None;
    }
    Some(projected_authorized_operation_window(
        registry,
        operation.authorized_at(),
        operation.kind(),
        operation.scheduled_for(),
        operation.constraints(),
    ))
}

/// Reconstructs the booking window visible at a historical instant for a currently non-terminal
/// operation. This is used only for persistence validation: when an overlap exists now because a
/// decision pause grew, the later authorization must still have observed a non-overlapping pair.
pub(crate) fn resolve_operation_booking_window_at(
    registry: &Registry,
    operation: &OperationRecord,
    at: SimTime,
) -> Option<(SimTime, SimTime)> {
    if operation.status() == OperationStatus::Authorized
        || operation
            .started_at()
            .is_none_or(|started_at| at < started_at)
    {
        return Some(projected_authorized_operation_window(
            registry,
            operation.authorized_at(),
            operation.kind(),
            operation.scheduled_for(),
            operation.constraints(),
        ));
    }
    if matches!(
        operation.status(),
        OperationStatus::Completed | OperationStatus::Aborted
    ) {
        return None;
    }
    let start = operation.started_at()?;
    let mut end = operation.resolution_due_at()?;
    if operation.status() == OperationStatus::AwaitingDecision
        && let Some(paused_at) = operation.awaiting_decision_since()
        && paused_at <= at
    {
        end = checked_shift_past_pause(end, pause_duration_minutes(paused_at, at))
            .unwrap_or(SimTime::from_minutes(u64::MAX));
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
    let Some((existing_start, existing_end)) =
        resolve_operation_booking_window(registry, existing, now)
    else {
        return false;
    };
    requested_start < existing_end && existing_start < requested_end
}

pub(crate) fn find_due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state
        .operations
        .find_due_authorized(state.now())
        .into_iter()
        .filter(|operation| {
            state
                .operations
                .get_operation(*operation)
                .is_some_and(|record| resolve_operation_earliest_start(record) <= state.now())
        })
        .collect()
}

/// True once an operation can no longer satisfy its earliest completion deadline. Running or
/// paused work misses only when the deadline minute arrives; authorized work also misses when a
/// delayed begin leaves no executable minute after its authored entry milestone. This lets the
/// scheduler fail closed as soon as waiting has made the authored plan impossible.
pub(crate) fn has_missed_operation_deadline(
    registry: &Registry,
    state: &AppState,
    operation: OperationId,
) -> bool {
    let Some(record) = state.operations.get_operation(operation) else {
        return false;
    };
    let Some(deadline) = resolve_earliest_operation_deadline(record) else {
        return false;
    };
    if state.now() >= deadline {
        return true;
    }
    if record.status() != OperationStatus::Authorized {
        return false;
    }
    resolve_deadline_without_execution_window(
        registry.get_operation(record.kind()).execution(),
        state.now(),
        record.constraints(),
    )
    .is_some()
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
        .map(|operation| {
            (
                resolve_earliest_operation_deadline(operation)
                    .expect("filtered overdue operation must retain a completion deadline"),
                operation.id(),
            )
        })
        .collect::<Vec<_>>();
    // The status indexes are separate, so restore one global chronological order before any
    // deadline abort creates IDs or reports that later work can observe. ID breaks ties only
    // among operations whose deadline is the same instant.
    due.sort_unstable();
    due.into_iter().map(|(_, operation)| operation).collect()
}

pub(crate) fn resolve_earliest_operation_deadline(record: &OperationRecord) -> Option<SimTime> {
    record
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            crate::operations::OperationConstraint::CompleteBy(deadline) => Some(*deadline),
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
    if transition == OperationTransition::Begin {
        let earliest_start = resolve_operation_earliest_start(record);
        if state.now() < earliest_start {
            return Err(OperationError::StartBeforeEarliestStart {
                operation,
                earliest_start,
            });
        }
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
        ensure_version_can_advance(record.version(), "operation")?;
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
        PoliceResponseIntegrationError::SimulationTimeOverflow => {
            OperationError::SimulationTimeOverflow
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
    ensure_version_can_advance(record.version(), "operation")?;
    let earliest_start = resolve_operation_earliest_start(record);
    if state.now() < earliest_start {
        return Err(OperationError::StartBeforeEarliestStart {
            operation,
            earliest_start,
        });
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
    let execution = registry.get_operation(record.kind()).execution();
    if let Some(deadline) =
        resolve_deadline_without_execution_window(execution, state.now(), record.constraints())
    {
        return Err(OperationError::DeadlineMissed {
            operation,
            deadline,
            now: state.now(),
        });
    }
    let duration = execution.duration();
    // A binding deadline compresses the modeled window: the operation resolves on the deadline
    // minute under time pressure (`resolve_time_pressure`), and only a deadline that passes
    // without resolution — a decision-paused operation — is hard-aborted afterwards.
    let mut resolution_due_at = state
        .now()
        .checked_add(duration)
        .ok_or(OperationError::SimulationTimeOverflow)?;
    for constraint in record.constraints() {
        let crate::operations::OperationConstraint::CompleteBy(deadline) = constraint else {
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
    let participants = record.participants();
    for character in &participants {
        if let Some(arrest) = state.legal.active_arrest_for_character(*character) {
            return Err(OperationError::DetainedParticipant {
                character: *character,
                arrest: arrest.id(),
            });
        }
    }
    if let Some((character, conflicting_operation)) = find_busy_participant_for_begin(
        registry,
        state,
        &participants,
        operation,
        state.now(),
        resolution_due_at,
    ) {
        return Err(OperationError::ParticipantBusy {
            character,
            operation: conflicting_operation,
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
