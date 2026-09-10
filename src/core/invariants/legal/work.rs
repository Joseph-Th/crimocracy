//! Registry-relative validation for scheduled investigation work and witness attempt limits.

use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::legal::investigation_work_execution::validate_historical_work_factors;
use crate::legal::{InvestigationWorkKind, InvestigationWorkOutcome, InvestigationWorkRecord};
use crate::registry::Registry;
use std::collections::BTreeSet;

pub(super) fn validate_investigation_work_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    validate_witness_attempts_against_registry(registry, state)?;
    let mut derived_evidence = BTreeSet::new();
    for work in state.legal.investigation_work() {
        derived_evidence.clear();
        validate_investigation_work_record_against_registry(
            registry,
            state,
            work,
            &mut derived_evidence,
        )?;
    }
    Ok(())
}

fn validate_witness_attempts_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let witness_attempt_limit = registry.legal().witness_interview_attempt_limit();
    for witness in state.legal.case_witnesses() {
        if witness.interview_attempts() > witness_attempt_limit {
            return Err(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            });
        }
    }
    Ok(())
}

fn validate_investigation_work_record_against_registry(
    registry: &Registry,
    state: &AppState,
    work: &InvestigationWorkRecord,
    derived_evidence: &mut BTreeSet<crate::core::id::EvidenceId>,
) -> Result<(), StateValidationError> {
    let definition = registry.get_investigation_work(work.kind());
    let expected_due_at = work
        .scheduled_at()
        .checked_add(definition.duration())
        .ok_or_else(|| invalid_investigation_work(work))?;
    if work.due_at() != expected_due_at {
        return Err(invalid_investigation_work(work));
    }
    let Some(resolution) = work.resolution() else {
        return Ok(());
    };
    let factors = resolution.factors();
    let expected_margin = validate_historical_work_factors(definition, state, work, factors)
        .map_err(|_| invalid_investigation_work(work))?;
    if factors.variance().unsigned_abs() > definition.variance_limit()
        || resolution.margin() != expected_margin
    {
        return Err(invalid_investigation_work(work));
    }
    match resolution.outcome() {
        InvestigationWorkOutcome::Connected => {
            if work.kind() != InvestigationWorkKind::WitnessInterview
                || expected_margin < definition.connected_margin()
            {
                return Err(invalid_investigation_work(work));
            }
        }
        InvestigationWorkOutcome::Developed => {
            if work.kind() != InvestigationWorkKind::EvidenceReview
                || expected_margin < definition.connected_margin()
            {
                return Err(invalid_investigation_work(work));
            }
            super::casework::validate_developed_review_evidence(state, work, derived_evidence)?;
        }
        InvestigationWorkOutcome::Inconclusive => {
            if expected_margin >= definition.connected_margin()
                || resolution.derived_evidence().is_some()
            {
                return Err(invalid_investigation_work(work));
            }
        }
    }
    Ok(())
}

fn invalid_investigation_work(work: &InvestigationWorkRecord) -> StateValidationError {
    StateValidationError::InvalidInvestigationWork { work: work.id() }
}
