//! Deterministic top-level simulation tick and state-owned random decision helpers.
//!
//! `run_tick` is the only authoritative minute (contractual phase order).
//! See `ARCHITECTURE.md` for the authoritative phase diagram and dependency tower.
//! New autonomous work must slot explicitly here with a "runs after X so Y" comment.

use crate::core::id::{
    BusinessCycleId, CharacterId, EnterpriseCycleId, IdKind, InvestigationId, InvestigationWorkId,
    OperationId, OpportunityId, PoliceResponseId, ProsecutionCaseId, RecruitmentAttemptId,
    ReportId,
};
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::decision_system::DecisionRequestOutcome;
use crate::legal::investigation_system::apply_autonomous_investigator_staffing;
use crate::legal::investigation_system::apply_cold_case_decay;
use crate::legal::investigation_work_execution::{
    InvestigationWorkSchedulingOutcome, apply_investigation_work_scheduling,
};
use crate::opportunities::opportunity_system::apply_opportunity_expiry;
use crate::recruitment::autonomous_recruitment::apply_due_autonomous_recruitment;
use crate::registry::Registry;
use crate::reports::executive_brief::{
    decide_executive_brief, is_executive_brief_due, validate_executive_brief_plan,
};
use crate::reputation::reputation_system::OperationReputationEvent;
use rand_core::RngCore;
use thiserror::Error;

mod operations_phase;
mod scheduled_phases;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
    pub arrived_police_responses: Vec<PoliceResponseId>,
    /// Every decision request raised this tick, regardless of owning subsystem. The outcome
    /// preserves whether this player-owned attention event requests an adapter pause.
    pub decision_requests: Vec<DecisionRequestOutcome>,
    /// Operations aborted during the operation phase itself. Custody-driven aborts remain
    /// represented by the arrest that caused them later in the tick.
    pub aborted_operations: Vec<OperationId>,
    pub resolved_operations: Vec<OperationId>,
    pub staffed_investigations: Vec<(InvestigationId, CharacterId)>,
    pub scheduled_investigation_work: Vec<InvestigationWorkId>,
    pub scheduled_witness_interviews: Vec<InvestigationWorkId>,
    pub resolved_investigation_work: Vec<InvestigationWorkId>,
    pub evidence_arrests: Vec<crate::core::id::ArrestId>,
    pub staffed_prosecution_cases: Vec<(ProsecutionCaseId, CharacterId)>,
    pub informant_recruitments: Vec<crate::core::id::InformantId>,
    pub informant_disclosures: Vec<crate::core::id::InformantDisclosureId>,
    pub custody_releases: Vec<crate::core::id::ArrestId>,
    pub automatic_legal_support: Vec<crate::core::id::LegalRepresentationId>,
    /// Automatic-policy retainers ended because their represented matter is no longer active.
    /// The legal-support owner reports this separately from newly retained counsel so a
    /// conclusion-only tick remains visible to adapters and validation without rescanning state.
    pub concluded_automatic_legal_support: usize,
    pub business_cycles: Vec<BusinessCycleId>,
    pub resumed_businesses: Vec<crate::core::id::BusinessId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
    pub payrolls: Vec<crate::world::payroll_execution::PayrollOutcome>,
    /// Number of individual reputation dimensions that actually moved this tick, including
    /// day-boundary decay and current operation/racket consequences. This makes reputation-only
    /// persistent ticks observable to validation/adapters instead of hiding them behind phases
    /// that happen to have no other surfaced outcome.
    pub reputation_changes: usize,
    pub recruitment_attempts: Vec<RecruitmentAttemptId>,
    pub resumed_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub retired_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub autonomous_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub expired_opportunities: Vec<OpportunityId>,
    pub cold_case_suspensions: Vec<InvestigationId>,
    pub executive_brief: Option<ReportId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TickError {
    #[error("simulation clock is exhausted at minute {now:?}; no later canonical tick exists")]
    ClockExhausted { now: SimTime },
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> Result<TickOutcome, TickError> {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    // The state owner performs the checked mutation itself. A terminal campaign state is valid,
    // but it has no successor minute; rejecting here keeps the whole tick atomic and leaves no
    // unchecked production clock mutator for another caller to reuse accidentally.
    let previous_now = state.now();
    state
        .try_advance_clock(SimDuration::ONE_MINUTE)
        .ok_or(TickError::ClockExhausted { now: previous_now })?;
    // The custody cap is a hard lifecycle boundary. Release due detainees before any same-minute
    // work so an expired detention cannot block a participant, remain an extraction target, or
    // otherwise influence systems after its authored end instant. The informant decision delay is
    // required by registry validation to be strictly shorter than maximum detention, so canonical
    // minute-by-minute ticks always give that detainee decision its intended earlier window.
    let custody_releases = crate::legal::arrest_system::apply_due_custody_releases(
        state,
        registry.legal().maximum_detention(),
    )
    .expect("valid state should release custody that reached the authored maximum");
    // Phase order is the contract: bounded custody release first; opportunity expiry next so its
    // durable lifecycle report is available to every remaining same-minute consumer; then
    // operations (police arrivals, overdue cleanup, resolution, starts), legal institutional work
    // (staffing, detective work, new custody, representation, informants, cold decay), economy
    // cycles plus legitimate-business recovery, then the day-boundary governance cluster: payroll,
    // reputation (decay before current consequences), recruitment, delegated expansion (which
    // consumes current police fear), and executive synthesis last so the due brief sees everything
    // above.
    let expired_opportunities = apply_opportunity_expiry(registry, state)
        .expect("valid state should expire every due opportunity atomically");
    let OperationsPhaseOutcome {
        started: started_operations,
        arrived_police_responses,
        mut decision_requests,
        aborted: aborted_operations,
        resolved: resolved_operations,
    } = run_operations_phase(registry, state);
    let staffed_investigations = apply_autonomous_investigator_staffing(state)
        .expect("valid state should staff available investigators onto active cases");
    // Autonomous casework traverses the active-case index once per minute. Reviewable evidence
    // keeps first claim on a free detective; when no review is due, the same pass may schedule a
    // witness interview. Later evidence and witnesses therefore remain actionable without two
    // duplicate active-case scans every canonical minute.
    let InvestigationWorkSchedulingOutcome {
        evidence_reviews: scheduled_investigation_work,
        witness_interviews: scheduled_witness_interviews,
    } = apply_investigation_work_scheduling(registry, state)
        .expect("valid state should schedule due investigation work");
    let resolved_investigation_work = run_investigation_work_phase(registry, state)
        .expect("valid due investigation-work cohort must resolve atomically");
    // The police institution converts accumulated case evidence into custody after detective
    // work resolves, so an interview or forensic analysis finishing this minute is visible to
    // the same minute's arrest decision.
    let evidence_arrests =
        crate::legal::arrest_system::apply_autonomous_evidence_arrests(registry, state)
            .expect("valid state should convert qualifying case evidence into custody");
    // Custody can remove a prosecutor from every review they were carrying. Restaff immediately
    // after arrests so one individual's detention cannot freeze unrelated prosecution matters.
    let staffed_prosecution_cases =
        crate::legal::prosecution_system::apply_autonomous_prosecution_staffing(state)
            .expect("valid state should staff available prosecutors onto open reviews");
    // Legal-support governance runs before the detainee's one-time informant decision. This lets
    // promised counsel materially affect custodial cooperation risk instead of retaining counsel
    // only after the irreversible decision. It sees every new arrest above and any open
    // prosecution that outlived a release, so promised defense can start after custody too;
    // automatic retainers are concluded only after their final matter ends.
    let automatic_legal_support_outcome =
        crate::legal::legal_representation_system::apply_automatic_legal_support(registry, state)
            .expect("valid state should resolve automatic legal-support retention");
    let automatic_legal_support = automatic_legal_support_outcome.retained;
    let concluded_automatic_legal_support = automatic_legal_support_outcome.concluded;
    // A member arrested exactly one cadence window ago now faces the decision with current
    // representation visible. New informants then disclose personally-held knowledge.
    let informant_recruitments =
        crate::legal::informant_system::apply_detainee_informant_recruitment(registry, state)
            .expect("valid state should resolve detainee informant recruitment decisions");
    let informant_disclosures = crate::legal::informant_system::apply_informant_disclosures(state)
        .expect("valid state should record due informant disclosures");
    // Cold-case decay runs after detective work resolution so the case's last-activity instant is
    // final for the minute; an authored institutional-inactivity window then shelves operation-
    // originated cases whose owning authority has gone quiet. No random stream is consumed, so the
    // decay does not perturb any domain RNG sequence.
    let cold_case_decay = apply_cold_case_decay(state, registry.legal().cold_case_window())
        .expect("valid state should resolve cold-case decay");
    let business_cycles = run_business_cycle_phase(registry, state)
        .expect("valid state should settle the complete due business-cycle cohort");
    // Legitimate-business recovery follows today's settlements so a chronic-loss suspension has
    // already taken effect. The lifecycle pass skips a business suspended at this exact instant,
    // and only non-player owners with positive current zero-variance economics resume through the
    // canonical economy lifecycle token.
    let resumed_businesses =
        crate::economy::business_economy_system::apply_due_autonomous_business_lifecycle(
            registry, state,
        )
        .expect("valid state should maintain suspended non-player business economies");
    let enterprise_cycles = run_enterprise_cycle_phase(registry, state)
        .expect("valid due enterprise cycles must settle through the preflighted sequential phase");
    // Payroll runs after the day's enterprise and business cycles so earned revenue can fund
    // the same day's wages. Reputation then settles the day boundary before recruitment: daily
    // decay advances only impressions old enough to fade, while operation/racket consequences from
    // this minute land before candidates judge an outfit's underworld competence. Together with
    // payroll, every authored recruitment input therefore reflects the current minute rather
    // than a mixture of pre- and post-boundary state.
    let payrolls = crate::world::payroll_execution::apply_daily_payroll(registry, state)
        .expect("valid state should settle every due criminal-organization payroll");
    let reputation_changes =
        apply_reputation_phase(registry, state, &resolved_operations, &enterprise_cycles)
            .expect("valid state should apply the preflighted reputation consequence cohort");
    let recruitment = apply_due_autonomous_recruitment(registry, state)
        .expect("valid state should resolve every due autonomous recruitment action");
    let recruitment_attempts = recruitment.attempts;
    decision_requests.extend(recruitment.approval_requests);
    // Suspended rival rackets are reconsidered before new growth. This gives chronic-loss
    // suspension a recoverable lifecycle without allowing an organization to spend one cash pool
    // on both a reopened racket and a fresh establishment. A racket suspended by a cycle earlier
    // in this same minute remains suspended until a later daily boundary.
    let enterprise_lifecycle =
        crate::enterprises::autonomous_lifecycle::apply_due_autonomous_enterprise_lifecycle(
            registry, state,
        )
        .expect("valid state should maintain suspended autonomous enterprises");
    let resumed_enterprises = enterprise_lifecycle.resumed;
    let retired_enterprises = enterprise_lifecycle.retired;
    // Delegated rival expansion runs after lifecycle maintenance and recruitment so a mandate
    // whose crew changed this minute governs with its current roster, and after reputation so a
    // racket hit this minute can make the organization keep its head down immediately. Selection
    // consumes no randomness, so matched branches observe identical rival growth unless their own
    // actions touched rival state.
    let autonomous_enterprises =
        crate::enterprises::autonomous_expansion::apply_due_autonomous_enterprises_excluding(
            registry,
            state,
            &enterprise_lifecycle.resumed_mandates,
        )
        .expect("valid state should resolve every due autonomous enterprise expansion");
    // Executive synthesis runs last so a due brief sees every report and decision created by
    // operational, investigative, financial, and delegated personnel work that resolved in the
    // same simulation minute.
    let executive_brief = synthesize_executive_brief(registry, state);
    validate_invariants(state);
    Ok(TickOutcome {
        now: state.now(),
        started_operations,
        arrived_police_responses,
        decision_requests,
        aborted_operations,
        resolved_operations,
        staffed_investigations,
        scheduled_investigation_work,
        scheduled_witness_interviews,
        resolved_investigation_work,
        evidence_arrests,
        staffed_prosecution_cases,
        informant_recruitments,
        informant_disclosures,
        custody_releases,
        automatic_legal_support,
        concluded_automatic_legal_support,
        business_cycles,
        resumed_businesses,
        enterprise_cycles,
        payrolls,
        reputation_changes,
        recruitment_attempts,
        resumed_enterprises,
        retired_enterprises,
        autonomous_enterprises,
        expired_opportunities,
        cold_case_suspensions: cold_case_decay,
        executive_brief,
    })
}

#[cfg(test)]
pub(crate) fn run_test_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    run_tick(registry, state).expect("test fixture must leave room for another simulation minute")
}

#[cfg(test)]
use operations_phase::{
    AuthorizedPrestartAction, apply_overdue_operation_cleanup, plan_authorized_prestarts,
    run_operation_resolution_phase,
};
use operations_phase::{OperationsPhaseOutcome, run_operations_phase};
use scheduled_phases::{
    run_business_cycle_phase, run_enterprise_cycle_phase, run_investigation_work_phase,
};

/// Day-boundary decay runs first in the reputation cluster: eligible aged impressions fade one
/// authored step before anything new lands, so consequences applied this minute are not
/// immediately eroded by the same boundary's decay pass. Resolved operations feed competence/
/// fear/exposure consequences; rackets that drew a racket inquiry this tick pay the same
/// institutional memory as an exposed operation. The player organization reads its own standing
/// shifts through the canonical Standing-report path — legitimate self-knowledge.
fn apply_reputation_phase(
    registry: &Registry,
    state: &mut AppState,
    resolved_operations: &[OperationId],
    enterprise_cycles: &[EnterpriseCycleId],
) -> Result<usize, crate::reputation::reputation_system::ReputationError> {
    // Most canonical minutes have no new reputation events. Decay is day-boundary-only;
    // without either trigger there is nothing to project, preflight, or mutate. Avoid
    // rebuilding the sparse organization/audience score map on every quiet minute.
    if resolved_operations.is_empty()
        && enterprise_cycles.is_empty()
        && !crate::core::time::is_day_boundary(state.now())
    {
        return Ok(0);
    }
    // Snapshot the consequence inputs before any reputation write. Both the read-only projection
    // and the authoritative pass consume these exact vectors in this exact order, so clamping and
    // same-minute accumulation cannot make the report preflight drift from the later mutations.
    let operation_consequences: Vec<_> = resolved_operations
        .iter()
        .map(|operation| {
            let record = state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must exist for reputation consequences");
            let resolution = record
                .resolution()
                .expect("resolved operation carries its resolution");
            (
                record.responsible_organization(),
                record.kind(),
                record.approach(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            )
        })
        .collect();
    let racket_inquiries: Vec<_> = enterprise_cycles
        .iter()
        .filter_map(|cycle_id| {
            let cycle = state
                .enterprises()
                .get_cycle(*cycle_id)
                .expect("settled enterprise cycle must exist for reputation consequences");
            cycle.drew_enforcement_attention().then(|| {
                state
                    .enterprises()
                    .get_enterprise(cycle.enterprise())
                    .expect("settled enterprise cycle must reference its enterprise")
                    .organization()
            })
        })
        .collect();

    match preflight_reputation_phase_reports(
        registry,
        state,
        &operation_consequences,
        &racket_inquiries,
    ) {
        Ok(()) => {}
        Err(crate::reputation::reputation_system::ReputationError::IdExhaustion(_)) => {
            return Ok(0);
        }
        Err(error) => return Err(error),
    }

    let mut changed =
        crate::reputation::reputation_system::apply_daily_reputation_decay(registry, state);
    for (organization, kind, approach, objective_outcome, exposure_level) in operation_consequences
    {
        let shifts = crate::reputation::reputation_system::apply_operation_reputation_consequences(
            registry,
            state,
            organization,
            kind,
            approach,
            objective_outcome,
            exposure_level,
        )?;
        changed = changed
            .checked_add(shifts.len())
            .expect("one tick cannot contain enough reputation shifts to overflow usize");
    }
    for organization in racket_inquiries {
        let shifts =
            crate::reputation::reputation_system::apply_racket_inquiry_reputation_consequences(
                registry,
                state,
                organization,
            )?;
        changed = changed
            .checked_add(shifts.len())
            .expect("one tick cannot contain enough reputation shifts to overflow usize");
    }
    Ok(changed)
}

fn preflight_reputation_phase_reports(
    registry: &Registry,
    state: &AppState,
    operation_consequences: &[OperationReputationEvent],
    racket_inquiries: &[crate::core::id::OrganizationId],
) -> Result<(), crate::reputation::reputation_system::ReputationError> {
    // Decay itself emits no report. Project that first, then replay every current consequence.
    // A player consequence group needs exactly one Standing report iff at least one of its
    // clamped shifts still moves after all earlier same-minute shifts. Reserve that exact count
    // before decay so report exhaustion cannot leave a partially updated reputation phase.
    let player = state.player_organization();
    let mut projection =
        crate::reputation::reputation_system::ReputationPhaseProjection::after_daily_decay(
            registry, state,
        );
    let mut standing_reports = 0_u32;
    for &event in operation_consequences {
        let organization = event.0;
        let moved = projection.apply_operation_consequences(registry, state, event)?;
        if moved > 0 && player == Some(organization) {
            standing_reports = standing_reports
                .checked_add(1)
                .expect("same-minute Standing report count must fit u32");
        }
    }
    for &organization in racket_inquiries {
        let moved = projection.apply_racket_inquiry_consequences(registry, state, organization)?;
        if moved > 0 && player == Some(organization) {
            standing_reports = standing_reports
                .checked_add(1)
                .expect("same-minute Standing report count must fit u32");
        }
    }
    state.ids.reserve(IdKind::Report, standing_reports)?;
    Ok(())
}

/// Synthesizes the player organization's due executive brief.
fn synthesize_executive_brief(registry: &Registry, state: &mut AppState) -> Option<ReportId> {
    state.player_organization().and_then(|recipient| {
        is_executive_brief_due(registry, state.now())
            .then(|| {
                let plan = decide_executive_brief(registry, state, recipient)
                    .expect("due player executive brief must produce a valid synthesis plan");
                let validated = validate_executive_brief_plan(state, plan)
                    .expect("fresh executive brief plan must validate");
                match validated.commit(state) {
                    Ok(report) => Some(report),
                    Err(crate::reports::executive_brief::ExecutiveBriefError::Report(
                        crate::reports::report_system::ReportError::IdExhaustion(_),
                    )) => None,
                    Err(error) => {
                        panic!("validated executive brief must commit atomically: {error}")
                    }
                }
            })
            .flatten()
    })
}

fn draw_signed_variance(rng: &mut impl RngCore, limit: u8) -> i8 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("signed variance choice range overflowed usize");
    let draw = draw_index(rng, width).expect("signed variance range is never empty");
    let signed =
        i16::try_from(draw).expect("operation variance draw must fit i16") - i16::from(limit);
    i8::try_from(signed).expect("authored signed variance limit must fit i8")
}

fn draw_basis_point_variance(rng: &mut impl RngCore, limit: u16) -> i16 {
    let width = usize::from(limit)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("enterprise variance choice range overflowed usize");
    let draw = draw_index(rng, width).expect("enterprise variance range is never empty");
    let signed =
        i32::try_from(draw).expect("enterprise variance draw must fit i32") - i32::from(limit);
    i16::try_from(signed).expect("authored enterprise variance limit must fit i16")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum RandomDecisionError {
    #[error("cannot choose from an empty choice set")]
    EmptyChoiceSet,
}

pub(crate) fn draw_index(
    rng: &mut impl RngCore,
    choice_count: usize,
) -> Result<usize, RandomDecisionError> {
    if choice_count == 0 {
        return Err(RandomDecisionError::EmptyChoiceSet);
    }
    let bound = u64::try_from(choice_count).expect("usize choice count must fit into u64");
    // `rejection_zone` is an exact multiple of `bound`. The comparison must stay strict:
    // accepting the boundary itself would give remainder zero one extra representation and
    // bias the choice. This formulation may reject a tiny full block when `bound` divides
    // 2^64 exactly, but keeps the accepted domain trivially auditable and unbiased.
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = rng.next_u64();
        if draw < rejection_zone {
            return Ok((draw % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests;
