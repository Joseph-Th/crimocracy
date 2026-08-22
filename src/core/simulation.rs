//! Deterministic top-level simulation tick and state-owned random decision helpers.

use crate::core::id::{
    BusinessCycleId, CharacterId, EnterpriseCycleId, InvestigationId, InvestigationWorkId,
    OperationId, OpportunityId, PoliceResponseId, RecruitmentAttemptId, ReportId,
};
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::decision_system::{validate_resolve_decision, DecisionRequestOutcome};
use crate::decisions::DecisionResponse;
use crate::economy::business_economy_system::{
    decide_business_cycle, find_due_businesses, validate_business_cycle_plan,
};
use crate::enterprises::enterprise_execution::{
    decide_enterprise_cycle, find_due_enterprises, validate_enterprise_cycle_plan,
};
use crate::legal::investigation_system::apply_autonomous_investigator_staffing;
use crate::legal::investigation_system::process_cold_case_decay;
use crate::legal::investigation_work_execution::{
    apply_initial_evidence_reviews, decide_investigation_work_resolution,
    due_scheduled_investigation_work, validate_investigation_work_resolution_plan,
    InvestigationWorkRandomness,
};
use crate::operations::operation_execution::{
    decide_operation_resolution, due_in_progress_operations, validate_operation_resolution_plan,
    OperationResolutionRandomness,
};
use crate::operations::operation_system::{
    apply_transition, due_authorized_operations, due_operations_with_missed_deadlines,
    has_missed_operation_deadline, validate_deadline_missed_operation, OperationTransition,
};
use crate::operations::police_response_integration::apply_due_police_response_arrivals;
use crate::opportunities::opportunity_system::expire_due_opportunities;
use crate::recruitment::recruitment_system::resolve_due_autonomous_recruitment;
use crate::registry::Registry;
use crate::reports::executive_brief::{
    decide_executive_brief, is_executive_brief_due, validate_executive_brief_plan,
};
use rand_core::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
    pub arrived_police_responses: Vec<PoliceResponseId>,
    pub decision_requests: Vec<DecisionRequestOutcome>,
    pub resolved_operations: Vec<OperationId>,
    pub staffed_investigations: Vec<(InvestigationId, CharacterId)>,
    pub scheduled_investigation_work: Vec<InvestigationWorkId>,
    pub scheduled_witness_interviews: Vec<InvestigationWorkId>,
    pub resolved_investigation_work: Vec<InvestigationWorkId>,
    pub business_cycles: Vec<BusinessCycleId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
    pub recruitment_attempts: Vec<RecruitmentAttemptId>,
    pub expired_opportunities: Vec<OpportunityId>,
    pub cold_case_suspensions: Vec<InvestigationId>,
    pub executive_brief: Option<ReportId>,
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    state.advance_clock(SimDuration::ONE_MINUTE);
    // Opportunity expiry runs before other due work so its durable lifecycle report is available
    // to every same-minute consumer, including the executive brief synthesized at the end.
    let expired_opportunities = expire_due_opportunities(registry, state);
    let due_authorized = due_authorized_operations(state);
    let mut started_operations = Vec::with_capacity(due_authorized.len());
    for operation in due_authorized {
        if has_missed_operation_deadline(state, operation) {
            validate_deadline_missed_operation(state, operation)
                .expect("a missed operation deadline must validate")
                .commit(state)
                .expect("a missed operation deadline must commit atomically");
        } else {
            apply_transition(registry, state, operation, OperationTransition::Begin)
                .expect("due authorized operation must support the begin transition");
            started_operations.push(operation);
        }
    }
    for operation in due_operations_with_missed_deadlines(state) {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("overdue operation must still exist");
        if let Some(decision) = state.decisions().pending_for_operation(operation) {
            let recipient = record.responsible_organization();
            validate_resolve_decision(
                registry,
                state,
                decision,
                recipient,
                DecisionResponse::Abort,
            )
            .expect("an overdue operation decision must support automatic abort")
            .commit(state)
            .expect("automatic deadline decision abort must commit atomically");
        } else {
            validate_deadline_missed_operation(state, operation)
                .expect("an overdue in-progress operation must validate a deadline abort")
                .commit(state)
                .expect("an overdue in-progress operation must abort atomically");
        }
    }
    let police_response_outcome = apply_due_police_response_arrivals(state)
        .expect("due police responses must commit through canonical arrival processing");
    let arrived_police_responses = police_response_outcome.arrived;
    let decision_requests = police_response_outcome.decisions;
    let due_operations = due_in_progress_operations(state);
    let mut resolved_operations = Vec::with_capacity(due_operations.len());
    for operation in due_operations {
        let kind = state
            .operations()
            .get_operation(operation)
            .expect("due operation must still exist")
            .kind();
        let execution = registry.get_operation(kind).execution();
        let execution_variance =
            draw_signed_variance(state.operation_rng_mut(), execution.variance_limit());
        let exposure_variance = draw_signed_variance(
            state.operation_rng_mut(),
            execution.exposure_variance_limit(),
        );
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
    let staffed_investigations = apply_autonomous_investigator_staffing(state)
        .expect("valid state should staff available investigators onto active cases");
    let scheduled_investigation_work =
        apply_initial_evidence_reviews(registry, state, &staffed_investigations)
            .expect("newly staffed investigations should schedule valid initial evidence work");
    // Witness interviews are scheduled after evidence reviews so a witness registered by an
    // operation resolving earlier in this same minute is interviewable as soon as its case
    // has an investigator.
    let witness_interviews =
        crate::legal::investigation_work_execution::schedule_due_witness_interviews(
            registry, state,
        )
        .expect("valid state should schedule due witness interviews");
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
        let variance = draw_signed_variance(state.investigation_rng_mut(), variance_limit);
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
    // Cold-case decay runs after detective work resolution so the case's last-activity instant is
    // final for the minute; an authored institutional-inactivity window then shelves operation-
    // originated cases whose owning authority has gone quiet. No random stream is consumed, so the
    // decay does not perturb any domain RNG sequence.
    let cold_case_suspensions = process_cold_case_decay(state, registry.legal().cold_case_window())
        .expect("valid state should resolve cold-case decay");
    let due_businesses = find_due_businesses(state);
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
        let variance = draw_basis_point_variance(state.business_rng_mut(), variance_limit);
        let plan = decide_business_cycle(registry, state, business, variance)
            .expect("due active business must resolve a valid cycle plan");
        let cycle = validate_business_cycle_plan(state, plan)
            .expect("fresh business cycle plan must validate")
            .commit(state)
            .expect("validated business cycle must commit atomically");
        business_cycles.push(cycle);
    }
    let due_enterprises = find_due_enterprises(state);
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
        let variance = draw_basis_point_variance(state.enterprise_rng_mut(), variance_limit);
        let plan = decide_enterprise_cycle(registry, state, enterprise, variance)
            .expect("due active enterprise must resolve a valid cycle plan");
        let cycle = validate_enterprise_cycle_plan(state, plan)
            .expect("fresh enterprise cycle plan must validate")
            .commit(state)
            .expect("validated enterprise cycle must commit atomically");
        enterprise_cycles.push(cycle);
    }
    let recruitment_attempts = resolve_due_autonomous_recruitment(registry, state)
        .expect("valid state should resolve due autonomous recruitment");
    // Executive synthesis runs last so a due brief sees every report and decision created by
    // operational, investigative, financial, and delegated personnel work that resolved in the
    // same simulation minute.
    let executive_brief = state.player_organization().and_then(|recipient| {
        is_executive_brief_due(registry, state.now()).then(|| {
            let plan = decide_executive_brief(registry, state, recipient)
                .expect("due player executive brief must produce a valid synthesis plan");
            validate_executive_brief_plan(state, plan)
                .expect("fresh executive brief plan must validate")
                .commit(state)
                .expect("validated executive brief must commit atomically")
        })
    });
    validate_invariants(state);
    TickOutcome {
        now: state.now(),
        started_operations,
        arrived_police_responses,
        decision_requests,
        resolved_operations,
        staffed_investigations,
        scheduled_investigation_work,
        scheduled_witness_interviews: witness_interviews,
        resolved_investigation_work,
        business_cycles,
        enterprise_cycles,
        recruitment_attempts,
        expired_opportunities,
        cold_case_suspensions,
        executive_brief,
    }
}

fn draw_signed_variance(rng: &mut impl RngCore, limit: u8) -> i8 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("signed variance choice range overflowed usize");
    let draw = draw_index(rng, width).expect("signed variance range is never empty");
    let signed =
        i16::try_from(draw).expect("operation variance draw must fit i16") - i16::from(limit);
    i8::try_from(signed).expect("authored signed variance limit must fit i8")
}

fn draw_basis_point_variance(rng: &mut impl RngCore, limit: u16) -> i16 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("enterprise variance choice range overflowed usize");
    let draw = draw_index(rng, width).expect("enterprise variance range is never empty");
    let signed =
        i32::try_from(draw).expect("enterprise variance draw must fit i32") - i32::from(limit);
    i16::try_from(signed).expect("authored enterprise variance limit must fit i16")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RandomDecisionError {
    #[error("cannot choose from an empty choice set")]
    EmptyChoiceSet,
}

fn draw_index(rng: &mut impl RngCore, choice_count: usize) -> Result<usize, RandomDecisionError> {
    if choice_count == 0 {
        return Err(RandomDecisionError::EmptyChoiceSet);
    }
    let bound = u64::try_from(choice_count).expect("usize choice count must fit into u64");
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = rng.next_u64();
        if draw < rejection_zone {
            return Ok((draw % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_random_streams_do_not_cross_contaminate_unrelated_simulation_work() {
        let mut baseline = AppState::new(0x1933_0814);
        let mut operation_heavy = baseline.clone();

        for _ in 0..64 {
            draw_signed_variance(operation_heavy.operation_rng_mut(), 12);
        }

        for _ in 0..32 {
            assert_eq!(
                draw_basis_point_variance(baseline.business_rng_mut(), 2_500),
                draw_basis_point_variance(operation_heavy.business_rng_mut(), 2_500)
            );
            assert_eq!(
                draw_basis_point_variance(baseline.enterprise_rng_mut(), 2_500),
                draw_basis_point_variance(operation_heavy.enterprise_rng_mut(), 2_500)
            );
            assert_eq!(
                draw_signed_variance(baseline.investigation_rng_mut(), 12),
                draw_signed_variance(operation_heavy.investigation_rng_mut(), 12)
            );
        }
    }
}
