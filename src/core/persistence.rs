//! Versioned persistence envelope; serialization adapters remain outside the simulation core.

use crate::core::invariants::{
    StateValidationError, validate_state, validate_state_against_registry,
};
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_SAVE_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEnvelope {
    format_version: u16,
    content_revision: u32,
    state: AppState,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SaveError {
    #[error("cannot save invalid application state: {0}")]
    InvalidState(#[from] StateValidationError),
}

pub fn build_save(registry: &Registry, state: &AppState) -> Result<SaveEnvelope, SaveError> {
    validate_state(state)?;
    validate_state_against_registry(registry, state)?;
    Ok(SaveEnvelope {
        format_version: CURRENT_SAVE_FORMAT_VERSION,
        content_revision: registry.content_revision(),
        state: state.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("unsupported save format version {found}; expected {expected}")]
    UnsupportedFormat { found: u16, expected: u16 },
    #[error("unsupported state schema version {found}; expected {expected}")]
    UnsupportedStateSchema { found: u16, expected: u16 },
    #[error("save content revision {found} does not match loaded registry revision {expected}")]
    ContentRevisionMismatch { found: u32, expected: u32 },
    #[error("save authoritative records cannot rebuild deterministic derived indexes")]
    InvalidDerivedIndexRebuild,
    #[error("save contains invalid application state: {0}")]
    InvalidState(#[source] StateValidationError),
}

pub fn restore_save(registry: &Registry, envelope: SaveEnvelope) -> Result<AppState, LoadError> {
    if envelope.format_version != CURRENT_SAVE_FORMAT_VERSION {
        return Err(LoadError::UnsupportedFormat {
            found: envelope.format_version,
            expected: CURRENT_SAVE_FORMAT_VERSION,
        });
    }
    if envelope.state.state_schema_version() != CURRENT_STATE_SCHEMA_VERSION {
        return Err(LoadError::UnsupportedStateSchema {
            found: envelope.state.state_schema_version(),
            expected: CURRENT_STATE_SCHEMA_VERSION,
        });
    }
    if envelope.content_revision != registry.content_revision() {
        return Err(LoadError::ContentRevisionMismatch {
            found: envelope.content_revision,
            expected: registry.content_revision(),
        });
    }
    let mut state = envelope.state;
    if !state.rebuild_derived_indexes_after_restore() {
        return Err(LoadError::InvalidDerivedIndexRebuild);
    }
    validate_state(&state).map_err(LoadError::InvalidState)?;
    validate_state_against_registry(registry, &state).map_err(LoadError::InvalidState)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn save_bytes_omit_derived_indexes_and_restore_rebuilds_them() {
        let registry = build_registry();
        let mut state = AppState::new(0x5A9E_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Persistence Index Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let character = insert_character(
            &mut state,
            CharacterDraft {
                name: "Persistence Index Test Character".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("character fixture should validate");
        assert_eq!(
            state
                .world()
                .characters_in_organization(organization)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![character]
        );

        let envelope = build_save(&registry, &state).expect("valid state should save");
        let bytes = bincode::serialize(&envelope).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should decode");
        assert_eq!(
            decoded
                .state
                .world()
                .get_character(character)
                .map(|record| record.id()),
            Some(character),
            "authoritative records must persist"
        );
        assert_eq!(
            decoded
                .state
                .world()
                .characters_in_organization(organization)
                .count(),
            0,
            "derived organization membership index must not be serialized"
        );

        let restored = restore_save(&registry, decoded).expect("restore should rebuild indexes");
        assert_eq!(
            restored
                .world()
                .characters_in_organization(organization)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![character]
        );
    }
}
