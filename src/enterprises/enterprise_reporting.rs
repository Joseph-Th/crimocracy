//! Read-only financial aggregation over enterprise cycle history; ledger-backed cycle records remain the source of truth.

use crate::core::attention::AttentionClass;
use crate::core::id::{BusinessId, CharacterId, EnterpriseId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::enterprises::{EnterpriseKind, EnterpriseLocation, EnterpriseRecord};
use crate::finance::Money;
use std::collections::{BTreeMap, BTreeSet};
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
    #[error("enterprise {0} does not exist")]
    MissingEnterprise(EnterpriseId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("manager {0} does not exist")]
    MissingManager(CharacterId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("enterprise financial aggregation overflowed")]
    ArithmeticOverflow,
}

pub fn resolve_enterprise_financial_summary(
    state: &AppState,
    enterprise: EnterpriseId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    validate_window(state, period_start, period_end)?;
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .ok_or(EnterpriseReportingError::MissingEnterprise(enterprise))?;
    resolve_summary(state, [record], period_start, period_end)
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
        state
            .enterprises()
            .enterprises_for_organization(organization),
        period_start,
        period_end,
    )
}

pub fn resolve_manager_enterprise_financial_summary(
    state: &AppState,
    manager: CharacterId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    validate_window(state, period_start, period_end)?;
    if state.world().get_character(manager).is_none() {
        return Err(EnterpriseReportingError::MissingManager(manager));
    }
    resolve_summary(
        state,
        state.enterprises().enterprises_for_manager(manager),
        period_start,
        period_end,
    )
}

pub fn resolve_location_enterprise_financial_summary(
    state: &AppState,
    location: EnterpriseLocation,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    validate_window(state, period_start, period_end)?;
    match location {
        EnterpriseLocation::Neighborhood(id) => {
            if state.world().get_neighborhood(id).is_none() {
                return Err(EnterpriseReportingError::MissingNeighborhood(id));
            }
        }
        EnterpriseLocation::Business(id) => {
            let business = state
                .world()
                .get_business(id)
                .ok_or(EnterpriseReportingError::MissingBusiness(id))?;
            if state
                .world()
                .get_neighborhood(business.neighborhood())
                .is_none()
            {
                return Err(EnterpriseReportingError::MissingNeighborhood(
                    business.neighborhood(),
                ));
            }
        }
    }
    resolve_summary(
        state,
        state.enterprises().enterprises_at(location),
        period_start,
        period_end,
    )
}

pub fn resolve_neighborhood_enterprise_financial_summary(
    state: &AppState,
    neighborhood: NeighborhoodId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    validate_window(state, period_start, period_end)?;
    if state.world().get_neighborhood(neighborhood).is_none() {
        return Err(EnterpriseReportingError::MissingNeighborhood(neighborhood));
    }
    let mut enterprise_ids: BTreeSet<EnterpriseId> = state
        .enterprises()
        .enterprises_at(EnterpriseLocation::Neighborhood(neighborhood))
        .map(EnterpriseRecord::id)
        .collect();
    for business in state.world().businesses_in_neighborhood(neighborhood) {
        enterprise_ids.extend(
            state
                .enterprises()
                .enterprises_at(EnterpriseLocation::Business(business.id()))
                .map(EnterpriseRecord::id),
        );
    }
    let enterprises = enterprise_ids
        .into_iter()
        .filter_map(|id| state.enterprises().get_enterprise(id));
    resolve_summary(state, enterprises, period_start, period_end)
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
    enterprises: impl IntoIterator<Item = &'a EnterpriseRecord>,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<EnterpriseFinancialSummary, EnterpriseReportingError> {
    let mut totals = EnterpriseFinancialTotals::default();
    let mut by_kind = BTreeMap::new();
    for enterprise in enterprises {
        if enterprise.established_at() > period_end {
            continue;
        }
        increment_enterprise_count(&mut totals)?;
        let kind_totals = by_kind.entry(enterprise.kind()).or_default();
        increment_enterprise_count(kind_totals)?;
        for cycle in state
            .enterprises()
            .cycles_for(enterprise.id())
            .filter(|cycle| {
                cycle.occurred_at() >= period_start && cycle.occurred_at() <= period_end
            })
        {
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
    totals.cycle_count = totals
        .cycle_count
        .checked_add(1)
        .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    if attention == AttentionClass::Notable {
        totals.notable_cycle_count = totals
            .notable_cycle_count
            .checked_add(1)
            .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    }
    totals.gross_revenue = totals
        .gross_revenue
        .checked_add(gross_revenue)
        .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    totals.operating_cost = totals
        .operating_cost
        .checked_add(operating_cost)
        .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    totals.net_cash = totals
        .net_cash
        .checked_add(net_cash)
        .ok_or(EnterpriseReportingError::ArithmeticOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::time::SimDuration;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
    use crate::enterprises::enterprise_execution::validate_establish_enterprise;
    use crate::enterprises::EnterpriseDraft;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::world::world_system::{insert_character, insert_neighborhood, insert_organization};
    use crate::world::{
        AutonomyLevel, CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    #[test]
    fn historical_summary_excludes_enterprise_established_after_window() {
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
                        political_influence: rating(50),
                        social_cohesion: rating(50),
                        visible_violence_tolerance: rating(50),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let manager = insert_character(
            &registry,
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
            &registry,
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
                label: "Enterprise cash".to_owned(),
            },
        )
        .expect("cash account fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::Settlement,
                label: "Enterprise settlement".to_owned(),
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

        let summary = resolve_enterprise_financial_summary(
            &state,
            enterprise,
            SimTime::ZERO,
            SimTime::from_minutes(9),
        )
        .expect("historical summary should resolve");
        assert_eq!(summary.totals.enterprise_count, 0);
        assert_eq!(summary.totals.cycle_count, 0);
        assert!(summary.by_kind.is_empty());
    }
}
