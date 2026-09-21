//! Release-safe structural validation for delegated authority and mandate budgets.

use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::delegation_system::{
    BudgetFundingAccountError, ResponsibilityScopeLivenessError, required_scope_for_policy,
    validate_budget_funding_account, validate_responsibility_scope_liveness,
};
use crate::delegation::{MandateRecord, MandateStatus, ResponsibilityScope};

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
        let required_scope = required_scope_for_policy(*kind);
        if !mandate.scopes().contains(&required_scope) {
            return Err(StateValidationError::MandatePolicyOutsideScope {
                mandate: mandate.id(),
                policy: *kind,
                required_scope,
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
    validate_responsibility_scope_liveness(state, scope).map_err(|error| match error {
        ResponsibilityScopeLivenessError::MissingNeighborhood(id) => {
            StateValidationError::MissingEntity {
                context: "mandate neighborhood scope",
                entity: EntityRef::Neighborhood(id),
            }
        }
        ResponsibilityScopeLivenessError::MissingBusiness(id) => {
            StateValidationError::MissingEntity {
                context: "mandate business scope",
                entity: EntityRef::Business(id),
            }
        }
    })
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
    validate_budget_funding_account(state, mandate.organization(), budget.funding_account).map_err(
        |error| match error {
            BudgetFundingAccountError::Missing(account) => StateValidationError::MissingEntity {
                context: "mandate budget account",
                entity: EntityRef::FinancialAccount(account),
            },
            BudgetFundingAccountError::OwnerMismatch(account)
            | BudgetFundingAccountError::InvalidKind(account) => {
                StateValidationError::MandateBudgetAccountOwnerMismatch {
                    mandate: mandate.id(),
                    account,
                }
            }
        },
    )
}
