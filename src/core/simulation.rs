//! Deterministic top-level simulation tick and state-owned random decision helpers.

use crate::core::id::OperationId;
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::operations::operation_system::{
    apply_transition, due_authorized_operations, OperationTransition,
};
use rand_core::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
}

pub fn run_tick(state: &mut AppState) -> TickOutcome {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    state.advance_clock(SimDuration::ONE_MINUTE);
    let started_operations = due_authorized_operations(state);
    for operation in &started_operations {
        apply_transition(state, *operation, OperationTransition::Begin)
            .expect("due authorized operation must support the begin transition");
    }
    validate_invariants(state);
    TickOutcome {
        now: state.now(),
        started_operations,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RandomDecisionError {
    #[error("cannot choose from an empty choice set")]
    EmptyChoiceSet,
}

pub fn decide_index(
    state: &mut AppState,
    choice_count: usize,
) -> Result<usize, RandomDecisionError> {
    if choice_count == 0 {
        return Err(RandomDecisionError::EmptyChoiceSet);
    }
    let bound = u64::try_from(choice_count).expect("usize choice count must fit into u64");
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = state.rng_mut().next_u64();
        if draw < rejection_zone {
            return Ok((draw % bound) as usize);
        }
    }
}
