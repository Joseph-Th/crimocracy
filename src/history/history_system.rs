//! Campaign-history validation and insertion; sibling history state owns the record map.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    CharacterId, EnterpriseId, HistoryEventId, IdExhaustionError, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimTime, ensure_time_current};
use crate::finance::Money;
use crate::history::{HistoryEventDraft, HistoryEventKind, HistoryEventRecord};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum HistoryError {
    #[error("history summary must not be empty")]
    EmptySummary,
    #[error("history event must reference at least one entity")]
    NoEntities,
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("history event was validated at {expected:?}, but simulation time is now {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("payroll departures must be recorded through the payroll departure owner")]
    ReservedPayrollDeparture,
    #[error("campaign victories must be recorded through the campaign victory owner")]
    ReservedCampaignVictory,
    #[error("territorial standstills must be recorded through the territory owner")]
    ReservedTerritoryStandstill,
    #[error("territorial clashes must be recorded through the territory owner")]
    ReservedTerritoryClash,
    #[error("payroll departure does not agree with member {0}'s current membership transition")]
    InvalidPayrollDeparture(CharacterId),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub struct ValidatedHistoryEvent {
    draft: HistoryEventDraft,
    occurred_at: SimTime,
}
impl ValidatedHistoryEvent {
    pub fn commit(self, state: &mut AppState) -> Result<HistoryEventId, HistoryError> {
        ensure_time_current(state.now(), self.occurred_at)
            .map_err(|(expected, found)| HistoryError::StaleTime { expected, found })?;
        if let HistoryEventKind::PayrollDeparture {
            member,
            resulting_version,
            ..
        } = self.draft.kind
            && state.world().get_character(member).is_none_or(|record| {
                record.organization().is_some()
                    || record.supervisor().is_some()
                    || record.version() != resulting_version
            })
        {
            return Err(HistoryError::InvalidPayrollDeparture(member));
        }
        let id = state.ids.next_history_event()?;
        state.history.insert(HistoryEventRecord {
            id,
            occurred_at: self.occurred_at,
            kind: self.draft.kind,
            summary: self.draft.summary,
            entities: self.draft.entities,
        });
        Ok(id)
    }
}

pub fn validate_record_event(
    state: &AppState,
    draft: HistoryEventDraft,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    match draft.kind {
        HistoryEventKind::PayrollDeparture { .. } => {
            return Err(HistoryError::ReservedPayrollDeparture);
        }
        HistoryEventKind::CampaignVictory { .. } => {
            return Err(HistoryError::ReservedCampaignVictory);
        }
        HistoryEventKind::TerritoryStandstill { .. } => {
            return Err(HistoryError::ReservedTerritoryStandstill);
        }
        HistoryEventKind::TerritoryClash { .. } => {
            return Err(HistoryError::ReservedTerritoryClash);
        }
        HistoryEventKind::Operation | HistoryEventKind::Recruitment => {}
    }
    validate_event(state, draft)
}

pub(crate) fn validate_campaign_victory_event(
    state: &AppState,
    organization: OrganizationId,
    path: crate::core::campaign_victory::VictoryPath,
    settlement: Option<crate::core::id::LedgerTransactionId>,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    validate_event(
        state,
        HistoryEventDraft {
            kind: HistoryEventKind::CampaignVictory {
                organization,
                path,
                settlement,
            },
            summary: format!("Campaign victory: {}", path.label()),
            entities: BTreeSet::from([EntityRef::Organization(organization)]),
        },
    )
}

pub(crate) fn validate_territory_standstill_event(
    state: &AppState,
    district: NeighborhoodId,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    tribute: Money,
    settlement: Option<crate::core::id::LedgerTransactionId>,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    let name = state
        .world()
        .get_neighborhood(district)
        .ok_or(HistoryError::MissingEntity(EntityRef::Neighborhood(
            district,
        )))?
        .name();
    validate_event(
        state,
        HistoryEventDraft {
            kind: HistoryEventKind::TerritoryStandstill {
                district,
                initiator,
                counterparty,
                tribute,
                settlement,
            },
            summary: territory_standstill_summary(name, initiator, counterparty, tribute),
            entities: BTreeSet::from([
                EntityRef::Neighborhood(district),
                EntityRef::Organization(initiator),
                EntityRef::Organization(counterparty),
            ]),
        },
    )
}

pub(crate) fn territory_standstill_summary(
    name: &str,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    tribute: Money,
) -> String {
    let terms = if tribute == Money::ZERO {
        "No tribute.".to_owned()
    } else {
        format!(
            "{initiator} paid {} cents in street-cash tribute to {counterparty}.",
            tribute.cents()
        )
    };
    format!(
        "Thirty-day territorial standstill in {name} between {initiator} and {counterparty}. {terms}"
    )
}

pub(crate) fn validate_territory_clash_event(
    state: &AppState,
    district: NeighborhoodId,
    attacker: OrganizationId,
    defender: OrganizationId,
    assault: EnterpriseId,
    target: EnterpriseId,
    loser: EnterpriseId,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    validate_event(
        state,
        HistoryEventDraft {
            kind: HistoryEventKind::TerritoryClash {
                district,
                attacker,
                defender,
                assault,
                target,
                loser,
            },
            summary: format!(
                "Territorial clash in {district}: {attacker} challenged {defender}; {loser} lost the confrontation."
            ),
            entities: BTreeSet::from([
                EntityRef::Neighborhood(district),
                EntityRef::Organization(attacker),
                EntityRef::Organization(defender),
                EntityRef::Enterprise(assault),
                EntityRef::Enterprise(target),
            ]),
        },
    )
}

pub(crate) fn validate_payroll_departure_event(
    state: &AppState,
    member: CharacterId,
    organization: OrganizationId,
    daily_wage: Money,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    let person = state
        .world()
        .get_character(member)
        .ok_or(HistoryError::InvalidPayrollDeparture(member))?;
    if person.organization() != Some(organization)
        || !crate::core::time::is_day_boundary(state.now())
        || daily_wage <= Money::ZERO
    {
        return Err(HistoryError::InvalidPayrollDeparture(member));
    }
    let resulting_version = person
        .version()
        .checked_add(1)
        .ok_or(HistoryError::InvalidPayrollDeparture(member))?;
    validate_event(
        state,
        HistoryEventDraft {
            kind: HistoryEventKind::PayrollDeparture {
                member,
                organization,
                resulting_version,
                daily_wage,
            },
            summary: format!("{} left after wage nonpayment.", person.name()),
            entities: BTreeSet::from([
                EntityRef::Character(member),
                EntityRef::Organization(organization),
            ]),
        },
    )
}

fn validate_event(
    state: &AppState,
    draft: HistoryEventDraft,
) -> Result<ValidatedHistoryEvent, HistoryError> {
    if draft.summary.trim().is_empty() {
        return Err(HistoryError::EmptySummary);
    }
    if draft.entities.is_empty() {
        return Err(HistoryError::NoEntities);
    }
    for entity in &draft.entities {
        if !is_entity_present(state, *entity) {
            return Err(HistoryError::MissingEntity(*entity));
        }
    }
    Ok(ValidatedHistoryEvent {
        draft,
        occurred_at: state.now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::time::SimDuration;
    use crate::history::{HistoryEventDraft, HistoryEventKind};
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};
    use std::collections::BTreeSet;

    fn draft(organization: crate::core::id::OrganizationId, summary: &str) -> HistoryEventDraft {
        HistoryEventDraft {
            kind: HistoryEventKind::Recruitment,
            summary: summary.to_owned(),
            entities: BTreeSet::from([EntityRef::Organization(organization)]),
        }
    }

    #[test]
    fn validated_history_event_rejects_clock_drift_before_allocating_or_inserting() {
        let registry = build_registry();
        let mut state = AppState::new(0x4859_5354);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "History Freshness Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("history fixture organization should validate");

        state.advance_clock(SimDuration::from_minutes(10));
        let stale = validate_record_event(&state, draft(organization, "Prepared history event"))
            .expect("history event should validate at the current instant");

        state.advance_clock(SimDuration::ONE_MINUTE);
        let fresh = validate_record_event(&state, draft(organization, "Fresh history event"))
            .expect("fresh history event should validate")
            .commit(&mut state)
            .expect("fresh history event should commit");
        assert_eq!(
            state
                .history()
                .get_event(fresh)
                .expect("fresh history event should persist")
                .occurred_at(),
            state.now()
        );

        let error = stale
            .commit(&mut state)
            .expect_err("history token held across a clock advance must stale");
        assert_eq!(
            error,
            HistoryError::StaleTime {
                expected: crate::core::time::SimTime::from_minutes(10),
                found: crate::core::time::SimTime::from_minutes(11),
            }
        );
        assert_eq!(
            state.history().events().count(),
            1,
            "stale history commit must not allocate or insert a record"
        );
        crate::core::invariants::validate_invariants(&state);
    }
}
