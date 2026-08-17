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
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::decisions::DecisionResponse;
use crate::delegation::{MandateStatus, ResponsibilityScope};
use crate::enterprises::EnterpriseLocation;
use crate::finance::{AccountLifecycle, FinancialOwner};
use crate::intelligence::{InformationSourceKind, KnowledgeHolder};
use crate::legal::investigation_work_execution::{
    calculate_work_factors_and_margin, derive_pattern_admissibility, derive_pattern_strength,
    find_superseding_evidence, improve_evidence_reliability, minimum_source_reliability,
};
use crate::legal::patrol_system::is_canonical_patrol_schedule;
use crate::legal::{
    EvidenceKind, InformantStatus, InvestigationStatus, InvestigationWorkKind,
    InvestigationWorkOutcome, PatrolDeploymentStatus,
};
use crate::operations::operation_execution::{
    calculate_execution_margin, calculate_exposure_score, calculate_intelligence_factors,
    calculate_property_proceeds, classify_exposure_level, classify_objective_outcome,
    did_police_response_arrive_by,
};
use crate::operations::property_disposition::calculate_property_liquidation_value;
use crate::operations::{OperationConstraint, OperationContingency, OperationStatus};
use crate::opportunities::OpportunityResolution;
use crate::registry::Registry;
use crate::world::{
    BusinessFunction, BusinessOwner, CapabilityKind, Lifecycle, OrganizationKind, PolicyKind,
    ALL_POLICY_KINDS,
};
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
    #[error("active operation {operation} belongs to an inactive organization")]
    ActiveOperationInactiveOrganization { operation: OperationId },
    #[error("active operation {operation} has an inactive or foreign leader")]
    ActiveOperationInvalidLeader { operation: OperationId },
    #[error("active operation {operation} has inactive participant {participant}")]
    ActiveOperationInvalidParticipant {
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
    #[error("active mandate {mandate} budget account {account} is not open")]
    ActiveMandateBudgetAccountNotOpen {
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
use self::legal::validate_legal_reports_and_history;
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
    validate_legal_reports_and_history(state)?;
    Ok(())
}

fn validate_id_allocators(state: &AppState) -> Result<(), StateValidationError> {
    validate_id_allocator(
        &state.ids,
        IdKind::Organization,
        state.world.organizations().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Character,
        state.world.characters().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Neighborhood,
        state.world.neighborhoods().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Business,
        state.world.businesses().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::BusinessOwnershipChange,
        state
            .world
            .business_ownership_changes()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Operation,
        state
            .operations
            .operations()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Opportunity,
        state
            .opportunities
            .opportunities()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Information,
        state
            .intelligence
            .information()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Contact,
        state.contacts.contacts().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ContactDisclosure,
        state.contacts.disclosures().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Investigation,
        state.legal.investigations().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::InvestigationWork,
        state
            .legal
            .investigation_work()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::PatrolDeployment,
        state
            .legal
            .patrol_deployments()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::PoliceResponse,
        state
            .legal
            .police_responses()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::CaseWitness,
        state.legal.case_witnesses().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::WitnessStatement,
        state
            .legal
            .witness_statements()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Informant,
        state.legal.informants().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::InformantDisclosure,
        state
            .legal
            .informant_disclosures()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Evidence,
        state.legal.all_evidence().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Arrest,
        state.legal.arrests().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::LegalRepresentation,
        state
            .legal
            .legal_representations()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ProsecutionCase,
        state
            .legal
            .prosecution_cases()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::ProsecutionReferral,
        state
            .legal
            .prosecution_referrals()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Report,
        state.reports.reports().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::HistoryEvent,
        state.history.events().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::FinancialAccount,
        state.finance.accounts().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::LedgerTransaction,
        state.finance.transactions().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::DecisionRequest,
        state.decisions.decisions().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Mandate,
        state.delegation.mandates().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::RecruitmentAttempt,
        state.recruitment.attempts().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::Enterprise,
        state
            .enterprises
            .enterprises()
            .map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::EnterpriseCycle,
        state.enterprises.cycles().map(|record| record.id().raw()),
    )?;
    validate_id_allocator(
        &state.ids,
        IdKind::BusinessCycle,
        state.economy.cycles().map(|record| record.id().raw()),
    )?;
    Ok(())
}

fn validate_id_allocator(
    counters: &IdCounters,
    kind: IdKind,
    ids: impl Iterator<Item = u32>,
) -> Result<(), StateValidationError> {
    let mut highest = 0;
    for id in ids {
        if id == 0 {
            return Err(StateValidationError::InvalidPersistentId { kind: kind.label() });
        }
        highest = highest.max(id);
    }
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

pub fn validate_state_against_registry(
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
                        let reduction =
                            u32::from(response.response_presence().value()).saturating_mul(
                                u32::from(execution.patrol_response_reduction_minutes()),
                            ) / 100;
                        let delay = execution
                            .base_police_response_delay()
                            .as_minutes()
                            .saturating_sub(reduction)
                            .max(execution.minimum_police_response_delay().as_minutes());
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
            let expected_margin = calculate_execution_margin(execution, factors);
            let expected_outcome = classify_objective_outcome(execution, expected_margin);
            let (
                expected_intelligence_quality,
                expected_intelligence_adjustment,
                expected_intelligence_topics_covered,
                expected_intelligence_topics_relevant,
            ) = calculate_intelligence_factors(registry, state, operation.id());
            let expected_police_response_arrived =
                did_police_response_arrive_by(state, operation, resolution.resolved_at());
            let expected_property_proceeds = calculate_property_proceeds(
                registry,
                state,
                operation,
                resolution.objective_outcome(),
            )
            .map_err(|_| StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            })?;
            if factors.variance().unsigned_abs() > execution.variance_limit()
                || factors.time_pressure() > 30
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
                || resolution.property_proceeds() != expected_property_proceeds
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
                let expected_realized = calculate_property_liquidation_value(
                    registry,
                    operation.kind(),
                    proceeds.estimated_value(),
                    operation.id(),
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
            let expected_exposure_score = calculate_exposure_score(execution, exposure_factors);
            let expected_exposure_level =
                classify_exposure_level(execution, expected_exposure_score);
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
    for opportunity in state.opportunities.opportunities() {
        let context = opportunity.context().operation();
        let definition = registry.get_operation(context.operation_kind());
        let report = state.reports.get_report(opportunity.report()).ok_or(
            StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            },
        )?;
        if report.title() != format!("{} opportunity", definition.display_name()) {
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
            if report.title() != format!("{} opportunity expired", definition.display_name()) {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
        }
    }
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
            calculate_work_factors_and_margin(definition, state, work, factors.variance())
                .map_err(|_| StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        if factors != expected_factors
            || factors.variance().unsigned_abs() > definition.variance_limit()
            || resolution.margin() != expected_margin
        {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        let expected_superseded_by = find_superseding_evidence(state, work);
        match resolution.outcome() {
            InvestigationWorkOutcome::Connected => {
                if work.kind() != InvestigationWorkKind::PatternAnalysis
                    || expected_margin < definition.connected_margin()
                    || expected_superseded_by.is_some()
                    || resolution.superseded_by().is_some()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                let evidence = state
                    .legal
                    .get_evidence(resolution.derived_evidence().ok_or(
                        StateValidationError::InvalidInvestigationWork { work: work.id() },
                    )?)
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                let expected_reliability =
                    minimum_source_reliability(state, work).map_err(|_| {
                        StateValidationError::InvalidInvestigationWork { work: work.id() }
                    })?;
                if evidence.strength() != derive_pattern_strength(factors.source_support())
                    || evidence.reliability() != expected_reliability
                    || evidence.admissibility() != derive_pattern_admissibility(state, work)
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkOutcome::Developed => {
                if work.kind() != InvestigationWorkKind::EvidenceReview
                    || expected_margin < definition.connected_margin()
                    || expected_superseded_by.is_some()
                    || resolution.superseded_by().is_some()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                let source_id = work
                    .focus()
                    .evidence_id()
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                let source = state
                    .legal
                    .get_evidence(source_id)
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                let evidence = state
                    .legal
                    .get_evidence(resolution.derived_evidence().ok_or(
                        StateValidationError::InvalidInvestigationWork { work: work.id() },
                    )?)
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                if evidence.kind() != EvidenceKind::ForensicAnalysis
                    || evidence.subject() != source.subject()
                    || evidence.origin() != source.origin()
                    || evidence.strength() != source.strength()
                    || evidence.reliability() != improve_evidence_reliability(source.reliability())
                    || evidence.admissibility() != source.admissibility()
                    || evidence.derived_from() != &BTreeSet::from([source_id])
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkOutcome::Inconclusive => {
                if expected_margin >= definition.connected_margin()
                    || expected_superseded_by.is_some()
                    || resolution.superseded_by().is_some()
                    || resolution.derived_evidence().is_some()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkOutcome::Superseded => {
                if expected_superseded_by.is_none()
                    || resolution.superseded_by() != expected_superseded_by
                    || resolution.derived_evidence().is_some()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
        }
    }
    for business in state.world.businesses() {
        registry.get_business(business.kind());
    }
    for cycle in state.economy.cycles() {
        let business = state
            .world
            .get_business(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let economics = registry.get_business(business.kind()).economics();
        let variance = i32::from(cycle.variance_basis_points()).unsigned_abs();
        let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
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
        let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
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
    validate_recruitment_against_registry(registry, state)?;
    Ok(())
}

fn validate_indexes(state: &AppState) -> Result<(), StateValidationError> {
    let checks = [
        ("world", state.world.has_consistent_indexes()),
        ("finance", state.finance.has_consistent_indexes()),
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
        ("history", state.history.has_consistent_indexes()),
    ];
    for (subsystem, is_consistent) in checks {
        if !is_consistent {
            return Err(StateValidationError::IndexInconsistency { subsystem });
        }
    }

    for account in state.finance.accounts() {
        let owner = account.owner().entity();
        if !is_entity_present(state, owner) {
            return Err(StateValidationError::MissingEntity {
                context: "financial account owner",
                entity: owner,
            });
        }
    }
    for transaction in state.finance.transactions() {
        if transaction.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "ledger transaction",
            });
        }
        let mut net_cents = 0_i64;
        for posting in transaction.postings() {
            if state.finance.get_account(posting.account).is_none() {
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
        }
        if net_cents != 0 {
            return Err(StateValidationError::UnbalancedLedgerTransaction {
                transaction: transaction.id(),
                net_cents,
            });
        }
        if let Some(usage) = transaction.budget_usage() {
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
    if !state.finance.has_consistent_balances() {
        return Err(StateValidationError::FinancialBalanceMismatch);
    }
    Ok(())
}

pub fn validate_invariants(state: &AppState) {
    debug_assert_eq!(
        state.state_schema_version(),
        CURRENT_STATE_SCHEMA_VERSION,
        "Serialization Completeness: in-memory state schema version is not current"
    );
    debug_assert!(
        validate_id_allocators(state).is_ok(),
        "Serialization Completeness: persistent ID allocators are not ahead of stored records"
    );

    state.world.debug_validate_indexes();
    state.finance.debug_validate_indexes();
    state.social.debug_validate_indexes();
    state.intelligence.debug_validate_indexes();
    state.contacts.debug_validate_indexes();
    state.recruitment.debug_validate_indexes();
    state.operations.debug_validate_indexes();
    state.opportunities.debug_validate_indexes();
    state.decisions.debug_validate_indexes();
    state.delegation.debug_validate_indexes();
    state.economy.debug_validate_indexes();
    state.enterprises.debug_validate_indexes();
    state.legal.debug_validate_indexes();
    state.reports.debug_validate_indexes();
    state.history.debug_validate_indexes();
    debug_assert!(
        validate_world_state(state).is_ok(),
        "World Runtime Validity: hierarchy, business ownership, or world references are inconsistent"
    );
    debug_assert!(
        validate_business_economies(state).is_ok(),
        "Business Economy Runtime Validity: business schedules, accounts, cycles, or provenance are inconsistent"
    );
    debug_assert!(
        validate_enterprises(state).is_ok(),
        "Enterprise Runtime Validity: enterprise authority, schedules, accounts, or cycle history are inconsistent"
    );
    debug_assert!(
        validate_operations(state).is_ok(),
        "Operation Runtime Validity: operation lifecycle, schedules, after-action knowledge, or history are inconsistent"
    );
    debug_assert!(
        validate_contacts(state).is_ok(),
        "Contact Runtime Validity: institutional contact ownership, lifecycle, or disclosure provenance is inconsistent"
    );
    debug_assert!(
        validate_opportunities(state).is_ok(),
        "Opportunity Runtime Validity: provenance, expiry schedules, reports, or converted operation links are inconsistent"
    );
    debug_assert!(
        validate_recruitment(state).is_ok(),
        "Recruitment Runtime Validity: recruitment history, causal factors, cooldowns, or membership snapshots are inconsistent"
    );
    debug_assert!(
        validate_legal_reports_and_history(state).is_ok(),
        "Legal Runtime Validity: jurisdiction, staffing, investigative work, evidence provenance, reports, or history are inconsistent"
    );

    if let Some(player) = state.player_organization() {
        let organization = state
            .world
            .get_organization(player)
            .expect("Record Reference Validity: player organization does not exist");
        debug_assert_eq!(
            organization.kind(),
            OrganizationKind::Criminal,
            "Lifecycle Validity: player organization is not a criminal organization"
        );
    }

    for account in state.finance.accounts() {
        debug_assert!(
            is_entity_present(state, account.owner().entity()),
            "Record Reference Validity: financial account owner does not exist"
        );
        match account.lifecycle() {
            AccountLifecycle::Open | AccountLifecycle::Frozen | AccountLifecycle::Closed => {}
        }
    }
    for transaction in state.finance.transactions() {
        let mut net_cents = 0_i64;
        for posting in transaction.postings() {
            debug_assert!(
                state.finance.get_account(posting.account).is_some(),
                "Record Reference Validity: ledger posting account does not exist"
            );
            net_cents = net_cents
                .checked_add(posting.amount.cents())
                .expect("Transaction Atomicity: ledger posting sum overflowed");
        }
        debug_assert_eq!(
            net_cents, 0,
            "Transaction Atomicity: ledger transaction postings do not balance"
        );
        debug_assert!(
            transaction.occurred_at() <= state.now(),
            "Lifecycle Validity: ledger transaction occurs in the future"
        );
        if let Some(usage) = transaction.budget_usage() {
            let mandate = state
                .delegation
                .get_mandate(usage.mandate())
                .expect("Record Reference Validity: ledger budget mandate does not exist");
            debug_assert!(state.world.get_character(usage.manager()).is_some());
            debug_assert_eq!(mandate.manager(), usage.manager());
            debug_assert!(usage.mandate_version() > 0);
            debug_assert!(usage.mandate_version() <= mandate.version());
            if usage.mandate_version() == mandate.version() {
                debug_assert!(mandate.scopes().contains(&usage.scope()));
            }
            debug_assert!(
                state.finance.get_account(usage.funding_account()).is_some(),
                "Record Reference Validity: ledger budget funding account does not exist"
            );
            debug_assert!(
                usage.amount().cents() > 0,
                "Lifecycle Validity: ledger budget usage is not positive"
            );
            debug_assert!(
                usage.period_start() < usage.period_end()
                    && transaction.occurred_at() >= usage.period_start()
                    && transaction.occurred_at() < usage.period_end(),
                "Lifecycle Validity: ledger budget usage window is invalid"
            );
            let expected_outflow = usage
                .amount()
                .cents()
                .checked_neg()
                .expect("Lifecycle Validity: ledger budget usage cannot be negated");
            debug_assert!(
                transaction.postings().iter().any(|posting| {
                    posting.account == usage.funding_account()
                        && posting.amount.cents() == expected_outflow
                }),
                "Derived Data Consistency: ledger budget usage does not match funding posting"
            );
        }
    }

    for organization in state.world.organizations() {
        for kind in ALL_POLICY_KINDS {
            let setting = organization
                .policy(kind)
                .expect("Definition/Runtime Separation: organization is missing a registered policy setting");
            debug_assert_eq!(
                setting.kind(),
                kind,
                "Definition/Runtime Separation: policy key does not match policy value"
            );
        }
    }

    for character in state.world.characters() {
        if let Some(organization) = character.organization() {
            debug_assert!(
                state.world.get_organization(organization).is_some(),
                "Record Reference Validity: character organization does not exist"
            );
        }
        if let Some(supervisor) = character.supervisor() {
            let supervisor_record = state
                .world
                .get_character(supervisor)
                .expect("Record Reference Validity: character supervisor does not exist");
            debug_assert_eq!(
                supervisor_record.organization(),
                character.organization(),
                "Ownership Exclusivity: supervisor and direct report belong to different organizations"
            );
            debug_assert_ne!(
                supervisor,
                character.id(),
                "Record Reference Validity: character supervises itself"
            );
        }
        let mut cursor = character.supervisor();
        while let Some(current) = cursor {
            debug_assert_ne!(
                current,
                character.id(),
                "Ownership Exclusivity: supervision hierarchy contains a cycle"
            );
            cursor = state
                .world
                .get_character(current)
                .and_then(|record| record.supervisor());
        }
    }

    for business in state.world.businesses() {
        debug_assert!(
            state
                .world
                .get_neighborhood(business.neighborhood())
                .is_some(),
            "Record Reference Validity: business neighborhood does not exist"
        );
        match business.owner() {
            BusinessOwner::Independent => {}
            BusinessOwner::Organization(id) => debug_assert!(
                state.world.get_organization(id).is_some(),
                "Record Reference Validity: business organization owner does not exist"
            ),
            BusinessOwner::Character(id) => debug_assert!(
                state.world.get_character(id).is_some(),
                "Record Reference Validity: business character owner does not exist"
            ),
        }
    }

    for relationship in state.social.relationships() {
        debug_assert!(
            state.world.get_character(relationship.from()).is_some(),
            "Record Reference Validity: relationship source character does not exist"
        );
        debug_assert!(
            state.world.get_character(relationship.to()).is_some(),
            "Record Reference Validity: relationship target character does not exist"
        );
    }

    for information in state.intelligence.information() {
        match information.holder() {
            KnowledgeHolder::Character(id) => debug_assert!(
                state.world.get_character(id).is_some(),
                "Record Reference Validity: information holder character does not exist"
            ),
            KnowledgeHolder::Organization(id) => debug_assert!(
                state.world.get_organization(id).is_some(),
                "Record Reference Validity: information holder organization does not exist"
            ),
        }
        debug_assert!(
            is_entity_present(state, information.subject()),
            "Record Reference Validity: information subject does not exist"
        );
        if let Some(source) = information.source_entity() {
            debug_assert!(
                is_entity_present(state, source),
                "Record Reference Validity: information source entity does not exist"
            );
        }
        debug_assert!(
            information.observed_at() <= information.recorded_at(),
            "Lifecycle Validity: information was recorded before it was observed"
        );
        if information.source_kind() == InformationSourceKind::InternalReport {
            debug_assert!(
                information.derived_from().len() == 1 && information.source_entity().is_some(),
                "Knowledge Provenance: internal report must have exactly one source and a source entity"
            );
            let source = *information
                .derived_from()
                .iter()
                .next()
                .expect("internal report must have one provenance record");
            let source_record = state
                .intelligence
                .get_information(source)
                .expect("Knowledge Provenance: internal report source information is missing");
            debug_assert_eq!(
                information.source_entity(),
                Some(source_record.holder().entity()),
                "Knowledge Provenance: internal report source entity disagrees with source holder"
            );
            debug_assert_eq!(
                information.topic(),
                source_record.topic(),
                "Knowledge Provenance: internal report topic disagrees with source information"
            );
            debug_assert_eq!(
                information.subject(),
                source_record.subject(),
                "Knowledge Provenance: internal report subject disagrees with source information"
            );
            debug_assert_eq!(
                information.observed_at(),
                source_record.observed_at(),
                "Knowledge Provenance: internal report observation time disagrees with source information"
            );
            debug_assert_eq!(
                information.reliability(),
                source_record.reliability(),
                "Knowledge Provenance: internal report reliability disagrees with source information"
            );
            debug_assert_eq!(
                information.specificity(),
                source_record.specificity(),
                "Knowledge Provenance: internal report specificity disagrees with source information"
            );
            debug_assert_eq!(
                information.summary(),
                source_record.summary(),
                "Knowledge Provenance: internal report summary disagrees with source information"
            );
        } else if information.derived_from().is_empty() {
            debug_assert!(
                state
                    .contacts
                    .disclosure_for_information(information.id())
                    .is_none(),
                "Knowledge Provenance: original information must not be owned by a contact disclosure"
            );
        } else {
            debug_assert!(
                matches!(
                    information.source_kind(),
                    InformationSourceKind::PoliceContact
                        | InformationSourceKind::Lawyer
                        | InformationSourceKind::PoliticalContact
                        | InformationSourceKind::ProfessionalContact
                        | InformationSourceKind::Press
                ),
                "Knowledge Provenance: non-internal derived information must use a contact source kind"
            );
            debug_assert!(
                information.derived_from().len() == 1
                    && state
                        .contacts
                        .disclosure_for_information(information.id())
                        .is_some(),
                "Knowledge Provenance: contact-derived information must have one source and a disclosure record"
            );
            let source = *information
                .derived_from()
                .iter()
                .next()
                .expect("contact-derived information must have one provenance record");
            let source_record = state
                .intelligence
                .get_information(source)
                .expect("Knowledge Provenance: contact source information is missing");
            debug_assert!(matches!(
                source_record.holder(),
                KnowledgeHolder::Character(_)
            ));
            debug_assert_eq!(
                information.source_entity(),
                Some(source_record.holder().entity())
            );
            debug_assert_eq!(information.topic(), source_record.topic());
            debug_assert_eq!(information.subject(), source_record.subject());
            debug_assert_eq!(information.observed_at(), source_record.observed_at());
            debug_assert_eq!(information.reliability(), source_record.reliability());
            debug_assert_eq!(information.specificity(), source_record.specificity());
            debug_assert_eq!(information.summary(), source_record.summary());
        }
        for source in information.derived_from() {
            let source_record = state
                .intelligence
                .get_information(*source)
                .expect("Knowledge Provenance: derived information references missing source");
            debug_assert!(
                *source < information.id(),
                "Knowledge Provenance: information lineage must point to an earlier record"
            );
            debug_assert!(
                source_record.recorded_at() <= information.recorded_at(),
                "Knowledge Provenance: derived information predates its source record"
            );
        }
    }

    for operation in state.operations.operations() {
        let organization = state
            .world
            .get_organization(operation.responsible_organization())
            .expect("Record Reference Validity: operation organization does not exist");
        let leader = state
            .world
            .get_character(operation.leader())
            .expect("Record Reference Validity: operation leader does not exist");
        let requires_active_participants = match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => true,
            OperationStatus::Completed | OperationStatus::Aborted => false,
        };
        for participant in operation.roles().values() {
            let participant_record = state
                .world
                .get_character(*participant)
                .expect("Record Reference Validity: operation participant does not exist");
            if requires_active_participants {
                debug_assert_eq!(
                    participant_record.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active operation has inactive participant"
                );
            }
        }
        for entity in operation.objective().referenced_entities() {
            debug_assert!(
                is_entity_present(state, entity),
                "Record Reference Validity: operation objective references a missing entity"
            );
        }
        for constraint in operation.constraints() {
            match constraint {
                OperationConstraint::AvoidCasualties
                | OperationConstraint::DoNotHarmEmployees
                | OperationConstraint::AvoidFirearms
                | OperationConstraint::ProtectLeadershipIdentity
                | OperationConstraint::PreserveMerchandise
                | OperationConstraint::CompleteBefore(_) => {}
                OperationConstraint::ExcludeCharacter(id) => debug_assert!(
                    state.world.get_character(*id).is_some(),
                    "Record Reference Validity: operation constraint references a missing character"
                ),
            }
        }
        for contingency in operation.contingencies() {
            match contingency {
                OperationContingency::AbortOnPoliceArrivalBeforeEntry
                | OperationContingency::UseForceOnResistance
                | OperationContingency::UseSecondaryExitIfBlocked
                | OperationContingency::RequestDecisionOnUnexpectedCondition => {}
                OperationContingency::ContactIfDetained(id) => debug_assert!(
                    state.world.get_character(*id).is_some(),
                    "Record Reference Validity: operation contingency references a missing character"
                ),
            }
        }
        match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => {
                debug_assert_eq!(
                    organization.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active operation belongs to inactive organization"
                );
                debug_assert_eq!(
                    leader.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active operation has inactive leader"
                );
                debug_assert_eq!(
                    leader.organization(),
                    Some(operation.responsible_organization()),
                    "Ownership Exclusivity: active operation leader belongs to another organization"
                );
            }
            OperationStatus::Completed | OperationStatus::Aborted => {}
        }
    }

    let decision_validation = validate_decisions(state);
    debug_assert!(
        decision_validation.is_ok(),
        "Lifecycle Validity: decision subsystem failed release-safe validation: {decision_validation:?}"
    );

    for operation in state
        .operations
        .operations_with_status(OperationStatus::AwaitingDecision)
    {
        debug_assert!(
            state
                .decisions
                .pending_for_operation(operation.id())
                .is_some(),
            "No Lost Runtime State: operation awaiting input has no pending decision"
        );
    }

    for mandate in state.delegation.mandates() {
        let organization = state
            .world
            .get_organization(mandate.organization())
            .expect("Record Reference Validity: mandate organization does not exist");
        let manager = state
            .world
            .get_character(mandate.manager())
            .expect("Record Reference Validity: mandate manager does not exist");
        debug_assert_eq!(
            manager.organization(),
            Some(mandate.organization()),
            "Ownership Exclusivity: mandate manager belongs to another organization"
        );
        debug_assert!(
            !mandate.scopes().is_empty(),
            "Lifecycle Validity: mandate has no responsibility scopes"
        );
        for (kind, setting) in mandate.standing_orders() {
            debug_assert_eq!(
                setting.kind(),
                *kind,
                "Definition/Runtime Separation: mandate policy key does not match value"
            );
        }
        for scope in mandate.scopes() {
            match scope {
                ResponsibilityScope::Neighborhood(id) => debug_assert!(
                    state.world.get_neighborhood(*id).is_some(),
                    "Record Reference Validity: mandate neighborhood scope does not exist"
                ),
                ResponsibilityScope::Business(id) => debug_assert!(
                    state.world.get_business(*id).is_some(),
                    "Record Reference Validity: mandate business scope does not exist"
                ),
                ResponsibilityScope::Function(_) => {}
            }
        }
        let budget_account = mandate.budget().map(|budget| {
            debug_assert!(
                budget.limit.cents() >= 0,
                "Lifecycle Validity: mandate budget limit is negative"
            );
            let account = state
                .finance
                .get_account(budget.funding_account)
                .expect("Record Reference Validity: mandate budget account does not exist");
            debug_assert_eq!(
                account.owner(),
                FinancialOwner::Organization(mandate.organization()),
                "Ownership Exclusivity: mandate budget account belongs to another owner"
            );
            account
        });
        match mandate.status() {
            MandateStatus::Active => {
                debug_assert_eq!(
                    organization.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active mandate belongs to inactive organization"
                );
                debug_assert_eq!(
                    manager.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active mandate has inactive manager"
                );
                if let Some(account) = budget_account {
                    debug_assert_eq!(
                        account.lifecycle(),
                        AccountLifecycle::Open,
                        "Lifecycle Validity: active mandate budget account is not open"
                    );
                }
            }
            MandateStatus::Revoked => {}
        }
    }

    for investigation in state.legal.investigations() {
        debug_assert!(
            state
                .world
                .get_organization(investigation.owner())
                .is_some(),
            "Record Reference Validity: investigation owner does not exist"
        );
        for subject in investigation.subjects() {
            debug_assert!(
                is_entity_present(state, *subject),
                "Record Reference Validity: investigation subject does not exist"
            );
        }
        debug_assert!(
            investigation.version() > 0,
            "Lifecycle Validity: investigation is unversioned"
        );
        if let Some(lead) = investigation.lead_investigator() {
            debug_assert!(
                investigation.assigned_investigators().contains(&lead),
                "Derived Data Consistency: investigation lead is not assigned to the case"
            );
        }
        for investigator in investigation.assigned_investigators() {
            let character = state
                .world
                .get_character(*investigator)
                .expect("Record Reference Validity: assigned investigator does not exist");
            if investigation.status() == InvestigationStatus::Active {
                debug_assert_eq!(
                    character.lifecycle(),
                    Lifecycle::Active,
                    "Lifecycle Validity: active investigation has inactive investigator"
                );
                debug_assert_eq!(
                    character.organization(),
                    Some(investigation.owner()),
                    "Ownership Exclusivity: active investigation has foreign investigator"
                );
                debug_assert!(
                    character
                        .capability(CapabilityKind::Investigation)
                        .is_some(),
                    "Lifecycle Validity: active investigation has unqualified investigator"
                );
            }
        }
    }
    for informant in state.legal.informants() {
        let character = state
            .world
            .get_character(informant.character())
            .expect("Record Reference Validity: informant character does not exist");
        let handler = state
            .world
            .get_organization(informant.handler())
            .expect("Record Reference Validity: informant handler does not exist");
        debug_assert!(
            matches!(
                handler.kind(),
                OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
            ),
            "Ownership Exclusivity: informant handler is not a legal organization"
        );
        debug_assert!(
            informant.established_at() <= state.now() && informant.version() > 0,
            "Lifecycle Validity: informant relationship has invalid chronology or version"
        );
        match informant.status() {
            InformantStatus::Active => {
                debug_assert_eq!(character.lifecycle(), Lifecycle::Active);
                debug_assert_eq!(handler.lifecycle(), Lifecycle::Active);
                debug_assert_ne!(character.organization(), Some(informant.handler()));
                debug_assert!(informant.terminated_at().is_none());
            }
            InformantStatus::Terminated => {
                let terminated_at = informant
                    .terminated_at()
                    .expect("Lifecycle Validity: terminated informant is missing termination time");
                debug_assert!(terminated_at >= informant.established_at());
                debug_assert!(terminated_at <= state.now());
            }
        }
    }
    for disclosure in state.legal.informant_disclosures() {
        let informant = state
            .legal
            .get_informant(disclosure.informant())
            .expect("Record Reference Validity: informant disclosure has no relationship");
        let investigation = state
            .legal
            .get_investigation(disclosure.investigation())
            .expect("Record Reference Validity: informant disclosure has no investigation");
        let information = state
            .intelligence
            .get_information(disclosure.source_information())
            .expect("Record Reference Validity: informant disclosure has no source information");
        let evidence = state
            .legal
            .get_evidence(disclosure.evidence())
            .expect("Record Reference Validity: informant disclosure has no evidence");
        debug_assert_eq!(investigation.owner(), informant.handler());
        debug_assert_eq!(
            information.holder(),
            KnowledgeHolder::Character(informant.character())
        );
        debug_assert_eq!(evidence.kind(), EvidenceKind::InformantStatement);
        debug_assert_eq!(
            evidence.source(),
            Some(EntityRef::Character(informant.character()))
        );
        debug_assert_eq!(evidence.subject(), information.subject());
        debug_assert_eq!(evidence.discovered_at(), disclosure.disclosed_at());
        debug_assert!(disclosure.disclosed_at() >= informant.established_at());
        debug_assert!(informant
            .terminated_at()
            .is_none_or(|terminated_at| disclosure.disclosed_at() <= terminated_at));
    }
    for jurisdiction in state.legal.jurisdictions() {
        let organization = state
            .world
            .get_organization(jurisdiction.organization())
            .expect("Record Reference Validity: jurisdiction authority does not exist");
        debug_assert!(
            matches!(
                organization.kind(),
                OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
            ),
            "Ownership Exclusivity: legal jurisdiction belongs to non-legal organization"
        );
        debug_assert!(
            !jurisdiction.neighborhoods().is_empty() && jurisdiction.version() > 0,
            "Lifecycle Validity: legal jurisdiction is empty or unversioned"
        );
        for neighborhood in jurisdiction.neighborhoods() {
            debug_assert!(
                state.world.get_neighborhood(*neighborhood).is_some(),
                "Record Reference Validity: legal jurisdiction neighborhood does not exist"
            );
        }
    }
    for deployment in state.legal.patrol_deployments() {
        let authority = state
            .world
            .get_organization(deployment.organization())
            .expect("Record Reference Validity: patrol authority does not exist");
        let neighborhood = state
            .world
            .get_neighborhood(deployment.neighborhood())
            .expect("Record Reference Validity: patrol neighborhood does not exist");
        debug_assert_eq!(
            authority.kind(),
            OrganizationKind::LawEnforcement,
            "Ownership Exclusivity: patrol deployment belongs to non-law-enforcement organization"
        );
        debug_assert!(
            deployment.version() > 0
                && deployment.established_at() <= deployment.last_changed_at()
                && deployment.last_changed_at() <= state.now()
                && is_canonical_patrol_schedule(deployment.windows()),
            "Lifecycle Validity: patrol deployment has invalid schedule, chronology, or version"
        );
        match deployment.status() {
            PatrolDeploymentStatus::Active => {
                debug_assert_eq!(authority.lifecycle(), Lifecycle::Active);
                debug_assert_eq!(neighborhood.lifecycle(), Lifecycle::Active);
                debug_assert!(
                    state
                        .legal
                        .get_jurisdiction(deployment.organization())
                        .is_some_and(|jurisdiction| jurisdiction
                            .neighborhoods()
                            .contains(&deployment.neighborhood())),
                    "Ownership Exclusivity: active patrol is outside its authority's jurisdiction"
                );
                debug_assert_eq!(
                    state
                        .legal
                        .active_patrol_for(deployment.organization(), deployment.neighborhood())
                        .map(|record| record.id()),
                    Some(deployment.id()),
                    "Derived Data Consistency: active patrol uniqueness index disagrees with source record"
                );
            }
            PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired => {}
        }
    }
    for evidence in state.legal.all_evidence() {
        debug_assert!(
            state
                .legal
                .get_investigation(evidence.investigation())
                .is_some(),
            "Record Reference Validity: evidence investigation does not exist"
        );
        debug_assert!(
            state.world.get_organization(evidence.custodian()).is_some(),
            "Record Reference Validity: evidence custodian does not exist"
        );
        debug_assert!(
            is_entity_present(state, evidence.subject()),
            "Record Reference Validity: evidence subject does not exist"
        );
        if let Some(origin) = evidence.origin() {
            debug_assert!(
                is_entity_present(state, origin),
                "Record Reference Validity: evidence origin does not exist"
            );
        }
    }

    for report in state.reports.reports() {
        debug_assert!(
            state.world.get_organization(report.recipient()).is_some(),
            "Record Reference Validity: report recipient does not exist"
        );
        for entry in report.entries() {
            for source in &entry.sources {
                let information = state
                    .intelligence
                    .get_information(*source)
                    .expect("Record Reference Validity: report source information does not exist");
                match information.holder() {
                    KnowledgeHolder::Organization(organization) => debug_assert_eq!(
                        organization,
                        report.recipient(),
                        "Knowledge Boundary: report cites information held by another organization"
                    ),
                    KnowledgeHolder::Character(_) => debug_assert!(
                        false,
                        "Knowledge Boundary: persisted organization reports must cite organization-held information"
                    ),
                }
            }
            for entity in &entry.entities {
                debug_assert!(
                    is_entity_present(state, *entity),
                    "Record Reference Validity: report entity does not exist"
                );
            }
            if let Some(decision) = entry.decision {
                let decision_record = state
                    .decisions
                    .get_decision(decision)
                    .expect("Record Reference Validity: report decision does not exist");
                debug_assert_eq!(
                    decision_record.recipient(),
                    report.recipient(),
                    "Ownership Exclusivity: report references a decision for another recipient"
                );
            }
        }
    }

    for event in state.history.events() {
        for entity in event.entities() {
            debug_assert!(
                is_entity_present(state, *entity),
                "Record Reference Validity: history event entity does not exist"
            );
        }
    }
}
