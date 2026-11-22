//! Canonical rank-and-file enterprise staffing adjustments.

use super::*;

use crate::core::time::is_day_boundary;
use crate::world::CapabilityKind;

/// A unit of responsibility is one active associate, one independently operating
/// racket, one supporting storefront, or one additional district in the captain's
/// portfolio. These are things a captain must actually supervise, not arbitrary
/// dollars of legal business revenue. Management aptitude determines how many
/// simultaneous responsibilities they can take on. Anyone can personally run
/// one operation; a second operation requires demonstrable managerial skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagerWorkload {
    pub required: u32,
    pub available: u32,
    pub rackets: u32,
    pub associates: u32,
    pub districts: u32,
}

#[derive(Clone, Copy)]
struct ManagedJob {
    location: EnterpriseLocation,
    associates: u16,
    supporting_businesses: usize,
}

fn resolve_management_workload(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    proposal: Option<ManagedJob>,
    excluding: Option<EnterpriseId>,
) -> Result<ManagerWorkload, EnterpriseError> {
    let person = state
        .world()
        .get_character(manager)
        .ok_or(EnterpriseError::InvalidManager(manager))?;
    let mut required = 0_u32;
    let mut largest_job = 0_u32;
    let mut rackets = 0_u32;
    let mut associates = 0_u32;
    let mut districts = BTreeSet::new();
    let mut visit = |job: ManagedJob| -> Result<(), EnterpriseError> {
        let district = resolve_location_neighborhood(state, job.location)?;
        districts.insert(district);
        let workers = u32::from(job.associates);
        // A separate site, each actual worker, and each support arrangement
        // needs one recurring coordination task, irrespective of how much money
        // happens to pass through that operation.
        let work = workers
            .saturating_add(1)
            .saturating_add(job.supporting_businesses.min(u32::MAX as usize) as u32);
        required = required.saturating_add(work);
        largest_job = largest_job.max(work);
        rackets = rackets.saturating_add(1);
        associates = associates.saturating_add(workers);
        Ok(())
    };
    for active in state
        .enterprises()
        .active_for_organization(organization)
        .filter(|record| record.manager() == manager && Some(record.id()) != excluding)
    {
        visit(ManagedJob {
            location: active.location(),
            associates: active.associates(),
            supporting_businesses: active.supporting_businesses().len(),
        })?;
    }
    if let Some(job) = proposal {
        visit(job)?;
    }
    let distinct_districts = districts.len().min(u32::MAX as usize) as u32;
    required = required.saturating_add(distinct_districts.saturating_sub(1));
    let skill = person
        .capability(CapabilityKind::Management)
        .map_or(0_u32, |value| u32::from(value.value()));
    // Aptitude is a 0..100 rating, while the workload counts concrete daily
    // responsibilities. The skill-to-task scale is a tuning parameter, not
    // a fixed limit on sites, workers, districts, or the organization's roster.
    let available = if person.organization() != Some(organization)
        || state.legal().active_arrest_for_character(manager).is_some()
    {
        0
    } else {
        skill.div_ceil(3).max(largest_job)
    };
    Ok(ManagerWorkload {
        required,
        available,
        rackets,
        associates,
        districts: distinct_districts,
    })
}

/// Current real workload for an assigned captain, without manufacturing
/// organization-wide capacity from unassigned recruits.
pub fn manager_workload(
    state: &AppState,
    manager: crate::core::id::CharacterId,
) -> Option<ManagerWorkload> {
    let organization = state.world().get_character(manager)?.organization()?;
    resolve_management_workload(state, organization, manager, None, None).ok()
}

pub(crate) fn ensure_manager_capacity(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    location: EnterpriseLocation,
    additional_associates: u16,
    supporting_businesses: usize,
    excluding: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    let workload = resolve_management_workload(
        state,
        organization,
        manager,
        Some(ManagedJob {
            location,
            associates: additional_associates,
            supporting_businesses,
        }),
        excluding,
    )?;
    if workload.required > workload.available {
        return Err(EnterpriseError::ManagerWorkloadExceeded {
            manager,
            required: workload.required,
            available: workload.available,
        });
    }
    Ok(())
}

/// Existing active portfolios can become unmanageable after arrests, departures
/// or a changed command structure. Suspend newest rackets until each captain's
/// surviving portfolio is within their current workload. Recovery/reassignment
/// remains a player choice and uses canonical enterprise lifecycle operations.
pub(crate) fn apply_due_overstretched_manager_suspensions(
    state: &mut AppState,
) -> Vec<EnterpriseId> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let managers = state
        .enterprises()
        .enterprises()
        .filter(|record| record.status() == EnterpriseStatus::Active)
        .map(|record| record.manager())
        .collect::<BTreeSet<_>>();
    let mut suspended = Vec::new();
    for manager in managers {
        while let Some(load) = manager_workload(state, manager) {
            if load.required <= load.available {
                break;
            }
            let Some(racket) = state
                .enterprises()
                .enterprises()
                .filter(|record| {
                    record.status() == EnterpriseStatus::Active && record.manager() == manager
                })
                .map(|record| record.id())
                .max()
            else {
                break;
            };
            let Ok(token) = validate_suspend_enterprise(state, racket) else {
                break;
            };
            if token.commit(state).is_err() {
                break;
            }
            suspended.push(racket);
        }
    }
    suspended
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
            record.location(),
            associates,
            record.supporting_businesses().len(),
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
