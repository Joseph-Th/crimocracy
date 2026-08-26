//! Release-safe structural validation for the legal subsystems plus persisted reports and history,
//! split per aggregate so each contract stays readable on its own.

mod casework;
mod custody;
mod enforcement;
mod prosecution;
mod references;

pub(crate) use casework::validate_developed_review_evidence;

use crate::core::invariants::StateValidationError;

use crate::core::state::AppState;
use std::collections::BTreeSet;

/// Full legal-subsystem record validation, ordered like the custody cluster it guards:
/// institutions, patrols, arrests, representation, prosecution, investigation casework,
/// witnesses, informants, evidence provenance, then player-facing report/history integrity.
pub(super) fn validate_legal_subsystems(state: &AppState) -> Result<(), StateValidationError> {
    enforcement::validate_jurisdictions(state)?;
    enforcement::validate_police_responses(state)?;
    enforcement::validate_patrol_deployments(state)?;
    custody::validate_arrests(state)?;
    custody::validate_legal_representations(state)?;
    prosecution::validate_prosecution_cases(state)?;
    casework::validate_investigations(state)?;
    // Derived-evidence uniqueness spans detective work and the evidence graph, so the
    // seen-set is built here and threaded through both passes.
    let mut derived_evidence_from_work = BTreeSet::new();
    casework::validate_investigation_work_records(state, &mut derived_evidence_from_work)?;
    casework::validate_case_witnesses(state)?;
    let named_witness_evidence = casework::validate_witness_statements(state)?;
    custody::validate_informants(state)?;
    let informant_evidence = custody::validate_informant_disclosures(state)?;
    casework::validate_evidence_records(
        state,
        &derived_evidence_from_work,
        &named_witness_evidence,
        &informant_evidence,
    )?;
    references::validate_report_holders(state)?;
    references::validate_history_event_references(state)?;
    Ok(())
}
