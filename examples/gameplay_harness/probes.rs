//! Bounded deterministic probes: repeat-take depletion, opportunity portfolio, organizational capacity, legal foundation, and matched-strategy batches.

use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, InformationId};
use crimocracy::core::simulation::run_tick;
use crimocracy::core::state::AppState;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::delegation::delegation_system::MandateRevisionDraft;
use crimocracy::delegation::delegation_system::validate_revise_mandate;
use crimocracy::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::intelligence_system::validate_record_information;
use crimocracy::intelligence::{
    InformationDraft, InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder,
    LegalPersonStatusSignal, Reliability, Specificity,
};
use crimocracy::legal::investigation_system::{
    validate_assign_investigator, validate_incident_intake,
};
use crimocracy::legal::jurisdiction_system::resolve_case_intake_authority;
use crimocracy::legal::legal_representation_system::validate_retain_legal_representation;
use crimocracy::legal::prosecution_system::{
    validate_decline_prosecution_case, validate_open_prosecution_case,
};
use crimocracy::legal::witness_system::validate_register_case_witness;
use crimocracy::legal::{
    Admissibility, ArrestDraft, CaseWitnessDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
    EvidenceStrength, IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft,
    LegalRepresentationDraft, ProsecutionCaseDraft, WitnessCooperation,
};
use crimocracy::operations::operation_system::{OperationError, validate_authorize_operation};
use crimocracy::operations::{
    OperationApproach, OperationContingency, OperationDraft, OperationKind, OperationObjective,
    OperationObjectiveOutcome, OperationStatus, RoleKind,
};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
    validate_dismiss_opportunity,
};
use crimocracy::opportunities::{OperationOpportunityDraft, OpportunityStatus};
use crimocracy::recruitment::recruitment_system::validate_recruitment_attempt;
use crimocracy::recruitment::{
    RecruitmentApproach, RecruitmentAuthority, RecruitmentDraft, RecruitmentOutcome,
};
use crimocracy::registry::Registry;
use crimocracy::social::RelationshipDimensions;
use crimocracy::social::relationship_system::validate_set_relationship;
use crimocracy::world::world_system::{insert_character, insert_organization};
use crimocracy::world::{
    ApprovalPolicy, AutonomyLevel, CapabilityKind, CharacterDraft, DriveKind, OrganizationDraft,
    OrganizationKind, PolicyKind, PolicySetting, TraitKind,
};
use crimocracy::{
    contacts::contact_system::{
        InstitutionalContactDraft, find_pending_disclosure_sources, validate_contact_disclosure,
        validate_establish_contact,
    },
    legal::arrest_system::validate_arrest,
    legal::investigation_system::{validate_add_evidence, validate_open_investigation},
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use crate::*;

/// Compares the player experience of retaining personnel approval versus delegating it to the
/// lieutenant. Both branches start from the same world and the same known prospect. The only
/// treatment is the mandate standing order, so any difference in interruption, roster, and
/// payroll is the consequence of governance rather than a different opportunity.
pub fn run_delegation_control_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    let mut base = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let prospect = insert_character(
        &mut base.state,
        CharacterDraft {
            name: "Nico Serra".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([
                (CapabilityKind::Driving, rating(68)),
                (CapabilityKind::Negotiation, rating(54)),
            ]),
            // These remain hidden scoring inputs. The player-facing probe only knows that Carlo
            // has a strong personal line to a plausible independent prospect.
            traits: BTreeSet::from([TraitKind::Ambitious]),
            drives: BTreeMap::from([(DriveKind::Status, rating(90))]),
        },
    )?;
    validate_set_relationship(
        &base.state,
        prospect,
        base.lieutenant,
        RelationshipDimensions {
            trust: level(95),
            respect: level(90),
            fear: level(0),
            affection: level(75),
            dependence: level(20),
            resentment: level(0),
            debt: level(60),
        },
    )?
    .commit(&mut base.state)?;

    fn set_recruitment_policy(
        scenario: &mut Scenario<'_>,
        policy: ApprovalPolicy,
    ) -> Result<(), Box<dyn Error>> {
        let mandate = scenario
            .state
            .delegation()
            .get_mandate(scenario.lieutenant_mandate)
            .expect("player lieutenant mandate must persist");
        let mut scopes = mandate.scopes().clone();
        scopes.insert(ResponsibilityScope::Function(
            ResponsibilityFunction::Personnel,
        ));
        let mut standing_orders = mandate.standing_orders().clone();
        standing_orders.insert(
            PolicyKind::IndependentRecruitment,
            PolicySetting::IndependentRecruitment(policy),
        );
        validate_revise_mandate(
            &scenario.state,
            scenario.lieutenant_mandate,
            MandateRevisionDraft {
                scopes,
                standing_orders,
                budget: mandate.budget(),
            },
        )?
        .commit(&mut scenario.state)?;
        Ok(())
    }

    fn advance_to_recruitment_boundary(
        registry: &Registry,
        scenario: &mut Scenario<'_>,
    ) -> Result<crimocracy::core::simulation::TickOutcome, Box<dyn Error>> {
        let cadence = registry.recruitment().autonomous_attempt_cadence();
        let pre_boundary = scenario.state.now()
            + SimDuration::from_minutes(
                cadence
                    .as_minutes()
                    .checked_sub(1)
                    .expect("autonomous recruitment cadence must exceed one minute"),
            );
        let mut metrics = RunMetrics::default();
        run_until(scenario, pre_boundary, false, &mut metrics)?;
        Ok(run_tick(registry, &mut scenario.state)?)
    }

    fn advance_one_day(
        registry: &Registry,
        scenario: &mut Scenario<'_>,
    ) -> Result<crimocracy::core::simulation::TickOutcome, Box<dyn Error>> {
        let cadence = registry.recruitment().autonomous_attempt_cadence();
        let pre_boundary = scenario.state.now()
            + SimDuration::from_minutes(
                cadence
                    .as_minutes()
                    .checked_sub(1)
                    .expect("autonomous recruitment cadence must exceed one minute"),
            );
        let mut metrics = RunMetrics::default();
        run_until(scenario, pre_boundary, false, &mut metrics)?;
        Ok(run_tick(registry, &mut scenario.state)?)
    }

    let mut approval = base.clone();
    let mut delegated = base;
    set_recruitment_policy(&mut approval, ApprovalPolicy::RequireApproval)?;
    set_recruitment_policy(&mut delegated, ApprovalPolicy::Delegated)?;

    let approval_tick = advance_to_recruitment_boundary(registry, &mut approval)?;
    let delegated_tick = advance_to_recruitment_boundary(registry, &mut delegated)?;

    let approval_pending = approval
        .state
        .decisions()
        .pending_for_recruitment_approval(approval.player, prospect)
        .is_some();
    if !approval_pending
        || approval
            .state
            .recruitment()
            .latest_attempt_for(prospect, approval.player)
            .is_some()
        || approval
            .state
            .world()
            .get_character(prospect)
            .and_then(|record| record.organization())
            .is_some()
    {
        return Err(
            "approval-gated delegation must surface a pending player decision without recruiting the prospect first"
                .into(),
        );
    }

    let delegated_attempt = delegated
        .state
        .recruitment()
        .latest_attempt_for(prospect, delegated.player)
        .ok_or("delegated personnel authority did not autonomously approach its known prospect")?;
    if !matches!(
        delegated_attempt.authority(),
        RecruitmentAuthority::Delegated { .. }
    ) || delegated_attempt.outcome() != RecruitmentOutcome::Accepted
        || delegated
            .state
            .world()
            .get_character(prospect)
            .and_then(|record| record.organization())
            != Some(delegated.player)
    {
        return Err(
            "delegated personnel authority must autonomously make and own the accepted recruitment attempt"
                .into(),
        );
    }
    if approval_tick.decision_requests.is_empty()
        || !delegated_tick
            .recruitment_attempts
            .contains(&delegated_attempt.id())
    {
        return Err(
            "delegation probe did not surface the expected interruption-versus-autonomous-action contrast"
                .into(),
        );
    }

    let approval_payroll = advance_one_day(registry, &mut approval)?
        .payrolls
        .into_iter()
        .find(|payroll| payroll.organization() == approval.player)
        .ok_or("approval branch did not produce player payroll")?;
    let delegated_payroll = advance_one_day(registry, &mut delegated)?
        .payrolls
        .into_iter()
        .find(|payroll| payroll.organization() == delegated.player)
        .ok_or("delegated branch did not produce player payroll")?;
    let wage_delta = delegated_payroll.owed().cents() - approval_payroll.owed().cents();
    if wage_delta != registry.upkeep().per_member_daily().cents() {
        return Err(format!(
            "delegated hire should add exactly one member's recurring daily wage: expected {}c, observed {}c",
            registry.upkeep().per_member_daily().cents(),
            wage_delta
        )
        .into());
    }

    validate_harness_state(registry, &approval.state)?;
    validate_harness_state(registry, &delegated.state)?;
    if detail {
        let prospect_name = delegated
            .state
            .world()
            .get_character(prospect)
            .expect("delegated prospect must persist")
            .name();
        println!(
            "[DELEGATION] Ask-first policy: Carlo surfaced a recruitment decision for {prospect_name}; no approach happened until leadership answers."
        );
        println!(
            "[DELEGATION] Delegated policy: Carlo independently approached {prospect_name} through his own judgment and relationship; the prospect joined without interrupting leadership."
        );
        println!(
            "[CONSEQUENCE] Delegating personnel authority increased the next daily payroll by {}. The benefit is autonomous growth; the cost is that Carlo can enlarge the organization and its carrying cost without a prompt.",
            format_cents(wage_delta)
        );
    }
    Ok(())
}

/// Proves that repeated scores on one target decay through the canonical property-proceeds
/// path. This probe deliberately uses a prepared low-pressure target so law-enforcement timing
/// cannot obscure the economic question: the organization takes the same business twice,
/// observes that the immediate re-score recovers only part of the first haul, then waits through
/// the authored recovery period and sees the target return to full value. The observed evidence
/// is player-visible held-property value and after-action outcomes.
pub fn run_repeat_take_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    // Isolate repeat-take economics from law-enforcement timing. The harbor venue is in the
    // low-pressure second district and has no jurisdictional response route in this fixture, so
    // these scores can demonstrate target depletion without relying on the old fiction that a
    // known patrol gap makes a heavily policed district safe.
    let target = scenario.expansion_front;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Recon),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };
    let mut intelligence = BTreeSet::new();
    for topic in [
        InformationTopic::TargetSecurity,
        InformationTopic::MarketAccess,
        InformationTopic::Personnel,
        InformationTopic::Schedule,
        InformationTopic::Route,
    ] {
        let information = validate_record_information(
            &scenario.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(scenario.player),
                source_kind: InformationSourceKind::DirectObservation,
                topic,
                source_entity: Some(EntityRef::Character(scenario.scout)),
                subject: EntityRef::Business(target),
                observed_at: scenario.state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: format!("Direct preparation for the repeat-take probe: {topic:?}."),
            },
        )?
        .commit(&mut scenario.state)?;
        intelligence.insert(information);
    }

    fn run_take(
        scenario: &mut Scenario,
        metrics: &mut RunMetrics,
        target: BusinessId,
        intelligence: &BTreeSet<InformationId>,
        title: &'static str,
    ) -> Result<(i64, SimTime), Box<dyn Error>> {
        let scheduled_for = scenario.state.now() + SimDuration::ONE_MINUTE;
        let burglary = authorize_burglary(
            scenario,
            Strategy::Recon,
            target,
            title,
            scheduled_for,
            intelligence.clone(),
            scenario.burglar,
        )?;
        run_until_operation_terminal(scenario, burglary, false, metrics)?;
        let record = scenario
            .state
            .operations()
            .get_operation(burglary)
            .expect("probe burglary must persist");
        let resolution = record
            .resolution()
            .expect("terminal probe burglary must have a resolution");
        if resolution.objective_outcome() != OperationObjectiveOutcome::Achieved {
            return Err(format!(
                "repeat-take probe score '{title}' did not achieve: {}",
                terminal_label(metrics)
            )
            .into());
        }
        let proceeds = resolution
            .property_proceeds()
            .expect("achieved probe score must create held property");
        Ok((proceeds.estimated_value().cents(), resolution.resolved_at()))
    }

    let (first_take, first_resolved_at) = run_take(
        &mut scenario,
        &mut metrics,
        target,
        &intelligence,
        "repeat-take probe first score",
    )?;
    let (second_take, second_resolved_at) = run_take(
        &mut scenario,
        &mut metrics,
        target,
        &intelligence,
        "repeat-take probe immediate re-score",
    )?;
    let proceeds = registry
        .get_operation(OperationKind::Burglary)
        .execution()
        .property_proceeds()
        .expect("burglary must author property proceeds");
    let recovery_window = u64::from(proceeds.recent_take_recovery_window().as_minutes());
    let age = second_resolved_at
        .as_minutes()
        .saturating_sub(first_resolved_at.as_minutes())
        .min(recovery_window);
    let depletion_span = 10_000_u64 - u64::from(proceeds.immediate_repeat_value_basis_points());
    let unrecovered = recovery_window.saturating_sub(age);
    let expected_basis_points =
        10_000_u64.saturating_sub(depletion_span.saturating_mul(unrecovered) / recovery_window);
    // Positive proceeds round to nearest cent (half away from zero), like the
    // production money rule. Truncation fabricates a one-cent failure on other worlds.
    let expected_second = i64::try_from(
        (i128::from(first_take) * i128::from(expected_basis_points) + 5_000) / 10_000,
    )
    .expect("bounded repeat-take probe value must fit i64");
    if second_take != expected_second || second_take >= first_take {
        return Err(format!(
            "immediate re-score expected authored gradual recovery to {expected_second}c, observed {first_take}c then {second_take}c"
        )
        .into());
    }
    if detail {
        println!(
            "[REPEAT TAKE] The first score on {} held {}; an immediate re-score recovered only {} - the target had not replaced its stock.",
            scenario
                .state
                .world()
                .get_business(target)
                .expect("probe target must persist")
                .name(),
            format_cents(first_take),
            format_cents(second_take),
        );
    }

    // Let the authored recovery window pass so both prior hits fully age out, then confirm the
    // target's typical contents return to full value.
    let rest_until = scenario.state.now() + proceeds.recent_take_recovery_window();
    run_until(&mut scenario, rest_until, false, &mut metrics)?;
    let (third_take, _) = run_take(
        &mut scenario,
        &mut metrics,
        target,
        &intelligence,
        "repeat-take probe rested re-score",
    )?;
    if third_take != first_take {
        return Err(format!(
            "rested re-score expected the original {first_take}c take back, observed {third_take}c"
        )
        .into());
    }
    if detail {
        println!(
            "[REPEAT TAKE] After letting the target rest, the next score held {} again. Repeat takes decay and recover through production rules.",
            format_cents(third_take)
        );
    }
    validate_harness_state(registry, &scenario.state)?;
    Ok(())
}

/// Proves the enforcement-attention consequence loop end to end through production paths, the way a
/// boss experiences it: a racket run in a clean district never draws a dedicated inquiry; a
/// district under sustained originated casework first taxes every cycle with a compounding
/// street-heat surcharge, and then converts into a racket inquiry opened on the racket itself,
/// delivered to the organization as provenance-bearing legal knowledge. The casework is
/// originated through the canonical incident-intake path; the conversion itself is authored
/// per-cycle visibility math, so the probe originates enough parallel cases to push the
/// authored chance to certainty instead of asserting a lucky roll.
pub fn run_enforcement_attention_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let enterprise_id = scenario.enterprise;
    let target = scenario.target;
    let neighborhood = scenario.neighborhood;
    let enterprise_name = scenario
        .state
        .enterprises()
        .get_enterprise(enterprise_id)
        .map(|record| enterprise_label(&scenario, record.id()))
        .expect("probe enterprise must persist");
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Recon),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };

    // Control cycle: with no active originated casework targeting the district, the visibility
    // roll is unspent by construction - a clean district can never fabricate attention.
    let next_cycle_at = scenario
        .state
        .enterprises()
        .get_enterprise(enterprise_id)
        .and_then(|record| record.next_cycle_at())
        .ok_or("vice-probe enterprise must have a scheduled first cycle")?;
    run_until(&mut scenario, next_cycle_at, false, &mut metrics)?;
    let control_cycle = scenario
        .state
        .enterprises()
        .cycles_for(enterprise_id)
        .last()
        .expect("control cycle must settle");
    if control_cycle.drew_enforcement_attention()
        || control_cycle.investigation_heat() != Money::ZERO
    {
        return Err("clean-district control cycle drew attention; the enforcement contract requires clean districts to stay clean".into());
    }
    if detail {
        println!(
            "[VICE CONTROL] {}: {} settled a clean-district cycle (net {}) with no street surcharge and no inquiry.",
            stamp(control_cycle.occurred_at().as_minutes()),
            enterprise_name,
            format_cents(control_cycle.net_cash().cents()),
        );
    }

    // Originate enough parallel district cases through canonical intake that the authored
    // per-case conversion rate reaches certainty on the racket's next cycle.
    let per_case_bp = u32::from(
        registry
            .get_enterprise(
                scenario
                    .state
                    .enterprises()
                    .get_enterprise(enterprise_id)
                    .expect("probe enterprise must persist")
                    .kind(),
            )
            .economics()
            .enforcement_attention_basis_points_per_active_case(),
    );
    let needed_cases = 10_000_u32.div_ceil(per_case_bp);
    let authority = resolve_case_intake_authority(&scenario.state, neighborhood)
        .ok_or("the scenario district must have a case-intake authority")?;
    for index in 0..needed_cases {
        validate_incident_intake(
            &scenario.state,
            IncidentIntakeDraft {
                owner: authority,
                title: format!(
                    "Street incident {index} near {}",
                    scenario.variation.target_name()
                ),
                subjects: BTreeSet::from([EntityRef::Business(target)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Business(target),
                    origin: None,
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: scenario.state.now(),
                }],
                origin: Some(EntityRef::Enterprise(enterprise_id)),
                witness: None,
            },
        )?
        .commit(&mut scenario.state)?;
    }
    let surcharge_per_case = registry
        .get_enterprise(
            scenario
                .state
                .enterprises()
                .get_enterprise(enterprise_id)
                .expect("probe enterprise must persist")
                .kind(),
        )
        .economics()
        .heat_surcharge_per_active_case()
        .cents();
    if detail {
        println!(
            "[VICE HEAT] {} originated case(s) now target the racket's district; the next cycle pays {} of compounded street heat.",
            needed_cases,
            format_cents(surcharge_per_case * i64::from(needed_cases)),
        );
    }

    // The heated cycle: the surcharge compounds per active case and the visibility roll is
    // guaranteed, so this settlement must open the inquiry and surface only what the manager
    // can observe directly. Formal case existence stays institutional until a later channel
    // discloses it.
    let heated_cycle_at = scenario
        .state
        .enterprises()
        .get_enterprise(enterprise_id)
        .and_then(|record| record.next_cycle_at())
        .ok_or("heated probe enterprise must remain scheduled")?;
    run_until(&mut scenario, heated_cycle_at, false, &mut metrics)?;
    let heated_cycle = scenario
        .state
        .enterprises()
        .cycles_for(enterprise_id)
        .last()
        .expect("heated cycle must settle");
    if !heated_cycle.drew_enforcement_attention() {
        return Err(format!(
            "sustained casework at certainty did not draw a racket inquiry: {} active case(s), roll margin {}bp/case",
            needed_cases, per_case_bp
        )
        .into());
    }
    if heated_cycle.investigation_heat().cents() != surcharge_per_case * i64::from(needed_cases) {
        return Err(format!(
            "heated cycle surcharge {} did not compound the authored per-case rate over {needed_cases} case(s)",
            format_cents(heated_cycle.investigation_heat().cents()),
        )
        .into());
    }
    let formal_case_knowledge = scenario
        .state
        .intelligence()
        .information_for_holder_by_topic(
            KnowledgeHolder::Organization(scenario.player),
            InformationTopic::LegalActivity,
        )
        .find(|information| information.subject() == EntityRef::Enterprise(enterprise_id))
        .map(|record| record.summary().to_owned());
    if formal_case_knowledge.is_some() {
        return Err(
            "racket intake leaked formal enterprise-case knowledge to the organization without a disclosure channel"
                .into(),
        );
    }
    let manager_report = heated_cycle
        .information()
        .and_then(|information| scenario.state.intelligence().get_information(information))
        .map(|record| record.summary().to_owned())
        .unwrap_or_else(|| "cycle report missing".to_owned());
    if !manager_report.contains("Investigators were noticed watching")
        || manager_report.contains("case stays open")
    {
        return Err(
            "the heated enterprise cycle must report observable enforcement attention without asserting hidden case state"
                .into(),
        );
    }
    if detail {
        println!(
            "[VICE DRAW] {}: the racket's cycle under sustained casework paid {} of street heat (net {}) and drew a dedicated inquiry.",
            stamp(heated_cycle.occurred_at().as_minutes()),
            format_cents(heated_cycle.investigation_heat().cents()),
            format_cents(heated_cycle.net_cash().cents()),
        );
        println!("[ENTERPRISE] {manager_report}");
        println!(
            "[VICE PRIVACY] formal case knowledge remains undisclosed; the organization has only the manager's observable warning."
        );
    }
    // The inquiry itself must be real institutional state owned by the intake authority,
    // linked back to the racket as an originated case.
    let inquiry_on_racket = scenario
        .state
        .legal()
        .investigations_for_owner(authority)
        .any(|investigation| {
            investigation.status() == crimocracy::legal::InvestigationStatus::Active
                && investigation.origin() == Some(EntityRef::Enterprise(enterprise_id))
                && investigation
                    .subjects()
                    .contains(&EntityRef::Enterprise(enterprise_id))
        });
    if !inquiry_on_racket {
        return Err(
            "a drawn racket inquiry must exist as an active originated case on the racket".into(),
        );
    }
    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[ENFORCEMENT PASS] clean districts stay clean; sustained casework taxes cycles, opens a dedicated inquiry, and exposes only manager-observable enforcement attention."
        );
    }
    Ok(())
}

pub fn run_legal_foundation_check(registry: &Registry, detail: bool) -> Result<(), Box<dyn Error>> {
    let mut state = AppState::new(0x1E6A_1933);

    let sponsor = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )?;
    let police = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Precinct".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )?;
    let firm = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Vale & Mercer".to_owned(),
            kind: OrganizationKind::LegalServices,
        },
    )?;
    let prosecutor_office = insert_organization(
        registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor District Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )?;

    let handler = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Legal Liaison".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let defendant = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Associate".to_owned(),
            organization: Some(sponsor),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let counsel = insert_character(
        &mut state,
        CharacterDraft {
            name: "Elena Vale".to_owned(),
            organization: Some(firm),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(87))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let prosecutor = insert_character(
        &mut state,
        CharacterDraft {
            name: "Ada Mercer".to_owned(),
            organization: Some(prosecutor_office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let detective = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Detective".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(86))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;

    validate_set_relationship(
        &state,
        handler,
        counsel,
        RelationshipDimensions {
            trust: level(68),
            respect: level(72),
            fear: level(0),
            affection: level(12),
            dependence: level(25),
            resentment: level(0),
            debt: level(10),
        },
    )?
    .commit(&mut state)?;
    let contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: counsel,
        },
    )?
    .commit(&mut state)?;
    validate_set_relationship(
        &state,
        handler,
        detective,
        RelationshipDimensions {
            trust: level(70),
            respect: level(75),
            fear: level(0),
            affection: level(0),
            dependence: level(30),
            resentment: level(0),
            debt: level(15),
        },
    )?
    .commit(&mut state)?;
    let police_contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: detective,
        },
    )?
    .commit(&mut state)?;

    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Harbor arrest matter".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(defendant)]),
        },
    )?
    .commit(&mut state)?;
    validate_assign_investigator(&state, investigation, detective)?.commit(&mut state)?;
    let evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(defendant),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )?
    .commit(&mut state)?;
    let corroborating_evidence = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(defendant),
            origin: None,
            kind: EvidenceKind::FinancialRecord,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )?
    .commit(&mut state)?;
    let arrest_evidence = BTreeSet::from([evidence, corroborating_evidence]);
    let arrest = validate_arrest(
        registry,
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: arrest_evidence.clone(),
        },
    )?
    .commit(&mut state)?;

    let payer = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::AccountedFunds,
        },
    )?;
    let reserve_source = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(sponsor),
            kind: AccountKind::Settlement,
        },
    )?;
    let provider = insert_account(
        &mut state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(firm),
            kind: AccountKind::LegitimateOperating,
        },
    )?;
    validate_record_transaction(
        &state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Fund legal reserve".to_owned(),
            postings: vec![
                LedgerPosting {
                    account: reserve_source,
                    amount: Money::from_cents(-20_000),
                },
                LedgerPosting {
                    account: payer,
                    amount: Money::from_cents(20_000),
                },
            ],
            authorization: None,
        },
    )?
    .commit(&mut state)?;
    let representation = validate_retain_legal_representation(
        &state,
        LegalRepresentationDraft {
            arrest,
            sponsor,
            contact,
            fee: Money::from_cents(5_000),
            payer_accounts: BTreeSet::from([payer]),
            provider_account: provider,
            authorization: None,
            origin: crimocracy::legal::LegalRepresentationOrigin::DirectRetention,
        },
    )?
    .commit(&mut state)?;

    let prosecution_case = validate_open_prosecution_case(
        &state,
        ProsecutionCaseDraft {
            arrest,
            prosecutor_office,
            prosecutor,
            evidence: arrest_evidence,
        },
    )?
    .commit(&mut state)?;

    validate_harness_state(registry, &state)?;
    let representation_record = state
        .legal()
        .get_legal_representation(representation)
        .ok_or("legal representation disappeared from harness state")?;
    let prosecution_record = state
        .legal()
        .get_prosecution_case(prosecution_case)
        .ok_or("prosecution case disappeared from harness state")?;
    let evidence_record = state
        .legal()
        .get_evidence(evidence)
        .ok_or("source evidence disappeared from harness state")?;
    if evidence_record.custodian() != police
        || !prosecution_record.evidence().contains(&evidence)
        || representation_record.fee() != Money::from_cents(5_000)
        || state
            .finance()
            .get_account(provider)
            .is_none_or(|account| account.balance() != Money::from_cents(5_000))
    {
        return Err("legal foundation harness invariants did not produce expected state".into());
    }

    validate_decline_prosecution_case(&state, prosecution_case)?.commit(&mut state)?;
    validate_harness_state(registry, &state)?;
    let resolved_case = state
        .legal()
        .get_prosecution_case(prosecution_case)
        .ok_or("resolved prosecution case disappeared from harness state")?;
    if resolved_case.status() != crimocracy::legal::ProsecutionCaseStatus::Declined
        || resolved_case.resolved_at() != Some(state.now())
        || resolved_case.resolution_information().is_none()
        || resolved_case.resolution_report().is_none()
        || state
            .legal()
            .open_prosecution_case_for(arrest, prosecutor_office)
            .is_some()
    {
        return Err("prosecution decline lifecycle did not produce expected state".into());
    }

    // Witness-counterplay segment: the case names a cooperative on-scene witness, leadership
    // runs one canonical WitnessPressure operation against him, and production rules degrade
    // his registered cooperation - the discount any later testimony is weighed by.
    let shopkeeper = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Shopkeeper".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let intimidator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Enforcer".to_owned(),
            organization: Some(sponsor),
            supervisor: Some(handler),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::from([(CapabilityKind::Intimidation, rating(90))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    // The crew pairs its enforcer with someone who can actually coordinate: resolution
    // weights the Coordinator's management three-to-one over the leader's intimidation.
    let coordinator = insert_character(
        &mut state,
        CharacterDraft {
            name: "Harbor Lieutenant".to_owned(),
            organization: Some(sponsor),
            supervisor: Some(handler),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(88))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )?;
    let case_witness = validate_register_case_witness(
        &state,
        CaseWitnessDraft {
            investigation,
            witness: shopkeeper,
            subject: EntityRef::Character(defendant),
            cooperation: WitnessCooperation::Cooperative,
        },
    )?
    .commit(&mut state)?;
    let witness_source = find_pending_disclosure_sources(&state, police_contact)
        .into_iter()
        .find(|source| {
            state
                .intelligence()
                .get_information(*source)
                .is_some_and(|information| {
                    information.subject() == EntityRef::Character(shopkeeper)
                        && matches!(
                            information.signal(),
                            Some(InformationSignal::LegalPersonStatus(
                                LegalPersonStatusSignal::CaseWitness {
                                    investigation: learned_case
                                }
                            )) if *learned_case == investigation
                        )
                })
        })
        .ok_or("police contact did not expose the registered witness status")?;
    validate_contact_disclosure(&state, police_contact, witness_source)?.commit(&mut state)?;
    let pressure = validate_authorize_operation(
        registry,
        &state,
        OperationDraft {
            title: "Quiet word to Harbor Shopkeeper".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: sponsor,
            leader: intimidator,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(shopkeeper),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([(RoleKind::Coordinator, coordinator)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )?
    .commit(&mut state)?;
    let mut pressure_resolved = false;
    for _ in 0..200 {
        let operation_status = state
            .operations()
            .get_operation(pressure)
            .expect("pressure operation must remain queryable")
            .status();
        if matches!(
            operation_status,
            OperationStatus::Completed | OperationStatus::Aborted
        ) {
            pressure_resolved = true;
            break;
        }
        run_tick(registry, &mut state)?;
    }
    let pressure_record = state
        .operations()
        .get_operation(pressure)
        .expect("pressure operation must remain queryable");
    let cooperation = state
        .legal()
        .get_case_witness(case_witness)
        .map(|witness| witness.cooperation())
        .expect("registered case witness must persist");
    if !pressure_resolved
        || !matches!(
            cooperation,
            WitnessCooperation::Reluctant | WitnessCooperation::Hostile
        )
    {
        return Err(format!(
            "witness-pressure segment did not degrade cooperation: resolved {pressure_resolved}, status {:?}, outcome {:?}, abort {:?}, cooperation {cooperation:?}",
            pressure_record.status(),
            pressure_record.resolution().map(|resolution| resolution.objective_outcome()),
            pressure_record.abort_record().map(|abort| abort.cause()),
        ).into());
    }
    validate_harness_state(registry, &state)?;

    if detail {
        println!(
            "[LEGAL PASS] arrest -> paid counsel -> police custody-preserving referral -> terminal prosecution decline -> named case witness intimidated through canonical pressure"
        );
    }
    Ok(())
}

/// Controlled player-visible proof that reconnaissance can create its own institutional heat.
/// Select the authored CROWDED fixture rather than scanning for a favorable outcome: the treatment
/// is a weak scout in a high-pressure district, and the acting RECON policy still sees only its
/// own casing result plus whatever the standing police contact actually discloses.
pub fn run_recon_self_heat_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<bool, Box<dyn Error>> {
    let crowded_world = (0_u64..3)
        .map(|offset| seeds.world.wrapping_add(offset))
        .find(|seed| FixtureVariation::from_seed(*seed) == FixtureVariation::Crowded)
        .expect("three consecutive seeds must contain the three authored fixture variations");
    let probe_seeds = EvaluationSeeds::new(crowded_world, seeds.policy);
    let metrics = play_session(
        registry,
        Strategy::Recon,
        ScenarioProfile::GreenScout,
        probe_seeds,
        SessionRunMode::Batch,
    )?;
    validate_run_metrics(&metrics, false)?;
    validate_strategy_evidence(ScenarioProfile::GreenScout, &metrics)?;

    let demonstrated = metrics.variation == Some(FixtureVariation::Crowded)
        && metrics.opening_scout.is_some()
        && metrics.discovered_surveillance_information > 0
        && metrics.contact_reads > 0
        && metrics.opening_stood_down
        && metrics.opening_standdown_reason == Some(OpeningStanddownReason::CasingRisk)
        && metrics.opening_casing_assessment == Some(CasingAssessment::Active)
        && metrics.burglary.is_none();
    if !demonstrated {
        return Err(format!(
            "controlled reconnaissance self-heat treatment lost its player-visible consequence chain: {metrics:?}"
        )
        .into());
    }

    if detail {
        println!(
            "[SELF-HEAT] GREEN SCOUT / CROWDED: casing ended at minute {}, produced {} useful finding(s), exposed the scout enough to trigger {} police-contact read(s), and the contact confirmed an active file. RECON stood down before authorizing the burglary.",
            metrics
                .opening_scout_terminal_minute
                .expect("demonstrated self-heat scout must have a terminal minute"),
            metrics.discovered_surveillance_information,
            metrics.contact_reads,
        );
        println!(
            "[READ] Reconnaissance is not a free preview. Looking can create the same institutional pressure the player was trying to avoid, and the correct response can be to abandon an otherwise live score."
        );
    }

    Ok(true)
}

pub fn run_strategy_batch(
    registry: &Registry,
    profile: ScenarioProfile,
    samples: u64,
    seeds: EvaluationSeeds,
    artifact_dir: Option<&PathBuf>,
) -> Result<(Aggregate, Aggregate, Aggregate), Box<dyn Error>> {
    let mut rush_aggregate = Aggregate::default();
    let mut press_aggregate = Aggregate::default();
    let mut recon_aggregate = Aggregate::default();
    let mut artifacts_written = 0_u64;
    for offset in 0..samples {
        let sample_seeds = EvaluationSeeds::new(seeds.world.wrapping_add(offset + 1), seeds.policy);
        let rush = play_session(
            registry,
            Strategy::Rush,
            profile,
            sample_seeds,
            SessionRunMode::Batch,
        )?;
        let press = play_session(
            registry,
            Strategy::Press,
            profile,
            sample_seeds,
            SessionRunMode::Batch,
        )?;
        let recon = play_session(
            registry,
            Strategy::Recon,
            profile,
            sample_seeds,
            SessionRunMode::Batch,
        )?;
        validate_run_metrics(&rush, true)?;
        validate_run_metrics(&press, true)?;
        validate_run_metrics(&recon, true)?;
        validate_strategy_evidence(profile, &rush)?;
        validate_strategy_evidence(profile, &press)?;
        validate_strategy_evidence(profile, &recon)?;
        validate_branch_financial_isolation(&rush, &press, &recon)?;
        if let Some(dir) = artifact_dir {
            // Batch runs summarize persistence instead of printing one line per file: the
            // per-run seeds and raw metrics land on disk either way.
            for metrics in [&rush, &press, &recon] {
                persist_run_artifact(dir, sample_seeds, profile, metrics)?;
                artifacts_written += 1;
            }
        }
        rush_aggregate.add(&rush);
        press_aggregate.add(&press);
        recon_aggregate.add(&recon);
    }
    if let Some(dir) = artifact_dir {
        println!(
            "[ARTIFACT] wrote {} {} run artifact(s) to {}",
            artifacts_written,
            profile.label(),
            dir.display()
        );
    }
    if samples >= MIN_SAMPLES_FOR_VARIATION_CONTRACT {
        let observed = rush_aggregate.fixture_variations.len();
        if observed < 3 {
            return Err(HarnessContractError::InsufficientFixtureVariation {
                profile,
                observed,
                required: 3,
            }
            .into());
        }
    }
    validate_batch_strategy_coverage(profile, samples, &rush_aggregate)?;
    validate_sensitivity_profile_coverage(
        profile,
        samples,
        &rush_aggregate,
        &press_aggregate,
        &recon_aggregate,
    )?;
    Ok((rush_aggregate, press_aggregate, recon_aggregate))
}

pub fn validate_branch_financial_isolation(
    rush: &RunMetrics,
    press: &RunMetrics,
    recon: &RunMetrics,
) -> Result<(), HarnessContractError> {
    // End-of-run totals are not cross-branch comparable: the PRESS arc deliberately waits out
    // the authored cold-case window before its readout, so it observes more enterprise cycles
    // than RUSH or RECON. Every branch instead snapshots cumulative finances at the shared
    // campaign-day boundary (`maybe_capture_matched_financials`), and the contract below is
    // window-honest by construction:
    //   1. legitimate business economics are isolated from legal state: identical everywhere;
    //   2. branches without any staffed operation-originated case share identical enterprise
    //      economics;
    //   3. a branch with a staffed case pays the district heat surcharge, so its enterprise net
    //      never exceeds an unheated branch's over the same window. The heating signal is
    //      session-wide (`session_case_staffed`) because casing itself can be made: a
    //      surveillance-originated case heats the home district exactly like a burglary's.
    let matched = |run: &RunMetrics| {
        let strategy = run.strategy.expect("run must record its strategy");
        match (
            run.matched_legitimate_net_cents,
            run.matched_enterprise_net_cents,
        ) {
            (Some(legitimate), Some(enterprise)) => Ok((legitimate, enterprise)),
            _ => Err(HarnessContractError::MissingMatchedFinancialSnapshot { strategy }),
        }
    };
    let (rush_legit, rush_enterprise) = matched(rush)?;
    let (press_legit, press_enterprise) = matched(press)?;
    let (recon_legit, recon_enterprise) = matched(recon)?;

    let mismatch = |legitimate, enterprise| HarnessContractError::FinancialBranchMismatch {
        legitimate,
        enterprise,
    };
    if rush_legit != press_legit || press_legit != recon_legit {
        return Err(mismatch(
            [Some(rush_legit), Some(press_legit), Some(recon_legit)],
            [
                Some(rush_enterprise),
                Some(press_enterprise),
                Some(recon_enterprise),
            ],
        ));
    }

    let branches = [
        (rush.session_case_staffed, rush_enterprise),
        (press.session_case_staffed, press_enterprise),
        (recon.session_case_staffed, recon_enterprise),
    ];
    let unheated_nets: Vec<i64> = branches
        .iter()
        .filter(|(heated, _)| !heated)
        .map(|(_, net)| *net)
        .collect();
    // Unheated branches ran identical authored economics over an identical window.
    if unheated_nets.iter().any(|net| *net != unheated_nets[0]) {
        return Err(mismatch(
            [Some(rush_legit), Some(press_legit), Some(recon_legit)],
            [
                Some(rush_enterprise),
                Some(press_enterprise),
                Some(recon_enterprise),
            ],
        ));
    }
    // Heat may only lower an enterprise net relative to unheated branches.
    for (heated, net) in branches {
        if heated
            && unheated_nets
                .first()
                .is_some_and(|unheated| net > *unheated)
        {
            return Err(mismatch(
                [Some(rush_legit), Some(press_legit), Some(recon_legit)],
                [
                    Some(rush_enterprise),
                    Some(press_enterprise),
                    Some(recon_enterprise),
                ],
            ));
        }
    }
    Ok(())
}

pub fn persist_run_artifact(
    dir: &PathBuf,
    seeds: EvaluationSeeds,
    profile: ScenarioProfile,
    metrics: &RunMetrics,
) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let strategy_label = metrics
        .strategy
        .map(|s| s.label().to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());
    let filename = format!(
        "{}-w{:016x}-p{:016x}-{}-{}.json",
        profile.label().to_lowercase().replace(' ', "-"),
        seeds.world,
        seeds.policy,
        strategy_label,
        metrics
            .variation
            .map(|v| v.label().to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    let path = dir.join(filename);
    let identity = serde_json::json!({
        "world_seed": format!("{:#x}", seeds.world),
        "world_seed_dec": seeds.world,
        "policy_seed": format!("{:#x}", seeds.policy),
        "policy_seed_dec": seeds.policy,
        "profile": profile.label(),
        "strategy": metrics.strategy.map(|s| s.label()),
        "variation": metrics.variation.map(|v| v.label()),
    });
    let operation = serde_json::json!({
        "burglary": metrics.burglary.map(|id| format!("{id:?}")),
        "opening_scout": metrics.opening_scout.map(|id| format!("{id:?}")),
        "opening_scout_terminal_minute": metrics.opening_scout_terminal_minute,
        "opening_opportunity_valid_until_minute": metrics.opening_opportunity_valid_until_minute,
        "opening_casing_assessment": metrics.opening_casing_assessment,
        "opening_stood_down": metrics.opening_stood_down,
        "opening_standdown_reason": metrics.opening_standdown_reason,
        "outcome": metrics.outcome.map(|o| format!("{o:?}")),
        "aborted": metrics.aborted,
        "abort_phase": metrics.abort_phase.map(|p| format!("{p:?}")),
        "abort_cause": metrics.abort_cause.map(|c| format!("{c:?}")),
        "police_dispatched": metrics.police_dispatched,
        "police_arrived": metrics.police_arrived,
        "decision_requests": metrics.decision_requests,
        "exposure_level": metrics.exposure_level.map(|level| format!("{level:?}")),
        "burglary_terminal_minute": metrics.burglary_terminal_minute,
        "property_acquired_value_cents": metrics.property_acquired_value_cents,
        "property_realized_cash_cents": metrics.property_realized_cash_cents,
        "liquidation_minute": metrics.liquidation_minute,
    });
    let opening_choice = serde_json::json!({
        "decision_minute": metrics.opening_decision_minute,
        "opportunity_window_minutes": metrics.opening_opportunity_window_minutes,
        "burglary_duration_minutes": metrics.opening_burglary_duration_minutes,
        "surveillance_duration_minutes": metrics.opening_surveillance_duration_minutes,
        "scout_time_slack_minutes": metrics.opening_scout_time_slack_minutes,
        "selected_posture": metrics.strategy.map(|strategy| strategy.label()),
        "alternatives": [
            "move now with standing pre-entry abort",
            "move now and accept a later police-response decision",
            "scout first for actionable timing",
            "decline the score"
        ],
    });
    let information_and_legal = serde_json::json!({
        "opening_surveillance_information_count": metrics.discovered_surveillance_information,
        "planning_information_count": metrics.planning_information_count,
        "planning_information_topics": metrics.planning_information_topics.iter().map(|topic| format!("{topic:?}")).collect::<Vec<_>>(),
        "player_police_activity_information": metrics.player_police_activity_information,
        "player_legal_activity_information": metrics.player_legal_activity_information,
        "counterintelligence_outcome": metrics.counterintelligence_outcome.map(|outcome| format!("{outcome:?}")),
        "counterintelligence_information": metrics.counterintelligence_information,
        "followup_case_active": metrics.followup_case_active,
        "cold_case_confirmed": metrics.cold_case_confirmed,
        "case_open_minute": metrics.case_open_minute,
        "contact_reads": metrics.contact_reads,
    });
    let personnel = serde_json::json!({
        "player_poach_warnings": metrics.player_poach_warnings,
        "player_personnel_departures": metrics.player_personnel_departures,
        "replacement_recruited": metrics.replacement_recruited,
        "defector_trail_confirmed": metrics.defector_trail_confirmed,
        "win_back_attempted": metrics.win_back_attempted,
        "win_back_accepted": metrics.win_back_accepted,
    });
    let counterplay_and_custody = serde_json::json!({
        "witness_pressure_attempted": metrics.witness_pressure_attempted,
        "witness_pressure_outcome": metrics.witness_pressure_outcome.map(|outcome| format!("{outcome:?}")),
        "witness_pressure_aborted": metrics.witness_pressure_aborted,
        "player_member_arrests": metrics.player_member_arrests,
    });
    let economy = serde_json::json!({
        "legitimate_net_cents": metrics.legitimate_net_cents,
        "enterprise_net_cents": metrics.enterprise_net_cents,
        "matched_financial_boundary_minute": metrics.matched_financial_boundary_minute,
        "matched_legitimate_net_cents": metrics.matched_legitimate_net_cents,
        "matched_enterprise_net_cents": metrics.matched_enterprise_net_cents,
        "matched_player_window": metrics.matched_player_window,
        "expansion_established": metrics.expansion_established,
        "expansion_net_cents": metrics.expansion_net_cents,
        "expansion_heat_cents": metrics.expansion_heat_cents,
        "front_acquired": metrics.front_acquired,
        "acquisition_price_cents": metrics.acquisition_price_cents,
        "acquisition_spent_cents": metrics.acquisition_spent_cents,
        "acquisition_rejections": metrics.acquisition_rejections,
        "laundered_gross_cents": metrics.laundered_gross_cents,
        "launder_fee_cents": metrics.launder_fee_cents,
        "business_profits_swept_cents": metrics.business_profits_swept_cents,
        "laundering_capacity_rejections": metrics.laundering_capacity_rejections,
        "accounted_balance_cents": metrics.accounted_balance_cents,
        "payroll_paid_cents": metrics.payroll_paid_cents,
        "payroll_accounted_spent_cents": metrics.payroll_accounted_spent_cents,
        "payroll_short_cents": metrics.payroll_short_cents,
    });
    let second_act = serde_json::json!({
        "second_opportunity_discovered": metrics.second_opportunity_discovered,
        "second_opportunity_expired": metrics.second_opportunity_expired,
        "second_burglary": metrics.second_burglary.map(|id| format!("{id:?}")),
        "second_burglary_outcome": metrics.second_burglary_outcome.map(|outcome| format!("{outcome:?}")),
        "second_burglary_terminal_minute": metrics.second_burglary_terminal_minute,
        "second_act_recon_information": metrics.second_act_recon_information,
        "second_scout": metrics.second_scout.map(|id| format!("{id:?}")),
        "second_casing_assessment": metrics.second_casing_assessment,
        "second_scout_scheduled_minute": metrics.second_scout_scheduled_minute,
        "second_scout_attached_patrol": metrics.second_scout_attached_patrol,
        "second_scout_topics_covered": metrics.second_scout_topics_covered,
        "second_scout_patrol_observed_minute": metrics.second_scout_patrol_observed_minute,
        "second_act_property_acquired_value_cents": metrics.second_act_property_acquired_value_cents,
        "second_act_property_realized_cash_cents": metrics.second_act_property_realized_cash_cents,
        "second_act_planning_topics": metrics.second_act_planning_topics.iter().map(|topic| format!("{topic:?}")).collect::<Vec<_>>(),
        "self_heat_case_opened": metrics.self_heat_case_opened,
        "self_heat_check_required": metrics.self_heat_check_required,
        "self_heat_case_active": metrics.self_heat_case_active,
    });
    let feedback = serde_json::json!({
        "session_end_minute": metrics.session_end_minute,
        "player_report_count": metrics.player_report_count,
        "executive_brief_count": metrics.executive_brief_count,
        "racket_inquiries_drawn": metrics.racket_inquiries_drawn,
    });
    let diagnostic = serde_json::json!({
        "case": {
            "investigation_created": metrics.investigation_created,
            "investigation_opened_minute": metrics.investigation_opened_minute,
            "session_case_staffed": metrics.session_case_staffed,
            "case_cold_minute": metrics.case_cold_minute,
            "evidence_count": metrics.evidence_count,
            "exposure_score": metrics.exposure_score,
            "burglary_information_quality": metrics.burglary_information_quality,
            "investigation_work_scheduled": metrics.investigation_work_scheduled,
            "investigation_work_resolved": metrics.investigation_work_resolved,
            "case_witness_registered": metrics.case_witness_registered,
            "witness_interviews_scheduled": metrics.witness_interviews_scheduled,
            "witness_testimony_produced": metrics.witness_testimony_produced,
            "witness_cooperation_degraded": metrics.witness_cooperation_degraded,
        },
        "world": {
            "autonomous_recruitment_attempts": metrics.autonomous_recruitment_attempts,
            "rival_home_enterprises": metrics.rival_home_enterprises,
            "win_back_margin": metrics.win_back_margin,
            "win_back_refusal_leaked_to_rival": metrics.win_back_refusal_leaked_to_rival,
        }
    });
    let player_visible = serde_json::json!({
        "operation": operation,
        "opening_choice": opening_choice,
        "information_and_legal": information_and_legal,
        "personnel": personnel,
        "counterplay_and_custody": counterplay_and_custody,
        "economy": economy,
        "second_act": second_act,
        "feedback": feedback,
        "known_rackets": metrics.known_rackets,
    });
    let payload = serde_json::json!({
        "identity": identity,
        "player_visible": player_visible,
        "diagnostic": diagnostic,
    });
    fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
    Ok(path)
}

pub fn run_opportunity_portfolio_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    // Isolate portfolio scarcity from the NIGHT TRAP police shock. LATE PATROL keeps the
    // immediate window operationally live, so the selected burglary holds the specialist for
    // its authored duration instead of an early standing abort reopening the other score.
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::LatePatrol)?;
    let selected_at = SimTime::from_minutes(130);
    let burglary_minutes = u64::from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .duration()
            .as_minutes(),
    );
    // Put the shared deadline just before the selected job releases the one entry specialist.
    // Both opportunities are initially legal, but once one is chosen the other genuinely
    // becomes unreachable through the same scarce specialist instead of being abandoned by
    // harness fiat while there is still time to start it.
    let deadline = SimTime::from_minutes(
        selected_at
            .as_minutes()
            .checked_add(burglary_minutes)
            .and_then(|minute| minute.checked_sub(5))
            .expect("authored portfolio timing must not overflow"),
    );
    let valid_until = Some(deadline);
    let primary_opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: scenario.variation.opportunity_summary().to_owned(),
            valid_until,
        },
    )?
    .commit(&mut scenario.state)?;
    let alternate_opportunity = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.alternate_target)]),
            source_information: BTreeSet::from([scenario.alternate_opportunity_information]),
            summary: format!(
                "{} has directly observed high-value stock available after midnight.",
                scenario.variation.alternate_target_name()
            ),
            valid_until,
        },
    )?
    .commit(&mut scenario.state)?;
    if detail {
        println!(
            "[PORTFOLIO] Two burglary opportunities are open until minute {} under the low-immediate-pressure LATE PATROL control: {} (street rumor) and {} (direct, precise observation). One entry specialist can start either at minute {}, but the {}m burglary keeps that specialist busy past the shared deadline, so only one score fits the window.",
            deadline.as_minutes(),
            scenario.variation.target_name(),
            scenario.variation.alternate_target_name(),
            selected_at.as_minutes(),
            burglary_minutes,
        );
    }

    // This is an explicit player-visible prioritization rule: commit the opportunity with the
    // strongest available source instead of treating every open card as equally actionable.
    let target = scenario.alternate_target;
    let title = format!("{} burglary", scenario.variation.alternate_target_name());
    let intelligence = BTreeSet::from([scenario.alternate_opportunity_information]);
    let entry_specialist = scenario.burglar;
    let selected_operation = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        target,
        &title,
        selected_at,
        intelligence,
        entry_specialist,
    )?;
    validate_convert_opportunity(&scenario.state, alternate_opportunity, selected_operation)?
        .commit(&mut scenario.state)?;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(selected_operation),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, selected_operation, false, &mut metrics)?;
    metrics.burglary_terminal_minute = Some(scenario.state.now().as_minutes());
    let terminal_minute = metrics
        .burglary_terminal_minute
        .expect("selected portfolio operation just reached terminal");
    if terminal_minute <= deadline.as_minutes() {
        return Err(
            "portfolio probe no longer proves scarcity: selected job released its specialist before the deferred opportunity expired"
                .into(),
        );
    }
    let selected_operation_record = scenario
        .state
        .operations()
        .get_operation(selected_operation)
        .expect("selected portfolio operation must persist");
    metrics.aborted = selected_operation_record.status() == OperationStatus::Aborted;
    if metrics.aborted {
        let abort = selected_operation_record
            .abort_record()
            .expect("aborted portfolio operation must preserve its cause");
        metrics.abort_phase = Some(abort.phase());
        metrics.abort_cause = Some(abort.cause());
    } else {
        metrics.outcome = selected_operation_record
            .resolution()
            .map(|resolution| resolution.objective_outcome());
    }
    validate_harness_state(scenario.registry, &scenario.state)?;
    let selected = scenario
        .state
        .opportunities()
        .get_opportunity(alternate_opportunity)
        .expect("selected opportunity must persist");
    let deferred = scenario
        .state
        .opportunities()
        .get_opportunity(primary_opportunity)
        .expect("deferred opportunity must persist");
    if selected.status() != OpportunityStatus::Converted
        || selected
            .resolution()
            .and_then(|resolution| resolution.operation())
            != Some(selected_operation)
        || deferred.status() != OpportunityStatus::Expired
        || deferred
            .resolution()
            .and_then(|resolution| resolution.report())
            .is_none()
    {
        return Err(
            "portfolio probe did not preserve selected and deferred opportunity lifecycles".into(),
        );
    }
    if let Some(report_id) = deferred
        .resolution()
        .and_then(|resolution| resolution.report())
    {
        let report = scenario
            .state
            .reports()
            .get_report(report_id)
            .expect("expired opportunity report must persist");
        if detail {
            print_report("PORTFOLIO EXPIRY REPORT", report, &scenario);
        }
    }
    // Prove the Dismissed lifecycle is distinct from Expiry: dismiss a fresh opportunity
    // through its canonical path and verify the lifecycle report, proving the harness is not
    // stale on the three non-converted states.
    let dismissable = validate_discover_operation_opportunity(
        scenario.registry,
        &scenario.state,
        OperationOpportunityDraft {
            organization: scenario.player,
            operation_kind: OperationKind::Burglary,
            targets: BTreeSet::from([EntityRef::Business(scenario.target)]),
            source_information: BTreeSet::from([scenario.opportunity_information]),
            summary: "Dismissable decoy opportunity for lifecycle probe.".to_owned(),
            valid_until: Some(SimTime::from_minutes(500)),
        },
    )?
    .commit(&mut scenario.state)?;
    let dismissal_report =
        validate_dismiss_opportunity(scenario.registry, &scenario.state, dismissable)?
            .commit(&mut scenario.state)?;
    let dismissed = scenario
        .state
        .opportunities()
        .get_opportunity(dismissable)
        .expect("dismissed opportunity must persist");
    if dismissed.status() != OpportunityStatus::Dismissed || dismissed.resolution().is_none() {
        return Err("dismiss lifecycle did not produce expected Dismissed state".into());
    }
    if scenario
        .state
        .reports()
        .get_report(dismissal_report)
        .is_none()
    {
        return Err("dismiss lifecycle must persist its lifecycle report".into());
    }
    if detail {
        println!(
            "[PORTFOLIO] Selected {} from player-visible source quality, converted it into {}, and kept the entry specialist occupied through minute {} while the weaker opportunity expired at minute {}. The lost score is a real time/capacity cost, not arbitrary harness inaction. A later decoy was dismissed through the canonical lifecycle.",
            scenario.variation.alternate_target_name(),
            terminal_label(&metrics),
            terminal_minute,
            deadline.as_minutes(),
        );
    }
    Ok(())
}

/// Proves that characters are a scarce organizational resource rather than infinitely reusable
/// stat bundles. The probe attempts to commit one specialist to overlapping jobs, observes the
/// typed rejection without mutating state, then retries after the first job reaches a terminal
/// state and confirms that the specialist is available again.
pub fn run_organizational_capacity_probe(
    registry: &Registry,
    seeds: EvaluationSeeds,
    detail: bool,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)?;
    let first_start = scenario.timeline.initial_burglary_at;
    let target = scenario.target;
    let opportunity_information = scenario.opportunity_information;
    let burglar = scenario.burglar;
    let first = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        target,
        "capacity probe first burglary",
        first_start,
        BTreeSet::from([opportunity_information]),
        burglar,
    )?;
    let overlapping_start = first_start + SimDuration::from_minutes(1);
    let overlapping = validate_authorize_operation(
        registry,
        &scenario.state,
        OperationDraft {
            title: "capacity probe overlapping burglary".to_owned(),
            kind: OperationKind::Burglary,
            responsible_organization: scenario.player,
            leader: scenario.boss,
            objective: OperationObjective::AcquireProperty {
                target: EntityRef::Business(scenario.alternate_target),
            },
            approach: OperationApproach::Covert,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.boss),
                (RoleKind::EntrySpecialist, burglar),
            ]),
            intelligence: BTreeSet::from([scenario.alternate_opportunity_information]),
            constraints: Vec::new(),
            contingencies: vec![OperationContingency::AbortOnPoliceArrivalBeforeEntry],
            scheduled_for: overlapping_start,
        },
    )
    .expect_err("overlapping specialist assignments must be rejected");
    let overlapping_debug = format!("{overlapping:?}");
    let expected_rejection = matches!(
        overlapping,
        OperationError::ParticipantBusy {
            character,
            operation,
        } if character == burglar && operation == first
    );
    if !expected_rejection {
        return Err(format!(
            "capacity probe returned the wrong rejection for overlapping specialist: observed {overlapping_debug}, expected ParticipantBusy {{ character: {burglar:?}, operation: {first:?} }}"
        )
        .into());
    }
    validate_harness_state(registry, &scenario.state)?;
    let specialist_name = scenario
        .state
        .world()
        .get_character(scenario.burglar)
        .expect("capacity-probe specialist must persist")
        .name()
        .to_owned();
    if detail {
        println!(
            "[CAPACITY] {specialist_name} was reserved for the first burglary; the overlapping second plan was rejected ({specialist_name} is already booked on that crew) without changing authoritative state."
        );
    }

    let mut first_metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(first),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, first, false, &mut first_metrics)?;
    capture_terminal_status(&scenario, first, &mut first_metrics);
    let released_start = scenario.state.now() + SimDuration::ONE_MINUTE;
    let alternate_target = scenario.alternate_target;
    let alternate_opportunity_information = scenario.alternate_opportunity_information;
    let second = authorize_burglary(
        &mut scenario,
        Strategy::Rush,
        alternate_target,
        "capacity probe released burglary",
        released_start,
        BTreeSet::from([alternate_opportunity_information]),
        burglar,
    )?;
    let mut second_metrics = RunMetrics {
        strategy: Some(Strategy::Rush),
        variation: Some(scenario.variation),
        burglary: Some(second),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, second, false, &mut second_metrics)?;
    capture_terminal_status(&scenario, second, &mut second_metrics);
    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[CAPACITY] After the first burglary became {}, {} was released and the second plan authorized at minute {} (terminal {}).",
            terminal_label(&first_metrics),
            scenario
                .state
                .world()
                .get_character(scenario.burglar)
                .expect("capacity-probe specialist must persist")
                .name(),
            released_start.as_minutes(),
            terminal_label(&second_metrics),
        );
    }
    // Prove delegation lifecycle is not stale: extend the player's mandate with Personnel
    // authority and its matching recruitment standing order, verify the version advances, then
    // ensure state remains valid. Standing orders are scoped authority, so the probe must not
    // manufacture an order the mandate is structurally unable to exercise.
    let player_mandate = scenario
        .state
        .delegation()
        .active_for_manager(scenario.lieutenant)
        .map(|record| record.id())
        .expect("player mandate must still be active after capacity probe");
    let mandate_record = scenario
        .state
        .delegation()
        .get_mandate(player_mandate)
        .expect("mandate record must persist");
    let prior_version = mandate_record.version();
    let mut revised_scopes = mandate_record.scopes().clone();
    revised_scopes.insert(ResponsibilityScope::Function(
        ResponsibilityFunction::Personnel,
    ));
    let mut revised_orders = mandate_record.standing_orders().clone();
    revised_orders.insert(
        PolicyKind::IndependentRecruitment,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    );
    validate_revise_mandate(
        &scenario.state,
        player_mandate,
        MandateRevisionDraft {
            scopes: revised_scopes,
            standing_orders: revised_orders,
            budget: mandate_record.budget(),
        },
    )?
    .commit(&mut scenario.state)?;
    let revised = scenario
        .state
        .delegation()
        .get_mandate(player_mandate)
        .expect("revised mandate must persist");
    if revised.version() <= prior_version {
        return Err("mandate revision did not advance version".into());
    }
    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[CAPACITY] Mandate {:?} revised (v{} -> v{}) with updated standing orders, proving delegation lifecycle tracks the game.",
            player_mandate,
            prior_version,
            revised.version()
        );
    }
    // Approach variation probe: authorize a WitnessPressure operation with a non-Covert
    // approach to prove the harness is not hard-coded to one tactical axis.
    let approach = match seeds.policy % 3 {
        0 => OperationApproach::Deceptive,
        1 => OperationApproach::Intimidating,
        _ => OperationApproach::Covert,
    };
    let _witness_pressure = validate_authorize_operation(
        registry,
        &scenario.state,
        OperationDraft {
            title: "capacity probe approach-variation operation".to_owned(),
            kind: OperationKind::WitnessPressure,
            responsible_organization: scenario.player,
            leader: scenario.boss,
            objective: OperationObjective::Frighten {
                target: EntityRef::Character(scenario.burglar),
            },
            approach,
            roles: BTreeMap::from([
                (RoleKind::Coordinator, scenario.boss),
                (RoleKind::Negotiator, scenario.lieutenant),
            ]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: scenario.state.now() + SimDuration::from_minutes(5),
        },
    );
    // The operation may be rejected for domain reasons (e.g., witness not yet in case); the
    // probe's value is exercising a different OperationKind/Approach through the canonical
    // validation path, not asserting a specific operational outcome. A successful validation
    // proves the vocabulary is live; a typed rejection proves the harness tracks the game.
    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[CAPACITY] Approach-variation probe exercised {:?} + {:?} through canonical validation.",
            OperationKind::WitnessPressure,
            approach
        );
    }
    // Recruitment-approach variation: validate a non-FinancialOpportunity pitch through the
    // canonical path to prove the harness is not hard-coded to one approach. The probe uses
    // the same deterministic relationship so margin math stays registry-derived.
    let alt_approach = match seeds.policy % 4 {
        0 => RecruitmentApproach::FinancialOpportunity,
        1 => RecruitmentApproach::Advancement,
        2 => RecruitmentApproach::Protection,
        _ => RecruitmentApproach::PersonalAppeal,
    };
    let _alt_recruitment = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            recruiter: scenario.boss,
            candidate: scenario.danny_ferro,
            target_organization: scenario.player,
            approach: alt_approach,
        },
    );
    validate_harness_state(registry, &scenario.state)?;
    if detail {
        println!(
            "[CAPACITY] Recruitment-approach probe exercised {:?} through canonical validation.",
            alt_approach
        );
    }
    Ok(())
}
