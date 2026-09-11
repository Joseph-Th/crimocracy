//! Enterprise cycle planning and atomic settlement, with establishment/lifecycle helpers in sibling modules.

mod economics;
mod establishment;
mod lifecycle;
mod support;

use support::{
    build_cycle_report_summary, build_vice_incident_draft, count_district_originated_cases,
    has_active_enterprise_inquiry, resolve_location_profile, snapshot_supporting_business_versions,
    validate_enterprise_accounts, validate_enterprise_business_dependencies,
    validate_enterprise_environment, validate_supporting_business_versions,
    validate_supporting_businesses,
};
pub(crate) use support::{can_authority_cover_location, resolve_location_neighborhood};

use economics::{
    resolve_basis_point_variance, resolve_gross_before_variance, resolve_operating_cost,
};
pub(crate) use economics::{
    resolve_enterprise_financial_projection, resolve_enterprise_operating_cost_projection,
    resolve_historical_enterprise_cycle_financials,
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
    NeighborhoodId, OrganizationId,
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
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::investigation_system::validate_incident_intake;
use crate::legal::jurisdiction_system::{
    CaseIntakeAuthoritySnapshot, CaseIntakeAuthoritySnapshotError,
    resolve_case_intake_authority_snapshot, validate_case_intake_authority_snapshot,
};
use crate::registry::{EnterpriseDefinition, Registry};
use crate::world::{
    BusinessFunction, BusinessOwner, CapabilityKind, NeighborhoodProfile, OrganizationKind,
};
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
        "enterprise {enterprise} vice intake routing changed for neighborhood {neighborhood}; expected authority {expected:?}, found {found:?}"
    )]
    StaleViceIntakeRouting {
        enterprise: EnterpriseId,
        neighborhood: NeighborhoodId,
        expected: Option<OrganizationId>,
        found: Option<OrganizationId>,
    },
    #[error(
        "enterprise {enterprise} vice intake jurisdiction changed for neighborhood {neighborhood}; organization {organization} expected version {expected_version}, found {found_version:?}"
    )]
    StaleViceIntakeJurisdictionVersion {
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
    /// Active district casework feeds both street-heat cost and vice probability. Pin the
    /// count so a held plan cannot settle economics from a legal-pressure picture that no
    /// longer exists.
    active_district_cases: u32,
    /// An existing dedicated inquiry suppresses another vice inquiry. This is a separate
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
    /// roll converted sustained district casework into a vice inquiry on this racket.
    vice_incident: Option<crate::legal::IncidentIntakeDraft>,
    /// Routing/version snapshot captured whenever the visibility roll hits, including an
    /// unroutable `None` authority. A held cycle token must re-decide if case-intake eligibility
    /// changes after planning, even when no incident draft existed initially.
    vice_authority: Option<CaseIntakeAuthoritySnapshot>,
}

/// Explicit per-cycle randomness injected by the tick pipeline so decide stays read-only and
/// every draw is visible in one scheduling surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnterpriseCycleRandomness {
    variance_basis_points: i16,
    vice_attention_roll: u16,
}

impl EnterpriseCycleRandomness {
    pub(crate) fn new(variance_basis_points: i16, vice_attention_roll: u16) -> Self {
        Self {
            variance_basis_points,
            vice_attention_roll,
        }
    }

    pub(crate) fn variance_basis_points(self) -> i16 {
        self.variance_basis_points
    }

    pub(crate) fn vice_attention_roll(self) -> u16 {
        self.vice_attention_roll
    }
}

pub fn decide_enterprise_cycle(
    registry: &Registry,
    state: &AppState,
    enterprise: EnterpriseId,
    randomness: EnterpriseCycleRandomness,
) -> Result<EnterpriseCyclePlan, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() != EnterpriseStatus::Active {
        return Err(EnterpriseError::EnterpriseNotActive(enterprise));
    }
    let Some(due_at) = record.next_cycle_at() else {
        return Err(EnterpriseError::SimulationTimeOverflow);
    };
    if state.now() < due_at {
        return Err(EnterpriseError::CycleNotDue { enterprise, due_at });
    }
    let definition = registry.get_enterprise(record.kind());
    let variance_basis_points = randomness.variance_basis_points();
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
        record.supporting_businesses(),
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
    let district = resolve_location_neighborhood(state, record.location())?;
    // Sustained originated casework in the racket's district is resolved once per cycle:
    // the shared pressure signal behind both the street-heat surcharge and vice attention.
    let active_district_cases = count_district_originated_cases(state, district);
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
        economics,
        neighborhood,
        record.supporting_businesses().len(),
        active_district_cases,
        enterprise,
    )?;
    let operating_cost = cost.total;
    // Active casework converts into vice attention: every cycle run under an active case
    // risks a dedicated inquiry on this racket. An already-active inquiry keeps contributing
    // district heat but cannot recursively open another concurrent inquiry into the same
    // racket; clean districts never draw one, so lying low or moving the book remain real
    // counter-play.
    let vice_chance_basis_points = u32::try_from(
        (u64::from(
            definition
                .economics()
                .vice_attention_basis_points_per_active_case(),
        ) * u64::from(active_district_cases))
        .min(10_000),
    )
    .expect("vice chance is explicitly capped to basis-point range");
    let had_active_enterprise_inquiry = has_active_enterprise_inquiry(state, enterprise);
    let vice_roll_hits = active_district_cases > 0
        && !had_active_enterprise_inquiry
        && u32::from(randomness.vice_attention_roll()) < vice_chance_basis_points;
    let vice_authority =
        vice_roll_hits.then(|| resolve_case_intake_authority_snapshot(state, district));
    let vice_incident = vice_authority.and_then(|authority| {
        authority
            .organization
            .map(|owner| build_vice_incident_draft(state, enterprise, record, owner, state.now()))
    });
    // A visibility roll is only an actual vice event when an institution currently exists to
    // own the case. Hot districts can retain pressure from old cases after jurisdiction moves
    // away; in that state the roll is unspent rather than becoming a phantom notable event.
    let draws_vice_attention = vice_incident.is_some();
    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(EnterpriseError::ArithmeticOverflow(enterprise))?;
    let variance_notable = i32::from(variance_basis_points).unsigned_abs()
        >= u32::from(economics.notable_variance_basis_points());
    // A losing night is always manager-report-worthy: chronic silent losses are exactly what
    // the authority must see before the authored suspension threshold stops the bleeding.
    // Street heat is report-worthy when it *appears or changes* — the first taxed cycle (and
    // any later change in the surcharge) tells the organization why its racket got more
    // expensive. A sustained identical surcharge is a known cost, not fresh news: repeating it
    // every cycle would bury real exceptions in alert noise, so it settles as routine and
    // stays visible through the financial summaries instead.
    let previous_heat = latest_cycle_investigation_heat(state, enterprise);
    let heat_reportable =
        enterprise_heat_change_is_reportable(previous_heat, cost.investigation_heat);
    let attention =
        if variance_notable || net_cash < Money::ZERO || draws_vice_attention || heat_reportable {
            AttentionClass::Notable
        } else {
            AttentionClass::Routine
        };
    let trailing_losing_cycles = count_trailing_losing_cycles(
        state,
        enterprise,
        economics.losing_cycles_before_suspension(),
    );
    // A losing settlement that reaches the authored consecutive-loss threshold suspends the
    // racket: the domain owner acts on the negative result instead of scheduling another
    // identical loss. Resumption is a manual canonical decision.
    let suspends_after_settlement = net_cash < Money::ZERO
        && trailing_losing_cycles + 1 >= u32::from(economics.losing_cycles_before_suspension());
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
    // The current settlement is already due and executable. If only the *next* recurrence lies
    // past the finite clock, preserve the present result and persist an exhausted recurrence
    // instead of turning valid current work into a scheduling failure.
    let next_cycle_at = state.now().checked_add(economics.cycle());
    Ok(EnterpriseCyclePlan {
        snapshot: EnterpriseCycleSnapshot {
            enterprise,
            expected_enterprise_version: record.version(),
            authority,
            occurred_at: state.now(),
            // A detained manager leaves the enterprise overdue, but missed cycles are not
            // retroactively paid out in a burst after release. Re-anchor the next cycle to the
            // actual settlement instant so routine work resumes at its authored cadence.
            next_cycle_at,
            suspends_after_settlement,
            supporting_business_versions,
            host_business_version,
            active_district_cases,
            had_active_enterprise_inquiry,
        },
        economics: EnterpriseCycleEconomics {
            gross_revenue,
            operating_cost,
            net_cash,
            variance_basis_points,
            investigation_heat: cost.investigation_heat,
            previous_investigation_heat: previous_heat,
            attention,
        },
        accounts: EnterpriseCycleAccounts {
            cash_account: record.cash_account(),
            settlement_account: record.settlement_account(),
        },
        vice_incident,
        vice_authority,
    })
}

/// The street-heat portion of the enterprise's most recently settled cycle, or `None` when the
/// racket has never settled. Delegates to the owner's O(log n) latest-cycle lookup.
fn latest_cycle_investigation_heat(state: &AppState, enterprise: EnterpriseId) -> Option<Money> {
    state
        .enterprises
        .latest_cycle(enterprise)
        .map(|cycle| cycle.investigation_heat())
}

fn validate_vice_intake_authority_snapshot(
    state: &AppState,
    enterprise: EnterpriseId,
    snapshot: CaseIntakeAuthoritySnapshot,
) -> Result<(), EnterpriseError> {
    validate_case_intake_authority_snapshot(state, snapshot).map_err(|error| match error {
        CaseIntakeAuthoritySnapshotError::Routing {
            neighborhood,
            expected,
            found,
        } => EnterpriseError::StaleViceIntakeRouting {
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
        } => EnterpriseError::StaleViceIntakeJurisdictionVersion {
            enterprise,
            neighborhood,
            organization,
            expected_version,
            found_version,
        },
    })
}

/// One owner for whether a heat transition deserves a fresh manager report. The first positive
/// surcharge is new information, any change between positive levels is new information, and a
/// drop from positive heat to zero is recovery worth surfacing. A never-hot zero cycle and an
/// unchanged surcharge stay routine.
pub(crate) fn enterprise_heat_change_is_reportable(
    previous_heat: Option<Money>,
    current_heat: Money,
) -> bool {
    previous_heat != Some(current_heat)
        && (current_heat > Money::ZERO || previous_heat.is_some_and(|heat| heat > Money::ZERO))
}

/// Consecutive most-recent settled cycles whose net cash was negative, capped at `limit` so
/// the scan stays bounded regardless of how much history a long-lived racket accumulates.
/// Cycles settled before the enterprise's loss-streak anchor predate its current grace window
/// (a resumed racket starts counting fresh) and do not extend the streak.
fn count_trailing_losing_cycles(state: &AppState, enterprise: EnterpriseId, limit: u8) -> u32 {
    let anchor = state
        .enterprises
        .get_enterprise(enterprise)
        .and_then(|record| record.loss_streak_anchor());
    crate::finance::helpers::count_trailing_losing_cycles(
        state
            .enterprises
            .cycles_for(enterprise)
            .rev()
            .take(usize::from(limit)),
        |cycle| cycle.occurred_at(),
        |cycle| cycle.net_cash(),
        anchor,
        limit,
    )
}

pub struct ValidatedEnterpriseCycle {
    plan: EnterpriseCyclePlan,
    ledger: Option<ValidatedLedgerTransaction>,
    information: Option<ValidatedInformation>,
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
    pub fn commit(self, state: &mut AppState) -> Result<EnterpriseCycleId, EnterpriseError> {
        let mut budget = Vec::new();
        if self.ledger.is_some() {
            budget.push((IdKind::LedgerTransaction, 1));
        }
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
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
        ensure_version_can_advance_by(
            record.version(),
            1 + u32::from(self.plan.snapshot.suspends_after_settlement),
            "enterprise",
        )?;
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
        validate_legal_pressure_context(state, record, &self.plan.snapshot)?;
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
                return Err(EnterpriseError::StaleHostBusiness {
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
        if let Some(snapshot) = self.plan.vice_authority {
            validate_vice_intake_authority_snapshot(
                state,
                self.plan.snapshot.enterprise,
                snapshot,
            )?;
        }
        if let Some(incident) = &self.incident {
            incident.ensure_current(state)?;
        }
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
        let vice_investigation = self.incident.map(|incident| {
            incident
                .commit(state)
                .expect("preflighted enterprise vice intake must remain current during settlement")
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
                    drew_vice_attention: vice_investigation.is_some(),
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
            // scheduling another identical loss. Resumption is a manual canonical decision.
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
    if state.now() != plan.snapshot.occurred_at {
        return Err(EnterpriseError::StaleCycleTime {
            expected: plan.snapshot.occurred_at,
            found: state.now(),
        });
    }
    ensure_mandate_authority_current(state, plan.snapshot.authority)?;
    validate_legal_pressure_context(state, record, &plan.snapshot)?;
    validate_supporting_business_versions(state, &plan.snapshot.supporting_business_versions)?;
    if let Some((business_id, expected)) = plan.snapshot.host_business_version {
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
    if let Some(snapshot) = plan.vice_authority {
        validate_vice_intake_authority_snapshot(state, plan.snapshot.enterprise, snapshot)?;
    }
    debug_assert!(
        match (&plan.vice_incident, plan.vice_authority) {
            (Some(incident), Some(snapshot)) => snapshot.organization == Some(incident.owner),
            (None, Some(snapshot)) => snapshot.organization.is_none(),
            (None, None) => true,
            (Some(_), None) => false,
        },
        "vice planning must pair incidents with their routed authority and unroutable hits with an explicit absent-authority snapshot"
    );
    let drew_vice_attention = plan.vice_incident.is_some();
    let incident = match &plan.vice_incident {
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
                summary: build_cycle_report_summary(
                    state,
                    record,
                    &plan.economics,
                    drew_vice_attention,
                    plan.snapshot.suspends_after_settlement,
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
