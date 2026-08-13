//! Runtime invariant enforcement and release-safe structural state validation.

use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    CharacterId, DecisionRequestId, InformationId, LedgerTransactionId, MandateId, OperationId,
    OrganizationId, ReportId,
};
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::decisions::{DecisionResponse, DecisionStatus};
use crate::delegation::{MandateStatus, ResponsibilityScope};
use crate::finance::{AccountLifecycle, FinancialOwner};
use crate::intelligence::KnowledgeHolder;
use crate::operations::{OperationConstraint, OperationContingency, OperationStatus};
use crate::world::{BusinessOwner, Lifecycle, OrganizationKind, PolicyKind, ALL_POLICY_KINDS};
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
    #[error("active operation {operation} belongs to an inactive organization")]
    ActiveOperationInactiveOrganization { operation: OperationId },
    #[error("active operation {operation} has an inactive or foreign leader")]
    ActiveOperationInvalidLeader { operation: OperationId },
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
}

pub fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
    validate_indexes(state)?;
    validate_world_state(state)?;
    validate_social_and_intelligence(state)?;
    validate_operations(state)?;
    validate_decisions(state)?;
    validate_delegation(state)?;
    validate_legal_reports_and_history(state)?;
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
            if state.delegation.get_mandate(usage.mandate()).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "ledger budget mandate",
                    entity: EntityRef::Mandate(usage.mandate()),
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
        if leader.organization() != Some(operation.responsible_organization()) {
            return Err(StateValidationError::ActiveOperationInvalidLeader {
                operation: operation.id(),
            });
        }
        for participant in operation.roles().values() {
            if state.world.get_character(*participant).is_none() {
                return Err(StateValidationError::MissingEntity {
                    context: "operation participant",
                    entity: EntityRef::Character(*participant),
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
            }
            OperationStatus::Completed | OperationStatus::Aborted => {}
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

fn validate_legal_reports_and_history(state: &AppState) -> Result<(), StateValidationError> {
    for investigation in state.legal.investigations() {
        if state
            .world
            .get_organization(investigation.owner())
            .is_none()
        {
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
        if state
            .legal
            .get_investigation(evidence.investigation())
            .is_none()
        {
            return Err(StateValidationError::MissingEntity {
                context: "evidence investigation",
                entity: EntityRef::Investigation(evidence.investigation()),
            });
        }
        if state.world.get_organization(evidence.custodian()).is_none() {
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
                if state.intelligence.get_information(*information).is_none() {
                    return Err(StateValidationError::MissingReportInformation {
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
    state.legal.debug_validate_indexes();
    state.reports.debug_validate_indexes();
    state.history.debug_validate_indexes();

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
            debug_assert!(
                state.delegation.get_mandate(usage.mandate()).is_some(),
                "Record Reference Validity: ledger budget mandate does not exist"
            );
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
        for participant in operation.roles().values() {
            debug_assert!(
                state.world.get_character(*participant).is_some(),
                "Record Reference Validity: operation participant does not exist"
            );
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
    }

    for report in state.reports.reports() {
        debug_assert!(
            state.world.get_organization(report.recipient()).is_some(),
            "Record Reference Validity: report recipient does not exist"
        );
        for entry in report.entries() {
            for source in &entry.sources {
                debug_assert!(
                    state.intelligence.get_information(*source).is_some(),
                    "Record Reference Validity: report source information does not exist"
                );
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
