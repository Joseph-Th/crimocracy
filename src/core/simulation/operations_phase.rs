//! Due operations, pre-start admission, deadline cleanup, and resolution.

use super::draw_signed_variance;
use crate::core::id::{IdExhaustionError, OperationId, PoliceResponseId};
use crate::core::state::AppState;
use crate::decisions::DecisionResponse;
use crate::decisions::decision_system::{
    DecisionError, DecisionRequestOutcome, ValidatedDecisionResolution, validate_resolve_decision,
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
use crate::registry::Registry;
use thiserror::Error;

/// Processes due police-response arrivals, aborts missed deadlines (through the pending decision
/// when one exists), resolves due in-progress operations with pre-drawn deterministic variance,
/// then starts due authorized operations.
pub(super) struct OperationsPhaseOutcome {
    pub(super) started: Vec<OperationId>,
    pub(super) arrived_police_responses: Vec<PoliceResponseId>,
    pub(super) decision_requests: Vec<DecisionRequestOutcome>,
    pub(super) aborted: Vec<OperationId>,
    pub(super) resolved: Vec<OperationId>,
}

pub(super) fn run_operations_phase(
    registry: &Registry,
    state: &mut AppState,
) -> OperationsPhaseOutcome {
    // Process responses that were dispatched on earlier ticks before closing or admitting work.
    // Authorization deliberately allows exact back-to-back participant windows. A response
    // arriving on that boundary can turn the earlier operation into an unresolved commitment;
    // that state must be visible before due resolution releases the participant and before the
    // follow-up starts.
    // Newly started operations cannot add another due arrival here because registry validation
    // requires every police-response delay to be strictly positive.
    let police_response_outcome = apply_due_police_response_arrivals(state)
        .expect("due police responses must commit through canonical arrival processing");
    let arrived_police_responses = police_response_outcome.arrived;
    let decision_requests = police_response_outcome.decisions;
    let mut aborted_operations = police_response_outcome.aborted_operations;

    // Hard completion deadlines are lifecycle boundaries, not advisory scheduling hints. Clear
    // work that was already overdue at the beginning of this minute before it can block a new
    // commitment during the same minute. A deadline equal to now is still live and may resolve
    // below; only strictly missed deadlines enter this cleanup.
    aborted_operations.extend(
        apply_overdue_operation_cleanup(registry, state)
            .expect("valid overdue operations must abort as one artifact-preflighted cohort"),
    );
    // Resolve windows ending at this minute before admitting exact back-to-back starts. This makes
    // the half-open booking contract [start, end) executable in practice: an operation ending at
    // minute N releases its participants and applies its consequences before a successor begins at
    // N. A police arrival above can still pause that operation first, in which case it is not due
    // resolution and continues to block the successor.
    let resolved_operations = run_operation_resolution_phase(registry, state).expect(
        "valid due operation resolutions must commit through the preflighted sequential phase",
    );

    let due_authorized = find_due_authorized_operations(state);
    let prestart_actions = plan_authorized_prestarts(registry, state, &due_authorized)
        .expect("valid due authorized prestarts must plan without mutation");
    let mut started_operations = Vec::with_capacity(due_authorized.len());
    for (operation, action) in due_authorized.into_iter().zip(prestart_actions) {
        match action {
            AuthorizedPrestartAction::Abort(abort) => {
                (*abort).commit_preflighted(state);
                aborted_operations.push(operation);
            }
            // One impossible abort must not suppress an unrelated due start. Keep the blocked
            // operation Authorized, rather than attempting to begin an expired objective.
            AuthorizedPrestartAction::Blocked => {}
            AuthorizedPrestartAction::Begin => match apply_transition(
                registry,
                state,
                operation,
                OperationTransition::Begin,
            ) {
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
                Err(error) if operation_error_is_terminally_blocked(&error) => {}
                Err(error) => {
                    panic!(
                        "due authorized operation could not begin through its canonical path: {error}"
                    )
                }
            },
        }
    }
    OperationsPhaseOutcome {
        started: started_operations,
        arrived_police_responses,
        decision_requests,
        aborted: aborted_operations,
        resolved: resolved_operations,
    }
}

#[derive(Debug, Error)]
pub(super) enum OperationPhaseBatchError {
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

pub(super) enum AuthorizedPrestartAction {
    Begin,
    Abort(Box<ValidatedOperationAbort>),
    Blocked,
}

pub(super) fn plan_authorized_prestarts(
    registry: &Registry,
    state: &AppState,
    due_authorized: &[OperationId],
) -> Result<Vec<AuthorizedPrestartAction>, OperationPhaseBatchError> {
    let mut planned = Vec::with_capacity(due_authorized.len());
    let mut budget = Vec::new();
    for operation in due_authorized {
        let record = state
            .operations()
            .get_operation(*operation)
            .expect("due authorized operation must still exist");
        let abort: Result<_, OperationPhaseBatchError> = (|| {
            if has_missed_operation_deadline(registry, state, *operation) {
                Ok(Some(validate_deadline_missed_operation(
                    registry, state, *operation,
                )?))
            } else if let Some((opportunity, _)) = state
                .opportunities()
                .expired_window_for_operation(*operation, state.now())
            {
                Ok(Some(validate_expired_opportunity_operation(
                    state,
                    *operation,
                    opportunity.id(),
                )?))
            } else if let Some(blocker) =
                crate::operations::operation_objective::resolve_objective_blocker(state, record)
            {
                Ok(Some(validate_objective_unavailable_operation(
                    state, *operation, blocker,
                )?))
            } else {
                Ok(None)
            }
        })();
        let action = match abort {
            Ok(Some(abort)) => {
                budget.extend(abort.id_budget());
                AuthorizedPrestartAction::Abort(Box::new(abort))
            }
            Ok(None) => AuthorizedPrestartAction::Begin,
            Err(error) if operation_phase_batch_is_terminally_blocked(&error) => {
                AuthorizedPrestartAction::Blocked
            }
            Err(error) => return Err(error),
        };
        planned.push(action);
    }
    // No ID-ordered prefix of aborts may consume the last artifact IDs. If their combined
    // budget cannot fit, retain *all* those abort candidates without mutation; independent
    // starts remain actionable. Begin only allocates PoliceResponseId, so successful starts
    // cannot consume the preflighted abort artifact kinds.
    if state.ids.reserve_many(&budget).is_err() {
        for action in &mut planned {
            if matches!(action, AuthorizedPrestartAction::Abort(_)) {
                *action = AuthorizedPrestartAction::Blocked;
            }
        }
    }
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

pub(super) fn apply_overdue_operation_cleanup(
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

pub(super) fn run_operation_resolution_phase(
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
    let mut viable_operations = Vec::with_capacity(due_operations.len());
    for operation in due_operations {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("due operation must still exist");
        let budget = mandatory_operation_resolution_id_budget(record);
        // If even this operation's guaranteed after-action footprint cannot fit, its
        // resolution is independently impossible at the finite ID rail. It must not
        // inflate the shared budget and freeze unrelated operations that *can* finish.
        // This read-only test consumes neither ID capacity nor operation RNG.
        if state.ids.reserve_many(&budget).is_err() {
            continue;
        }
        mandatory_budget.extend(budget);
        viable_operations.push(operation);
    }
    if state.ids.reserve_many(&mandatory_budget).is_err() {
        // Mandatory after-action persistence is part of every resolution. If the complete due
        // cohort no longer fits a finite ID rail, resolve none of it and publish no RNG draws;
        // allowing an ID-ordered prefix would make terminal behavior depend on stable IDs.
        return Ok(Vec::new());
    }
    let mut resolved_operations = Vec::with_capacity(viable_operations.len());
    for operation in viable_operations {
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
