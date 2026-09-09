//! Relationship validation and atomic replacement; sibling social records are passive data.

use crate::core::id::CharacterId;
use crate::core::state::AppState;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::social::RelationshipDimensions;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RelationshipError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("a character cannot have a relationship edge to itself")]
    SelfRelationship,
    #[error("relationship from {from} to {to} already has the requested dimensions")]
    RelationshipUnchanged { from: CharacterId, to: CharacterId },
    #[error(
        "relationship from {from} to {to} changed after validation; expected version {expected:?}, found {found:?}"
    )]
    StaleRelationship {
        from: CharacterId,
        to: CharacterId,
        expected: Option<u32>,
        found: Option<u32>,
    },
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

pub struct ValidatedRelationship {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
    expected_version: Option<u32>,
}

impl ValidatedRelationship {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), RelationshipError> {
        let found = state
            .social
            .get_relationship(self.from, self.to)
            .map(|record| record.version());
        if found != self.expected_version {
            return Err(RelationshipError::StaleRelationship {
                from: self.from,
                to: self.to,
                expected: self.expected_version,
                found,
            });
        }
        if let Some(version) = found {
            ensure_version_can_advance(version, "relationship")?;
        }
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), RelationshipError> {
        self.ensure_current(state)?;
        self.commit_preflighted(state);
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
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
    let expected_version = state
        .social
        .get_relationship(from, to)
        .map(|record| record.version());
    if state
        .social
        .get_relationship(from, to)
        .is_some_and(|record| record.dimensions() == dimensions)
    {
        return Err(RelationshipError::RelationshipUnchanged { from, to });
    }
    if let Some(version) = expected_version {
        ensure_version_can_advance(version, "relationship")?;
    }
    Ok(ValidatedRelationship {
        from,
        to,
        dimensions,
        expected_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::RelationshipLevel;
    use crate::world::world_system::insert_character;
    use crate::world::{AutonomyLevel, CharacterDraft};
    use std::collections::{BTreeMap, BTreeSet};

    fn character(state: &mut AppState, name: &str) -> CharacterId {
        insert_character(
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("relationship test character should validate")
    }

    fn dimensions(trust: u8) -> RelationshipDimensions {
        let mut dimensions = RelationshipDimensions::zero();
        dimensions.trust = RelationshipLevel::try_new(trust).expect("test trust should be valid");
        dimensions
    }

    #[test]
    fn concurrent_first_relationship_tokens_cannot_overwrite_newer_state() {
        let mut state = AppState::new(0x50C1_A11A);
        let from = character(&mut state, "Relationship Source");
        let to = character(&mut state, "Relationship Target");
        let stale = validate_set_relationship(&state, from, to, dimensions(20))
            .expect("first relationship token should validate");
        let winner = validate_set_relationship(&state, from, to, dimensions(80))
            .expect("concurrent relationship token should validate");

        winner
            .commit(&mut state)
            .expect("winning relationship token should commit");
        assert_eq!(
            stale
                .commit(&mut state)
                .expect_err("older absent-record token must stale after another commit"),
            RelationshipError::StaleRelationship {
                from,
                to,
                expected: None,
                found: Some(1),
            }
        );
        let record = state
            .social()
            .get_relationship(from, to)
            .expect("winning relationship should persist");
        assert_eq!(record.dimensions(), dimensions(80));
        assert_eq!(record.version(), 1);
    }

    #[test]
    fn identical_relationship_write_is_rejected_without_version_churn() {
        let mut state = AppState::new(0x50C1_A11B);
        let from = character(&mut state, "Relationship Source");
        let to = character(&mut state, "Relationship Target");
        let value = dimensions(60);
        validate_set_relationship(&state, from, to, value)
            .expect("initial relationship should validate")
            .commit(&mut state)
            .expect("initial relationship should commit");
        let before = bincode::serialize(&state).expect("fixture state should serialize");

        let error = match validate_set_relationship(&state, from, to, value) {
            Ok(_) => panic!("identical relationship replacement must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, RelationshipError::RelationshipUnchanged { from, to });
        assert_eq!(
            bincode::serialize(&state).expect("rejected state should serialize"),
            before,
            "unchanged relationship must not invalidate dependent snapshots"
        );
    }
}
