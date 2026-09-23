//! Daily NPC maintenance for suspended enterprises.
//!
//! Suspension is operational, not terminal history. Governed rivals therefore reconsider a
//! suspended racket on day boundaries using the same information and runway constraints as new
//! expansion. Temporary pressure leaves it suspended; revoked or re-scoped authority, lost
//! required venue/support ownership, or immutable Tight/Guided manager autonomy cause autonomous
//! rivals to abandon that stale frozen configuration so dead records cannot reserve a location
//! forever.

use crate::core::id::{EnterpriseId, MandateId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, available_working_capital, reserve_working_capital,
    resolve_committed_working_capital, resolve_observed_district_case_count,
    resolve_observed_district_pressure,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, ValidatedEnterpriseStatusChange, resolve_enterprise_financial_projection,
    validate_resume_enterprise, validate_retire_enterprise,
};
use crate::finance::Money;
use crate::registry::Registry;
use crate::world::{AutonomyLevel, CapabilityKind};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutonomousEnterpriseLifecycleOutcome {
    pub(crate) resumed: Vec<EnterpriseId>,
    pub(crate) retired: Vec<EnterpriseId>,
    /// Mandates that spent their one daily enterprise-governance action reopening a racket.
    /// The subsequent expansion pass excludes them so one manager cannot both reopen and expand
    /// on the same boundary.
    pub(crate) resumed_mandates: BTreeSet<MandateId>,
}

struct ResumeCandidate {
    enterprise: EnterpriseId,
    mandate: MandateId,
    cash_account: crate::core::id::FinancialAccountId,
    required_working_capital: Money,
    expected_net_cash: Money,
    resume: ValidatedEnterpriseStatusChange,
}

struct AutonomousEnterpriseLifecyclePlan {
    retirements: Vec<(EnterpriseId, ValidatedEnterpriseStatusChange)>,
    resumptions: Vec<(EnterpriseId, MandateId, ValidatedEnterpriseStatusChange)>,
}

/// Reconsiders suspended non-player rackets on the daily governance boundary.
///
/// A chronic-loss suspension that happened on this same minute is deliberately ineligible:
/// immediately reopening it would erase the practical meaning of the loss threshold. Otherwise,
/// a rival resumes only when its frozen authority/assets are still valid, police-fear posture is
/// below the expansion ceiling, current observed economics are positive, and its existing cash
/// account can cover one operating cycle after all active-racket reservations.
pub(crate) fn apply_due_autonomous_enterprise_lifecycle(
    registry: &Registry,
    state: &mut AppState,
) -> Result<AutonomousEnterpriseLifecycleOutcome, AutonomousEnterpriseError> {
    if !crate::core::time::is_day_boundary(state.now()) {
        return Ok(AutonomousEnterpriseLifecycleOutcome::default());
    }

    let player_organization = state.player_organization();
    let observed_district_pressure = resolve_observed_district_pressure(registry, state)?;
    let mut reservations =
        resolve_committed_working_capital(registry, state, &observed_district_pressure)?;
    let suspended: Vec<_> = state
        .enterprises()
        .suspended_enterprises()
        .filter(|enterprise| Some(enterprise.organization()) != player_organization)
        .map(|enterprise| enterprise.id())
        .collect();

    let mut retirements = Vec::new();
    let mut candidates = Vec::new();
    for enterprise_id in suspended {
        let enterprise = state
            .enterprises()
            .get_enterprise(enterprise_id)
            .expect("suspended enterprise collected from authoritative state must persist");
        // A losing cycle may suspend exactly on this day boundary. Give that suspension at least
        // one governance interval of consequence instead of resetting its streak immediately.
        if enterprise.last_cycle_at() == Some(state.now()) {
            continue;
        }

        let organization = enterprise.organization();
        let resume = match validate_resume_enterprise(registry, state, enterprise_id) {
            Ok(resume) => resume,
            Err(error) if stale_configuration_requires_retirement(&error) => {
                if let Some(retirement) =
                    plan_stale_autonomous_enterprise_retirement(state, enterprise_id)?
                {
                    retirements.push((enterprise_id, retirement));
                }
                continue;
            }
            Err(EnterpriseError::Delegation(DelegationError::DetainedManager { .. }))
            | Err(EnterpriseError::HostBusinessSuspended { .. })
            | Err(EnterpriseError::SupportingBusinessSuspended { .. })
            | Err(EnterpriseError::VersionCapacity(_))
            | Err(EnterpriseError::SimulationTimeOverflow) => continue,
            Err(error) => return Err(error.into()),
        };

        let manager = enterprise.manager();
        let manager_record = state
            .world()
            .get_character(manager)
            .expect("validated enterprise authority must reference a live manager");
        if !matches!(
            manager_record.autonomy(),
            AutonomyLevel::Delegated | AutonomyLevel::Broad
        ) {
            // Autonomy is authored character state, not a temporary operational blocker. An NPC
            // manager without delegated discretion has no autonomous route that can ever reopen
            // this suspended racket, so retaining the frozen record would reserve its slot
            // forever. Abandon it through the same terminal lifecycle used for stale authority.
            if let Some(retirement) =
                plan_stale_autonomous_enterprise_retirement(state, enterprise_id)?
            {
                retirements.push((enterprise_id, retirement));
            }
            continue;
        }
        // Police posture is a temporary operating choice, so evaluate it only after canonical
        // resume validation has had a chance to retire a frozen configuration whose authority
        // or required assets no longer belong to this racket.
        let police_fear = crate::reputation::reputation_system::resolve_score(
            registry,
            state.reputation(),
            organization,
            crate::reputation::AudienceKind::Police,
        );
        if police_fear >= registry.reputation().expansion_police_fear_ceiling() {
            continue;
        }

        let observed_active_cases = resolve_observed_district_case_count(
            state,
            &observed_district_pressure,
            organization,
            enterprise.location(),
        )?;
        let (required_working_capital, expected_net_cash) =
            resolve_enterprise_financial_projection(
                registry,
                state,
                enterprise.kind(),
                enterprise.location(),
                enterprise.supporting_businesses().len(),
                manager_record.capability(CapabilityKind::Management),
                observed_active_cases,
            )?;
        if expected_net_cash <= Money::ZERO {
            continue;
        }
        candidates.push(ResumeCandidate {
            enterprise: enterprise_id,
            mandate: enterprise.authority().mandate,
            cash_account: enterprise.cash_account(),
            required_working_capital,
            expected_net_cash,
            resume,
        });
    }

    // Scarce shared cash goes to the strongest positive-net suspended racket first. Stable IDs
    // break exact economic ties only.
    candidates.sort_unstable_by(|left, right| {
        right
            .expected_net_cash
            .cmp(&left.expected_net_cash)
            .then(
                left.required_working_capital
                    .cmp(&right.required_working_capital),
            )
            .then(left.enterprise.cmp(&right.enterprise))
    });
    let mut resumed_mandates = BTreeSet::new();
    let mut resumptions = Vec::new();
    for candidate in candidates {
        if resumed_mandates.contains(&candidate.mandate) {
            continue;
        }
        let Some(account) = state.finance().get_account(candidate.cash_account) else {
            continue;
        };
        if available_working_capital(account, &reservations) < candidate.required_working_capital {
            continue;
        }
        // Reserve before authoritative mutation so there is no fallible planning step after a
        // successful resume. The fresh canonical token re-proves authority and asset liveness.
        reserve_working_capital(
            &mut reservations,
            candidate.cash_account,
            candidate.required_working_capital,
        )?;
        resumed_mandates.insert(candidate.mandate);
        resumptions.push((candidate.enterprise, candidate.mandate, candidate.resume));
    }

    Ok(AutonomousEnterpriseLifecyclePlan {
        retirements,
        resumptions,
    }
    .commit(state))
}

impl AutonomousEnterpriseLifecyclePlan {
    fn commit(self, state: &mut AppState) -> AutonomousEnterpriseLifecycleOutcome {
        let mut outcome = AutonomousEnterpriseLifecycleOutcome::default();
        for (enterprise, retirement) in self.retirements {
            retirement
                .commit(state)
                .expect("preplanned autonomous retirement must remain current within one pass");
            outcome.retired.push(enterprise);
        }
        for (enterprise, mandate, resume) in self.resumptions {
            resume
                .commit(state)
                .expect("preplanned autonomous resumption must remain current within one pass");
            outcome.resumed.push(enterprise);
            outcome.resumed_mandates.insert(mandate);
        }
        outcome
    }
}

fn plan_stale_autonomous_enterprise_retirement(
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<Option<ValidatedEnterpriseStatusChange>, AutonomousEnterpriseError> {
    match validate_retire_enterprise(state, enterprise) {
        Ok(retirement) => Ok(Some(retirement)),
        // Version exhaustion is a valid finite terminal rail. The enterprise remains suspended
        // because no further versioned lifecycle transition is representable.
        Err(EnterpriseError::VersionCapacity(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn stale_configuration_requires_retirement(error: &EnterpriseError) -> bool {
    matches!(
        error,
        EnterpriseError::Delegation(
            DelegationError::InactiveMandate(_) | DelegationError::ScopeOutsideMandate { .. }
        ) | EnterpriseError::SupportingBusinessOwnershipMismatch { .. }
            | EnterpriseError::HostBusinessOwnershipMismatch { .. }
    )
}
