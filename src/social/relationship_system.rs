//! Relationship validation and atomic replacement; sibling social records are passive data.

use crate::core::id::CharacterId;
use crate::core::state::AppState;
use crate::social::RelationshipDimensions;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RelationshipError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("a character cannot have a relationship edge to itself")]
    SelfRelationship,
}

pub struct ValidatedRelationship {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
}

impl ValidatedRelationship {
    pub fn commit(self, state: &mut AppState) {
        state
            .social
            .set_relationship(self.from, self.to, self.dimensions);
    }
}

pub fn validate_set_relationship(
    state: &AppState,
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
) -> Result<ValidatedRelationship, RelationshipError> {
    if from == to {
        return Err(RelationshipError::SelfRelationship);
    }
    let _ = state
        .world
        .get_character(from)
        .ok_or(RelationshipError::MissingCharacter(from))?;
    let _ = state
        .world
        .get_character(to)
        .ok_or(RelationshipError::MissingCharacter(to))?;
    Ok(ValidatedRelationship {
        from,
        to,
        dimensions,
    })
}
