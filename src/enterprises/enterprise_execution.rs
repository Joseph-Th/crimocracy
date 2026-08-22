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
    ensure_mandate_authority_current, resolve_mandate_authority, resolve_policy_for_manager,
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
use crate::finance::{AccountKind, FinancialOwner, LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{EnterpriseDefinition, EnterpriseEconomicsDefinition, Registry};
use crate::world::{
    BusinessFunction, BusinessOwner, CapabilityKind, Lifecycle, NeighborhoodProfile, Rating,
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
        ensure_mandate_authority_current(state, self.authority)?;
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
    /// Street-heat portion of `operating_cost`. Nonzero heat makes the cycle notable so the
    /// organization hears why its racket got more expensive while police work stays heavy.
    investigation_heat: Money,
    attention: AttentionClass,
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
    accounts: EnterpriseCycleAccounts,
}

impl EnterpriseCyclePlan {
    pub fn gross_revenue(&self) -> Money {
        self.economics.gross_revenue
    }
    pub fn operating_cost(&self) -> Money {
        self.economics.operating_cost
    }
    pub fn net_cash(&self) -> Money {
        self.economics.net_cash
    }
    pub fn investigation_heat(&self) -> Money {
        self.economics.investigation_heat
    }
    pub fn attention(&self) -> AttentionClass {
        self.economics.attention
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
        resolve_basis_point_variance(enterprise, gross_before_variance, variance_basis_points)?;
    let cost = resolve_operating_cost(
        state,
        enterprise,
        economics,
        neighborhood,
        record.location(),
        record.supporting_businesses().len(),
    )?;
    let operating_cost = cost.total;
    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    // Validate that the manager still holds the authored policy authority for this enterprise
    // kind. The resolved setting is delegation-owned state and is never persisted on the cycle.
    if let Some(policy_kind) = definition.policy() {
        resolve_policy_for_manager(state, record.manager(), policy_kind)?;
    }
    let variance_notable = i32::from(variance_basis_points).unsigned_abs()
        >= u32::from(economics.notable_variance_basis_points());
    // A hot district taxes the racket even on a normal-variance night; the manager reports that
    // cost pressure rather than letting the racket quietly underperform its expectations.
    let attention = if variance_notable || cost.investigation_heat > Money::ZERO {
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
            investigation_heat: cost.investigation_heat,
            attention,
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
        ensure_mandate_authority_current(state, self.plan.snapshot.authority)?;
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
                    investigation_heat: self.plan.economics.investigation_heat,
                },
                artifacts: super::EnterpriseCycleArtifacts {
                    attention: self.plan.economics.attention,
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
    ensure_mandate_authority_current(state, plan.snapshot.authority)?;
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
    // A balanced settlement moves no money, and the ledger rejects zero-value postings, so
    // net-zero cycles record their modeled gross/cost financials without a ledger transaction
    // (see `core::invariants::business` for the matching validity rule).
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
                summary: build_cycle_report_summary(state, record, &plan.economics),
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
            ensure_mandate_authority_current(state, authority)?;
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

pub(crate) fn find_due_enterprises(state: &AppState) -> Vec<EnterpriseId> {
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
    // Settlement-account exclusivity is permanent, including after the incumbent closes:
    // cycle history and provenance keep referencing the account, so reassigning it would
    // corrupt past settlements' ownership trail.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OperatingCostBreakdown {
    total: Money,
    /// Portion of `total` caused by active investigations in the enterprise's district.
    investigation_heat: Money,
}

fn resolve_operating_cost(
    state: &crate::core::state::AppState,
    enterprise: EnterpriseId,
    economics: &EnterpriseEconomicsDefinition,
    profile: NeighborhoodProfile,
    location: EnterpriseLocation,
    supporting_business_count: usize,
) -> Result<OperatingCostBreakdown, EnterpriseError> {
    let base = economics.base_operating_cost().checked_add(weighted_rating(
        enterprise,
        economics.police_cost_per_point(),
        profile.institutions.police_presence,
    )?);
    let base = base.ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let heat = resolve_investigation_heat_surcharge(state, enterprise, location, economics)?;
    let support_surcharge = economics
        .support_surcharge_per_business()
        .checked_mul(i64::try_from(supporting_business_count).expect("usize must fit i64"))
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let total = base
        .checked_add(heat)
        .and_then(|sum| sum.checked_add(support_surcharge))
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    Ok(OperatingCostBreakdown {
        total,
        investigation_heat: heat,
    })
}

/// The manager's cycle report to leadership. Heat-bearing cycles must say *why* cost rose —
/// the crew pays the street premium while police work stays heavy in their district — so the
/// player can connect their own exposed operations to the racket's shrinking margin without
/// any hidden case detail leaking into organization knowledge.
fn build_cycle_report_summary(
    state: &crate::core::state::AppState,
    record: &crate::enterprises::EnterpriseRecord,
    economics: &EnterpriseCycleEconomics,
) -> String {
    let base = format!(
        "Enterprise cycle reported gross {}, operating cost {}",
        crate::finance::helpers::format_money_cents(economics.gross_revenue.cents()),
        crate::finance::helpers::format_money_cents(economics.operating_cost.cents()),
    );
    let heat = if economics.investigation_heat > Money::ZERO {
        format!(
            ", including a {} street surcharge while police work stays heavy in {}",
            crate::finance::helpers::format_money_cents(economics.investigation_heat.cents()),
            resolve_enterprise_district_name(state, record),
        )
    } else {
        String::new()
    };
    format!(
        "{base}{heat}, net cash {}, and {}.",
        crate::finance::helpers::format_money_cents(economics.net_cash.cents()),
        crate::finance::helpers::describe_gross_variance(economics.variance_basis_points),
    )
}

fn resolve_enterprise_district_name(
    state: &crate::core::state::AppState,
    record: &crate::enterprises::EnterpriseRecord,
) -> String {
    let neighborhood = match record.location() {
        EnterpriseLocation::Neighborhood(id) => Some(id),
        EnterpriseLocation::Business(business_id) => state
            .world
            .get_business(business_id)
            .map(|business| business.neighborhood()),
    };
    neighborhood
        .and_then(|id| state.world.get_neighborhood(id))
        .map(|profile| profile.name().to_owned())
        .unwrap_or_else(|| "the district".to_owned())
}

fn resolve_investigation_heat_surcharge(
    state: &crate::core::state::AppState,
    enterprise: EnterpriseId,
    location: EnterpriseLocation,
    economics: &EnterpriseEconomicsDefinition,
) -> Result<Money, EnterpriseError> {
    let neighborhood = match location {
        EnterpriseLocation::Neighborhood(id) => id,
        EnterpriseLocation::Business(business_id) => {
            // The host business was validated as existing and active earlier in the same
            // decide cycle, so a missing record here would mean corrupted state.
            let business = state
                .world
                .get_business(business_id)
                .expect("validated enterprise host business must exist");
            business.neighborhood()
        }
    };
    let Some(authority) =
        crate::legal::jurisdiction_system::resolve_case_intake_authority(state, neighborhood)
    else {
        return Ok(Money::ZERO);
    };
    // Only operation-originated cases targeting this enterprise's neighborhood generate local
    // street heat; an authority spanning several districts must not tax rackets in districts
    // its cases never touched.
    let active = state
        .legal
        .investigations_for_owner(authority)
        .filter(|investigation| {
            investigation.status() == crate::legal::InvestigationStatus::Active
                && investigation.origin_operation().is_some()
                && crate::operations::operation_execution::resolve_investigation_target_neighborhoods(
                    state, investigation,
                )
                .contains(&neighborhood)
        })
        .count();
    if active == 0 {
        Ok(Money::ZERO)
    } else {
        // Street heat makes the racket more expensive to run (bribes, lookouts, missed nights);
        // the authored per-case rate must erode daily net without instantly bankrupting it.
        economics
            .heat_surcharge_per_active_case()
            .checked_mul(i64::try_from(active).expect("case count must fit i64"))
            .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
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

fn resolve_basis_point_variance(
    enterprise: EnterpriseId,
    amount: Money,
    basis_points: i16,
) -> Result<Money, EnterpriseError> {
    crate::finance::helpers::resolve_basis_point_variance(amount, basis_points)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))
}

#[cfg(test)]
mod tests;
