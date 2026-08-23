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
use crate::legal::investigation_system::apply_cold_case_decay;
use crate::legal::investigation_work_execution::{
    apply_initial_evidence_reviews, decide_investigation_work_resolution,
    find_due_scheduled_investigation_work, validate_investigation_work_resolution_plan,
    InvestigationWorkRandomness,
};
use crate::operations::operation_execution::{
    decide_operation_resolution, find_due_in_progress_operations,
    validate_operation_resolution_plan, OperationResolutionRandomness,
};
use crate::operations::operation_system::{
    apply_transition, find_due_authorized_operations, find_due_operations_with_missed_deadlines,
    has_missed_operation_deadline, validate_deadline_missed_operation, OperationTransition,
};
use crate::operations::police_response_integration::apply_due_police_response_arrivals;
use crate::opportunities::opportunity_system::apply_opportunity_expiry;
use crate::recruitment::recruitment_system::apply_due_autonomous_recruitment;
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
    pub evidence_arrests: Vec<crate::core::id::ArrestId>,
    pub informant_recruitments: Vec<crate::core::id::InformantId>,
    pub informant_disclosures: Vec<crate::core::id::InformantDisclosureId>,
    pub automatic_legal_support: Vec<crate::core::id::LegalRepresentationId>,
    pub business_cycles: Vec<BusinessCycleId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
    pub payrolls: Vec<crate::world::payroll_execution::PayrollOutcome>,
    pub recruitment_attempts: Vec<RecruitmentAttemptId>,
    pub autonomous_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub expired_opportunities: Vec<OpportunityId>,
    pub cold_case_suspensions: Vec<InvestigationId>,
    pub cold_case_closures: Vec<InvestigationId>,
    pub executive_brief: Option<ReportId>,
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    state.advance_clock(SimDuration::ONE_MINUTE);
    // Opportunity expiry runs before other due work so its durable lifecycle report is available
    // to every same-minute consumer, including the executive brief synthesized at the end.
    let expired_opportunities = apply_opportunity_expiry(registry, state);
    let due_authorized = find_due_authorized_operations(state);
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
    for operation in find_due_operations_with_missed_deadlines(state) {
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
    let due_operations = find_due_in_progress_operations(state);
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
        crate::legal::investigation_work_execution::apply_due_witness_interview_scheduling(
            registry, state,
        )
        .expect("valid state should schedule due witness interviews");
    // Detective work resolves after operation consequences so legal state created by an operation
    // is visible to later institutional work in the same minute without bypassing evidence ownership.
    let due_investigation_work = find_due_scheduled_investigation_work(state);
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
    // The police institution converts accumulated case evidence into custody after detective
    // work resolves, so an interview or forensic analysis finishing this minute is visible to
    // the same minute's arrest decision.
    let evidence_arrests = crate::legal::arrest_system::apply_autonomous_evidence_arrests(state)
        .expect("valid state should convert qualifying case evidence into custody");
    // Detainee informant recruitment runs right after custody conversion: a member arrested
    // exactly one cadence window ago faces their single recruitment decision this minute, and
    // active informants disclose personally-held knowledge into their handler's cases.
    let informant_recruitments =
        crate::legal::informant_system::apply_detainee_informant_recruitment(state)
            .expect("valid state should resolve detainee informant recruitment decisions");
    let informant_disclosures = crate::legal::informant_system::apply_informant_disclosures(state)
        .expect("valid state should record due informant disclosures");
    // Automatic legal-support governance runs last in the custody cluster: it sees every
    // arrest made this minute and retains counsel through the canonical representation path
    // when the organization's standing policy promises it.
    let automatic_legal_support =
        crate::legal::legal_representation_system::apply_automatic_legal_support(state)
            .expect("valid state should resolve automatic legal-support retention");
    // Cold-case decay runs after detective work resolution so the case's last-activity instant is
    // final for the minute; an authored institutional-inactivity window then shelves operation-
    // originated cases whose owning authority has gone quiet. No random stream is consumed, so the
    // decay does not perturb any domain RNG sequence.
    let cold_case_decay = apply_cold_case_decay(state, registry.legal().cold_case_window())
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
    // Payroll runs after the day's enterprise and business cycles so earned revenue can fund
    // the same day's wages, and before autonomous recruitment so an unpaid crew's resentment is
    // already in place when a rival pitches them.
    let payrolls = crate::world::payroll_execution::apply_daily_payroll(registry, state);
    let recruitment_attempts = apply_due_autonomous_recruitment(registry, state)
        .expect("valid state should resolve due autonomous recruitment");
    // Delegated rival expansion runs after recruitment so a mandate whose crew changed this
    // minute governs with its current roster. Selection consumes no randomness, so matched
    // branches observe identical rival growth unless their own actions touched rival state.
    let autonomous_enterprises =
        crate::enterprises::enterprise_execution::apply_due_autonomous_enterprises(registry, state);
    // Reputation consequences run right after the day's operational work so an audience's
    // impression moves in the same tick that produced it, and before the daily decay pass so
    // fresh adjustments are never immediately eroded. The player organization additionally
    // receives a Standing report per shifting operation — its own street standing is
    // legitimate self-knowledge, surfaced through the canonical report path.
    let player_organization = state.player_organization();
    for operation in &resolved_operations {
        let (organization, approach, objective_outcome, exposure_level) = {
            let record = state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must exist for reputation consequences");
            let resolution = record
                .resolution()
                .expect("resolved operation carries its resolution");
            (
                record.responsible_organization(),
                record.approach(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            )
        };
        let shifts = crate::reputation::reputation_system::apply_operation_reputation_consequences(
            registry,
            state,
            organization,
            approach,
            objective_outcome,
            exposure_level,
        )
        .expect("valid state should apply operation reputation consequences");
        if Some(organization) == player_organization {
            crate::reputation::reputation_system::apply_standing_feedback(
                state,
                organization,
                &shifts,
            )
            .expect("player standing feedback must record through the canonical report path");
        }
    }
    crate::reputation::reputation_system::apply_daily_reputation_decay(registry, state);
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
        evidence_arrests,
        informant_recruitments,
        informant_disclosures,
        automatic_legal_support,
        business_cycles,
        enterprise_cycles,
        payrolls,
        recruitment_attempts,
        autonomous_enterprises,
        expired_opportunities,
        cold_case_suspensions: cold_case_decay.suspended,
        cold_case_closures: cold_case_decay.closed,
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
pub(crate) enum RandomDecisionError {
    #[error("cannot choose from an empty choice set")]
    EmptyChoiceSet,
}

pub(crate) fn draw_index(
    rng: &mut impl RngCore,
    choice_count: usize,
) -> Result<usize, RandomDecisionError> {
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
