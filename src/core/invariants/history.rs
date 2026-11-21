//! Release-safe structural validation for persisted campaign history.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::finance::Money;
use crate::history::HistoryEventKind;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_history_against_registry(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for event in state.history.events() {
        if let HistoryEventKind::PayrollDeparture { daily_wage, .. } = event.kind()
            && daily_wage != registry.upkeep().per_member_daily()
        {
            return Err(StateValidationError::InvalidPayrollDeparture { event: event.id() });
        }
    }
    Ok(())
}

pub(super) fn validate_history(state: &AppState) -> Result<(), StateValidationError> {
    let mut previous_departures = BTreeMap::new();
    let mut victory_seen = false;
    for event in state.history.events() {
        if event.summary().trim().is_empty() {
            return Err(StateValidationError::EmptyHistorySummary { event: event.id() });
        }
        if event.entities().is_empty() {
            return Err(StateValidationError::HistoryEventHasNoEntities { event: event.id() });
        }
        if event.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "history event",
            });
        }
        for entity in event.entities() {
            if !is_entity_present(state, *entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "history event",
                    entity: *entity,
                });
            }
        }
        if let HistoryEventKind::CampaignVictory {
            organization,
            path,
            settlement,
        } = event.kind()
        {
            let invalid = || StateValidationError::InvalidCampaignVictory { event: event.id() };
            let report_count = state
                .reports()
                .reports_for(organization)
                .filter(|report| {
                    report.kind() == crate::reports::ReportKind::CampaignVictory
                        && report.generated_at() == event.occurred_at()
                        && report.title() == format!("Campaign victory: {}", path.label())
                })
                .count();
            let settlement_valid = match (path, settlement) {
                (crate::core::campaign_victory::VictoryPath::FundedRetirement, Some(transaction)) => {
                    state.finance().get_transaction(transaction).is_some_and(|record| {
                        record.occurred_at() == event.occurred_at()
                            && record.memo() == "Final campaign retirement settlement: leader nest egg and crew severance"
                            && record.postings().iter().any(|posting| posting.amount < Money::ZERO
                                && state.finance().get_account(posting.account).is_some_and(|account| {
                                    account.owner() == crate::finance::FinancialOwner::Organization(organization)
                                        && account.kind() == crate::finance::AccountKind::AccountedFunds
                                }))
                            && record.postings().iter().any(|posting| posting.amount > Money::ZERO
                                && state.finance().get_account(posting.account).is_some_and(|account| {
                                    matches!(account.owner(), crate::finance::FinancialOwner::Character(_))
                                        && account.kind() == crate::finance::AccountKind::AccountedFunds
                                }))
                    })
                }
                (crate::core::campaign_victory::VictoryPath::FundedRetirement, None) => false,
                (_, None) => true,
                (_, Some(_)) => false,
            };
            if victory_seen
                || state.player_organization() != Some(organization)
                || state
                    .world()
                    .characters_in_organization(organization)
                    .next()
                    .is_none()
                || !settlement_valid
                || report_count != 1
                || event.summary() != format!("Campaign victory: {}", path.label())
                || event.entities() != &BTreeSet::from([EntityRef::Organization(organization)])
            {
                return Err(invalid());
            }
            victory_seen = true;
        }
        if let HistoryEventKind::PayrollDeparture {
            member,
            organization,
            resulting_version,
            daily_wage,
        } = event.kind()
        {
            let invalid = || StateValidationError::InvalidPayrollDeparture { event: event.id() };
            let person = state.world().get_character(member).ok_or_else(invalid)?;
            if !crate::core::time::is_day_boundary(event.occurred_at())
                || daily_wage <= Money::ZERO
                || resulting_version < 2
                || resulting_version > person.version()
                || (resulting_version == person.version() && person.organization().is_some())
                || state
                    .world()
                    .get_organization(organization)
                    .is_none_or(|record| record.kind() != crate::world::OrganizationKind::Criminal)
                || event.entities()
                    != &BTreeSet::from([
                        EntityRef::Character(member),
                        EntityRef::Organization(organization),
                    ])
                || event.summary() != format!("{} left after wage nonpayment.", person.name())
                || previous_departures
                    .insert(member, resulting_version)
                    .is_some_and(|previous| previous >= resulting_version)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
