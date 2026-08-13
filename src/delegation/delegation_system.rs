//! Mandate validation, lifecycle transactions, and policy resolution; sibling delegation state owns synchronized indexes.

use crate::core::id::{BusinessId, CharacterId, MandateId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::{
    build_mandate_record, BudgetAuthority, MandateDraft, MandateStatus, ResponsibilityScope,
};
use crate::finance::{AccountLifecycle, FinancialOwner};
use crate::registry::Registry;
use crate::world::{Lifecycle, PolicyKind, PolicySetting};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DelegationError {
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} is not active")]
    InactiveOrganization(OrganizationId),
    #[error("manager {0} does not exist")]
    MissingManager(CharacterId),
    #[error("manager {manager} is not an active member of organization {organization}")]
    InvalidManager {
        manager: CharacterId,
        organization: OrganizationId,
    },
    #[error("manager {manager} already has active mandate {mandate}")]
    ExistingMandate {
        manager: CharacterId,
        mandate: MandateId,
    },
    #[error("mandate must contain at least one responsibility scope")]
    NoScopes,
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("neighborhood {0} is not active")]
    InactiveNeighborhood(NeighborhoodId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} is not active")]
    InactiveBusiness(BusinessId),
    #[error("standing order key {expected:?} does not match setting {actual:?}")]
    PolicyKindMismatch {
        expected: PolicyKind,
        actual: PolicyKind,
    },
    #[error("budget limit must not be negative")]
    NegativeBudgetLimit,
    #[error("budget funding account {0} does not exist")]
    MissingBudgetAccount(crate::core::id::FinancialAccountId),
    #[error("budget funding account {0} is not open")]
    BudgetAccountNotOpen(crate::core::id::FinancialAccountId),
    #[error("budget funding account {account} is not owned by organization {organization}")]
    BudgetAccountOwnerMismatch {
        account: crate::core::id::FinancialAccountId,
        organization: OrganizationId,
    },
    #[error("mandate {0} does not exist")]
    MissingMandate(MandateId),
    #[error("mandate {0} is not active")]
    InactiveMandate(MandateId),
    #[error(
        "mandate {mandate} changed after validation; expected version {expected}, found {found}"
    )]
    StaleMandate {
        mandate: MandateId,
        expected: u32,
        found: u32,
    },
    #[error(
        "manager {manager} changed after validation; expected version {expected}, found {found}"
    )]
    StaleManager {
        manager: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("manager {0} is not assigned to an organization")]
    ManagerUnassigned(CharacterId),
    #[error("organization {organization} is missing policy {policy:?}")]
    MissingOrganizationPolicy {
        organization: OrganizationId,
        policy: PolicyKind,
    },
}

#[derive(Debug)]
pub struct ValidatedMandateAssignment {
    draft: MandateDraft,
    expected_manager_version: u32,
}

impl ValidatedMandateAssignment {
    pub fn commit(self, state: &mut AppState) -> Result<MandateId, DelegationError> {
        validate_manager_snapshot(
            state,
            self.draft.manager,
            self.draft.organization,
            self.expected_manager_version,
        )?;
        if let Some(existing) = state.delegation.active_for_manager(self.draft.manager) {
            return Err(DelegationError::ExistingMandate {
                manager: self.draft.manager,
                mandate: existing.id(),
            });
        }
        let id = state.ids.next_mandate();
        state
            .delegation
            .insert(build_mandate_record(id, self.draft));
        Ok(id)
    }
}

pub fn validate_assign_mandate(
    registry: &Registry,
    state: &AppState,
    draft: MandateDraft,
) -> Result<ValidatedMandateAssignment, DelegationError> {
    validate_mandate_content(
        registry,
        state,
        draft.organization,
        &draft.scopes,
        &draft.standing_orders,
        draft.budget,
    )?;
    let manager = validate_manager(state, draft.manager, draft.organization)?;
    if let Some(existing) = state.delegation.active_for_manager(draft.manager) {
        return Err(DelegationError::ExistingMandate {
            manager: draft.manager,
            mandate: existing.id(),
        });
    }
    Ok(ValidatedMandateAssignment {
        draft,
        expected_manager_version: manager.version(),
    })
}

#[derive(Clone, Debug)]
pub struct MandateRevisionDraft {
    pub scopes: BTreeSet<ResponsibilityScope>,
    pub standing_orders: BTreeMap<PolicyKind, PolicySetting>,
    pub budget: Option<BudgetAuthority>,
}

#[derive(Debug)]
pub struct ValidatedMandateRevision {
    mandate: MandateId,
    draft: MandateRevisionDraft,
    expected_mandate_version: u32,
    manager: CharacterId,
    organization: OrganizationId,
    expected_manager_version: u32,
}

impl ValidatedMandateRevision {
    pub fn commit(self, state: &mut AppState) -> Result<(), DelegationError> {
        let record = state
            .delegation
            .get_mandate(self.mandate)
            .ok_or(DelegationError::MissingMandate(self.mandate))?;
        if record.version() != self.expected_mandate_version {
            return Err(DelegationError::StaleMandate {
                mandate: self.mandate,
                expected: self.expected_mandate_version,
                found: record.version(),
            });
        }
        if record.status() != MandateStatus::Active {
            return Err(DelegationError::InactiveMandate(self.mandate));
        }
        validate_manager_snapshot(
            state,
            self.manager,
            self.organization,
            self.expected_manager_version,
        )?;
        let MandateRevisionDraft {
            scopes,
            standing_orders,
            budget,
        } = self.draft;
        state
            .delegation
            .revise(self.mandate, scopes, standing_orders, budget);
        Ok(())
    }
}

pub fn validate_revise_mandate(
    registry: &Registry,
    state: &AppState,
    mandate: MandateId,
    draft: MandateRevisionDraft,
) -> Result<ValidatedMandateRevision, DelegationError> {
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(DelegationError::MissingMandate(mandate))?;
    if record.status() != MandateStatus::Active {
        return Err(DelegationError::InactiveMandate(mandate));
    }
    validate_mandate_content(
        registry,
        state,
        record.organization(),
        &draft.scopes,
        &draft.standing_orders,
        draft.budget,
    )?;
    let manager = validate_manager(state, record.manager(), record.organization())?;
    Ok(ValidatedMandateRevision {
        mandate,
        draft,
        expected_mandate_version: record.version(),
        manager: record.manager(),
        organization: record.organization(),
        expected_manager_version: manager.version(),
    })
}

#[derive(Debug)]
pub struct ValidatedMandateRevocation {
    mandate: MandateId,
    expected_version: u32,
}

impl ValidatedMandateRevocation {
    pub fn commit(self, state: &mut AppState) -> Result<(), DelegationError> {
        let record = state
            .delegation
            .get_mandate(self.mandate)
            .ok_or(DelegationError::MissingMandate(self.mandate))?;
        if record.version() != self.expected_version {
            return Err(DelegationError::StaleMandate {
                mandate: self.mandate,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != MandateStatus::Active {
            return Err(DelegationError::InactiveMandate(self.mandate));
        }
        state.delegation.revoke(self.mandate);
        Ok(())
    }
}

pub fn validate_revoke_mandate(
    state: &AppState,
    mandate: MandateId,
) -> Result<ValidatedMandateRevocation, DelegationError> {
    let record = state
        .delegation
        .get_mandate(mandate)
        .ok_or(DelegationError::MissingMandate(mandate))?;
    if record.status() != MandateStatus::Active {
        return Err(DelegationError::InactiveMandate(mandate));
    }
    Ok(ValidatedMandateRevocation {
        mandate,
        expected_version: record.version(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicySource {
    Organization(OrganizationId),
    Mandate(MandateId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPolicy {
    pub setting: PolicySetting,
    pub source: PolicySource,
}

pub fn resolve_policy_for_manager(
    state: &AppState,
    manager: CharacterId,
    kind: PolicyKind,
) -> Result<ResolvedPolicy, DelegationError> {
    let manager_record = state
        .world
        .get_character(manager)
        .ok_or(DelegationError::MissingManager(manager))?;
    let organization = manager_record
        .organization()
        .ok_or(DelegationError::ManagerUnassigned(manager))?;
    if let Some(mandate) = state.delegation.active_for_manager(manager) {
        if let Some(setting) = mandate.standing_order(kind) {
            return Ok(ResolvedPolicy {
                setting,
                source: PolicySource::Mandate(mandate.id()),
            });
        }
    }
    let organization_record = state
        .world
        .get_organization(organization)
        .ok_or(DelegationError::MissingOrganization(organization))?;
    let setting =
        organization_record
            .policy(kind)
            .ok_or(DelegationError::MissingOrganizationPolicy {
                organization,
                policy: kind,
            })?;
    Ok(ResolvedPolicy {
        setting,
        source: PolicySource::Organization(organization),
    })
}

fn validate_manager(
    state: &AppState,
    manager: CharacterId,
    organization: OrganizationId,
) -> Result<&crate::world::CharacterRecord, DelegationError> {
    let organization_record = state
        .world
        .get_organization(organization)
        .ok_or(DelegationError::MissingOrganization(organization))?;
    if organization_record.lifecycle() != Lifecycle::Active {
        return Err(DelegationError::InactiveOrganization(organization));
    }
    let manager_record = state
        .world
        .get_character(manager)
        .ok_or(DelegationError::MissingManager(manager))?;
    if manager_record.lifecycle() != Lifecycle::Active
        || manager_record.organization() != Some(organization)
    {
        return Err(DelegationError::InvalidManager {
            manager,
            organization,
        });
    }
    Ok(manager_record)
}

fn validate_manager_snapshot(
    state: &AppState,
    manager: CharacterId,
    organization: OrganizationId,
    expected_version: u32,
) -> Result<(), DelegationError> {
    let record = validate_manager(state, manager, organization)?;
    if record.version() != expected_version {
        return Err(DelegationError::StaleManager {
            manager,
            expected: expected_version,
            found: record.version(),
        });
    }
    Ok(())
}

fn validate_mandate_content(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    scopes: &BTreeSet<ResponsibilityScope>,
    standing_orders: &BTreeMap<PolicyKind, PolicySetting>,
    budget: Option<BudgetAuthority>,
) -> Result<(), DelegationError> {
    if scopes.is_empty() {
        return Err(DelegationError::NoScopes);
    }
    for scope in scopes {
        match scope {
            ResponsibilityScope::Neighborhood(id) => {
                let record = state
                    .world
                    .get_neighborhood(*id)
                    .ok_or(DelegationError::MissingNeighborhood(*id))?;
                if record.lifecycle() != Lifecycle::Active {
                    return Err(DelegationError::InactiveNeighborhood(*id));
                }
            }
            ResponsibilityScope::Business(id) => {
                let record = state
                    .world
                    .get_business(*id)
                    .ok_or(DelegationError::MissingBusiness(*id))?;
                if record.lifecycle() != Lifecycle::Active {
                    return Err(DelegationError::InactiveBusiness(*id));
                }
            }
            ResponsibilityScope::Function(_) => {}
        }
    }
    for (kind, setting) in standing_orders {
        registry.get_policy(*kind);
        if setting.kind() != *kind {
            return Err(DelegationError::PolicyKindMismatch {
                expected: *kind,
                actual: setting.kind(),
            });
        }
    }
    if let Some(budget) = budget {
        if budget.limit.cents() < 0 {
            return Err(DelegationError::NegativeBudgetLimit);
        }
        let account = state.finance.get_account(budget.funding_account).ok_or(
            DelegationError::MissingBudgetAccount(budget.funding_account),
        )?;
        if account.lifecycle() != AccountLifecycle::Open {
            return Err(DelegationError::BudgetAccountNotOpen(
                budget.funding_account,
            ));
        }
        if account.owner() != FinancialOwner::Organization(organization) {
            return Err(DelegationError::BudgetAccountOwnerMismatch {
                account: budget.funding_account,
                organization,
            });
        }
    }
    Ok(())
}
