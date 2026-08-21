//! Mandate validation, lifecycle transactions, and policy resolution; sibling delegation state owns synchronized indexes.

use crate::core::id::{
    ArrestId, BusinessId, CharacterId, EnterpriseId, IdExhaustionError, MandateId, NeighborhoodId,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::delegation::{
    build_mandate_record, BudgetAuthority, MandateAuthority, MandateDraft, MandateStatus,
    ResolvedMandateAuthority, ResponsibilityScope,
};
use crate::finance::FinancialOwner;
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
    #[error("manager {manager} is detained under arrest {arrest}")]
    DetainedManager {
        manager: CharacterId,
        arrest: ArrestId,
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
    #[error("budget funding account {account} is not owned by organization {organization}")]
    BudgetAccountOwnerMismatch {
        account: crate::core::id::FinancialAccountId,
        organization: OrganizationId,
    },
    #[error("mandate {0} does not exist")]
    MissingMandate(MandateId),
    #[error("mandate {0} is not active")]
    InactiveMandate(MandateId),
    #[error("mandate {mandate} belongs to manager {expected}, not authority manager {manager}")]
    AuthorityManagerMismatch {
        mandate: MandateId,
        manager: CharacterId,
        expected: CharacterId,
    },
    #[error("scope {scope:?} is outside mandate {mandate}")]
    ScopeOutsideMandate {
        mandate: MandateId,
        scope: ResponsibilityScope,
    },
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
    #[error("active enterprise {enterprise} still depends on mandate {mandate}")]
    ActiveEnterpriseDependency {
        mandate: MandateId,
        enterprise: EnterpriseId,
    },
    #[error(
        "active enterprise {enterprise} still depends on scope {scope:?} in mandate {mandate}"
    )]
    ActiveEnterpriseScopeDependency {
        mandate: MandateId,
        enterprise: EnterpriseId,
        scope: ResponsibilityScope,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
        let id = state.ids.next_mandate()?;
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
        validate_enterprise_scope_dependencies(state, self.mandate, &self.draft.scopes)?;
        // Revalidate budget and scope liveness that could have changed between validation and commit.
        for scope in &self.draft.scopes {
            match scope {
                ResponsibilityScope::Neighborhood(id) => {
                    let rec = state
                        .world
                        .get_neighborhood(*id)
                        .ok_or(DelegationError::MissingNeighborhood(*id))?;
                    if rec.lifecycle() != Lifecycle::Active {
                        return Err(DelegationError::InactiveNeighborhood(*id));
                    }
                }
                ResponsibilityScope::Business(id) => {
                    let rec = state
                        .world
                        .get_business(*id)
                        .ok_or(DelegationError::MissingBusiness(*id))?;
                    if rec.lifecycle() != Lifecycle::Active {
                        return Err(DelegationError::InactiveBusiness(*id));
                    }
                }
                ResponsibilityScope::Function(_) => {}
            }
        }
        for (kind, setting) in &self.draft.standing_orders {
            if setting.kind() != *kind {
                return Err(DelegationError::PolicyKindMismatch {
                    expected: *kind,
                    actual: setting.kind(),
                });
            }
        }
        if let Some(budget) = self.draft.budget {
            if budget.limit.cents() < 0 {
                return Err(DelegationError::NegativeBudgetLimit);
            }
            let account = state.finance.get_account(budget.funding_account).ok_or(
                DelegationError::MissingBudgetAccount(budget.funding_account),
            )?;
            if account.owner() != FinancialOwner::Organization(self.organization) {
                return Err(DelegationError::BudgetAccountOwnerMismatch {
                    account: budget.funding_account,
                    organization: self.organization,
                });
            }
        }
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
    validate_enterprise_scope_dependencies(state, mandate, &draft.scopes)?;
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
        validate_no_active_enterprise_dependencies(state, self.mandate)?;
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
    validate_no_active_enterprise_dependencies(state, mandate)?;
    Ok(ValidatedMandateRevocation {
        mandate,
        expected_version: record.version(),
    })
}

fn validate_no_active_enterprise_dependencies(
    state: &AppState,
    mandate: MandateId,
) -> Result<(), DelegationError> {
    if let Some(enterprise) = state.enterprises.active_for_mandate(mandate).next() {
        return Err(DelegationError::ActiveEnterpriseDependency {
            mandate,
            enterprise: enterprise.id(),
        });
    }
    Ok(())
}

fn validate_enterprise_scope_dependencies(
    state: &AppState,
    mandate: MandateId,
    scopes: &BTreeSet<ResponsibilityScope>,
) -> Result<(), DelegationError> {
    for enterprise in state.enterprises.active_for_mandate(mandate) {
        let scope = enterprise.authority().scope;
        if !scopes.contains(&scope) {
            return Err(DelegationError::ActiveEnterpriseScopeDependency {
                mandate,
                enterprise: enterprise.id(),
                scope,
            });
        }
    }
    Ok(())
}

pub fn resolve_mandate_authority(
    state: &AppState,
    authority: MandateAuthority,
) -> Result<ResolvedMandateAuthority, DelegationError> {
    let record = state
        .delegation
        .get_mandate(authority.mandate)
        .ok_or(DelegationError::MissingMandate(authority.mandate))?;
    if record.status() != MandateStatus::Active {
        return Err(DelegationError::InactiveMandate(authority.mandate));
    }
    if record.manager() != authority.manager {
        return Err(DelegationError::AuthorityManagerMismatch {
            mandate: authority.mandate,
            manager: authority.manager,
            expected: record.manager(),
        });
    }
    if !record.scopes().contains(&authority.scope) {
        return Err(DelegationError::ScopeOutsideMandate {
            mandate: authority.mandate,
            scope: authority.scope,
        });
    }
    let manager = validate_manager(state, authority.manager, record.organization())?;
    Ok(ResolvedMandateAuthority {
        authority,
        organization: record.organization(),
        mandate_version: record.version(),
        manager_version: manager.version(),
    })
}

pub fn validate_mandate_authority_snapshot(
    state: &AppState,
    snapshot: ResolvedMandateAuthority,
) -> Result<(), DelegationError> {
    let authority = snapshot.authority();
    let record = state
        .delegation
        .get_mandate(authority.mandate)
        .ok_or(DelegationError::MissingMandate(authority.mandate))?;
    if record.version() != snapshot.mandate_version() {
        return Err(DelegationError::StaleMandate {
            mandate: authority.mandate,
            expected: snapshot.mandate_version(),
            found: record.version(),
        });
    }
    if record.status() != MandateStatus::Active {
        return Err(DelegationError::InactiveMandate(authority.mandate));
    }
    if record.manager() != authority.manager {
        return Err(DelegationError::AuthorityManagerMismatch {
            mandate: authority.mandate,
            manager: authority.manager,
            expected: record.manager(),
        });
    }
    if !record.scopes().contains(&authority.scope) {
        return Err(DelegationError::ScopeOutsideMandate {
            mandate: authority.mandate,
            scope: authority.scope,
        });
    }
    validate_manager_snapshot(
        state,
        authority.manager,
        snapshot.organization(),
        snapshot.manager_version(),
    )
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
    validate_manager(state, manager, organization)?;
    if let Some(mandate) = state.delegation.active_for_manager(manager) {
        if let Some(setting) = mandate.standing_order(kind) {
            return build_resolved_policy(kind, setting, PolicySource::Mandate(mandate.id()));
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
    build_resolved_policy(kind, setting, PolicySource::Organization(organization))
}

fn build_resolved_policy(
    expected: PolicyKind,
    setting: PolicySetting,
    source: PolicySource,
) -> Result<ResolvedPolicy, DelegationError> {
    let actual = setting.kind();
    if actual != expected {
        return Err(DelegationError::PolicyKindMismatch { expected, actual });
    }
    Ok(ResolvedPolicy { setting, source })
}

impl ResolvedPolicy {
    /// Destructures a policy resolved for [`PolicyKind::IndependentRecruitment`]; the kind
    /// match is already guaranteed by `build_resolved_policy`.
    pub fn independent_recruitment_approval(&self) -> crate::world::ApprovalPolicy {
        match self.setting {
            PolicySetting::IndependentRecruitment(approval) => approval,
            PolicySetting::CollectionForce(_)
            | PolicySetting::PatrolBribery(_)
            | PolicySetting::CasualtyResponse(_)
            | PolicySetting::AssociateLegalSupport(_) => {
                unreachable!("independent-recruitment resolution returned another policy kind")
            }
        }
    }
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
    if let Some(arrest) = state.legal.active_arrest_for_character(manager) {
        return Err(DelegationError::DetainedManager {
            manager,
            arrest: arrest.id(),
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
        if account.owner() != FinancialOwner::Organization(organization) {
            return Err(DelegationError::BudgetAccountOwnerMismatch {
                account: budget.funding_account,
                organization,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::delegation::ResponsibilityFunction;
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

    fn make_authority_fixture() -> (Registry, AppState, MandateAuthority) {
        let registry = build_registry();
        let mut state = AppState::new(67);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Authority Test Organization".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("organization fixture should validate");
        let manager = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Authority Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("manager fixture should validate");
        let mandate = validate_assign_mandate(
            &registry,
            &state,
            MandateDraft {
                organization,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Finance,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("mandate fixture should validate")
        .commit(&mut state)
        .expect("validated mandate should remain current");
        (
            registry,
            state,
            MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Function(ResponsibilityFunction::Finance),
            },
        )
    }

    #[test]
    fn resolves_authority_with_versioned_dependencies() {
        let (_registry, state, authority) = make_authority_fixture();
        let resolved = resolve_mandate_authority(&state, authority)
            .expect("valid mandate authority should resolve");

        assert_eq!(resolved.authority(), authority);
        assert_eq!(
            resolved.organization(),
            state
                .delegation()
                .get_mandate(authority.mandate)
                .expect("mandate should exist")
                .organization()
        );
        assert_eq!(resolved.mandate_version(), 1);
        assert_eq!(resolved.manager_version(), 1);
        validate_invariants(&state);
    }

    #[test]
    fn authority_rejects_wrong_manager_and_scope() {
        let (registry, mut state, authority) = make_authority_fixture();
        let organization = state
            .delegation()
            .get_mandate(authority.mandate)
            .expect("mandate should exist")
            .organization();
        let other_manager = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Other Authority Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("second manager fixture should validate");

        let wrong_manager = MandateAuthority {
            manager: other_manager,
            ..authority
        };
        assert_eq!(
            resolve_mandate_authority(&state, wrong_manager)
                .expect_err("another manager must not exercise the mandate"),
            DelegationError::AuthorityManagerMismatch {
                mandate: authority.mandate,
                manager: other_manager,
                expected: authority.manager,
            }
        );

        let wrong_scope = MandateAuthority {
            scope: ResponsibilityScope::Function(ResponsibilityFunction::Operations),
            ..authority
        };
        assert_eq!(
            resolve_mandate_authority(&state, wrong_scope)
                .expect_err("authority must remain inside the mandate scope"),
            DelegationError::ScopeOutsideMandate {
                mandate: authority.mandate,
                scope: wrong_scope.scope,
            }
        );
        validate_invariants(&state);
    }

    #[test]
    fn authority_snapshot_rejects_later_mandate_revision() {
        let (registry, mut state, authority) = make_authority_fixture();
        let snapshot = resolve_mandate_authority(&state, authority)
            .expect("valid authority should resolve before revision");
        validate_revise_mandate(
            &registry,
            &state,
            authority.mandate,
            MandateRevisionDraft {
                scopes: BTreeSet::from([
                    ResponsibilityScope::Function(ResponsibilityFunction::Finance),
                    ResponsibilityScope::Function(ResponsibilityFunction::Operations),
                ]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("mandate revision should validate")
        .commit(&mut state)
        .expect("mandate revision should commit");

        assert_eq!(
            validate_mandate_authority_snapshot(&state, snapshot)
                .expect_err("authority snapshot must become stale after mandate revision"),
            DelegationError::StaleMandate {
                mandate: authority.mandate,
                expected: 1,
                found: 2,
            }
        );
        validate_invariants(&state);
    }
}
