//! Release-safe structural validation for the decisions and delegation subsystems.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::OperationId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::decisions::{
    DecisionContext, DecisionResponse, DecisionStatus, OperationExceptionReason,
};
use crate::delegation::{MandateStatus, ResponsibilityFunction, ResponsibilityScope};
use crate::finance::{AccountLifecycle, FinancialOwner};
use crate::legal::PoliceResponseStatus;
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationContingency, OperationStatus,
};
use crate::recruitment::RecruitmentPolicySource;
use crate::world::{Lifecycle, OrganizationKind};

pub(super) fn validate_decisions(state: &AppState) -> Result<(), StateValidationError> {
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

pub(super) fn validate_delegation(state: &AppState) -> Result<(), StateValidationError> {
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