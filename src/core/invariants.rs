//! Runtime invariant enforcement and release-safe structural state validation.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    BusinessCycleId, BusinessId, CharacterId, DecisionRequestId, EnterpriseCycleId, EnterpriseId,
    InformationId, LedgerTransactionId, MandateId, OperationId, OrganizationId, ReportId,
};
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::decisions::{DecisionResponse, DecisionStatus};
use crate::delegation::{MandateStatus, ResponsibilityFunction, ResponsibilityScope};
use crate::economy::BusinessOperatingStatus;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::{AccountKind, AccountLifecycle, FinancialOwner, Money};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::{EvidenceReliability, EvidenceStrength};
use crate::operations::operation_execution::{
    calculate_execution_margin, calculate_exposure_score, calculate_intelligence_factors,
    classify_exposure_level, classify_objective_outcome,
};
use crate::operations::operation_system::is_information_subject_relevant;
use crate::operations::{
    OperationConstraint, OperationContingency, OperationExposureLevel, OperationStatus,
};
use crate::registry::Registry;
use crate::world::{
    BusinessFunction, BusinessOwner, Lifecycle, OrganizationKind, PolicyKind, ALL_POLICY_KINDS,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StateValidationError {
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
    #[error("completed operation {operation} has an invalid campaign-history link")]
    InvalidOperationHistory { operation: OperationId },
    #[error("operation {operation} is incompatible with its authored definition")]
    InvalidOperationDefinition { operation: OperationId },
    #[error("operation {operation} has invalid persisted exposure or legal consequences")]
    InvalidOperationExposure { operation: OperationId },
    #[error("organization {organization} has invalid legal jurisdiction state")]
    InvalidLegalJurisdiction { organization: OrganizationId },
    #[error("decision {decision} has an invalid attention class")]
    InvalidDecisionAttention { decision: DecisionRequestId },
    #[error("decision {decision} has an empty summary")]
    EmptyDecisionSummary { decision: DecisionRequestId },
    #[error("decision {decision} has no available responses")]
    DecisionHasNoResponses { decision: DecisionRequestId },
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
    #[error("business {business} has invalid operating economy state")]
    InvalidBusinessEconomy { business: BusinessId },
    #[error("business {business} has invalid operating economy account configuration")]
    InvalidBusinessEconomyAccounts { business: BusinessId },
    #[error("business {business} has invalid operating economy scheduling state")]
    InvalidBusinessEconomySchedule { business: BusinessId },
    #[error("business cycle {cycle} has invalid economics or provenance")]
    InvalidBusinessCycle { cycle: BusinessCycleId },
}

pub fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_indexes(state)?;
    validate_world_state(state)?;
    validate_social_and_intelligence(state)?;
    validate_operations(state)?;
    validate_decisions(state)?;
    validate_delegation(state)?;
    validate_business_economies(state)?;
    validate_enterprises(state)?;
    validate_legal_reports_and_history(state)?;
    Ok(())
}

pub fn validate_state_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for operation in state.operations.operations() {
        let definition = registry.get_operation(operation.kind());
        let execution = definition.execution();
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
        {
            return Err(StateValidationError::InvalidOperationDefinition {
                operation: operation.id(),
            });
        }
        if let Some(resolution) = operation.resolution() {
            let factors = resolution.factors();
            let expected_margin = calculate_execution_margin(execution, factors);
            let expected_outcome = classify_objective_outcome(execution, expected_margin);
            let (expected_intelligence_quality, expected_intelligence_adjustment) =
                calculate_intelligence_factors(registry, state, operation.id());
            if factors.variance().unsigned_abs() > execution.variance_limit()
                || factors.time_pressure() > 30
                || factors.approach_adjustment()
                    != execution
                        .approach_difficulty_adjustment(operation.approach())
                        .expect("validated operation approach must have an execution adjustment")
                || factors.intelligence_quality() != expected_intelligence_quality
                || factors.intelligence_adjustment() != expected_intelligence_adjustment
                || resolution.execution_margin() != expected_margin
                || resolution.objective_outcome() != expected_outcome
            {
                return Err(StateValidationError::InvalidOperationDefinition {
                    operation: operation.id(),
                });
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
        let EnterpriseLocation::Business(business_id) = enterprise.location() else {
            continue;
        };
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
    Ok(())
}

fn validate_indexes(state: &AppState) -> Result<(), StateValidationError> {
    let checks = [
        ("world", state.world.has_consistent_indexes()),
        ("finance", state.finance.has_consistent_indexes()),
        ("social", state.social.has_consistent_indexes()),
        ("intelligence", state.intelligence.has_consistent_indexes()),
        ("operations", state.operations.has_consistent_indexes()),
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
            return Err(StateValidationError::InvalidInformationProvenance {
                information: information.id(),
                source_information: *information
                    .derived_from()
                    .iter()
                    .next()
                    .expect("non-empty provenance must contain a source"),
            });
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

fn validate_operations(state: &AppState) -> Result<(), StateValidationError> {
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
        match operation.status() {
            OperationStatus::Authorized => {
                if operation.started_at().is_some()
                    || operation.resolution_due_at().is_some()
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
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
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
                    });
                }
                let valid_information = state
                    .intelligence
                    .get_information(resolution.after_action_information())
                    .is_some_and(|information| {
                        information.holder()
                            == KnowledgeHolder::Organization(operation.responsible_organization())
                            && information.source_kind() == InformationSourceKind::AfterAction
                            && information.topic() == InformationTopic::OperationalOutcome
                            && information.source_entity()
                                == Some(EntityRef::Character(operation.leader()))
                            && information.subject() == EntityRef::Operation(operation.id())
                            && information.observed_at() == resolution.resolved_at()
                    });
                if !valid_information {
                    return Err(StateValidationError::InvalidOperationAfterAction {
                        operation: operation.id(),
                    });
                }
                let valid_history = state
                    .history
                    .get_event(resolution.history_event())
                    .is_some_and(|event| {
                        event.kind() == HistoryEventKind::Operation
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
                validate_operation_exposure_links(state, operation, resolution)?;
            }
            OperationStatus::Aborted => {
                let execution_times_match =
                    operation.started_at().is_some() == operation.resolution_due_at().is_some();
                if !execution_times_match
                    || operation.awaiting_decision_since().is_some()
                    || operation.resolution().is_some()
                {
                    return Err(StateValidationError::InvalidOperationRuntime {
                        operation: operation.id(),
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

        let operation_id = decision.context().operation();
        let operation = state.operations.get_operation(operation_id).ok_or(
            StateValidationError::MissingEntity {
                context: "decision operation",
                entity: EntityRef::Operation(operation_id),
            },
        )?;
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
            }
            DecisionStatus::Resolved => {
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
                match resolution.response() {
                    DecisionResponse::Continue => {
                        if operation.status() == OperationStatus::AwaitingDecision {
                            return Err(StateValidationError::PendingDecisionOperationMismatch {
                                decision: decision.id(),
                                operation: operation_id,
                                status: operation.status(),
                            });
                        }
                    }
                    DecisionResponse::Abort => {
                        if operation.status() != OperationStatus::Aborted {
                            return Err(StateValidationError::AbortDecisionOperationMismatch {
                                decision: decision.id(),
                                operation: operation_id,
                            });
                        }
                    }
                }
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
        if cycle.occurred_at() < economy.established_at()
            || cycle.occurred_at() > state.now()
            || cycle.gross_revenue().cents() < 0
            || cycle.operating_cost().cents() < 0
            || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
        {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
        let expected_holder = match business.owner() {
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
        for subject in investigation.subjects() {
            if !is_entity_present(state, *subject) {
                return Err(StateValidationError::MissingEntity {
                    context: "investigation subject",
                    entity: *subject,
                });
            }
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
        if evidence.discovered_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "evidence",
            });
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

    state.world.debug_validate_indexes();
    state.finance.debug_validate_indexes();
    state.social.debug_validate_indexes();
    state.intelligence.debug_validate_indexes();
    state.operations.debug_validate_indexes();
    state.decisions.debug_validate_indexes();
    state.delegation.debug_validate_indexes();
    state.economy.debug_validate_indexes();
    state.enterprises.debug_validate_indexes();
    state.legal.debug_validate_indexes();
    state.reports.debug_validate_indexes();
    state.history.debug_validate_indexes();
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
        } else {
            debug_assert!(
                information.derived_from().is_empty(),
                "Knowledge Provenance: original information must not contain derived lineage"
            );
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

    for decision in state.decisions.decisions() {
        debug_assert!(
            state.world.get_organization(decision.recipient()).is_some(),
            "Record Reference Validity: decision recipient does not exist"
        );
        debug_assert!(
            state.world.get_character(decision.requester()).is_some(),
            "Record Reference Validity: decision requester does not exist"
        );
        debug_assert!(
            !decision.summary().trim().is_empty(),
            "Lifecycle Validity: decision summary is empty"
        );
        debug_assert!(
            !decision.options().is_empty(),
            "Lifecycle Validity: decision has no available responses"
        );
        match decision.attention() {
            AttentionClass::Exception | AttentionClass::Crisis => {}
            AttentionClass::Routine | AttentionClass::Notable => debug_assert!(
                false,
                "Lifecycle Validity: pending authority decision has non-interrupting attention"
            ),
        }
        debug_assert!(
            decision.requested_at() <= state.now(),
            "Lifecycle Validity: decision request occurs in the future"
        );

        let operation_id = decision.context().operation();
        let operation = state
            .operations
            .get_operation(operation_id)
            .expect("Record Reference Validity: decision operation does not exist");
        debug_assert_eq!(
            operation.leader(),
            decision.requester(),
            "Ownership Exclusivity: decision requester is not operation leader"
        );
        debug_assert_eq!(
            operation.responsible_organization(),
            decision.recipient(),
            "Ownership Exclusivity: decision recipient does not own operation"
        );

        match decision.status() {
            DecisionStatus::Pending => {
                debug_assert_eq!(
                    operation.status(),
                    OperationStatus::AwaitingDecision,
                    "Lifecycle Validity: pending decision operation is not awaiting input"
                );
                debug_assert_eq!(
                    state.decisions.pending_for_operation(operation_id),
                    Some(decision.id()),
                    "Index Completeness: pending operation decision index is missing decision"
                );
            }
            DecisionStatus::Resolved => {
                let resolution = decision
                    .resolution()
                    .expect("Lifecycle Validity: resolved decision has no resolution");
                debug_assert!(
                    resolution.resolved_at() >= decision.requested_at()
                        && resolution.resolved_at() <= state.now(),
                    "Lifecycle Validity: decision resolution chronology is invalid"
                );
                debug_assert_eq!(
                    resolution.resolved_by(),
                    decision.recipient(),
                    "Ownership Exclusivity: decision was resolved by a foreign organization"
                );
                debug_assert!(
                    decision.options().contains(&resolution.response()),
                    "Lifecycle Validity: decision resolution was not an offered response"
                );
                match resolution.response() {
                    DecisionResponse::Continue => debug_assert_ne!(
                        operation.status(),
                        OperationStatus::AwaitingDecision,
                        "Lifecycle Validity: resolved continue decision left operation awaiting input"
                    ),
                    DecisionResponse::Abort => debug_assert_eq!(
                        operation.status(),
                        OperationStatus::Aborted,
                        "Lifecycle Validity: resolved abort decision did not abort operation"
                    ),
                }
            }
        }
    }

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
