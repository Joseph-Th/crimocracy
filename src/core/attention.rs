//! Player-attention classification and persistent auto-pause preferences used across subsystems.

use crate::core::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AttentionClass {
    Routine,
    Notable,
    Exception,
    Crisis,
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

pub fn set_auto_pause(state: &mut AppState, attention: AttentionClass, enabled: bool) {
    state.set_attention_auto_pause(attention, enabled);
}
