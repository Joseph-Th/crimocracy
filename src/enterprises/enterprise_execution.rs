//! Enterprise establishment, lifecycle, cycle planning, and atomic settlement for persistent routine activity.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, EnterpriseCycleId, EnterpriseId, FinancialAccountId, IdExhaustionError, IdKind,
    OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::delegation::delegation_system::{
    resolve_mandate_authority, resolve_policy_for_manager, validate_mandate_authority_snapshot,
    DelegationError,
};
use crate::delegation::{
    MandateAuthority, ResolvedMandateAuthority, ResponsibilityFunction, ResponsibilityScope,
};
use crate::enterprises::{
    build_enterprise_record, EnterpriseCycleRecord, EnterpriseDraft, EnterpriseLocation,
    EnterpriseStatus,
};
use crate::finance::finance_system::{
    validate_record_transaction, FinanceError, ValidatedLedgerTransaction,
};
use crate::finance::{
    AccountKind, AccountLifecycle, FinancialOwner, LedgerTransactionDraft, Money,
};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{EnterpriseDefinition, EnterpriseEconomicsDefinition, Registry};
use crate::world::{
    BusinessFunction, BusinessOwner, CapabilityKind, Lifecycle, NeighborhoodProfile, PolicySetting,
    Rating,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EnterpriseError {
    #[error("enterprise {0} does not exist")]
    MissingEnterprise(EnterpriseId),
    #[error("organization {0} does not exist or is inactive")]
    InvalidOrganization(OrganizationId),
    #[error("enterprise authority belongs to organization {authority_organization}, not {enterprise_organization}")]
    AuthorityOrganizationMismatch {
        authority_organization: OrganizationId,
        enterprise_organization: OrganizationId,
    },
    #[error("authority scope {scope:?} does not cover enterprise location {location:?}")]
    AuthorityLocationMismatch {
        scope: ResponsibilityScope,
        location: EnterpriseLocation,
    },
    #[error("enterprise location {0:?} does not exist or is inactive")]
    InvalidLocation(EnterpriseLocation),
    #[error("business {business} lacks required enterprise function {function:?}")]
    MissingBusinessFunction {
        business: BusinessId,
        function: BusinessFunction,
    },
    #[error("supporting business {0} does not exist or is inactive")]
    InvalidSupportingBusiness(BusinessId),
    #[error("supporting business {business} is owned by {owner:?}, not enterprise organization {organization}")]
    SupportingBusinessOwnershipMismatch {
        business: BusinessId,
        owner: BusinessOwner,
        organization: OrganizationId,
    },
    #[error("hosted business {business} is owned by {owner:?}, not enterprise organization {organization}")]
    HostBusinessOwnershipMismatch {
        business: BusinessId,
        owner: BusinessOwner,
        organization: OrganizationId,
    },
    #[error("supporting business {business} duplicates the enterprise's hosted business location")]
    DuplicateSupportingLocation { business: BusinessId },
    #[error("enterprise support network lacks required function {function:?}")]
    MissingNetworkFunction { function: BusinessFunction },
    #[error("supporting business {business} changed after validation; expected version {expected}, found {found}")]
    StaleSupportingBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("financial account {account} is not owned by organization {organization}")]
    AccountOwnerMismatch {
        account: FinancialAccountId,
        organization: OrganizationId,
    },
    #[error("enterprise cash account {0} must be street or concealed cash")]
    InvalidCashAccountKind(FinancialAccountId),
    #[error("enterprise settlement account {0} must be a settlement account")]
    InvalidSettlementAccountKind(FinancialAccountId),
    #[error("enterprise financial account {0} is not open")]
    AccountNotOpen(FinancialAccountId),
    #[error("settlement account {account} is already reserved by enterprise {enterprise}")]
    SettlementAccountInUse {
        account: FinancialAccountId,
        enterprise: EnterpriseId,
    },
    #[error("enterprise {0} is not active")]
    EnterpriseNotActive(EnterpriseId),
    #[error("enterprise {0} is not suspended")]
    EnterpriseNotSuspended(EnterpriseId),
    #[error("enterprise {0} is already closed")]
    EnterpriseClosed(EnterpriseId),
    #[error("enterprise {enterprise} is not due for a cycle until {due_at:?}")]
    CycleNotDue {
        enterprise: EnterpriseId,
        due_at: SimTime,
    },
    #[error(
        "enterprise cycle variance {basis_points} basis points exceeds authored limit {limit}"
    )]
    VarianceOutOfRange { basis_points: i16, limit: u16 },
    #[error("enterprise economics overflowed while resolving cycle {0}")]
    ArithmeticOverflow(EnterpriseId),
    #[error("enterprise {enterprise} changed after validation; expected version {expected}, found {found}")]
    StaleEnterprise {
        enterprise: EnterpriseId,
        expected: u32,
        found: u32,
    },
    #[error(
        "enterprise cycle plan was resolved at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleCycleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedEnterpriseEstablishment {
    draft: EnterpriseDraft,
    authority: ResolvedMandateAuthority,
    cycle_duration: SimDuration,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
}

impl ValidatedEnterpriseEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<EnterpriseId, EnterpriseError> {
        validate_mandate_authority_snapshot(state, self.authority)?;
        validate_enterprise_environment(
            state,
            self.draft.organization,
            self.draft.authority,
            self.draft.location,
        )?;
        validate_supporting_business_versions(state, &self.supporting_business_versions)?;
        validate_supporting_businesses(
            state,
            self.draft.organization,
            self.draft.location,
            &self.draft.supporting_businesses,
        )?;
        validate_enterprise_accounts(
            state,
            self.draft.organization,
            self.draft.cash_account,
            self.draft.settlement_account,
            None,
        )?;
        let id = state.ids.next_enterprise()?;
        let established_at = state.now();
        let next_cycle_at = established_at + self.cycle_duration;
        state.enterprises.insert(build_enterprise_record(
            id,
            self.draft,
            established_at,
            next_cycle_at,
        ));
        Ok(id)
    }
}

pub fn validate_establish_enterprise(
    registry: &Registry,
    state: &AppState,
    draft: EnterpriseDraft,
) -> Result<ValidatedEnterpriseEstablishment, EnterpriseError> {
    let definition = registry.get_enterprise(draft.kind);
    let authority = resolve_mandate_authority(state, draft.authority)?;
    validate_enterprise_environment(state, draft.organization, draft.authority, draft.location)?;
    validate_enterprise_business_dependencies(
        definition,
        state,
        draft.organization,
        draft.location,
        &draft.supporting_businesses,
    )?;
    validate_enterprise_accounts(
        state,
        draft.organization,
        draft.cash_account,
        draft.settlement_account,
        None,
    )?;
    let cycle_duration = definition.economics().cycle();
    let supporting_business_versions =
        snapshot_supporting_business_versions(state, &draft.supporting_businesses)?;
    Ok(ValidatedEnterpriseEstablishment {
        draft,
        authority,
        cycle_duration,
        supporting_business_versions,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleSnapshot {
    enterprise: EnterpriseId,
    expected_enterprise_version: u32,
    authority: ResolvedMandateAuthority,
    occurred_at: SimTime,
    next_cycle_at: SimTime,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
    host_business_version: Option<(BusinessId, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleEconomics {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    attention: AttentionClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleManagement {
    manager_management: Option<Rating>,
    policy_setting: Option<PolicySetting>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleAccounts {
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterpriseCyclePlan {
    snapshot: EnterpriseCycleSnapshot,
    economics: EnterpriseCycleEconomics,
    management: EnterpriseCycleManagement,
    accounts: EnterpriseCycleAccounts,
}

impl EnterpriseCyclePlan {
    pub fn enterprise(&self) -> EnterpriseId {
        self.snapshot.enterprise
    }
    pub fn gross_revenue(&self) -> Money {
        self.economics.gross_revenue
    }
    pub fn operating_cost(&self) -> Money {
        self.economics.operating_cost
    }
    pub fn net_cash(&self) -> Money {
        self.economics.net_cash
    }
    pub fn variance_basis_points(&self) -> i16 {
        self.economics.variance_basis_points
    }
    pub fn attention(&self) -> AttentionClass {
        self.economics.attention
    }
    pub fn policy_setting(&self) -> Option<PolicySetting> {
        self.management.policy_setting
    }
}

pub fn decide_enterprise_cycle(
    registry: &Registry,
    state: &AppState,
    enterprise: EnterpriseId,
    variance_basis_points: i16,
) -> Result<EnterpriseCyclePlan, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() != EnterpriseStatus::Active {
        return Err(EnterpriseError::EnterpriseNotActive(enterprise));
    }
    let due_at = record
        .next_cycle_at()
        .ok_or(EnterpriseError::EnterpriseNotActive(enterprise))?;
    if state.now() < due_at {
        return Err(EnterpriseError::CycleNotDue { enterprise, due_at });
    }
    let definition = registry.get_enterprise(record.kind());
    let variance_limit = definition.economics().gross_variance_basis_points();
    if i32::from(variance_basis_points).unsigned_abs() > u32::from(variance_limit) {
        return Err(EnterpriseError::VarianceOutOfRange {
            basis_points: variance_basis_points,
            limit: variance_limit,
        });
    }
    let authority = resolve_mandate_authority(state, record.authority())?;
    validate_enterprise_environment(
        state,
        record.organization(),
        record.authority(),
        record.location(),
    )?;
    validate_enterprise_business_dependencies(
        definition,
        state,
        record.organization(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_accounts(
        state,
        record.organization(),
        record.cash_account(),
        record.settlement_account(),
        Some(record.id()),
    )?;
    let neighborhood = resolve_location_profile(state, record.location())?;
    let manager = state
        .world
        .get_character(record.manager())
        .expect("resolved enterprise authority manager must exist");
    let manager_management = manager.capability(CapabilityKind::Management);
    let economics = definition.economics();
    let gross_before_variance =
        resolve_gross_before_variance(enterprise, economics, neighborhood, manager_management)?;
    let gross_revenue =
        apply_basis_point_variance(enterprise, gross_before_variance, variance_basis_points)?;
    let operating_cost = resolve_operating_cost(
        state,
        enterprise,
        economics,
        neighborhood,
        record.location(),
    )?;
    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let policy_setting = match definition.policy() {
        Some(kind) => Some(resolve_policy_for_manager(state, record.manager(), kind)?.setting),
        None => None,
    };
    let attention = if i32::from(variance_basis_points).unsigned_abs()
        >= u32::from(economics.notable_variance_basis_points())
    {
        AttentionClass::Notable
    } else {
        AttentionClass::Routine
    };
    let supporting_business_versions =
        snapshot_supporting_business_versions(state, record.supporting_businesses())?;
    let host_business_version = match record.location() {
        EnterpriseLocation::Business(business_id) => {
            let business = state
                .world
                .get_business(business_id)
                .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
            Some((business_id, business.version()))
        }
        EnterpriseLocation::Neighborhood(_) => None,
    };
    Ok(EnterpriseCyclePlan {
        snapshot: EnterpriseCycleSnapshot {
            enterprise,
            expected_enterprise_version: record.version(),
            authority,
            occurred_at: state.now(),
            // A detained manager leaves the enterprise overdue, but missed cycles are not
            // retroactively paid out in a burst after release. Re-anchor the next cycle to the
            // actual settlement instant so routine work resumes at its authored cadence.
            next_cycle_at: state.now() + economics.cycle(),
            supporting_business_versions,
            host_business_version,
        },
        economics: EnterpriseCycleEconomics {
            gross_revenue,
            operating_cost,
            net_cash,
            variance_basis_points,
            attention,
        },
        management: EnterpriseCycleManagement {
            manager_management,
            policy_setting,
        },
        accounts: EnterpriseCycleAccounts {
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
        },
    })
}

pub struct ValidatedEnterpriseCycle {
    plan: EnterpriseCyclePlan,
    ledger: Option<ValidatedLedgerTransaction>,
    information: Option<ValidatedInformation>,
}

impl ValidatedEnterpriseCycle {
    pub fn commit(self, state: &mut AppState) -> Result<EnterpriseCycleId, EnterpriseError> {
        let mut budget = Vec::new();
        if self.ledger.is_some() {
            budget.push((IdKind::LedgerTransaction, 1));
        }
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        budget.push((IdKind::EnterpriseCycle, 1));
        state.ids.reserve_many(&budget)?;
        let record = state
            .enterprises
            .get_enterprise(self.plan.snapshot.enterprise)
            .ok_or(EnterpriseError::MissingEnterprise(
                self.plan.snapshot.enterprise,
            ))?;
        if record.version() != self.plan.snapshot.expected_enterprise_version {
            return Err(EnterpriseError::StaleEnterprise {
                enterprise: self.plan.snapshot.enterprise,
                expected: self.plan.snapshot.expected_enterprise_version,
                found: record.version(),
            });
        }
        if record.status() != EnterpriseStatus::Active {
            return Err(EnterpriseError::EnterpriseNotActive(
                self.plan.snapshot.enterprise,
            ));
        }
        if state.now() != self.plan.snapshot.occurred_at {
            return Err(EnterpriseError::StaleCycleTime {
                expected: self.plan.snapshot.occurred_at,
                found: state.now(),
            });
        }
        validate_mandate_authority_snapshot(state, self.plan.snapshot.authority)?;
        validate_supporting_business_versions(
            state,
            &self.plan.snapshot.supporting_business_versions,
        )?;
        if let Some((business_id, expected)) = self.plan.snapshot.host_business_version {
            let business = state
                .world
                .get_business(business_id)
                .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
            if business.version() != expected {
                return Err(EnterpriseError::StaleSupportingBusiness {
                    business: business_id,
                    expected,
                    found: business.version(),
                });
            }
        }
        validate_supporting_businesses(
            state,
            record.organization(),
            record.location(),
            record.supporting_businesses(),
        )?;
        validate_enterprise_accounts(
            state,
            record.organization(),
            self.plan.accounts.cash_account,
            self.plan.accounts.settlement_account,
            Some(record.id()),
        )?;
        let transaction = match self.ledger {
            Some(ledger) => Some(ledger.commit(state)?),
            None => None,
        };
        let information = match self.information {
            Some(information) => Some(information.commit(state)?),
            None => None,
        };
        let cycle_id = state.ids.next_enterprise_cycle()?;
        state.enterprises.apply_cycle(
            EnterpriseCycleRecord {
                id: cycle_id,
                context: super::EnterpriseCycleContext {
                    enterprise: self.plan.snapshot.enterprise,
                    occurred_at: self.plan.snapshot.occurred_at,
                },
                financials: super::EnterpriseCycleFinancials {
                    gross_revenue: self.plan.economics.gross_revenue,
                    operating_cost: self.plan.economics.operating_cost,
                    net_cash: self.plan.economics.net_cash,
                    variance_basis_points: self.plan.economics.variance_basis_points,
                },
                artifacts: super::EnterpriseCycleArtifacts {
                    attention: self.plan.economics.attention,
                    manager_management: self.plan.management.manager_management,
                    policy_setting: self.plan.management.policy_setting,
                },
                provenance: super::EnterpriseCycleProvenance {
                    transaction,
                    information,
                },
            },
            self.plan.snapshot.next_cycle_at,
        );
        Ok(cycle_id)
    }
}

pub fn validate_enterprise_cycle_plan(
    state: &AppState,
    plan: EnterpriseCyclePlan,
) -> Result<ValidatedEnterpriseCycle, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(plan.snapshot.enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(plan.snapshot.enterprise))?;
    if record.version() != plan.snapshot.expected_enterprise_version {
        return Err(EnterpriseError::StaleEnterprise {
            enterprise: plan.snapshot.enterprise,
            expected: plan.snapshot.expected_enterprise_version,
            found: record.version(),
        });
    }
    if record.status() != EnterpriseStatus::Active {
        return Err(EnterpriseError::EnterpriseNotActive(
            plan.snapshot.enterprise,
        ));
    }
    if state.now() != plan.snapshot.occurred_at {
        return Err(EnterpriseError::StaleCycleTime {
            expected: plan.snapshot.occurred_at,
            found: state.now(),
        });
    }
    validate_mandate_authority_snapshot(state, plan.snapshot.authority)?;
    validate_supporting_business_versions(state, &plan.snapshot.supporting_business_versions)?;
    if let Some((business_id, expected)) = plan.snapshot.host_business_version {
        let business = state
            .world
            .get_business(business_id)
            .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
        if business.version() != expected {
            return Err(EnterpriseError::StaleSupportingBusiness {
                business: business_id,
                expected,
                found: business.version(),
            });
        }
    }
    validate_supporting_businesses(
        state,
        record.organization(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_accounts(
        state,
        record.organization(),
        plan.accounts.cash_account,
        plan.accounts.settlement_account,
        Some(record.id()),
    )?;
    let ledger = if plan.economics.net_cash == Money::ZERO {
        None
    } else {
        let postings = crate::finance::helpers::build_settlement_postings(
            plan.accounts.cash_account,
            plan.accounts.settlement_account,
            plan.economics.net_cash,
        )
        .ok_or(EnterpriseError::ArithmeticOverflow(
            plan.snapshot.enterprise,
        ))?;
        Some(validate_record_transaction(
            state,
            LedgerTransactionDraft {
                occurred_at: plan.snapshot.occurred_at,
                memo: format!(
                    "Routine enterprise settlement for {}",
                    plan.snapshot.enterprise
                ),
                postings: postings.to_vec(),
                authorization: None,
            },
        )?)
    };
    let information = match plan.economics.attention {
        AttentionClass::Notable => Some(validate_record_information(
            state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(record.organization()),
                source_kind: InformationSourceKind::AfterAction,
                topic: crate::intelligence::InformationTopic::FinancialPerformance,
                source_entity: Some(EntityRef::Character(record.manager())),
                subject: EntityRef::Enterprise(record.id()),
                observed_at: plan.snapshot.occurred_at,
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: format!(
                    "Enterprise cycle reported gross {} cents, operating cost {} cents, net cash {} cents, with variance {} basis points.",
                    plan.economics.gross_revenue.cents(),
                    plan.economics.operating_cost.cents(),
                    plan.economics.net_cash.cents(),
                    plan.economics.variance_basis_points,
                ),
            },
        )?),
        AttentionClass::Routine => None,
        AttentionClass::Exception | AttentionClass::Crisis => {
            unreachable!("enterprise cycle plans only produce routine or notable attention")
        }
    };
    Ok(ValidatedEnterpriseCycle {
        plan,
        ledger,
        information,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnterpriseStatusChange {
    Suspend,
    Resume,
    Close,
}

pub struct ValidatedEnterpriseStatusChange {
    enterprise: EnterpriseId,
    expected_version: u32,
    change: EnterpriseStatusChange,
    cycle_duration: Option<SimDuration>,
    authority: Option<ResolvedMandateAuthority>,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
}

impl ValidatedEnterpriseStatusChange {
    pub fn commit(self, state: &mut AppState) -> Result<(), EnterpriseError> {
        let record = state
            .enterprises
            .get_enterprise(self.enterprise)
            .ok_or(EnterpriseError::MissingEnterprise(self.enterprise))?;
        if record.version() != self.expected_version {
            return Err(EnterpriseError::StaleEnterprise {
                enterprise: self.enterprise,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if let Some(authority) = self.authority {
            validate_mandate_authority_snapshot(state, authority)?;
            validate_enterprise_environment(
                state,
                record.organization(),
                record.authority(),
                record.location(),
            )?;
            validate_supporting_business_versions(state, &self.supporting_business_versions)?;
            validate_supporting_businesses(
                state,
                record.organization(),
                record.location(),
                record.supporting_businesses(),
            )?;
            validate_enterprise_accounts(
                state,
                record.organization(),
                record.cash_account(),
                record.settlement_account(),
                Some(record.id()),
            )?;
        }
        let next_status = match self.change {
            EnterpriseStatusChange::Suspend => EnterpriseStatus::Suspended,
            EnterpriseStatusChange::Resume => EnterpriseStatus::Active,
            EnterpriseStatusChange::Close => EnterpriseStatus::Closed,
        };
        let next_cycle_at = self.cycle_duration.map(|duration| state.now() + duration);
        state
            .enterprises
            .set_status(self.enterprise, next_status, next_cycle_at);
        Ok(())
    }
}

pub fn validate_suspend_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() != EnterpriseStatus::Active {
        return Err(match record.status() {
            EnterpriseStatus::Active => unreachable!(),
            EnterpriseStatus::Suspended => EnterpriseError::EnterpriseNotActive(enterprise),
            EnterpriseStatus::Closed => EnterpriseError::EnterpriseClosed(enterprise),
        });
    }
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Suspend,
        cycle_duration: None,
        authority: None,
        supporting_business_versions: BTreeMap::new(),
    })
}

pub fn validate_resume_enterprise(
    registry: &Registry,
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    match record.status() {
        EnterpriseStatus::Active => {
            return Err(EnterpriseError::EnterpriseNotSuspended(enterprise))
        }
        EnterpriseStatus::Suspended => {}
        EnterpriseStatus::Closed => return Err(EnterpriseError::EnterpriseClosed(enterprise)),
    }
    let authority = resolve_mandate_authority(state, record.authority())?;
    validate_enterprise_environment(
        state,
        record.organization(),
        record.authority(),
        record.location(),
    )?;
    let definition = registry.get_enterprise(record.kind());
    validate_enterprise_business_dependencies(
        definition,
        state,
        record.organization(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_accounts(
        state,
        record.organization(),
        record.cash_account(),
        record.settlement_account(),
        Some(record.id()),
    )?;
    let cycle_duration = definition.economics().cycle();
    let supporting_business_versions =
        snapshot_supporting_business_versions(state, record.supporting_businesses())?;
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Resume,
        cycle_duration: Some(cycle_duration),
        authority: Some(authority),
        supporting_business_versions,
    })
}

pub fn validate_close_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() == EnterpriseStatus::Closed {
        return Err(EnterpriseError::EnterpriseClosed(enterprise));
    }
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Close,
        cycle_duration: None,
        authority: None,
        supporting_business_versions: BTreeMap::new(),
    })
}

pub(crate) fn due_active_enterprises(state: &AppState) -> Vec<EnterpriseId> {
    state
        .enterprises
        .due_at_or_before(state.now())
        .into_iter()
        .filter(|enterprise| {
            state
                .enterprises
                .get_enterprise(*enterprise)
                .is_some_and(|record| {
                    state
                        .legal
                        .active_arrest_for_character(record.manager())
                        .is_none()
                })
        })
        .collect()
}

fn validate_enterprise_environment(
    state: &AppState,
    organization: OrganizationId,
    authority: MandateAuthority,
    location: EnterpriseLocation,
) -> Result<(), EnterpriseError> {
    let organization_record = state
        .world
        .get_organization(organization)
        .ok_or(EnterpriseError::InvalidOrganization(organization))?;
    if organization_record.lifecycle() != Lifecycle::Active {
        return Err(EnterpriseError::InvalidOrganization(organization));
    }
    let resolved = resolve_mandate_authority(state, authority)?;
    if resolved.organization() != organization {
        return Err(EnterpriseError::AuthorityOrganizationMismatch {
            authority_organization: resolved.organization(),
            enterprise_organization: organization,
        });
    }
    let neighborhood = resolve_location_neighborhood(state, location)?;
    if !can_authority_cover_location(authority.scope, location, neighborhood) {
        return Err(EnterpriseError::AuthorityLocationMismatch {
            scope: authority.scope,
            location,
        });
    }
    Ok(())
}

fn can_authority_cover_location(
    scope: ResponsibilityScope,
    location: EnterpriseLocation,
    neighborhood: crate::core::id::NeighborhoodId,
) -> bool {
    match scope {
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
        ResponsibilityScope::Neighborhood(id) => id == neighborhood,
        ResponsibilityScope::Business(id) => {
            matches!(location, EnterpriseLocation::Business(location_id) if location_id == id)
        }
    }
}

fn resolve_location_neighborhood(
    state: &AppState,
    location: EnterpriseLocation,
) -> Result<crate::core::id::NeighborhoodId, EnterpriseError> {
    match location {
        EnterpriseLocation::Neighborhood(id) => {
            let neighborhood = state
                .world
                .get_neighborhood(id)
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            if neighborhood.lifecycle() != Lifecycle::Active {
                return Err(EnterpriseError::InvalidLocation(location));
            }
            Ok(id)
        }
        EnterpriseLocation::Business(id) => {
            let business = state
                .world
                .get_business(id)
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            if business.lifecycle() != Lifecycle::Active {
                return Err(EnterpriseError::InvalidLocation(location));
            }
            let neighborhood = state
                .world
                .get_neighborhood(business.neighborhood())
                .ok_or(EnterpriseError::InvalidLocation(location))?;
            if neighborhood.lifecycle() != Lifecycle::Active {
                return Err(EnterpriseError::InvalidLocation(location));
            }
            Ok(business.neighborhood())
        }
    }
}

fn resolve_location_profile(
    state: &AppState,
    location: EnterpriseLocation,
) -> Result<NeighborhoodProfile, EnterpriseError> {
    let neighborhood = resolve_location_neighborhood(state, location)?;
    Ok(state
        .world
        .get_neighborhood(neighborhood)
        .expect("validated enterprise neighborhood must exist")
        .profile())
}

fn validate_business_location_requirements(
    definition: &EnterpriseDefinition,
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
) -> Result<(), EnterpriseError> {
    let EnterpriseLocation::Business(business_id) = location else {
        return Ok(());
    };
    let business = state
        .world
        .get_business(business_id)
        .ok_or(EnterpriseError::InvalidLocation(location))?;
    for function in definition.required_business_functions() {
        if !business.has_function(*function) {
            return Err(EnterpriseError::MissingBusinessFunction {
                business: business_id,
                function: *function,
            });
        }
    }
    // The hosting venue must remain organization-owned while the racket runs, just like the
    // support network: a racket cannot keep settling at a business the organization no longer owns.
    if business.owner() != BusinessOwner::Organization(organization) {
        return Err(EnterpriseError::HostBusinessOwnershipMismatch {
            business: business_id,
            owner: business.owner(),
            organization,
        });
    }
    Ok(())
}

fn validate_enterprise_business_dependencies(
    definition: &EnterpriseDefinition,
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<(), EnterpriseError> {
    validate_business_location_requirements(definition, state, organization, location)?;
    validate_supporting_businesses(state, organization, location, supporting_businesses)?;

    if definition.required_network_functions().is_empty() {
        return Ok(());
    }
    let mut available = BTreeSet::new();
    if let EnterpriseLocation::Business(business_id) = location {
        let business = state
            .world
            .get_business(business_id)
            .ok_or(EnterpriseError::InvalidLocation(location))?;
        available.extend(business.functions().iter().copied());
    }
    for business_id in supporting_businesses {
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        available.extend(business.functions().iter().copied());
    }
    for function in definition.required_network_functions() {
        if !available.contains(function) {
            return Err(EnterpriseError::MissingNetworkFunction {
                function: *function,
            });
        }
    }
    Ok(())
}

fn validate_supporting_businesses(
    state: &AppState,
    organization: OrganizationId,
    location: EnterpriseLocation,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<(), EnterpriseError> {
    for business_id in supporting_businesses {
        if matches!(location, EnterpriseLocation::Business(location_id) if location_id == *business_id)
        {
            return Err(EnterpriseError::DuplicateSupportingLocation {
                business: *business_id,
            });
        }
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        if business.lifecycle() != Lifecycle::Active {
            return Err(EnterpriseError::InvalidSupportingBusiness(*business_id));
        }
        if business.owner() != BusinessOwner::Organization(organization) {
            return Err(EnterpriseError::SupportingBusinessOwnershipMismatch {
                business: *business_id,
                owner: business.owner(),
                organization,
            });
        }
    }
    Ok(())
}

fn snapshot_supporting_business_versions(
    state: &AppState,
    supporting_businesses: &BTreeSet<BusinessId>,
) -> Result<BTreeMap<BusinessId, u32>, EnterpriseError> {
    supporting_businesses
        .iter()
        .map(|business_id| {
            let business = state
                .world
                .get_business(*business_id)
                .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
            Ok((*business_id, business.version()))
        })
        .collect()
}

fn validate_supporting_business_versions(
    state: &AppState,
    versions: &BTreeMap<BusinessId, u32>,
) -> Result<(), EnterpriseError> {
    for (business_id, expected) in versions {
        let business = state
            .world
            .get_business(*business_id)
            .ok_or(EnterpriseError::InvalidSupportingBusiness(*business_id))?;
        if business.version() != *expected {
            return Err(EnterpriseError::StaleSupportingBusiness {
                business: *business_id,
                expected: *expected,
                found: business.version(),
            });
        }
    }
    Ok(())
}

fn validate_enterprise_accounts(
    state: &AppState,
    organization: OrganizationId,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    current_enterprise: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    let cash = state
        .finance
        .get_account(cash_account)
        .ok_or(EnterpriseError::MissingAccount(cash_account))?;
    let settlement = state
        .finance
        .get_account(settlement_account)
        .ok_or(EnterpriseError::MissingAccount(settlement_account))?;
    for account in [cash, settlement] {
        if account.owner() != FinancialOwner::Organization(organization) {
            return Err(EnterpriseError::AccountOwnerMismatch {
                account: account.id(),
                organization,
            });
        }
        if account.lifecycle() != AccountLifecycle::Open {
            return Err(EnterpriseError::AccountNotOpen(account.id()));
        }
    }
    match cash.kind() {
        AccountKind::StreetCash | AccountKind::ConcealedCash => {}
        AccountKind::AccountedFunds
        | AccountKind::LegitimateOperating
        | AccountKind::Receivable
        | AccountKind::Payable
        | AccountKind::Settlement => {
            return Err(EnterpriseError::InvalidCashAccountKind(cash_account))
        }
    }
    match settlement.kind() {
        AccountKind::Settlement => {}
        AccountKind::StreetCash
        | AccountKind::ConcealedCash
        | AccountKind::AccountedFunds
        | AccountKind::LegitimateOperating
        | AccountKind::Receivable
        | AccountKind::Payable => {
            return Err(EnterpriseError::InvalidSettlementAccountKind(
                settlement_account,
            ))
        }
    }
    if let Some(existing) = state
        .enterprises
        .get_by_settlement_account(settlement_account)
    {
        if Some(existing.id()) != current_enterprise {
            return Err(EnterpriseError::SettlementAccountInUse {
                account: settlement_account,
                enterprise: existing.id(),
            });
        }
    }
    Ok(())
}

fn resolve_gross_before_variance(
    enterprise: EnterpriseId,
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    management: Option<Rating>,
) -> Result<Money, EnterpriseError> {
    let components = [
        weighted_rating(
            enterprise,
            economics.demand_revenue_per_point(),
            profile.economy.illicit_demand,
        )?,
        weighted_rating(
            enterprise,
            economics.commerce_revenue_per_point(),
            profile.economy.commercial_activity,
        )?,
        weighted_rating(
            enterprise,
            economics.wealth_revenue_per_point(),
            profile.economy.wealth,
        )?,
        weighted_optional_rating(
            enterprise,
            economics.management_revenue_per_point(),
            management,
        )?,
    ];
    let mut gross = economics.base_gross();
    for component in components {
        gross = gross
            .checked_add(component)
            .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    }
    Ok(gross)
}

fn resolve_operating_cost(
    state: &crate::core::state::AppState,
    enterprise: EnterpriseId,
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    location: EnterpriseLocation,
) -> Result<Money, EnterpriseError> {
    let base = economics.base_operating_cost().checked_add(weighted_rating(
        enterprise,
        economics.police_cost_per_point(),
        profile.institutions.police_presence,
    )?);
    let base = base.ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let heat = resolve_investigation_heat_surcharge(state, location);
    let support_cost = state
        .enterprises
        .get_enterprise(enterprise)
        .map(|record| record.supporting_businesses().len() as i64 * 5_000)
        .unwrap_or(0);
    let support_surcharge = Money::from_cents(support_cost);
    base.checked_add(heat)
        .and_then(|total| total.checked_add(support_surcharge))
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}

fn resolve_investigation_heat_surcharge(
    state: &crate::core::state::AppState,
    location: EnterpriseLocation,
) -> Money {
    let neighborhood = match location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(business_id) => {
            let Some(business) = state.world.get_business(business_id) else {
                return Money::ZERO;
            };
            business.neighborhood()
        }
    };
    let Some(authority) =
        crate::legal::jurisdiction_system::resolve_case_intake_authority(state, neighborhood)
    else {
        return Money::ZERO;
    };
    let active = state
        .legal
        .investigations_for_owner(authority)
        .filter(|investigation| {
            investigation.status() == crate::legal::InvestigationStatus::Active
                && investigation.origin_operation().is_some()
        })
        .count();
    if active == 0 {
        Money::ZERO
    } else {
        // $25 per active case in the district: street heat makes the racket more expensive
        // to run (bribes, lookouts, missed nights). The surcharge is deliberately linear
        // so one hot investigation hurts but does not instantly bankrupt a gambling enterprise.
        Money::from_cents((active as i64) * 2_500)
    }
}

fn weighted_optional_rating(
    enterprise: EnterpriseId,
    per_point: Money,
    rating: Option<Rating>,
) -> Result<Money, EnterpriseError> {
    match rating {
        Some(value) => weighted_rating(enterprise, per_point, value),
        None => Ok(Money::ZERO),
    }
}

fn weighted_rating(
    enterprise: EnterpriseId,
    per_point: Money,
    rating: Rating,
) -> Result<Money, EnterpriseError> {
    crate::finance::helpers::weighted_rating(per_point, rating.value())
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}

fn apply_basis_point_variance(
    enterprise: EnterpriseId,
    amount: Money,
    basis_points: i16,
) -> Result<Money, EnterpriseError> {
    crate::finance::helpers::apply_basis_point_variance(amount, basis_points)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{
        validate_invariants, validate_state, validate_state_against_registry,
    };
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::delegation::delegation_system::{
        validate_assign_mandate, validate_revise_mandate, validate_revoke_mandate, DelegationError,
        MandateRevisionDraft,
    };
    use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
    use crate::enterprises::enterprise_reporting::{
        resolve_enterprise_financial_summary, resolve_manager_enterprise_financial_summary,
        resolve_neighborhood_enterprise_financial_summary,
        resolve_organization_enterprise_financial_summary,
    };
    use crate::enterprises::EnterpriseKind;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{FinancialAccountDraft, FinancialOwner};
    use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft,
    };
    use crate::reports::enterprise_financial_report::validate_enterprise_financial_report;
    use crate::reports::ReportKind;
    use crate::world::world_system::{
        insert_business, insert_character, insert_neighborhood, insert_organization,
        validate_transfer_business_ownership, WorldError,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, OrganizationDraft, OrganizationKind,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct EnterpriseFixture {
        state: AppState,
        authority: MandateAuthority,
        organization: OrganizationId,
        location: EnterpriseLocation,
        cash: FinancialAccountId,
        settlement: FinancialAccountId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn make_test_enterprise_fixture() -> EnterpriseFixture {
        let registry = build_registry();
        let mut state = AppState::new(0xE17E_1931);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Enterprise Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Market Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(60),
                        commercial_activity: rating(70),
                        illicit_demand: rating(50),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(40),
                        political_influence: rating(55),
                        social_cohesion: rating(65),
                        visible_violence_tolerance: rating(25),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let manager = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Enterprise Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
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
                scopes: BTreeSet::from([ResponsibilityScope::Neighborhood(neighborhood)]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("mandate fixture should validate")
        .commit(&mut state)
        .expect("mandate fixture should commit");
        let cash = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::StreetCash,
                label: "Enterprise cash".to_owned(),
            },
        )
        .expect("cash account fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
                label: "Enterprise external settlement".to_owned(),
            },
        )
        .expect("settlement account fixture should validate");
        EnterpriseFixture {
            state,
            authority: MandateAuthority {
                mandate,
                manager,
                scope: ResponsibilityScope::Neighborhood(neighborhood),
            },
            organization,
            location: EnterpriseLocation::Neighborhood(neighborhood),
            cash,
            settlement,
        }
    }

    fn establish_protection(registry: &Registry, fixture: &mut EnterpriseFixture) -> EnterpriseId {
        validate_establish_enterprise(
            registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization: fixture.organization,
                authority: fixture.authority,
                location: fixture.location,
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        )
        .expect("enterprise fixture should validate")
        .commit(&mut fixture.state)
        .expect("enterprise fixture should commit")
    }

    fn insert_support_business(
        registry: &Registry,
        fixture: &mut EnterpriseFixture,
        name: &str,
        kind: BusinessKind,
        functions: BTreeSet<BusinessFunction>,
        owner: BusinessOwner,
    ) -> BusinessId {
        let neighborhood = match fixture.location {
            EnterpriseLocation::Neighborhood(id) => id,
            EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
        };
        insert_business(
            registry,
            &mut fixture.state,
            BusinessDraft {
                name: name.to_owned(),
                kind,
                functions,
                neighborhood,
                owner,
            },
        )
        .expect("support business fixture should validate")
    }

    fn alcohol_support_network(
        registry: &Registry,
        fixture: &mut EnterpriseFixture,
    ) -> (BusinessId, BusinessId) {
        let transport = insert_support_business(
            registry,
            fixture,
            "Harbor Freight & Storage",
            BusinessKind::Transportation,
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
            ]),
            BusinessOwner::Organization(fixture.organization),
        );
        let retail = insert_support_business(
            registry,
            fixture,
            "Neighborhood Bottle Shop",
            BusinessKind::Retail,
            BTreeSet::from([BusinessFunction::CustomerAccess]),
            BusinessOwner::Organization(fixture.organization),
        );
        (transport, retail)
    }

    fn establish_alcohol_distribution(
        registry: &Registry,
        fixture: &mut EnterpriseFixture,
        supporting_businesses: BTreeSet<BusinessId>,
    ) -> Result<EnterpriseId, EnterpriseError> {
        validate_establish_enterprise(
            registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::AlcoholDistribution,
                organization: fixture.organization,
                authority: fixture.authority,
                location: fixture.location,
                supporting_businesses,
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        )?
        .commit(&mut fixture.state)
    }

    #[test]
    fn routine_cycle_records_causal_economics_and_balanced_cash_settlement() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, 0)
            .expect("due enterprise cycle should resolve");
        assert_eq!(plan.gross_revenue(), Money::from_cents(22_000));
        assert_eq!(plan.operating_cost(), Money::from_cents(3_900));
        assert_eq!(plan.net_cash(), Money::from_cents(18_100));
        assert_eq!(
            plan.policy_setting(),
            Some(PolicySetting::CollectionForce(
                crate::world::ForcePolicy::ThreatsOnly
            ))
        );

        let cycle = validate_enterprise_cycle_plan(&fixture.state, plan)
            .expect("cycle plan should validate")
            .commit(&mut fixture.state)
            .expect("cycle settlement should commit");
        let cycle_record = fixture
            .state
            .enterprises()
            .get_cycle(cycle)
            .expect("cycle should exist");
        assert!(cycle_record.transaction().is_some());
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.cash)
                .expect("cash account should exist")
                .balance(),
            Money::from_cents(18_100)
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.settlement)
                .expect("settlement account should exist")
                .balance(),
            Money::from_cents(-18_100)
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn detained_enterprise_manager_pauses_due_cycles_until_release() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        let manager = fixture.authority.manager;
        let police = insert_organization(
            &registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Enterprise Custody Police".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let investigation = validate_open_investigation(
            &fixture.state,
            InvestigationDraft {
                owner: police,
                title: "Enterprise manager custody inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(manager)]),
            },
        )
        .expect("custody investigation should validate")
        .commit(&mut fixture.state)
        .expect("custody investigation should commit");
        let evidence = validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(manager),
                origin: None,
                kind: EvidenceKind::FinancialRecord,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("custody evidence should validate")
        .commit(&mut fixture.state)
        .expect("custody evidence should commit");

        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let arrest = validate_arrest(
            &fixture.state,
            ArrestDraft {
                character: manager,
                investigation,
                evidence: BTreeSet::from([evidence]),
            },
        )
        .expect("manager arrest should not require revoking formal enterprise authority")
        .commit(&mut fixture.state)
        .expect("manager arrest should commit");

        assert!(due_active_enterprises(&fixture.state).is_empty());
        let detained_tick = run_tick(&registry, &mut fixture.state);
        assert!(detained_tick.enterprise_cycles.is_empty());
        assert_eq!(
            fixture
                .state
                .enterprises()
                .get_enterprise(enterprise)
                .expect("enterprise should persist")
                .next_cycle_at(),
            Some(SimTime::from_minutes(1_440))
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(2_880));
        let still_detained_tick = run_tick(&registry, &mut fixture.state);
        assert!(still_detained_tick.enterprise_cycles.is_empty());
        validate_state(&fixture.state).expect("paused enterprise detention state should validate");
        validate_invariants(&fixture.state);

        validate_release_arrest(&fixture.state, arrest)
            .expect("manager detention should release")
            .commit(&mut fixture.state)
            .expect("manager release should commit");
        let released_tick = run_tick(&registry, &mut fixture.state);
        assert_eq!(released_tick.enterprise_cycles.len(), 1);
        assert_eq!(
            fixture
                .state
                .enterprises()
                .get_cycle(released_tick.enterprise_cycles[0])
                .expect("released manager should produce the overdue enterprise cycle")
                .enterprise(),
            enterprise
        );
        let next_cycle_at = fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should persist after release")
            .next_cycle_at();
        assert_eq!(
            next_cycle_at,
            Some(fixture.state.now() + SimDuration::from_minutes(1_440))
        );
        let no_burst_tick = run_tick(&registry, &mut fixture.state);
        assert!(no_burst_tick.enterprise_cycles.is_empty());
        validate_state(&fixture.state).expect("resumed enterprise state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn settlement_account_is_exclusive_to_one_enterprise_history() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let first = establish_protection(&registry, &mut fixture);

        let error = match validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization: fixture.organization,
                authority: fixture.authority,
                location: fixture.location,
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        ) {
            Ok(_) => panic!("settlement account reuse must fail before mutation"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            EnterpriseError::SettlementAccountInUse {
                account: fixture.settlement,
                enterprise: first,
            }
        );
        assert_eq!(
            fixture
                .state
                .enterprises()
                .enterprises_at(fixture.location)
                .count(),
            1
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn business_hosted_gambling_requires_concrete_venue_functions() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let neighborhood = match fixture.location {
            EnterpriseLocation::Neighborhood(id) => id,
            EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
        };
        let incomplete_venue = insert_business(
            &registry,
            &mut fixture.state,
            BusinessDraft {
                name: "Sparse Storefront".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("incomplete venue should still be a valid business");

        let error = match validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization: fixture.organization,
                authority: fixture.authority,
                location: EnterpriseLocation::Business(incomplete_venue),
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        ) {
            Ok(_) => panic!("gambling must reject a venue without its required functions"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            EnterpriseError::MissingBusinessFunction {
                business: incomplete_venue,
                function: BusinessFunction::CashIntensive,
            }
        );

        let valid_venue = insert_business(
            &registry,
            &mut fixture.state,
            BusinessDraft {
                name: "Market Social Club".to_owned(),
                kind: BusinessKind::Hospitality,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::MeetingSpace,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(fixture.organization),
            },
        )
        .expect("complete venue should validate");
        let enterprise = validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Gambling,
                organization: fixture.organization,
                authority: fixture.authority,
                location: EnterpriseLocation::Business(valid_venue),
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        )
        .expect("gambling should accept a venue with all required functions")
        .commit(&mut fixture.state)
        .expect("business-hosted enterprise should commit");
        assert_eq!(
            fixture
                .state
                .enterprises()
                .get_enterprise(enterprise)
                .expect("enterprise should exist")
                .location(),
            EnterpriseLocation::Business(valid_venue)
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn alcohol_distribution_uses_owned_business_network_and_survives_save_before_cycle() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
        let enterprise = establish_alcohol_distribution(
            &registry,
            &mut fixture,
            BTreeSet::from([transport, retail]),
        )
        .expect("complete owned distribution network should establish");
        assert_eq!(
            fixture
                .state
                .enterprises()
                .enterprises_supported_by_business(transport)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![enterprise]
        );
        validate_state(&fixture.state).expect("alcohol distribution state should validate");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("alcohol distribution network should satisfy authored content");
        validate_invariants(&fixture.state);

        let save = build_save(&registry, &fixture.state)
            .expect("alcohol distribution state should build a save");
        let bytes = bincode::serialize(&save).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let mut restored = restore_save(&registry, decoded)
            .expect("alcohol distribution support indexes should restore");
        assert_eq!(
            restored
                .enterprises()
                .enterprises_supported_by_business(retail)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![enterprise]
        );

        restored.advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(&registry, &restored, enterprise, 0)
            .expect("valid alcohol distribution network should resolve a due cycle");
        assert_eq!(plan.gross_revenue(), Money::from_cents(31_100));
        assert_eq!(plan.operating_cost(), Money::from_cents(21_600));
        assert_eq!(plan.net_cash(), Money::from_cents(9_500));
        validate_enterprise_cycle_plan(&restored, plan)
            .expect("fresh alcohol distribution cycle should validate")
            .commit(&mut restored)
            .expect("alcohol distribution cycle should commit");
        validate_state(&restored).expect("resolved alcohol distribution state should validate");
        validate_state_against_registry(&registry, &restored)
            .expect("resolved alcohol distribution state should remain authored-valid");
        validate_invariants(&restored);
    }

    #[test]
    fn alcohol_distribution_rejects_incomplete_or_foreign_support_networks() {
        let registry = build_registry();
        let mut incomplete = make_test_enterprise_fixture();
        let incomplete_organization = incomplete.organization;
        let transport = insert_support_business(
            &registry,
            &mut incomplete,
            "Incomplete Freight Network",
            BusinessKind::Transportation,
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
            ]),
            BusinessOwner::Organization(incomplete_organization),
        );
        let error =
            establish_alcohol_distribution(&registry, &mut incomplete, BTreeSet::from([transport]))
                .expect_err("distribution network without retail access must be rejected");
        assert_eq!(
            error,
            EnterpriseError::MissingNetworkFunction {
                function: BusinessFunction::CustomerAccess,
            }
        );

        let mut foreign = make_test_enterprise_fixture();
        let network = insert_support_business(
            &registry,
            &mut foreign,
            "Independent Distribution Combine",
            BusinessKind::Transportation,
            BTreeSet::from([
                BusinessFunction::VehicleFleet,
                BusinessFunction::Warehousing,
                BusinessFunction::DistributionInfrastructure,
                BusinessFunction::CustomerAccess,
            ]),
            BusinessOwner::Independent,
        );
        let error =
            establish_alcohol_distribution(&registry, &mut foreign, BTreeSet::from([network]))
                .expect_err("foreign business capacity must not be consumed implicitly");
        assert_eq!(
            error,
            EnterpriseError::SupportingBusinessOwnershipMismatch {
                business: network,
                owner: BusinessOwner::Independent,
                organization: foreign.organization,
            }
        );
        validate_state(&incomplete.state).expect("rejected incomplete network should not mutate");
        validate_state(&foreign.state).expect("rejected foreign network should not mutate");
        validate_invariants(&incomplete.state);
        validate_invariants(&foreign.state);
    }

    #[test]
    fn distribution_establishment_token_stales_when_support_ownership_changes() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
        let expected_version = fixture
            .state
            .world()
            .get_business(retail)
            .expect("support business should exist")
            .version();
        let establishment = validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::AlcoholDistribution,
                organization: fixture.organization,
                authority: fixture.authority,
                location: fixture.location,
                supporting_businesses: BTreeSet::from([transport, retail]),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        )
        .expect("complete distribution network should initially validate");

        validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
            .expect("no committed enterprise should lock support ownership yet")
            .commit(&mut fixture.state)
            .expect("support ownership transfer should commit before enterprise establishment");
        let found_version = fixture
            .state
            .world()
            .get_business(retail)
            .expect("support business should remain")
            .version();
        assert_eq!(
            establishment
                .commit(&mut fixture.state)
                .expect_err("support mutation must stale validated establishment"),
            EnterpriseError::StaleSupportingBusiness {
                business: retail,
                expected: expected_version,
                found: found_version,
            }
        );
        assert_eq!(
            fixture
                .state
                .enterprises()
                .enterprises_supported_by_business(transport)
                .count(),
            0
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.cash)
                .expect("cash account should persist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.settlement)
                .expect("settlement account should persist")
                .balance(),
            Money::ZERO
        );
        validate_state(&fixture.state)
            .expect("stale establishment rejection should preserve valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn active_distribution_network_locks_business_ownership_and_resume_revalidates_versions() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let (transport, retail) = alcohol_support_network(&registry, &mut fixture);
        let enterprise = establish_alcohol_distribution(
            &registry,
            &mut fixture,
            BTreeSet::from([transport, retail]),
        )
        .expect("complete network should establish");

        let error = validate_transfer_business_ownership(
            &fixture.state,
            retail,
            BusinessOwner::Independent,
        )
        .expect_err("active enterprise must lock supporting business ownership");
        assert_eq!(
            error,
            WorldError::ActiveEnterpriseSupport {
                business: retail,
                enterprise,
                organization: fixture.organization,
            }
        );

        validate_suspend_enterprise(&fixture.state, enterprise)
            .expect("active distribution enterprise should suspend")
            .commit(&mut fixture.state)
            .expect("distribution suspension should commit");
        let stale_resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
            .expect("owned support network should initially validate for resume");
        validate_transfer_business_ownership(&fixture.state, retail, BusinessOwner::Independent)
            .expect("suspended enterprise should release support ownership lock")
            .commit(&mut fixture.state)
            .expect("support ownership transfer should commit while suspended");
        assert_eq!(
            stale_resume
                .commit(&mut fixture.state)
                .expect_err("support ownership mutation must stale prior resume token"),
            EnterpriseError::StaleSupportingBusiness {
                business: retail,
                expected: 1,
                found: 2,
            }
        );
        let fresh_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
            Ok(_) => panic!("foreign-owned support network must not resume"),
            Err(error) => error,
        };
        assert_eq!(
            fresh_error,
            EnterpriseError::SupportingBusinessOwnershipMismatch {
                business: retail,
                owner: BusinessOwner::Independent,
                organization: fixture.organization,
            }
        );

        validate_transfer_business_ownership(
            &fixture.state,
            retail,
            BusinessOwner::Organization(fixture.organization),
        )
        .expect("suspended support business should be transferable back")
        .commit(&mut fixture.state)
        .expect("support ownership restoration should commit");
        validate_resume_enterprise(&registry, &fixture.state, enterprise)
            .expect("restored network should resume")
            .commit(&mut fixture.state)
            .expect("restored distribution enterprise resume should commit");
        assert_eq!(
            fixture
                .state
                .enterprises()
                .get_enterprise(enterprise)
                .expect("distribution enterprise should persist")
                .status(),
            EnterpriseStatus::Active
        );
        validate_state(&fixture.state).expect("restored distribution network should validate");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("restored distribution network should satisfy authored content");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn suspension_removes_enterprise_from_due_work_and_resume_reschedules_it() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        validate_suspend_enterprise(&fixture.state, enterprise)
            .expect("active enterprise should suspend")
            .commit(&mut fixture.state)
            .expect("suspension should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        assert!(due_active_enterprises(&fixture.state).is_empty());

        let resume = validate_resume_enterprise(&registry, &fixture.state, enterprise)
            .expect("suspended enterprise with valid authority should resume");
        fixture.state.advance_clock(SimDuration::from_minutes(30));
        resume
            .commit(&mut fixture.state)
            .expect("resume should commit");
        let record = fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should exist");
        assert_eq!(record.status(), EnterpriseStatus::Active);
        assert_eq!(
            record.next_cycle_at(),
            Some(fixture.state.now() + SimDuration::from_minutes(1_440))
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn enterprise_establishment_schedule_starts_at_commit_time() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let establishment = validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization: fixture.organization,
                authority: fixture.authority,
                location: fixture.location,
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: fixture.settlement,
            },
        )
        .expect("enterprise should validate before delayed commit");
        fixture.state.advance_clock(SimDuration::from_minutes(60));
        let enterprise = establishment
            .commit(&mut fixture.state)
            .expect("delayed enterprise establishment should commit");
        let record = fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("enterprise should exist");
        assert_eq!(record.established_at(), SimTime::from_minutes(60));
        assert_eq!(record.next_cycle_at(), Some(SimTime::from_minutes(1_500)));
        validate_invariants(&fixture.state);
    }

    #[test]
    fn stale_cycle_plan_cannot_commit_after_enterprise_lifecycle_change() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, 0)
            .expect("cycle should resolve");
        validate_suspend_enterprise(&fixture.state, enterprise)
            .expect("enterprise should suspend")
            .commit(&mut fixture.state)
            .expect("suspension should commit");

        let error = match validate_enterprise_cycle_plan(&fixture.state, plan) {
            Ok(_) => panic!("cycle plan must become stale after lifecycle mutation"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            EnterpriseError::StaleEnterprise {
                enterprise,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.cash)
                .expect("cash account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn active_enterprise_blocks_authority_removal_until_suspended() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        let mandate = fixture.authority.mandate;

        let revoke_error = validate_revoke_mandate(&fixture.state, mandate)
            .expect_err("active routine work must block mandate revocation");
        assert_eq!(
            revoke_error,
            DelegationError::ActiveEnterpriseDependency {
                mandate,
                enterprise,
            }
        );

        let replacement_scope = ResponsibilityScope::Function(ResponsibilityFunction::Finance);
        let revision_error = validate_revise_mandate(
            &registry,
            &fixture.state,
            mandate,
            MandateRevisionDraft {
                scopes: BTreeSet::from([replacement_scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect_err("active routine work must preserve its delegated scope");
        assert_eq!(
            revision_error,
            DelegationError::ActiveEnterpriseScopeDependency {
                mandate,
                enterprise,
                scope: fixture.authority.scope,
            }
        );

        validate_suspend_enterprise(&fixture.state, enterprise)
            .expect("enterprise should suspend before authority is removed")
            .commit(&mut fixture.state)
            .expect("enterprise suspension should commit");
        validate_revoke_mandate(&fixture.state, mandate)
            .expect("suspended routine work should release active mandate dependency")
            .commit(&mut fixture.state)
            .expect("mandate revocation should commit after suspension");

        let resume_error = match validate_resume_enterprise(&registry, &fixture.state, enterprise) {
            Ok(_) => panic!("enterprise must not resume under revoked authority"),
            Err(error) => error,
        };
        assert_eq!(
            resume_error,
            EnterpriseError::Delegation(DelegationError::InactiveMandate(mandate))
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn save_round_trip_preserves_due_schedule_and_deterministic_cycle_resolution() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));

        let envelope = build_save(&registry, &fixture.state)
            .expect("active enterprise state should build a valid save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored =
            restore_save(&registry, decoded).expect("enterprise save should restore cleanly");
        assert_eq!(
            restored
                .enterprises()
                .get_enterprise(enterprise)
                .expect("restored enterprise should exist")
                .next_cycle_at(),
            Some(SimTime::from_minutes(1_440))
        );

        let original_outcome = run_tick(&registry, &mut fixture.state);
        let restored_outcome = run_tick(&registry, &mut restored);
        assert_eq!(original_outcome, restored_outcome);
        assert_eq!(original_outcome.enterprise_cycles.len(), 1);
        let cycle = original_outcome.enterprise_cycles[0];
        let original_cycle = fixture
            .state
            .enterprises()
            .get_cycle(cycle)
            .expect("original cycle should exist");
        let restored_cycle = restored
            .enterprises()
            .get_cycle(cycle)
            .expect("restored continuation should create the same cycle ID");
        assert_eq!(
            original_cycle.gross_revenue(),
            restored_cycle.gross_revenue()
        );
        assert_eq!(
            original_cycle.operating_cost(),
            restored_cycle.operating_cost()
        );
        assert_eq!(original_cycle.net_cash(), restored_cycle.net_cash());
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.cash)
                .expect("original cash account should exist")
                .balance(),
            restored
                .finance()
                .get_account(fixture.cash)
                .expect("restored cash account should exist")
                .balance()
        );
        validate_invariants(&fixture.state);
        validate_invariants(&restored);
    }

    #[test]
    fn financial_reporting_drills_down_without_cached_totals() {
        let registry = build_registry();
        let mut fixture = make_test_enterprise_fixture();
        let enterprise = establish_protection(&registry, &mut fixture);
        for variance in [0, 700] {
            fixture
                .state
                .advance_clock(SimDuration::from_minutes(1_440));
            let plan = decide_enterprise_cycle(&registry, &fixture.state, enterprise, variance)
                .expect("due cycle should resolve for reporting fixture");
            validate_enterprise_cycle_plan(&fixture.state, plan)
                .expect("reporting fixture cycle should validate")
                .commit(&mut fixture.state)
                .expect("reporting fixture cycle should commit");
        }

        let period_start = SimTime::ZERO;
        let period_end = fixture.state.now();
        let enterprise_summary = resolve_enterprise_financial_summary(
            &fixture.state,
            enterprise,
            period_start,
            period_end,
        )
        .expect("enterprise financial summary should resolve");
        let organization_summary = resolve_organization_enterprise_financial_summary(
            &fixture.state,
            fixture.organization,
            period_start,
            period_end,
        )
        .expect("organization financial summary should resolve");
        let manager_summary = resolve_manager_enterprise_financial_summary(
            &fixture.state,
            fixture.authority.manager,
            period_start,
            period_end,
        )
        .expect("manager financial summary should resolve");
        let neighborhood = match fixture.location {
            EnterpriseLocation::Neighborhood(id) => id,
            EnterpriseLocation::Business(_) => panic!("fixture should use neighborhood location"),
        };
        let neighborhood_summary = resolve_neighborhood_enterprise_financial_summary(
            &fixture.state,
            neighborhood,
            period_start,
            period_end,
        )
        .expect("neighborhood financial summary should resolve");

        assert_eq!(enterprise_summary.totals.enterprise_count, 1);
        assert_eq!(enterprise_summary.totals.cycle_count, 2);
        assert_eq!(enterprise_summary.totals.notable_cycle_count, 1);
        assert_eq!(enterprise_summary.totals, organization_summary.totals);
        assert_eq!(enterprise_summary.totals, manager_summary.totals);
        assert_eq!(enterprise_summary.totals, neighborhood_summary.totals);
        assert_eq!(
            enterprise_summary
                .by_kind
                .get(&EnterpriseKind::Protection)
                .expect("protection bucket should exist"),
            &enterprise_summary.totals
        );
        let cycle_net = fixture
            .state
            .enterprises()
            .cycles_for(enterprise)
            .try_fold(Money::ZERO, |total, cycle| {
                total.checked_add(cycle.net_cash())
            })
            .expect("reporting fixture total should not overflow");
        assert_eq!(enterprise_summary.totals.net_cash, cycle_net);
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.cash)
                .expect("cash account should exist")
                .balance(),
            enterprise_summary.totals.net_cash
        );
        let report = validate_enterprise_financial_report(
            &fixture.state,
            fixture.organization,
            period_start,
            period_end,
        )
        .expect("financial report should synthesize only recipient-known enterprise information")
        .commit(&mut fixture.state)
        .expect("enterprise financial report should commit");
        let report = fixture
            .state
            .reports()
            .get_report(report)
            .expect("generated financial report should persist");
        assert_eq!(report.kind(), ReportKind::Financial);
        assert_eq!(report.entries().len(), 2);
        assert_eq!(report.entries()[0].attention, AttentionClass::Routine);
        assert_eq!(report.entries()[1].attention, AttentionClass::Notable);
        assert_eq!(report.entries()[1].sources.len(), 1);
        assert!(report.entries()[1]
            .entities
            .contains(&EntityRef::Enterprise(enterprise)));
        validate_invariants(&fixture.state);
    }
}
