//! Release-safe structural validation for social relationships.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::social::RelationshipRecord;

pub(super) fn validate_social(state: &AppState) -> Result<(), StateValidationError> {
    for relationship in state.social.relationships() {
        validate_relationship(state, relationship)?;
    }
    Ok(())
}

fn validate_relationship(
    state: &AppState,
    relationship: &RelationshipRecord,
) -> Result<(), StateValidationError> {
    if relationship.from() == relationship.to() || relationship.version() == 0 {
        return Err(StateValidationError::InvalidRelationship {
            from: relationship.from(),
            to: relationship.to(),
        });
    }
    for (context, entity) in [
        (
            "relationship source",
            EntityRef::Character(relationship.from()),
        ),
        (
            "relationship target",
            EntityRef::Character(relationship.to()),
        ),
    ] {
        if !is_entity_present(state, entity) {
            return Err(StateValidationError::MissingEntity { context, entity });
        }
    }
    Ok(())
}
