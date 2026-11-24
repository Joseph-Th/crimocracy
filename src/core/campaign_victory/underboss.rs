//! Read-only institutional, executive and operating-capital evidence for city command.
//! The victory owner derives these from live relationships, the reporting tree and
//! canonical enterprise cost projections; no parallel prestige counter is persisted.

use super::{
    UNDERBOSS_CONTACT_DAYS, UNDERBOSS_OPERATING_RUNWAY_DAYS, UNDERBOSS_SENIOR_MANAGEMENT_RATING,
    VictoryError, established_for,
};
use crate::contacts::contact_system::{
    are_channel_endpoints_available, has_current_contact_relationship_basis,
    resolve_contact_kind_for_institution_kind,
};
use crate::contacts::{ContactKind, ContactStatus};
use crate::core::id::{CharacterId, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::DAY_MINUTES;
use crate::enterprises::EnterpriseRecord;
use crate::enterprises::enterprise_execution::{
    EnterpriseProjectionStaffing, manager_workload, resolve_enterprise_financial_projection,
};
use crate::registry::Registry;
use crate::world::{AutonomyLevel, CapabilityKind};
use std::collections::{BTreeMap, BTreeSet};

/// Longstanding, currently usable civic relationships, not purchased reputation.
/// Police contacts are optional; an organization need not cultivate law enforcement.
pub(super) fn institutional_reach(state: &AppState, owner: OrganizationId) -> (usize, usize) {
    let mut kinds = [false; 4];
    let mut handlers = BTreeSet::new();
    for contact in state.contacts().contacts_for_sponsor(owner) {
        let kind = match contact.kind() {
            ContactKind::Legal => 0,
            ContactKind::Political => 1,
            ContactKind::Labor => 2,
            ContactKind::Professional => 3,
            ContactKind::Police => continue,
        };
        if contact.status() != ContactStatus::Active
            || !established_for(
                state.now(),
                contact.established_at(),
                UNDERBOSS_CONTACT_DAYS,
            )
            || !are_channel_endpoints_available(state, contact)
            || !has_current_contact_relationship_basis(state, contact)
            || !state
                .world()
                .get_character(contact.handler())
                .is_some_and(|person| person.organization() == Some(owner))
            || !state
                .world()
                .get_character(contact.contact())
                .is_some_and(|person| person.organization() == Some(contact.institution()))
            || !state
                .world()
                .get_organization(contact.institution())
                .is_some_and(|institution| {
                    resolve_contact_kind_for_institution_kind(institution.kind())
                        == Some(contact.kind())
                })
        {
            continue;
        }
        kinds[kind] = true;
        handlers.insert(contact.handler());
    }
    (
        kinds.into_iter().filter(|present| *present).count(),
        handlers.len(),
    )
}

/// A genuine middle layer must direct proven captains in more than one district.
/// Neither an idle appointment nor the boss personally supervising every captain qualifies.
pub(super) fn senior_command_count(
    state: &AppState,
    owner: OrganizationId,
    eligible_districts: &BTreeSet<NeighborhoodId>,
    captains: &BTreeMap<NeighborhoodId, BTreeSet<CharacterId>>,
) -> usize {
    let mut reports =
        BTreeMap::<CharacterId, (BTreeSet<CharacterId>, BTreeSet<NeighborhoodId>)>::new();
    for district in eligible_districts {
        for captain in captains.get(district).into_iter().flatten() {
            if let Some(supervisor) = state
                .world()
                .get_character(*captain)
                .and_then(|person| person.supervisor())
            {
                let (people, districts) = reports.entry(supervisor).or_default();
                people.insert(*captain);
                districts.insert(*district);
            }
        }
    }
    reports
        .into_iter()
        .filter(|(supervisor, (people, districts))| {
            people.len() >= 2
                && districts.len() >= 2
                && state
                    .world()
                    .get_character(*supervisor)
                    .is_some_and(|person| {
                        person.organization() == Some(owner)
                            && person.supervisor().is_some()
                            && matches!(
                                person.autonomy(),
                                AutonomyLevel::Delegated | AutonomyLevel::Broad
                            )
                            && person
                                .capability(CapabilityKind::Management)
                                .is_some_and(|rating| {
                                    rating.value() >= UNDERBOSS_SENIOR_MANAGEMENT_RATING
                                })
                    })
                && manager_workload(state, *supervisor)
                    .is_some_and(|load| load.command_lines >= 2 && load.required <= load.available)
        })
        .count()
}

/// Reserve for the actual active network's current staff, venues and support costs.
/// Ask the canonical projection for maximum modeled heat, not private police truth.
/// Round up the number of cycles needed to cover the runway (including partial days).
pub(super) fn operating_reserve(
    registry: &Registry,
    state: &AppState,
    rackets: &[&EnterpriseRecord],
) -> Result<i128, VictoryError> {
    let runway_minutes = UNDERBOSS_OPERATING_RUNWAY_DAYS * DAY_MINUTES;
    let mut total = 0_i128;
    for racket in rackets {
        let manager = state
            .world()
            .get_character(racket.manager())
            .ok_or(VictoryError::Arithmetic)?;
        let (cost, _) = resolve_enterprise_financial_projection(
            registry,
            state,
            racket.kind(),
            racket.location(),
            racket.supporting_businesses().len(),
            u32::MAX,
            EnterpriseProjectionStaffing {
                management: manager.capability(CapabilityKind::Management),
                associates: racket.associates(),
            },
        )
        .map_err(|_| VictoryError::Arithmetic)?;
        let cycle_minutes = u64::from(
            registry
                .get_enterprise(racket.kind())
                .economics()
                .cycle()
                .as_minutes(),
        );
        let cycles = runway_minutes.div_ceil(cycle_minutes);
        total = total
            .checked_add(i128::from(cost.cents()) * i128::from(cycles))
            .ok_or(VictoryError::Arithmetic)?;
    }
    Ok(total)
}
