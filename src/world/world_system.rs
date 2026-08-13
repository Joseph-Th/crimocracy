//! Canonical world mutation systems; sibling `world` types remain passive records and indexes.

use crate::core::id::{
    BusinessId, CharacterId, MandateId, NeighborhoodId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::operations::OperationStatus;
use crate::registry::Registry;
use crate::world::{
    BusinessDraft, BusinessOwner, BusinessRecord, CharacterCapabilities, CharacterDisposition,
    CharacterDraft, CharacterIdentity, CharacterMembership, CharacterRecord, CharacterRuntime,
    Lifecycle, NeighborhoodDraft, NeighborhoodRecord, OrganizationDraft, OrganizationKind,
    OrganizationRecord, PolicySetting,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum WorldError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("supervisor {supervisor} does not belong to requested organization {organization:?}")]
    SupervisorOrganizationMismatch {
        supervisor: CharacterId,
        organization: Option<OrganizationId>,
    },
    #[error("character {character} cannot supervise itself")]
    SelfSupervision { character: CharacterId },
    #[error("reassignment would create a supervision cycle involving character {character}")]
    SupervisionCycle { character: CharacterId },
    #[error("character {character} is assigned to active operation {operation}")]
    ActiveOperationAssignment {
        character: CharacterId,
        operation: OperationId,
    },
    #[error("character {character} owns active mandate {mandate}")]
    ActiveMandateAssignment {
        character: CharacterId,
        mandate: MandateId,
    },
    #[error("character {character} changed after validation; expected version {expected}, found {found}")]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "organization {0} is not a criminal organization and cannot be the player organization"
    )]
    InvalidPlayerOrganization(OrganizationId),
}

pub fn insert_organization(
    registry: &Registry,
    state: &mut AppState,
    draft: OrganizationDraft,
) -> Result<OrganizationId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    let id = state.ids.next_organization();
    let policies = registry.default_policies();
    state.world.insert_organization(OrganizationRecord {
        id,
        name: draft.name,
        kind: draft.kind,
        lifecycle: Lifecycle::Active,
        policies,
    });
    Ok(id)
}

pub fn designate_player_organization(
    state: &mut AppState,
    organization: OrganizationId,
) -> Result<(), WorldError> {
    let record = state
        .world
        .get_organization(organization)
        .ok_or(WorldError::MissingOrganization(organization))?;
    if record.kind() != OrganizationKind::Criminal {
        return Err(WorldError::InvalidPlayerOrganization(organization));
    }
    state.set_player_organization(organization);
    Ok(())
}

pub fn insert_neighborhood(
    state: &mut AppState,
    draft: NeighborhoodDraft,
) -> Result<NeighborhoodId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    let id = state.ids.next_neighborhood();
    state.world.insert_neighborhood(NeighborhoodRecord {
        id,
        name: draft.name,
        lifecycle: Lifecycle::Active,
    });
    Ok(id)
}

pub fn insert_character(
    registry: &Registry,
    state: &mut AppState,
    draft: CharacterDraft,
) -> Result<CharacterId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    validate_membership(state, draft.organization, draft.supervisor)?;
    for kind in draft.capabilities.keys() {
        registry.get_capability(*kind);
    }
    for kind in &draft.traits {
        registry.get_trait(*kind);
    }

    let id = state.ids.next_character();
    state.world.insert_character(CharacterRecord {
        identity: CharacterIdentity {
            id,
            name: draft.name,
        },
        membership: CharacterMembership {
            organization: draft.organization,
            supervisor: draft.supervisor,
            autonomy: draft.autonomy,
        },
        capabilities: CharacterCapabilities {
            ratings: draft.capabilities,
        },
        disposition: CharacterDisposition {
            traits: draft.traits,
            drives: draft.drives,
        },
        runtime: CharacterRuntime {
            lifecycle: Lifecycle::Active,
            version: 1,
        },
    });
    Ok(id)
}

#[derive(Debug)]
pub struct ValidatedCharacterReassignment {
    character: CharacterId,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    expected_version: u32,
}

impl ValidatedCharacterReassignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), WorldError> {
        let record = state
            .world
            .get_character(self.character)
            .ok_or(WorldError::MissingCharacter(self.character))?;
        if record.version() != self.expected_version {
            return Err(WorldError::StaleCharacter {
                character: self.character,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        state
            .world
            .reassign_character(self.character, self.organization, self.supervisor);
        Ok(())
    }
}

pub fn validate_reassign_character(
    state: &AppState,
    character: CharacterId,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
) -> Result<ValidatedCharacterReassignment, WorldError> {
    let record = state
        .world
        .get_character(character)
        .ok_or(WorldError::MissingCharacter(character))?;
    if supervisor == Some(character) {
        return Err(WorldError::SelfSupervision { character });
    }
    validate_membership(state, organization, supervisor)?;

    if organization != record.organization() {
        if let Some(mandate) = state.delegation.active_for_manager(character) {
            return Err(WorldError::ActiveMandateAssignment {
                character,
                mandate: mandate.id(),
            });
        }
        for operation in state.operations.operations() {
            match operation.status() {
                OperationStatus::Authorized
                | OperationStatus::InProgress
                | OperationStatus::AwaitingDecision => {
                    if operation.leader() == character
                        || operation
                            .roles()
                            .values()
                            .any(|participant| *participant == character)
                    {
                        return Err(WorldError::ActiveOperationAssignment {
                            character,
                            operation: operation.id(),
                        });
                    }
                }
                OperationStatus::Completed | OperationStatus::Aborted => {}
            }
        }
    }

    let mut cursor = supervisor;
    while let Some(current) = cursor {
        if current == character {
            return Err(WorldError::SupervisionCycle { character });
        }
        cursor = state
            .world
            .get_character(current)
            .and_then(|record| record.supervisor());
    }

    Ok(ValidatedCharacterReassignment {
        character,
        organization,
        supervisor,
        expected_version: record.version(),
    })
}

pub fn insert_business(
    state: &mut AppState,
    draft: BusinessDraft,
) -> Result<BusinessId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    if state.world.get_neighborhood(draft.neighborhood).is_none() {
        return Err(WorldError::MissingNeighborhood(draft.neighborhood));
    }
    match draft.owner {
        BusinessOwner::Independent => {}
        BusinessOwner::Organization(id) => {
            if state.world.get_organization(id).is_none() {
                return Err(WorldError::MissingOrganization(id));
            }
        }
        BusinessOwner::Character(id) => {
            if state.world.get_character(id).is_none() {
                return Err(WorldError::MissingCharacter(id));
            }
        }
    }

    let id = state.ids.next_business();
    state.world.insert_business(BusinessRecord {
        id,
        name: draft.name,
        neighborhood: draft.neighborhood,
        owner: draft.owner,
        lifecycle: Lifecycle::Active,
    });
    Ok(id)
}

pub fn set_policy(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    setting: PolicySetting,
) -> Result<(), WorldError> {
    if state.world.get_organization(organization).is_none() {
        return Err(WorldError::MissingOrganization(organization));
    }
    registry.get_policy(setting.kind());
    state.world.set_policy(organization, setting);
    Ok(())
}

fn validate_membership(
    state: &AppState,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
) -> Result<(), WorldError> {
    if let Some(organization_id) = organization {
        if state.world.get_organization(organization_id).is_none() {
            return Err(WorldError::MissingOrganization(organization_id));
        }
    }
    if let Some(supervisor_id) = supervisor {
        let supervisor_record = state
            .world
            .get_character(supervisor_id)
            .ok_or(WorldError::MissingCharacter(supervisor_id))?;
        if supervisor_record.organization() != organization {
            return Err(WorldError::SupervisorOrganizationMismatch {
                supervisor: supervisor_id,
                organization,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    fn make_test_character(
        registry: &Registry,
        state: &mut AppState,
        name: &str,
        organization: OrganizationId,
        supervisor: Option<CharacterId>,
    ) -> CharacterId {
        insert_character(
            registry,
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("test character should validate")
    }

    #[test]
    fn reassignment_rejects_supervision_cycle_without_mutation() {
        let registry = build_registry();
        let mut state = AppState::new(7);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let lieutenant = make_test_character(
            &registry,
            &mut state,
            "Lieutenant",
            organization,
            Some(boss),
        );
        let soldier = make_test_character(
            &registry,
            &mut state,
            "Soldier",
            organization,
            Some(lieutenant),
        );

        let error = validate_reassign_character(&state, boss, Some(organization), Some(soldier))
            .expect_err("cycle must be rejected before mutation");
        assert_eq!(error, WorldError::SupervisionCycle { character: boss });
        assert_eq!(
            state
                .world
                .get_character(boss)
                .expect("boss should still exist")
                .supervisor(),
            None
        );
        assert_eq!(state.world.direct_reports(lieutenant).count(), 1);
        validate_invariants(&state);
    }

    #[test]
    fn reassignment_updates_hierarchy_indexes_atomically() {
        let registry = build_registry();
        let mut state = AppState::new(11);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let lieutenant = make_test_character(
            &registry,
            &mut state,
            "Lieutenant",
            organization,
            Some(boss),
        );
        let soldier = make_test_character(
            &registry,
            &mut state,
            "Soldier",
            organization,
            Some(lieutenant),
        );

        validate_reassign_character(&state, soldier, Some(organization), Some(boss))
            .expect("valid reassignment should produce a token")
            .commit(&mut state)
            .expect("validated reassignment should remain current");

        assert_eq!(state.world.direct_reports(lieutenant).count(), 0);
        assert_eq!(state.world.direct_reports(boss).count(), 2);
        validate_invariants(&state);
    }

    #[test]
    fn unassigned_character_cannot_have_organization_supervisor() {
        let registry = build_registry();
        let mut state = AppState::new(13);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let supervisor =
            make_test_character(&registry, &mut state, "Supervisor", organization, None);

        let error = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Unassigned".to_owned(),
                organization: None,
                supervisor: Some(supervisor),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect_err("unassigned character must not enter an organization hierarchy");

        assert_eq!(
            error,
            WorldError::SupervisorOrganizationMismatch {
                supervisor,
                organization: None,
            }
        );
        validate_invariants(&state);
    }

    #[test]
    fn stale_reassignment_token_cannot_overwrite_newer_membership() {
        let registry = build_registry();
        let mut state = AppState::new(17);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let first = make_test_character(&registry, &mut state, "First", organization, Some(boss));
        let second = make_test_character(&registry, &mut state, "Second", organization, Some(boss));
        let member =
            make_test_character(&registry, &mut state, "Member", organization, Some(first));

        let stale = validate_reassign_character(&state, member, Some(organization), Some(second))
            .expect("first reassignment should validate");
        let current = validate_reassign_character(&state, member, Some(organization), Some(boss))
            .expect("second reassignment should validate against the same snapshot");
        current
            .commit(&mut state)
            .expect("current reassignment should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale reassignment must not overwrite newer membership");
        assert_eq!(
            error,
            WorldError::StaleCharacter {
                character: member,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .world()
                .get_character(member)
                .expect("member should exist")
                .supervisor(),
            Some(boss)
        );
        validate_invariants(&state);
    }
}
