//! Release-safe structural validation for the world subsystem.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::registry::Registry;
use crate::world::{
    ALL_POLICY_KINDS, BusinessOwner, BusinessRecord, CharacterRecord, OrganizationKind,
    OrganizationRecord, PolicySetting,
};
use std::collections::BTreeSet;

pub(super) fn validate_world_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_player_organization(state)?;
    for organization in state.world.organizations() {
        validate_organization(organization)?;
    }
    for neighborhood in state.world.neighborhoods() {
        if neighborhood.name().trim().is_empty() {
            return Err(StateValidationError::EmptyEntityName {
                entity: EntityRef::Neighborhood(neighborhood.id()),
            });
        }
    }

    for character in state.world.characters() {
        validate_character(state, character)?;
    }
    validate_supervision_graph(state)?;
    for business in state.world.businesses() {
        validate_business(state, business)?;
    }
    Ok(())
}

fn validate_player_organization(state: &AppState) -> Result<(), StateValidationError> {
    let Some(player) = state.player_organization() else {
        return Ok(());
    };
    let organization =
        state
            .world
            .get_organization(player)
            .ok_or(StateValidationError::MissingEntity {
                context: "player organization",
                entity: EntityRef::Organization(player),
            })?;
    if organization.kind() != OrganizationKind::Criminal {
        return Err(StateValidationError::InvalidPlayerOrganization {
            organization: player,
        });
    }
    Ok(())
}

fn validate_organization(organization: &OrganizationRecord) -> Result<(), StateValidationError> {
    if organization.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Organization(organization.id()),
        });
    }
    for policy in ALL_POLICY_KINDS {
        let setting = organization
            .policy(policy)
            .ok_or(StateValidationError::MissingPolicy {
                organization: organization.id(),
                policy,
            })?;
        if setting.kind() != policy {
            return Err(StateValidationError::PolicyKindMismatch {
                organization: organization.id(),
                expected: policy,
                actual: setting.kind(),
            });
        }
        if organization.policy_version(policy) == Some(0) {
            return Err(StateValidationError::InvalidOrganizationPolicyVersion {
                organization: organization.id(),
                policy,
            });
        }
        if organization.policy_version(policy).is_none() {
            return Err(StateValidationError::MissingPolicy {
                organization: organization.id(),
                policy,
            });
        }
    }
    Ok(())
}

/// Registry-aware reachability for versioned organization policies. The registry authors version
/// one; every later version is a real setting change because the canonical writer does not bump
/// no-op writes. Binary recruitment policy therefore alternates exactly. Three-state legal
/// support can return to any setting after two changes, but version two still cannot equal the
/// authored default.
pub(in crate::core::invariants) fn validate_organization_policies_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for organization in state.world.organizations() {
        for kind in ALL_POLICY_KINDS {
            let current = organization
                .policy(kind)
                .ok_or(StateValidationError::MissingPolicy {
                    organization: organization.id(),
                    policy: kind,
                })?;
            let version =
                organization
                    .policy_version(kind)
                    .ok_or(StateValidationError::MissingPolicy {
                        organization: organization.id(),
                        policy: kind,
                    })?;
            let default = registry.get_policy(kind).default();
            let reachable = match (default, current) {
                (
                    PolicySetting::IndependentRecruitment(default),
                    PolicySetting::IndependentRecruitment(current),
                ) => {
                    version > 0
                        && if !version.is_multiple_of(2) {
                            current == default
                        } else {
                            current != default
                        }
                }
                (
                    PolicySetting::AssociateLegalSupport(default),
                    PolicySetting::AssociateLegalSupport(current),
                ) => match version {
                    0 => false,
                    1 => current == default,
                    2 => current != default,
                    _ => true,
                },
                _ => false,
            };
            if !reachable {
                return Err(StateValidationError::InvalidOrganizationPolicyVersion {
                    organization: organization.id(),
                    policy: kind,
                });
            }
        }
    }
    Ok(())
}

fn validate_character(
    state: &AppState,
    character: &CharacterRecord,
) -> Result<(), StateValidationError> {
    if character.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Character(character.id()),
        });
    }
    if character.version() == 0 {
        return Err(StateValidationError::InvalidCharacterVersion {
            character: character.id(),
        });
    }
    if let Some(organization) = character.organization()
        && state.world.get_organization(organization).is_none()
    {
        return Err(StateValidationError::MissingEntity {
            context: "character organization",
            entity: EntityRef::Organization(organization),
        });
    }
    if let Some(supervisor) = character.supervisor() {
        let supervisor_record =
            state
                .world
                .get_character(supervisor)
                .ok_or(StateValidationError::MissingEntity {
                    context: "character supervisor",
                    entity: EntityRef::Character(supervisor),
                })?;
        if supervisor_record.organization() != character.organization() {
            return Err(StateValidationError::SupervisorOrganizationMismatch {
                character: character.id(),
                supervisor,
            });
        }
    }
    Ok(())
}

fn validate_supervision_graph(state: &AppState) -> Result<(), StateValidationError> {
    // Supervision is a functional graph: every character has at most one outgoing supervisor
    // edge. Once one chain reaches the top, every node on that path is proven acyclic and later
    // starts can stop there. This avoids re-walking the same long ancestor chain for every
    // subordinate while keeping the traversal iterative and deterministic.
    let mut validated = BTreeSet::new();
    let mut current_path = BTreeSet::new();
    let mut path_nodes = Vec::new();
    for character in state.world.characters() {
        if validated.contains(&character.id()) {
            continue;
        }
        current_path.clear();
        path_nodes.clear();
        let mut cursor = Some(character.id());
        while let Some(current) = cursor {
            if validated.contains(&current) {
                break;
            }
            if !current_path.insert(current) {
                return Err(StateValidationError::SupervisionCycle {
                    character: character.id(),
                });
            }
            path_nodes.push(current);
            cursor = state
                .world
                .get_character(current)
                .ok_or(StateValidationError::MissingEntity {
                    context: "supervision hierarchy",
                    entity: EntityRef::Character(current),
                })?
                .supervisor();
        }
        for id in path_nodes.drain(..) {
            validated.insert(id);
        }
    }
    Ok(())
}

fn validate_business(
    state: &AppState,
    business: &BusinessRecord,
) -> Result<(), StateValidationError> {
    if business.name().trim().is_empty() {
        return Err(StateValidationError::EmptyEntityName {
            entity: EntityRef::Business(business.id()),
        });
    }
    if state
        .world
        .get_neighborhood(business.neighborhood())
        .is_none()
    {
        return Err(StateValidationError::MissingEntity {
            context: "business neighborhood",
            entity: EntityRef::Neighborhood(business.neighborhood()),
        });
    }
    if let Some(entity) = business_owner_entity(business.owner())
        && !is_entity_present(state, entity)
    {
        return Err(StateValidationError::MissingEntity {
            context: "business owner",
            entity,
        });
    }
    if business.version() == 0
        || state
            .world
            .get_business_ownership_change_for_version(business.id(), business.version())
            .is_none_or(|change| change.new_owner() != business.owner())
    {
        return Err(invalid_business_history(business));
    }
    validate_business_history(state, business)
}

fn validate_business_history(
    state: &AppState,
    business: &BusinessRecord,
) -> Result<(), StateValidationError> {
    for change in state.world.business_ownership_history(business.id()) {
        if change.changed_at() > state.now() {
            return Err(invalid_business_history(business));
        }
        for historical_owner in [change.previous_owner(), Some(change.new_owner())]
            .into_iter()
            .flatten()
        {
            if business_owner_entity(historical_owner)
                .is_some_and(|entity| !is_entity_present(state, entity))
            {
                return Err(invalid_business_history(business));
            }
        }
    }
    Ok(())
}

fn business_owner_entity(owner: BusinessOwner) -> Option<EntityRef> {
    match owner {
        BusinessOwner::Independent => None,
        BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
        BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
    }
}

fn invalid_business_history(business: &BusinessRecord) -> StateValidationError {
    StateValidationError::InvalidBusinessOwnershipHistory {
        business: business.id(),
    }
}
