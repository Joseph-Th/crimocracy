//! Daily NPC maintenance for suspended enterprises.
//!
//! Suspension is operational, not terminal history. Governed rivals therefore reconsider a
//! suspended racket on day boundaries using the same information and runway constraints as new
//! expansion. Temporary pressure leaves it suspended; revoked or re-scoped authority, lost
//! required venue/support ownership, or Tight/Guided manager autonomy at the maintenance boundary cause autonomous
//! rivals to abandon that stale frozen configuration so dead records cannot reserve a location
//! forever.

use crate::core::id::{EnterpriseId, FinancialAccountId, IdKind, MandateId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::DelegationError;
use crate::enterprises::autonomous_planning::{
    AutonomousEnterpriseError, available_working_capital, reserve_working_capital,
    resolve_committed_working_capital, resolve_observed_district_case_count,
    resolve_observed_district_pressure,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, EnterpriseProjectionStaffing, ProjectedCommandJob,
    ValidatedEnterpriseStatusChange, projected_command_jobs_fit,
    resolve_enterprise_financial_projection, validate_resume_enterprise,
    validate_retire_enterprise,
};
use crate::finance::finance_system::{
    StreetCapitalizationDraft, ValidatedLedgerTransaction, validate_capitalize_street_cash,
};
use crate::finance::{AccountKind, FinancialOwner, Money};
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
    command_job: ProjectedCommandJob,
    resume: ValidatedEnterpriseStatusChange,
}

struct PlannedResumption {
    enterprise: EnterpriseId,
    mandate: MandateId,
    resume: ValidatedEnterpriseStatusChange,
    capitalization: Option<ValidatedLedgerTransaction>,
}

struct AutonomousEnterpriseLifecyclePlan {
    retirements: Vec<(EnterpriseId, ValidatedEnterpriseStatusChange)>,
    resumptions: Vec<PlannedResumption>,
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
    let suspended: Vec<_> = state
        .enterprises()
        .suspended_enterprises()
        .filter(|enterprise| Some(enterprise.organization()) != player_organization)
        // A loss-threshold suspension on this exact daily boundary must remain suspended
        // for at least one full governance interval. Remove it before constructing economic
        // projections; those facts cannot make it eligible for action today.
        .filter(|enterprise| enterprise.last_cycle_at() != Some(state.now()))
        .map(|enterprise| enterprise.id())
        .collect();
    // Reconstructing observed district pressure walks recent settled cycles. On most days
    // there is no suspended rival racket to maintain, so do not build pressure or working-
    // capital projections that no candidate can consume.
    if suspended.is_empty() {
        return Ok(AutonomousEnterpriseLifecycleOutcome::default());
    }
    let observed_district_pressure = resolve_observed_district_pressure(registry, state)?;
    let mut reservations =
        resolve_committed_working_capital(registry, state, &observed_district_pressure)?;

    let mut retirements = Vec::new();
    let mut candidates = Vec::new();
    for enterprise_id in suspended {
        let enterprise = state
            .enterprises()
            .get_enterprise(enterprise_id)
            .expect("suspended enterprise collected from authoritative state must persist");
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
            | Err(EnterpriseError::ManagerWorkloadExceeded { .. })
            | Err(EnterpriseError::TerritorySuppressed { .. })
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
                observed_active_cases,
                EnterpriseProjectionStaffing {
                    management: manager_record.capability(CapabilityKind::Management),
                    associates: enterprise.associates(),
                },
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
            command_job: ProjectedCommandJob {
                organization,
                captain: manager,
                kind: enterprise.kind(),
                location: enterprise.location(),
                associates: enterprise.associates(),
                supporting_businesses: enterprise.supporting_businesses().len(),
            },
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
    let mut capitalized_organizations = BTreeSet::new();
    let mut projected_jobs = Vec::new();
    let mut resumptions = Vec::new();
    for candidate in candidates {
        if resumed_mandates.contains(&candidate.mandate) {
            continue;
        }
        let mut prospective_jobs = projected_jobs.clone();
        prospective_jobs.push(candidate.command_job);
        // Distinct mandates can share a superior. Just as in new expansion, the entire
        // selected cohort must fit the actual command chain before any funding is moved.
        if !projected_command_jobs_fit(state, &prospective_jobs) {
            continue;
        }
        let organization = candidate.command_job.organization;
        let funded = state
            .finance()
            .get_account(candidate.cash_account)
            .is_some_and(|account| {
                available_working_capital(account, &reservations)
                    >= candidate.required_working_capital
            });
        let capitalization = if funded {
            None
        } else {
            // Each transfer belongs to a distinct organization and consumes one ledger ID.
            // Plan it without writing: its proceeds are fully reserved by this resumption,
            // and no later candidate may plan a second transfer for the same owner.
            let count = u32::try_from(capitalized_organizations.len() + 1)
                .expect("capitalization cohort must fit the organization ID space");
            if capitalized_organizations.contains(&organization)
                || state
                    .ids
                    .reserve_many(&[(IdKind::LedgerTransaction, count)])
                    .is_err()
            {
                continue;
            }
            let Some(capitalization) = plan_suspended_racket_capitalization(
                registry,
                state,
                organization,
                candidate.cash_account,
                candidate.required_working_capital,
                &reservations,
            )?
            else {
                continue;
            };
            capitalized_organizations.insert(organization);
            Some(capitalization)
        };
        reserve_working_capital(
            &mut reservations,
            candidate.cash_account,
            candidate.required_working_capital,
        );
        projected_jobs = prospective_jobs;
        resumed_mandates.insert(candidate.mandate);
        resumptions.push(PlannedResumption {
            enterprise: candidate.enterprise,
            mandate: candidate.mandate,
            resume: candidate.resume,
            capitalization,
        });
    }

    Ok(AutonomousEnterpriseLifecyclePlan {
        retirements,
        resumptions,
    }
    .commit(state))
}

/// A delegated rival can use earned legitimate dividends to restore a viable street
/// operation. It must clear the old deficit in full, fund one cycle of projected costs,
/// and retain two days of clean payroll. No credit or invented cash is created.
fn plan_suspended_racket_capitalization(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    cash_account: FinancialAccountId,
    required: Money,
    reservations: &std::collections::BTreeMap<FinancialAccountId, Money>,
) -> Result<Option<ValidatedLedgerTransaction>, AutonomousEnterpriseError> {
    let Some(destination) = state.finance().get_account(cash_account) else {
        return Ok(None);
    };
    // A capitalization must leave at least one posting version for the resumed cycle.
    if destination.kind() != AccountKind::StreetCash || destination.version() >= u32::MAX - 1 {
        return Ok(None);
    }
    let reserved_cents = reservations
        .get(&cash_account)
        .map_or(0, |value| value.cents());
    let target = i128::from(required.cents()) + i128::from(reserved_cents);
    let needed = target - i128::from(destination.balance().cents());
    if target > i128::from(i64::MAX) || needed <= 0 || needed > i128::from(i64::MAX) {
        return Ok(None);
    }
    let needed = Money::from_cents(needed as i64);
    let payroll_reserve = i128::from(registry.upkeep().per_member_daily().cents())
        * state
            .world()
            .characters_in_organization(organization)
            .count() as i128
        * 2;
    let source = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .filter(|account| {
            i128::from(account.available_payment_balance().cents())
                >= i128::from(needed.cents()) + payroll_reserve
        })
        .max_by_key(|account| {
            (
                account.available_payment_balance(),
                std::cmp::Reverse(account.id()),
            )
        });
    let Some(source) = source else {
        return Ok(None);
    };
    Ok(Some(validate_capitalize_street_cash(
        state,
        StreetCapitalizationDraft {
            organization,
            accounted_account: source.id(),
            street_account: cash_account,
            amount: needed,
        },
    )?))
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
        for planned in self.resumptions {
            if let Some(capitalization) = planned.capitalization {
                capitalization.commit(state).expect(
                    "preplanned independent capitalization must remain current within one pass",
                );
            }
            planned
                .resume
                .commit(state)
                .expect("preplanned autonomous resumption must remain current within one pass");
            outcome.resumed.push(planned.enterprise);
            outcome.resumed_mandates.insert(planned.mandate);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city_generation::{CityScale, generate_city};
    use crate::core::invariants::validate_invariants;
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::simulation::run_test_tick;
    use crate::enterprises::enterprise_execution::validate_suspend_enterprise;
    use crate::finance::finance_system::{insert_account, validate_record_transaction};
    use crate::finance::{FinancialAccountDraft, LedgerPosting, LedgerTransactionDraft};

    #[test]
    fn indebted_rival_reinvests_real_clean_dividends_to_reopen_racket() {
        let registry = crate::build_registry();
        let mut city = generate_city(&registry, CityScale::Small.config(82_734_113)).unwrap();
        let organization = city.rivals[0];
        let racket = city
            .state
            .enterprises()
            .active_for_organization(organization)
            .next()
            .unwrap()
            .id();
        let cash = city
            .state
            .enterprises()
            .get_enterprise(racket)
            .unwrap()
            .cash_account();
        let opening_balance = city
            .state
            .finance()
            .get_account(cash)
            .unwrap()
            .balance()
            .cents();
        let accounted = insert_account(
            &mut city.state,
            FinancialAccountDraft {
                owner: FinancialOwner::Organization(organization),
                kind: AccountKind::AccountedFunds,
            },
        )
        .unwrap();
        let settlement = city
            .state
            .finance()
            .accounts_for(FinancialOwner::Organization(organization))
            .find(|account| account.kind() == AccountKind::Settlement)
            .unwrap()
            .id();
        for (memo, from, to, amount) in [
            (
                "Existing earned legitimate reserve",
                settlement,
                accounted,
                200_000,
            ),
            (
                "Old enterprise operating loss",
                cash,
                settlement,
                opening_balance + 50_000,
            ),
        ] {
            validate_record_transaction(
                &city.state,
                LedgerTransactionDraft {
                    occurred_at: city.state.now(),
                    memo: memo.into(),
                    postings: vec![
                        LedgerPosting {
                            account: from,
                            amount: Money::from_cents(-amount),
                        },
                        LedgerPosting {
                            account: to,
                            amount: Money::from_cents(amount),
                        },
                    ],
                    authorization: None,
                },
            )
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        }
        validate_suspend_enterprise(&city.state, racket)
            .unwrap()
            .commit(&mut city.state)
            .unwrap();
        assert_eq!(
            city.state
                .finance()
                .get_account(cash)
                .unwrap()
                .balance()
                .cents(),
            -50_000
        );
        let clean_before = city
            .state
            .finance()
            .get_account(accounted)
            .unwrap()
            .balance();
        // A financially viable reopening still needs a representable ledger posting.
        // An exhausted ledger must not spend clean funds, resume the racket, or fail
        // the entire daily maintenance phase.
        let mut exhausted = city.state.clone();
        exhausted.advance_clock(crate::core::time::SimDuration::from_minutes(1_440));
        exhausted
            .ids
            .set_next_raw_for_test(IdKind::LedgerTransaction, u32::MAX);
        let before = bincode::serialize(&exhausted).unwrap();
        let outcome = apply_due_autonomous_enterprise_lifecycle(&registry, &mut exhausted).unwrap();
        assert_eq!(outcome, AutonomousEnterpriseLifecycleOutcome::default());
        assert_eq!(bincode::serialize(&exhausted).unwrap(), before);

        let mut replay =
            restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
        for _ in 0..1_440 {
            run_test_tick(&registry, &mut city.state);
            run_test_tick(&registry, &mut replay);
        }
        let reopened = city.state.enterprises().get_enterprise(racket).unwrap();
        assert_eq!(
            reopened.status(),
            crate::enterprises::EnterpriseStatus::Active
        );
        assert!(city.state.finance().get_account(cash).unwrap().balance() > Money::ZERO);
        assert!(
            city.state
                .finance()
                .get_account(accounted)
                .unwrap()
                .balance()
                < clean_before
        );
        assert!(city.state.finance().transactions().any(|transaction| {
            transaction.memo().contains("Treasury capitalization")
                && transaction
                    .postings()
                    .iter()
                    .any(|posting| posting.account == cash)
        }));
        assert_eq!(
            bincode::serialize(&city.state).unwrap(),
            bincode::serialize(&replay).unwrap()
        );
        validate_invariants(&city.state);
        restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    }
}
