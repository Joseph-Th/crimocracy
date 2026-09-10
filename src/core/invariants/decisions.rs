//! Release-safe structural validation for the decisions and delegation subsystems.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::OperationId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::decisions::{
    DecisionCancellationReason, DecisionContext, DecisionRequestRecord, DecisionResponse,
    DecisionStatus, RecruitmentApprovalContext,
};
use crate::delegation::{
    MandateRecord, MandateStatus, ResponsibilityFunction, ResponsibilityScope,
};
use crate::finance::FinancialOwner;
use crate::legal::PoliceResponseStatus;
use crate::operations::{
    OperationAbortCause, OperationAbortPhase, OperationContingency, OperationStatus,
};
use crate::recruitment::RecruitmentPolicySource;
use crate::world::{ApprovalPolicy, OrganizationKind, PolicyKind, PolicySetting};

pub(super) fn validate_decisions(state: &AppState) -> Result<(), StateValidationError> {
    for decision in state.decisions.decisions() {
        validate_decision(state, decision)?;
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

fn validate_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
    validate_decision_references(state, decision)?;
    validate_decision_definition(state, decision)?;
    validate_decision_lifecycle(state, decision)?;
    match decision.context() {
        DecisionContext::OperationPoliceArrival {
            operation,
            response,
        } => validate_operation_decision(state, decision, operation, response),
        DecisionContext::RecruitmentApproval(context) => {
            validate_recruitment_approval_decision(state, decision, context)
        }
    }
}

fn validate_decision_references(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
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
    Ok(())
}

fn validate_decision_definition(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
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
        return Err(invalid_decision_chronology(decision));
    }
    Ok(())
}

fn validate_decision_lifecycle(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
    match decision.status() {
        DecisionStatus::Pending => validate_pending_decision(decision),
        DecisionStatus::Resolved => validate_resolved_decision(state, decision),
        DecisionStatus::Cancelled => validate_cancelled_decision(state, decision),
    }
}

fn validate_pending_decision(decision: &DecisionRequestRecord) -> Result<(), StateValidationError> {
    if decision.version() != 1
        || decision.resolution().is_some()
        || decision.cancellation().is_some()
    {
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_resolved_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
    if decision.version() != 2 {
        return Err(invalid_decision_context(decision));
    }
    let resolution =
        decision
            .resolution()
            .ok_or(StateValidationError::ResolvedDecisionWithoutResolution {
                decision: decision.id(),
            })?;
    if decision.cancellation().is_some()
        || resolution.resolved_at() < decision.requested_at()
        || resolution.resolved_at() > state.now()
    {
        return Err(invalid_decision_chronology(decision));
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
    Ok(())
}

fn validate_cancelled_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
) -> Result<(), StateValidationError> {
    if decision.version() != 2 {
        return Err(invalid_decision_context(decision));
    }
    let cancellation = decision
        .cancellation()
        .ok_or_else(|| invalid_decision_context(decision))?;
    if decision.resolution().is_some()
        || cancellation.cancelled_at() < decision.requested_at()
        || cancellation.cancelled_at() > state.now()
    {
        return Err(invalid_decision_chronology(decision));
    }
    Ok(())
}

fn invalid_decision_context(decision: &DecisionRequestRecord) -> StateValidationError {
    StateValidationError::InvalidDecisionContext {
        decision: decision.id(),
    }
}

fn invalid_decision_chronology(decision: &DecisionRequestRecord) -> StateValidationError {
    StateValidationError::InvalidDecisionChronology {
        decision: decision.id(),
    }
}

fn validate_operation_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
    operation_id: OperationId,
    response_id: crate::core::id::PoliceResponseId,
) -> Result<(), StateValidationError> {
    let operation = state.operations.get_operation(operation_id).ok_or(
        StateValidationError::MissingEntity {
            context: "decision operation",
            entity: EntityRef::Operation(operation_id),
        },
    )?;
    validate_operation_decision_definition(decision, operation, operation_id)?;
    validate_operation_decision_response_link(
        state,
        decision,
        operation,
        operation_id,
        response_id,
    )?;
    match decision.status() {
        DecisionStatus::Pending => {
            validate_pending_operation_decision(state, decision, operation, operation_id)
        }
        DecisionStatus::Resolved => {
            validate_resolved_operation_decision(state, decision, operation, operation_id)
        }
        DecisionStatus::Cancelled => {
            validate_cancelled_operation_decision(decision, operation, operation_id)
        }
    }
}

fn validate_operation_decision_definition(
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
    operation_id: OperationId,
) -> Result<(), StateValidationError> {
    if decision.options().len() != 2
        || !decision.options().contains(&DecisionResponse::Continue)
        || !decision.options().contains(&DecisionResponse::Abort)
    {
        return Err(invalid_decision_context(decision));
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
        .contains(&OperationContingency::RequestDecisionOnPoliceArrival)
    {
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_operation_decision_response_link(
    state: &AppState,
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
    operation_id: OperationId,
    response_id: crate::core::id::PoliceResponseId,
) -> Result<(), StateValidationError> {
    let response = state
        .legal
        .get_police_response(response_id)
        .ok_or_else(|| invalid_decision_context(decision))?;
    let Some(arrived_at) = response.arrived_at() else {
        return Err(invalid_decision_context(decision));
    };
    let matching_decisions = state
        .decisions
        .decisions_for_operation(operation_id)
        .filter(|candidate| {
            matches!(
              candidate.context(),
              DecisionContext::OperationPoliceArrival {
                response: candidate_response,
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
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_pending_operation_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
    operation_id: OperationId,
) -> Result<(), StateValidationError> {
    if operation.status() != OperationStatus::AwaitingDecision {
        return Err(pending_operation_mismatch(decision, operation));
    }
    if state.decisions.pending_for_operation(operation_id) != Some(decision.id()) {
        return Err(StateValidationError::IndexInconsistency {
            subsystem: "decisions",
        });
    }
    if operation.awaiting_decision_since() != Some(decision.requested_at()) {
        return Err(pending_operation_mismatch(decision, operation));
    }
    Ok(())
}

fn validate_resolved_operation_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
    operation_id: OperationId,
) -> Result<(), StateValidationError> {
    let resolution =
        decision
            .resolution()
            .ok_or(StateValidationError::ResolvedDecisionWithoutResolution {
                decision: decision.id(),
            })?;
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
                            && operation.awaiting_decision_since() == Some(pending.requested_at())
                    });
                if !newer_pending {
                    return Err(pending_operation_mismatch(decision, operation));
                }
            }
            Ok(())
        }
        DecisionResponse::Abort => {
            let abort = operation.abort_record();
            if operation.status() != OperationStatus::Aborted
                || !abort.is_some_and(|abort| {
                    (abort.cause() == OperationAbortCause::Decision(decision.id())
                        || abort.cause() == OperationAbortCause::DeadlineMissed)
                        && abort.phase() == OperationAbortPhase::AwaitingDecision
                        && abort.aborted_at() == resolution.resolved_at()
                })
            {
                return Err(StateValidationError::AbortDecisionOperationMismatch {
                    decision: decision.id(),
                    operation: operation_id,
                });
            }
            Ok(())
        }
        DecisionResponse::Approve | DecisionResponse::Reject => {
            Err(invalid_decision_context(decision))
        }
    }
}

fn validate_cancelled_operation_decision(
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
    operation_id: OperationId,
) -> Result<(), StateValidationError> {
    let cancellation = decision
        .cancellation()
        .ok_or_else(|| invalid_decision_context(decision))?;
    let character = match cancellation.reason() {
        DecisionCancellationReason::OperationParticipantDetained(character) => character,
        DecisionCancellationReason::RecruitmentAuthorityChanged(_)
        | DecisionCancellationReason::RecruitmentOrganizationPolicyChanged(_) => {
            return Err(invalid_decision_context(decision));
        }
    };
    let abort = operation.abort_record();
    if operation.status() != OperationStatus::Aborted
        || !operation.participants().contains(&character)
        || !abort.is_some_and(|abort| {
            abort.cause() == OperationAbortCause::ParticipantDetained(character)
                && abort.phase() == OperationAbortPhase::AwaitingDecision
                && abort.aborted_at() == cancellation.cancelled_at()
        })
    {
        return Err(StateValidationError::AbortDecisionOperationMismatch {
            decision: decision.id(),
            operation: operation_id,
        });
    }
    Ok(())
}

fn pending_operation_mismatch(
    decision: &DecisionRequestRecord,
    operation: &crate::operations::OperationRecord,
) -> StateValidationError {
    StateValidationError::PendingDecisionOperationMismatch {
        decision: decision.id(),
        operation: operation.id(),
        status: operation.status(),
    }
}

fn validate_recruitment_approval_decision(
    state: &AppState,
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
) -> Result<(), StateValidationError> {
    validate_recruitment_approval_definition(decision, context)?;
    validate_recruitment_approval_authority(state, decision, context)?;
    validate_recruitment_approval_lifecycle(state, decision, context)
}

fn validate_recruitment_approval_definition(
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
) -> Result<(), StateValidationError> {
    if decision.options().len() != 2
        || !decision.options().contains(&DecisionResponse::Approve)
        || !decision.options().contains(&DecisionResponse::Reject)
        || decision.requester() != context.recruiter()
        || decision.recipient() != context.target_organization()
    {
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_recruitment_approval_authority(
    state: &AppState,
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
) -> Result<(), StateValidationError> {
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
        .ok_or_else(|| invalid_decision_context(decision))?;
    let valid_policy_source = match authority.policy_source() {
        RecruitmentPolicySource::Organization {
            organization: source,
            version,
        } => {
            source == context.target_organization()
                && organization.independent_recruitment_policy_at_version(version)
                    == Some(ApprovalPolicy::RequireApproval)
        }
        RecruitmentPolicySource::Mandate {
            mandate: source,
            version,
        } => source == mandate_authority.mandate && version == authority.mandate_version(),
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
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_recruitment_approval_lifecycle(
    state: &AppState,
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
) -> Result<(), StateValidationError> {
    let linked_attempt = state
        .recruitment
        .get_attempt_for_approval_decision(decision.id());
    match decision.status() {
        DecisionStatus::Pending => {
            validate_pending_recruitment_approval(state, decision, context, linked_attempt)
        }
        DecisionStatus::Resolved => {
            validate_resolved_recruitment_approval(decision, linked_attempt)
        }
        DecisionStatus::Cancelled => {
            validate_cancelled_recruitment_approval(state, decision, context, linked_attempt)
        }
    }
}

fn validate_cancelled_recruitment_approval(
    state: &AppState,
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
    linked_attempt: Option<&crate::recruitment::RecruitmentAttemptRecord>,
) -> Result<(), StateValidationError> {
    let cancellation = decision
        .cancellation()
        .ok_or_else(|| invalid_decision_context(decision))?;
    let authority = context.authority();
    let cancellation_is_valid = match cancellation.reason() {
        DecisionCancellationReason::RecruitmentAuthorityChanged(mandate_id) => {
            let mandate = state
                .delegation
                .get_mandate(mandate_id)
                .ok_or_else(|| invalid_decision_context(decision))?;
            authority.authority().mandate == mandate_id
                && authority.mandate_version() < mandate.version()
        }
        DecisionCancellationReason::RecruitmentOrganizationPolicyChanged(organization) => {
            context.target_organization() == organization
                && matches!(
                    authority.policy_source(),
                    RecruitmentPolicySource::Organization {
                        organization: source,
                        version,
                    } if source == organization
                        && state
                            .world
                            .get_organization(organization)
                            .and_then(|record| {
                                record.policy_version(PolicyKind::IndependentRecruitment)
                            })
                            .is_some_and(|current| current > version)
                )
        }
        DecisionCancellationReason::OperationParticipantDetained(_) => {
            return Err(invalid_decision_context(decision));
        }
    };
    if !cancellation_is_valid || linked_attempt.is_some() {
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_pending_recruitment_approval(
    state: &AppState,
    decision: &DecisionRequestRecord,
    context: RecruitmentApprovalContext,
    linked_attempt: Option<&crate::recruitment::RecruitmentAttemptRecord>,
) -> Result<(), StateValidationError> {
    let authority = context.authority();
    let mandate = state
        .delegation
        .get_mandate(authority.authority().mandate)
        .ok_or_else(|| invalid_decision_context(decision))?;
    let recruiter = state
        .world
        .get_character(context.recruiter())
        .ok_or_else(|| invalid_decision_context(decision))?;
    let organization = state
        .world
        .get_organization(context.target_organization())
        .ok_or_else(|| invalid_decision_context(decision))?;
    let (effective_policy, effective_source) = mandate
        .standing_orders()
        .get(&PolicyKind::IndependentRecruitment)
        .copied()
        .map(|setting| {
            (
                setting,
                RecruitmentPolicySource::Mandate {
                    mandate: mandate.id(),
                    version: mandate.version(),
                },
            )
        })
        .or_else(|| {
            let setting = organization.policy(PolicyKind::IndependentRecruitment)?;
            let version = organization.policy_version(PolicyKind::IndependentRecruitment)?;
            Some((
                setting,
                RecruitmentPolicySource::Organization {
                    organization: context.target_organization(),
                    version,
                },
            ))
        })
        .ok_or_else(|| invalid_decision_context(decision))?;
    if state
        .decisions
        .pending_for_recruitment_approval(context.target_organization(), context.candidate())
        != Some(decision.id())
        || linked_attempt.is_some()
        || mandate.status() != MandateStatus::Active
        || mandate.version() != authority.mandate_version()
        || recruiter.version() != authority.manager_version()
        || effective_policy
            != PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval)
        || effective_source != authority.policy_source()
    {
        return Err(invalid_decision_context(decision));
    }
    Ok(())
}

fn validate_resolved_recruitment_approval(
    decision: &DecisionRequestRecord,
    linked_attempt: Option<&crate::recruitment::RecruitmentAttemptRecord>,
) -> Result<(), StateValidationError> {
    let resolution =
        decision
            .resolution()
            .ok_or(StateValidationError::ResolvedDecisionWithoutResolution {
                decision: decision.id(),
            })?;
    match resolution.response() {
        DecisionResponse::Approve => {
            let attempt = linked_attempt.ok_or_else(|| invalid_decision_context(decision))?;
            if attempt.occurred_at() != resolution.resolved_at() {
                return Err(invalid_decision_context(decision));
            }
            Ok(())
        }
        DecisionResponse::Reject if linked_attempt.is_none() => Ok(()),
        DecisionResponse::Reject | DecisionResponse::Continue | DecisionResponse::Abort => {
            Err(invalid_decision_context(decision))
        }
    }
}

pub(super) fn validate_delegation(state: &AppState) -> Result<(), StateValidationError> {
    for mandate in state.delegation.mandates() {
        validate_mandate(state, mandate)?;
    }
    Ok(())
}

fn validate_mandate(state: &AppState, mandate: &MandateRecord) -> Result<(), StateValidationError> {
    validate_mandate_owner_and_manager(state, mandate)?;
    validate_mandate_policy_and_scopes(state, mandate)?;
    validate_mandate_budget(state, mandate)?;
    // Exhaustiveness tripwire: a new mandate status must be explicitly classified.
    match mandate.status() {
        MandateStatus::Active | MandateStatus::Revoked => {}
    }
    Ok(())
}

fn validate_mandate_owner_and_manager(
    state: &AppState,
    mandate: &MandateRecord,
) -> Result<(), StateValidationError> {
    if mandate.version() == 0 {
        return Err(StateValidationError::InvalidMandateVersion {
            mandate: mandate.id(),
        });
    }
    state.world.get_organization(mandate.organization()).ok_or(
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
    // Active authority requires a live manager inside the owning organization. Revoked
    // mandates are durable governance history and survive later canonical membership changes.
    if mandate.status() == MandateStatus::Active
        && manager.organization() != Some(mandate.organization())
    {
        return Err(StateValidationError::MandateManagerOrganizationMismatch {
            mandate: mandate.id(),
            manager: mandate.manager(),
        });
    }
    Ok(())
}

fn validate_mandate_policy_and_scopes(
    state: &AppState,
    mandate: &MandateRecord,
) -> Result<(), StateValidationError> {
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
        validate_mandate_scope(state, *scope)?;
    }
    Ok(())
}

fn validate_mandate_scope(
    state: &AppState,
    scope: ResponsibilityScope,
) -> Result<(), StateValidationError> {
    match scope {
        ResponsibilityScope::Neighborhood(id) if state.world.get_neighborhood(id).is_none() => {
            Err(StateValidationError::MissingEntity {
                context: "mandate neighborhood scope",
                entity: EntityRef::Neighborhood(id),
            })
        }
        ResponsibilityScope::Business(id) if state.world.get_business(id).is_none() => {
            Err(StateValidationError::MissingEntity {
                context: "mandate business scope",
                entity: EntityRef::Business(id),
            })
        }
        ResponsibilityScope::Neighborhood(_)
        | ResponsibilityScope::Business(_)
        | ResponsibilityScope::Function(_) => Ok(()),
    }
}

fn validate_mandate_budget(
    state: &AppState,
    mandate: &MandateRecord,
) -> Result<(), StateValidationError> {
    let Some(budget) = mandate.budget() else {
        return Ok(());
    };
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
    if account.owner() != FinancialOwner::Organization(mandate.organization())
        || account.kind() != crate::finance::AccountKind::AccountedFunds
    {
        return Err(StateValidationError::MandateBudgetAccountOwnerMismatch {
            mandate: mandate.id(),
            account: budget.funding_account,
        });
    }
    Ok(())
}
