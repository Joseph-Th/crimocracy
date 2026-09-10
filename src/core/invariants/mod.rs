//! Runtime invariant enforcement and release-safe structural state validation.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, BusinessCycleId, BusinessId, CaseWitnessId, CharacterId, ContactDisclosureId,
    ContactId, DecisionRequestId, EnterpriseCycleId, EnterpriseId, FinancialAccountId,
    HistoryEventId, InformantDisclosureId, InformantId, InformationId, InvestigationId,
    InvestigationWorkId, LedgerTransactionId, LegalRepresentationId, MandateId, OperationId,
    OpportunityId, OrganizationId, PatrolDeploymentId, PoliceResponseId, ProsecutionCaseId,
    ProsecutionReferralId, RecruitmentAttemptId, ReportId, WitnessStatementId,
};
use crate::core::state::AppState;
#[cfg(debug_assertions)]
use crate::core::state::CURRENT_STATE_SCHEMA_VERSION;
use crate::core::time::SimTime;
use crate::decisions::DecisionResponse;
use crate::operations::OperationStatus;
use crate::registry::Registry;
use crate::world::{BusinessFunction, PolicyKind};
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
    #[error("persisted entity {entity:?} has an empty name")]
    EmptyEntityName { entity: EntityRef },
    #[error("character {character} has invalid persisted version 0")]
    InvalidCharacterVersion { character: CharacterId },
    #[error("organization {organization:?} stores an out-of-range {audience:?} reputation score")]
    InvalidReputationScore {
        organization: OrganizationId,
        audience: crate::reputation::AudienceKind,
    },
    #[error("organization {organization:?} stores future-dated {audience:?} reputation movement")]
    InvalidReputationChronology {
        organization: OrganizationId,
        audience: crate::reputation::AudienceKind,
    },
    #[error("organization {organization:?} stores a neutral {audience:?} reputation record")]
    NeutralReputationRecord {
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
    #[error("organization {organization} policy {policy:?} has invalid persisted version/history")]
    InvalidOrganizationPolicyVersion {
        organization: OrganizationId,
        policy: PolicyKind,
    },
    #[error("character {character} and supervisor {supervisor} belong to different organizations")]
    SupervisorOrganizationMismatch {
        character: CharacterId,
        supervisor: CharacterId,
    },
    #[error("supervision hierarchy contains a cycle involving character {character}")]
    SupervisionCycle { character: CharacterId },
    #[error("relationship {from}->{to} has invalid persisted state")]
    InvalidRelationship { from: CharacterId, to: CharacterId },
    #[error("information {information} has an empty summary")]
    EmptyInformationSummary { information: InformationId },
    #[error("information {information} has invalid observation/recording chronology")]
    InvalidInformationChronology { information: InformationId },
    #[error("information {information} has an invalid typed semantic signal")]
    InvalidInformationSignal { information: InformationId },
    #[error("information {information} has invalid provenance source {source_information}")]
    InvalidInformationProvenance {
        information: InformationId,
        source_information: InformationId,
    },
    #[error("institutional contact {contact} has invalid persisted state")]
    InvalidInstitutionalContact { contact: ContactId },
    #[error("institutional contact disclosure {disclosure} has invalid persisted provenance")]
    InvalidContactDisclosure { disclosure: ContactDisclosureId },
    #[error("active operation {operation} has a leader outside its responsible organization")]
    ActiveOperationInvalidLeader { operation: OperationId },
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
    #[error("aborted operation {operation} has invalid abort provenance or after-action artifacts")]
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
    #[error("investigation {investigation} has invalid persisted definition")]
    InvalidInvestigationDefinition { investigation: InvestigationId },
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
    #[error("mandate {mandate} has invalid persisted version 0")]
    InvalidMandateVersion { mandate: MandateId },
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
    #[error("campaign attention auto-pause contains invalid class {attention:?}")]
    InvalidAttentionPreference { attention: AttentionClass },
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
    #[error("report {report} has an empty title")]
    EmptyReportTitle { report: ReportId },
    #[error("executive brief {report} has invalid persisted cadence or shape")]
    InvalidExecutiveBrief { report: ReportId },
    #[error("report {report} entry {entry} has an empty summary")]
    EmptyReportEntrySummary { report: ReportId, entry: usize },
    #[error("history event {event} has an empty summary")]
    EmptyHistorySummary { event: HistoryEventId },
    #[error("history event {event} references no entities")]
    HistoryEventHasNoEntities { event: HistoryEventId },
    #[error("{context} contains a timestamp later than the current simulation time")]
    FutureTimestamp { context: &'static str },
    #[error("financial account balances do not match their ledger postings")]
    FinancialBalanceMismatch,
    #[error("financial account {account} has an invalid persisted version")]
    InvalidFinancialAccount { account: FinancialAccountId },
    #[error("ledger transaction {transaction} postings overflow while summing")]
    LedgerArithmeticOverflow {
        transaction: crate::core::id::LedgerTransactionId,
    },
    #[error("ledger transaction {transaction} has an invalid persisted memo or posting shape")]
    InvalidLedgerTransaction { transaction: LedgerTransactionId },
    #[error(
        "ledger transaction {transaction} at {occurred_at:?} precedes the prior committed ledger time {previous_at:?}"
    )]
    InvalidLedgerTransactionChronology {
        transaction: LedgerTransactionId,
        previous_at: SimTime,
        occurred_at: SimTime,
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
    #[error("enterprise {enterprise} has invalid persisted runtime state")]
    InvalidEnterpriseRuntime { enterprise: EnterpriseId },
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
mod enterprise;
mod finance;
mod id_allocators;
mod legal;
mod operations;
mod opportunities;
mod recruitment;
mod reports;
mod reputation;
mod world;

use self::business::{validate_business_economies, validate_businesses_against_registry};
use self::decisions::{validate_decisions, validate_delegation};
use self::enterprise::{validate_enterprises, validate_enterprises_against_registry};
use self::id_allocators::validate_id_allocators;
use self::legal::{validate_legal_subsystems, validate_legal_subsystems_against_registry};
use self::operations::validate_operations;
use self::opportunities::{validate_opportunities, validate_opportunities_against_registry};
use self::recruitment::{validate_recruitment, validate_recruitment_against_registry};
use self::reports::validate_executive_briefs_against_registry;
use self::reputation::{validate_reputations, validate_reputations_against_registry};
use self::world::{validate_contacts, validate_social_and_intelligence, validate_world_state};

pub fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_id_allocators(state)?;
    validate_indexes(state)?;
    validate_campaign(state)?;
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

fn validate_campaign(state: &AppState) -> Result<(), StateValidationError> {
    for attention in state.attention_settings().auto_pause.iter().copied() {
        if !matches!(
            attention,
            AttentionClass::Exception | AttentionClass::Crisis
        ) {
            return Err(StateValidationError::InvalidAttentionPreference { attention });
        }
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
    world::validate_organization_policies_against_registry(registry, state)?;
    validate_legal_subsystems_against_registry(registry, state)?;
    operations::validate_operations_against_registry(registry, state)?;
    validate_opportunities_against_registry(registry, state)?;
    validate_businesses_against_registry(registry, state)?;
    validate_enterprises_against_registry(registry, state)?;
    validate_recruitment_against_registry(registry, state)?;
    validate_reputations_against_registry(registry, state)?;
    validate_executive_briefs_against_registry(registry, state)?;
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
        ("history", state.history.has_consistent_indexes()),
    ];
    for (subsystem, is_consistent) in checks {
        if !is_consistent {
            return Err(StateValidationError::IndexInconsistency { subsystem });
        }
    }
    finance::validate_finance_indexes_and_ledger(state)?;
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
