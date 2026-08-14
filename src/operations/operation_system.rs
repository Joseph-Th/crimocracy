//! Operation validation and lifecycle execution; sibling records contain no resolution logic.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{CharacterId, InformationId, OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::operations::{
    OperationCommand, OperationDraft, OperationIdentity, OperationRecord, OperationRuntime,
    OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::world::Lifecycle;
use std::collections::BTreeMap;
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
        let id = state.ids.next_operation();
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
                awaiting_decision_since: None,
                resolution: None,
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
    if draft.scheduled_for < state.now() {
        return Err(OperationError::ScheduledInPast);
    }
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
    for participant in draft.roles.values() {
        let record = state
            .world
            .get_character(*participant)
            .ok_or(OperationError::MissingCharacter(*participant))?;
        if record.lifecycle() != Lifecycle::Active {
            return Err(OperationError::InactiveParticipant(*participant));
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

pub(crate) fn due_authorized_operations(state: &AppState) -> Vec<OperationId> {
    state.operations.due_authorized_at_or_before(state.now())
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
            let duration = registry.get_operation(record.kind()).execution().duration();
            let mut resolution_due_at = state.now() + duration;
            for constraint in record.constraints() {
                if let crate::operations::OperationConstraint::CompleteBefore(deadline) = constraint
                {
                    if *deadline < resolution_due_at {
                        resolution_due_at = *deadline;
                    }
                }
            }
            state
                .operations
                .begin(operation, state.now(), resolution_due_at);
        }
        (OperationStatus::Authorized, OperationTransition::Abort)
        | (OperationStatus::InProgress, OperationTransition::Abort) => {
            state.operations.abort(operation);
        }
        (OperationStatus::InProgress, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Begin)
        | (OperationStatus::AwaitingDecision, OperationTransition::Abort)
        | (OperationStatus::Completed, OperationTransition::Begin)
        | (OperationStatus::Completed, OperationTransition::Abort)
        | (OperationStatus::Aborted, OperationTransition::Begin)
        | (OperationStatus::Aborted, OperationTransition::Abort) => {
            return Err(OperationError::InvalidTransition { status, transition });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::validate_invariants;
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
        .commit(&mut state);
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
        .commit(&mut state);

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
