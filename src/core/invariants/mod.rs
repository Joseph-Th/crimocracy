//! Runtime invariant enforcement and release-safe structural state validation.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    ArrestId, BusinessCycleId, BusinessId, CaseWitnessId, CharacterId, ContactDisclosureId,
    ContactId, DecisionRequestId, EnterpriseCycleId, EnterpriseId, IdCounters, IdKind,
    InformantDisclosureId, InformantId, InformationId, InvestigationId, InvestigationWorkId,
    LedgerTransactionId, LegalRepresentationId, MandateId, OperationId, OpportunityId,
    OrganizationId, PatrolDeploymentId, PoliceResponseId, ProsecutionCaseId, ProsecutionReferralId,
    RecruitmentAttemptId, ReportId, WitnessStatementId,
};
use crate::core::invariants::legal as legal_invariants;
use crate::core::state::AppState;
#[cfg(debug_assertions)]
use crate::core::state::CURRENT_STATE_SCHEMA_VERSION;
use crate::decisions::DecisionResponse;
use crate::enterprises::EnterpriseLocation;
use crate::legal::investigation_work_execution::resolve_work_factors_and_margin;
use crate::legal::{InvestigationWorkKind, InvestigationWorkOutcome};
use crate::operations::operation_economics::resolve_property_proceeds;
use crate::operations::operation_execution::{
    has_police_response_arrived_by, resolve_execution_margin, resolve_exposure_level,
    resolve_exposure_score, resolve_intelligence_factors, resolve_objective_outcome,
};
use crate::operations::police_response_integration::resolve_police_arrival_delay;
use crate::operations::property_disposition::resolve_property_liquidation_value;
use crate::operations::{OperationContingency, OperationStatus};
use crate::opportunities::OpportunityResolution;
use crate::registry::Registry;
use crate::world::{BusinessFunction, PolicyKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StateValidationError {
    #[error("{kind} state contains reserved persistent ID 0")]
    InvalidPersistentId { kind: &'static str },
    #[error(
        "{kind} ID allocator next value {next} is not greater than highest persisted ID {highest}"
    )]
    InvalidIdAllocator {
        kind: &'static str,
        next: u32,
        highest: u32,
    },
    #[error("{subsystem} derived indexes are inconsistent with source records")]
    IndexInconsistency { subsystem: &'static str },
    #[error("{context} references missing entity {entity:?}")]
    MissingEntity {
        context: &'static str,
        entity: EntityRef,
    },
    #[error("organization {organization:?} stores an out-of-range {audience:?} reputation score")]
    InvalidReputationScore {
        organization: OrganizationId,
        audience: crate::reputation::AudienceKind,
    },
    #[error("player organization {organization} is not a criminal organization")]
    InvalidPlayerOrganization { organization: OrganizationId },
    #[error("organization {organization} is missing policy {policy:?}")]
    MissingPolicy {
        organization: OrganizationId,
        policy: PolicyKind,
    },
    #[error("organization {organization} stores policy {actual:?} under key {expected:?}")]
    PolicyKindMismatch {
        organization: OrganizationId,
        expected: PolicyKind,
        actual: PolicyKind,
    },
    #[error("character {character} and supervisor {supervisor} belong to different organizations")]
    SupervisorOrganizationMismatch {
        character: CharacterId,
        supervisor: CharacterId,
    },
    #[error("supervision hierarchy contains a cycle involving character {character}")]
    SupervisionCycle { character: CharacterId },
    #[error("information {information} has invalid observation/recording chronology")]
    InvalidInformationChronology { information: InformationId },
    #[error("information {information} has invalid provenance source {source_information}")]
    InvalidInformationProvenance {
        information: InformationId,
        source_information: InformationId,
    },
    #[error("institutional contact {contact} has invalid persisted state")]
    InvalidInstitutionalContact { contact: ContactId },
    #[error("institutional contact disclosure {disclosure} has invalid persisted provenance")]
    InvalidContactDisclosure { disclosure: ContactDisclosureId },
    #[error("active operation {operation} has an inactive or foreign leader")]
    ActiveOperationInvalidLeader { operation: OperationId },
    #[error("active operation {operation} has inactive participant {participant}")]
    ActiveOperationInvalidParticipant {
        operation: OperationId,
        participant: CharacterId,
    },
    #[error("active operation {operation} has a foreign participant {participant}")]
    ActiveOperationForeignParticipant {
        operation: OperationId,
        participant: CharacterId,
    },
    #[error("operation {operation} has invalid execution lifecycle state")]
    InvalidOperationRuntime { operation: OperationId },
    #[error("completed operation {operation} has an invalid after-action information link")]
    InvalidOperationAfterAction { operation: OperationId },
    #[error("completed operation {operation} has an invalid after-action report link")]
    InvalidOperationAfterActionReport { operation: OperationId },
    #[error(
        "aborted operation {operation} has invalid abort provenance or after-action artifacts"
    )]
    InvalidOperationAbort { operation: OperationId },
    #[error("completed operation {operation} has an invalid campaign-history link")]
    InvalidOperationHistory { operation: OperationId },
    #[error("completed operation {operation} has invalid discovered-information provenance")]
    InvalidOperationDiscovery { operation: OperationId },
    #[error("completed operation {operation} has invalid player legal-activity information")]
    InvalidOperationLegalActivity { operation: OperationId },
    #[error("operation {operation} is incompatible with its authored definition")]
    InvalidOperationDefinition { operation: OperationId },
    #[error("operation {operation} has invalid persisted exposure or legal consequences")]
    InvalidOperationExposure { operation: OperationId },
    #[error("operation {operation} has invalid persisted property disposition")]
    InvalidOperationPropertyDisposition { operation: OperationId },
    #[error("operation {operation} has invalid persisted cash proceeds")]
    InvalidOperationCashProceeds { operation: OperationId },
    #[error("operation {operation} has invalid persisted cash disposition")]
    InvalidOperationCashDisposition { operation: OperationId },
    #[error("opportunity {opportunity} has invalid persisted provenance or lifecycle state")]
    InvalidOpportunity { opportunity: OpportunityId },
    #[error("organization {organization} has invalid legal jurisdiction state")]
    InvalidLegalJurisdiction { organization: OrganizationId },
    #[error("patrol deployment {deployment} has invalid persisted state")]
    InvalidPatrolDeployment { deployment: PatrolDeploymentId },
    #[error("police response {response} has invalid persisted state")]
    InvalidPoliceResponse { response: PoliceResponseId },
    #[error("arrest {arrest} has invalid persisted custody state or provenance")]
    InvalidArrest { arrest: ArrestId },
    #[error("legal representation {representation} has invalid persisted state or provenance")]
    InvalidLegalRepresentation {
        representation: LegalRepresentationId,
    },
    #[error("prosecution case {case} has invalid persisted state or referral provenance")]
    InvalidProsecutionCase { case: ProsecutionCaseId },
    #[error("prosecution referral {referral} has invalid persisted evidence or report provenance")]
    InvalidProsecutionReferral { referral: ProsecutionReferralId },
    #[error("investigation {investigation} has invalid investigator staffing")]
    InvalidInvestigationStaffing { investigation: InvestigationId },
    #[error("investigation {investigation} has invalid origin or case-awareness provenance")]
    InvalidInvestigationActivity { investigation: InvestigationId },
    #[error("investigation work {work} has invalid persisted state")]
    InvalidInvestigationWork { work: InvestigationWorkId },
    #[error("evidence {evidence} has invalid derived provenance")]
    InvalidEvidenceProvenance {
        evidence: crate::core::id::EvidenceId,
    },
    #[error("case witness {witness} has invalid persisted state")]
    InvalidCaseWitness { witness: CaseWitnessId },
    #[error("witness statement {statement} has invalid persisted state")]
    InvalidWitnessStatement { statement: WitnessStatementId },
    #[error("informant {informant} has invalid persisted state")]
    InvalidInformant { informant: InformantId },
    #[error("informant disclosure {disclosure} has invalid persisted provenance")]
    InvalidInformantDisclosure { disclosure: InformantDisclosureId },
    #[error("recruitment attempt {attempt} has invalid persisted state")]
    InvalidRecruitmentAttempt { attempt: RecruitmentAttemptId },
    #[error("decision {decision} has an invalid attention class")]
    InvalidDecisionAttention { decision: DecisionRequestId },
    #[error("decision {decision} has an empty summary")]
    EmptyDecisionSummary { decision: DecisionRequestId },
    #[error("decision {decision} has no available responses")]
    DecisionHasNoResponses { decision: DecisionRequestId },
    #[error("resolved decision {decision} carries no resolution record")]
    ResolvedDecisionWithoutResolution { decision: DecisionRequestId },
    #[error("decision {decision} has invalid persisted context state")]
    InvalidDecisionContext { decision: DecisionRequestId },
    #[error("decision {decision} requester {requester} is not operation {operation}'s leader")]
    DecisionRequesterMismatch {
        decision: DecisionRequestId,
        requester: CharacterId,
        operation: OperationId,
    },
    #[error("decision {decision} recipient {recipient} does not own operation {operation}")]
    DecisionRecipientMismatch {
        decision: DecisionRequestId,
        recipient: OrganizationId,
        operation: OperationId,
    },
    #[error("decision {decision} has invalid request/resolution chronology")]
    InvalidDecisionChronology { decision: DecisionRequestId },
    #[error("pending decision {decision} points to operation {operation} in status {status:?}")]
    PendingDecisionOperationMismatch {
        decision: DecisionRequestId,
        operation: OperationId,
        status: OperationStatus,
    },
    #[error("operation {operation} is awaiting a decision but has no pending decision record")]
    AwaitingOperationMissingDecision { operation: OperationId },
    #[error(
        "decision {decision} was resolved by organization {resolver}, not recipient {recipient}"
    )]
    DecisionResolverMismatch {
        decision: DecisionRequestId,
        resolver: OrganizationId,
        recipient: OrganizationId,
    },
    #[error("decision {decision} resolved with response {response:?} that was not offered")]
    DecisionResponseNotOffered {
        decision: DecisionRequestId,
        response: DecisionResponse,
    },
    #[error("decision {decision} resolved as Abort but operation {operation} is not aborted")]
    AbortDecisionOperationMismatch {
        decision: DecisionRequestId,
        operation: OperationId,
    },
    #[error("mandate {mandate} has no responsibility scopes")]
    MandateHasNoScopes { mandate: crate::core::id::MandateId },
    #[error("active mandate {mandate} has invalid manager {manager}")]
    ActiveMandateInvalidManager {
        mandate: crate::core::id::MandateId,
        manager: CharacterId,
    },
    #[error("mandate {mandate} manager {manager} belongs to a different organization")]
    MandateManagerOrganizationMismatch {
        mandate: crate::core::id::MandateId,
        manager: CharacterId,
    },
    #[error("mandate {mandate} stores policy {actual:?} under key {expected:?}")]
    MandatePolicyKindMismatch {
        mandate: crate::core::id::MandateId,
        expected: PolicyKind,
        actual: PolicyKind,
    },
    #[error("mandate {mandate} has a negative budget limit")]
    NegativeMandateBudget { mandate: MandateId },
    #[error("mandate {mandate} budget account {account} is not owned by its organization")]
    MandateBudgetAccountOwnerMismatch {
        mandate: MandateId,
        account: crate::core::id::FinancialAccountId,
    },
    #[error("report {report} references missing information {information}")]
    MissingReportInformation {
        report: ReportId,
        information: InformationId,
    },
    #[error("report {report} references information {information} unavailable to its recipient")]
    ReportInformationUnavailable {
        report: ReportId,
        information: InformationId,
    },
    #[error("report {report} references missing decision {decision}")]
    MissingReportDecision {
        report: ReportId,
        decision: DecisionRequestId,
    },
    #[error("report {report} references decision {decision} belonging to another recipient")]
    ReportDecisionRecipientMismatch {
        report: ReportId,
        decision: DecisionRequestId,
    },
    #[error("{context} contains a timestamp later than the current simulation time")]
    FutureTimestamp { context: &'static str },
    #[error("financial account balances do not match their ledger postings")]
    FinancialBalanceMismatch,
    #[error("ledger transaction {transaction} postings overflow while summing")]
    LedgerArithmeticOverflow {
        transaction: crate::core::id::LedgerTransactionId,
    },
    #[error("ledger transaction {transaction} is unbalanced by {net_cents} cents")]
    UnbalancedLedgerTransaction {
        transaction: crate::core::id::LedgerTransactionId,
        net_cents: i64,
    },
    #[error("ledger transaction {transaction} has invalid persisted budget usage")]
    InvalidBudgetUsage { transaction: LedgerTransactionId },
    #[error("enterprise {enterprise} has invalid authority or ownership state")]
    InvalidEnterpriseAuthority { enterprise: EnterpriseId },
    #[error("enterprise {enterprise} has invalid location state")]
    InvalidEnterpriseLocation { enterprise: EnterpriseId },
    #[error("enterprise {enterprise} has invalid financial account configuration")]
    InvalidEnterpriseAccounts { enterprise: EnterpriseId },
    #[error("enterprise {enterprise} has invalid lifecycle scheduling state")]
    InvalidEnterpriseSchedule { enterprise: EnterpriseId },
    #[error("enterprise cycle {cycle} has invalid economics or ledger linkage")]
    InvalidEnterpriseCycle { cycle: EnterpriseCycleId },
    #[error("enterprise {enterprise} business {business} lacks required function {function:?}")]
    EnterpriseBusinessRequirementMissing {
        enterprise: EnterpriseId,
        business: BusinessId,
        function: BusinessFunction,
    },
    #[error("enterprise {enterprise} has invalid supporting business {business}")]
    InvalidEnterpriseSupportingBusiness {
        enterprise: EnterpriseId,
        business: BusinessId,
    },
    #[error("enterprise {enterprise} support network lacks required function {function:?}")]
    EnterpriseNetworkRequirementMissing {
        enterprise: EnterpriseId,
        function: BusinessFunction,
    },
    #[error("business {business} has invalid operating economy state")]
    InvalidBusinessEconomy { business: BusinessId },
    #[error("business {business} has invalid operating economy account configuration")]
    InvalidBusinessEconomyAccounts { business: BusinessId },
    #[error("business {business} has invalid operating economy scheduling state")]
    InvalidBusinessEconomySchedule { business: BusinessId },
    #[error("business {business} has invalid ownership history")]
    InvalidBusinessOwnershipHistory { business: BusinessId },
    #[error("business cycle {cycle} has invalid economics or provenance")]
    InvalidBusinessCycle { cycle: BusinessCycleId },
}

mod business;
mod decisions;
mod legal;
mod operations;
mod opportunities;
mod recruitment;
mod world;

use self::business::{validate_business_economies, validate_enterprises};
use self::decisions::{validate_decisions, validate_delegation};
use self::legal::validate_legal_subsystems;
use self::operations::validate_operations;
use self::opportunities::validate_opportunities;
use self::recruitment::{validate_recruitment, validate_recruitment_against_registry};
use self::world::{validate_contacts, validate_social_and_intelligence, validate_world_state};

pub fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_id_allocators(state)?;
    validate_indexes(state)?;
    validate_world_state(state)?;
    validate_social_and_intelligence(state)?;
    validate_contacts(state)?;
    validate_recruitment(state)?;
    validate_operations(state)?;
    validate_opportunities(state)?;
    validate_decisions(state)?;
    validate_delegation(state)?;
    validate_business_economies(state)?;
    validate_enterprises(state)?;
    validate_reputations(state)?;
    validate_legal_subsystems(state)?;
    Ok(())
}

/// Reputation records are sparse and clamped at every mutation; the structural checks here
/// guard the persisted surface against drift the clamp path cannot produce.
fn validate_reputations(state: &AppState) -> Result<(), StateValidationError> {
    if !state.reputation.has_consistent_indexes() {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "reputation",
        });
    }
    for record in state.reputation.records() {
        let entity = crate::core::entity::EntityRef::Organization(record.organization());
        if !is_entity_present(state, entity) {
            return Err(StateValidationError::MissingEntity {
                context: "reputation record",
                entity,
            });
        }
        for dimension in crate::reputation::ALL_REPUTATION_DIMENSIONS {
            if record.score(dimension) > 100 {
                return Err(StateValidationError::InvalidReputationScore {
                    organization: record.organization(),
                    audience: record.audience(),
                });
            }
        }
    }
    Ok(())
}

fn validate_id_allocators(state: &AppState) -> Result<(), StateValidationError> {
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

/// Registry-relative re-derivation: recomputes every authored-content-dependent value from
/// the live registry and requires persisted state to agree. Split per domain so each
/// re-derivation contract stays independently readable.
pub fn validate_state_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    validate_operations_against_registry(registry, state)?;
    validate_opportunities_against_registry(registry, state)?;
    validate_investigation_work_against_registry(registry, state)?;
    validate_business_cycles_against_registry(registry, state)?;
    validate_enterprises_against_registry(registry, state)?;
    validate_recruitment_against_registry(registry, state)?;
    Ok(())
}

fn validate_operations_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for operation in state.operations.operations() {
        let definition = registry.get_operation(operation.kind());
        let execution = definition.execution();
        let has_police_entry_contingency = operation
            .contingencies()
            .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry);
        let police_response_matches_authorship =
            operation.police_response().is_none_or(|response| {
                state
                    .legal
                    .get_police_response(response)
                    .is_some_and(|response| {
                        let delay = resolve_police_arrival_delay(
                            execution,
                            response.response_presence().value(),
                        );
                        response.alert_score() >= execution.police_dispatch_threshold()
                            && response.arrival_due_at()
                                == response.dispatched_at()
                                    + crate::core::time::SimDuration::from_minutes(delay)
                    })
            });
        if !definition
            .supported_approaches()
            .contains(&operation.approach())
            || definition
                .required_roles()
                .iter()
                .any(|role| !operation.roles().contains_key(role))
            || operation
                .roles()
                .keys()
                .any(|role| execution.capability_for_role(*role).is_none())
            || operation.intelligence().iter().any(|information| {
                state
                    .intelligence
                    .get_information(*information)
                    .is_none_or(|record| {
                        !execution
                            .relevant_intelligence_topics()
                            .contains(&record.topic())
                    })
            })
            || (has_police_entry_contingency && execution.operation_entry_offset().is_none())
            || (execution.operation_entry_offset().is_none() && operation.entry_at().is_some())
            || (operation.started_at().is_some()
                && execution.operation_entry_offset().is_some()
                && operation.entry_at().is_none())
            || !police_response_matches_authorship
        {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
        if let Some(resolution) = operation.resolution() {
            let factors = resolution.factors();
            let expected_margin = resolve_execution_margin(execution, factors);
            let expected_outcome = resolve_objective_outcome(execution, expected_margin);
            let (
                expected_intelligence_quality,
                expected_intelligence_adjustment,
                expected_intelligence_topics_covered,
                expected_intelligence_topics_relevant,
            ) = resolve_intelligence_factors(registry, state, operation.id());
            let expected_police_response_arrived =
                has_police_response_arrived_by(state, operation, resolution.resolved_at());
            let expected_property_proceeds = resolve_property_proceeds(
                registry,
                state,
                operation,
                resolution.objective_outcome(),
            )
            .map_err(|_| StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            })?;
            if factors.variance().unsigned_abs() > execution.variance_limit()
                || factors.time_pressure()
                    > crate::operations::operation_execution::MAX_TIME_PRESSURE
                || factors.approach_adjustment()
                    != execution
                        .approach_difficulty_adjustment(operation.approach())
                        .expect("validated operation approach must have an execution adjustment")
                || factors.intelligence_quality() != expected_intelligence_quality
                || factors.intelligence_adjustment() != expected_intelligence_adjustment
                || factors.intelligence_topics_covered() != expected_intelligence_topics_covered
                || factors.intelligence_topics_relevant() != expected_intelligence_topics_relevant
                || factors.intelligence_topics_covered() > factors.intelligence_topics_relevant()
                || factors.police_response_arrived() != expected_police_response_arrived
                || resolution.execution_margin() != expected_margin
                || resolution.objective_outcome() != expected_outcome
                || resolution.property_proceeds() != expected_property_proceeds.proceeds
            {
                return Err(StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                });
            }
            if let Some(disposition) = operation.property_disposition() {
                let proceeds = resolution.property_proceeds().ok_or(
                    StateValidationError::InvalidOperationPropertyDisposition {
                        operation: operation.id(),
                    },
                )?;
                let expected_realized = resolve_property_liquidation_value(
                    registry,
                    state,
                    operation.kind(),
                    proceeds.estimated_value(),
                    operation.id(),
                    disposition.venue(),
                )
                .map_err(|_| {
                    StateValidationError::InvalidOperationPropertyDisposition {
                        operation: operation.id(),
                    }
                })?;
                if disposition.realized_value() != expected_realized {
                    return Err(StateValidationError::InvalidOperationPropertyDisposition {
                        operation: operation.id(),
                    });
                }
            }

            let exposure = resolution.exposure();
            let exposure_factors = exposure.factors();
            let expected_intelligence_mitigation =
                u16::from(factors.intelligence_quality().value())
                    .saturating_mul(u16::from(execution.intelligence_mitigation_weight()))
                    / 100;
            let expected_exposure_score = resolve_exposure_score(execution, exposure_factors);
            let expected_exposure_level =
                resolve_exposure_level(execution, expected_exposure_score);
            if exposure_factors.variance().unsigned_abs() > execution.exposure_variance_limit()
                || exposure_factors.approach_adjustment()
                    != execution
                        .exposure_approach_adjustment(operation.approach())
                        .expect("validated operation approach must have an exposure adjustment")
                || exposure_factors.intelligence_mitigation()
                    != u8::try_from(expected_intelligence_mitigation)
                        .expect("bounded exposure intelligence mitigation must fit u8")
                || exposure_factors.police_response_arrived() != expected_police_response_arrived
                || exposure.score() != expected_exposure_score
                || exposure.level() != expected_exposure_level
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
            if let Some(evidence_id) = exposure.evidence().iter().next() {
                let evidence = state.legal.get_evidence(*evidence_id).ok_or(
                    StateValidationError::InvalidOperationExposure {
                        operation: operation.id(),
                    },
                )?;
                if evidence.kind() != execution.exposure_evidence_kind() {
                    return Err(StateValidationError::InvalidOperationExposure {
                        operation: operation.id(),
                    });
                }
            }
        }
    }

    Ok(())
}

fn validate_opportunities_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for opportunity in state.opportunities.opportunities() {
        let context = opportunity.context().operation();
        let definition = registry.get_operation(context.operation_kind());
        let report = state.reports.get_report(opportunity.report()).ok_or(
            StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            },
        )?;
        if report.title()
            != crate::opportunities::opportunity_system::discovery_report_title(
                definition.display_name(),
            )
        {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }
        if let Some(OpportunityResolution::Expired {
            report: expiry_report,
            ..
        }) = opportunity.resolution()
        {
            let report = state.reports.get_report(expiry_report).ok_or(
                StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                },
            )?;
            if report.title()
                != crate::opportunities::opportunity_system::expiry_report_title(
                    definition.display_name(),
                )
            {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
        }
    }

    Ok(())
}

fn validate_investigation_work_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for work in state.legal.investigation_work() {
        let definition = registry.get_investigation_work(work.kind());
        if work.due_at() != work.scheduled_at() + definition.duration() {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        let Some(resolution) = work.resolution() else {
            continue;
        };
        let factors = resolution.factors();
        let (expected_factors, expected_margin) =
            resolve_work_factors_and_margin(definition, state, work, factors.variance())
                .map_err(|_| StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        if factors != expected_factors
            || factors.variance().unsigned_abs() > definition.variance_limit()
            || resolution.margin() != expected_margin
        {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        match resolution.outcome() {
            InvestigationWorkOutcome::Connected => {
                // Only a witness interview resolves as Connected; its economics were
                // re-derived above and its testimony provenance is checked in
                // `invariants::legal`.
                if work.kind() != InvestigationWorkKind::WitnessInterview
                    || expected_margin < definition.connected_margin()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkOutcome::Developed => {
                if work.kind() != InvestigationWorkKind::EvidenceReview
                    || expected_margin < definition.connected_margin()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                legal_invariants::validate_developed_review_evidence(
                    state,
                    work,
                    &mut BTreeSet::new(),
                )?;
            }
            InvestigationWorkOutcome::Inconclusive => {
                if expected_margin >= definition.connected_margin()
                    || resolution.derived_evidence().is_some()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
        }
    }

    Ok(())
}

fn validate_business_cycles_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for cycle in state.economy.cycles() {
        let business = state
            .world
            .get_business(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let economics = registry.get_business(business.kind()).economics();
        // Notability must agree with the production rule in `business_economy_system`: a
        // notable variance or a net-losing settlement is accountant-worthy.
        let variance = i32::from(cycle.variance_basis_points()).unsigned_abs();
        let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
            || cycle.net_cash() < crate::finance::Money::ZERO
        {
            AttentionClass::Notable
        } else {
            AttentionClass::Routine
        };
        if variance > u32::from(economics.gross_variance_basis_points())
            || cycle.attention() != expected_attention
        {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
    }

    Ok(())
}

fn validate_enterprises_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for enterprise in state.enterprises.enterprises() {
        let definition = registry.get_enterprise(enterprise.kind());
        let mut network_functions = BTreeSet::new();
        if let EnterpriseLocation::Business(business_id) = enterprise.location() {
            let business = state.world.get_business(business_id).ok_or(
                StateValidationError::InvalidEnterpriseLocation {
                    enterprise: enterprise.id(),
                },
            )?;
            for function in definition.required_business_functions() {
                if !business.has_function(*function) {
                    return Err(StateValidationError::EnterpriseBusinessRequirementMissing {
                        enterprise: enterprise.id(),
                        business: business_id,
                        function: *function,
                    });
                }
            }
            network_functions.extend(business.functions().iter().copied());
        } else if !definition.required_business_functions().is_empty() {
            return Err(StateValidationError::InvalidEnterpriseLocation {
                enterprise: enterprise.id(),
            });
        }
        for business_id in enterprise.supporting_businesses() {
            let business = state.world.get_business(*business_id).ok_or(
                StateValidationError::InvalidEnterpriseSupportingBusiness {
                    enterprise: enterprise.id(),
                    business: *business_id,
                },
            )?;
            network_functions.extend(business.functions().iter().copied());
        }
        for function in definition.required_network_functions() {
            if !network_functions.contains(function) {
                return Err(StateValidationError::EnterpriseNetworkRequirementMissing {
                    enterprise: enterprise.id(),
                    function: *function,
                });
            }
        }
    }
    for cycle in state.enterprises.cycles() {
        let enterprise = state
            .enterprises
            .get_enterprise(cycle.enterprise())
            .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
        let economics = registry.get_enterprise(enterprise.kind()).economics();
        let variance = i32::from(cycle.variance_basis_points()).unsigned_abs();
        // Notability must agree with the production rule in `enterprise_execution`: a notable
        // variance, a net-losing settlement, a drawn vice inquiry, or street heat that appeared
        // or changed since the previous settlement makes the manager's cycle report
        // player-visible. A sustained identical surcharge is a known cost and settles as
        // routine. Heat is read from the committed cycles rather than recomputed, because the
        // investigations that produced it may since have closed; it must still be a whole
        // number of the authored per-case surcharge.
        let per_case = economics.heat_surcharge_per_active_case().cents();
        if cycle.investigation_heat().cents() < 0
            || (per_case == 0 && cycle.investigation_heat().cents() != 0)
            || (per_case > 0 && cycle.investigation_heat().cents() % per_case != 0)
        {
            return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
        }
        // Settlement order is sequential-ID order, so the prior settlement is the highest
        // indexed cycle ID below this one; walking every earlier cycle per cycle would be
        // quadratic in settled history.
        let previous_heat = state
            .enterprises
            .prior_cycle(cycle.enterprise(), cycle.id())
            .map(|prior| prior.investigation_heat());
        let heat_reportable = cycle.investigation_heat() > crate::finance::Money::ZERO
            && previous_heat != Some(cycle.investigation_heat());
        let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
            || heat_reportable
            || cycle.net_cash() < crate::finance::Money::ZERO
            || cycle.drew_vice_attention()
        {
            AttentionClass::Notable
        } else {
            AttentionClass::Routine
        };
        if variance > u32::from(economics.gross_variance_basis_points())
            || cycle.attention() != expected_attention
        {
            return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
        }
    }
    Ok(())
}

fn validate_indexes(state: &AppState) -> Result<(), StateValidationError> {
    let checks = [
        ("world", state.world.has_consistent_indexes()),
        ("social", state.social.has_consistent_indexes()),
        ("intelligence", state.intelligence.has_consistent_indexes()),
        ("contacts", state.contacts.has_consistent_indexes()),
        ("recruitment", state.recruitment.has_consistent_indexes()),
        ("operations", state.operations.has_consistent_indexes()),
        (
            "opportunities",
            state.opportunities.has_consistent_indexes(),
        ),
        ("decisions", state.decisions.has_consistent_indexes()),
        ("delegation", state.delegation.has_consistent_indexes()),
        ("economy", state.economy.has_consistent_indexes()),
        ("enterprises", state.enterprises.has_consistent_indexes()),
        ("legal", state.legal.has_consistent_indexes()),
        ("reports", state.reports.has_consistent_indexes()),
    ];
    for (subsystem, is_consistent) in checks {
        if !is_consistent {
            return Err(StateValidationError::IndexInconsistency { subsystem });
        }
    }
    validate_finance_indexes_and_ledger(state)?;
    Ok(())
}

/// Finance ownership, ledger, and balance coherence in ONE pass over the append-only
/// transaction history: every per-transaction index-membership, posting, arithmetic, and
/// budget-authority check runs while the referenced-account set and derived balances are
/// accumulated, so per-tick validation walks campaign-length history once instead of once
/// per concern. Posting-level membership and balance accumulation use dense vectors keyed
/// by raw account id (ids are allocated monotonically), keeping the hottest loop free of
/// ordered-map traversals without weakening any check.
fn validate_finance_indexes_and_ledger(state: &AppState) -> Result<(), StateValidationError> {
    // Forward membership plus exact-count agreement proves bidirectional index coherence:
    // ids are unique and each account or transaction occupies at most one slot per key, so
    // matching entry totals rule out stale, duplicate, or foreign index entries.
    let mut account_count = 0_usize;
    for account in state.finance.accounts() {
        let owner = account.owner().entity();
        if !is_entity_present(state, owner) {
            return Err(StateValidationError::MissingEntity {
                context: "financial account owner",
                entity: owner,
            });
        }
        if !state
            .finance
            .account_is_indexed_for_owner(account.id(), account.owner())
        {
            return Err(StateValidationError::IndexInconsistency {
                subsystem: "finance",
            });
        }
        account_count += 1;
    }
    if state.finance.indexed_account_entries() != account_count {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "finance",
        });
    }

    // Dense scratch keyed by raw account id; sized by the largest persisted account so any
    // posting beyond it is the missing-entity rejection below rather than an allocation.
    let highest_account = state
        .finance
        .account_id_bounds()
        .map_or(0, |(_, highest)| highest) as usize;
    let mut account_present = vec![false; highest_account + 1];
    let mut derived_balance_cents = vec![0_i64; highest_account + 1];
    let mut seen_posted = vec![false; highest_account + 1];
    let mut seen_posted_count = 0_usize;
    for account in state.finance.accounts() {
        account_present[account.id().raw() as usize] = true;
    }

    let mut expected_mandate_entries = 0_usize;
    for transaction in state.finance.transactions() {
        if transaction.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "ledger transaction",
            });
        }
        let mut net_cents = 0_i64;
        for posting in transaction.postings() {
            let raw = posting.account.raw() as usize;
            if raw >= account_present.len() || !account_present[raw] {
                return Err(StateValidationError::MissingEntity {
                    context: "ledger posting account",
                    entity: EntityRef::FinancialAccount(posting.account),
                });
            }
            net_cents = net_cents.checked_add(posting.amount.cents()).ok_or(
                StateValidationError::LedgerArithmeticOverflow {
                    transaction: transaction.id(),
                },
            )?;
            derived_balance_cents[raw] = derived_balance_cents[raw]
                .checked_add(posting.amount.cents())
                .ok_or(StateValidationError::FinancialBalanceMismatch)?;
            // Distinct-account tally for the exact posted-index set comparison below.
            if !seen_posted[raw] {
                seen_posted[raw] = true;
                seen_posted_count += 1;
            }
        }
        if net_cents != 0 {
            return Err(StateValidationError::UnbalancedLedgerTransaction {
                transaction: transaction.id(),
                net_cents,
            });
        }
        if let Some(usage) = transaction.budget_usage() {
            expected_mandate_entries += 1;
            if !state
                .finance
                .transaction_is_indexed_for_mandate(transaction.id(), usage.mandate())
            {
                return Err(StateValidationError::IndexInconsistency {
                    subsystem: "finance",
                });
            }
            let mandate = state.delegation.get_mandate(usage.mandate()).ok_or(
                StateValidationError::MissingEntity {
                    context: "ledger budget mandate",
                    entity: EntityRef::Mandate(usage.mandate()),
                },
            )?;
            if state.world.get_character(usage.manager()).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "ledger budget manager",
                    entity: EntityRef::Character(usage.manager()),
                });
            }
            if state.finance.get_account(usage.funding_account()).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "ledger budget funding account",
                    entity: EntityRef::FinancialAccount(usage.funding_account()),
                });
            }
            let expected_outflow = usage.amount().cents().checked_neg();
            let matching_posting = expected_outflow.is_some_and(|expected| {
                transaction.postings().iter().any(|posting| {
                    posting.account == usage.funding_account() && posting.amount.cents() == expected
                })
            });
            if usage.amount().cents() <= 0
                || mandate.manager() != usage.manager()
                || usage.mandate_version() == 0
                || usage.mandate_version() > mandate.version()
                || (usage.mandate_version() == mandate.version()
                    && !mandate.scopes().contains(&usage.scope()))
                || usage.period_start() >= usage.period_end()
                || transaction.occurred_at() < usage.period_start()
                || transaction.occurred_at() >= usage.period_end()
                || !matching_posting
            {
                return Err(StateValidationError::InvalidBudgetUsage {
                    transaction: transaction.id(),
                });
            }
        }
    }
    // Exact set agreement between accounts referenced by postings and the maintained
    // posted-accounts index: every indexed id must have been seen, and equal distinct
    // totals rule out any seen-but-unindexed reference.
    let posted_snapshot = state.finance.posted_accounts_snapshot();
    let snapshot_all_seen = posted_snapshot.iter().all(|id| {
        let raw = id.raw() as usize;
        raw < seen_posted.len() && seen_posted[raw]
    });
    if seen_posted_count != posted_snapshot.len() || !snapshot_all_seen {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "finance",
        });
    }
    if state.finance.indexed_mandate_entries() != expected_mandate_entries {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "finance",
        });
    }
    if !state
        .finance
        .balances_agree_with_derived_cents(&derived_balance_cents)
    {
        return Err(StateValidationError::FinancialBalanceMismatch);
    }
    Ok(())
}

/// Full structural validation across every subsystem. This is a debug-boundary tool: the
/// whole body compiles out of release builds, where save/load and observation boundaries own
/// validation (see STATUS.md). Release builds pay none of this per tick.
#[cfg(debug_assertions)]
pub fn validate_invariants(state: &AppState) {
    debug_assert_eq!(
        state.state_schema_version(),
        CURRENT_STATE_SCHEMA_VERSION,
        "Serialization Completeness: in-memory state schema version is not current"
    );

    // The release-safe structural validators are the single source of truth for record,
    // lifecycle, provenance, index, and balance coherence — including every subsystem's
    // derived-index consistency (`validate_indexes`), finance balance agreement, and the
    // reputation index check inside `validate_reputations`. Keep them authoritative here
    // instead of re-running reduced-fidelity copies alongside them, which has historically
    // drifted from the release-safe checks (for example, the supervision-cycle walk must
    // detect multi-character cycles rather than only self-reference).
    if let Err(error) = validate_state(state) {
        panic!("State Runtime Validity: release-safe structural validation failed: {error:?}");
    }
}

/// Release builds pay none of the debug-boundary validation per tick; save/load and
/// observation boundaries own validation there via [`validate_state`].
#[cfg(not(debug_assertions))]
pub fn validate_invariants(_state: &AppState) {}
