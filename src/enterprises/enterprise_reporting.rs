//! Read-only financial aggregation over enterprise cycle history; ledger-backed cycle records remain the source of truth.

use crate::core::attention::AttentionClass;
use crate::core::id::{CharacterId, EnterpriseId, FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::enterprises::autonomous_planning::resolve_observed_district_pressure;
use crate::enterprises::enterprise_execution::{
    EnterpriseError, EnterpriseProjectionStaffing, resolve_enterprise_financial_projection,
    resolve_location_neighborhood, validate_establish_enterprise,
};
use crate::enterprises::{
    EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseRecord, EnterpriseStatus,
};
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::CapabilityKind;
use crate::world::organization_pressure::vice_inquiry_chance_basis_points;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnterpriseFinancialTotals {
    pub enterprise_count: u32,
    pub cycle_count: u32,
    pub notable_cycle_count: u32,
    pub gross_revenue: Money,
    pub operating_cost: Money,
    pub net_cash: Money,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterpriseFinancialSummary {
    pub period_start: SimTime,
    pub period_end: SimTime,
    pub totals: EnterpriseFinancialTotals,
    pub by_kind: BTreeMap<EnterpriseKind, EnterpriseFinancialTotals>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EnterpriseReportingError {
    #[error("financial reporting window starts after it ends")]
    InvalidWindow,
    #[error("financial reporting window ends after current simulation time")]
    FutureWindow,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("enterprise financial aggregation overflowed")]
    ArithmeticOverflow,
}

/// Conditional operating estimate based only on the organization's own recent settlements.
/// A missing observation is explicitly unknown, never evidence that the district is quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnterpriseOutlook {
    pub baseline_net_cash: Money,
    pub heat_surcharge_per_case: Money,
    /// First number of case equivalents at which expected net is nonpositive.
    /// None means no finite threshold under the authored surcharge.
    pub nonpositive_at_cases: Option<u64>,
    pub estimated_operating_cost: Money,
    pub estimated_net_cash: Money,
    pub observed_pressure: Option<(SimTime, u32)>,
    /// Authored chance contributed by each exposure source.
    pub inquiry_risk_basis_points_per_source: u16,
    /// Projected fresh inquiry risk with no current district cases, including the
    /// visible commercial footprint and the planned operating racket roster.
    pub inquiry_risk_no_cases_basis_points: u32,
    pub shared_cash_balance: Money,
    pub shared_spendable_cash: Money,
}

#[derive(Debug, Error)]
pub enum EnterpriseOutlookError {
    #[error("enterprise {0} is not owned by this organization")]
    NotOwned(EnterpriseId),
    #[error("enterprise manager {0} does not exist")]
    MissingManager(CharacterId),
    #[error("enterprise cash account {0} does not exist")]
    MissingCashAccount(FinancialAccountId),
    #[error(transparent)]
    Enterprise(#[from] EnterpriseError),
    #[error("enterprise projection could not be calculated: {0}")]
    Projection(String),
}

pub fn resolve_enterprise_outlook(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    enterprise: EnterpriseId,
) -> Result<EnterpriseOutlook, EnterpriseOutlookError> {
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .filter(|record| record.organization() == organization)
        .ok_or(EnterpriseOutlookError::NotOwned(enterprise))?;
    resolve_configuration_outlook(
        registry,
        state,
        OutlookConfiguration {
            organization,
            kind: record.kind(),
            location: record.location(),
            support_count: record.supporting_businesses().len(),
            manager: record.manager(),
            cash: record.cash_account(),
            associates: record.associates(),
            additional_active_racket: record.status() != EnterpriseStatus::Active,
        },
    )
}

/// Validate an investment using the same owner that establishes it. The preview neither
/// reserves IDs nor consumes randomness and cannot read live investigation pressure.
pub fn preview_enterprise_establishment(
    registry: &Registry,
    state: &AppState,
    draft: &EnterpriseDraft,
) -> Result<EnterpriseOutlook, EnterpriseOutlookError> {
    validate_establish_enterprise(registry, state, draft.clone())?;
    resolve_configuration_outlook(
        registry,
        state,
        OutlookConfiguration {
            organization: draft.organization,
            kind: draft.kind,
            location: draft.location,
            support_count: draft.supporting_businesses.len(),
            manager: draft.authority.manager,
            cash: draft.cash_account,
            associates: draft.kind.baseline_associates(),
            additional_active_racket: true,
        },
    )
}

struct OutlookConfiguration {
    organization: OrganizationId,
    kind: EnterpriseKind,
    location: EnterpriseLocation,
    support_count: usize,
    manager: CharacterId,
    cash: FinancialAccountId,
    associates: u16,
    additional_active_racket: bool,
}

fn resolve_configuration_outlook(
    registry: &Registry,
    state: &AppState,
    configuration: OutlookConfiguration,
) -> Result<EnterpriseOutlook, EnterpriseOutlookError> {
    let manager = state.world().get_character(configuration.manager).ok_or(
        EnterpriseOutlookError::MissingManager(configuration.manager),
    )?;
    let account = state.finance().get_account(configuration.cash).ok_or(
        EnterpriseOutlookError::MissingCashAccount(configuration.cash),
    )?;
    let district = resolve_location_neighborhood(state, configuration.location)?;
    let observed = resolve_observed_district_pressure(registry, state)
        .map_err(|error| EnterpriseOutlookError::Projection(error.to_string()))?;
    let observation = observed.get(&(configuration.organization, district));
    let project = |case_count| {
        resolve_enterprise_financial_projection(
            registry,
            state,
            configuration.kind,
            configuration.location,
            configuration.support_count,
            case_count,
            EnterpriseProjectionStaffing {
                management: manager.capability(CapabilityKind::Management),
                associates: configuration.associates,
            },
        )
        .map_err(|error| EnterpriseOutlookError::Projection(error.to_string()))
    };
    let (baseline_cost, baseline_net_cash) = project(0)?;
    let (estimated_operating_cost, estimated_net_cash) = match observation {
        Some(observation) if observation.case_count != 0 => project(observation.case_count)?,
        _ => (baseline_cost, baseline_net_cash),
    };
    let heat_surcharge_per_case = registry
        .get_enterprise(configuration.kind)
        .economics()
        .heat_surcharge_per_active_case();
    let nonpositive_at_cases =
        nonpositive_case_threshold(baseline_net_cash, heat_surcharge_per_case);
    let per_source = registry
        .get_enterprise(configuration.kind)
        .economics()
        .enforcement_attention_basis_points_per_exposure_source();
    let active_rackets = state
        .enterprises()
        .active_for_organization(configuration.organization)
        .count()
        .saturating_add(usize::from(configuration.additional_active_racket));
    Ok(EnterpriseOutlook {
        baseline_net_cash,
        heat_surcharge_per_case,
        nonpositive_at_cases,
        estimated_operating_cost,
        estimated_net_cash,
        observed_pressure: observation.map(|value| (value.observed_at, value.case_count)),
        inquiry_risk_basis_points_per_source: per_source,
        inquiry_risk_no_cases_basis_points: vice_inquiry_chance_basis_points(
            state,
            configuration.organization,
            active_rackets,
            0,
            per_source,
        ),
        shared_cash_balance: account.balance(),
        shared_spendable_cash: account.spendable_balance(),
    })
}

fn nonpositive_case_threshold(net: Money, surcharge: Money) -> Option<u64> {
    let net = net.cents();
    let surcharge = surcharge.cents();
    if net <= 0 {
        Some(0)
    } else if surcharge > 0 {
        Some((net as u64).div_ceil(surcharge as u64))
    } else {
        None
    }
}

pub fn resolve_organization_enterprise_financial_summary(
    state: &AppState,
    organization: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    validate_window(state, period_start, period_end)?;
    if state.world().get_organization(organization).is_none() {
        return Err(EnterpriseReportingError::MissingOrganization(organization));
    }
    resolve_summary(
        state,
        organization,
        state
            .enterprises()
            .enterprises_for_organization(organization),
        period_start,
        period_end,
    )
}

fn validate_window(
    state: &AppState,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<(), EnterpriseReportingError> {
    if period_start > period_end {
        return Err(EnterpriseReportingError::InvalidWindow);
    }
    if period_end > state.now() {
        return Err(EnterpriseReportingError::FutureWindow);
    }
    Ok(())
}

fn resolve_summary<'a>(
    state: &AppState,
    organization: OrganizationId,
    enterprises: impl IntoIterator<Item = &'a EnterpriseRecord>,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    let mut totals = EnterpriseFinancialTotals::default();
    let mut by_kind = BTreeMap::new();
    let mut candidate_kinds = BTreeMap::<EnterpriseId, EnterpriseKind>::new();
    for enterprise in enterprises {
        if enterprise.established_at() > period_end
            || enterprise
                .retired_at()
                .is_some_and(|retired_at| retired_at < period_start)
        {
            continue;
        }
        candidate_kinds.insert(enterprise.id(), enterprise.kind());
        increment_enterprise_count(&mut totals)?;
        let kind_totals = by_kind.entry(enterprise.kind()).or_default();
        increment_enterprise_count(kind_totals)?;
    }
    // The organization/time index visits only this organization's reporting window, in
    // chronological order so checked accumulation remains deterministic.
    for cycle in state.enterprises().cycles_for_organization_from_through(
        organization,
        period_start,
        period_end,
    ) {
        let Some(kind) = candidate_kinds.get(&cycle.enterprise()).copied() else {
            continue;
        };
        let kind_totals = by_kind
            .get_mut(&kind)
            .expect("a counted enterprise must have its kind bucket");
        add_cycle(
            &mut totals,
            cycle.gross_revenue(),
            cycle.operating_cost(),
            cycle.net_cash(),
            cycle.attention(),
        )?;
        add_cycle(
            kind_totals,
            cycle.gross_revenue(),
            cycle.operating_cost(),
            cycle.net_cash(),
            cycle.attention(),
        )?;
    }
    Ok(EnterpriseFinancialSummary {
        period_start,
        period_end,
        totals,
        by_kind,
    })
}

fn increment_enterprise_count(
    totals: &mut EnterpriseFinancialTotals,
) -> Result<(), EnterpriseReportingError> {
    totals.enterprise_count = totals
        .enterprise_count
        .checked_add(1)
        .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    Ok(())
}

fn add_cycle(
    totals: &mut EnterpriseFinancialTotals,
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    attention: AttentionClass,
) -> Result<(), EnterpriseReportingError> {
    crate::finance::financial_aggregation::accumulate_cycle(
        &mut totals.cycle_count,
        &mut totals.notable_cycle_count,
        &mut totals.gross_revenue,
        &mut totals.operating_cost,
        &mut totals.net_cash,
        crate::finance::financial_aggregation::OperatingCycleAmounts {
            gross_revenue,
            operating_cost,
            net_cash,
        },
        attention,
    )
    .ok_or(EnterpriseReportingError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::time::SimDuration;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
    use crate::enterprises::enterprise_execution::{
        validate_establish_enterprise, validate_retire_enterprise, validate_suspend_enterprise,
    };
    use crate::enterprises::{EnterpriseDraft, EnterpriseLocation};
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::world::world_system::{insert_character, insert_neighborhood, insert_organization};
    use crate::world::{
        AutonomyLevel, CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };
    use std::collections::BTreeSet;

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    #[test]
    fn pressure_threshold_handles_exact_break_even_and_zero_surcharge() {
        let threshold = |net, surcharge| {
            nonpositive_case_threshold(Money::from_cents(net), Money::from_cents(surcharge))
        };
        assert_eq!(threshold(24_000, 12_000), Some(2));
        assert_eq!(threshold(24_001, 12_000), Some(3));
        assert_eq!(threshold(0, 0), Some(0));
        assert_eq!(threshold(-1, 12_000), Some(0));
        assert_eq!(threshold(1, 0), None);
        assert_eq!(threshold(i64::MAX, 1), Some(i64::MAX as u64));
    }

    #[test]
    fn historical_summary_respects_enterprise_lifecycle_window() {
        let registry = build_registry();
        let mut state = AppState::new(0xE173_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Historical Enterprise Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("organization fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Historical Enterprise Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(50),
                        commercial_activity: rating(50),
                        illicit_demand: rating(50),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(50),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let manager = insert_character(
            &mut state,
            CharacterDraft {
                name: "Historical Enterprise Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("manager fixture should validate");
        let scope = ResponsibilityScope::Neighborhood(neighborhood);
        let mandate = validate_assign_mandate(
            &state,
            MandateDraft {
                organization,
                manager,
                scopes: BTreeSet::from([scope]),
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
            },
        )
        .expect("cash account fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
            },
        )
        .expect("settlement account fixture should validate");
        state.advance_clock(SimDuration::from_minutes(10));
        let enterprise = validate_establish_enterprise(
            &registry,
            &state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization,
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope,
                },
                location: EnterpriseLocation::Neighborhood(neighborhood),
                supporting_businesses: BTreeSet::new(),
                cash_account: cash,
                settlement_account: settlement,
            },
        )
        .expect("enterprise fixture should validate")
        .commit(&mut state)
        .expect("enterprise fixture should commit");

        let summary = resolve_organization_enterprise_financial_summary(
            &state,
            organization,
            SimTime::ZERO,
            SimTime::from_minutes(9),
        )
        .expect("historical organization summary should resolve");
        assert_eq!(summary.totals.enterprise_count, 0);
        assert_eq!(summary.totals.cycle_count, 0);
        assert!(summary.by_kind.is_empty());

        validate_suspend_enterprise(&state, enterprise)
            .expect("enterprise should suspend before retirement")
            .commit(&mut state)
            .expect("enterprise suspension should commit");
        validate_retire_enterprise(&state, enterprise)
            .expect("suspended enterprise should retire")
            .commit(&mut state)
            .expect("enterprise retirement should commit");
        assert_eq!(
            state
                .enterprises()
                .get_enterprise(enterprise)
                .expect("retired enterprise should persist")
                .retired_at(),
            Some(SimTime::from_minutes(10))
        );

        let historical = resolve_organization_enterprise_financial_summary(
            &state,
            organization,
            SimTime::ZERO,
            SimTime::from_minutes(10),
        )
        .expect("period containing the enterprise lifetime should include it");
        assert_eq!(historical.totals.enterprise_count, 1);

        let retirement_minute = resolve_organization_enterprise_financial_summary(
            &state,
            organization,
            SimTime::from_minutes(10),
            SimTime::from_minutes(10),
        )
        .expect("inclusive reporting at the retirement instant should resolve");
        assert_eq!(
            retirement_minute.totals.enterprise_count, 1,
            "an enterprise established and retired at the reporting instant still overlaps that inclusive window"
        );

        state.advance_clock(SimDuration::from_minutes(10));
        let after_retirement = resolve_organization_enterprise_financial_summary(
            &state,
            organization,
            SimTime::from_minutes(11),
            state.now(),
        )
        .expect("post-retirement period should resolve");
        assert_eq!(after_retirement.totals.enterprise_count, 0);
        assert_eq!(after_retirement.totals.cycle_count, 0);
        assert!(after_retirement.by_kind.is_empty());
    }
}
