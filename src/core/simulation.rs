//! Deterministic top-level simulation tick and state-owned random decision helpers.
//!
//! `run_tick` is the only authoritative minute (contractual phase order).
//! See `ARCHITECTURE.md` for the authoritative phase diagram and dependency tower.
//! New autonomous work must slot explicitly here with a "runs after X so Y" comment.

use crate::core::id::{
    BusinessCycleId, CharacterId, EnterpriseCycleId, InvestigationId, InvestigationWorkId,
    OperationId, OpportunityId, PoliceResponseId, ProsecutionCaseId, RecruitmentAttemptId,
    ReportId,
};
use crate::core::invariants::validate_invariants;
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::DecisionResponse;
use crate::decisions::decision_system::{DecisionRequestOutcome, validate_resolve_decision};
use crate::economy::business_economy_system::{
    decide_business_cycle, find_due_businesses, validate_business_cycle_plan,
};
use crate::enterprises::enterprise_execution::{
    EnterpriseCycleRandomness, decide_enterprise_cycle, find_due_enterprises,
    validate_enterprise_cycle_plan,
};
use crate::legal::investigation_system::apply_autonomous_investigator_staffing;
use crate::legal::investigation_system::apply_cold_case_decay;
use crate::legal::investigation_work_execution::{
    InvestigationWorkRandomness, apply_evidence_review_scheduling,
    decide_investigation_work_resolution, find_due_scheduled_investigation_work,
    validate_investigation_work_resolution_plan,
};
use crate::operations::operation_abort::{
    validate_deadline_missed_operation, validate_expired_opportunity_operation,
    validate_objective_unavailable_operation,
};
use crate::operations::operation_execution::{
    OperationResolutionRandomness, decide_operation_resolution, find_due_in_progress_operations,
    validate_operation_resolution_plan,
};
use crate::operations::operation_system::{
    OperationError, OperationTransition, apply_transition, find_due_authorized_operations,
    find_due_operations_with_missed_deadlines, has_missed_operation_deadline,
};
use crate::operations::police_response_integration::apply_due_police_response_arrivals;
use crate::opportunities::opportunity_system::apply_opportunity_expiry;
use crate::recruitment::autonomous_recruitment::apply_due_autonomous_recruitment;
use crate::registry::Registry;
use crate::reports::executive_brief::{
    decide_executive_brief, is_executive_brief_due, validate_executive_brief_plan,
};
use rand_core::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    pub now: SimTime,
    pub started_operations: Vec<OperationId>,
    pub arrived_police_responses: Vec<PoliceResponseId>,
    /// Every decision request raised this tick, regardless of owning subsystem. The outcome
    /// preserves whether this player-owned attention event requests an adapter pause.
    pub decision_requests: Vec<DecisionRequestOutcome>,
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
    pub business_cycles: Vec<BusinessCycleId>,
    pub enterprise_cycles: Vec<EnterpriseCycleId>,
    pub payrolls: Vec<crate::world::payroll_execution::PayrollOutcome>,
    pub recruitment_attempts: Vec<RecruitmentAttemptId>,
    pub autonomous_enterprises: Vec<crate::core::id::EnterpriseId>,
    pub expired_opportunities: Vec<OpportunityId>,
    pub cold_case_suspensions: Vec<InvestigationId>,
    pub cold_case_closures: Vec<InvestigationId>,
    pub executive_brief: Option<ReportId>,
}

pub fn run_tick(registry: &Registry, state: &mut AppState) -> TickOutcome {
    // Simulation speed is an adapter concern. The canonical pipeline always advances one minute,
    // so normal/fast/very-fast modes call the exact same deterministic path more often.
    state.advance_clock(SimDuration::ONE_MINUTE);
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
    // operations (police arrivals, starts, overdue cleanup, resolution), legal institutional work
    // (staffing, detective work, new custody, representation, informants, cold decay), economy
    // cycles (businesses, enterprises), then the day-boundary governance cluster: payroll,
    // reputation (decay before current consequences), recruitment, delegated expansion (which
    // consumes current police fear), and executive synthesis last so the due brief sees everything
    // above.
    let expired_opportunities = apply_opportunity_expiry(registry, state)
        .expect("valid state should expire every due opportunity atomically");
    let (started_operations, arrived_police_responses, mut decision_requests, resolved_operations) =
        run_operations_phase(registry, state);
    let staffed_investigations = apply_autonomous_investigator_staffing(state)
        .expect("valid state should staff available investigators onto active cases");
    // Evidence-review scheduling scans every active staffed case, not only cases staffed this
    // minute: later reviewable evidence must enter institutional casework sequentially rather
    // than becoming inert after the case's first forensic attempt.
    let scheduled_investigation_work = apply_evidence_review_scheduling(registry, state)
        .expect("valid state should schedule due reviewable evidence for active staffed cases");
    // Witness interviews are scheduled after evidence reviews so a witness registered by an
    // operation resolving earlier in this same minute is interviewable as soon as its case
    // has an investigator.
    let scheduled_witness_interviews =
        crate::legal::investigation_work_execution::apply_witness_interview_scheduling(
            registry, state,
        )
        .expect("valid state should schedule due witness interviews");
    let resolved_investigation_work = run_investigation_work_phase(registry, state);
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
    // only after the irreversible decision. It also sees every new arrest created above and
    // concludes automatic retainers for matters already ended.
    let automatic_legal_support =
        crate::legal::legal_representation_system::apply_automatic_legal_support(registry, state)
            .expect("valid state should resolve automatic legal-support retention");
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
    let business_cycles = run_business_cycle_phase(registry, state);
    let enterprise_cycles = run_enterprise_cycle_phase(registry, state);
    // Payroll runs after the day's enterprise and business cycles so earned revenue can fund
    // the same day's wages. Reputation then settles the day boundary before recruitment: daily
    // decay advances only impressions old enough to fade, while operation/vice consequences from
    // this minute land before candidates judge an outfit's underworld competence. Together with
    // payroll, every authored recruitment input therefore reflects the current minute rather
    // than a mixture of pre- and post-boundary state.
    let payrolls = crate::world::payroll_execution::apply_daily_payroll(registry, state)
        .expect("valid state should settle every due criminal-organization payroll");
    apply_reputation_phase(registry, state, &resolved_operations, &enterprise_cycles);
    let recruitment = apply_due_autonomous_recruitment(registry, state)
        .expect("valid state should resolve every due autonomous recruitment action");
    let recruitment_attempts = recruitment.attempts;
    decision_requests.extend(recruitment.approval_requests);
    // Delegated rival expansion runs after recruitment so a mandate whose crew changed this
    // minute governs with its current roster, and after reputation so a vice hit this minute can
    // make the organization keep its head down immediately. Selection consumes
    // no randomness, so matched branches observe identical rival growth unless their own actions
    // touched rival state.
    let autonomous_enterprises =
        crate::enterprises::autonomous_expansion::apply_due_autonomous_enterprises(registry, state)
            .expect("valid state should resolve every due autonomous enterprise expansion");
    // Executive synthesis runs last so a due brief sees every report and decision created by
    // operational, investigative, financial, and delegated personnel work that resolved in the
    // same simulation minute.
    let executive_brief = synthesize_executive_brief(registry, state);
    validate_invariants(state);
    TickOutcome {
        now: state.now(),
        started_operations,
        arrived_police_responses,
        decision_requests,
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
        business_cycles,
        enterprise_cycles,
        payrolls,
        recruitment_attempts,
        autonomous_enterprises,
        expired_opportunities,
        cold_case_suspensions: cold_case_decay.suspended,
        cold_case_closures: cold_case_decay.closed,
        executive_brief,
    }
}

/// Processes due police-response arrivals, starts due authorized operations, aborts missed
/// deadlines (through the pending decision when one exists), and resolves due in-progress
/// operations with pre-drawn deterministic variance.
fn run_operations_phase(
    registry: &Registry,
    state: &mut AppState,
) -> (
    Vec<OperationId>,
    Vec<PoliceResponseId>,
    Vec<DecisionRequestOutcome>,
    Vec<OperationId>,
) {
    // Process responses that were dispatched on earlier ticks before admitting new work.
    // Authorization deliberately allows exact back-to-back participant windows. A response
    // arriving on that boundary can turn the earlier operation into an unresolved commitment;
    // that state must be visible to begin-time participant validation before the follow-up starts.
    // Newly started operations cannot add another due arrival here because registry validation
    // requires every police-response delay to be strictly positive.
    let police_response_outcome = apply_due_police_response_arrivals(state)
        .expect("due police responses must commit through canonical arrival processing");
    let arrived_police_responses = police_response_outcome.arrived;
    let decision_requests = police_response_outcome.decisions;

    let due_authorized = find_due_authorized_operations(state);
    let mut started_operations = Vec::with_capacity(due_authorized.len());
    for operation in due_authorized {
        if has_missed_operation_deadline(registry, state, operation) {
            validate_deadline_missed_operation(registry, state, operation)
                .expect("a missed operation deadline must validate")
                .commit(state)
                .expect("a missed operation deadline must commit atomically");
        } else if let Some((opportunity, _)) = state
            .opportunities()
            .expired_window_for_operation(operation, state.now())
        {
            validate_expired_opportunity_operation(state, operation, opportunity.id())
                .expect("an expired linked opportunity must validate a pre-start abort")
                .commit(state)
                .expect("an expired linked opportunity must abort atomically");
        } else {
            match apply_transition(registry, state, operation, OperationTransition::Begin) {
                Ok(()) => started_operations.push(operation),
                // A future assignment can become temporarily unavailable when an earlier
                // operation remains paused longer than projected at authorization time. The due
                // operation stays Authorized and retries on later ticks. Completion deadlines
                // and linked opportunity windows are handled by the pre-checks above, so temporary
                // unavailability cannot silently carry work beyond an authored viability boundary.
                Err(
                    OperationError::ParticipantBusy { .. }
                    | OperationError::DetainedParticipant { .. },
                ) => {}
                Err(OperationError::ObjectiveUnavailable { blocker, .. }) => {
                    validate_objective_unavailable_operation(state, operation, blocker)
                        .expect("an unavailable due objective must validate a pre-start abort")
                        .commit(state)
                        .expect("an unavailable due objective must abort atomically");
                }
                Err(error) => {
                    panic!(
                        "due authorized operation could not begin through its canonical path: {error}"
                    )
                }
            }
        }
    }
    // Preserve the established retry boundary: a follow-up that was blocked at begin time by a
    // still-paused operation does not immediately retry merely because overdue cleanup releases
    // the participant later in this same phase. It remains Authorized until the next tick.
    for operation in find_due_operations_with_missed_deadlines(state) {
        let record = state
            .operations()
            .get_operation(operation)
            .expect("overdue operation must still exist");
        if let Some(decision) = state.decisions().pending_for_operation(operation) {
            let recipient = record.responsible_organization();
            validate_resolve_decision(
                registry,
                state,
                decision,
                recipient,
                DecisionResponse::Abort,
            )
            .expect("an overdue operation decision must support automatic abort")
            .commit(state)
            .expect("automatic deadline decision abort must commit atomically");
        } else {
            validate_deadline_missed_operation(registry, state, operation)
                .expect("an overdue in-progress operation must validate a deadline abort")
                .commit(state)
                .expect("an overdue in-progress operation must abort atomically");
        }
    }
    let due_operations = find_due_in_progress_operations(state);
    let mut resolved_operations = Vec::with_capacity(due_operations.len());
    for operation in due_operations {
        let kind = state
            .operations()
            .get_operation(operation)
            .expect("due operation must still exist")
            .kind();
        let execution = registry.get_operation(kind).execution();
        let execution_variance =
            draw_signed_variance(state.operation_rng_mut(), execution.variance_limit());
        let exposure_variance = draw_signed_variance(
            state.operation_rng_mut(),
            execution.exposure_variance_limit(),
        );
        let plan = decide_operation_resolution(
            registry,
            state,
            operation,
            OperationResolutionRandomness::new(execution_variance, exposure_variance),
        )
        .expect("due in-progress operation must resolve a valid plan");
        let resolved = validate_operation_resolution_plan(registry, state, plan)
            .expect("fresh operation resolution plan must validate")
            .commit(state)
            .expect("validated operation resolution must commit atomically");
        resolved_operations.push(resolved);
    }
    (
        started_operations,
        arrived_police_responses,
        decision_requests,
        resolved_operations,
    )
}

/// Resolves due scheduled detective work with pre-drawn variance. Runs after operation
/// consequences so legal state created by an operation is visible to later institutional work
/// in the same minute without bypassing evidence ownership.
fn run_investigation_work_phase(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<InvestigationWorkId> {
    let due_work = find_due_scheduled_investigation_work(state);
    let mut resolved = Vec::with_capacity(due_work.len());
    for work in due_work {
        let kind = state
            .legal()
            .get_investigation_work(work)
            .expect("due investigation work must still exist")
            .kind();
        let variance_limit = registry.get_investigation_work(kind).variance_limit();
        let variance = draw_signed_variance(state.investigation_rng_mut(), variance_limit);
        let plan = decide_investigation_work_resolution(
            registry,
            state,
            work,
            InvestigationWorkRandomness::new(variance),
        )
        .expect("due investigation work must resolve a valid plan");
        let committed = validate_investigation_work_resolution_plan(registry, state, plan)
            .expect("fresh investigation work resolution plan must validate")
            .commit(state)
            .expect("validated investigation work must commit atomically");
        resolved.push(committed);
    }
    resolved
}

/// Settles due business operating cycles with pre-drawn gross variance.
fn run_business_cycle_phase(registry: &Registry, state: &mut AppState) -> Vec<BusinessCycleId> {
    let due_businesses = find_due_businesses(state);
    let mut business_cycles = Vec::with_capacity(due_businesses.len());
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
        let variance = draw_basis_point_variance(state.business_rng_mut(), variance_limit);
        let plan = decide_business_cycle(registry, state, business, variance)
            .expect("due active business must resolve a valid cycle plan");
        let cycle = validate_business_cycle_plan(state, plan)
            .expect("fresh business cycle plan must validate")
            .commit(state)
            .expect("validated business cycle must commit atomically");
        business_cycles.push(cycle);
    }
    business_cycles
}

/// Settles due enterprise cycles. Both draws happen unconditionally per due cycle so the
/// enterprise stream consumes the same number of values whatever the district's case pressure
/// turns out to be.
fn run_enterprise_cycle_phase(registry: &Registry, state: &mut AppState) -> Vec<EnterpriseCycleId> {
    let due_enterprises = find_due_enterprises(state);
    let mut enterprise_cycles = Vec::with_capacity(due_enterprises.len());
    for enterprise in due_enterprises {
        let kind = state
            .enterprises()
            .get_enterprise(enterprise)
            .expect("due enterprise must exist")
            .kind();
        let economics = registry.get_enterprise(kind).economics();
        let variance = draw_basis_point_variance(
            state.enterprise_rng_mut(),
            economics.gross_variance_basis_points(),
        );
        let vice_attention_roll = u16::try_from(
            draw_index(state.enterprise_rng_mut(), 10_000)
                .expect("vice-attention roll range is never empty"),
        )
        .expect("vice-attention roll fits u16");
        let plan = decide_enterprise_cycle(
            registry,
            state,
            enterprise,
            EnterpriseCycleRandomness::new(variance, vice_attention_roll),
        )
        .expect("due active enterprise must resolve a valid cycle plan");
        let cycle = validate_enterprise_cycle_plan(state, plan)
            .expect("fresh enterprise cycle plan must validate")
            .commit(state)
            .expect("validated enterprise cycle must commit atomically");
        enterprise_cycles.push(cycle);
    }
    enterprise_cycles
}

/// Day-boundary decay runs first in the reputation cluster: eligible aged impressions fade one
/// authored step before anything new lands, so consequences applied this minute are not
/// immediately eroded by the same boundary's decay pass. Resolved operations feed competence/
/// fear/exposure consequences; rackets that drew a vice inquiry this tick pay the same
/// institutional memory as an exposed operation. The player organization reads its own standing
/// shifts through the canonical Standing-report path — legitimate self-knowledge.
fn apply_reputation_phase(
    registry: &Registry,
    state: &mut AppState,
    resolved_operations: &[OperationId],
    enterprise_cycles: &[EnterpriseCycleId],
) {
    crate::reputation::reputation_system::apply_daily_reputation_decay(registry, state);
    for operation in resolved_operations {
        let (organization, approach, objective_outcome, exposure_level) = {
            let record = state
                .operations()
                .get_operation(*operation)
                .expect("resolved operation must exist for reputation consequences");
            let resolution = record
                .resolution()
                .expect("resolved operation carries its resolution");
            (
                record.responsible_organization(),
                record.approach(),
                resolution.objective_outcome(),
                resolution.exposure().level(),
            )
        };
        crate::reputation::reputation_system::apply_operation_reputation_consequences(
            registry,
            state,
            organization,
            approach,
            objective_outcome,
            exposure_level,
        )
        .expect("valid state should apply operation reputation consequences");
    }
    for cycle_id in enterprise_cycles {
        let organization = {
            let cycle = state
                .enterprises()
                .get_cycle(*cycle_id)
                .expect("settled enterprise cycle must exist for reputation consequences");
            if !cycle.drew_vice_attention() {
                continue;
            }
            state
                .enterprises()
                .get_enterprise(cycle.enterprise())
                .expect("settled enterprise cycle must reference its enterprise")
                .organization()
        };
        crate::reputation::reputation_system::apply_vice_inquiry_reputation_consequences(
            registry,
            state,
            organization,
        )
        .expect("valid state should apply vice-inquiry reputation consequences");
    }
}

/// Synthesizes the player organization's due executive brief.
fn synthesize_executive_brief(registry: &Registry, state: &mut AppState) -> Option<ReportId> {
    state.player_organization().and_then(|recipient| {
        is_executive_brief_due(registry, state.now()).then(|| {
            let plan = decide_executive_brief(registry, state, recipient)
                .expect("due player executive brief must produce a valid synthesis plan");
            validate_executive_brief_plan(state, plan)
                .expect("fresh executive brief plan must validate")
                .commit(state)
                .expect("validated executive brief must commit atomically")
        })
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
    let rejection_zone = u64::MAX - (u64::MAX % bound);
    loop {
        let draw = rng.next_u64();
        if draw < rejection_zone {
            return Ok((draw % bound) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::legal::JurisdictionDraft;
    use crate::legal::jurisdiction_system::validate_set_jurisdiction;
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{
        OperationApproach, OperationConstraint, OperationContingency, OperationDraft,
        OperationKind, OperationObjective, OperationStatus, RoleKind,
    };
    use crate::world::world_system::{
        designate_player_organization, insert_business, insert_character, insert_neighborhood,
        insert_organization,
    };
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CapabilityKind, CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn test_rating(value: u8) -> Rating {
        Rating::try_new(value).expect("simulation test rating must be valid")
    }

    #[test]
    fn domain_random_streams_do_not_cross_contaminate_unrelated_simulation_work() {
        let mut baseline = AppState::new(0x1933_0814);
        let mut operation_heavy = baseline.clone();

        for _ in 0..64 {
            draw_signed_variance(operation_heavy.operation_rng_mut(), 12);
        }

        for _ in 0..32 {
            assert_eq!(
                draw_basis_point_variance(baseline.business_rng_mut(), 2_500),
                draw_basis_point_variance(operation_heavy.business_rng_mut(), 2_500)
            );
            assert_eq!(
                draw_basis_point_variance(baseline.enterprise_rng_mut(), 2_500),
                draw_basis_point_variance(operation_heavy.enterprise_rng_mut(), 2_500)
            );
            assert_eq!(
                draw_signed_variance(baseline.investigation_rng_mut(), 12),
                draw_signed_variance(operation_heavy.investigation_rng_mut(), 12)
            );
        }
    }

    #[test]
    fn same_minute_police_arrival_blocks_back_to_back_participant_start() {
        let registry = build_registry();
        let mut state = AppState::new(0xB0A0_DA7A);
        let crew = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Boundary Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("crew should validate");
        designate_player_organization(&mut state, crew)
            .expect("boundary crew should be the player organization");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Boundary Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police organization should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Boundary Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: test_rating(50),
                        commercial_activity: test_rating(50),
                        illicit_demand: test_rating(50),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: test_rating(100),
                    },
                },
            },
        )
        .expect("neighborhood should validate");
        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: police,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: test_rating(80),
            },
        )
        .expect("jurisdiction should validate")
        .commit(&mut state)
        .expect("jurisdiction should commit");

        let leader = insert_character(
            &mut state,
            CharacterDraft {
                name: "Boundary Leader".to_owned(),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([
                    (CapabilityKind::Management, test_rating(75)),
                    (CapabilityKind::Intimidation, test_rating(75)),
                ]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("leader should validate");
        let target = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Boundary Store".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner: BusinessOwner::Independent,
            },
        )
        .expect("target business should validate");

        let first = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Boundary collection".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: crew,
                leader,
                objective: OperationObjective::ObtainCash {
                    target: EntityRef::Business(target),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
                intelligence: BTreeSet::new(),
                constraints: vec![OperationConstraint::CompleteBy(SimTime::from_minutes(4))],
                contingencies: vec![OperationContingency::RequestDecisionOnPoliceArrival],
                scheduled_for: SimTime::ZERO,
            },
        )
        .expect("first boundary operation should validate")
        .commit(&mut state)
        .expect("first boundary operation should commit");
        let follow_up = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: "Boundary follow-up".to_owned(),
                kind: OperationKind::Intimidation,
                responsible_organization: crew,
                leader,
                objective: OperationObjective::ObtainCash {
                    target: EntityRef::Business(target),
                },
                approach: OperationApproach::Intimidating,
                roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: SimTime::from_minutes(4),
            },
        )
        .expect("exact back-to-back follow-up should authorize")
        .commit(&mut state)
        .expect("exact back-to-back follow-up should commit");

        let first_tick = run_tick(&registry, &mut state);
        assert_eq!(first_tick.now, SimTime::from_minutes(1));
        assert_eq!(first_tick.started_operations, vec![first]);
        let first_record = state
            .operations()
            .get_operation(first)
            .expect("first operation should persist");
        assert_eq!(
            first_record.resolution_due_at(),
            Some(SimTime::from_minutes(4))
        );
        let response = first_record
            .police_response()
            .expect("high ambient police presence should dispatch a response");
        assert_eq!(
            state
                .legal()
                .get_police_response(response)
                .expect("response should persist")
                .arrival_due_at(),
            SimTime::from_minutes(4)
        );

        for expected_minute in 2..=3 {
            let tick = run_tick(&registry, &mut state);
            assert_eq!(tick.now, SimTime::from_minutes(expected_minute));
            assert!(tick.started_operations.is_empty());
            assert!(tick.arrived_police_responses.is_empty());
        }

        let boundary = run_tick(&registry, &mut state);
        assert_eq!(boundary.now, SimTime::from_minutes(4));
        assert_eq!(boundary.arrived_police_responses, vec![response]);
        assert_eq!(boundary.decision_requests.len(), 1);
        assert!(
            boundary.started_operations.is_empty(),
            "the unresolved arrival decision must retain the leader before the back-to-back follow-up begins"
        );
        assert_eq!(
            state
                .operations()
                .get_operation(first)
                .expect("first operation should persist")
                .status(),
            OperationStatus::AwaitingDecision
        );
        assert_eq!(
            state
                .operations()
                .get_operation(follow_up)
                .expect("deferred follow-up should persist")
                .status(),
            OperationStatus::Authorized
        );
        validate_state(&state).expect("same-minute arrival boundary state should validate");
        validate_invariants(&state);
    }
}
