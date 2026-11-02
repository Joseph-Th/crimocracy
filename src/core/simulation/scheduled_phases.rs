//! Due institutional work and financial cycle settlement.

use super::{draw_basis_point_variance, draw_index, draw_signed_variance};
use crate::core::id::{BusinessCycleId, EnterpriseCycleId, IdKind, InvestigationWorkId};
use crate::core::state::AppState;
use crate::economy::business_economy_system::{
    decide_business_cycle, find_due_businesses, validate_business_cycle_plan,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseCycleRandomness, EnterpriseError, decide_enterprise_cycle, find_due_enterprises,
    validate_enterprise_cycle_plan,
};
use crate::legal::investigation_work_execution::{
    InvestigationWorkError, InvestigationWorkRandomness, decide_investigation_work_resolution,
    find_due_scheduled_investigation_work, validate_investigation_work_resolution_plan,
};
use crate::registry::Registry;

/// Resolves due scheduled detective work with pre-drawn variance. Runs after operation
/// consequences so legal state created by an operation is visible to later institutional work
/// in the same minute without bypassing evidence ownership.
pub(super) fn run_investigation_work_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let due_work = find_due_scheduled_investigation_work(state);
    let mut advanced_rng = state.investigation_rng_mut().clone();
    let mut planned = Vec::with_capacity(due_work.len());
    let mut id_budget = Vec::new();
    for work in due_work {
        let kind = state
            .legal()
            .get_investigation_work(work)
            .expect("due investigation work must still exist")
            .kind();
        let variance_limit = registry.get_investigation_work(kind).variance_limit();
        let mut candidate_rng = advanced_rng.clone();
        let variance = draw_signed_variance(&mut candidate_rng, variance_limit);
        let plan = match decide_investigation_work_resolution(
            registry,
            state,
            work,
            InvestigationWorkRandomness::new(variance),
        ) {
            Ok(plan) => plan,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let validated = match validate_investigation_work_resolution_plan(registry, state, plan) {
            Ok(validated) => validated,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let work_budget = validated.id_budget();
        // An individually impossible forensic or interview artifact must not hold unrelated
        // cases hostage. Keep its work Scheduled and discard its speculative RNG draw; the
        // full viable cohort's global budget still has to fit before anything can commit.
        if state.ids.reserve_many(&work_budget).is_err() {
            continue;
        }
        advanced_rng = candidate_rng;
        id_budget.extend(work_budget);
        planned.push(validated);
    }

    // Due work belongs to distinct active case/lead pairs. Freeze each viable draw and validate
    // every viable resolution against the same pre-pass snapshot, then reserve the complete budget
    // before publishing either RNG progress or the first case mutation. This prevents a later
    // successful review/interview from leaving an earlier prefix resolved when global evidence or
    // statement IDs are nearly exhausted.
    if state.ids.reserve_many(&id_budget).is_err() {
        return Ok(Vec::new());
    }

    let mut resolved = Vec::with_capacity(planned.len());
    for validated in planned {
        resolved.push(
            validated
                .commit(state)
                .expect("prevalidated distinct investigation work must remain current"),
        );
    }
    // Publish the frozen draw sequence only after every due resolution has committed. The work
    // items are disjoint, but keeping RNG publication last preserves the transaction boundary
    // even if a future resolution effect introduces a new fallible dependency.
    *state.investigation_rng_mut() = advanced_rng;
    Ok(resolved)
}

fn investigation_work_is_terminally_blocked(error: &InvestigationWorkError) -> bool {
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::witness_system::WitnessError;

    match error {
        InvestigationWorkError::IdExhaustion(_) | InvestigationWorkError::VersionCapacity(_) => {
            true
        }
        InvestigationWorkError::InterviewStatementFailed { error, .. } => matches!(
            error,
            WitnessError::IdExhaustion(_)
                | WitnessError::VersionCapacity(_)
                | WitnessError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
        ),
        InvestigationWorkError::MissingInvestigation(_)
        | InvestigationWorkError::InactiveInvestigation(_)
        | InvestigationWorkError::MissingInvestigator(_)
        | InvestigationWorkError::InvestigatorNotAssigned { .. }
        | InvestigationWorkError::DetainedInvestigator { .. }
        | InvestigationWorkError::MissingInvestigationCapability(_)
        | InvestigationWorkError::InvalidFocus
        | InvestigationWorkError::WitnessAlreadyStatemented { .. }
        | InvestigationWorkError::WitnessIsCaseSubject { .. }
        | InvestigationWorkError::WitnessInterviewLimitReached { .. }
        | InvestigationWorkError::EvidenceAlreadyReviewed { .. }
        | InvestigationWorkError::EvidenceReviewAlreadyAttempted { .. }
        | InvestigationWorkError::DuplicateScheduledWork { .. }
        | InvestigationWorkError::InvestigatorBusy { .. }
        | InvestigationWorkError::StaleInvestigation { .. }
        | InvestigationWorkError::StaleInvestigator { .. }
        | InvestigationWorkError::MissingWork(_)
        | InvestigationWorkError::WorkNotScheduled(_)
        | InvestigationWorkError::WorkNotDue { .. }
        | InvestigationWorkError::StaleWork { .. }
        | InvestigationWorkError::StaleResolutionContext { .. }
        | InvestigationWorkError::StaleResolutionTime { .. }
        | InvestigationWorkError::StaleScheduleTime { .. }
        | InvestigationWorkError::VarianceOutOfRange { .. }
        | InvestigationWorkError::SimulationTimeOverflow
        | InvestigationWorkError::InvalidSourceEvidence(_)
        | InvestigationWorkError::WitnessInterviewAttemptCapacity { .. } => false,
    }
}

/// Settles due business operating cycles with pre-drawn gross variance.
pub(super) fn run_business_cycle_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<BusinessCycleId>, crate::economy::business_economy_system::BusinessEconomyError> {
    let due_businesses = find_due_businesses(state);
    let mut advanced_rng = state.business_rng_mut().clone();
    let mut planned = Vec::with_capacity(due_businesses.len());
    let mut id_budget = Vec::new();
    for business in due_businesses {
        let kind = state
            .world()
            .get_business(business)
            .expect("due business economy must reference an existing business")
            .kind();
        let variance_limit = registry
            .get_business(kind)
            .economics()
            .gross_variance_basis_points();
        // An individually blocked business, including one whose till has reached finite money
        // headroom, must not consume variance or freeze independent businesses. Draw
        // speculatively and publish progress only after its settlement validates.
        let mut candidate_rng = advanced_rng.clone();
        let variance = draw_basis_point_variance(&mut candidate_rng, variance_limit);
        let plan = match decide_business_cycle(registry, state, business, variance) {
            Ok(plan) => plan,
            Err(error) if business_cycle_is_locally_blocked(&error) => continue,
            Err(error) => return Err(error),
        };
        let validated = match validate_business_cycle_plan(state, plan) {
            Ok(validated) => validated,
            Err(error) if business_cycle_is_locally_blocked(&error) => continue,
            Err(error) => return Err(error),
        };
        let cycle_budget = validated.id_budget();
        // A business that can never allocate its own minimum artifact footprint is inert at
        // this persistence rail. Skip it instead of using its impossible allocation to hold
        // every independent storefront hostage. This read-only check does not consume IDs:
        // the aggregate check below still rejects a viable cohort that cannot *all* fit.
        if state.ids.reserve_many(&cycle_budget).is_err() {
            continue;
        }
        advanced_rng = candidate_rng;
        id_budget.extend(cycle_budget);
        planned.push(validated);
    }

    // Business economies have business-owned operating/settlement accounts, with settlement
    // accounts unique by owner index. Independently blocked businesses are omitted before this
    // point; the viable cohort cannot stale one another's financial snapshots. Freeze each
    // successful draw and reserve the complete shared persistent-ID budget before publishing
    // RNG progress or the first settlement. Global allocator exhaustion still cannot settle
    // an arbitrary prefix merely because its businesses have lower stable IDs.
    if state.ids.reserve_many(&id_budget).is_err() {
        return Ok(Vec::new());
    }
    *state.business_rng_mut() = advanced_rng;

    let mut business_cycles = Vec::with_capacity(planned.len());
    for validated in planned {
        business_cycles.push(validated.commit_preflighted(state));
    }
    Ok(business_cycles)
}

fn business_cycle_is_locally_blocked(
    error: &crate::economy::business_economy_system::BusinessEconomyError,
) -> bool {
    use crate::economy::business_economy_system::BusinessEconomyError;
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;

    match error {
        BusinessEconomyError::IdExhaustion(_) | BusinessEconomyError::VersionCapacity(_) => true,
        BusinessEconomyError::Finance(
            FinanceError::IdExhaustion(_)
            | FinanceError::VersionCapacity(_)
            | FinanceError::BalanceOverflow(_),
        ) => true,
        BusinessEconomyError::Intelligence(IntelligenceError::IdExhaustion(_)) => true,
        BusinessEconomyError::Enterprise(error) => enterprise_cycle_is_locally_blocked(error),
        BusinessEconomyError::MissingBusiness(_)
        | BusinessEconomyError::MissingBusinessEconomy(_)
        | BusinessEconomyError::MissingBusinessNeighborhood(_)
        | BusinessEconomyError::ExistingBusinessEconomy(_)
        | BusinessEconomyError::MissingAccount(_)
        | BusinessEconomyError::AccountOwnerMismatch { .. }
        | BusinessEconomyError::InvalidOperatingAccountKind(_)
        | BusinessEconomyError::InvalidSettlementAccountKind(_)
        | BusinessEconomyError::SettlementAccountInUse { .. }
        | BusinessEconomyError::EconomyNotActive(_)
        | BusinessEconomyError::EconomyNotSuspended(_)
        | BusinessEconomyError::ActiveEnterpriseDependency { .. }
        | BusinessEconomyError::StaleEnterpriseDependency { .. }
        | BusinessEconomyError::CycleNotDue { .. }
        | BusinessEconomyError::VarianceOutOfRange { .. }
        | BusinessEconomyError::ArithmeticOverflow(_)
        | BusinessEconomyError::SimulationTimeOverflow
        | BusinessEconomyError::StaleEconomy { .. }
        | BusinessEconomyError::StaleBusiness { .. }
        | BusinessEconomyError::StaleOperatingAccount { .. }
        | BusinessEconomyError::StaleCycleTime { .. }
        | BusinessEconomyError::StaleDisruptionTime { .. }
        | BusinessEconomyError::Finance(_)
        | BusinessEconomyError::Intelligence(_) => false,
    }
}

/// Settles due enterprise cycles. Both draws happen unconditionally per due cycle so the
/// enterprise stream consumes the same number of values whatever the district's case pressure
/// turns out to be.
pub(super) fn run_enterprise_cycle_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<EnterpriseCycleId>, EnterpriseError> {
    let due_enterprises = find_due_enterprises(state);
    // Later racket decisions observe preceding same-minute settlements: shared cash and newly
    // opened inquiries can change the next racket's economics. Only mandatory cycle IDs can be
    // reserved for the whole sequential cohort without freezing those later dependencies.
    // When those IDs are scarce, exclude individually impossible settlements first; otherwise
    // avoid the cost of duplicate projections on every ordinary campaign day.
    let due_enterprises = if state
        .ids
        .reserve(
            IdKind::EnterpriseCycle,
            u32::try_from(due_enterprises.len())
                .expect("persisted due enterprise count must fit the cycle ID space"),
        )
        .is_ok()
    {
        due_enterprises
    } else {
        if state.ids.reserve(IdKind::EnterpriseCycle, 1).is_err() {
            return Ok(Vec::new());
        }
        let eligible =
            filter_individually_settleable_enterprises(registry, state, due_enterprises)?;
        if state
            .ids
            .reserve(
                IdKind::EnterpriseCycle,
                u32::try_from(eligible.len())
                    .expect("eligible enterprise count must fit the cycle ID space"),
            )
            .is_err()
        {
            // No ID-ordered prefix is allowed to consume the last cycle IDs. Every still-due
            // candidate and the persistent RNG remain untouched if the viable cohort cannot fit.
            return Ok(Vec::new());
        }
        eligible
    };
    let mut enterprise_cycles = Vec::with_capacity(due_enterprises.len());
    for enterprise in due_enterprises {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let mut advanced_rng = state.enterprise_rng_mut().clone();
        let randomness = draw_enterprise_cycle_randomness(registry, kind, &mut advanced_rng);
        let cycle = match decide_enterprise_cycle(registry, state, enterprise, randomness)
            .and_then(|plan| validate_enterprise_cycle_plan(state, plan))
            .and_then(|validated| validated.commit(state))
        {
            Ok(cycle) => cycle,
            Err(error) if enterprise_cycle_is_locally_blocked(&error) => {
                // A local finite allocator, version, or account-balance rail is not corruption.
                // The failed cycle published neither RNG progress nor any mutation; leave it
                // due and continue settling independent rackets instead of failing the tick.
                continue;
            }
            Err(error) => return Err(error),
        };
        // Enterprise cycles stay sequential because they may share organization cash and one
        // racket's enforcement incident can change another racket's live legal-pressure context.
        // Still publish each item's two draws only after that item commits.
        *state.enterprise_rng_mut() = advanced_rng;
        enterprise_cycles.push(cycle);
    }
    Ok(enterprise_cycles)
}

/// Exclude rackets that cannot allocate even their own settlement artifacts before reserving
/// mandatory cycle IDs for the cohort. Without this pass a notable racket with exhausted report
/// IDs can indefinitely prevent an unrelated routine racket from settling when fewer cycle IDs
/// remain than the total number due. Preflight uses speculative draws: only an individually
/// settleable candidate advances the preview stream, and the real sequential pass still resolves
/// economics against the state left by preceding committed rackets.
fn filter_individually_settleable_enterprises(
    registry: &Registry,
    state: &AppState,
    due: Vec<crate::core::id::EnterpriseId>,
) -> Result<Vec<crate::core::id::EnterpriseId>, EnterpriseError> {
    let mut preview_rng = state.enterprise_rng().clone();
    let mut eligible = Vec::with_capacity(due.len());
    for enterprise in due {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let mut candidate_rng = preview_rng.clone();
        let randomness = draw_enterprise_cycle_randomness(registry, kind, &mut candidate_rng);
        let candidate = decide_enterprise_cycle(registry, state, enterprise, randomness)
            .and_then(|plan| validate_enterprise_cycle_plan(state, plan));
        match candidate {
            Ok(validated) => {
                let budget = validated.id_budget()?;
                if state.ids.reserve_many(&budget).is_ok() {
                    preview_rng = candidate_rng;
                    eligible.push(enterprise);
                }
            }
            Err(error) if enterprise_cycle_is_locally_blocked(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(eligible)
}

/// One draw contract for both speculative capacity checks and real sequential settlements.
/// The district's current case pressure never affects the number or order of RNG draws.
fn draw_enterprise_cycle_randomness(
    registry: &Registry,
    kind: crate::enterprises::EnterpriseKind,
    rng: &mut impl rand_core::RngCore,
) -> EnterpriseCycleRandomness {
    let variance = draw_basis_point_variance(
        rng,
        registry
            .get_enterprise(kind)
            .economics()
            .gross_variance_basis_points(),
    );
    let enforcement_roll = u16::try_from(
        draw_index(
            rng,
            EnterpriseCycleRandomness::ENFORCEMENT_ATTENTION_ROLL_COUNT,
        )
        .expect("enterprise enforcement roll range is never empty"),
    )
    .expect("enterprise enforcement roll fits u16");
    EnterpriseCycleRandomness::new(variance, enforcement_roll)
}

fn enterprise_cycle_is_locally_blocked(error: &EnterpriseError) -> bool {
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::investigation_system::InvestigationError;
    use crate::reports::report_system::ReportError;

    matches!(
        error,
        EnterpriseError::IdExhaustion(_)
            | EnterpriseError::VersionCapacity(_)
            | EnterpriseError::Finance(
                FinanceError::IdExhaustion(_)
                    | FinanceError::VersionCapacity(_)
                    | FinanceError::BalanceOverflow(_)
            )
            | EnterpriseError::Intelligence(IntelligenceError::IdExhaustion(_))
            | EnterpriseError::Report(ReportError::IdExhaustion(_))
            | EnterpriseError::Investigation(
                InvestigationError::IdExhaustion(_)
                    | InvestigationError::VersionCapacity(_)
                    | InvestigationError::CaseKnowledge(IntelligenceError::IdExhaustion(_))
            )
    )
}
