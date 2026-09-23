//! Deterministic top-level simulation tick and state-owned random decision helpers.
//!
//! `run_tick` is the only authoritative minute (contractual phase order).
//! See `ARCHITECTURE.md` for the authoritative phase diagram and dependency tower.
//! New autonomous work must slot explicitly here with a "runs after X so Y" comment.

use crate::core::id::{
    BusinessCycleId, CharacterId, EnterpriseCycleId, IdExhaustionError, IdKind, InvestigationId,
    InvestigationWorkId, OperationId, OpportunityId, PoliceResponseId, ProsecutionCaseId,
    RecruitmentAttemptId, ReportId,
};
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::DecisionResponse;
use crate::decisions::decision_system::{
    DecisionError, DecisionRequestOutcome, ValidatedDecisionResolution, validate_resolve_decision,
};
use crate::economy::business_economy_system::{
    decide_business_cycle, find_due_businesses, validate_business_cycle_plan,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseCycleRandomness, EnterpriseError, decide_enterprise_cycle, find_due_enterprises,
    validate_enterprise_cycle_plan,
};
use crate::legal::investigation_system::apply_autonomous_investigator_staffing;
use crate::legal::investigation_system::apply_cold_case_decay;
use crate::legal::investigation_work_execution::{
    InvestigationWorkError, InvestigationWorkRandomness, InvestigationWorkSchedulingOutcome,
    apply_investigation_work_scheduling, decide_investigation_work_resolution,
    find_due_scheduled_investigation_work, validate_investigation_work_resolution_plan,
};
use crate::operations::operation_abort::{
    ValidatedOperationAbort, validate_deadline_missed_operation,
    validate_expired_opportunity_operation, validate_objective_unavailable_operation,
};
use crate::operations::operation_execution::{
    OperationResolutionError, OperationResolutionRandomness, decide_operation_resolution,
    find_due_in_progress_operations, mandatory_operation_resolution_id_budget,
    validate_operation_resolution_plan,
};
use crate::operations::operation_scheduling::{
    find_due_authorized_operations, find_due_operations_with_missed_deadlines,
    has_missed_operation_deadline,
};
use crate::operations::operation_system::{OperationError, OperationTransition, apply_transition};
use crate::operations::police_response_integration::apply_due_police_response_arrivals;
use crate::opportunities::opportunity_system::apply_opportunity_expiry;
use crate::recruitment::autonomous_recruitment::apply_due_autonomous_recruitment;
use crate::registry::Registry;
use crate::reports::executive_brief::{
    decide_executive_brief, is_executive_brief_due, validate_executive_brief_plan,
};
use crate::reputation::reputation_system::OperationReputationEvent;
use rand_core::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
    pub arrived_police_responses: Vec<PoliceResponseId>,
    /// Every decision request raised this tick, regardless of owning subsystem. The outcome
    /// preserves whether this player-owned attention event requests an adapter pause.
    pub decision_requests: Vec<DecisionRequestOutcome>,
    /// Operations aborted during the operation phase itself. Custody-driven aborts remain
    /// represented by the arrest that caused them later in the tick.
    pub aborted_operations: Vec<OperationId>,
    pub resolved_operations: Vec<OperationId>,
    pub staffed_investigations: Vec<(InvestigationId, CharacterId)>,
    pub scheduled_investigation_work: Vec<InvestigationWorkId>,
    pub scheduled_witness_interviews: Vec<InvestigationWorkId>,
    pub resolved_investigation_work: Vec<InvestigationWorkId>,
    pub evidence_arrests: Vec<crate::core::id::ArrestId>,
    pub staffed_prosecution_cases: Vec<(ProsecutionCaseId, CharacterId)>,
    pub informant_recruitments: Vec<crate::core::id::InformantId>,
    pub informant_disclosures: Vec<crate::core::id::InformantDisclosureId>,
    pub custody_releases: Vec<crate::core::id::ArrestId>,
    pub automatic_legal_support: Vec<crate::core::id::LegalRepresentationId>,
    /// Automatic-policy retainers ended because their represented matter is no longer active.
    /// The legal-support owner reports this separately from newly retained counsel so a
    /// conclusion-only tick remains visible to adapters and validation without rescanning state.
    pub concluded_automatic_legal_support: usize,
    pub business_cycles: Vec<BusinessCycleId>,
    pub resumed_businesses: Vec<crate::core::id::BusinessId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
    pub payrolls: Vec<crate::world::payroll_execution::PayrollOutcome>,
    /// Number of individual reputation dimensions that actually moved this tick, including
    /// day-boundary decay and current operation/racket consequences. This makes reputation-only
    /// persistent ticks observable to validation/adapters instead of hiding them behind phases
    /// that happen to have no other surfaced outcome.
    pub reputation_changes: usize,
    pub recruitment_attempts: Vec<RecruitmentAttemptId>,
    pub resumed_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub retired_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub autonomous_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub expired_opportunities: Vec<OpportunityId>,
    pub cold_case_suspensions: Vec<InvestigationId>,
    pub executive_brief: Option<ReportId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TickError {
    #[error("simulation clock is exhausted at minute {now:?}; no later canonical tick exists")]
    ClockExhausted { now: SimTime },
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> Result<TickOutcome, TickError> {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    // The state owner performs the checked mutation itself. A terminal campaign state is valid,
    // but it has no successor minute; rejecting here keeps the whole tick atomic and leaves no
    // unchecked production clock mutator for another caller to reuse accidentally.
    let previous_now = state.now();
    state
        .try_advance_clock(SimDuration::ONE_MINUTE)
        .ok_or(TickError::ClockExhausted { now: previous_now })?;
    // The custody cap is a hard lifecycle boundary. Release due detainees before any same-minute
    // work so an expired detention cannot block a participant, remain an extraction target, or
    // otherwise influence systems after its authored end instant. The informant decision delay is
    // required by registry validation to be strictly shorter than maximum detention, so canonical
    // minute-by-minute ticks always give that detainee decision its intended earlier window.
    let custody_releases = crate::legal::arrest_system::apply_due_custody_releases(
        state,
        registry.legal().maximum_detention(),
    )
    .expect("valid state should release custody that reached the authored maximum");
    // Phase order is the contract: bounded custody release first; opportunity expiry next so its
    // durable lifecycle report is available to every remaining same-minute consumer; then
    // operations (police arrivals, starts, overdue cleanup, resolution), legal institutional work
    // (staffing, detective work, new custody, representation, informants, cold decay), economy
    // cycles plus legitimate-business recovery, then the day-boundary governance cluster: payroll,
    // reputation (decay before current consequences), recruitment, delegated expansion (which
    // consumes current police fear), and executive synthesis last so the due brief sees everything
    // above.
    let expired_opportunities = apply_opportunity_expiry(registry, state)
        .expect("valid state should expire every due opportunity atomically");
    let OperationsPhaseOutcome {
        started: started_operations,
        arrived_police_responses,
        mut decision_requests,
        aborted: aborted_operations,
        resolved: resolved_operations,
    } = run_operations_phase(registry, state);
    let staffed_investigations = apply_autonomous_investigator_staffing(state)
        .expect("valid state should staff available investigators onto active cases");
    // Autonomous casework traverses the active-case index once per minute. Reviewable evidence
    // keeps first claim on a free detective; when no review is due, the same pass may schedule a
    // witness interview. Later evidence and witnesses therefore remain actionable without two
    // duplicate active-case scans every canonical minute.
    let InvestigationWorkSchedulingOutcome {
        evidence_reviews: scheduled_investigation_work,
        witness_interviews: scheduled_witness_interviews,
    } = apply_investigation_work_scheduling(registry, state)
        .expect("valid state should schedule due investigation work");
    let resolved_investigation_work = run_investigation_work_phase(registry, state)
        .expect("valid due investigation-work cohort must resolve atomically");
    // The police institution converts accumulated case evidence into custody after detective
    // work resolves, so an interview or forensic analysis finishing this minute is visible to
    // the same minute's arrest decision.
    let evidence_arrests =
        crate::legal::arrest_system::apply_autonomous_evidence_arrests(registry, state)
            .expect("valid state should convert qualifying case evidence into custody");
    // Custody can remove a prosecutor from every review they were carrying. Restaff immediately
    // after arrests so one individual's detention cannot freeze unrelated prosecution matters.
    let staffed_prosecution_cases =
        crate::legal::prosecution_system::apply_autonomous_prosecution_staffing(state)
            .expect("valid state should staff available prosecutors onto open reviews");
    // Legal-support governance runs before the detainee's one-time informant decision. This lets
    // promised counsel materially affect custodial cooperation risk instead of retaining counsel
    // only after the irreversible decision. It also sees every new arrest created above and
    // concludes automatic retainers for matters already ended.
    let automatic_legal_support_outcome =
        crate::legal::legal_representation_system::apply_automatic_legal_support(registry, state)
            .expect("valid state should resolve automatic legal-support retention");
    let automatic_legal_support = automatic_legal_support_outcome.retained;
    let concluded_automatic_legal_support = automatic_legal_support_outcome.concluded;
    // A member arrested exactly one cadence window ago now faces the decision with current
    // representation visible. New informants then disclose personally-held knowledge.
    let informant_recruitments =
        crate::legal::informant_system::apply_detainee_informant_recruitment(registry, state)
            .expect("valid state should resolve detainee informant recruitment decisions");
    let informant_disclosures = crate::legal::informant_system::apply_informant_disclosures(state)
        .expect("valid state should record due informant disclosures");
    // Cold-case decay runs after detective work resolution so the case's last-activity instant is
    // final for the minute; an authored institutional-inactivity window then shelves operation-
    // originated cases whose owning authority has gone quiet. No random stream is consumed, so the
    // decay does not perturb any domain RNG sequence.
    let cold_case_decay = apply_cold_case_decay(state, registry.legal().cold_case_window())
        .expect("valid state should resolve cold-case decay");
    let business_cycles = run_business_cycle_phase(registry, state)
        .expect("valid state should settle the complete due business-cycle cohort");
    // Legitimate-business recovery follows today's settlements so a chronic-loss suspension has
    // already taken effect. The lifecycle pass skips a business suspended at this exact instant,
    // and only non-player owners with positive current zero-variance economics resume through the
    // canonical economy lifecycle token.
    let resumed_businesses =
        crate::economy::business_economy_system::apply_due_autonomous_business_lifecycle(
            registry, state,
        )
        .expect("valid state should maintain suspended non-player business economies");
    let enterprise_cycles = run_enterprise_cycle_phase(registry, state)
        .expect("valid due enterprise cycles must settle through the preflighted sequential phase");
    // Payroll runs after the day's enterprise and business cycles so earned revenue can fund
    // the same day's wages. Reputation then settles the day boundary before recruitment: daily
    // decay advances only impressions old enough to fade, while operation/racket consequences from
    // this minute land before candidates judge an outfit's underworld competence. Together with
    // payroll, every authored recruitment input therefore reflects the current minute rather
    // than a mixture of pre- and post-boundary state.
    let payrolls = crate::world::payroll_execution::apply_daily_payroll(registry, state)
        .expect("valid state should settle every due criminal-organization payroll");
    let reputation_changes =
        apply_reputation_phase(registry, state, &resolved_operations, &enterprise_cycles)
            .expect("valid state should apply the preflighted reputation consequence cohort");
    let recruitment = apply_due_autonomous_recruitment(registry, state)
        .expect("valid state should resolve every due autonomous recruitment action");
    let recruitment_attempts = recruitment.attempts;
    decision_requests.extend(recruitment.approval_requests);
    // Suspended rival rackets are reconsidered before new growth. This gives chronic-loss
    // suspension a recoverable lifecycle without allowing an organization to spend one cash pool
    // on both a reopened racket and a fresh establishment. A racket suspended by a cycle earlier
    // in this same minute remains suspended until a later daily boundary.
    let enterprise_lifecycle =
        crate::enterprises::autonomous_lifecycle::apply_due_autonomous_enterprise_lifecycle(
            registry, state,
        )
        .expect("valid state should maintain suspended autonomous enterprises");
    let resumed_enterprises = enterprise_lifecycle.resumed;
    let retired_enterprises = enterprise_lifecycle.retired;
    // Delegated rival expansion runs after lifecycle maintenance and recruitment so a mandate
    // whose crew changed this minute governs with its current roster, and after reputation so a
    // racket hit this minute can make the organization keep its head down immediately. Selection
    // consumes no randomness, so matched branches observe identical rival growth unless their own
    // actions touched rival state.
    let autonomous_enterprises =
        crate::enterprises::autonomous_expansion::apply_due_autonomous_enterprises_excluding(
            registry,
            state,
            &enterprise_lifecycle.resumed_mandates,
        )
        .expect("valid state should resolve every due autonomous enterprise expansion");
    // Executive synthesis runs last so a due brief sees every report and decision created by
    // operational, investigative, financial, and delegated personnel work that resolved in the
    // same simulation minute.
    let executive_brief = synthesize_executive_brief(registry, state);
    validate_invariants(state);
    Ok(TickOutcome {
        now: state.now(),
        started_operations,
        arrived_police_responses,
        decision_requests,
        aborted_operations,
        resolved_operations,
        staffed_investigations,
        scheduled_investigation_work,
        scheduled_witness_interviews,
        resolved_investigation_work,
        evidence_arrests,
        staffed_prosecution_cases,
        informant_recruitments,
        informant_disclosures,
        custody_releases,
        automatic_legal_support,
        concluded_automatic_legal_support,
        business_cycles,
        resumed_businesses,
        enterprise_cycles,
        payrolls,
        reputation_changes,
        recruitment_attempts,
        resumed_enterprises,
        retired_enterprises,
        autonomous_enterprises,
        expired_opportunities,
        cold_case_suspensions: cold_case_decay,
        executive_brief,
    })
}

#[cfg(test)]
pub(crate) fn run_test_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    run_tick(registry, state).expect("test fixture must leave room for another simulation minute")
}

/// Processes due police-response arrivals, starts due authorized operations, aborts missed
/// deadlines (through the pending decision when one exists), and resolves due in-progress
/// operations with pre-drawn deterministic variance.
struct OperationsPhaseOutcome {
    started: Vec<OperationId>,
    arrived_police_responses: Vec<PoliceResponseId>,
    decision_requests: Vec<DecisionRequestOutcome>,
    aborted: Vec<OperationId>,
    resolved: Vec<OperationId>,
}

fn run_operations_phase(registry: &Registry, state: &mut AppState) -> OperationsPhaseOutcome {
    // Process responses that were dispatched on earlier ticks before admitting new work.
    // Authorization deliberately allows exact back-to-back participant windows. A response
    // arriving on that boundary can turn the earlier operation into an unresolved commitment;
    // that state must be visible to begin-time participant validation before the follow-up starts.
    // Newly started operations cannot add another due arrival here because registry validation
    // requires every police-response delay to be strictly positive.
    let police_response_outcome = apply_due_police_response_arrivals(state)
        .expect("due police responses must commit through canonical arrival processing");
    let arrived_police_responses = police_response_outcome.arrived;
    let decision_requests = police_response_outcome.decisions;
    let mut aborted_operations = police_response_outcome.aborted_operations;

    let due_authorized = find_due_authorized_operations(state);
    let prestart_aborts = prepare_authorized_prestart_aborts(registry, state, &due_authorized)
        .expect("valid due authorized abort cohort must preflight atomically");
    let mut started_operations = Vec::with_capacity(due_authorized.len());
    for (operation, prestart_abort) in due_authorized
        .into_iter()
        .zip(prestart_aborts.unwrap_or_default())
    {
        if let Some(abort) = prestart_abort {
            (*abort).commit_preflighted(state);
            aborted_operations.push(operation);
        } else {
            match apply_transition(registry, state, operation, OperationTransition::Begin) {
                Ok(()) => started_operations.push(operation),
                // A future assignment can become temporarily unavailable when an earlier
                // operation remains paused longer than projected at authorization time. The due
                // operation stays Authorized and retries on later ticks. Completion deadlines
                // and linked opportunity windows are handled by the pre-checks above, so temporary
                // unavailability cannot silently carry work beyond an authored viability boundary.
                Err(OperationError::ParticipantBusy { .. })
                // Direct begin correctly reports that no complete execution window remains.
                // The autonomous tick has no useful mutation to make in that terminal state:
                // retain the authorized plan and let the finite clock reach its canonical end
                // instead of treating exhaustion of future time as an impossible-state panic.
                | Err(OperationError::SimulationTimeOverflow) => {}
                Err(error) if operation_begin_is_terminally_blocked(&error) => {}
                Err(error) => {
                    panic!(
                        "due authorized operation could not begin through its canonical path: {error}"
                    )
                }
            }
        }
    }
    // Preserve the established retry boundary: a follow-up that was blocked at begin time by a
    // still-paused operation does not immediately retry merely because overdue cleanup releases
    // the participant later in this same phase. It remains Authorized until the next tick.
    aborted_operations.extend(
        apply_overdue_operation_cleanup(registry, state)
            .expect("valid overdue operations must abort as one artifact-preflighted cohort"),
    );
    let resolved_operations = run_operation_resolution_phase(registry, state).expect(
        "valid due operation resolutions must commit through the preflighted sequential phase",
    );
    OperationsPhaseOutcome {
        started: started_operations,
        arrived_police_responses,
        decision_requests,
        aborted: aborted_operations,
        resolved: resolved_operations,
    }
}

#[derive(Debug, Error)]
enum OperationPhaseBatchError {
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

fn operation_error_is_terminally_blocked(error: &OperationError) -> bool {
    use crate::legal::police_response_system::PoliceResponseError;

    matches!(
        error,
        OperationError::IdExhaustion(_)
            | OperationError::VersionCapacity(_)
            | OperationError::PoliceResponseDispatch(
                PoliceResponseError::IdExhaustion(_) | PoliceResponseError::VersionCapacity(_)
            )
    )
}

fn operation_begin_is_terminally_blocked(error: &OperationError) -> bool {
    operation_error_is_terminally_blocked(error)
}

fn operation_phase_batch_is_terminally_blocked(error: &OperationPhaseBatchError) -> bool {
    match error {
        OperationPhaseBatchError::IdExhaustion(_) => true,
        OperationPhaseBatchError::Operation(error) => operation_error_is_terminally_blocked(error),
        OperationPhaseBatchError::Decision(error) => match error {
            DecisionError::IdExhaustion(_) | DecisionError::VersionCapacity(_) => true,
            DecisionError::Operation(error) => operation_error_is_terminally_blocked(error),
            DecisionError::EmptySummary
            | DecisionError::StaleResolutionTime { .. }
            | DecisionError::InvalidAttention
            | DecisionError::MissingOperation(_)
            | DecisionError::MissingCharacter(_)
            | DecisionError::MissingOrganization(_)
            | DecisionError::OperationNotInProgress { .. }
            | DecisionError::OperationNotAwaitingDecision { .. }
            | DecisionError::InvalidRequester { .. }
            | DecisionError::InvalidOperationDecisionContext
            | DecisionError::MissingContingency { .. }
            | DecisionError::ExistingPendingDecision { .. }
            | DecisionError::MissingPoliceResponse(_)
            | DecisionError::InvalidPoliceResponseDecision { .. }
            | DecisionError::StalePoliceResponse { .. }
            | DecisionError::ExistingPendingRecruitmentApproval { .. }
            | DecisionError::RecruitmentApprovalManagerMismatch { .. }
            | DecisionError::RecruitmentApprovalRequiresPersonnelScope { .. }
            | DecisionError::RecruitmentApprovalOrganizationMismatch { .. }
            | DecisionError::RecruitmentApprovalPolicyMismatch { .. }
            | DecisionError::StaleRecruitmentApprovalAuthority
            | DecisionError::MissingDecision(_)
            | DecisionError::DecisionNotPending(_)
            | DecisionError::InvalidDetentionCancellation { .. }
            | DecisionError::InvalidResolver { .. }
            | DecisionError::InvalidResponse { .. }
            | DecisionError::StaleOperation { .. }
            | DecisionError::StaleDecision { .. }
            | DecisionError::Delegation(_)
            | DecisionError::Recruitment(_) => false,
        },
    }
}

fn prepare_authorized_prestart_aborts(
    registry: &Registry,
    state: &AppState,
    due_authorized: &[OperationId],
) -> Result<Option<Vec<Option<Box<ValidatedOperationAbort>>>>, OperationPhaseBatchError> {
    match prepare_authorized_prestart_aborts_strict(registry, state, due_authorized) {
        Ok(planned) => Ok(Some(planned)),
        Err(error) if operation_phase_batch_is_terminally_blocked(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn prepare_authorized_prestart_aborts_strict(
    registry: &Registry,
    state: &AppState,
    due_authorized: &[OperationId],
) -> Result<Vec<Option<Box<ValidatedOperationAbort>>>, OperationPhaseBatchError> {
    let mut planned = Vec::with_capacity(due_authorized.len());
    let mut budget = Vec::new();
    for operation in due_authorized {
        let record = state
            .operations()
            .get_operation(*operation)
            .expect("due authorized operation must still exist");
        let abort = if has_missed_operation_deadline(registry, state, *operation) {
            Some(validate_deadline_missed_operation(
                registry, state, *operation,
            )?)
        } else if let Some((opportunity, _)) = state
            .opportunities()
            .expired_window_for_operation(*operation, state.now())
        {
            Some(validate_expired_opportunity_operation(
                state,
                *operation,
                opportunity.id(),
            )?)
        } else if let Some(blocker) =
            crate::operations::operation_objective::resolve_objective_blocker(state, record)
        {
            Some(validate_objective_unavailable_operation(
                state, *operation, blocker,
            )?)
        } else {
            None
        };
        if let Some(abort) = &abort {
            budget.extend(abort.id_budget());
        }
        planned.push(abort.map(Box::new));
    }
    // Preserve the existing per-operation begin/abort ordering, but prove the complete artifact
    // budget for every deterministic pre-start abort first. Operation begin may allocate only a
    // PoliceResponseId, so intervening successful starts cannot consume these reserved artifact
    // kinds before a later prepared abort commits.
    state.ids.reserve_many(&budget)?;
    Ok(planned)
}

enum PreparedOverdueOperationCleanup {
    Direct {
        operation: OperationId,
        abort: Box<ValidatedOperationAbort>,
    },
    Decision {
        operation: OperationId,
        resolution: ValidatedDecisionResolution,
    },
}

fn apply_overdue_operation_cleanup(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<OperationId>, OperationPhaseBatchError> {
    match apply_overdue_operation_cleanup_strict(registry, state) {
        Ok(aborted) => Ok(aborted),
        Err(error) if operation_phase_batch_is_terminally_blocked(&error) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn apply_overdue_operation_cleanup_strict(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<OperationId>, OperationPhaseBatchError> {
    let due = find_due_operations_with_missed_deadlines(state);
    let mut planned = Vec::with_capacity(due.len());
    let mut budget = Vec::new();

    for operation in due {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("overdue operation must still exist");
        if let Some(decision) = state.decisions().pending_for_operation(operation) {
            let resolution = validate_resolve_decision(
                registry,
                state,
                decision,
                record.responsible_organization(),
                DecisionResponse::Abort,
            )?;
            budget.extend(resolution.id_budget());
            planned.push(PreparedOverdueOperationCleanup::Decision {
                operation,
                resolution,
            });
        } else {
            let abort = validate_deadline_missed_operation(registry, state, operation)?;
            budget.extend(abort.id_budget());
            planned.push(PreparedOverdueOperationCleanup::Direct {
                operation,
                abort: Box::new(abort),
            });
        }
    }

    // Active-operation invariants forbid participant overlap and each pending decision belongs to
    // exactly one operation. No planned overdue abort can therefore stale another member of this
    // cohort. Reserve every persistent artifact before terminating the first operation so global
    // ID pressure cannot make deadline chronology decide which same-minute operation survives.
    state.ids.reserve_many(&budget)?;

    let mut aborted = Vec::with_capacity(planned.len());
    for action in planned {
        match action {
            PreparedOverdueOperationCleanup::Direct { operation, abort } => {
                (*abort).commit_preflighted(state);
                aborted.push(operation);
            }
            PreparedOverdueOperationCleanup::Decision {
                operation,
                resolution,
            } => {
                resolution
                    .commit(state)
                    .expect("prevalidated disjoint overdue decision must remain current");
                aborted.push(operation);
            }
        }
    }
    Ok(aborted)
}

fn run_operation_resolution_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<OperationId>, OperationResolutionError> {
    let due_operations = find_due_in_progress_operations(state);
    // Resolution effects remain sequential because one operation can change world/legal context
    // observed by a later same-minute operation. Some persistence is nevertheless unconditional:
    // every resolution emits one report, one history event, one organization after-action fact,
    // and one personal after-action fact per participant. Reserve that exact mandatory footprint
    // for the complete cohort before the first RNG draw or mutation. Outcome-dependent incident
    // and discovery artifacts remain in each resolution's canonical per-item preflight.
    let mut mandatory_budget = Vec::with_capacity(due_operations.len() * 3);
    for operation in &due_operations {
        let record = state
            .operations()
            .get_operation(*operation)
            .expect("due operation must still exist");
        mandatory_budget.extend(mandatory_operation_resolution_id_budget(record));
    }
    if state.ids.reserve_many(&mandatory_budget).is_err() {
        // Mandatory after-action persistence is part of every resolution. If the complete due
        // cohort no longer fits a finite ID rail, resolve none of it and publish no RNG draws;
        // allowing an ID-ordered prefix would make terminal behavior depend on stable IDs.
        return Ok(Vec::new());
    }
    let mut resolved_operations = Vec::with_capacity(due_operations.len());
    for operation in due_operations {
        let kind = state
            .operations()
            .get_operation(operation)
            .expect("due operation must still exist")
            .kind();
        let execution = registry.get_operation(kind).execution();
        let mut advanced_rng = state.operation_rng_mut().clone();
        let execution_variance =
            draw_signed_variance(&mut advanced_rng, execution.variance_limit());
        let exposure_variance =
            draw_signed_variance(&mut advanced_rng, execution.exposure_variance_limit());
        let resolved = match decide_operation_resolution(
            registry,
            state,
            operation,
            OperationResolutionRandomness::new(execution_variance, exposure_variance),
        )
        .and_then(|plan| validate_operation_resolution_plan(registry, state, plan))
        .and_then(|validated| validated.commit(state))
        {
            Ok(resolved) => resolved,
            Err(error) if operation_resolution_is_terminally_blocked(&error) => {
                // Outcome-dependent artifacts and objective effects are preflighted per item
                // because earlier same-minute resolutions may legitimately change later context.
                // A finite persistence/version rail therefore leaves only this operation due;
                // its speculative draw is discarded and later independent work may continue.
                continue;
            }
            Err(error) => return Err(error),
        };
        // A cycle's RNG draw is part of that cycle's transaction. Publish it only after the
        // validated resolution has committed, so allocator or freshness rejection cannot consume
        // randomness for an operation that did not actually resolve.
        *state.operation_rng_mut() = advanced_rng;
        resolved_operations.push(resolved);
    }
    Ok(resolved_operations)
}

fn operation_resolution_is_terminally_blocked(error: &OperationResolutionError) -> bool {
    use crate::economy::business_economy_system::BusinessEconomyError;
    use crate::finance::finance_system::FinanceError;
    use crate::history::history_system::HistoryError;
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::arrest_system::ArrestError;
    use crate::legal::investigation_system::InvestigationError;
    use crate::legal::witness_system::WitnessError;
    use crate::reports::report_system::ReportError;

    fn arrest_capacity(error: &ArrestError) -> bool {
        matches!(
            error,
            ArrestError::IdExhaustion(_) | ArrestError::VersionCapacity(_)
        )
    }
    fn investigation_capacity(error: &InvestigationError) -> bool {
        matches!(
            error,
            InvestigationError::IdExhaustion(_)
                | InvestigationError::VersionCapacity(_)
                | InvestigationError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
        )
    }
    fn witness_capacity(error: &WitnessError) -> bool {
        matches!(
            error,
            WitnessError::IdExhaustion(_)
                | WitnessError::VersionCapacity(_)
                | WitnessError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
        )
    }
    fn business_capacity(error: &BusinessEconomyError) -> bool {
        matches!(
            error,
            BusinessEconomyError::IdExhaustion(_)
                | BusinessEconomyError::VersionCapacity(_)
                | BusinessEconomyError::Finance(
                    FinanceError::IdExhaustion(_) | FinanceError::VersionCapacity(_)
                )
                | BusinessEconomyError::Intelligence(IntelligenceError::IdExhaustion(_))
        )
    }

    match error {
        OperationResolutionError::IdExhaustion(_)
        | OperationResolutionError::VersionCapacity(_)
        | OperationResolutionError::Intelligence(IntelligenceError::IdExhaustion(_))
        | OperationResolutionError::History(HistoryError::IdExhaustion(_))
        | OperationResolutionError::Report(ReportError::IdExhaustion(_)) => true,
        OperationResolutionError::Investigation(error) => investigation_capacity(error),
        OperationResolutionError::Witness(error) => witness_capacity(error),
        OperationResolutionError::Arrest(error) => arrest_capacity(error),
        OperationResolutionError::DetaineeRelease { error, .. } => arrest_capacity(error),
        OperationResolutionError::BusinessEconomy(error) => business_capacity(error),
        OperationResolutionError::MissingOperation(_)
        | OperationResolutionError::OperationNotInProgress(_)
        | OperationResolutionError::ResolutionNotDue { .. }
        | OperationResolutionError::VarianceOutOfRange { .. }
        | OperationResolutionError::ExposureVarianceOutOfRange { .. }
        | OperationResolutionError::PropertyProceedsOverflow { .. }
        | OperationResolutionError::StalePropertyProceedsContext { .. }
        | OperationResolutionError::CashProceedsOverflow { .. }
        | OperationResolutionError::StaleCashProceedsContext { .. }
        | OperationResolutionError::StaleObjectiveContext { .. }
        | OperationResolutionError::StaleExtractionContext { .. }
        | OperationResolutionError::StaleOperation { .. }
        | OperationResolutionError::StaleResolutionTime { .. }
        | OperationResolutionError::StalePoliceDeploymentContext { .. }
        | OperationResolutionError::StalePoliceResponseContext { .. }
        | OperationResolutionError::StaleIncidentRouting { .. }
        | OperationResolutionError::StaleIncidentJurisdictionVersion { .. }
        | OperationResolutionError::Intelligence(_)
        | OperationResolutionError::History(_)
        | OperationResolutionError::Report(_)
        | OperationResolutionError::InformationAcquisition(_) => false,
    }
}

/// Resolves due scheduled detective work with pre-drawn variance. Runs after operation
/// consequences so legal state created by an operation is visible to later institutional work
/// in the same minute without bypassing evidence ownership.
fn run_investigation_work_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let due_work = find_due_scheduled_investigation_work(state);
    let mut advanced_rng = state.investigation_rng_mut().clone();
    let mut planned = Vec::with_capacity(due_work.len());
    let mut id_budget = Vec::new();
    for work in due_work {
        let kind = state
            .legal()
            .get_investigation_work(work)
            .expect("due investigation work must still exist")
            .kind();
        let variance_limit = registry.get_investigation_work(kind).variance_limit();
        let variance = draw_signed_variance(&mut advanced_rng, variance_limit);
        let plan = match decide_investigation_work_resolution(
            registry,
            state,
            work,
            InvestigationWorkRandomness::new(variance),
        ) {
            Ok(plan) => plan,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let validated = match validate_investigation_work_resolution_plan(registry, state, plan) {
            Ok(validated) => validated,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        id_budget.extend(validated.id_budget());
        planned.push(validated);
    }

    // Due work belongs to distinct active case/lead pairs. Freeze every draw and validate every
    // resolution against the same pre-pass snapshot, then reserve the complete artifact budget
    // before publishing either RNG progress or the first case mutation. This prevents a later
    // successful review/interview from leaving an earlier prefix resolved when global evidence or
    // statement IDs are nearly exhausted.
    if state.ids.reserve_many(&id_budget).is_err() {
        return Ok(Vec::new());
    }

    let mut resolved = Vec::with_capacity(planned.len());
    for validated in planned {
        resolved.push(
            validated
                .commit(state)
                .expect("prevalidated distinct investigation work must remain current"),
        );
    }
    // Publish the frozen draw sequence only after every due resolution has committed. The work
    // items are disjoint, but keeping RNG publication last preserves the transaction boundary
    // even if a future resolution effect introduces a new fallible dependency.
    *state.investigation_rng_mut() = advanced_rng;
    Ok(resolved)
}

fn investigation_work_is_terminally_blocked(error: &InvestigationWorkError) -> bool {
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::witness_system::WitnessError;

    match error {
        InvestigationWorkError::IdExhaustion(_) | InvestigationWorkError::VersionCapacity(_) => {
            true
        }
        InvestigationWorkError::InterviewStatementFailed { error, .. } => matches!(
            error,
            WitnessError::IdExhaustion(_)
                | WitnessError::VersionCapacity(_)
                | WitnessError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
        ),
        InvestigationWorkError::MissingInvestigation(_)
        | InvestigationWorkError::InactiveInvestigation(_)
        | InvestigationWorkError::MissingInvestigator(_)
        | InvestigationWorkError::InvestigatorNotAssigned { .. }
        | InvestigationWorkError::DetainedInvestigator { .. }
        | InvestigationWorkError::MissingInvestigationCapability(_)
        | InvestigationWorkError::InvalidFocus
        | InvestigationWorkError::WitnessAlreadyStatemented { .. }
        | InvestigationWorkError::WitnessIsCaseSubject { .. }
        | InvestigationWorkError::WitnessInterviewLimitReached { .. }
        | InvestigationWorkError::EvidenceAlreadyReviewed { .. }
        | InvestigationWorkError::EvidenceReviewAlreadyAttempted { .. }
        | InvestigationWorkError::DuplicateScheduledWork { .. }
        | InvestigationWorkError::InvestigatorBusy { .. }
        | InvestigationWorkError::StaleInvestigation { .. }
        | InvestigationWorkError::StaleInvestigator { .. }
        | InvestigationWorkError::MissingWork(_)
        | InvestigationWorkError::WorkNotScheduled(_)
        | InvestigationWorkError::WorkNotDue { .. }
        | InvestigationWorkError::StaleWork { .. }
        | InvestigationWorkError::StaleResolutionContext { .. }
        | InvestigationWorkError::StaleResolutionTime { .. }
        | InvestigationWorkError::VarianceOutOfRange { .. }
        | InvestigationWorkError::SimulationTimeOverflow
        | InvestigationWorkError::InvalidSourceEvidence(_)
        | InvestigationWorkError::WitnessInterviewAttemptCapacity { .. } => false,
    }
}

/// Settles due business operating cycles with pre-drawn gross variance.
fn run_business_cycle_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<BusinessCycleId>, crate::economy::business_economy_system::BusinessEconomyError> {
    let due_businesses = find_due_businesses(state);
    let mut advanced_rng = state.business_rng_mut().clone();
    let mut planned = Vec::with_capacity(due_businesses.len());
    let mut id_budget = Vec::new();
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
        let variance = draw_basis_point_variance(&mut advanced_rng, variance_limit);
        let plan = match decide_business_cycle(registry, state, business, variance) {
            Ok(plan) => plan,
            Err(error) if business_cycle_is_terminally_blocked(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let validated = match validate_business_cycle_plan(state, plan) {
            Ok(validated) => validated,
            Err(error) if business_cycle_is_terminally_blocked(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        id_budget.extend(validated.id_budget());
        planned.push(validated);
    }

    // Business economies have business-owned operating/settlement accounts, with settlement
    // accounts unique by owner index. Due cycles therefore cannot stale one another's financial
    // snapshots. Freeze every draw and reserve the complete persistent-ID budget before
    // publishing RNG progress or the first settlement so allocator pressure cannot settle only
    // the lowest business IDs in a same-minute cohort.
    if state.ids.reserve_many(&id_budget).is_err() {
        return Ok(Vec::new());
    }
    *state.business_rng_mut() = advanced_rng;

    let mut business_cycles = Vec::with_capacity(planned.len());
    for validated in planned {
        business_cycles.push(validated.commit_preflighted(state));
    }
    Ok(business_cycles)
}

fn business_cycle_is_terminally_blocked(
    error: &crate::economy::business_economy_system::BusinessEconomyError,
) -> bool {
    use crate::economy::business_economy_system::BusinessEconomyError;
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;

    match error {
        BusinessEconomyError::IdExhaustion(_) | BusinessEconomyError::VersionCapacity(_) => true,
        BusinessEconomyError::Finance(
            FinanceError::IdExhaustion(_) | FinanceError::VersionCapacity(_),
        ) => true,
        BusinessEconomyError::Intelligence(IntelligenceError::IdExhaustion(_)) => true,
        BusinessEconomyError::Enterprise(error) => enterprise_cycle_is_terminally_blocked(error),
        BusinessEconomyError::MissingBusiness(_)
        | BusinessEconomyError::MissingBusinessEconomy(_)
        | BusinessEconomyError::MissingBusinessNeighborhood(_)
        | BusinessEconomyError::ExistingBusinessEconomy(_)
        | BusinessEconomyError::MissingAccount(_)
        | BusinessEconomyError::AccountOwnerMismatch { .. }
        | BusinessEconomyError::InvalidOperatingAccountKind(_)
        | BusinessEconomyError::InvalidSettlementAccountKind(_)
        | BusinessEconomyError::SettlementAccountInUse { .. }
        | BusinessEconomyError::EconomyNotActive(_)
        | BusinessEconomyError::EconomyNotSuspended(_)
        | BusinessEconomyError::ActiveEnterpriseDependency { .. }
        | BusinessEconomyError::StaleEnterpriseDependency { .. }
        | BusinessEconomyError::CycleNotDue { .. }
        | BusinessEconomyError::VarianceOutOfRange { .. }
        | BusinessEconomyError::ArithmeticOverflow(_)
        | BusinessEconomyError::SimulationTimeOverflow
        | BusinessEconomyError::StaleEconomy { .. }
        | BusinessEconomyError::StaleBusiness { .. }
        | BusinessEconomyError::StaleOperatingAccount { .. }
        | BusinessEconomyError::StaleCycleTime { .. }
        | BusinessEconomyError::StaleDisruptionTime { .. }
        | BusinessEconomyError::Finance(_)
        | BusinessEconomyError::Intelligence(_) => false,
    }
}

/// Settles due enterprise cycles. Both draws happen unconditionally per due cycle so the
/// enterprise stream consumes the same number of values whatever the district's case pressure
/// turns out to be.
fn run_enterprise_cycle_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<EnterpriseCycleId>, EnterpriseError> {
    let due_enterprises = find_due_enterprises(state);
    // Later racket decisions intentionally observe earlier same-minute settlements: shared cash
    // and newly opened enforcement inquiries can change the next racket's economics. We therefore
    // cannot freeze the complete artifact budget up front without changing simulation semantics.
    // Every due settlement does, however, unconditionally persist exactly one EnterpriseCycle.
    // Prove that mandatory cohort capacity before the first mutation so this predictable finite
    // rail cannot leave a successful prefix merely because stable enterprise order reached it.
    if state
        .ids
        .reserve(
            IdKind::EnterpriseCycle,
            u32::try_from(due_enterprises.len())
                .expect("persisted due enterprise count must fit the cycle ID space"),
        )
        .is_err()
    {
        // ID exhaustion is a valid finite terminal rail. Do not settle an ID-ordered prefix and
        // do not consume RNG for work that cannot be persisted; every due racket simply remains
        // due and inert once no complete cycle-id cohort is representable.
        return Ok(Vec::new());
    }
    let mut enterprise_cycles = Vec::with_capacity(due_enterprises.len());
    for enterprise in due_enterprises {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let economics = registry.get_enterprise(kind).economics();
        let mut advanced_rng = state.enterprise_rng_mut().clone();
        let variance =
            draw_basis_point_variance(&mut advanced_rng, economics.gross_variance_basis_points());
        let enforcement_attention_roll = u16::try_from(
            draw_index(
                &mut advanced_rng,
                crate::enterprises::enterprise_execution::EnterpriseCycleRandomness::ENFORCEMENT_ATTENTION_ROLL_COUNT,
            )
                .expect("racket-attention roll range is never empty"),
        )
        .expect("racket-attention roll fits u16");
        let cycle = match decide_enterprise_cycle(
            registry,
            state,
            enterprise,
            EnterpriseCycleRandomness::new(variance, enforcement_attention_roll),
        )
        .and_then(|plan| validate_enterprise_cycle_plan(state, plan))
        .and_then(|validated| validated.commit(state))
        {
            Ok(cycle) => cycle,
            Err(error) if enterprise_cycle_is_terminally_blocked(&error) => {
                // Capacity exhaustion is not corruption. The failed cycle published neither its
                // cloned RNG state nor any authoritative mutation, so leave it due and continue
                // settling later independent rackets rather than panicking the canonical tick.
                continue;
            }
            Err(error) => return Err(error),
        };
        // Enterprise cycles stay sequential because they may share organization cash and one
        // racket's enforcement incident can change another racket's live legal-pressure context.
        // Still publish each item's two draws only after that item commits.
        *state.enterprise_rng_mut() = advanced_rng;
        enterprise_cycles.push(cycle);
    }
    Ok(enterprise_cycles)
}

fn enterprise_cycle_is_terminally_blocked(error: &EnterpriseError) -> bool {
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::investigation_system::InvestigationError;
    use crate::reports::report_system::ReportError;

    matches!(
        error,
        EnterpriseError::IdExhaustion(_)
            | EnterpriseError::VersionCapacity(_)
            | EnterpriseError::Finance(
                FinanceError::IdExhaustion(_) | FinanceError::VersionCapacity(_)
            )
            | EnterpriseError::Intelligence(IntelligenceError::IdExhaustion(_))
            | EnterpriseError::Report(ReportError::IdExhaustion(_))
            | EnterpriseError::Investigation(
                InvestigationError::IdExhaustion(_)
                    | InvestigationError::VersionCapacity(_)
                    | InvestigationError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
            )
    )
}

/// Day-boundary decay runs first in the reputation cluster: eligible aged impressions fade one
/// authored step before anything new lands, so consequences applied this minute are not
/// immediately eroded by the same boundary's decay pass. Resolved operations feed competence/
/// fear/exposure consequences; rackets that drew a racket inquiry this tick pay the same
/// institutional memory as an exposed operation. The player organization reads its own standing
/// shifts through the canonical Standing-report path — legitimate self-knowledge.
fn apply_reputation_phase(
    registry: &Registry,
    state: &mut AppState,
    resolved_operations: &[OperationId],
    enterprise_cycles: &[EnterpriseCycleId],
) -> Result<usize, crate::reputation::reputation_system::ReputationError> {
    // Snapshot the consequence inputs before any reputation write. Both the read-only projection
    // and the authoritative pass consume these exact vectors in this exact order, so clamping and
    // same-minute accumulation cannot make the report preflight drift from the later mutations.
    let operation_consequences: Vec<_> = resolved_operations
        .iter()
        .map(|operation| {
            let record = state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must exist for reputation consequences");
            let resolution = record
                .resolution()
                .expect("resolved operation carries its resolution");
            (
                record.responsible_organization(),
                record.kind(),
                record.approach(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            )
        })
        .collect();
    let racket_inquiries: Vec<_> = enterprise_cycles
        .iter()
        .filter_map(|cycle_id| {
            let cycle = state
                .enterprises()
                .get_cycle(*cycle_id)
                .expect("settled enterprise cycle must exist for reputation consequences");
            cycle.drew_enforcement_attention().then(|| {
                state
                    .enterprises()
                    .get_enterprise(cycle.enterprise())
                    .expect("settled enterprise cycle must reference its enterprise")
                    .organization()
            })
        })
        .collect();

    match preflight_reputation_phase_reports(
        registry,
        state,
        &operation_consequences,
        &racket_inquiries,
    ) {
        Ok(()) => {}
        Err(crate::reputation::reputation_system::ReputationError::IdExhaustion(_)) => {
            return Ok(0);
        }
        Err(error) => return Err(error),
    }

    let mut changed =
        crate::reputation::reputation_system::apply_daily_reputation_decay(registry, state);
    for (organization, kind, approach, objective_outcome, exposure_level) in operation_consequences
    {
        let shifts = crate::reputation::reputation_system::apply_operation_reputation_consequences(
            registry,
            state,
            organization,
            kind,
            approach,
            objective_outcome,
            exposure_level,
        )?;
        changed = changed
            .checked_add(shifts.len())
            .expect("one tick cannot contain enough reputation shifts to overflow usize");
    }
    for organization in racket_inquiries {
        let shifts =
            crate::reputation::reputation_system::apply_racket_inquiry_reputation_consequences(
                registry,
                state,
                organization,
            )?;
        changed = changed
            .checked_add(shifts.len())
            .expect("one tick cannot contain enough reputation shifts to overflow usize");
    }
    Ok(changed)
}

fn preflight_reputation_phase_reports(
    registry: &Registry,
    state: &AppState,
    operation_consequences: &[OperationReputationEvent],
    racket_inquiries: &[crate::core::id::OrganizationId],
) -> Result<(), crate::reputation::reputation_system::ReputationError> {
    // Decay itself emits no report. Project that first, then replay every current consequence.
    // A player consequence group needs exactly one Standing report iff at least one of its
    // clamped shifts still moves after all earlier same-minute shifts. Reserve that exact count
    // before decay so report exhaustion cannot leave a partially updated reputation phase.
    let player = state.player_organization();
    let mut projection =
        crate::reputation::reputation_system::ReputationPhaseProjection::after_daily_decay(
            registry, state,
        );
    let mut standing_reports = 0_u32;
    for &event in operation_consequences {
        let organization = event.0;
        let moved = projection.apply_operation_consequences(registry, state, event)?;
        if moved > 0 && player == Some(organization) {
            standing_reports = standing_reports
                .checked_add(1)
                .expect("same-minute Standing report count must fit u32");
        }
    }
    for &organization in racket_inquiries {
        let moved = projection.apply_racket_inquiry_consequences(registry, state, organization)?;
        if moved > 0 && player == Some(organization) {
            standing_reports = standing_reports
                .checked_add(1)
                .expect("same-minute Standing report count must fit u32");
        }
    }
    state.ids.reserve(IdKind::Report, standing_reports)?;
    Ok(())
}

/// Synthesizes the player organization's due executive brief.
fn synthesize_executive_brief(registry: &Registry, state: &mut AppState) -> Option<ReportId> {
    state.player_organization().and_then(|recipient| {
        is_executive_brief_due(registry, state.now())
            .then(|| {
                let plan = decide_executive_brief(registry, state, recipient)
                    .expect("due player executive brief must produce a valid synthesis plan");
                let validated = validate_executive_brief_plan(state, plan)
                    .expect("fresh executive brief plan must validate");
                match validated.commit(state) {
                    Ok(report) => Some(report),
                    Err(crate::reports::executive_brief::ExecutiveBriefError::Report(
                        crate::reports::report_system::ReportError::IdExhaustion(_),
                    )) => None,
                    Err(error) => {
                        panic!("validated executive brief must commit atomically: {error}")
                    }
                }
            })
            .flatten()
    })
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
    // `rejection_zone` is an exact multiple of `bound`. The comparison must stay strict:
    // accepting the boundary itself would give remainder zero one extra representation and
    // bias the choice. This formulation may reject a tiny full block when `bound` divides
    // 2^64 exactly, but keeps the accepted domain trivially auditable and unbiased.
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = rng.next_u64();
        if draw < rejection_zone {
            return Ok((draw % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests;
