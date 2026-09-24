//! Operation authorization and lifecycle mutation; read-only timing/booking policy lives in `operation_scheduling`, while objective actionability lives in the child validator.

mod authorization_validation;
mod objective_validation;
mod start;

use authorization_validation::{
    resolve_known_extraction_arrest, validate_authorization_constraints,
    validate_authorization_contingencies, validate_authorization_intelligence,
    validate_authorization_participants, validate_deadline_execution_window,
    validate_extraction_basis_current, validate_extraction_custody_window,
    validate_representable_operation_window,
};
use objective_validation::validate_operation_objective;
pub(crate) use objective_validation::{
    is_actionable_opportunity_target, is_information_subject_relevant, is_valid_operation_objective,
};
pub(crate) use start::validate_begin_operation;

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, IdExhaustionError, InformationId, OperationId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::VersionCapacityError;
use crate::history::history_system::HistoryError;
use crate::intelligence::KnowledgeHolder;
use crate::intelligence::intelligence_system::IntelligenceError;
use crate::operations::operation_abort::validate_authority_abort_operation;
use crate::operations::operation_basis_knowledge::known_witness_cases;
use crate::operations::operation_scheduling::{
    find_busy_participant, projected_operation_window, resolve_operation_earliest_start,
};
use crate::operations::operation_state::{checked_shift_past_pause, pause_duration_minutes};
use crate::operations::surveillance_integration::{
    SurveillanceRequestError, validate_surveillance_request,
};
use crate::operations::{
    OperationAbortCause, OperationCommand, OperationDraft, OperationIdentity, OperationKind,
    OperationObjectiveKind, OperationRecord, OperationRuntime, OperationStatus, RoleKind,
};
use crate::registry::Registry;
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
    #[error("operation organization {0} is not a criminal organization")]
    InvalidOrganizationKind(OrganizationId),
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
    #[error(
        "organization lacks current learned legal-status basis for {kind:?} against character {character}"
    )]
    TargetLegalBasisUnknown {
        kind: OperationKind,
        character: crate::core::id::CharacterId,
    },
    #[error(
        "detainee {character} is already the extraction target of non-terminal operation {operation}"
    )]
    DetaineeAlreadyTargeted {
        character: crate::core::id::CharacterId,
        operation: OperationId,
    },
    #[error(
        "learned extraction basis for character {character} changed after authorization validation; expected arrest {expected}, found {found:?}"
    )]
    StaleExtractionBasis {
        character: CharacterId,
        expected: ArrestId,
        found: Option<ArrestId>,
    },
    #[error(
        "known witness-case basis for character {character} changed after authorization validation"
    )]
    StaleWitnessPressureBasis { character: CharacterId },
    #[error(
        "extraction for detainee {character} would finish at {planned_end:?}, but custody ends at {custody_ends_at:?} and must still be active through completion"
    )]
    ExtractionMissesCustodyWindow {
        character: CharacterId,
        planned_end: SimTime,
        custody_ends_at: SimTime,
    },
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
    #[error("operation objective target {character} cannot also be a crew participant")]
    ObjectiveTargetIsParticipant { character: CharacterId },
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
    #[error(
        "operation {operation} cannot resume with projected completion {projected_due_at:?} after hard deadline {deadline:?}"
    )]
    ResumeExceedsCompletionDeadline {
        operation: OperationId,
        projected_due_at: SimTime,
        deadline: SimTime,
    },
    #[error("plan lacks usable required {0:?} intelligence at its earliest planned start")]
    MissingRequiredIntelligenceTopic(crate::intelligence::InformationTopic),
    #[error("business {0} has no active operating economy for this operation")]
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
    #[error(
        "operation {operation} cannot begin because linked opportunity {opportunity} expired at {valid_until:?} before {now:?}"
    )]
    OpportunityWindowExpired {
        operation: OperationId,
        opportunity: crate::core::id::OpportunityId,
        valid_until: SimTime,
        now: SimTime,
    },
    #[error("operation {operation} has not missed a completion deadline")]
    DeadlineNotMissed { operation: OperationId },
    #[error("operation {operation} objective is no longer actionable: {blocker:?}")]
    ObjectiveUnavailable {
        operation: OperationId,
        blocker: crate::operations::OperationObjectiveBlocker,
    },
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
    witness_pressure_cases: BTreeSet<CaseWitnessId>,
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
        validate_extraction_basis_current(
            self.registry,
            state,
            self.draft.responsible_organization,
            &self.draft.objective,
            self.extraction_arrest,
        )?;
        validate_operation_objective(
            self.registry,
            state,
            self.draft.kind,
            self.draft.responsible_organization,
            &self.draft.objective,
        )?;
        let current_witness_pressure_cases =
            resolve_witness_pressure_cases(self.registry, state, &self.draft)?;
        if current_witness_pressure_cases != self.witness_pressure_cases {
            let character = match self.draft.objective {
                crate::operations::OperationObjective::Frighten {
                    target: EntityRef::Character(character),
                } => character,
                crate::operations::OperationObjective::AcquireProperty { .. }
                | crate::operations::OperationObjective::ObtainCash { .. }
                | crate::operations::OperationObjective::Frighten { .. }
                | crate::operations::OperationObjective::GatherInformation { .. }
                | crate::operations::OperationObjective::FreeDetainee { .. }
                | crate::operations::OperationObjective::DisruptBusiness { .. } => unreachable!(
                    "witness pressure case set is non-empty only for character frighten"
                ),
            };
            return Err(OperationError::StaleWitnessPressureBasis { character });
        }
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
                witness_pressure_cases: self.witness_pressure_cases,
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
    let organization = state
        .world
        .get_organization(draft.responsible_organization)
        .ok_or(OperationError::MissingOrganization(
            draft.responsible_organization,
        ))?;
    if organization.kind() != crate::world::OrganizationKind::Criminal {
        return Err(OperationError::InvalidOrganizationKind(
            draft.responsible_organization,
        ));
    }
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
        SurveillanceRequestError::InvalidObjective => OperationError::InvalidSurveillanceObjective,
        SurveillanceRequestError::UnsupportedTarget(target) => {
            OperationError::UnsupportedSurveillanceTarget(target)
        }
    })?;
    validate_operation_objective(
        registry,
        state,
        draft.kind,
        draft.responsible_organization,
        &draft.objective,
    )?;
    let extraction_arrest = resolve_known_extraction_arrest(
        registry,
        state,
        draft.responsible_organization,
        &draft.objective,
    )?;
    let witness_pressure_cases = resolve_witness_pressure_cases(registry, state, &draft)?;
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
    if let Some(character) =
        crate::operations::operation_objective::character_objective_target(&draft.objective)
        && participants.contains(&character)
    {
        return Err(OperationError::ObjectiveTargetIsParticipant { character });
    }
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
        witness_pressure_cases,
        registry,
    })
}

fn resolve_witness_pressure_cases(
    registry: &Registry,
    state: &AppState,
    draft: &OperationDraft,
) -> Result<BTreeSet<CaseWitnessId>, OperationError> {
    let (
        OperationKind::WitnessPressure,
        crate::operations::OperationObjective::Frighten {
            target: EntityRef::Character(character),
        },
    ) = (draft.kind, &draft.objective)
    else {
        return Ok(BTreeSet::new());
    };
    let known = known_witness_cases(registry, state, draft.responsible_organization, *character);
    if known.is_empty() {
        return Err(OperationError::TargetLegalBasisUnknown {
            kind: OperationKind::WitnessPressure,
            character: *character,
        });
    }
    Ok(known)
}

/// Validates that resuming a decision-blocked operation at `resumed_at` remains inside any hard
/// completion deadline and does not double-book its participants. Resuming shifts the resolution
/// deadline forward by the pause duration, so the post-resume window can exceed an authored
/// deadline or collide with operations authorized while this one was paused.
///
/// Operations that have not yet begun keep no persisted end time, so an authorized operation whose
/// start falls inside the resumed window is treated as a conflict; its duration cannot shorten the
/// overlap because a start inside the window always overlaps it.
pub(crate) fn validate_operation_resume(
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
    if let Some(deadline) = record.completion_deadline()
        && shifted_due_at > deadline
    {
        return Err(OperationError::ResumeExceedsCompletionDeadline {
            operation: operation_id,
            projected_due_at: shifted_due_at,
            deadline,
        });
    }
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
/// participant conflicts and hard-deadline viability are preflighted by
/// `validate_operation_resume`; this
/// mutation is deliberately infallible after that validation so the decision record and operation
/// lifecycle can commit as one cross-domain transaction.
pub(crate) fn apply_decision_resume_preflighted(
    state: &mut AppState,
    operation: OperationId,
    resumed_at: SimTime,
) {
    state.operations.resume(operation, resumed_at);
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

#[cfg(test)]
mod tests;
