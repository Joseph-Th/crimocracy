//! Runtime invariant enforcement and release-safe structural state validation.

use crate::contacts::contact_system::{expected_contact_kind, information_source_kind};
use crate::contacts::{ContactRelationshipSnapshot, ContactStatus};
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
use crate::core::time::SimTime;
use crate::decisions::{
    DecisionContext, DecisionResponse, DecisionStatus, OperationExceptionReason,
};
use crate::delegation::{MandateStatus, ResponsibilityFunction, ResponsibilityScope};
use crate::economy::BusinessOperatingStatus;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::{AccountKind, AccountLifecycle, FinancialOwner, Money};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::informant_system::{informant_reliability, informant_strength};
use crate::legal::investigation_work_execution::{
    calculate_work_factors_and_margin, derive_pattern_admissibility, derive_pattern_strength,
    find_superseding_evidence, improve_evidence_reliability, is_reviewable_evidence_kind,
    minimum_source_reliability, source_evidence_forms_simple_path,
};
use crate::legal::patrol_system::is_canonical_patrol_schedule;
use crate::legal::witness_system::{witness_reliability, witness_strength};
use crate::legal::{
    Admissibility, ArrestStatus, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InformantStatus, InvestigationStatus, InvestigationWorkFocus, InvestigationWorkKind,
    InvestigationWorkOutcome, InvestigationWorkStatus, LegalRepresentationStatus,
    PatrolDeploymentStatus, PoliceResponseStatus, ProsecutionCaseStatus, WitnessCooperation,
};
use crate::operations::operation_execution::{
    build_legal_activity_summary, calculate_execution_margin, calculate_exposure_score,
    calculate_intelligence_factors, calculate_property_proceeds, classify_exposure_level,
    classify_objective_outcome, did_police_response_arrive_by,
};
use crate::operations::operation_system::is_information_subject_relevant;
use crate::operations::property_disposition::{
    build_disposition_summary, calculate_property_liquidation_value,
};
use crate::operations::surveillance_integration::{
    expected_persisted_surveillance_signatures, is_supported_surveillance_target,
    is_valid_persisted_surveillance_information,
};
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationConstraint, OperationContingency,
    OperationExposureLevel, OperationKind, OperationObjective, OperationObjectiveOutcome,
    OperationRecord, OperationResolutionRecord, OperationStatus,
};
use crate::opportunities::OpportunityResolution;
use crate::recruitment::recruitment_system::{
    calculate_recruitment_factors_from_context, calculate_recruitment_margin,
    classify_recruitment_outcome, select_perceived_legal_pressure_at, RecruitmentFactorContext,
};
use crate::recruitment::{RecruitmentAuthority, RecruitmentOutcome, RecruitmentPolicySource};
use crate::registry::Registry;
use crate::reports::ReportKind;
use crate::world::{
    ApprovalPolicy, BusinessFunction, BusinessOwner, CapabilityKind, Lifecycle, OrganizationKind,
    PolicyKind, ALL_POLICY_KINDS,
};
use std::collections::{BTreeMap, BTreeSet};
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

fn validate_world_state(state: &AppState) -> Result<(), StateValidationError> {
    if let Some(player) = state.player_organization() {
        let organization =
            state
                .world
                .get_organization(player)
                .ok_or(StateValidationError::MissingEntity {
                    context: "player organization",
                    entity: EntityRef::Organization(player),
                })?;
        if organization.kind() != OrganizationKind::Criminal {
            return Err(StateValidationError::InvalidPlayerOrganization {
                organization: player,
            });
        }
    }

    for organization in state.world.organizations() {
        for policy in ALL_POLICY_KINDS {
            let setting =
                organization
                    .policy(policy)
                    .ok_or(StateValidationError::MissingPolicy {
                        organization: organization.id(),
                        policy,
                    })?;
            if setting.kind() != policy {
                return Err(StateValidationError::PolicyKindMismatch {
                    organization: organization.id(),
                    expected: policy,
                    actual: setting.kind(),
                });
            }
        }
    }

    for character in state.world.characters() {
        if let Some(organization) = character.organization() {
            if state.world.get_organization(organization).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "character organization",
                    entity: EntityRef::Organization(organization),
                });
            }
        }
        if let Some(supervisor) = character.supervisor() {
            let supervisor_record = state.world.get_character(supervisor).ok_or(
                StateValidationError::MissingEntity {
                    context: "character supervisor",
                    entity: EntityRef::Character(supervisor),
                },
            )?;
            if supervisor_record.organization() != character.organization() {
                return Err(StateValidationError::SupervisorOrganizationMismatch {
                    character: character.id(),
                    supervisor,
                });
            }
        }
        let mut visited = BTreeSet::new();
        let mut cursor = character.supervisor();
        while let Some(current) = cursor {
            if current == character.id() || !visited.insert(current) {
                return Err(StateValidationError::SupervisionCycle {
                    character: character.id(),
                });
            }
            cursor = state
                .world
                .get_character(current)
                .ok_or(StateValidationError::MissingEntity {
                    context: "supervision hierarchy",
                    entity: EntityRef::Character(current),
                })?
                .supervisor();
        }
    }

    for business in state.world.businesses() {
        if state
            .world
            .get_neighborhood(business.neighborhood())
            .is_none()
        {
            return Err(StateValidationError::MissingEntity {
                context: "business neighborhood",
                entity: EntityRef::Neighborhood(business.neighborhood()),
            });
        }
        let owner = match business.owner() {
            BusinessOwner::Independent => None,
            BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
            BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
        };
        if let Some(entity) = owner {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "business owner",
                    entity,
                });
            }
        }
        if business.version() == 0
            || state
                .world
                .get_business_ownership_change_for_version(business.id(), business.version())
                .is_none_or(|change| change.new_owner() != business.owner())
        {
            return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                business: business.id(),
            });
        }
        for change in state.world.business_ownership_history(business.id()) {
            if change.changed_at() > state.now() {
                return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                    business: business.id(),
                });
            }
            for historical_owner in [change.previous_owner(), Some(change.new_owner())]
                .into_iter()
                .flatten()
            {
                let entity = match historical_owner {
                    BusinessOwner::Independent => None,
                    BusinessOwner::Organization(id) => Some(EntityRef::Organization(id)),
                    BusinessOwner::Character(id) => Some(EntityRef::Character(id)),
                };
                if entity.is_some_and(|entity| !is_entity_present(state, entity)) {
                    return Err(StateValidationError::InvalidBusinessOwnershipHistory {
                        business: business.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_social_and_intelligence(state: &AppState) -> Result<(), StateValidationError> {
    for relationship in state.social.relationships() {
        for (context, entity) in [
            (
                "relationship source",
                EntityRef::Character(relationship.from()),
            ),
            (
                "relationship target",
                EntityRef::Character(relationship.to()),
            ),
        ] {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity { context, entity });
            }
        }
    }

    for information in state.intelligence.information() {
        match information.holder() {
            KnowledgeHolder::Character(id) => {
                if state.world.get_character(id).is_none() {
                    return Err(StateValidationError::MissingEntity {
                        context: "information holder",
                        entity: EntityRef::Character(id),
                    });
                }
            }
            KnowledgeHolder::Organization(id) => {
                if state.world.get_organization(id).is_none() {
                    return Err(StateValidationError::MissingEntity {
                        context: "information holder",
                        entity: EntityRef::Organization(id),
                    });
                }
            }
        }
        if !is_entity_present(state, information.subject()) {
            return Err(StateValidationError::MissingEntity {
                context: "information subject",
                entity: information.subject(),
            });
        }
        if let Some(source) = information.source_entity() {
            if !is_entity_present(state, source) {
                return Err(StateValidationError::MissingEntity {
                    context: "information source",
                    entity: source,
                });
            }
        }
        if information.observed_at() > information.recorded_at()
            || information.recorded_at() > state.now()
        {
            return Err(StateValidationError::InvalidInformationChronology {
                information: information.id(),
            });
        }
        if information.source_kind() == InformationSourceKind::InternalReport {
            if information.derived_from().len() != 1 || information.source_entity().is_none() {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: information.id(),
                });
            }
            let source = *information
                .derived_from()
                .iter()
                .next()
                .expect("validated internal report must have one provenance record");
            let source_record = state.intelligence.get_information(source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                },
            )?;
            if information.source_entity() != Some(source_record.holder().entity())
                || information.topic() != source_record.topic()
                || information.subject() != source_record.subject()
                || information.observed_at() != source_record.observed_at()
                || information.reliability() != source_record.reliability()
                || information.specificity() != source_record.specificity()
                || information.summary() != source_record.summary()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                });
            }
        } else if !information.derived_from().is_empty() {
            let valid_contact_kind = matches!(
                information.source_kind(),
                InformationSourceKind::PoliceContact
                    | InformationSourceKind::Lawyer
                    | InformationSourceKind::PoliticalContact
                    | InformationSourceKind::ProfessionalContact
                    | InformationSourceKind::Press
            );
            let source = information.derived_from().iter().next().copied().ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: information.id(),
                },
            )?;
            let source_record = state.intelligence.get_information(source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                },
            )?;
            if !valid_contact_kind
                || information.derived_from().len() != 1
                || state
                    .contacts
                    .disclosure_for_information(information.id())
                    .is_none()
                || information.source_entity() != Some(source_record.holder().entity())
                || !matches!(source_record.holder(), KnowledgeHolder::Character(_))
                || information.topic() != source_record.topic()
                || information.subject() != source_record.subject()
                || information.observed_at() != source_record.observed_at()
                || information.reliability() != source_record.reliability()
                || information.specificity() != source_record.specificity()
                || information.summary() != source_record.summary()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: source,
                });
            }
        }
        for source in information.derived_from() {
            let source_record = state.intelligence.get_information(*source).ok_or(
                StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: *source,
                },
            )?;
            if *source >= information.id()
                || source_record.recorded_at() > information.recorded_at()
            {
                return Err(StateValidationError::InvalidInformationProvenance {
                    information: information.id(),
                    source_information: *source,
                });
            }
        }
    }
    Ok(())
}

fn validate_contacts(state: &AppState) -> Result<(), StateValidationError> {
    for contact in state.contacts.contacts() {
        let sponsor = state.world.get_organization(contact.sponsor()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let handler = state.world.get_character(contact.handler()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let source = state.world.get_character(contact.contact()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        let institution = state.world.get_organization(contact.institution()).ok_or(
            StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            },
        )?;
        if sponsor.kind() != OrganizationKind::Criminal
            || expected_contact_kind(institution.kind()) != Some(contact.kind())
            || contact.handler() == contact.contact()
            || contact.version() == 0
            || contact.established_at() > state.now()
            || !contact_relationship_basis_is_valid(
                contact.handler(),
                contact.contact(),
                contact.handler_to_contact(),
                contact.contact_to_handler(),
            )
        {
            return Err(StateValidationError::InvalidInstitutionalContact {
                contact: contact.id(),
            });
        }
        match contact.status() {
            ContactStatus::Active => {
                if contact.terminated_at().is_some()
                    || sponsor.lifecycle() != Lifecycle::Active
                    || handler.lifecycle() != Lifecycle::Active
                    || handler.organization() != Some(contact.sponsor())
                    || source.lifecycle() != Lifecycle::Active
                    || source.organization() != Some(contact.institution())
                    || institution.lifecycle() != Lifecycle::Active
                    || state
                        .contacts
                        .active_contact_for(contact.sponsor(), contact.contact())
                        .is_none_or(|current| current.id() != contact.id())
                {
                    return Err(StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    });
                }
            }
            ContactStatus::Terminated => {
                let terminated_at = contact.terminated_at().ok_or(
                    StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    },
                )?;
                if terminated_at < contact.established_at() || terminated_at > state.now() {
                    return Err(StateValidationError::InvalidInstitutionalContact {
                        contact: contact.id(),
                    });
                }
            }
        }
    }

    for disclosure in state.contacts.disclosures() {
        let contact = state.contacts.get_contact(disclosure.contact()).ok_or(
            StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        let source = state
            .intelligence
            .get_information(disclosure.source_information())
            .ok_or(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            })?;
        let disclosed = state
            .intelligence
            .get_information(disclosure.disclosed_information())
            .ok_or(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            })?;
        if disclosure.disclosed_at() < contact.established_at()
            || disclosure.disclosed_at() > state.now()
            || contact
                .terminated_at()
                .is_some_and(|terminated_at| disclosure.disclosed_at() > terminated_at)
            || source.holder() != KnowledgeHolder::Character(contact.contact())
            || source.recorded_at() > disclosure.disclosed_at()
            || source.observed_at() > disclosure.disclosed_at()
            || disclosed.holder() != KnowledgeHolder::Organization(contact.sponsor())
            || disclosed.source_kind() != information_source_kind(contact.kind())
            || disclosed.source_entity() != Some(EntityRef::Character(contact.contact()))
            || disclosed.topic() != source.topic()
            || disclosed.subject() != source.subject()
            || disclosed.observed_at() != source.observed_at()
            || disclosed.recorded_at() != disclosure.disclosed_at()
            || disclosed.reliability() != source.reliability()
            || disclosed.specificity() != source.specificity()
            || disclosed.summary() != source.summary()
            || disclosed.derived_from() != &BTreeSet::from([source.id()])
            || state
                .contacts
                .disclosure_for_information(disclosed.id())
                .is_none_or(|record| record.id() != disclosure.id())
        {
            return Err(StateValidationError::InvalidContactDisclosure {
                disclosure: disclosure.id(),
            });
        }
    }
    Ok(())
}

fn contact_relationship_basis_is_valid(
    handler: CharacterId,
    contact: CharacterId,
    handler_to_contact: Option<ContactRelationshipSnapshot>,
    contact_to_handler: Option<ContactRelationshipSnapshot>,
) -> bool {
    let valid_snapshot = |snapshot: ContactRelationshipSnapshot, from, to| {
        snapshot.from() == from
            && snapshot.to() == to
            && snapshot.version() > 0
            && relationship_dimensions_have_basis(snapshot.dimensions())
    };
    let forward =
        handler_to_contact.is_some_and(|snapshot| valid_snapshot(snapshot, handler, contact));
    let reverse =
        contact_to_handler.is_some_and(|snapshot| valid_snapshot(snapshot, contact, handler));
    (forward || reverse)
        && handler_to_contact.is_none_or(|snapshot| valid_snapshot(snapshot, handler, contact))
        && contact_to_handler.is_none_or(|snapshot| valid_snapshot(snapshot, contact, handler))
}

fn relationship_dimensions_have_basis(dimensions: crate::social::RelationshipDimensions) -> bool {
    [
        dimensions.trust,
        dimensions.respect,
        dimensions.fear,
        dimensions.affection,
        dimensions.dependence,
        dimensions.resentment,
        dimensions.debt,
    ]
    .into_iter()
    .any(|level| level.value() > 0)
}

fn validate_recruitment(state: &AppState) -> Result<(), StateValidationError> {
    let mut previous_attempt_by_pair: BTreeMap<
        (CharacterId, OrganizationId),
        crate::core::time::SimTime,
    > = BTreeMap::new();
    let mut recruitment_history_events = BTreeSet::new();
    let mut recruitment_outcome_information = BTreeSet::new();
    for attempt in state.recruitment.attempts() {
        let candidate = state.world.get_character(attempt.candidate()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let recruiter = state.world.get_character(attempt.recruiter()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let target = state
            .world
            .get_organization(attempt.target_organization())
            .ok_or(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            })?;
        if candidate.id() == recruiter.id()
            || target.kind() != OrganizationKind::Criminal
            || attempt.occurred_at() > state.now()
            || attempt.previous_organization() == Some(attempt.target_organization())
            || attempt
                .previous_supervisor()
                .is_some_and(|supervisor| state.world.get_character(supervisor).is_none())
            || (attempt.previous_supervisor().is_some()
                && attempt.previous_organization().is_none())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }
        if let Some(previous_organization) = attempt.previous_organization() {
            let previous = state.world.get_organization(previous_organization).ok_or(
                StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                },
            )?;
            if previous.kind() != OrganizationKind::Criminal {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        let recruiter_relationship = attempt.recruiter_relationship();
        if recruiter_relationship.from() != attempt.candidate()
            || recruiter_relationship.to() != attempt.recruiter()
            || recruiter_relationship.dimensions().is_none()
            || recruiter_relationship.version().is_none()
            || recruiter_relationship.version() == Some(0)
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }
        match (
            attempt.previous_supervisor(),
            attempt.incumbent_relationship(),
        ) {
            (None, None) => {}
            (Some(supervisor), Some(snapshot)) => {
                let snapshot_shape_is_valid = match (snapshot.dimensions(), snapshot.version()) {
                    (Some(_), Some(version)) => version > 0,
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                };
                if snapshot.from() != attempt.candidate()
                    || snapshot.to() != supervisor
                    || !snapshot_shape_is_valid
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            (None, Some(_)) | (Some(_), None) => {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        match attempt.authority() {
            RecruitmentAuthority::ExecutiveApproval => {}
            RecruitmentAuthority::ApprovedDecision {
                decision,
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            } => {
                let mandate_record = state.delegation.get_mandate(mandate).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                let decision_record = state.decisions.get_decision(decision).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                let approval_context = match decision_record.context() {
                    DecisionContext::RecruitmentApproval(context) => context,
                    DecisionContext::OperationException { .. } => {
                        return Err(StateValidationError::InvalidRecruitmentAttempt {
                            attempt: attempt.id(),
                        });
                    }
                };
                let approval_resolution = decision_record.resolution().ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if manager != attempt.recruiter()
                    || mandate_record.manager() != manager
                    || mandate_record.organization() != attempt.target_organization()
                    || scope
                        != crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        )
                    || mandate_version == 0
                    || mandate_version > mandate_record.version()
                    || manager_version == 0
                    || manager_version > recruiter.version()
                    || policy != ApprovalPolicy::RequireApproval
                    || decision_record.status() != DecisionStatus::Resolved
                    || decision_record.requester() != attempt.recruiter()
                    || decision_record.recipient() != attempt.target_organization()
                    || approval_resolution.response() != DecisionResponse::Approve
                    || approval_resolution.resolved_at() != attempt.occurred_at()
                    || approval_context.target_organization() != attempt.target_organization()
                    || approval_context.recruiter() != attempt.recruiter()
                    || approval_context.candidate() != attempt.candidate()
                    || approval_context.approach() != attempt.approach()
                    || approval_context.authority().authority().mandate != mandate
                    || approval_context.authority().authority().manager != manager
                    || approval_context.authority().authority().scope != scope
                    || approval_context.authority().mandate_version() != mandate_version
                    || approval_context.authority().manager_version() != manager_version
                    || approval_context.authority().policy_source() != policy_source
                    || state
                        .recruitment
                        .attempt_for_approval_decision(decision)
                        .map(|record| record.id())
                        != Some(attempt.id())
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                if mandate_version == mandate_record.version()
                    && !mandate_record.scopes().contains(
                        &crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        ),
                    )
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let valid_policy_source = match policy_source {
                    RecruitmentPolicySource::Organization(organization) => {
                        organization == attempt.target_organization()
                    }
                    RecruitmentPolicySource::Mandate(source_mandate) => source_mandate == mandate,
                };
                if !valid_policy_source {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            RecruitmentAuthority::Delegated {
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            } => {
                let mandate_record = state.delegation.get_mandate(mandate).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if manager != attempt.recruiter()
                    || mandate_record.manager() != manager
                    || mandate_record.organization() != attempt.target_organization()
                    || scope
                        != crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        )
                    || mandate_version == 0
                    || mandate_version > mandate_record.version()
                    || manager_version == 0
                    || manager_version > recruiter.version()
                    || policy != ApprovalPolicy::Delegated
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                if mandate_version == mandate_record.version()
                    && !mandate_record.scopes().contains(
                        &crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        ),
                    )
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let valid_policy_source = match policy_source {
                    RecruitmentPolicySource::Organization(organization) => {
                        organization == attempt.target_organization()
                    }
                    RecruitmentPolicySource::Mandate(source_mandate) => source_mandate == mandate,
                };
                if !valid_policy_source {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
        }

        if let Some(information_id) = attempt.pressure_information() {
            let information = state.intelligence.get_information(information_id).ok_or(
                StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                },
            )?;
            if information.holder() != KnowledgeHolder::Character(attempt.candidate())
                || information.topic() != InformationTopic::PoliceActivity
                || information.subject() != EntityRef::Character(attempt.candidate())
                || information.recorded_at() > attempt.occurred_at()
                || information.observed_at() > attempt.occurred_at()
            {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        let outcome_information = state
            .intelligence
            .get_information(attempt.outcome_information())
            .ok_or(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            })?;
        if !recruitment_outcome_information.insert(attempt.outcome_information())
            || outcome_information.holder()
                != KnowledgeHolder::Organization(attempt.target_organization())
            || outcome_information.source_kind() != InformationSourceKind::AfterAction
            || outcome_information.topic() != InformationTopic::Personnel
            || outcome_information.source_entity()
                != Some(EntityRef::Character(attempt.recruiter()))
            || outcome_information.subject() != EntityRef::Character(attempt.candidate())
            || outcome_information.observed_at() != attempt.occurred_at()
            || outcome_information.recorded_at() != attempt.occurred_at()
            || outcome_information.reliability() != Reliability::DirectAccess
            || outcome_information.specificity() != Specificity::Precise
            || !outcome_information.derived_from().is_empty()
            || outcome_information.summary().trim().is_empty()
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let factors = attempt.factors();
        if factors.recruiter_influence() > 100
            || factors.drive_alignment() > 100
            || factors.relationship_support() > 100
            || factors.incumbent_attachment() > 100
            || factors.incumbent_resentment() > 100
            || factors.perceived_legal_pressure() > 100
            || attempt.outcome() != classify_recruitment_outcome(attempt.margin())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let pair = (attempt.candidate(), attempt.target_organization());
        if let Some(previous_time) = previous_attempt_by_pair.insert(pair, attempt.occurred_at()) {
            if attempt.occurred_at() < previous_time {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        match attempt.outcome() {
            RecruitmentOutcome::Accepted => {
                let history_id = attempt.history_event().ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if !recruitment_history_events.insert(history_id) {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let history = state.history.get_event(history_id).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if history.kind() != HistoryEventKind::Recruitment
                    || history.occurred_at() != attempt.occurred_at()
                    || !history
                        .entities()
                        .contains(&EntityRef::Character(attempt.candidate()))
                    || !history
                        .entities()
                        .contains(&EntityRef::Character(attempt.recruiter()))
                    || !history
                        .entities()
                        .contains(&EntityRef::Organization(attempt.target_organization()))
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            RecruitmentOutcome::Refused => {
                if attempt.history_event().is_some() {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_recruitment_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let definition = registry.recruitment();
    let mut previous_attempt_by_pair: BTreeMap<
        (CharacterId, OrganizationId),
        crate::core::time::SimTime,
    > = BTreeMap::new();
    for attempt in state.recruitment.attempts() {
        let candidate = state.world.get_character(attempt.candidate()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let recruiter = state.world.get_character(attempt.recruiter()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let (expected_pressure_information, expected_legal_pressure) =
            select_perceived_legal_pressure_at(
                definition,
                state,
                attempt.candidate(),
                attempt.occurred_at(),
            );
        let expected_factors =
            calculate_recruitment_factors_from_context(RecruitmentFactorContext {
                definition,
                candidate,
                recruiter,
                approach: attempt.approach(),
                recruiter_relationship: attempt.recruiter_relationship(),
                incumbent_relationship: attempt.incumbent_relationship(),
                perceived_legal_pressure: expected_legal_pressure,
                had_previous_organization: attempt.previous_organization().is_some(),
            });
        if expected_factors != Some(attempt.factors())
            || attempt.pressure_information() != expected_pressure_information
            || attempt.margin() != calculate_recruitment_margin(definition, attempt.factors())
            || attempt.outcome() != classify_recruitment_outcome(attempt.margin())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let pair = (attempt.candidate(), attempt.target_organization());
        if let Some(previous_time) = previous_attempt_by_pair.insert(pair, attempt.occurred_at()) {
            if attempt.occurred_at() < previous_time + definition.cooldown() {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }
    }
    Ok(())
}

fn validate_operations(state: &AppState) -> Result<(), StateValidationError> {
    let mut operation_after_action_information = BTreeSet::new();
    let mut operation_legal_activity_information = BTreeSet::new();
    let mut operation_discovered_information = BTreeSet::new();
    let mut operation_after_action_reports = BTreeSet::new();
    let mut operation_history_events = BTreeSet::new();
    let mut property_disposition_transactions = BTreeSet::new();
    let mut property_disposition_information = BTreeSet::new();
    let mut property_disposition_reports = BTreeSet::new();
    for operation in state.operations.operations() {
        let organization = state
            .world
            .get_organization(operation.responsible_organization())
            .ok_or(StateValidationError::MissingEntity {
                context: "operation organization",
                entity: EntityRef::Organization(operation.responsible_organization()),
            })?;
        let leader = state.world.get_character(operation.leader()).ok_or(
            StateValidationError::MissingEntity {
                context: "operation leader",
                entity: EntityRef::Character(operation.leader()),
            },
        )?;
        let requires_active_participants = match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => true,
            OperationStatus::Completed | OperationStatus::Aborted => false,
        };
        for participant in operation.roles().values() {
            let participant_record = state.world.get_character(*participant).ok_or(
                StateValidationError::MissingEntity {
                    context: "operation participant",
                    entity: EntityRef::Character(*participant),
                },
            )?;
            if requires_active_participants && participant_record.lifecycle() != Lifecycle::Active {
                return Err(StateValidationError::ActiveOperationInvalidParticipant {
                    operation: operation.id(),
                    participant: *participant,
                });
            }
        }
        for information in operation.intelligence() {
            let record = state.intelligence.get_information(*information).ok_or(
                StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                },
            )?;
            if record.holder()
                != KnowledgeHolder::Organization(operation.responsible_organization())
                || !is_information_subject_relevant(state, operation.objective(), record.subject())
            {
                return Err(StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                });
            }
        }
        for entity in operation.objective().referenced_entities() {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "operation objective",
                    entity,
                });
            }
        }
        for constraint in operation.constraints() {
            match constraint {
                OperationConstraint::AvoidCasualties
                | OperationConstraint::DoNotHarmEmployees
                | OperationConstraint::AvoidFirearms
                | OperationConstraint::ProtectLeadershipIdentity
                | OperationConstraint::PreserveMerchandise
                | OperationConstraint::CompleteBefore(_) => {}
                OperationConstraint::ExcludeCharacter(id) => {
                    if state.world.get_character(*id).is_none() {
                        return Err(StateValidationError::MissingEntity {
                            context: "operation constraint",
                            entity: EntityRef::Character(*id),
                        });
                    }
                }
            }
        }
        for contingency in operation.contingencies() {
            match contingency {
                OperationContingency::AbortOnPoliceArrivalBeforeEntry
                | OperationContingency::UseForceOnResistance
                | OperationContingency::UseSecondaryExitIfBlocked
                | OperationContingency::RequestDecisionOnUnexpectedCondition => {}
                OperationContingency::ContactIfDetained(id) => {
                    if state.world.get_character(*id).is_none() {
                        return Err(StateValidationError::MissingEntity {
                            context: "operation contingency",
                            entity: EntityRef::Character(*id),
                        });
                    }
                }
            }
        }
        if operation.entry_at().is_some_and(|entry_at| {
            operation
                .started_at()
                .is_none_or(|started_at| entry_at <= started_at)
        }) {
            return Err(StateValidationError::InvalidOperationRuntime {
                operation: operation.id(),
            });
        }
        if let Some(response_id) = operation.police_response() {
            if state
                .legal
                .get_police_response(response_id)
                .is_none_or(|response| response.source_operation() != operation.id())
            {
                return Err(StateValidationError::InvalidOperationRuntime {
                    operation: operation.id(),
                });
            }
        }
        match operation.status() {
            OperationStatus::Authorized
            | OperationStatus::InProgress
            | OperationStatus::AwaitingDecision => {
                if organization.lifecycle() != Lifecycle::Active {
                    return Err(StateValidationError::ActiveOperationInactiveOrganization {
                        operation: operation.id(),
                    });
                }
                if leader.lifecycle() != Lifecycle::Active {
                    return Err(StateValidationError::ActiveOperationInvalidLeader {
                        operation: operation.id(),
                    });
                }
                if leader.organization() != Some(operation.responsible_organization()) {
                    return Err(StateValidationError::ActiveOperationInvalidLeader {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::Completed | OperationStatus::Aborted => {}
        }
        if operation.status() != OperationStatus::Completed
            && operation.property_disposition().is_some()
        {
            return Err(StateValidationError::InvalidOperationPropertyDisposition {
                operation: operation.id(),
            });
        }
        match operation.status() {
            OperationStatus::Authorized => {
                if operation.started_at().is_some()
                    || operation.resolution_due_at().is_some()
                    || operation.entry_at().is_some()
                    || operation.police_response().is_some()
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::InProgress => {
                let (Some(started_at), Some(due_at)) =
                    (operation.started_at(), operation.resolution_due_at())
                else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || started_at > state.now()
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::AwaitingDecision => {
                let (Some(started_at), Some(due_at), Some(paused_at)) = (
                    operation.started_at(),
                    operation.resolution_due_at(),
                    operation.awaiting_decision_since(),
                ) else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || started_at > paused_at
                    || paused_at > state.now()
                    || operation.resolution().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
            }
            OperationStatus::Completed => {
                let (Some(started_at), Some(due_at), Some(resolution)) = (
                    operation.started_at(),
                    operation.resolution_due_at(),
                    operation.resolution(),
                ) else {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                };
                if started_at > due_at
                    || resolution.resolved_at() < due_at
                    || resolution.resolved_at() > state.now()
                    || operation.awaiting_decision_since().is_some()
                    || operation.abort_record().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
                if let Some(proceeds) = resolution.property_proceeds() {
                    let valid_target = matches!(
                        operation.objective(),
                        OperationObjective::AcquireProperty { target }
                            if *target == proceeds.target()
                    );
                    if !valid_target
                        || resolution.objective_outcome() == OperationObjectiveOutcome::Failed
                        || proceeds.estimated_value().cents() <= 0
                    {
                        return Err(StateValidationError::InvalidOperationDefinition {
                            operation: operation.id(),
                        });
                    }
                }
                validate_operation_property_disposition(
                    state,
                    operation,
                    resolution,
                    &mut property_disposition_transactions,
                    &mut property_disposition_information,
                    &mut property_disposition_reports,
                )?;
                let information = state
                    .intelligence
                    .get_information(resolution.after_action_information())
                    .ok_or(StateValidationError::InvalidOperationAfterAction {
                        operation: operation.id(),
                    })?;
                if !operation_after_action_information.insert(resolution.after_action_information())
                    || information.holder()
                        != KnowledgeHolder::Organization(operation.responsible_organization())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.topic() != InformationTopic::OperationalOutcome
                    || information.source_entity() != Some(EntityRef::Character(operation.leader()))
                    || information.subject() != EntityRef::Operation(operation.id())
                    || information.observed_at() != resolution.resolved_at()
                {
                    return Err(StateValidationError::InvalidOperationAfterAction {
                        operation: operation.id(),
                    });
                }
                let report = state
                    .reports
                    .get_report(resolution.after_action_report())
                    .ok_or(StateValidationError::InvalidOperationAfterActionReport {
                        operation: operation.id(),
                    })?;
                let report_entry = report.entries().first();
                if !operation_after_action_reports.insert(report.id())
                    || report.recipient() != operation.responsible_organization()
                    || report.kind() != ReportKind::AfterAction
                    || report.title() != format!("{} after-action report", operation.title())
                    || report.generated_at() != resolution.resolved_at()
                    || report.entries().len() != 1
                    || !report_entry.is_some_and(|entry| {
                        entry.attention == AttentionClass::Notable
                            && entry.summary == information.summary()
                            && entry.sources.is_empty()
                            && entry.decision.is_none()
                            && entry
                                .entities
                                .contains(&EntityRef::Operation(operation.id()))
                            && entry.entities.contains(&EntityRef::Organization(
                                operation.responsible_organization(),
                            ))
                            && entry
                                .entities
                                .contains(&EntityRef::Character(operation.leader()))
                    })
                {
                    return Err(StateValidationError::InvalidOperationAfterActionReport {
                        operation: operation.id(),
                    });
                }
                match resolution.legal_activity_information() {
                    Some(information_id) => {
                        let investigation_id = resolution.exposure().investigation().ok_or(
                            StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            },
                        )?;
                        let investigation = state.legal.get_investigation(investigation_id).ok_or(
                            StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            },
                        )?;
                        let legal_information = state
                            .intelligence
                            .get_information(information_id)
                            .ok_or(StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            })?;
                        if !operation_legal_activity_information.insert(information_id)
                            || legal_information.holder()
                                != KnowledgeHolder::Organization(
                                    operation.responsible_organization(),
                                )
                            || legal_information.source_kind() != InformationSourceKind::AfterAction
                            || legal_information.topic() != InformationTopic::LegalActivity
                            || legal_information.source_entity()
                                != Some(EntityRef::Character(operation.leader()))
                            || legal_information.subject() != EntityRef::Operation(operation.id())
                            || legal_information.observed_at() != resolution.resolved_at()
                            || legal_information.recorded_at() != resolution.resolved_at()
                            || legal_information.reliability() != Reliability::GenerallyReliable
                            || legal_information.specificity() != Specificity::Specific
                            || legal_information.summary()
                                != build_legal_activity_summary(
                                    state,
                                    operation,
                                    investigation.owner(),
                                )
                        {
                            return Err(StateValidationError::InvalidOperationLegalActivity {
                                operation: operation.id(),
                            });
                        }
                    }
                    None if resolution.exposure().investigation().is_some() => {
                        return Err(StateValidationError::InvalidOperationLegalActivity {
                            operation: operation.id(),
                        });
                    }
                    None => {}
                }
                let valid_history = state
                    .history
                    .get_event(resolution.history_event())
                    .is_some_and(|event| {
                        operation_history_events.insert(event.id())
                            && event.kind() == HistoryEventKind::Operation
                            && event.occurred_at() == resolution.resolved_at()
                            && event
                                .entities()
                                .contains(&EntityRef::Operation(operation.id()))
                            && event.entities().contains(&EntityRef::Organization(
                                operation.responsible_organization(),
                            ))
                            && event
                                .entities()
                                .contains(&EntityRef::Character(operation.leader()))
                    });
                if !valid_history {
                    return Err(StateValidationError::InvalidOperationHistory {
                        operation: operation.id(),
                    });
                }
                validate_operation_discoveries(
                    state,
                    operation,
                    resolution,
                    &mut operation_discovered_information,
                )?;
                validate_operation_exposure_links(state, operation, resolution)?;
            }
            OperationStatus::Aborted => {
                let abort = operation.abort_record().ok_or(
                    StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    },
                )?;
                let pause_shape_valid = match abort.phase() {
                    OperationAbortPhase::AwaitingDecision => operation
                        .awaiting_decision_since()
                        .is_some_and(|paused_at| paused_at <= abort.aborted_at()),
                    OperationAbortPhase::BeforeStart | OperationAbortPhase::InProgress => {
                        operation.awaiting_decision_since().is_none()
                    }
                };
                if !pause_shape_valid || operation.resolution().is_some() {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
                validate_operation_abort_links(
                    state,
                    operation,
                    abort,
                    &mut operation_after_action_information,
                    &mut operation_after_action_reports,
                    &mut operation_history_events,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_operation_property_disposition(
    state: &AppState,
    operation: &OperationRecord,
    resolution: &OperationResolutionRecord,
    transactions: &mut BTreeSet<LedgerTransactionId>,
    information_ids: &mut BTreeSet<InformationId>,
    reports: &mut BTreeSet<ReportId>,
) -> Result<(), StateValidationError> {
    let Some(disposition) = operation.property_disposition() else {
        return Ok(());
    };
    let invalid = || StateValidationError::InvalidOperationPropertyDisposition {
        operation: operation.id(),
    };
    let proceeds = resolution.property_proceeds().ok_or_else(invalid)?;
    if disposition.disposed_at() < resolution.resolved_at()
        || disposition.disposed_at() > state.now()
        || disposition.realized_value().cents() <= 0
        || disposition.realized_value().cents() > proceeds.estimated_value().cents()
        || !transactions.insert(disposition.transaction())
        || !information_ids.insert(disposition.information())
        || !reports.insert(disposition.report())
    {
        return Err(invalid());
    }

    let venue = state
        .world
        .get_business(disposition.venue())
        .ok_or_else(invalid)?;
    let ownership = state
        .world
        .get_business_ownership_change_for_version(disposition.venue(), disposition.venue_version())
        .ok_or_else(invalid)?;
    let next_ownership_at_disposition = disposition
        .venue_version()
        .checked_add(1)
        .and_then(|version| {
            state
                .world
                .get_business_ownership_change_for_version(disposition.venue(), version)
        })
        .is_some_and(|next| next.changed_at() <= disposition.disposed_at());
    if disposition.venue_version() > venue.version()
        || ownership.new_owner()
            != BusinessOwner::Organization(operation.responsible_organization())
        || ownership.changed_at() > disposition.disposed_at()
        || next_ownership_at_disposition
        || state
            .world
            .business_owner_at(disposition.venue(), disposition.disposed_at())
            != Some(BusinessOwner::Organization(
                operation.responsible_organization(),
            ))
        || !venue.has_function(BusinessFunction::ResaleMarket)
    {
        return Err(invalid());
    }

    let cash = state
        .finance
        .get_account(disposition.cash_account())
        .ok_or_else(invalid)?;
    let settlement = state
        .finance
        .get_account(disposition.settlement_account())
        .ok_or_else(invalid)?;
    let expected_owner = FinancialOwner::Organization(operation.responsible_organization());
    if disposition.cash_account() == disposition.settlement_account()
        || cash.owner() != expected_owner
        || settlement.owner() != expected_owner
        || !matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        )
        || settlement.kind() != AccountKind::Settlement
    {
        return Err(invalid());
    }

    let transaction = state
        .finance
        .get_transaction(disposition.transaction())
        .ok_or_else(invalid)?;
    let negative_value = disposition
        .realized_value()
        .cents()
        .checked_neg()
        .map(Money::from_cents)
        .ok_or_else(invalid)?;
    let has_cash_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.cash_account()
            && posting.amount == disposition.realized_value()
    });
    let has_settlement_posting = transaction.postings().iter().any(|posting| {
        posting.account == disposition.settlement_account() && posting.amount == negative_value
    });
    if transaction.occurred_at() != disposition.disposed_at()
        || transaction.memo()
            != format!(
                "Property liquidation for {} through {}",
                operation.id(),
                disposition.venue()
            )
        || transaction.postings().len() != 2
        || !has_cash_posting
        || !has_settlement_posting
        || transaction.budget_usage().is_some()
    {
        return Err(invalid());
    }

    let information = state
        .intelligence
        .get_information(disposition.information())
        .ok_or_else(invalid)?;
    if information.holder() != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::Accountant
        || information.topic() != InformationTopic::FinancialPerformance
        || information.source_entity() != Some(EntityRef::Business(disposition.venue()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != disposition.disposed_at()
        || information.recorded_at() != disposition.disposed_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || information.summary()
            != build_disposition_summary(
                operation.title(),
                venue.name(),
                proceeds.estimated_value(),
                disposition.realized_value(),
            )
    {
        return Err(invalid());
    }
    let report = state
        .reports
        .get_report(disposition.report())
        .ok_or_else(invalid)?;
    let expected_summary = build_disposition_summary(
        operation.title(),
        venue.name(),
        proceeds.estimated_value(),
        disposition.realized_value(),
    );
    if report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::Financial
        || report.title() != "Property disposition"
        || report.generated_at() != disposition.disposed_at()
        || report.entries().len() != 1
    {
        return Err(invalid());
    }
    let entry = &report.entries()[0];
    if entry.attention != AttentionClass::Notable
        || entry.summary != expected_summary
        || !entry.sources.is_empty()
        || entry.entities
            != BTreeSet::from([
                EntityRef::Operation(operation.id()),
                EntityRef::Business(disposition.venue()),
            ])
        || entry.decision.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_operation_discoveries(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
    discovered_information: &mut BTreeSet<InformationId>,
) -> Result<(), StateValidationError> {
    match operation.kind() {
        OperationKind::Surveillance => {
            let OperationObjective::GatherInformation { target } = operation.objective() else {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            };
            if !is_supported_surveillance_target(*target) {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
            match resolution.objective_outcome() {
                OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    if resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Failed
                    if !resolution.discovered_information().is_empty() =>
                {
                    return Err(StateValidationError::InvalidOperationDiscovery {
                        operation: operation.id(),
                    });
                }
                OperationObjectiveOutcome::Achieved
                | OperationObjectiveOutcome::Partial
                | OperationObjectiveOutcome::Failed => {}
            }
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::Kidnapping
        | OperationKind::Sabotage
        | OperationKind::Bribery
        | OperationKind::WitnessPressure
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::CovertTransfer
        | OperationKind::Extraction
        | OperationKind::RivalInfiltration => {
            if !resolution.discovered_information().is_empty() {
                return Err(StateValidationError::InvalidOperationDiscovery {
                    operation: operation.id(),
                });
            }
        }
    }

    let expected_signatures = expected_persisted_surveillance_signatures(state, operation);
    let mut actual_signatures = BTreeSet::new();
    for information_id in resolution.discovered_information() {
        let information = state.intelligence.get_information(*information_id).ok_or(
            StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            },
        )?;
        if !discovered_information.insert(*information_id)
            || !actual_signatures.insert((information.topic(), information.subject()))
            || state
                .operations
                .operation_for_discovered_information(*information_id)
                .is_none_or(|source| source.id() != operation.id())
            || information.recorded_at() != resolution.resolved_at()
            || !is_valid_persisted_surveillance_information(state, operation, information)
        {
            return Err(StateValidationError::InvalidOperationDiscovery {
                operation: operation.id(),
            });
        }
    }
    if operation.kind() == OperationKind::Surveillance
        && expected_signatures.as_ref() != Some(&actual_signatures)
    {
        return Err(StateValidationError::InvalidOperationDiscovery {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_operation_abort_links(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    abort: crate::operations::OperationAbortRecord,
    operation_after_action_information: &mut BTreeSet<InformationId>,
    operation_after_action_reports: &mut BTreeSet<ReportId>,
    operation_history_events: &mut BTreeSet<crate::core::id::HistoryEventId>,
) -> Result<(), StateValidationError> {
    if abort.aborted_at() > state.now() {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    match (abort.phase(), abort.cause(), abort.artifacts()) {
        (OperationAbortPhase::BeforeStart, OperationAbortCause::AuthorityOrder, None) => {
            if operation.started_at().is_some() || operation.resolution_due_at().is_some() {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            return Ok(());
        }
        (
            OperationAbortPhase::BeforeStart,
            OperationAbortCause::DeadlineMissed,
            Some(artifacts),
        ) => {
            let deadline = operation
                .constraints()
                .iter()
                .filter_map(|constraint| match constraint {
                    OperationConstraint::CompleteBefore(deadline) => Some(*deadline),
                    OperationConstraint::AvoidCasualties
                    | OperationConstraint::DoNotHarmEmployees
                    | OperationConstraint::AvoidFirearms
                    | OperationConstraint::ProtectLeadershipIdentity
                    | OperationConstraint::PreserveMerchandise
                    | OperationConstraint::ExcludeCharacter(_) => None,
                })
                .min();
            if operation.started_at().is_some()
                || operation.resolution_due_at().is_some()
                || deadline.is_none_or(|deadline| deadline >= abort.aborted_at())
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (OperationAbortPhase::InProgress, OperationAbortCause::AuthorityOrder, Some(artifacts)) => {
            let (Some(started_at), Some(due_at)) =
                (operation.started_at(), operation.resolution_due_at())
            else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            if started_at > due_at || abort.aborted_at() < started_at {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::InProgress,
            OperationAbortCause::PoliceArrival(response_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at), Some(entry_at)) = (
                operation.started_at(),
                operation.resolution_due_at(),
                operation.entry_at(),
            ) else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let response = state.legal.get_police_response(response_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || operation.police_response() != Some(response_id)
                || response.source_operation() != operation.id()
                || response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at > abort.aborted_at() || arrived_at >= entry_at
                })
                || !operation
                    .contingencies()
                    .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::AwaitingDecision,
            OperationAbortCause::PoliceArrival(response_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at), Some(entry_at), Some(paused_at)) = (
                operation.started_at(),
                operation.resolution_due_at(),
                operation.entry_at(),
                operation.awaiting_decision_since(),
            ) else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let response = state.legal.get_police_response(response_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            let paused_minutes = abort
                .aborted_at()
                .as_minutes()
                .checked_sub(paused_at.as_minutes())
                .ok_or(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                })?;
            let projected_entry = if entry_at > paused_at {
                SimTime::from_minutes(entry_at.as_minutes().checked_add(paused_minutes).ok_or(
                    StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    },
                )?)
            } else {
                entry_at
            };
            let matching_continue_decisions = state
                .decisions
                .decisions()
                .filter(|decision| {
                    matches!(
                        decision.context(),
                        DecisionContext::OperationException {
                            operation: decision_operation,
                            reason: _,
                        } if decision_operation == operation.id()
                    ) && decision.resolution().is_some_and(|resolution| {
                        resolution.response() == DecisionResponse::Continue
                            && resolution.resolved_at() == abort.aborted_at()
                    })
                })
                .count();
            if started_at > due_at
                || started_at > paused_at
                || operation.police_response() != Some(response_id)
                || response.source_operation() != operation.id()
                || response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at > abort.aborted_at() || arrived_at >= projected_entry
                })
                || !operation
                    .contingencies()
                    .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
                || matching_continue_decisions != 1
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (
            OperationAbortPhase::AwaitingDecision,
            OperationAbortCause::Decision(decision_id),
            Some(artifacts),
        ) => {
            let (Some(started_at), Some(due_at)) =
                (operation.started_at(), operation.resolution_due_at())
            else {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            };
            let decision = state.decisions.get_decision(decision_id).ok_or(
                StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                },
            )?;
            let decision_matches = matches!(
                decision.context(),
                DecisionContext::OperationException {
                    operation: decision_operation,
                    reason: _,
                } if decision_operation == operation.id()
            );
            let resolution =
                decision
                    .resolution()
                    .ok_or(StateValidationError::InvalidOperationAbort {
                        operation: operation.id(),
                    })?;
            if started_at > due_at
                || abort.aborted_at() < started_at
                || !decision_matches
                || decision.status() != DecisionStatus::Resolved
                || decision.recipient() != operation.responsible_organization()
                || decision.requester() != operation.leader()
                || resolution.response() != DecisionResponse::Abort
                || resolution.resolved_at() != abort.aborted_at()
            {
                return Err(StateValidationError::InvalidOperationAbort {
                    operation: operation.id(),
                });
            }
            validate_operation_abort_artifacts(
                state,
                operation,
                abort,
                artifacts,
                operation_after_action_information,
                operation_after_action_reports,
                operation_history_events,
            )?;
        }
        (OperationAbortPhase::BeforeStart, _, Some(_))
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::DeadlineMissed, None)
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::Decision(_), None)
        | (OperationAbortPhase::BeforeStart, OperationAbortCause::PoliceArrival(_), None)
        | (OperationAbortPhase::InProgress, _, None)
        | (OperationAbortPhase::InProgress, OperationAbortCause::Decision(_), Some(_))
        | (OperationAbortPhase::InProgress, OperationAbortCause::DeadlineMissed, Some(_))
        | (OperationAbortPhase::AwaitingDecision, _, None)
        | (OperationAbortPhase::AwaitingDecision, OperationAbortCause::DeadlineMissed, Some(_))
        | (OperationAbortPhase::AwaitingDecision, OperationAbortCause::AuthorityOrder, Some(_)) => {
            return Err(StateValidationError::InvalidOperationAbort {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_abort_artifacts(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    abort: crate::operations::OperationAbortRecord,
    artifacts: crate::operations::OperationAbortArtifacts,
    operation_after_action_information: &mut BTreeSet<InformationId>,
    operation_after_action_reports: &mut BTreeSet<ReportId>,
    operation_history_events: &mut BTreeSet<crate::core::id::HistoryEventId>,
) -> Result<(), StateValidationError> {
    let information = state
        .intelligence
        .get_information(artifacts.information())
        .ok_or(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        })?;
    if !operation_after_action_information.insert(information.id())
        || information.holder()
            != KnowledgeHolder::Organization(operation.responsible_organization())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::OperationalOutcome
        || information.source_entity() != Some(EntityRef::Character(operation.leader()))
        || information.subject() != EntityRef::Operation(operation.id())
        || information.observed_at() != abort.aborted_at()
        || information.recorded_at() != abort.aborted_at()
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    let report = state.reports.get_report(artifacts.report()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    let report_entry = report.entries().first();
    if !operation_after_action_reports.insert(report.id())
        || report.recipient() != operation.responsible_organization()
        || report.kind() != ReportKind::AfterAction
        || report.title() != format!("{} after-action report", operation.title())
        || report.generated_at() != abort.aborted_at()
        || report.entries().len() != 1
        || !report_entry.is_some_and(|entry| {
            entry.attention == AttentionClass::Notable
                && entry.summary == information.summary()
                && entry.sources.is_empty()
                && entry.decision.is_none()
                && entry
                    .entities
                    .contains(&EntityRef::Operation(operation.id()))
                && entry.entities.contains(&EntityRef::Organization(
                    operation.responsible_organization(),
                ))
                && entry
                    .entities
                    .contains(&EntityRef::Character(operation.leader()))
                && match abort.cause() {
                    OperationAbortCause::AuthorityOrder => true,
                    OperationAbortCause::Decision(decision) => entry
                        .entities
                        .contains(&EntityRef::DecisionRequest(decision)),
                    OperationAbortCause::PoliceArrival(response) => state
                        .legal
                        .get_police_response(response)
                        .is_some_and(|response| {
                            entry
                                .entities
                                .contains(&EntityRef::Organization(response.authority()))
                                && entry
                                    .entities
                                    .contains(&EntityRef::Neighborhood(response.neighborhood()))
                        }),
                    OperationAbortCause::DeadlineMissed => true,
                }
        })
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }

    let history = state.history.get_event(artifacts.history_event()).ok_or(
        StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        },
    )?;
    if !operation_history_events.insert(history.id())
        || history.kind() != HistoryEventKind::Operation
        || history.occurred_at() != abort.aborted_at()
        || history.summary() != information.summary()
        || !history
            .entities()
            .contains(&EntityRef::Operation(operation.id()))
        || !history.entities().contains(&EntityRef::Organization(
            operation.responsible_organization(),
        ))
        || !history
            .entities()
            .contains(&EntityRef::Character(operation.leader()))
        || match abort.cause() {
            OperationAbortCause::AuthorityOrder => false,
            OperationAbortCause::Decision(decision) => !history
                .entities()
                .contains(&EntityRef::DecisionRequest(decision)),
            OperationAbortCause::PoliceArrival(response) => state
                .legal
                .get_police_response(response)
                .is_none_or(|response| {
                    !history
                        .entities()
                        .contains(&EntityRef::Organization(response.authority()))
                        || !history
                            .entities()
                            .contains(&EntityRef::Neighborhood(response.neighborhood()))
                }),
            OperationAbortCause::DeadlineMissed => false,
        }
    {
        return Err(StateValidationError::InvalidOperationAbort {
            operation: operation.id(),
        });
    }
    Ok(())
}

fn validate_opportunities(state: &AppState) -> Result<(), StateValidationError> {
    for opportunity in state.opportunities.opportunities() {
        let organization = state
            .world
            .get_organization(opportunity.organization())
            .ok_or(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            })?;
        let context = opportunity.context().operation();
        if organization.kind() != OrganizationKind::Criminal
            || context.targets().is_empty()
            || opportunity.source_information().is_empty()
            || opportunity.summary().trim().is_empty()
            || opportunity.discovered_at() > state.now()
            || opportunity.version() == 0
            || opportunity
                .valid_until()
                .is_some_and(|valid_until| valid_until <= opportunity.discovered_at())
        {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        let mut covered_targets = BTreeSet::new();
        for target in context.targets() {
            if !is_entity_present(state, *target) {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
        }
        for source in opportunity.source_information() {
            let information = state.intelligence.get_information(*source).ok_or(
                StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                },
            )?;
            if information.holder() != KnowledgeHolder::Organization(opportunity.organization())
                || information.recorded_at() > opportunity.discovered_at()
                || !context.targets().contains(&information.subject())
            {
                return Err(StateValidationError::InvalidOpportunity {
                    opportunity: opportunity.id(),
                });
            }
            covered_targets.insert(information.subject());
        }
        if covered_targets != *context.targets() {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        let report = state.reports.get_report(opportunity.report()).ok_or(
            StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            },
        )?;
        let mut expected_entities = context.targets().clone();
        expected_entities.insert(EntityRef::Organization(opportunity.organization()));
        let expected_sources: Vec<_> = opportunity.source_information().iter().copied().collect();
        if report.recipient() != opportunity.organization()
            || report.kind() != ReportKind::Opportunity
            || report.generated_at() != opportunity.discovered_at()
            || report.entries().len() != 1
            || !report.entries().first().is_some_and(|entry| {
                entry.attention == AttentionClass::Notable
                    && entry.summary == opportunity.summary()
                    && entry.sources == expected_sources
                    && entry.entities == expected_entities
                    && entry.decision.is_none()
            })
        {
            return Err(StateValidationError::InvalidOpportunity {
                opportunity: opportunity.id(),
            });
        }

        match opportunity.resolution() {
            None => {
                if opportunity.version() != 1
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| valid_until <= state.now())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Dismissed { at }) => {
                if opportunity.version() != 2
                    || at < opportunity.discovered_at()
                    || at > state.now()
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| at >= valid_until)
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Expired { at, report }) => {
                let expiry_report = state.reports.get_report(report).ok_or(
                    StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    },
                )?;
                if opportunity.version() != 2
                    || opportunity.valid_until() != Some(at)
                    || at > state.now()
                    || expiry_report.recipient() != opportunity.organization()
                    || expiry_report.kind() != ReportKind::Opportunity
                    || expiry_report.generated_at() < at
                    || expiry_report.generated_at() > state.now()
                    || expiry_report.entries().len() != 1
                    || !expiry_report.entries().first().is_some_and(|entry| {
                        entry.attention == AttentionClass::Notable
                            && entry.summary
                                == format!("Opportunity expired: {}", opportunity.summary())
                            && entry.sources == expected_sources
                            && entry.entities == expected_entities
                            && entry.decision.is_none()
                    })
                    || state
                        .opportunities
                        .opportunity_for_report(report)
                        .map(|record| record.id())
                        != Some(opportunity.id())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
            Some(OpportunityResolution::Converted { at, operation }) => {
                let operation = state.operations.get_operation(operation).ok_or(
                    StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    },
                )?;
                let operation_targets: BTreeSet<_> = operation
                    .objective()
                    .referenced_entities()
                    .into_iter()
                    .collect();
                if opportunity.version() != 2
                    || at < opportunity.discovered_at()
                    || at > state.now()
                    || at > operation.scheduled_for()
                    || opportunity
                        .valid_until()
                        .is_some_and(|valid_until| at >= valid_until)
                    || operation.responsible_organization() != opportunity.organization()
                    || operation.kind() != context.operation_kind()
                    || operation_targets != *context.targets()
                    || state
                        .opportunities
                        .opportunity_for_operation(operation.id())
                        .map(|record| record.id())
                        != Some(opportunity.id())
                {
                    return Err(StateValidationError::InvalidOpportunity {
                        opportunity: opportunity.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_operation_exposure_links(
    state: &AppState,
    operation: &crate::operations::OperationRecord,
    resolution: &crate::operations::OperationResolutionRecord,
) -> Result<(), StateValidationError> {
    let exposure = resolution.exposure();
    if let Some(neighborhood) = exposure.neighborhood() {
        if state.world.get_neighborhood(neighborhood).is_none() {
            return Err(StateValidationError::InvalidOperationExposure {
                operation: operation.id(),
            });
        }
    }
    let participants: BTreeSet<_> = std::iter::once(operation.leader())
        .chain(operation.roles().values().copied())
        .collect();
    match exposure.level() {
        OperationExposureLevel::Identifying => {
            if !exposure
                .identified_character()
                .is_some_and(|character| participants.contains(&character))
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
        OperationExposureLevel::None
        | OperationExposureLevel::Trace
        | OperationExposureLevel::Witnessed => {
            if exposure.identified_character().is_some() {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
    }

    match exposure.investigation() {
        None => {
            if !exposure.evidence().is_empty() {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
        Some(investigation_id) => {
            if exposure.level() == OperationExposureLevel::None
                || exposure.neighborhood().is_none()
                || exposure.evidence().len() != 1
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
            let investigation = state.legal.get_investigation(investigation_id).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            let owner = state.world.get_organization(investigation.owner()).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            if !matches!(
                owner.kind(),
                OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
            ) || investigation.opened_at() != resolution.resolved_at()
                || !investigation
                    .subjects()
                    .contains(&EntityRef::Operation(operation.id()))
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
            if let Some(character) = exposure.identified_character() {
                if !investigation
                    .subjects()
                    .contains(&EntityRef::Character(character))
                {
                    return Err(StateValidationError::InvalidOperationExposure {
                        operation: operation.id(),
                    });
                }
            }
            let evidence_id = *exposure
                .evidence()
                .iter()
                .next()
                .expect("validated operation exposure contains one evidence record");
            let evidence = state.legal.get_evidence(evidence_id).ok_or(
                StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                },
            )?;
            let expected_subject = exposure
                .identified_character()
                .map(EntityRef::Character)
                .unwrap_or(EntityRef::Operation(operation.id()));
            let expected_strength = match exposure.level() {
                OperationExposureLevel::None => {
                    unreachable!("non-exposure cannot have legal evidence")
                }
                OperationExposureLevel::Trace => EvidenceStrength::Weak,
                OperationExposureLevel::Witnessed => EvidenceStrength::Corroborating,
                OperationExposureLevel::Identifying => EvidenceStrength::Strong,
            };
            let expected_reliability = match exposure.level() {
                OperationExposureLevel::None => {
                    unreachable!("non-exposure cannot have legal evidence")
                }
                OperationExposureLevel::Trace => EvidenceReliability::Questionable,
                OperationExposureLevel::Witnessed => EvidenceReliability::Credible,
                OperationExposureLevel::Identifying => EvidenceReliability::HighlyReliable,
            };
            if evidence.investigation() != investigation_id
                || evidence.custodian() != investigation.owner()
                || evidence.subject() != expected_subject
                || evidence.origin() != Some(EntityRef::Operation(operation.id()))
                || evidence.strength() != expected_strength
                || evidence.reliability() != expected_reliability
                || evidence.discovered_at() != resolution.resolved_at()
            {
                return Err(StateValidationError::InvalidOperationExposure {
                    operation: operation.id(),
                });
            }
        }
    }
    Ok(())
}

fn validate_decisions(state: &AppState) -> Result<(), StateValidationError> {
    for decision in state.decisions.decisions() {
        if state.world.get_organization(decision.recipient()).is_none() {
            return Err(StateValidationError::MissingEntity {
                context: "decision recipient",
                entity: EntityRef::Organization(decision.recipient()),
            });
        }
        if state.world.get_character(decision.requester()).is_none() {
            return Err(StateValidationError::MissingEntity {
                context: "decision requester",
                entity: EntityRef::Character(decision.requester()),
            });
        }
        if decision.summary().trim().is_empty() {
            return Err(StateValidationError::EmptyDecisionSummary {
                decision: decision.id(),
            });
        }
        if decision.options().is_empty() {
            return Err(StateValidationError::DecisionHasNoResponses {
                decision: decision.id(),
            });
        }
        match decision.attention() {
            AttentionClass::Exception | AttentionClass::Crisis => {}
            AttentionClass::Routine | AttentionClass::Notable => {
                return Err(StateValidationError::InvalidDecisionAttention {
                    decision: decision.id(),
                });
            }
        }
        if decision.requested_at() > state.now() {
            return Err(StateValidationError::InvalidDecisionChronology {
                decision: decision.id(),
            });
        }

        if decision.status() == DecisionStatus::Resolved {
            let resolution = decision
                .resolution()
                .expect("resolved decision must contain a resolution");
            if resolution.resolved_at() < decision.requested_at()
                || resolution.resolved_at() > state.now()
            {
                return Err(StateValidationError::InvalidDecisionChronology {
                    decision: decision.id(),
                });
            }
            if resolution.resolved_by() != decision.recipient() {
                return Err(StateValidationError::DecisionResolverMismatch {
                    decision: decision.id(),
                    resolver: resolution.resolved_by(),
                    recipient: decision.recipient(),
                });
            }
            if !decision.options().contains(&resolution.response()) {
                return Err(StateValidationError::DecisionResponseNotOffered {
                    decision: decision.id(),
                    response: resolution.response(),
                });
            }
        }

        match decision.context() {
            DecisionContext::OperationException { operation, reason } => {
                validate_operation_decision(state, decision, operation, reason)?
            }
            DecisionContext::RecruitmentApproval(context) => {
                validate_recruitment_approval_decision(state, decision, context)?
            }
        }
    }

    for operation in state
        .operations
        .operations_with_status(OperationStatus::AwaitingDecision)
    {
        if state
            .decisions
            .pending_for_operation(operation.id())
            .is_none()
        {
            return Err(StateValidationError::AwaitingOperationMissingDecision {
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

fn validate_operation_decision(
    state: &AppState,
    decision: &crate::decisions::DecisionRequestRecord,
    operation_id: OperationId,
    reason: OperationExceptionReason,
) -> Result<(), StateValidationError> {
    let operation = state.operations.get_operation(operation_id).ok_or(
        StateValidationError::MissingEntity {
            context: "decision operation",
            entity: EntityRef::Operation(operation_id),
        },
    )?;
    if decision.options().len() != 2
        || !decision.options().contains(&DecisionResponse::Continue)
        || !decision.options().contains(&DecisionResponse::Abort)
    {
        return Err(StateValidationError::InvalidDecisionContext {
            decision: decision.id(),
        });
    }
    if operation.leader() != decision.requester() {
        return Err(StateValidationError::DecisionRequesterMismatch {
            decision: decision.id(),
            requester: decision.requester(),
            operation: operation_id,
        });
    }
    if operation.responsible_organization() != decision.recipient() {
        return Err(StateValidationError::DecisionRecipientMismatch {
            decision: decision.id(),
            recipient: decision.recipient(),
            operation: operation_id,
        });
    }
    if !operation
        .contingencies()
        .contains(&OperationContingency::RequestDecisionOnUnexpectedCondition)
    {
        return Err(StateValidationError::InvalidDecisionContext {
            decision: decision.id(),
        });
    }
    match reason {
        OperationExceptionReason::UnexpectedCondition => {}
        OperationExceptionReason::PoliceArrival(response_id) => {
            let response = state.legal.get_police_response(response_id).ok_or(
                StateValidationError::InvalidDecisionContext {
                    decision: decision.id(),
                },
            )?;
            let Some(arrived_at) = response.arrived_at() else {
                return Err(StateValidationError::InvalidDecisionContext {
                    decision: decision.id(),
                });
            };
            let matching_decisions = state
                .decisions
                .decisions_for_operation(operation_id)
                .filter(|candidate| {
                    matches!(
                        candidate.context(),
                        DecisionContext::OperationException {
                            reason: OperationExceptionReason::PoliceArrival(candidate_response),
                            ..
                        } if candidate_response == response_id
                    )
                })
                .count();
            let standing_abort_should_have_applied = operation
                .contingencies()
                .contains(&OperationContingency::AbortOnPoliceArrivalBeforeEntry)
                && operation
                    .entry_at()
                    .is_some_and(|entry_at| arrived_at < entry_at);
            if operation.police_response() != Some(response_id)
                || response.source_operation() != operation_id
                || response.status() != PoliceResponseStatus::Arrived
                || arrived_at > decision.requested_at()
                || matching_decisions != 1
                || standing_abort_should_have_applied
            {
                return Err(StateValidationError::InvalidDecisionContext {
                    decision: decision.id(),
                });
            }
        }
    }

    match decision.status() {
        DecisionStatus::Pending => {
            if operation.status() != OperationStatus::AwaitingDecision {
                return Err(StateValidationError::PendingDecisionOperationMismatch {
                    decision: decision.id(),
                    operation: operation_id,
                    status: operation.status(),
                });
            }
            if state.decisions.pending_for_operation(operation_id) != Some(decision.id()) {
                return Err(StateValidationError::IndexInconsistency {
                    subsystem: "decisions",
                });
            }
            if operation.awaiting_decision_since() != Some(decision.requested_at()) {
                return Err(StateValidationError::PendingDecisionOperationMismatch {
                    decision: decision.id(),
                    operation: operation_id,
                    status: operation.status(),
                });
            }
        }
        DecisionStatus::Resolved => {
            let resolution = decision
                .resolution()
                .expect("resolved decision must contain a resolution");
            match resolution.response() {
                DecisionResponse::Continue => {
                    if operation.status() == OperationStatus::AwaitingDecision {
                        let newer_pending = state
                            .decisions
                            .pending_for_operation(operation_id)
                            .and_then(|pending| state.decisions.get_decision(pending))
                            .is_some_and(|pending| {
                                pending.id() != decision.id()
                                    && pending.status() == DecisionStatus::Pending
                                    && pending.requested_at() >= resolution.resolved_at()
                                    && operation.awaiting_decision_since()
                                        == Some(pending.requested_at())
                            });
                        if !newer_pending {
                            return Err(StateValidationError::PendingDecisionOperationMismatch {
                                decision: decision.id(),
                                operation: operation_id,
                                status: operation.status(),
                            });
                        }
                    }
                }
                DecisionResponse::Abort => {
                    let abort = operation.abort_record();
                    if operation.status() != OperationStatus::Aborted
                        || !abort.is_some_and(|abort| {
                            abort.cause() == OperationAbortCause::Decision(decision.id())
                                && abort.phase() == OperationAbortPhase::AwaitingDecision
                                && abort.aborted_at() == resolution.resolved_at()
                        })
                    {
                        return Err(StateValidationError::AbortDecisionOperationMismatch {
                            decision: decision.id(),
                            operation: operation_id,
                        });
                    }
                }
                DecisionResponse::Approve | DecisionResponse::Reject => {
                    return Err(StateValidationError::InvalidDecisionContext {
                        decision: decision.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_recruitment_approval_decision(
    state: &AppState,
    decision: &crate::decisions::DecisionRequestRecord,
    context: crate::decisions::RecruitmentApprovalContext,
) -> Result<(), StateValidationError> {
    if decision.options().len() != 2
        || !decision.options().contains(&DecisionResponse::Approve)
        || !decision.options().contains(&DecisionResponse::Reject)
        || decision.requester() != context.recruiter()
        || decision.recipient() != context.target_organization()
    {
        return Err(StateValidationError::InvalidDecisionContext {
            decision: decision.id(),
        });
    }
    let organization = state
        .world
        .get_organization(context.target_organization())
        .ok_or(StateValidationError::MissingEntity {
            context: "recruitment approval organization",
            entity: EntityRef::Organization(context.target_organization()),
        })?;
    let recruiter = state.world.get_character(context.recruiter()).ok_or(
        StateValidationError::MissingEntity {
            context: "recruitment approval recruiter",
            entity: EntityRef::Character(context.recruiter()),
        },
    )?;
    if state.world.get_character(context.candidate()).is_none() {
        return Err(StateValidationError::MissingEntity {
            context: "recruitment approval candidate",
            entity: EntityRef::Character(context.candidate()),
        });
    }
    let authority = context.authority();
    let mandate_authority = authority.authority();
    let mandate = state
        .delegation
        .get_mandate(mandate_authority.mandate)
        .ok_or(StateValidationError::InvalidDecisionContext {
            decision: decision.id(),
        })?;
    let valid_policy_source = match authority.policy_source() {
        RecruitmentPolicySource::Organization(source) => source == context.target_organization(),
        RecruitmentPolicySource::Mandate(source) => source == mandate_authority.mandate,
    };
    if organization.kind() != OrganizationKind::Criminal
        || mandate_authority.manager != context.recruiter()
        || mandate_authority.scope
            != ResponsibilityScope::Function(ResponsibilityFunction::Personnel)
        || mandate.manager() != context.recruiter()
        || mandate.organization() != context.target_organization()
        || authority.mandate_version() == 0
        || authority.mandate_version() > mandate.version()
        || authority.manager_version() == 0
        || authority.manager_version() > recruiter.version()
        || !valid_policy_source
    {
        return Err(StateValidationError::InvalidDecisionContext {
            decision: decision.id(),
        });
    }

    let linked_attempt = state
        .recruitment
        .attempt_for_approval_decision(decision.id());
    match decision.status() {
        DecisionStatus::Pending => {
            if state.decisions.pending_for_recruitment_approval(
                context.target_organization(),
                context.recruiter(),
                context.candidate(),
            ) != Some(decision.id())
                || linked_attempt.is_some()
            {
                return Err(StateValidationError::InvalidDecisionContext {
                    decision: decision.id(),
                });
            }
        }
        DecisionStatus::Resolved => {
            let resolution = decision
                .resolution()
                .expect("resolved decision must contain a resolution");
            match resolution.response() {
                DecisionResponse::Approve => {
                    let attempt =
                        linked_attempt.ok_or(StateValidationError::InvalidDecisionContext {
                            decision: decision.id(),
                        })?;
                    if attempt.occurred_at() != resolution.resolved_at() {
                        return Err(StateValidationError::InvalidDecisionContext {
                            decision: decision.id(),
                        });
                    }
                }
                DecisionResponse::Reject => {
                    if linked_attempt.is_some() {
                        return Err(StateValidationError::InvalidDecisionContext {
                            decision: decision.id(),
                        });
                    }
                }
                DecisionResponse::Continue | DecisionResponse::Abort => {
                    return Err(StateValidationError::InvalidDecisionContext {
                        decision: decision.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_delegation(state: &AppState) -> Result<(), StateValidationError> {
    for mandate in state.delegation.mandates() {
        let organization = state.world.get_organization(mandate.organization()).ok_or(
            StateValidationError::MissingEntity {
                context: "mandate organization",
                entity: EntityRef::Organization(mandate.organization()),
            },
        )?;
        let manager = state.world.get_character(mandate.manager()).ok_or(
            StateValidationError::MissingEntity {
                context: "mandate manager",
                entity: EntityRef::Character(mandate.manager()),
            },
        )?;
        if manager.organization() != Some(mandate.organization()) {
            return Err(StateValidationError::MandateManagerOrganizationMismatch {
                mandate: mandate.id(),
                manager: mandate.manager(),
            });
        }
        if mandate.scopes().is_empty() {
            return Err(StateValidationError::MandateHasNoScopes {
                mandate: mandate.id(),
            });
        }
        for (kind, setting) in mandate.standing_orders() {
            if setting.kind() != *kind {
                return Err(StateValidationError::MandatePolicyKindMismatch {
                    mandate: mandate.id(),
                    expected: *kind,
                    actual: setting.kind(),
                });
            }
        }
        for scope in mandate.scopes() {
            match scope {
                ResponsibilityScope::Neighborhood(id) => {
                    if state.world.get_neighborhood(*id).is_none() {
                        return Err(StateValidationError::MissingEntity {
                            context: "mandate neighborhood scope",
                            entity: EntityRef::Neighborhood(*id),
                        });
                    }
                }
                ResponsibilityScope::Business(id) => {
                    if state.world.get_business(*id).is_none() {
                        return Err(StateValidationError::MissingEntity {
                            context: "mandate business scope",
                            entity: EntityRef::Business(*id),
                        });
                    }
                }
                ResponsibilityScope::Function(_) => {}
            }
        }
        let budget_account = if let Some(budget) = mandate.budget() {
            if budget.limit.cents() < 0 {
                return Err(StateValidationError::NegativeMandateBudget {
                    mandate: mandate.id(),
                });
            }
            let account = state.finance.get_account(budget.funding_account).ok_or(
                StateValidationError::MissingEntity {
                    context: "mandate budget account",
                    entity: EntityRef::FinancialAccount(budget.funding_account),
                },
            )?;
            if account.owner() != FinancialOwner::Organization(mandate.organization()) {
                return Err(StateValidationError::MandateBudgetAccountOwnerMismatch {
                    mandate: mandate.id(),
                    account: budget.funding_account,
                });
            }
            Some(account)
        } else {
            None
        };
        match mandate.status() {
            MandateStatus::Active => {
                if organization.lifecycle() != Lifecycle::Active
                    || manager.lifecycle() != Lifecycle::Active
                {
                    return Err(StateValidationError::ActiveMandateInvalidManager {
                        mandate: mandate.id(),
                        manager: mandate.manager(),
                    });
                }
                if let Some(account) = budget_account {
                    if account.lifecycle() != AccountLifecycle::Open {
                        return Err(StateValidationError::ActiveMandateBudgetAccountNotOpen {
                            mandate: mandate.id(),
                            account: account.id(),
                        });
                    }
                }
            }
            MandateStatus::Revoked => {}
        }
    }
    Ok(())
}

fn validate_business_economies(state: &AppState) -> Result<(), StateValidationError> {
    for economy in state.economy.business_economies() {
        let business = state.world.get_business(economy.business()).ok_or(
            StateValidationError::InvalidBusinessEconomy {
                business: economy.business(),
            },
        )?;
        let neighborhood = state
            .world
            .get_neighborhood(business.neighborhood())
            .ok_or(StateValidationError::InvalidBusinessEconomy {
                business: economy.business(),
            })?;
        let operating = state
            .finance
            .get_account(economy.operating_account())
            .ok_or(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            })?;
        let settlement = state
            .finance
            .get_account(economy.settlement_account())
            .ok_or(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            })?;
        if operating.owner() != FinancialOwner::Business(economy.business())
            || settlement.owner() != FinancialOwner::Business(economy.business())
            || operating.kind() != AccountKind::LegitimateOperating
            || settlement.kind() != AccountKind::Settlement
            || economy.operating_account() == economy.settlement_account()
        {
            return Err(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            });
        }
        if economy.established_at() > state.now()
            || economy
                .last_cycle_at()
                .is_some_and(|last_cycle| last_cycle > state.now())
        {
            return Err(StateValidationError::InvalidBusinessEconomySchedule {
                business: economy.business(),
            });
        }
        let latest_cycle_at = state
            .economy
            .cycles_for(economy.business())
            .map(|cycle| cycle.occurred_at())
            .max();
        if latest_cycle_at != economy.last_cycle_at() {
            return Err(StateValidationError::InvalidBusinessEconomySchedule {
                business: economy.business(),
            });
        }
        match economy.status() {
            BusinessOperatingStatus::Active => {
                let next_cycle_at = economy.next_cycle_at().ok_or(
                    StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    },
                )?;
                if business.lifecycle() != Lifecycle::Active
                    || neighborhood.lifecycle() != Lifecycle::Active
                {
                    return Err(StateValidationError::InvalidBusinessEconomy {
                        business: economy.business(),
                    });
                }
                if operating.lifecycle() != AccountLifecycle::Open
                    || settlement.lifecycle() != AccountLifecycle::Open
                {
                    return Err(StateValidationError::InvalidBusinessEconomyAccounts {
                        business: economy.business(),
                    });
                }
                if next_cycle_at <= economy.established_at()
                    || economy
                        .last_cycle_at()
                        .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
                {
                    return Err(StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    });
                }
            }
            BusinessOperatingStatus::Suspended | BusinessOperatingStatus::Closed => {
                if economy.next_cycle_at().is_some() {
                    return Err(StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    });
                }
            }
        }
    }

    let mut used_transactions: BTreeSet<LedgerTransactionId> = state
        .enterprises
        .cycles()
        .filter_map(|cycle| cycle.transaction())
        .collect();
    for cycle in state.economy.cycles() {
        let economy = state
            .economy
            .get_business_economy(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let business = state
            .world
            .get_business(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let ownership = state
            .world
            .get_business_ownership_change_for_version(cycle.business(), cycle.business_version())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        if cycle.occurred_at() < economy.established_at()
            || cycle.occurred_at() > state.now()
            || cycle.business_version() == 0
            || cycle.business_version() > business.version()
            || ownership.new_owner() != cycle.owner()
            || ownership.changed_at() > cycle.occurred_at()
            || cycle.gross_revenue().cents() < 0
            || cycle.operating_cost().cents() < 0
            || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
        {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
        let expected_holder = match cycle.owner() {
            BusinessOwner::Independent => None,
            BusinessOwner::Organization(id) => Some(KnowledgeHolder::Organization(id)),
            BusinessOwner::Character(id) => Some(KnowledgeHolder::Character(id)),
        };
        match (cycle.attention(), expected_holder, cycle.information()) {
            (AttentionClass::Routine, _, None) | (AttentionClass::Notable, None, None) => {}
            (AttentionClass::Notable, Some(holder), Some(information_id)) => {
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                if information.holder() != holder
                    || information.source_kind() != InformationSourceKind::Accountant
                    || information.source_entity().is_some()
                    || information.subject() != EntityRef::Business(cycle.business())
                    || information.observed_at() != cycle.occurred_at()
                    || information.recorded_at() != cycle.occurred_at()
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                {
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
            }
            (AttentionClass::Routine, _, Some(_))
            | (AttentionClass::Notable, None, Some(_))
            | (AttentionClass::Notable, Some(_), None)
            | (AttentionClass::Exception | AttentionClass::Crisis, _, _) => {
                return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })
            }
        }
        match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
            (true, None) => {}
            (false, Some(transaction_id)) => {
                if !used_transactions.insert(transaction_id) {
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
                let transaction = state
                    .finance
                    .get_transaction(transaction_id)
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                let settlement_cents = cycle
                    .net_cash()
                    .cents()
                    .checked_neg()
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                let has_operating = transaction.postings().iter().any(|posting| {
                    posting.account == economy.operating_account()
                        && posting.amount == cycle.net_cash()
                });
                let has_settlement = transaction.postings().iter().any(|posting| {
                    posting.account == economy.settlement_account()
                        && posting.amount == Money::from_cents(settlement_cents)
                });
                if transaction.occurred_at() != cycle.occurred_at()
                    || transaction.postings().len() != 2
                    || !has_operating
                    || !has_settlement
                {
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
            }
            (true, Some(_)) | (false, None) => {
                return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })
            }
        }
    }
    Ok(())
}

fn validate_enterprises(state: &AppState) -> Result<(), StateValidationError> {
    for enterprise in state.enterprises.enterprises() {
        let organization = state
            .world
            .get_organization(enterprise.organization())
            .ok_or(StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            })?;
        let authority = enterprise.authority();
        let mandate = state.delegation.get_mandate(authority.mandate).ok_or(
            StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            },
        )?;
        let manager = state.world.get_character(authority.manager).ok_or(
            StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            },
        )?;
        if mandate.organization() != enterprise.organization()
            || mandate.manager() != authority.manager
            || enterprise.manager() != authority.manager
        {
            return Err(StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            });
        }

        let (neighborhood_id, location_is_active) = match enterprise.location() {
            EnterpriseLocation::Neighborhood(id) => {
                let neighborhood = state.world.get_neighborhood(id).ok_or(
                    StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    },
                )?;
                (id, neighborhood.lifecycle() == Lifecycle::Active)
            }
            EnterpriseLocation::Business(id) => {
                let business = state.world.get_business(id).ok_or(
                    StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    },
                )?;
                let neighborhood = state
                    .world
                    .get_neighborhood(business.neighborhood())
                    .ok_or(StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    })?;
                (
                    business.neighborhood(),
                    business.lifecycle() == Lifecycle::Active
                        && neighborhood.lifecycle() == Lifecycle::Active,
                )
            }
        };

        let mut supporting_businesses =
            Vec::with_capacity(enterprise.supporting_businesses().len());
        for business_id in enterprise.supporting_businesses() {
            if matches!(enterprise.location(), EnterpriseLocation::Business(location_id) if location_id == *business_id)
            {
                return Err(StateValidationError::InvalidEnterpriseSupportingBusiness {
                    enterprise: enterprise.id(),
                    business: *business_id,
                });
            }
            let business = state.world.get_business(*business_id).ok_or(
                StateValidationError::InvalidEnterpriseSupportingBusiness {
                    enterprise: enterprise.id(),
                    business: *business_id,
                },
            )?;
            supporting_businesses.push(business);
        }

        let cash = state.finance.get_account(enterprise.cash_account()).ok_or(
            StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            },
        )?;
        let settlement = state
            .finance
            .get_account(enterprise.settlement_account())
            .ok_or(StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            })?;
        let expected_owner = FinancialOwner::Organization(enterprise.organization());
        let cash_kind_is_valid = matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        );
        if cash.owner() != expected_owner
            || settlement.owner() != expected_owner
            || !cash_kind_is_valid
            || settlement.kind() != AccountKind::Settlement
            || enterprise.cash_account() == enterprise.settlement_account()
        {
            return Err(StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            });
        }

        if enterprise.established_at() > state.now()
            || enterprise
                .last_cycle_at()
                .is_some_and(|last_cycle| last_cycle > state.now())
        {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }
        let latest_cycle_at = state
            .enterprises
            .cycles_for(enterprise.id())
            .map(|cycle| cycle.occurred_at())
            .max();
        if latest_cycle_at != enterprise.last_cycle_at() {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }

        match enterprise.status() {
            EnterpriseStatus::Active => {
                let authority_covers_location = match authority.scope {
                    ResponsibilityScope::Function(ResponsibilityFunction::Enterprise) => true,
                    ResponsibilityScope::Function(
                        ResponsibilityFunction::Territory
                        | ResponsibilityFunction::Operations
                        | ResponsibilityFunction::Intelligence
                        | ResponsibilityFunction::Finance
                        | ResponsibilityFunction::Legal
                        | ResponsibilityFunction::Political
                        | ResponsibilityFunction::Personnel,
                    ) => false,
                    ResponsibilityScope::Neighborhood(id) => id == neighborhood_id,
                    ResponsibilityScope::Business(id) => {
                        matches!(enterprise.location(), EnterpriseLocation::Business(location_id) if location_id == id)
                    }
                };
                let next_cycle_at = enterprise.next_cycle_at().ok_or(
                    StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    },
                )?;
                if organization.lifecycle() != Lifecycle::Active
                    || manager.lifecycle() != Lifecycle::Active
                    || manager.organization() != Some(enterprise.organization())
                    || mandate.status() != MandateStatus::Active
                    || !mandate.scopes().contains(&authority.scope)
                    || !authority_covers_location
                    || !location_is_active
                    || supporting_businesses.iter().any(|business| {
                        business.lifecycle() != Lifecycle::Active
                            || business.owner()
                                != BusinessOwner::Organization(enterprise.organization())
                    })
                {
                    return Err(StateValidationError::InvalidEnterpriseAuthority {
                        enterprise: enterprise.id(),
                    });
                }
                if cash.lifecycle() != AccountLifecycle::Open
                    || settlement.lifecycle() != AccountLifecycle::Open
                {
                    return Err(StateValidationError::InvalidEnterpriseAccounts {
                        enterprise: enterprise.id(),
                    });
                }
                if next_cycle_at <= enterprise.established_at()
                    || enterprise
                        .last_cycle_at()
                        .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
                {
                    return Err(StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    });
                }
            }
            EnterpriseStatus::Suspended | EnterpriseStatus::Closed => {
                if enterprise.next_cycle_at().is_some() {
                    return Err(StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    });
                }
            }
        }
    }

    let mut used_transactions = BTreeSet::new();
    for cycle in state.enterprises.cycles() {
        let enterprise = state
            .enterprises
            .get_enterprise(cycle.enterprise())
            .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
        if cycle.occurred_at() < enterprise.established_at()
            || cycle.occurred_at() > state.now()
            || cycle.gross_revenue().cents() < 0
            || cycle.operating_cost().cents() < 0
            || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
        {
            return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
        }
        match cycle.attention() {
            AttentionClass::Routine => {
                if cycle.information().is_some() {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            AttentionClass::Notable => {
                let information_id = cycle
                    .information()
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                if information.holder() != KnowledgeHolder::Organization(enterprise.organization())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.source_entity()
                        != Some(EntityRef::Character(enterprise.manager()))
                    || information.subject() != EntityRef::Enterprise(enterprise.id())
                    || information.observed_at() != cycle.occurred_at()
                    || information.recorded_at() != cycle.occurred_at()
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            AttentionClass::Exception | AttentionClass::Crisis => {
                return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })
            }
        }
        match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
            (true, None) => {}
            (false, Some(transaction_id)) => {
                if !used_transactions.insert(transaction_id) {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
                let transaction = state
                    .finance
                    .get_transaction(transaction_id)
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                let settlement_cents =
                    cycle.net_cash().cents().checked_neg().ok_or(
                        StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() },
                    )?;
                let has_cash = transaction.postings().iter().any(|posting| {
                    posting.account == enterprise.cash_account()
                        && posting.amount == cycle.net_cash()
                });
                let has_settlement = transaction.postings().iter().any(|posting| {
                    posting.account == enterprise.settlement_account()
                        && posting.amount == Money::from_cents(settlement_cents)
                });
                if transaction.occurred_at() != cycle.occurred_at()
                    || transaction.postings().len() != 2
                    || !has_cash
                    || !has_settlement
                {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            (true, Some(_)) | (false, None) => {
                return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })
            }
        }
    }
    Ok(())
}

fn validate_legal_representations(state: &AppState) -> Result<(), StateValidationError> {
    let mut payments = BTreeSet::new();
    let mut information_ids = BTreeSet::new();
    let mut report_ids = BTreeSet::new();
    for representation in state.legal.legal_representations() {
        let invalid = || StateValidationError::InvalidLegalRepresentation {
            representation: representation.id(),
        };
        let arrest = state
            .legal
            .get_arrest(representation.arrest())
            .ok_or_else(invalid)?;
        let defendant = state
            .world
            .get_character(representation.defendant())
            .ok_or_else(invalid)?;
        let sponsor = state
            .world
            .get_organization(representation.sponsor())
            .ok_or_else(invalid)?;
        let counsel = state
            .world
            .get_character(representation.counsel())
            .ok_or_else(invalid)?;
        let firm = state
            .world
            .get_organization(representation.counsel_institution())
            .ok_or_else(invalid)?;
        let contact = state
            .contacts
            .get_contact(representation.contact())
            .ok_or_else(invalid)?;
        let payer = state
            .finance
            .get_account(representation.payer_account())
            .ok_or_else(invalid)?;
        let provider = state
            .finance
            .get_account(representation.provider_account())
            .ok_or_else(invalid)?;
        let payment = state
            .finance
            .get_transaction(representation.payment())
            .ok_or_else(invalid)?;
        let retained_information = state
            .intelligence
            .get_information(representation.information())
            .ok_or_else(invalid)?;
        let retained_report = state
            .reports
            .get_report(representation.report())
            .ok_or_else(invalid)?;

        let Some(outflow) = representation
            .fee()
            .cents()
            .checked_neg()
            .map(Money::from_cents)
        else {
            return Err(invalid());
        };
        let has_payer_posting = payment.postings().iter().any(|posting| {
            posting.account == representation.payer_account() && posting.amount == outflow
        });
        let has_provider_posting = payment.postings().iter().any(|posting| {
            posting.account == representation.provider_account()
                && posting.amount == representation.fee()
        });
        let authority_is_valid = match (representation.authorization(), payment.budget_usage()) {
            (None, None) => true,
            (Some(authority), Some(usage)) => {
                authority.scope == ResponsibilityScope::Function(ResponsibilityFunction::Legal)
                    && usage.mandate() == authority.mandate
                    && usage.manager() == authority.manager
                    && usage.scope() == authority.scope
                    && usage.funding_account() == representation.payer_account()
                    && usage.amount() == representation.fee()
            }
            (None, Some(_)) | (Some(_), None) => false,
        };
        let expected_retained_entities = BTreeSet::from([
            EntityRef::Character(representation.defendant()),
            EntityRef::Character(representation.counsel()),
            EntityRef::Organization(representation.counsel_institution()),
            EntityRef::Investigation(arrest.investigation()),
        ]);
        let retained_report_is_valid = retained_report.recipient() == representation.sponsor()
            && retained_report.kind() == ReportKind::Legal
            && retained_report.title() == "Legal representation retained"
            && retained_report.generated_at() == representation.retained_at()
            && retained_report.entries().len() == 1
            && retained_report.entries()[0].attention == AttentionClass::Notable
            && retained_report.entries()[0].summary == retained_information.summary()
            && retained_report.entries()[0].sources.is_empty()
            && retained_report.entries()[0].decision.is_none()
            && retained_report.entries()[0].entities == expected_retained_entities;

        if arrest.character() != representation.defendant()
            || sponsor.kind() != OrganizationKind::Criminal
            || firm.kind() != OrganizationKind::LegalServices
            || contact.sponsor() != representation.sponsor()
            || contact.contact() != representation.counsel()
            || contact.institution() != representation.counsel_institution()
            || contact.kind() != crate::contacts::ContactKind::Legal
            || counsel.capability(CapabilityKind::LegalKnowledge).is_none()
            || representation.fee() <= Money::ZERO
            || representation.retained_at() > state.now()
            || representation.version() == 0
            || payer.owner() != FinancialOwner::Organization(representation.sponsor())
            || !matches!(
                payer.kind(),
                AccountKind::StreetCash
                    | AccountKind::ConcealedCash
                    | AccountKind::AccountedFunds
                    | AccountKind::LegitimateOperating
            )
            || provider.owner()
                != FinancialOwner::Organization(representation.counsel_institution())
            || provider.kind() != AccountKind::LegitimateOperating
            || payment.occurred_at() != representation.retained_at()
            || payment.postings().len() != 2
            || !has_payer_posting
            || !has_provider_posting
            || !authority_is_valid
            || retained_information.holder()
                != KnowledgeHolder::Organization(representation.sponsor())
            || retained_information.source_kind() != InformationSourceKind::AfterAction
            || retained_information.topic() != InformationTopic::LegalActivity
            || retained_information.source_entity()
                != Some(EntityRef::Character(representation.counsel()))
            || retained_information.subject() != EntityRef::Character(representation.defendant())
            || retained_information.observed_at() != representation.retained_at()
            || retained_information.recorded_at() != representation.retained_at()
            || retained_information.reliability() != Reliability::DirectAccess
            || retained_information.specificity() != Specificity::Precise
            || !retained_information.derived_from().is_empty()
            || retained_information.summary().trim().is_empty()
            || !retained_report_is_valid
            || !payments.insert(representation.payment())
            || !information_ids.insert(representation.information())
            || !report_ids.insert(representation.report())
        {
            return Err(invalid());
        }

        match representation.status() {
            LegalRepresentationStatus::Active => {
                if representation.version() != 1
                    || representation.ended_at().is_some()
                    || representation.end_reason().is_some()
                    || representation.ended_information().is_some()
                    || representation.ended_report().is_some()
                    || contact.status() != ContactStatus::Active
                    || defendant.lifecycle() != Lifecycle::Active
                    || sponsor.lifecycle() != Lifecycle::Active
                    || counsel.lifecycle() != Lifecycle::Active
                    || counsel.organization() != Some(representation.counsel_institution())
                    || firm.lifecycle() != Lifecycle::Active
                    || state
                        .legal
                        .active_representation_for_arrest(representation.arrest())
                        .is_none_or(|active| active.id() != representation.id())
                {
                    return Err(invalid());
                }
            }
            LegalRepresentationStatus::Ended => {
                let ended_at = representation.ended_at().ok_or_else(invalid)?;
                let ended_information_id =
                    representation.ended_information().ok_or_else(invalid)?;
                let ended_report_id = representation.ended_report().ok_or_else(invalid)?;
                let ended_information = state
                    .intelligence
                    .get_information(ended_information_id)
                    .ok_or_else(invalid)?;
                let ended_report = state
                    .reports
                    .get_report(ended_report_id)
                    .ok_or_else(invalid)?;
                let expected_ended_entities = BTreeSet::from([
                    EntityRef::Character(representation.defendant()),
                    EntityRef::Character(representation.counsel()),
                    EntityRef::Organization(representation.counsel_institution()),
                ]);
                if representation.version() != 2
                    || ended_at < representation.retained_at()
                    || ended_at > state.now()
                    || representation.end_reason().is_none()
                    || state
                        .legal
                        .active_representation_for_arrest(representation.arrest())
                        .is_some_and(|active| active.id() == representation.id())
                    || ended_information.holder()
                        != KnowledgeHolder::Organization(representation.sponsor())
                    || ended_information.source_kind() != InformationSourceKind::AfterAction
                    || ended_information.topic() != InformationTopic::LegalActivity
                    || ended_information.source_entity()
                        != Some(EntityRef::Character(representation.counsel()))
                    || ended_information.subject()
                        != EntityRef::Character(representation.defendant())
                    || ended_information.observed_at() != ended_at
                    || ended_information.recorded_at() != ended_at
                    || ended_information.reliability() != Reliability::DirectAccess
                    || ended_information.specificity() != Specificity::Precise
                    || !ended_information.derived_from().is_empty()
                    || ended_information.summary().trim().is_empty()
                    || ended_report.recipient() != representation.sponsor()
                    || ended_report.kind() != ReportKind::Legal
                    || ended_report.title() != "Legal representation ended"
                    || ended_report.generated_at() != ended_at
                    || ended_report.entries().len() != 1
                    || ended_report.entries()[0].attention != AttentionClass::Notable
                    || ended_report.entries()[0].summary != ended_information.summary()
                    || !ended_report.entries()[0].sources.is_empty()
                    || ended_report.entries()[0].decision.is_some()
                    || ended_report.entries()[0].entities != expected_ended_entities
                    || !information_ids.insert(ended_information_id)
                    || !report_ids.insert(ended_report_id)
                {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

fn validate_prosecution_cases(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen_referrals = BTreeSet::new();
    let mut seen_information = BTreeSet::new();
    let mut seen_reports = BTreeSet::new();
    for case in state.legal.prosecution_cases() {
        let invalid_case = || StateValidationError::InvalidProsecutionCase { case: case.id() };
        let arrest = state
            .legal
            .get_arrest(case.arrest())
            .ok_or_else(invalid_case)?;
        let investigation = state
            .legal
            .get_investigation(case.source_investigation())
            .ok_or_else(invalid_case)?;
        let source_authority = state
            .world
            .get_organization(case.source_authority())
            .ok_or_else(invalid_case)?;
        let office = state
            .world
            .get_organization(case.prosecutor_office())
            .ok_or_else(invalid_case)?;
        let lead = state
            .world
            .get_character(case.lead_prosecutor())
            .ok_or_else(invalid_case)?;
        let defendant = state
            .world
            .get_character(case.defendant())
            .ok_or_else(invalid_case)?;
        let referral_version = u32::try_from(case.referrals().len()).map_err(|_| invalid_case())?;
        let expected_version = match case.status() {
            ProsecutionCaseStatus::Reviewing => referral_version,
            ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed => {
                referral_version.checked_add(1).ok_or_else(invalid_case)?
            }
        };
        if case.opened_at() > state.now()
            || case.version() != expected_version
            || case.referrals().is_empty()
            || !case.referrals().contains(&case.initial_referral())
            || arrest.character() != case.defendant()
            || arrest.investigation() != case.source_investigation()
            || arrest.authority() != case.source_authority()
            || investigation.owner() != case.source_authority()
            || source_authority.kind() != OrganizationKind::LawEnforcement
            || office.kind() != OrganizationKind::Prosecutor
            || lead.capability(CapabilityKind::LegalKnowledge).is_none()
            || case.evidence().is_empty()
            || !arrest.evidence().is_subset(case.evidence())
        {
            return Err(invalid_case());
        }

        let expected_entities = BTreeSet::from([
            EntityRef::Character(case.defendant()),
            EntityRef::Organization(case.source_authority()),
            EntityRef::Organization(case.prosecutor_office()),
            EntityRef::Character(case.lead_prosecutor()),
            EntityRef::Investigation(case.source_investigation()),
        ]);

        match case.status() {
            ProsecutionCaseStatus::Reviewing => {
                if case.resolved_at().is_some()
                    || case.resolution_information().is_some()
                    || case.resolution_report().is_some()
                    || source_authority.lifecycle() != Lifecycle::Active
                    || office.lifecycle() != Lifecycle::Active
                    || lead.lifecycle() != Lifecycle::Active
                    || lead.organization() != Some(case.prosecutor_office())
                    || state
                        .legal
                        .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
                        .is_none_or(|open| open.id() != case.id())
                {
                    return Err(invalid_case());
                }
            }
            ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed => {
                let resolved_at = case.resolved_at().ok_or_else(invalid_case)?;
                let information_id = case.resolution_information().ok_or_else(invalid_case)?;
                let report_id = case.resolution_report().ok_or_else(invalid_case)?;
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or_else(invalid_case)?;
                let report = state
                    .reports
                    .get_report(report_id)
                    .ok_or_else(invalid_case)?;
                let (expected_title, expected_summary) =
                    if case.status() == ProsecutionCaseStatus::Declined {
                        (
                            "Prosecution declined",
                            format!(
                                "{} declined prosecution of {} after review by {}.",
                                office.name(),
                                defendant.name(),
                                lead.name()
                            ),
                        )
                    } else {
                        (
                            "Prosecution review closed",
                            format!(
                                "{} closed its prosecution review of {} after review by {}.",
                                office.name(),
                                defendant.name(),
                                lead.name()
                            ),
                        )
                    };
                if resolved_at < case.opened_at()
                    || resolved_at > state.now()
                    || state
                        .legal
                        .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
                        .is_some_and(|open| open.id() == case.id())
                    || !seen_information.insert(information_id)
                    || information.holder()
                        != KnowledgeHolder::Organization(case.prosecutor_office())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.topic() != InformationTopic::LegalActivity
                    || information.source_entity()
                        != Some(EntityRef::Character(case.lead_prosecutor()))
                    || information.subject() != EntityRef::Character(case.defendant())
                    || information.observed_at() != resolved_at
                    || information.recorded_at() != resolved_at
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                    || !information.derived_from().is_empty()
                    || information.summary() != expected_summary
                    || !seen_reports.insert(report_id)
                    || report.recipient() != case.prosecutor_office()
                    || report.kind() != ReportKind::Legal
                    || report.title() != expected_title
                    || report.generated_at() != resolved_at
                    || report.entries().len() != 1
                    || report.entries()[0].attention != AttentionClass::Notable
                    || report.entries()[0].summary != information.summary()
                    || !report.entries()[0].sources.is_empty()
                    || report.entries()[0].decision.is_some()
                    || report.entries()[0].entities != expected_entities
                {
                    return Err(invalid_case());
                }
            }
        }
        let mut referred_evidence = BTreeSet::new();
        for referral_id in case.referrals() {
            let invalid_referral = || StateValidationError::InvalidProsecutionReferral {
                referral: *referral_id,
            };
            let referral = state
                .legal
                .get_prosecution_referral(*referral_id)
                .ok_or_else(invalid_referral)?;
            let information = state
                .intelligence
                .get_information(referral.information())
                .ok_or_else(invalid_referral)?;
            let report = state
                .reports
                .get_report(referral.report())
                .ok_or_else(invalid_referral)?;
            let is_initial = referral.id() == case.initial_referral();
            let expected_title = if is_initial {
                "Prosecution case referral"
            } else {
                "Prosecution evidence supplement"
            };
            if !seen_referrals.insert(referral.id())
                || referral.prosecution_case() != case.id()
                || referral.source_investigation() != case.source_investigation()
                || referral.source_authority() != case.source_authority()
                || referral.prosecutor_office() != case.prosecutor_office()
                || referral.evidence().is_empty()
                || referral.referred_at() < case.opened_at()
                || referral.referred_at() > state.now()
                || case
                    .resolved_at()
                    .is_some_and(|resolved_at| referral.referred_at() > resolved_at)
                || (is_initial && referral.referred_at() != case.opened_at())
                || referral.evidence().iter().any(|evidence_id| {
                    state
                        .legal
                        .get_evidence(*evidence_id)
                        .is_none_or(|evidence| {
                            evidence.investigation() != case.source_investigation()
                                || evidence.custodian() != case.source_authority()
                                || evidence.discovered_at() > referral.referred_at()
                        })
                        || !referred_evidence.insert(*evidence_id)
                })
                || !seen_information.insert(referral.information())
                || information.holder() != KnowledgeHolder::Organization(case.prosecutor_office())
                || information.source_kind() != InformationSourceKind::AfterAction
                || information.topic() != InformationTopic::LegalActivity
                || information.source_entity()
                    != Some(EntityRef::Organization(case.source_authority()))
                || information.subject() != EntityRef::Character(case.defendant())
                || information.observed_at() != referral.referred_at()
                || information.recorded_at() != referral.referred_at()
                || information.reliability() != Reliability::DirectAccess
                || information.specificity() != Specificity::Precise
                || !information.derived_from().is_empty()
                || information.summary().trim().is_empty()
                || !seen_reports.insert(referral.report())
                || report.recipient() != case.prosecutor_office()
                || report.kind() != ReportKind::Legal
                || report.title() != expected_title
                || report.generated_at() != referral.referred_at()
                || report.entries().len() != 1
                || report.entries()[0].attention != AttentionClass::Notable
                || report.entries()[0].summary != information.summary()
                || !report.entries()[0].sources.is_empty()
                || report.entries()[0].decision.is_some()
                || report.entries()[0].entities != expected_entities
            {
                return Err(invalid_referral());
            }
        }
        if referred_evidence != *case.evidence() {
            return Err(invalid_case());
        }
    }
    for referral in state.legal.prosecution_referrals() {
        if !seen_referrals.contains(&referral.id()) {
            return Err(StateValidationError::InvalidProsecutionReferral {
                referral: referral.id(),
            });
        }
    }
    Ok(())
}

fn validate_legal_reports_and_history(state: &AppState) -> Result<(), StateValidationError> {
    for jurisdiction in state.legal.jurisdictions() {
        let organization = state
            .world
            .get_organization(jurisdiction.organization())
            .ok_or(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            })?;
        if !matches!(
            organization.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) || jurisdiction.neighborhoods().is_empty()
            || jurisdiction.version() == 0
            || jurisdiction
                .neighborhoods()
                .iter()
                .any(|neighborhood| state.world.get_neighborhood(*neighborhood).is_none())
        {
            return Err(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            });
        }
    }

    for response in state.legal.police_responses() {
        let authority = state.world.get_organization(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || state
                .world
                .get_neighborhood(response.neighborhood())
                .is_none()
            || response.version() == 0
            || response.dispatched_at() >= response.arrival_due_at()
            || response.dispatched_at() > state.now()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        let operation = state
            .operations
            .get_operation(response.source_operation())
            .ok_or(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            })?;
        let jurisdiction = state.legal.get_jurisdiction(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if operation.police_response() != Some(response.id())
            || operation.started_at() != Some(response.dispatched_at())
            || response.jurisdiction_version() == 0
            || response.jurisdiction_version() > jurisdiction.version()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        if let Some(patrol) = response.patrol() {
            let deployment = state
                .legal
                .get_patrol_deployment(patrol.deployment())
                .ok_or(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                })?;
            if patrol.version() == 0
                || patrol.version() > deployment.version()
                || deployment.organization() != response.authority()
                || deployment.neighborhood() != response.neighborhood()
            {
                return Err(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                });
            }
        }
        match response.status() {
            PoliceResponseStatus::Dispatched => {
                if response.arrived_at().is_some() || response.version() != 1 {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
            PoliceResponseStatus::Arrived => {
                if response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at < response.arrival_due_at() || arrived_at > state.now()
                }) || response.version() < 2
                {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
        }
    }

    for deployment in state.legal.patrol_deployments() {
        let authority = state
            .world
            .get_organization(deployment.organization())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        let neighborhood = state
            .world
            .get_neighborhood(deployment.neighborhood())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || deployment.version() == 0
            || deployment.established_at() > deployment.last_changed_at()
            || deployment.last_changed_at() > state.now()
            || !is_canonical_patrol_schedule(deployment.windows())
        {
            return Err(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            });
        }
        match deployment.status() {
            PatrolDeploymentStatus::Active => {
                let jurisdiction = state.legal.get_jurisdiction(deployment.organization());
                if authority.lifecycle() != Lifecycle::Active
                    || neighborhood.lifecycle() != Lifecycle::Active
                    || jurisdiction.is_none_or(|record| {
                        !record.neighborhoods().contains(&deployment.neighborhood())
                    })
                    || state
                        .legal
                        .active_patrol_for(deployment.organization(), deployment.neighborhood())
                        .is_none_or(|record| record.id() != deployment.id())
                {
                    return Err(StateValidationError::InvalidPatrolDeployment {
                        deployment: deployment.id(),
                    });
                }
            }
            PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired => {}
        }
    }

    for arrest in state.legal.arrests() {
        let character = state.world.get_character(arrest.character()).ok_or(
            StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            },
        )?;
        let authority = state.world.get_organization(arrest.authority()).ok_or(
            StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            },
        )?;
        let investigation = state
            .legal
            .get_investigation(arrest.investigation())
            .ok_or(StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            })?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || investigation.owner() != arrest.authority()
            || !investigation
                .subjects()
                .contains(&EntityRef::Character(arrest.character()))
            || arrest.evidence().is_empty()
            || arrest.arrested_at() > state.now()
            || arrest.version() == 0
            || arrest.evidence().iter().any(|evidence_id| {
                state
                    .legal
                    .get_evidence(*evidence_id)
                    .is_none_or(|evidence| {
                        evidence.investigation() != arrest.investigation()
                            || evidence.custodian() != arrest.authority()
                            || evidence.subject() != EntityRef::Character(arrest.character())
                            || evidence.discovered_at() > arrest.arrested_at()
                    })
            })
        {
            return Err(StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            });
        }
        match arrest.status() {
            ArrestStatus::Detained => {
                let active_operation = state.operations.operations().any(|operation| {
                    matches!(
                        operation.status(),
                        OperationStatus::Authorized
                            | OperationStatus::InProgress
                            | OperationStatus::AwaitingDecision
                    ) && (operation.leader() == arrest.character()
                        || operation
                            .roles()
                            .values()
                            .any(|participant| *participant == arrest.character()))
                });
                if arrest.released_at().is_some()
                    || arrest.version() != 1
                    || investigation.status() != InvestigationStatus::Active
                    || character.lifecycle() != Lifecycle::Active
                    || authority.lifecycle() != Lifecycle::Active
                    || state
                        .legal
                        .active_arrest_for_character(arrest.character())
                        .is_none_or(|active| active.id() != arrest.id())
                    || state
                        .legal
                        .work_for_investigator(arrest.character())
                        .any(|work| work.status() == InvestigationWorkStatus::Scheduled)
                    || active_operation
                {
                    return Err(StateValidationError::InvalidArrest {
                        arrest: arrest.id(),
                    });
                }
            }
            ArrestStatus::Released => {
                if arrest.version() != 2
                    || arrest.released_at().is_none_or(|released_at| {
                        released_at < arrest.arrested_at() || released_at > state.now()
                    })
                {
                    return Err(StateValidationError::InvalidArrest {
                        arrest: arrest.id(),
                    });
                }
            }
        }
    }

    validate_legal_representations(state)?;
    validate_prosecution_cases(state)?;

    for investigation in state.legal.investigations() {
        let owner = state.world.get_organization(investigation.owner()).ok_or(
            StateValidationError::MissingEntity {
                context: "investigation owner",
                entity: EntityRef::Organization(investigation.owner()),
            },
        )?;
        if !matches!(
            owner.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) {
            return Err(StateValidationError::MissingEntity {
                context: "investigation owner",
                entity: EntityRef::Organization(investigation.owner()),
            });
        }
        if investigation.opened_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "investigation",
            });
        }
        match investigation.status() {
            InvestigationStatus::Active
            | InvestigationStatus::Suspended
            | InvestigationStatus::Closed => {}
        }
        if investigation.version() == 0
            || investigation
                .lead_investigator()
                .is_some_and(|lead| !investigation.assigned_investigators().contains(&lead))
        {
            return Err(StateValidationError::InvalidInvestigationStaffing {
                investigation: investigation.id(),
            });
        }
        for investigator in investigation.assigned_investigators() {
            let character = state.world.get_character(*investigator).ok_or(
                StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                },
            )?;
            if investigation.status() == InvestigationStatus::Active
                && (character.lifecycle() != Lifecycle::Active
                    || character.organization() != Some(investigation.owner())
                    || character
                        .capability(CapabilityKind::Investigation)
                        .is_none())
            {
                return Err(StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                });
            }
        }
        for subject in investigation.subjects() {
            if !is_entity_present(state, *subject) {
                return Err(StateValidationError::MissingEntity {
                    context: "investigation subject",
                    entity: *subject,
                });
            }
        }
    }

    let mut derived_evidence_from_work = BTreeSet::new();
    for work in state.legal.investigation_work() {
        let investigation = state
            .legal
            .get_investigation(work.investigation())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let investigator = state
            .world
            .get_character(work.investigator())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let focus_is_valid = match (work.kind(), work.focus()) {
            (
                InvestigationWorkKind::PatternAnalysis,
                InvestigationWorkFocus::EntityConnection { from, to },
            ) => {
                from < to
                    && is_entity_present(state, from)
                    && is_entity_present(state, to)
                    && source_evidence_forms_simple_path(state, work)
            }
            (InvestigationWorkKind::EvidenceReview, InvestigationWorkFocus::Evidence(source)) => {
                work.source_evidence() == &BTreeSet::from([source])
                    && state.legal.get_evidence(source).is_some_and(|evidence| {
                        evidence.investigation() == work.investigation()
                            && evidence.discovered_at() <= work.scheduled_at()
                            && is_reviewable_evidence_kind(evidence.kind())
                    })
            }
            (InvestigationWorkKind::PatternAnalysis, InvestigationWorkFocus::Evidence(_))
            | (
                InvestigationWorkKind::EvidenceReview,
                InvestigationWorkFocus::EntityConnection { from: _, to: _ },
            ) => false,
        };
        if !focus_is_valid
            || work.scheduled_at() > state.now()
            || work.due_at() <= work.scheduled_at()
            || work.source_evidence().iter().any(|source| {
                state.legal.get_evidence(*source).is_none_or(|evidence| {
                    evidence.investigation() != work.investigation()
                        || evidence.discovered_at() > work.scheduled_at()
                })
            })
        {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        match work.status() {
            InvestigationWorkStatus::Scheduled => {
                if work.version() != 1
                    || work.resolution().is_some()
                    || investigation.status() != InvestigationStatus::Active
                    || investigation
                        .investigator_role(work.investigator())
                        .is_none()
                    || investigator.lifecycle() != Lifecycle::Active
                    || investigator.organization() != Some(investigation.owner())
                    || investigator
                        .capability(CapabilityKind::Investigation)
                        .is_none()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkStatus::Completed => {
                let resolution = work
                    .resolution()
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                if work.version() != 2
                    || resolution.resolved_at() < work.due_at()
                    || resolution.resolved_at() > state.now()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                match resolution.outcome() {
                    InvestigationWorkOutcome::Connected => {
                        if work.kind() != InvestigationWorkKind::PatternAnalysis
                            || resolution.superseded_by().is_some()
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let derived_id = resolution.derived_evidence().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if !derived_evidence_from_work.insert(derived_id) {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let derived = state.legal.get_evidence(derived_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if derived.investigation() != work.investigation()
                            || derived.custodian() != investigation.owner()
                            || derived.kind() != EvidenceKind::PatternLink
                            || derived.origin() != Some(work.focus().from())
                            || derived.subject() != work.focus().to()
                            || derived.discovered_at() != resolution.resolved_at()
                            || derived.derived_from() != work.source_evidence()
                            || work
                                .source_evidence()
                                .iter()
                                .any(|source| *source >= derived_id)
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                    InvestigationWorkOutcome::Developed => {
                        if work.kind() != InvestigationWorkKind::EvidenceReview
                            || resolution.superseded_by().is_some()
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let source_id = work.focus().evidence_id().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let source = state.legal.get_evidence(source_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let derived_id = resolution.derived_evidence().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if !derived_evidence_from_work.insert(derived_id) {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let derived = state.legal.get_evidence(derived_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if derived.investigation() != work.investigation()
                            || derived.custodian() != investigation.owner()
                            || derived.kind() != EvidenceKind::ForensicAnalysis
                            || derived.subject() != source.subject()
                            || derived.origin() != source.origin()
                            || derived.strength() != source.strength()
                            || derived.reliability()
                                != improve_evidence_reliability(source.reliability())
                            || derived.admissibility() != source.admissibility()
                            || derived.discovered_at() != resolution.resolved_at()
                            || derived.derived_from() != &BTreeSet::from([source_id])
                            || source_id >= derived_id
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                    InvestigationWorkOutcome::Inconclusive => {
                        if resolution.superseded_by().is_some()
                            || resolution.derived_evidence().is_some()
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                    InvestigationWorkOutcome::Superseded => {
                        if resolution.derived_evidence().is_some() {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let superseding_id = resolution.superseded_by().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let superseding = state.legal.get_evidence(superseding_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let valid_superseding = match (work.kind(), work.focus()) {
                            (
                                InvestigationWorkKind::PatternAnalysis,
                                InvestigationWorkFocus::EntityConnection { from, to },
                            ) => superseding.origin().is_some_and(|origin| {
                                (origin == from && superseding.subject() == to)
                                    || (origin == to && superseding.subject() == from)
                            }),
                            (
                                InvestigationWorkKind::EvidenceReview,
                                InvestigationWorkFocus::Evidence(source),
                            ) => {
                                superseding.kind() == EvidenceKind::ForensicAnalysis
                                    && superseding.derived_from() == &BTreeSet::from([source])
                            }
                            (
                                InvestigationWorkKind::PatternAnalysis,
                                InvestigationWorkFocus::Evidence(_),
                            )
                            | (
                                InvestigationWorkKind::EvidenceReview,
                                InvestigationWorkFocus::EntityConnection { from: _, to: _ },
                            ) => false,
                        };
                        if superseding.investigation() != work.investigation()
                            || superseding.discovered_at() > resolution.resolved_at()
                            || !valid_superseding
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                }
            }
        }
    }

    for witness in state.legal.case_witnesses() {
        let investigation = state
            .legal
            .get_investigation(witness.investigation())
            .ok_or(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            })?;
        if state.world.get_character(witness.witness()).is_none()
            || witness.registered_at() < investigation.opened_at()
            || witness.registered_at() > state.now()
            || witness.version() == 0
        {
            return Err(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            });
        }
        match witness.cooperation() {
            WitnessCooperation::Hostile
            | WitnessCooperation::Reluctant
            | WitnessCooperation::Cooperative => {}
        }
    }

    let mut named_witness_evidence = BTreeSet::new();
    for statement in state.legal.witness_statements() {
        let case_witness = state
            .legal
            .get_case_witness(statement.case_witness())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        let investigation = state
            .legal
            .get_investigation(case_witness.investigation())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        if statement.summary().trim().is_empty()
            || statement.recorded_at() < case_witness.registered_at()
            || statement.recorded_at() > state.now()
            || !is_entity_present(state, statement.subject())
            || statement
                .origin()
                .is_some_and(|origin| !is_entity_present(state, origin))
            || !named_witness_evidence.insert(statement.evidence())
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
        let evidence = state.legal.get_evidence(statement.evidence()).ok_or(
            StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            },
        )?;
        if evidence.investigation() != case_witness.investigation()
            || evidence.custodian() != investigation.owner()
            || evidence.subject() != statement.subject()
            || evidence.origin() != statement.origin()
            || evidence.source() != Some(EntityRef::Character(case_witness.witness()))
            || evidence.kind() != EvidenceKind::WitnessTestimony
            || evidence.strength() != witness_strength(statement.confidence())
            || evidence.reliability() != witness_reliability(statement.confidence())
            || evidence.admissibility() != Admissibility::Unknown
            || evidence.discovered_at() != statement.recorded_at()
            || !evidence.derived_from().is_empty()
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
    }

    for informant in state.legal.informants() {
        let character = state.world.get_character(informant.character()).ok_or(
            StateValidationError::InvalidInformant {
                informant: informant.id(),
            },
        )?;
        let handler = state.world.get_organization(informant.handler()).ok_or(
            StateValidationError::InvalidInformant {
                informant: informant.id(),
            },
        )?;
        if !matches!(
            handler.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) || informant.established_at() > state.now()
            || informant.version() == 0
        {
            return Err(StateValidationError::InvalidInformant {
                informant: informant.id(),
            });
        }
        match informant.status() {
            InformantStatus::Active => {
                if informant.terminated_at().is_some()
                    || character.lifecycle() != Lifecycle::Active
                    || handler.lifecycle() != Lifecycle::Active
                    || character.organization() == Some(informant.handler())
                {
                    return Err(StateValidationError::InvalidInformant {
                        informant: informant.id(),
                    });
                }
            }
            InformantStatus::Terminated => {
                let terminated_at =
                    informant
                        .terminated_at()
                        .ok_or(StateValidationError::InvalidInformant {
                            informant: informant.id(),
                        })?;
                if terminated_at < informant.established_at() || terminated_at > state.now() {
                    return Err(StateValidationError::InvalidInformant {
                        informant: informant.id(),
                    });
                }
            }
        }
    }

    let mut informant_evidence = BTreeSet::new();
    for disclosure in state.legal.informant_disclosures() {
        let informant = state.legal.get_informant(disclosure.informant()).ok_or(
            StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        let investigation = state
            .legal
            .get_investigation(disclosure.investigation())
            .ok_or(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            })?;
        let information = state
            .intelligence
            .get_information(disclosure.source_information())
            .ok_or(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            })?;
        let evidence = state.legal.get_evidence(disclosure.evidence()).ok_or(
            StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        let after_termination = informant
            .terminated_at()
            .is_some_and(|terminated_at| disclosure.disclosed_at() > terminated_at);
        if investigation.owner() != informant.handler()
            || information.holder() != KnowledgeHolder::Character(informant.character())
            || information.recorded_at() > disclosure.disclosed_at()
            || disclosure.disclosed_at() < informant.established_at()
            || disclosure.disclosed_at() < investigation.opened_at()
            || disclosure.disclosed_at() > state.now()
            || after_termination
            || !informant_evidence.insert(disclosure.evidence())
            || evidence.investigation() != disclosure.investigation()
            || evidence.custodian() != informant.handler()
            || evidence.subject() != information.subject()
            || evidence.origin().is_some()
            || evidence.source() != Some(EntityRef::Character(informant.character()))
            || evidence.kind() != EvidenceKind::InformantStatement
            || evidence.strength() != informant_strength(information.specificity())
            || evidence.reliability() != informant_reliability(information.reliability())
            || evidence.admissibility() != Admissibility::Unknown
            || evidence.discovered_at() != disclosure.disclosed_at()
            || !evidence.derived_from().is_empty()
        {
            return Err(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            });
        }
    }

    for evidence in state.legal.all_evidence() {
        let investigation = state
            .legal
            .get_investigation(evidence.investigation())
            .ok_or(StateValidationError::MissingEntity {
                context: "evidence investigation",
                entity: EntityRef::Investigation(evidence.investigation()),
            })?;
        if state.world.get_organization(evidence.custodian()).is_none()
            || evidence.custodian() != investigation.owner()
        {
            return Err(StateValidationError::MissingEntity {
                context: "evidence custodian",
                entity: EntityRef::Organization(evidence.custodian()),
            });
        }
        if !is_entity_present(state, evidence.subject()) {
            return Err(StateValidationError::MissingEntity {
                context: "evidence subject",
                entity: evidence.subject(),
            });
        }
        if let Some(origin) = evidence.origin() {
            if !is_entity_present(state, origin) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence origin",
                    entity: origin,
                });
            }
        }
        if let Some(source) = evidence.source() {
            if !is_entity_present(state, source) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence source",
                    entity: source,
                });
            }
            let valid_source = matches!(source, EntityRef::Character(_))
                && match evidence.kind() {
                    EvidenceKind::WitnessTestimony => {
                        named_witness_evidence.contains(&evidence.id())
                            && !informant_evidence.contains(&evidence.id())
                    }
                    EvidenceKind::InformantStatement => {
                        informant_evidence.contains(&evidence.id())
                            && !named_witness_evidence.contains(&evidence.id())
                    }
                    EvidenceKind::VehicleDescription
                    | EvidenceKind::Fingerprint
                    | EvidenceKind::RecoveredProperty
                    | EvidenceKind::FinancialRecord
                    | EvidenceKind::Surveillance
                    | EvidenceKind::CommunicationRecord
                    | EvidenceKind::KnownAssociation
                    | EvidenceKind::Document
                    | EvidenceKind::Ballistics
                    | EvidenceKind::PatternLink
                    | EvidenceKind::ForensicAnalysis => false,
                };
            if !valid_source {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        } else if named_witness_evidence.contains(&evidence.id())
            || informant_evidence.contains(&evidence.id())
        {
            return Err(StateValidationError::InvalidEvidenceProvenance {
                evidence: evidence.id(),
            });
        }
        if evidence.discovered_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "evidence",
            });
        }
        match evidence.kind() {
            EvidenceKind::PatternLink => {
                if evidence.source().is_some()
                    || evidence.derived_from().len() < 2
                    || !derived_evidence_from_work.contains(&evidence.id())
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::ForensicAnalysis => {
                if evidence.source().is_some()
                    || evidence.derived_from().len() != 1
                    || !derived_evidence_from_work.contains(&evidence.id())
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::InformantStatement => {
                if !informant_evidence.contains(&evidence.id())
                    || evidence.source().is_none()
                    || !evidence.derived_from().is_empty()
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::WitnessTestimony
            | EvidenceKind::VehicleDescription
            | EvidenceKind::Fingerprint
            | EvidenceKind::RecoveredProperty
            | EvidenceKind::FinancialRecord
            | EvidenceKind::Surveillance
            | EvidenceKind::CommunicationRecord
            | EvidenceKind::KnownAssociation
            | EvidenceKind::Document
            | EvidenceKind::Ballistics => {
                if !evidence.derived_from().is_empty() {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
        }
        for source_id in evidence.derived_from() {
            let source = state.legal.get_evidence(*source_id).ok_or(
                StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                },
            )?;
            if *source_id >= evidence.id()
                || source.investigation() != evidence.investigation()
                || source.discovered_at() > evidence.discovered_at()
            {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        }
    }

    for report in state.reports.reports() {
        if state.world.get_organization(report.recipient()).is_none() {
            return Err(StateValidationError::MissingEntity {
                context: "report recipient",
                entity: EntityRef::Organization(report.recipient()),
            });
        }
        if report.generated_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp { context: "report" });
        }
        for entry in report.entries() {
            for information in &entry.sources {
                let information_record = state.intelligence.get_information(*information).ok_or(
                    StateValidationError::MissingReportInformation {
                        report: report.id(),
                        information: *information,
                    },
                )?;
                let is_available = match information_record.holder() {
                    KnowledgeHolder::Organization(organization) => {
                        organization == report.recipient()
                    }
                    KnowledgeHolder::Character(_) => false,
                };
                if !is_available {
                    return Err(StateValidationError::ReportInformationUnavailable {
                        report: report.id(),
                        information: *information,
                    });
                }
            }
            for entity in &entry.entities {
                if !is_entity_present(state, *entity) {
                    return Err(StateValidationError::MissingEntity {
                        context: "report entry",
                        entity: *entity,
                    });
                }
            }
            if let Some(decision) = entry.decision {
                let decision_record = state.decisions.get_decision(decision).ok_or(
                    StateValidationError::MissingReportDecision {
                        report: report.id(),
                        decision,
                    },
                )?;
                if decision_record.recipient() != report.recipient() {
                    return Err(StateValidationError::ReportDecisionRecipientMismatch {
                        report: report.id(),
                        decision,
                    });
                }
            }
        }
    }

    for event in state.history.events() {
        if event.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "history event",
            });
        }
        for entity in event.entities() {
            if !is_entity_present(state, *entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "history event",
                    entity: *entity,
                });
            }
        }
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
