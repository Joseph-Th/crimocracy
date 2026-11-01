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
        let variance = draw_signed_variance(&mut advanced_rng, variance_limit);
        let plan = match decide_investigation_work_resolution(
            registry,
            state,
            work,
            InvestigationWorkRandomness::new(variance),
        ) {
            Ok(plan) => plan,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let validated = match validate_investigation_work_resolution_plan(registry, state, plan) {
            Ok(validated) => validated,
            Err(error) if investigation_work_is_terminally_blocked(&error) => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        id_budget.extend(validated.id_budget());
        planned.push(validated);
    }

    // Due work belongs to distinct active case/lead pairs. Freeze every draw and validate every
    // resolution against the same pre-pass snapshot, then reserve the complete artifact budget
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
        let variance = draw_basis_point_variance(&mut advanced_rng, variance_limit);
        let plan = match decide_business_cycle(registry, state, business, variance) {
            Ok(plan) => plan,
            Err(error) if business_cycle_is_terminally_blocked(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let validated = match validate_business_cycle_plan(state, plan) {
            Ok(validated) => validated,
            Err(error) if business_cycle_is_terminally_blocked(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        id_budget.extend(validated.id_budget());
        planned.push(validated);
    }

    // Business economies have business-owned operating/settlement accounts, with settlement
    // accounts unique by owner index. Due cycles therefore cannot stale one another's financial
    // snapshots. Freeze every draw and reserve the complete persistent-ID budget before
    // publishing RNG progress or the first settlement so allocator pressure cannot settle only
    // the lowest business IDs in a same-minute cohort.
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

fn business_cycle_is_terminally_blocked(
    error: &crate::economy::business_economy_system::BusinessEconomyError,
) -> bool {
    use crate::economy::business_economy_system::BusinessEconomyError;
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;

    match error {
        BusinessEconomyError::IdExhaustion(_) | BusinessEconomyError::VersionCapacity(_) => true,
        BusinessEconomyError::Finance(
            FinanceError::IdExhaustion(_) | FinanceError::VersionCapacity(_),
        ) => true,
        BusinessEconomyError::Intelligence(IntelligenceError::IdExhaustion(_)) => true,
        BusinessEconomyError::Enterprise(error) => enterprise_cycle_is_terminally_blocked(error),
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
    // Later racket decisions intentionally observe earlier same-minute settlements: shared cash
    // and newly opened enforcement inquiries can change the next racket's economics. We therefore
    // cannot freeze the complete artifact budget up front without changing simulation semantics.
    // Every due settlement does, however, unconditionally persist exactly one EnterpriseCycle.
    // Prove that mandatory cohort capacity before the first mutation so this predictable finite
    // rail cannot leave a successful prefix merely because stable enterprise order reached it.
    if state
        .ids
        .reserve(
            IdKind::EnterpriseCycle,
            u32::try_from(due_enterprises.len())
                .expect("persisted due enterprise count must fit the cycle ID space"),
        )
        .is_err()
    {
        // ID exhaustion is a valid finite terminal rail. Do not settle an ID-ordered prefix and
        // do not consume RNG for work that cannot be persisted; every due racket simply remains
        // due and inert once no complete cycle-id cohort is representable.
        return Ok(Vec::new());
    }
    let mut enterprise_cycles = Vec::with_capacity(due_enterprises.len());
    for enterprise in due_enterprises {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let economics = registry.get_enterprise(kind).economics();
        let mut advanced_rng = state.enterprise_rng_mut().clone();
        let variance =
            draw_basis_point_variance(&mut advanced_rng, economics.gross_variance_basis_points());
        let enforcement_attention_roll = u16::try_from(
            draw_index(
                &mut advanced_rng,
                crate::enterprises::enterprise_execution::EnterpriseCycleRandomness::ENFORCEMENT_ATTENTION_ROLL_COUNT,
            )
                .expect("racket-attention roll range is never empty"),
        )
        .expect("racket-attention roll fits u16");
        let cycle = match decide_enterprise_cycle(
            registry,
            state,
            enterprise,
            EnterpriseCycleRandomness::new(variance, enforcement_attention_roll),
        )
        .and_then(|plan| validate_enterprise_cycle_plan(state, plan))
        .and_then(|validated| validated.commit(state))
        {
            Ok(cycle) => cycle,
            Err(error) if enterprise_cycle_is_terminally_blocked(&error) => {
                // Capacity exhaustion is not corruption. The failed cycle published neither its
                // cloned RNG state nor any authoritative mutation, so leave it due and continue
                // settling later independent rackets rather than panicking the canonical tick.
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

fn enterprise_cycle_is_terminally_blocked(error: &EnterpriseError) -> bool {
    use crate::finance::finance_system::FinanceError;
    use crate::intelligence::intelligence_system::IntelligenceError;
    use crate::legal::investigation_system::InvestigationError;
    use crate::reports::report_system::ReportError;

    matches!(
        error,
        EnterpriseError::IdExhaustion(_)
            | EnterpriseError::VersionCapacity(_)
            | EnterpriseError::Finance(
                FinanceError::IdExhaustion(_) | FinanceError::VersionCapacity(_)
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
