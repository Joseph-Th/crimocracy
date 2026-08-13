//! Versioned persistence envelope; serialization adapters remain outside the simulation core.

use crate::core::invariants::{validate_invariants, validate_state, StateValidationError};
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_SAVE_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveEnvelope {
    format_version: u16,
    content_revision: u32,
    state: AppState,
}

impl SaveEnvelope {
    pub fn format_version(&self) -> u16 {
        self.format_version
    }
    pub fn content_revision(&self) -> u32 {
        self.content_revision
    }
    pub fn state(&self) -> &AppState {
        &self.state
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SaveError {
    #[error("cannot save invalid application state: {0}")]
    InvalidState(#[from] StateValidationError),
}

pub fn build_save(registry: &Registry, state: &AppState) -> Result<SaveEnvelope, SaveError> {
    validate_state(state)?;
    validate_invariants(state);
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
    validate_state(&envelope.state).map_err(LoadError::InvalidState)?;
    validate_invariants(&envelope.state);
    Ok(envelope.state)
}
