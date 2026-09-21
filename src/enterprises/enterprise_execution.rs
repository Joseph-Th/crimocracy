//! Enterprise cycle settlement facade, with planning and lifecycle concerns split into focused siblings.

mod cycle_planning;
mod economics;
mod establishment;
mod lifecycle;
mod support;

use support::{
    build_cycle_report_summary, count_district_originated_cases, has_active_enterprise_inquiry,
    resolve_location_profile, snapshot_supporting_business_versions, validate_enterprise_accounts,
    validate_enterprise_business_dependencies, validate_enterprise_environment,
    validate_supporting_business_versions, validate_supporting_businesses,
};
pub(crate) use support::{can_authority_cover_location, resolve_location_neighborhood};

pub use cycle_planning::decide_enterprise_cycle;
pub(crate) use cycle_planning::enterprise_heat_change_is_reportable;

pub(crate) use economics::{
    decode_enterprise_investigation_case_count, resolve_enterprise_financial_projection,
    resolve_enterprise_operating_cost_projection, resolve_historical_enterprise_cycle_financials,
};
pub use establishment::{ValidatedEnterpriseEstablishment, validate_establish_enterprise};
pub(crate) use establishment::{
    enterprise_location_is_occupied, validate_establish_enterprise_with_openings,
};
pub use lifecycle::{
    ValidatedEnterpriseStatusChange, validate_resume_enterprise, validate_retire_enterprise,
    validate_suspend_enterprise,
};

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessId, EnterpriseCycleId, EnterpriseId, FinancialAccountId, IdExhaustionError, IdKind,
    InformationId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
use crate::delegation::delegation_system::{
    DelegationError, ensure_mandate_authority_current, resolve_mandate_authority,
};
use crate::delegation::{
    MandateAuthority, ResolvedMandateAuthority, ResponsibilityFunction, ResponsibilityScope,
};
use crate::enterprises::{
    EnterpriseCycleRecord, EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseStatus,
    build_enterprise_record,
};
use crate::finance::finance_system::{
    FinanceError, ValidatedFinancialAccountOpenings, ValidatedLedgerTransaction,
    validate_record_transaction,
};
use crate::finance::{AccountKind, FinancialOwner, LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    IntelligenceError, PlannedInformationSource, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::investigation_system::validate_incident_intake;
use crate::legal::jurisdiction_system::{
    CaseIntakeAuthoritySnapshot, CaseIntakeAuthoritySnapshotError,
    validate_case_intake_authority_snapshot,
};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::reports::report_system::{
    ReportError, ValidatedReport, validate_record_report_with_planned_information,
};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
#[cfg(test)]
use crate::world::CapabilityKind;
use crate::world::{BusinessFunction, BusinessOwner, NeighborhoodProfile, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EnterpriseError {
    #[error("enterprise {0} does not exist")]
    MissingEnterprise(EnterpriseId),
    #[error("organization {0} does not exist")]
    InvalidOrganization(OrganizationId),
    #[error("enterprise organization {0} is not a criminal organization")]
    InvalidOrganizationKind(OrganizationId),
    #[error(
        "enterprise authority belongs to organization {authority_organization}, not {enterprise_organization}"
    )]
    AuthorityOrganizationMismatch {
        authority_organization: OrganizationId,
        enterprise_organization: OrganizationId,
    },
    #[error("authority scope {scope:?} does not cover enterprise location {location:?}")]
    AuthorityLocationMismatch {
        scope: ResponsibilityScope,
        location: EnterpriseLocation,
    },
    #[error("authority scope {scope:?} does not cover supporting business {business}")]
    AuthoritySupportingBusinessMismatch {
        scope: ResponsibilityScope,
        business: BusinessId,
    },
    #[error("enterprise location {0:?} does not exist")]
    InvalidLocation(EnterpriseLocation),
    #[error("business {business} lacks required enterprise function {function:?}")]
    MissingBusinessFunction {
        business: BusinessId,
        function: BusinessFunction,
    },
    #[error("supporting business {0} does not exist")]
    InvalidSupportingBusiness(BusinessId),
    #[error(
        "supporting business {business} is owned by {owner:?}, not enterprise organization {organization}"
    )]
    SupportingBusinessOwnershipMismatch {
        business: BusinessId,
        owner: BusinessOwner,
        organization: OrganizationId,
    },
    #[error(
        "hosted business {business} is owned by {owner:?}, not enterprise organization {organization}"
    )]
    HostBusinessOwnershipMismatch {
        business: BusinessId,
        owner: BusinessOwner,
        organization: OrganizationId,
    },
    #[error("supporting business {business} duplicates the enterprise's hosted business location")]
    DuplicateSupportingLocation { business: BusinessId },
    #[error("enterprise support network lacks required function {function:?}")]
    MissingNetworkFunction { function: BusinessFunction },
    #[error(
        "supporting business {business} changed after validation; expected version {expected}, found {found}"
    )]
    StaleSupportingBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "hosted business {business} changed after validation; expected version {expected}, found {found}"
    )]
    StaleHostBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error(
        "pre-opened settlement account {account} does not match the establishment draft's settlement account"
    )]
    SettlementOpeningsMismatch { account: FinancialAccountId },
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
    #[error("enterprise {0} is retired and cannot return to operation")]
    EnterpriseRetired(EnterpriseId),
    #[error(transparent)]
    Investigation(#[from] crate::legal::investigation_system::InvestigationError),
    #[error("enterprise {enterprise} is not due for a cycle until {due_at:?}")]
    CycleNotDue {
        enterprise: EnterpriseId,
        due_at: SimTime,
    },
    #[error("enterprise cycle variance {basis_points} basis points exceeds authored limit {limit}")]
    VarianceOutOfRange { basis_points: i16, limit: u16 },
    #[error("enterprise economics overflowed while resolving cycle {0}")]
    ArithmeticOverflow(EnterpriseId),
    #[error("enterprise scheduling exceeds the representable simulation clock")]
    SimulationTimeOverflow,
    #[error("enterprise economics overflowed while projecting {kind:?} at {location:?}")]
    ProjectionArithmeticOverflow {
        kind: EnterpriseKind,
        location: EnterpriseLocation,
    },
    #[error(
        "enterprise {enterprise} changed after validation; expected version {expected}, found {found}"
    )]
    StaleEnterprise {
        enterprise: EnterpriseId,
        expected: u32,
        found: u32,
    },
    #[error("a {kind:?} enterprise already exists at {location:?}")]
    DuplicateEnterpriseAtLocation {
        kind: EnterpriseKind,
        location: EnterpriseLocation,
    },
    #[error(
        "enterprise cycle plan was resolved at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleCycleTime { expected: SimTime, found: SimTime },
    #[error("enterprise {enterprise} legal-pressure context changed after cycle planning")]
    StaleLegalPressureContext {
        enterprise: EnterpriseId,
        expected_active_district_cases: u32,
        found_active_district_cases: u32,
        expected_active_inquiry: bool,
        found_active_inquiry: bool,
    },
    #[error(
        "enterprise {enterprise} racket intake routing changed for neighborhood {neighborhood}; expected authority {expected:?}, found {found:?}"
    )]
    StaleEnforcementIntakeRouting {
        enterprise: EnterpriseId,
        neighborhood: NeighborhoodId,
        expected: Option<OrganizationId>,
        found: Option<OrganizationId>,
    },
    #[error(
        "enterprise {enterprise} racket intake jurisdiction changed for neighborhood {neighborhood}; organization {organization} expected version {expected_version}, found {found_version:?}"
    )]
    StaleEnforcementIntakeJurisdictionVersion {
        enterprise: EnterpriseId,
        neighborhood: NeighborhoodId,
        organization: OrganizationId,
        expected_version: u32,
        found_version: Option<u32>,
    },
    #[error(transparent)]
    Delegation(#[from] DelegationError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(
        "enterprise cycle information allocation changed after validation; expected {expected}, found {found}"
    )]
    StaleInformationAllocation {
        expected: InformationId,
        found: InformationId,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleSnapshot {
    enterprise: EnterpriseId,
    expected_enterprise_version: u32,
    authority: ResolvedMandateAuthority,
    occurred_at: SimTime,
    /// `None` means this cycle settled successfully but its next authored recurrence lies beyond
    /// the finite simulation clock. The enterprise stays live and authoritative, but there is no
    /// further representable settlement instant to schedule.
    next_cycle_at: Option<SimTime>,
    /// Set when this losing settlement reaches the authored consecutive-loss threshold:
    /// commit suspends the enterprise instead of leaving the next cycle scheduled.
    suspends_after_settlement: bool,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
    host_business_version: Option<(BusinessId, u32)>,
    /// Active district casework feeds both street-heat cost and enforcement probability. Pin the
    /// count so a held plan cannot settle economics from a legal-pressure picture that no
    /// longer exists.
    active_district_cases: u32,
    /// An existing dedicated inquiry suppresses another racket inquiry. This is a separate
    /// dependency from district case count because a case can open or close without changing
    /// the total district pressure count.
    had_active_enterprise_inquiry: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleEconomics {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    /// Street-heat portion of `operating_cost`. Heat that appears or changes makes the cycle
    /// notable so the organization hears both escalation and recovery; a sustained identical
    /// surcharge settles as routine.
    investigation_heat: Money,
    previous_investigation_heat: Option<Money>,
    attention: AttentionClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EnterpriseCycleAccounts {
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Debug)]
pub struct EnterpriseCyclePlan {
    snapshot: EnterpriseCycleSnapshot,
    economics: EnterpriseCycleEconomics,
    accounts: EnterpriseCycleAccounts,
    /// Validated at commit through the canonical intake path when this cycle's visibility
    /// roll converted sustained district casework into a racket inquiry on this racket.
    enforcement_incident: Option<crate::legal::IncidentIntakeDraft>,
    /// Routing/version snapshot captured whenever the visibility roll hits, including an
    /// unroutable `None` authority. A held cycle token must re-decide if case-intake eligibility
    /// changes after planning, even when no incident draft existed initially.
    enforcement_authority: Option<CaseIntakeAuthoritySnapshot>,
}

/// Explicit per-cycle randomness injected by the tick pipeline so decide stays read-only and
/// every draw is visible in one scheduling surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnterpriseCycleRandomness {
    variance_basis_points: i16,
    enforcement_attention_roll: u16,
}

impl EnterpriseCycleRandomness {
    pub(crate) const ENFORCEMENT_ATTENTION_ROLL_COUNT: usize = 10_000;
    pub(crate) const MAX_ENFORCEMENT_ATTENTION_ROLL: u16 = 9_999;

    pub(crate) fn new(variance_basis_points: i16, enforcement_attention_roll: u16) -> Self {
        debug_assert!(
            enforcement_attention_roll <= Self::MAX_ENFORCEMENT_ATTENTION_ROLL,
            "enterprise enforcement-attention roll must be in the production 0..10000 basis-point domain"
        );
        Self {
            variance_basis_points,
            enforcement_attention_roll,
        }
    }

    pub(crate) fn variance_basis_points(self) -> i16 {
        self.variance_basis_points
    }

    pub(crate) fn enforcement_attention_roll(self) -> u16 {
        self.enforcement_attention_roll
    }
}

fn validate_enforcement_intake_authority_snapshot(
    state: &AppState,
    enterprise: EnterpriseId,
    snapshot: CaseIntakeAuthoritySnapshot,
) -> Result<(), EnterpriseError> {
    validate_case_intake_authority_snapshot(state, snapshot).map_err(|error| match error {
        CaseIntakeAuthoritySnapshotError::Routing {
            neighborhood,
            expected,
            found,
        } => EnterpriseError::StaleEnforcementIntakeRouting {
            enterprise,
            neighborhood,
            expected,
            found,
        },
        CaseIntakeAuthoritySnapshotError::JurisdictionVersion {
            neighborhood,
            organization,
            expected_version,
            found_version,
        } => EnterpriseError::StaleEnforcementIntakeJurisdictionVersion {
            enterprise,
            neighborhood,
            organization,
            expected_version,
            found_version,
        },
    })
}

pub struct ValidatedEnterpriseCycle {
    plan: EnterpriseCyclePlan,
    ledger: Option<ValidatedLedgerTransaction>,
    information: Option<ValidatedInformation>,
    report: Option<ValidatedReport>,
    expected_information_id: Option<InformationId>,
    incident: Option<crate::legal::investigation_system::ValidatedIncidentIntake>,
}

fn validate_legal_pressure_context(
    state: &AppState,
    record: &crate::enterprises::EnterpriseRecord,
    snapshot: &EnterpriseCycleSnapshot,
) -> Result<(), EnterpriseError> {
    let district = resolve_location_neighborhood(state, record.location())?;
    let active_district_cases = count_district_originated_cases(state, district);
    let active_inquiry = has_active_enterprise_inquiry(state, record.id());
    if active_district_cases != snapshot.active_district_cases
        || active_inquiry != snapshot.had_active_enterprise_inquiry
    {
        return Err(EnterpriseError::StaleLegalPressureContext {
            enterprise: record.id(),
            expected_active_district_cases: snapshot.active_district_cases,
            found_active_district_cases: active_district_cases,
            expected_active_inquiry: snapshot.had_active_enterprise_inquiry,
            found_active_inquiry: active_inquiry,
        });
    }
    Ok(())
}

fn validate_enterprise_accounts_with_planned_settlement(
    state: &AppState,
    organization: OrganizationId,
    cash_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    openings: &ValidatedFinancialAccountOpenings,
) -> Result<(), EnterpriseError> {
    openings.ensure_current(state)?;
    validate_cash_account_kind(state, organization, cash_account)?;
    if !openings.account_matches(
        settlement_account,
        FinancialOwner::Organization(organization),
        AccountKind::Settlement,
    ) {
        return Err(EnterpriseError::InvalidSettlementAccountKind(
            settlement_account,
        ));
    }
    debug_assert!(
        state
            .enterprises
            .get_by_settlement_account(settlement_account)
            .is_none(),
        "a future account id cannot already back an enterprise"
    );
    Ok(())
}

fn validate_cash_account_kind(
    state: &AppState,
    organization: OrganizationId,
    cash_account: FinancialAccountId,
) -> Result<(), EnterpriseError> {
    let cash = state
        .finance
        .get_account(cash_account)
        .ok_or(EnterpriseError::MissingAccount(cash_account))?;
    if cash.owner() != FinancialOwner::Organization(organization) {
        return Err(EnterpriseError::AccountOwnerMismatch {
            account: cash_account,
            organization,
        });
    }
    match cash.kind() {
        AccountKind::StreetCash | AccountKind::ConcealedCash => Ok(()),
        AccountKind::AccountedFunds
        | AccountKind::LegitimateOperating
        | AccountKind::Settlement => Err(EnterpriseError::InvalidCashAccountKind(cash_account)),
    }
}

impl ValidatedEnterpriseCycle {
    fn id_budget(&self) -> Result<Vec<(IdKind, u32)>, EnterpriseError> {
        let mut budget = Vec::new();
        if self.ledger.is_some() {
            budget.push((IdKind::LedgerTransaction, 1));
        }
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        if self.plan.economics.attention == AttentionClass::Notable {
            budget.push((IdKind::Report, 1));
        }
        if let Some(incident) = &self.incident {
            budget.push((
                IdKind::Investigation,
                u32::from(incident.requires_new_investigation()),
            ));
            budget.push((IdKind::Evidence, incident.evidence_count()?));
            budget.push((IdKind::CaseWitness, u32::from(incident.has_witness())));
        }
        budget.push((IdKind::EnterpriseCycle, 1));
        Ok(budget)
    }

    fn ensure_current(&self, state: &AppState) -> Result<(), EnterpriseError> {
        if let Some(expected) = self.expected_information_id {
            let found = InformationId::from_raw(state.ids.next_raw(IdKind::Information));
            if found != expected {
                return Err(EnterpriseError::StaleInformationAllocation { expected, found });
            }
        }
        validate_enterprise_cycle_snapshot_current(state, &self.plan)?;
        if let Some(incident) = &self.incident {
            incident.ensure_current(state)?;
        }
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<EnterpriseCycleId, EnterpriseError> {
        self.ensure_current(state)?;
        state.ids.reserve_many(&self.id_budget()?)?;
        // Settlement atomicity rests on the ID budget reserved above: every downstream commit
        // (ledger, information, cycle) consumes pre-reserved IDs and cannot fail after the
        // first one mutates state.
        let transaction = match self.ledger {
            Some(ledger) => Some(ledger.commit(state)?),
            None => None,
        };
        let information = self.information.map(|information| {
            information
                .commit(state)
                .expect("enterprise-cycle information ID was preflighted before mutation")
        });
        if let Some(report) = self.report {
            debug_assert_eq!(information, self.expected_information_id);
            report
                .commit(state)
                .expect("enterprise report ID was preflighted before mutation");
        }
        let enforcement_investigation = self.incident.map(|incident| {
            incident
                .commit(state)
                .expect(
                    "preflighted enterprise racket intake must remain current during settlement",
                )
                .investigation
        });
        let cycle_id = state
            .ids
            .next_enterprise_cycle()
            .expect("enterprise-cycle ID was preflighted before settlement mutation");
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
                    drew_enforcement_attention: enforcement_investigation.is_some(),
                },
                provenance: super::EnterpriseCycleProvenance {
                    transaction,
                    information,
                },
            },
            self.plan.snapshot.next_cycle_at,
        );
        if self.plan.snapshot.suspends_after_settlement {
            // Domain-owner consequence for chronic losses: suspend the racket instead of
            // scheduling another identical loss. Any restart still uses the canonical resume
            // token; eligible non-player rackets may exercise it during daily maintenance.
            state.enterprises.set_status(
                self.plan.snapshot.enterprise,
                EnterpriseStatus::Suspended,
                None,
                None,
                state.now(),
            );
        }
        Ok(cycle_id)
    }
}

fn validate_enterprise_cycle_snapshot_current<'a>(
    state: &'a AppState,
    plan: &EnterpriseCyclePlan,
) -> Result<&'a crate::enterprises::EnterpriseRecord, EnterpriseError> {
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
    ensure_version_can_advance_by(
        record.version(),
        1 + u32::from(plan.snapshot.suspends_after_settlement),
        "enterprise",
    )?;
    if record.status() != EnterpriseStatus::Active {
        return Err(EnterpriseError::EnterpriseNotActive(
            plan.snapshot.enterprise,
        ));
    }
    crate::core::time::ensure_time_current(state.now(), plan.snapshot.occurred_at)
        .map_err(|(expected, found)| EnterpriseError::StaleCycleTime { expected, found })?;
    ensure_mandate_authority_current(state, plan.snapshot.authority)?;
    validate_legal_pressure_context(state, record, &plan.snapshot)?;
    validate_supporting_business_versions(state, &plan.snapshot.supporting_business_versions)?;
    validate_enterprise_cycle_host_business(state, record, &plan.snapshot)?;
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
    if let Some(snapshot) = plan.enforcement_authority {
        validate_enforcement_intake_authority_snapshot(state, plan.snapshot.enterprise, snapshot)?;
    }
    Ok(record)
}

fn validate_enterprise_cycle_host_business(
    state: &AppState,
    record: &crate::enterprises::EnterpriseRecord,
    snapshot: &EnterpriseCycleSnapshot,
) -> Result<(), EnterpriseError> {
    let Some((business_id, expected)) = snapshot.host_business_version else {
        return Ok(());
    };
    let business = state
        .world
        .get_business(business_id)
        .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
    if business.version() != expected {
        return Err(EnterpriseError::StaleHostBusiness {
            business: business_id,
            expected,
            found: business.version(),
        });
    }
    Ok(())
}

pub fn validate_enterprise_cycle_plan(
    state: &AppState,
    plan: EnterpriseCyclePlan,
) -> Result<ValidatedEnterpriseCycle, EnterpriseError> {
    let record = validate_enterprise_cycle_snapshot_current(state, &plan)?;
    debug_assert!(
        match (&plan.enforcement_incident, plan.enforcement_authority) {
            (Some(incident), Some(snapshot)) => snapshot.organization == Some(incident.owner),
            (None, Some(snapshot)) => snapshot.organization.is_none(),
            (None, None) => true,
            (Some(_), None) => false,
        },
        "enforcement planning must pair incidents with their routed authority and unroutable hits with an explicit absent-authority snapshot"
    );
    let drew_enforcement_attention = plan.enforcement_incident.is_some();
    let incident = match &plan.enforcement_incident {
        Some(draft) => Some(validate_incident_intake(state, draft.clone())?),
        None => None,
    };
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
    let (information, report, expected_information_id) = match plan.economics.attention {
        AttentionClass::Notable => {
            let summary = build_cycle_report_summary(
                state,
                record,
                &plan.economics,
                drew_enforcement_attention,
                plan.snapshot.suspends_after_settlement,
            );
            let information = validate_record_information(
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
                    summary: summary.clone(),
                },
            )?;
            let planned_source: PlannedInformationSource = information.planned_source(state);
            let expected_information_id = planned_source.id();
            let report = validate_record_report_with_planned_information(
                state,
                ReportDraft {
                    recipient: record.organization(),
                    kind: ReportKind::Financial,
                    title: "Enterprise cycle report".to_owned(),
                    entries: vec![ReportEntry {
                        attention: plan.economics.attention,
                        summary,
                        sources: vec![expected_information_id],
                        entities: BTreeSet::from([
                            EntityRef::Enterprise(record.id()),
                            EntityRef::Character(record.manager()),
                        ]),
                        decision: None,
                    }],
                },
                planned_source,
            )?;
            (
                Some(information),
                Some(report),
                Some(expected_information_id),
            )
        }
        AttentionClass::Routine => (None, None, None),
        AttentionClass::Exception | AttentionClass::Crisis => {
            unreachable!("enterprise cycle plans only produce routine or notable attention")
        }
    };
    Ok(ValidatedEnterpriseCycle {
        plan,
        ledger,
        information,
        report,
        expected_information_id,
        incident,
    })
}

pub(crate) fn find_due_enterprises(state: &AppState) -> Vec<EnterpriseId> {
    state
        .enterprises
        .find_due_cycles(state.now())
        .into_iter()
        .filter(|enterprise| {
            let record = state
                .enterprises
                .get_enterprise(*enterprise)
                .expect("due-enterprise index must reference a persisted enterprise");
            state
                .legal
                .active_arrest_for_character(record.manager())
                .is_none()
        })
        .collect()
}

#[cfg(test)]
mod tests;
