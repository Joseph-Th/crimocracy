//! Shared simulation primitives and the top-level state/pipeline contracts.
//!
//! `state` owns `AppState`; `simulation` owns `run_tick`; `persistence` owns the save
//! envelope; `invariants` owns `validate_state`; `id`, `time`, `entity`, `attention`,
//! and `version` own their value types.

pub mod attention;
pub mod entity;
pub mod id;
pub mod invariants;
pub mod persistence;
pub mod simulation;
pub mod state;
pub mod time;
pub mod version;
