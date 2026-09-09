//! Player-attention classification and persistent auto-pause preferences used across subsystems.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AttentionClass {
    Routine,
    Notable,
    Exception,
    Crisis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AttentionSettingsError {
    #[error("auto-pause can only be configured for Exception or Crisis attention")]
    UnsupportedAutoPauseClass,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttentionSettings {
    pub(crate) auto_pause: BTreeSet<AttentionClass>,
}

impl AttentionSettings {
    pub fn is_auto_pause_enabled(&self, attention: AttentionClass) -> bool {
        self.auto_pause.contains(&attention)
    }
}

impl Default for AttentionSettings {
    fn default() -> Self {
        Self {
            auto_pause: BTreeSet::from([AttentionClass::Exception, AttentionClass::Crisis]),
        }
    }
}
