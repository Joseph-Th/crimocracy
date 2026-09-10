//! Persistent ID high-water validation across every authoritative owner.

use crate::core::id::{IdCounters, IdKind};
use crate::core::state::AppState;

use super::StateValidationError;

pub(super) fn validate_id_allocators(state: &AppState) -> Result<(), StateValidationError> {
    // Each check reads only the smallest and largest persisted id from the collection's
    // key order; walking every record per allocator per validation would rescan whole
    // histories that grow for the life of the campaign.
    validate_id_allocator(
        &state.ids,
        IdKind::Organization,
        state.world.organization_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Character,
        state.world.character_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Neighborhood,
        state.world.neighborhood_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Business,
        state.world.business_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::BusinessOwnershipChange,
        state.world.ownership_change_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Operation,
        state.operations.operation_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Opportunity,
        state.opportunities.opportunity_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Information,
        state.intelligence.information_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Contact,
        state.contacts.contact_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ContactDisclosure,
        state.contacts.disclosure_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Investigation,
        state.legal.investigation_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::InvestigationWork,
        state.legal.investigation_work_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::PatrolDeployment,
        state.legal.patrol_deployment_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::PoliceResponse,
        state.legal.police_response_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::CaseWitness,
        state.legal.case_witness_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::WitnessStatement,
        state.legal.witness_statement_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Informant,
        state.legal.informant_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::InformantDisclosure,
        state.legal.informant_disclosure_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Evidence,
        state.legal.evidence_id_bounds(),
    )?;
    validate_id_allocator(&state.ids, IdKind::Arrest, state.legal.arrest_id_bounds())?;
    validate_id_allocator(
        &state.ids,
        IdKind::LegalRepresentation,
        state.legal.legal_representation_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ProsecutionCase,
        state.legal.prosecution_case_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ProsecutionReferral,
        state.legal.prosecution_referral_id_bounds(),
    )?;
    validate_id_allocator(&state.ids, IdKind::Report, state.reports.report_id_bounds())?;
    validate_id_allocator(
        &state.ids,
        IdKind::HistoryEvent,
        state.history.event_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::FinancialAccount,
        state.finance.account_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::LedgerTransaction,
        state.finance.transaction_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::DecisionRequest,
        state.decisions.decision_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Mandate,
        state.delegation.mandate_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::RecruitmentAttempt,
        state.recruitment.attempt_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Enterprise,
        state.enterprises.enterprise_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::EnterpriseCycle,
        state.enterprises.enterprise_cycle_id_bounds(),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::BusinessCycle,
        state.economy.business_cycle_id_bounds(),
    )?;
    Ok(())
}

fn validate_id_allocator(
    counters: &IdCounters,
    kind: IdKind,
    bounds: Option<(u32, u32)>,
) -> Result<(), StateValidationError> {
    // Ids are allocated monotonically, so a zero id would also be the smallest key.
    if bounds.is_some_and(|(smallest, _)| smallest == 0) {
        return Err(StateValidationError::InvalidPersistentId { kind: kind.label() });
    }
    let highest = bounds.map_or(0, |(_, largest)| largest);
    let next = counters.next_raw(kind);
    if next <= highest {
        return Err(StateValidationError::InvalidIdAllocator {
            kind: kind.label(),
            next,
            highest,
        });
    }
    Ok(())
}
