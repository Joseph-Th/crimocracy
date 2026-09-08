//! Core simulation library for Crimocracy — the agent's module map.
//!
//! Tower of abstractions (see `ARCHITECTURE.md` for the authoritative diagram and source map):
//! ```text
//! Layer 4  operations · legal · enterprises · economy
//! Layer 3  finance · delegation · reputation · decisions · contacts · opportunities · recruitment
//! Layer 2  world · social · intelligence · history · reports
//! Layer 1  registry ◄── content::build_registry
//! Layer 0  core::{id,time,entity,attention,state,simulation,persistence,invariants}
//! ```
//! Start at `core::state::AppState` and the ordered
//! `core::simulation::run_tick` pipeline.
//! Every `src/*/mod.rs` //! header names its canonical mutation path — treat it as contract.

pub mod contacts;
pub mod content;
pub mod core;
pub mod decisions;
pub mod delegation;
pub mod economy;
pub mod enterprises;
pub mod finance;
pub mod history;
pub mod intelligence;
pub mod legal;
pub mod operations;
pub mod opportunities;
pub mod recruitment;
pub mod registry;
pub mod reports;
pub mod reputation;
pub mod social;
pub mod world;

pub use content::build_registry;
pub use core::state::AppState;
pub use registry::Registry;
