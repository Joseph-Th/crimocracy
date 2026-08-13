//! Operation validation and lifecycle execution; sibling records contain no resolution logic.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{CharacterId, OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::operations::{
    OperationCommand, OperationDraft, OperationIdentity, OperationRecord, OperationRuntime,
    OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::world::Lifecycle;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationTransition {
    Begin,
    Complete,
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
    #[error(
        "character {leader} is not an active member of responsible organization {organization}"
    )]
    InvalidLeader {
        leader: CharacterId,
        organization: OrganizationId,
    },
    #[error("operation approach is not supported by the operation definition")]
    UnsupportedApproach,
    #[error("operation is missing required role {0:?}")]
    MissingRequiredRole(RoleKind),
    #[error("operation is scheduled in the past")]
    ScheduledInPast,
    #[error("operation completion deadline is earlier than its scheduled start")]
    DeadlineBeforeStart,
    #[error("excluded character {0} is assigned to the operation")]
    ExcludedParticipant(CharacterId),
    #[error("operation {0} does not exist")]
    MissingOperation(OperationId),
    #[error("operation {0} cannot begin before its scheduled time")]
    StartBeforeScheduled(OperationId),
    #[error("transition {transition:?} is invalid from status {status:?}")]
    InvalidTransition {
        status: OperationStatus,
        transition: OperationTransition,
    },
}

#[derive(Debug)]
pub struct ValidatedOperation {
    draft: OperationDraft,
}

impl ValidatedOperation {
    pub fn commit(self, state: &mut AppState) -> OperationId {
        let OperationDraft {
            title,
            kind,
            responsible_organization,
            leader,
            objective,
            approach,
            roles,
            constraints,
            contingencies,
            scheduled_for,
        } = self.draft;
        let id = state.ids.next_operation();
        state.operations.insert(OperationRecord {
            identity: OperationIdentity { id, title, kind },
            command: OperationCommand {
                responsible_organization,
                leader,
                objective,
                approach,
                roles,
                constraints,
                contingencies,
                scheduled_for,
            },
            runtime: OperationRuntime {
                status: OperationStatus::Authorized,
                version: 1,
            },
        });
        id
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
    if draft.scheduled_for < state.now() {
        return Err(OperationError::ScheduledInPast);
    }

    let definition = registry.get_operation(draft.kind);
    if !definition.supported_approaches().contains(&draft.approach) {
        return Err(OperationError::UnsupportedApproach);
    }
    for role in definition.required_roles() {
        if !draft.roles.contains_key(role) {
            return Err(OperationError::MissingRequiredRole(*role));
        }
    }
    for participant in draft.roles.values() {
        if state.world.get_character(*participant).is_none() {
            return Err(OperationError::MissingCharacter(*participant));
        }
    }
    for entity in draft.objective.referenced_entities() {
        if !is_entity_present(state, entity) {
            return Err(OperationError::MissingEntity(entity));
        }
    }
    for constraint in &draft.constraints {
        match constraint {
            crate::operations::OperationConstraint::AvoidCasualties
            | crate::operations::OperationConstraint::DoNotHarmEmployees
            | crate::operations::OperationConstraint::AvoidFirearms
            | crate::operations::OperationConstraint::ProtectLeadershipIdentity
            | crate::operations::OperationConstraint::PreserveMerchandise => {}
            crate::operations::OperationConstraint::CompleteBefore(deadline) => {
                if *deadline < draft.scheduled_for {
                    return Err(OperationError::DeadlineBeforeStart);
                }
            }
            crate::operations::OperationConstraint::ExcludeCharacter(character) => {
                if state.world.get_character(*character).is_none() {
                    return Err(OperationError::MissingCharacter(*character));
                }
                if *character == draft.leader || draft.roles.values().any(|id| id == character) {
                    return Err(OperationError::ExcludedParticipant(*character));
                }
            }
        }
    }
    for contingency in &draft.contingencies {
        if let crate::operations::OperationContingency::ContactIfDetained(character) = contingency {
            if state.world.get_character(*character).is_none() {
                return Err(OperationError::MissingCharacter(*character));
            }
        }
    }

    Ok(ValidatedOperation { draft })
}

pub(crate) fn due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state
        .operations
        .operations_with_status(OperationStatus::Authorized)
        .filter(|operation| operation.scheduled_for() <= state.now())
        .map(|operation| operation.id())
        .collect()
}

pub fn apply_transition(
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
    let next = match (status, transition) {
        (OperationStatus::Authorized, OperationTransition::Begin) => OperationStatus::InProgress,
        (OperationStatus::Authorized, OperationTransition::Abort) => OperationStatus::Aborted,
        (OperationStatus::InProgress, OperationTransition::Complete) => OperationStatus::Completed,
        (OperationStatus::InProgress, OperationTransition::Abort) => OperationStatus::Aborted,
        (OperationStatus::Authorized, OperationTransition::Complete)
        | (OperationStatus::InProgress, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Complete)
        | (OperationStatus::AwaitingDecision, OperationTransition::Abort)
        | (OperationStatus::Completed, OperationTransition::Begin)
        | (OperationStatus::Completed, OperationTransition::Complete)
        | (OperationStatus::Completed, OperationTransition::Abort)
        | (OperationStatus::Aborted, OperationTransition::Begin)
        | (OperationStatus::Aborted, OperationTransition::Complete)
        | (OperationStatus::Aborted, OperationTransition::Abort) => {
            return Err(OperationError::InvalidTransition { status, transition });
        }
    };
    state.operations.transition(operation, next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::validate_invariants;
    use crate::core::time::SimTime;
    use crate::operations::{
        OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
    };
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessOwner, CharacterDraft, NeighborhoodDraft,
        OrganizationDraft, OrganizationKind,
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
            },
        )
        .expect("neighborhood fixture should validate");
        let business = insert_business(
            &mut state,
            BusinessDraft {
                name: "Test Business".to_owned(),
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
        .commit(&mut state);

        apply_transition(&mut state, operation, OperationTransition::Begin)
            .expect("authorized operation should begin");
        apply_transition(&mut state, operation, OperationTransition::Complete)
            .expect("in-progress operation should complete");
        let before = state
            .operations
            .get_operation(operation)
            .expect("operation should exist")
            .version();

        let error = apply_transition(&mut state, operation, OperationTransition::Abort)
            .expect_err("terminal operation must reject further transitions");
        assert_eq!(
            error,
            OperationError::InvalidTransition {
                status: OperationStatus::Completed,
                transition: OperationTransition::Abort,
            }
        );
        let record = state
            .operations
            .get_operation(operation)
            .expect("operation should still exist");
        assert_eq!(record.status(), OperationStatus::Completed);
        assert_eq!(record.version(), before);
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
            .commit(&mut state);
        let version = state
            .operations()
            .get_operation(operation)
            .expect("operation should exist")
            .version();

        let error = apply_transition(&mut state, operation, OperationTransition::Begin)
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
}
