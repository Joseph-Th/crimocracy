//! Canonical rank-and-file enterprise staffing adjustments.

use super::*;

use crate::core::time::is_day_boundary;
use crate::world::{AutonomyLevel, CapabilityKind};

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
    /// Active subordinate command branches reporting directly to this person.
    pub command_lines: u32,
    /// Recurring supervision duties for those branches; less autonomy means
    /// more handholding by the immediate supervisor.
    pub reporting_duties: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct ManagedJob {
    pub(crate) kind: EnterpriseKind,
    pub(crate) location: EnterpriseLocation,
    pub(crate) associates: u16,
    pub(crate) supporting_businesses: usize,
}

/// A prospective racket in one same-minute autonomous cohort. The candidate
/// belongs to a real existing captain and must contribute both their own
/// duties and their supervisors' reporting-line obligations before any plan
/// commits. This is not persisted, so the live enterprise remains the truth.
#[derive(Clone, Copy)]
pub(crate) struct ProjectedCommandJob {
    pub(crate) organization: OrganizationId,
    pub(crate) captain: crate::core::id::CharacterId,
    pub(crate) kind: EnterpriseKind,
    pub(crate) location: EnterpriseLocation,
    pub(crate) associates: u16,
    pub(crate) supporting_businesses: usize,
}

/// Find the *immediate* reporting line through which a descendant's operating
/// crew is answerable to this supervisor. This lets an underboss absorb a
/// dozen captains without charging every soldier to the organization leader.
fn commanding_branch(
    state: &AppState,
    supervisor: crate::core::id::CharacterId,
    descendant: crate::core::id::CharacterId,
) -> Option<crate::core::id::CharacterId> {
    let mut child = descendant;
    loop {
        let parent = state.world().get_character(child)?.supervisor()?;
        if parent == supervisor {
            return Some(child);
        }
        child = parent;
    }
}

fn resolve_management_workload(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    proposal: Option<(crate::core::id::CharacterId, ManagedJob)>,
    excluding: Option<EnterpriseId>,
    projected: &[ProjectedCommandJob],
) -> Result<ManagerWorkload, EnterpriseError> {
    let person = state
        .world()
        .get_character(manager)
        .ok_or(EnterpriseError::InvalidManager(manager))?;
    let mut required = 0_u32;
    let mut largest_baseline_job = 0_u32;
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
        // Baseline rank-and-file staffing is the work that necessarily comes
        // with running this *kind* of racket. Extra associates increase the
        // required work, but cannot also increase the manager's capacity.
        // Otherwise an unskilled captain can make a job arbitrarily large and
        // always qualify for it by construction.
        let baseline = u32::from(job.kind.baseline_associates())
            .saturating_add(1)
            .saturating_add(job.supporting_businesses.min(u32::MAX as usize) as u32);
        largest_baseline_job = largest_baseline_job.max(baseline);
        rackets = rackets.saturating_add(1);
        associates = associates.saturating_add(workers);
        Ok(())
    };
    let active_jobs = state
        .enterprises()
        .active_for_organization(organization)
        .filter(|record| Some(record.id()) != excluding)
        .map(|record| {
            (
                record.manager(),
                ManagedJob {
                    kind: record.kind(),
                    location: record.location(),
                    associates: record.associates(),
                    supporting_businesses: record.supporting_businesses().len(),
                },
            )
        });
    let projected_jobs = projected
        .iter()
        .filter(|candidate| candidate.organization == organization)
        .map(|candidate| {
            (
                candidate.captain,
                ManagedJob {
                    kind: candidate.kind,
                    location: candidate.location,
                    associates: candidate.associates,
                    supporting_businesses: candidate.supporting_businesses,
                },
            )
        });
    // Count direct work and operating reporting branches in one pass over
    // current and proposed jobs. Idle hires create no command obligation.
    // Tight or guided branches require more handholding than delegated ones.
    let mut reporting_branches = BTreeSet::new();
    for (captain, job) in active_jobs.chain(proposal).chain(projected_jobs) {
        if captain == manager {
            visit(job)?;
        } else if let Some(branch) = commanding_branch(state, manager, captain) {
            reporting_branches.insert(branch);
        }
    }
    let command_lines = reporting_branches.len().min(u32::MAX as usize) as u32;
    let reporting_duties = reporting_branches.iter().fold(0_u32, |sum, &branch| {
        let autonomy = state
            .world()
            .get_character(branch)
            .expect("reporting branch is an actual subordinate")
            .autonomy();
        sum.saturating_add(match autonomy {
            AutonomyLevel::Tight | AutonomyLevel::Guided => 2,
            AutonomyLevel::Delegated | AutonomyLevel::Broad => 1,
        })
    });
    required = required.saturating_add(reporting_duties);
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
        skill
            .div_ceil(3)
            .max(largest_baseline_job)
            .max(u32::from(command_lines > 0))
    };
    Ok(ManagerWorkload {
        required,
        available,
        rackets,
        associates,
        districts: distinct_districts,
        command_lines,
        reporting_duties,
    })
}

/// Current real workload for an assigned captain, without manufacturing
/// organization-wide capacity from unassigned recruits.
pub fn manager_workload(
    state: &AppState,
    manager: crate::core::id::CharacterId,
) -> Option<ManagerWorkload> {
    let organization = state.world().get_character(manager)?.organization()?;
    resolve_management_workload(state, organization, manager, None, None, &[]).ok()
}

/// Verify a complete same-minute expansion cohort against every real manager
/// and superior affected by the proposed assignments. Checking each mandate
/// independently misses competition for their common executive's bandwidth;
/// a batch must not reach commit with individually valid but jointly
/// impossible reporting lines. The projection reuses the *same* workload
/// resolver as direct player staffing and the daily maintenance phase.
pub(crate) fn projected_command_jobs_fit(state: &AppState, jobs: &[ProjectedCommandJob]) -> bool {
    ensure_projected_command_jobs_fit(state, jobs).is_ok()
}

pub(super) fn ensure_projected_command_jobs_fit(
    state: &AppState,
    jobs: &[ProjectedCommandJob],
) -> Result<(), EnterpriseError> {
    let mut impacted = BTreeSet::new();
    for job in jobs {
        let mut person = Some(job.captain);
        while let Some(manager) = person {
            impacted.insert((job.organization, manager));
            person = state
                .world()
                .get_character(manager)
                .and_then(|record| record.supervisor());
        }
    }
    for (organization, manager) in impacted {
        let load = resolve_management_workload(state, organization, manager, None, None, jobs)?;
        ensure_workload_fits(manager, load)?;
    }
    Ok(())
}

fn ensure_workload_fits(
    manager: crate::core::id::CharacterId,
    load: ManagerWorkload,
) -> Result<(), EnterpriseError> {
    if load.required > load.available {
        return Err(EnterpriseError::ManagerWorkloadExceeded {
            manager,
            required: load.required,
            available: load.available,
        });
    }
    Ok(())
}

pub(crate) fn ensure_manager_capacity(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    job: ManagedJob,
    excluding: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    let workload = resolve_management_workload(
        state,
        organization,
        manager,
        Some((manager, job)),
        excluding,
        &[],
    )?;
    ensure_workload_fits(manager, workload)?;
    // A newly operating branch also consumes capacity up the actual command
    // chain. Strong captains no longer permit an unskilled boss to personally
    // supervise an unlimited number of independent crews.
    let mut child = manager;
    while let Some(supervisor) = state
        .world()
        .get_character(child)
        .expect("validated reporting hierarchy references current members")
        .supervisor()
    {
        let upstream = resolve_management_workload(
            state,
            organization,
            supervisor,
            Some((manager, job)),
            excluding,
            &[],
        )?;
        ensure_workload_fits(supervisor, upstream)?;
        child = supervisor;
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
    let captains = state
        .enterprises()
        .active_enterprises()
        .map(|record| record.manager())
        .collect::<BTreeSet<_>>();
    // Include the whole organizational reporting chain. Otherwise a captain
    // can satisfy their local workload while an executive simultaneously
    // claims infinitely many such captains as direct reports.
    let mut managers = captains.clone();
    for captain in captains {
        let mut person = captain;
        while let Some(supervisor) = state
            .world()
            .get_character(person)
            .expect("a live racket captain belongs to a valid reporting tree")
            .supervisor()
        {
            managers.insert(supervisor);
            person = supervisor;
        }
    }
    // Reconcile the deepest operating branches before their superiors. Closing
    // a child branch may remove the superior's overload without sacrificing a
    // healthy sibling. Character IDs alone do not encode reporting order.
    let mut managers: Vec<_> = managers.into_iter().collect();
    managers.sort_by_cached_key(|&manager| {
        let mut depth = 0;
        let mut cursor = manager;
        while let Some(supervisor) = state
            .world()
            .get_character(cursor)
            .expect("a manager belongs to a valid reporting tree")
            .supervisor()
        {
            depth += 1;
            cursor = supervisor;
        }
        (std::cmp::Reverse(depth), manager)
    });
    let mut suspended = Vec::new();
    for manager in managers {
        while let Some(load) = manager_workload(state, manager) {
            if load.required <= load.available {
                break;
            }
            let Some(racket) = select_overstretched_racket(state, manager, load) else {
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

/// Prefer reducing the captain's own work if their own sites alone exceed
/// capacity. Otherwise deactivate the smallest direct operating branch:
/// this removes the fewest rackets to free a superior's reporting capacity.
/// The newest establishment wins ties, preserving the oldest working crews.
fn select_overstretched_racket(
    state: &AppState,
    manager: crate::core::id::CharacterId,
    workload: ManagerWorkload,
) -> Option<EnterpriseId> {
    let organization = state.world().get_character(manager)?.organization()?;
    let mut own = None;
    let mut branches = BTreeMap::<crate::core::id::CharacterId, Vec<EnterpriseId>>::new();
    for racket in state.enterprises().active_for_organization(organization) {
        if racket.manager() == manager {
            own = own.max(Some(racket.id()));
        } else if let Some(branch) = commanding_branch(state, manager, racket.manager()) {
            branches.entry(branch).or_default().push(racket.id());
        }
    }
    if workload.required.saturating_sub(workload.reporting_duties) > workload.available
        && own.is_some()
    {
        return own;
    }
    let smallest_branch = branches.values().min_by_key(|rackets| {
        (
            rackets.len(),
            std::cmp::Reverse(rackets.iter().copied().max()),
        )
    })?;
    smallest_branch.iter().copied().max().or(own)
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
        // Releasing workers reduces the captain's own duties and leaves every
        // superior's operating reporting lines unchanged. It must remain possible
        // while repairing an already overloaded command chain.
        if associates > record.associates() {
            ensure_manager_capacity(
                state,
                record.organization(),
                record.manager(),
                ManagedJob {
                    kind: record.kind(),
                    location: record.location(),
                    associates,
                    supporting_businesses: record.supporting_businesses().len(),
                },
                Some(enterprise),
            )?;
        }
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
