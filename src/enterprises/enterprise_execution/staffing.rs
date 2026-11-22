//! Canonical rank-and-file enterprise staffing adjustments.

use super::*;
use crate::core::time::is_day_boundary;

/// A named captain can direct only so many independently operated rackets and
/// rank-and-file associates at once. This is an organizational constraint,
/// not a secret NPC modifier. It applies equally to autonomous and player moves.
pub const MAX_RACKETS_PER_MANAGER: u16 = 6;
pub const MAX_ASSOCIATES_PER_MANAGER: u16 = 24;
/// Existing small crews can run the first local set of rackets with their
/// baseline workers. Expansion beyond that requires paid named leadership,
/// even when all new rackets are delegated to a single prolific captain.
pub const BASE_ORGANIZATION_RACKET_CAPACITY: usize = 6;
pub const RACKETS_PER_AVAILABLE_MEMBER: usize = 2;
pub const BASE_ORGANIZATION_ASSOCIATE_CAPACITY: u32 = 24;
pub const ASSOCIATES_PER_AVAILABLE_MEMBER: u32 = 10;

pub fn organization_operating_capacity(
    state: &AppState,
    organization: OrganizationId,
) -> (usize, u32) {
    let available = state
        .world()
        .characters_in_organization(organization)
        .filter(|member| {
            state
                .legal()
                .active_arrest_for_character(member.id())
                .is_none()
        })
        .count();
    (
        BASE_ORGANIZATION_RACKET_CAPACITY
            .max(available.saturating_mul(RACKETS_PER_AVAILABLE_MEMBER)),
        BASE_ORGANIZATION_ASSOCIATE_CAPACITY
            .max((available as u32).saturating_mul(ASSOCIATES_PER_AVAILABLE_MEMBER)),
    )
}

pub(crate) fn organization_racket_overextension(
    state: &AppState,
    organization: OrganizationId,
) -> Vec<EnterpriseId> {
    let (limit_rackets, limit_associates) = organization_operating_capacity(state, organization);
    let active = state
        .enterprises()
        .active_for_organization(organization)
        .collect::<Vec<_>>();
    let mut count = active.len();
    let mut associates = active
        .iter()
        .map(|racket| u32::from(racket.associates()))
        .sum::<u32>();
    let mut suspend = Vec::new();
    for racket in active.into_iter().rev() {
        if count <= limit_rackets && associates <= limit_associates {
            break;
        }
        count -= 1;
        associates -= u32::from(racket.associates());
        suspend.push(racket.id());
    }
    suspend
}

/// Departures and arrests can leave an existing empire larger than its live
/// command structure. At each day boundary, suspend newest rackets until the
/// remaining ones fit the actual available workforce. Established operations
/// retain their identities and can resume through normal recovery.
pub(crate) fn apply_due_understaffing_suspensions(state: &mut AppState) -> Vec<EnterpriseId> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let organizations = state
        .world()
        .organizations()
        .filter(|organization| organization.kind() == OrganizationKind::Criminal)
        .map(|organization| organization.id())
        .collect::<Vec<_>>();
    let mut suspended = Vec::new();
    for organization in organizations {
        for enterprise in organization_racket_overextension(state, organization) {
            if let Ok(token) = validate_suspend_enterprise(state, enterprise)
                && token.commit(state).is_ok()
            {
                suspended.push(enterprise);
            }
        }
    }
    suspended
}

pub(crate) fn ensure_manager_capacity(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    additional_associates: u16,
    excluding: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    let mut rackets = 1_u16;
    let mut associates = u32::from(additional_associates);
    for active in state
        .enterprises()
        .active_for_organization(organization)
        .filter(|record| record.manager() == manager && Some(record.id()) != excluding)
    {
        rackets = rackets.saturating_add(1);
        associates += u32::from(active.associates());
    }
    if rackets > MAX_RACKETS_PER_MANAGER || associates > u32::from(MAX_ASSOCIATES_PER_MANAGER) {
        return Err(EnterpriseError::ManagerCapacityExceeded {
            manager,
            rackets,
            associates,
            max_rackets: MAX_RACKETS_PER_MANAGER,
            max_associates: MAX_ASSOCIATES_PER_MANAGER,
        });
    }
    let (max_active, max_associates) = organization_operating_capacity(state, organization);
    let mut total_rackets = 1_usize;
    let mut total_associates = u32::from(additional_associates);
    for active in state
        .enterprises()
        .active_for_organization(organization)
        .filter(|record| Some(record.id()) != excluding)
    {
        total_rackets += 1;
        total_associates += u32::from(active.associates());
    }
    if total_rackets > max_active || total_associates > max_associates {
        return Err(EnterpriseError::OrganizationCapacityExceeded {
            organization,
            rackets: total_rackets,
            associates: total_associates,
            max_rackets: max_active,
            max_associates,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedEnterpriseStaffing {
    enterprise: EnterpriseId,
    expected_version: u32,
    at: SimTime,
    associates: u16,
}

pub fn validate_staff_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
    associates: u16,
) -> Result<ValidatedEnterpriseStaffing, EnterpriseError> {
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() == EnterpriseStatus::Retired {
        return Err(EnterpriseError::EnterpriseRetired(enterprise));
    }
    let minimum = record.kind().baseline_associates();
    let maximum = record.kind().max_associates();
    if !(minimum..=maximum).contains(&associates) {
        return Err(EnterpriseError::InvalidAssociateCount {
            enterprise,
            minimum,
            maximum,
        });
    }
    if record.associates() == associates {
        return Err(EnterpriseError::EnterpriseStaffingUnchanged(enterprise));
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    if record.status() == EnterpriseStatus::Active {
        ensure_version_can_advance(record.version() + 1, "enterprise")?;
        if record.next_cycle_at().is_some_and(|due| due <= state.now()) {
            return Err(EnterpriseError::EnterpriseStaffingCycleDue(enterprise));
        }
        ensure_manager_capacity(
            state,
            record.organization(),
            record.manager(),
            associates,
            Some(enterprise),
        )?;
    }
    Ok(ValidatedEnterpriseStaffing {
        enterprise,
        expected_version: record.version(),
        at: state.now(),
        associates,
    })
}

impl ValidatedEnterpriseStaffing {
    pub fn commit(self, state: &mut AppState) -> Result<(), EnterpriseError> {
        if state.now() != self.at {
            return Err(EnterpriseError::StaleCycleTime {
                expected: self.at,
                found: state.now(),
            });
        }
        let record = state
            .enterprises()
            .get_enterprise(self.enterprise)
            .ok_or(EnterpriseError::MissingEnterprise(self.enterprise))?;
        if record.version() != self.expected_version {
            return Err(EnterpriseError::StaleEnterprise {
                enterprise: self.enterprise,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_staff_enterprise(state, self.enterprise, self.associates)?;
        state
            .enterprises
            .set_associates(self.enterprise, self.associates, self.at);
        Ok(())
    }
}
