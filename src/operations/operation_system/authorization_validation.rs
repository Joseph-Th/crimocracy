//! Operation-authorization validation helpers.
//!
//! The parent owner retains the public authorization transaction. This module isolates plan
//! semantics and freshness checks so authorization does not accrete unrelated lifecycle logic.

use super::OperationError;
use super::objective_validation::is_information_subject_relevant;
use crate::core::id::{ArrestId, CharacterId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::KnowledgeHolder;
use crate::operations::operation_intelligence::resolve_information_score;
use crate::operations::operation_scheduling::{
    checked_earliest_operation_start_from_authorization,
    earliest_operation_start_from_authorization, projected_authorized_operation_window,
    resolve_deadline_without_execution_window,
};
use crate::operations::{OperationDraft, OperationObjective};
use crate::registry::{OperationDefinition, Registry};
use std::collections::{BTreeMap, BTreeSet};

/// Validates the operation's required seats and participant availability while collecting the
/// version pins consumed by the authorization token.
pub(super) fn validate_authorization_participants(
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

pub(super) fn validate_authorization_intelligence(
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

pub(super) fn validate_authorization_constraints(
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

pub(super) fn validate_authorization_contingencies(
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
/// begin/entry boundary. Authorization tokens may commit later than validation, so callers pass
/// the authorization instant whose window they are proving.
pub(super) fn validate_representable_operation_window(
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

pub(super) fn validate_deadline_execution_window(
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

/// Rejects an extraction that cannot finish before the known custody episode's authored maximum
/// detention boundary. Early release is a hidden execution fact unless the organization later
/// learns it, so this check deliberately uses only the pinned episode's arrest time.
pub(super) fn validate_extraction_custody_window(
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

/// Pins an extraction plan to the freshest detention episode the organization has actually
/// learned about rather than whichever arrest happens to be active in hidden legal state.
pub(super) fn resolve_known_extraction_arrest(
    registry: &Registry,
    state: &AppState,
    organization: crate::core::id::OrganizationId,
    objective: &OperationObjective,
) -> Result<Option<ArrestId>, OperationError> {
    let OperationObjective::FreeDetainee { target } = objective else {
        return Ok(None);
    };
    crate::operations::operation_basis_knowledge::known_detention_arrest(
        registry,
        state,
        organization,
        *target,
    )
    .map(Some)
    .ok_or(OperationError::TargetLegalBasisUnknown {
        kind: crate::operations::OperationKind::Extraction,
        character: *target,
    })
}

pub(super) fn validate_extraction_basis_current(
    registry: &Registry,
    state: &AppState,
    organization: crate::core::id::OrganizationId,
    objective: &OperationObjective,
    expected: Option<ArrestId>,
) -> Result<(), OperationError> {
    let OperationObjective::FreeDetainee { target } = objective else {
        debug_assert!(expected.is_none());
        return Ok(());
    };
    let expected = expected.expect("validated extraction must retain its custody link");
    let found = crate::operations::operation_basis_knowledge::known_detention_arrest(
        registry,
        state,
        organization,
        *target,
    );
    if found != Some(expected) {
        return Err(OperationError::StaleExtractionBasis {
            character: *target,
            expected,
            found,
        });
    }
    Ok(())
}
