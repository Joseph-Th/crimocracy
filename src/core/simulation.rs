//! Deterministic top-level simulation tick and state-owned random decision helpers.

use crate::core::id::{BusinessCycleId, EnterpriseCycleId, InvestigationWorkId, OperationId};
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::economy::business_economy_system::{
    decide_business_cycle, due_active_businesses, validate_business_cycle_plan,
};
use crate::enterprises::enterprise_execution::{
    decide_enterprise_cycle, due_active_enterprises, validate_enterprise_cycle_plan,
};
use crate::legal::investigation_work_execution::{
    decide_investigation_work_resolution, due_scheduled_investigation_work,
    validate_investigation_work_resolution_plan, InvestigationWorkRandomness,
};
use crate::operations::operation_execution::{
    decide_operation_resolution, due_in_progress_operations, validate_operation_resolution_plan,
    OperationResolutionRandomness,
};
use crate::operations::operation_system::{
    apply_transition, due_authorized_operations, OperationTransition,
};
use crate::registry::Registry;
use rand_core::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
    pub resolved_operations: Vec<OperationId>,
    pub resolved_investigation_work: Vec<InvestigationWorkId>,
    pub business_cycles: Vec<BusinessCycleId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    state.advance_clock(SimDuration::ONE_MINUTE);
    let started_operations = due_authorized_operations(state);
    for operation in &started_operations {
        apply_transition(registry, state, *operation, OperationTransition::Begin)
            .expect("due authorized operation must support the begin transition");
    }
    let due_operations = due_in_progress_operations(state);
    let mut resolved_operations = Vec::with_capacity(due_operations.len());
    for operation in due_operations {
        let kind = state
            .operations()
            .get_operation(operation)
            .expect("due operation must still exist")
            .kind();
        let execution = registry.get_operation(kind).execution();
        let execution_variance = decide_signed_variance(state, execution.variance_limit());
        let exposure_variance = decide_signed_variance(state, execution.exposure_variance_limit());
        let plan = decide_operation_resolution(
            registry,
            state,
            operation,
            OperationResolutionRandomness::new(execution_variance, exposure_variance),
        )
        .expect("due in-progress operation must resolve a valid plan");
        let resolved = validate_operation_resolution_plan(registry, state, plan)
            .expect("fresh operation resolution plan must validate")
            .commit(state)
            .expect("validated operation resolution must commit atomically");
        resolved_operations.push(resolved);
    }
    // Detective work resolves after operation consequences so legal state created by an operation
    // is visible to later institutional work in the same minute without bypassing evidence ownership.
    let due_investigation_work = due_scheduled_investigation_work(state);
    let mut resolved_investigation_work = Vec::with_capacity(due_investigation_work.len());
    for work in due_investigation_work {
        let kind = state
            .legal()
            .get_investigation_work(work)
            .expect("due investigation work must still exist")
            .kind();
        let variance_limit = registry.get_investigation_work(kind).variance_limit();
        let variance = decide_signed_variance(state, variance_limit);
        let plan = decide_investigation_work_resolution(
            registry,
            state,
            work,
            InvestigationWorkRandomness::new(variance),
        )
        .expect("due investigation work must resolve a valid plan");
        let resolved = validate_investigation_work_resolution_plan(registry, state, plan)
            .expect("fresh investigation work resolution plan must validate")
            .commit(state)
            .expect("validated investigation work must commit atomically");
        resolved_investigation_work.push(resolved);
    }
    let due_businesses = due_active_businesses(state);
    let mut business_cycles = Vec::with_capacity(due_businesses.len());
    for business in due_businesses {
        let kind = state
            .world()
            .get_business(business)
            .expect("due business economy must reference an existing business")
            .kind();
        let variance_limit = registry
            .get_business(kind)
            .economics()
            .gross_variance_basis_points();
        let variance = decide_basis_point_variance(state, variance_limit);
        let plan = decide_business_cycle(registry, state, business, variance)
            .expect("due active business must resolve a valid cycle plan");
        let cycle = validate_business_cycle_plan(state, plan)
            .expect("fresh business cycle plan must validate")
            .commit(state)
            .expect("validated business cycle must commit atomically");
        business_cycles.push(cycle);
    }
    let due_enterprises = due_active_enterprises(state);
    let mut enterprise_cycles = Vec::with_capacity(due_enterprises.len());
    for enterprise in due_enterprises {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let variance_limit = registry
            .get_enterprise(kind)
            .economics()
            .gross_variance_basis_points();
        let variance = decide_basis_point_variance(state, variance_limit);
        let plan = decide_enterprise_cycle(registry, state, enterprise, variance)
            .expect("due active enterprise must resolve a valid cycle plan");
        let cycle = validate_enterprise_cycle_plan(state, plan)
            .expect("fresh enterprise cycle plan must validate")
            .commit(state)
            .expect("validated enterprise cycle must commit atomically");
        enterprise_cycles.push(cycle);
    }
    validate_invariants(state);
    TickOutcome {
        now: state.now(),
        started_operations,
        resolved_operations,
        resolved_investigation_work,
        business_cycles,
        enterprise_cycles,
    }
}

fn decide_signed_variance(state: &mut AppState, limit: u8) -> i8 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("signed variance choice range overflowed usize");
    let draw = decide_index(state, width).expect("signed variance range is never empty");
    let signed =
        i16::try_from(draw).expect("operation variance draw must fit i16") - i16::from(limit);
    i8::try_from(signed).expect("authored signed variance limit must fit i8")
}

fn decide_basis_point_variance(state: &mut AppState, limit: u16) -> i16 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("enterprise variance choice range overflowed usize");
    let draw = decide_index(state, width).expect("enterprise variance range is never empty");
    let signed =
        i32::try_from(draw).expect("enterprise variance draw must fit i32") - i32::from(limit);
    i16::try_from(signed).expect("authored enterprise variance limit must fit i16")
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
