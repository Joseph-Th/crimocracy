//! Core simulation library for Crimocracy.

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
pub mod social;
pub mod world;

pub use content::build_registry;
pub use core::state::AppState;
pub use registry::Registry;
