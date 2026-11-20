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
