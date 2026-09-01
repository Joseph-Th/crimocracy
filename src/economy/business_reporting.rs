//! Read-only legitimate business financial aggregation over persisted operating cycle history.

use crate::core::id::{BusinessId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::Money;
use crate::world::{BusinessKind, BusinessOwner};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusinessFinancialTotals {
    pub business_count: u32,
    pub cycle_count: u32,
    pub notable_cycle_count: u32,
    pub gross_revenue: Money,
    pub operating_cost: Money,
    pub net_cash: Money,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusinessFinancialSummary {
    pub period_start: SimTime,
    pub period_end: SimTime,
    pub totals: BusinessFinancialTotals,
    pub by_kind: BTreeMap<BusinessKind, BusinessFinancialTotals>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessReportingError {
    #[error("business reporting window starts after it ends")]
    InvalidWindow,
    #[error("business reporting window ends after current simulation time")]
    FutureWindow,
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business financial aggregation overflowed")]
    ArithmeticOverflow,
}

/// Test-only drill-down; production reporting aggregates at organization scope.
#[cfg(test)]
pub fn resolve_business_financial_summary(
    state: &AppState,
    business: BusinessId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<BusinessFinancialSummary, BusinessReportingError> {
    validate_window(state, period_start, period_end)?;
    let record = state
        .world()
        .get_business(business)
        .ok_or(BusinessReportingError::MissingBusiness(business))?;
    resolve_summary(state, [record], period_start, period_end, None)
}

pub fn resolve_organization_business_financial_summary(
    state: &AppState,
    organization: OrganizationId,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<BusinessFinancialSummary, BusinessReportingError> {
    validate_window(state, period_start, period_end)?;
    if state.world().get_organization(organization).is_none() {
        return Err(BusinessReportingError::MissingOrganization(organization));
    }
    let owner = BusinessOwner::Organization(organization);
    resolve_summary(
        state,
        state
            .world()
            .businesses_ever_owned_by_organization(organization)
            .filter(|business| {
                let Some(economy) = state.economy().get_business_economy(business.id()) else {
                    return false;
                };
                if economy.established_at() > period_end {
                    return false;
                }
                let ownership_start = period_start.max(economy.established_at());
                state.world.has_business_owner_during(
                    business.id(),
                    owner,
                    ownership_start,
                    period_end,
                )
            }),
        period_start,
        period_end,
        Some(owner),
    )
}

fn validate_window(
    state: &AppState,
    period_start: SimTime,
    period_end: SimTime,
) -> Result<(), BusinessReportingError> {
    if period_start > period_end {
        return Err(BusinessReportingError::InvalidWindow);
    }
    if period_end > state.now() {
        return Err(BusinessReportingError::FutureWindow);
    }
    Ok(())
}

fn resolve_summary<'a>(
    state: &AppState,
    businesses: impl IntoIterator<Item = &'a crate::world::BusinessRecord>,
    period_start: SimTime,
    period_end: SimTime,
    cycle_owner: Option<BusinessOwner>,
) -> Result<BusinessFinancialSummary, BusinessReportingError> {
    let mut totals = BusinessFinancialTotals::default();
    let mut by_kind = BTreeMap::new();
    for business in businesses {
        let Some(economy) = state.economy().get_business_economy(business.id()) else {
            continue;
        };
        if economy.established_at() > period_end {
            continue;
        }
        increment_business_count(&mut totals)?;
        let kind_totals = by_kind.entry(business.kind()).or_default();
        increment_business_count(kind_totals)?;
        for cycle in state.economy().cycles_for(business.id()).filter(|cycle| {
            cycle.occurred_at() >= period_start
                && cycle.occurred_at() <= period_end
                && cycle_owner.is_none_or(|owner| cycle.owner() == owner)
        }) {
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
    Ok(BusinessFinancialSummary {
        period_start,
        period_end,
        totals,
        by_kind,
    })
}

fn increment_business_count(
    totals: &mut BusinessFinancialTotals,
) -> Result<(), BusinessReportingError> {
    totals.business_count = totals
        .business_count
        .checked_add(1)
        .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    Ok(())
}

fn add_cycle(
    totals: &mut BusinessFinancialTotals,
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    attention: crate::core::attention::AttentionClass,
) -> Result<(), BusinessReportingError> {
    totals.cycle_count = totals
        .cycle_count
        .checked_add(1)
        .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    if attention == crate::core::attention::AttentionClass::Notable {
        totals.notable_cycle_count = totals
            .notable_cycle_count
            .checked_add(1)
            .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    }
    totals.gross_revenue = totals
        .gross_revenue
        .checked_add(gross_revenue)
        .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    totals.operating_cost = totals
        .operating_cost
        .checked_add(operating_cost)
        .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    totals.net_cash = totals
        .net_cash
        .checked_add(net_cash)
        .ok_or(BusinessReportingError::ArithmeticOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::time::SimDuration;
    use crate::economy::BusinessEconomyDraft;
    use crate::economy::business_economy_system::validate_establish_business_economy;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{AccountKind, FinancialAccountDraft, FinancialOwner};
    use crate::world::world_system::{insert_business, insert_neighborhood, insert_organization};
    use crate::world::{
        BusinessDraft, BusinessFunction, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };
    use std::collections::BTreeSet;

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    #[test]
    fn historical_summary_excludes_business_established_after_window() {
        let registry = build_registry();
        let mut state = AppState::new(0xB051_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Historical Accounting Organization".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("organization fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Historical Business Ward".to_owned(),
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
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Future Ledger Shop".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(organization),
            },
        )
        .expect("business fixture should validate");
        let operating = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .expect("operating account fixture should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .expect("settlement account fixture should validate");
        state.advance_clock(SimDuration::from_minutes(10));
        validate_establish_business_economy(
            &registry,
            &state,
            BusinessEconomyDraft {
                business,
                operating_account: operating,
                settlement_account: settlement,
            },
        )
        .expect("business economy should validate")
        .commit(&mut state)
        .expect("business economy should commit");

        let summary = resolve_organization_business_financial_summary(
            &state,
            organization,
            SimTime::ZERO,
            SimTime::from_minutes(9),
        )
        .expect("historical summary should resolve");
        assert_eq!(summary.totals.business_count, 0);
        assert_eq!(summary.totals.cycle_count, 0);
        assert!(summary.by_kind.is_empty());
    }
}
