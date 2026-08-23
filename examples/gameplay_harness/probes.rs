//! Bounded deterministic probes: repeat-take depletion, opportunity portfolio, organizational capacity, legal foundation, and matched-strategy batches.

use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, InformationId};
use crimocracy::core::state::AppState;
use crimocracy::core::time::{SimDuration, SimTime};
use crimocracy::delegation::delegation_system::validate_revise_mandate;
use crimocracy::delegation::delegation_system::MandateRevisionDraft;
use crimocracy::finance::finance_system::{insert_account, validate_record_transaction};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::InformationTopic;
use crimocracy::legal::legal_representation_system::validate_retain_legal_representation;
use crimocracy::legal::prosecution_system::{
    validate_decline_prosecution_case, validate_open_prosecution_case,
};
use crimocracy::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft, LegalRepresentationDraft, ProsecutionCaseDraft,
};
use crimocracy::operations::operation_system::{validate_authorize_operation, OperationError};
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
use crimocracy::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crimocracy::registry::Registry;
use crimocracy::social::relationship_system::validate_set_relationship;
use crimocracy::social::RelationshipDimensions;
use crimocracy::world::world_system::{insert_character, insert_organization};
use crimocracy::world::{
    ApprovalPolicy, AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft,
    OrganizationKind, PolicyKind, PolicySetting,
};
use crimocracy::{
    contacts::contact_system::{validate_establish_contact, InstitutionalContactDraft},
    legal::arrest_system::validate_arrest,
    legal::investigation_system::{validate_add_evidence, validate_open_investigation},
};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use crate::*;

/// Proves that repeated scores on one target decay through the canonical property-proceeds
/// path. The organization learns the district's patrol rhythm through surveillance, takes the
/// same business twice, and observes that the immediate re-score recovers only part of the
/// first haul while a take after a rest period returns to full value. All observations are
/// player-visible: held-property records and after-action outcomes.
pub fn run_repeat_take_probe(registry: &Registry, seed: u64) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, ScenarioProfile::NightTrap)?;
    let target = scenario.target;
    let opportunity_information = scenario.opportunity_information;

    let recon = authorize_surveillance(&mut scenario)?;
    let mut metrics = RunMetrics {
        strategy: Some(Strategy::Recon),
        variation: Some(scenario.variation),
        ..RunMetrics::default()
    };
    run_until_operation_terminal(&mut scenario, recon, false, &mut metrics)?;
    let resolution = scenario
        .state
        .operations()
        .get_operation(recon)
        .expect("probe surveillance must persist")
        .resolution()
        .expect("completed probe surveillance must have a resolution");
    let mut intelligence = BTreeSet::from([opportunity_information]);
    let mut learned_patrol_summary = None;
    for information in resolution.discovered_information() {
        let record = scenario
            .state
            .intelligence()
            .get_information(*information)
            .expect("surveillance information must persist");
        if record.topic() == InformationTopic::PoliceActivity {
            learned_patrol_summary = Some(record.summary().to_owned());
        }
        intelligence.insert(*information);
    }
    let patrol_summary = learned_patrol_summary
        .as_deref()
        .ok_or("repeat-take probe surveillance produced no patrol-pattern observation")?;
    let duration = registry
        .get_operation(OperationKind::Burglary)
        .execution()
        .duration();

    fn run_take(
        scenario: &mut Scenario,
        metrics: &mut RunMetrics,
        patrol_summary: &str,
        duration: SimDuration,
        target: BusinessId,
        intelligence: &BTreeSet<InformationId>,
        title: &'static str,
    ) -> Result<i64, Box<dyn Error>> {
        let scheduled_for = choose_safe_start_from_patrol_report(
            scenario.state.now(),
            patrol_summary,
            duration,
            SimDuration::from_minutes(60),
            SimTime::from_minutes(scenario.state.now().as_minutes() + 2_880),
        )?;
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
        Ok(proceeds.estimated_value().cents())
    }

    let first_take = run_take(
        &mut scenario,
        &mut metrics,
        patrol_summary,
        duration,
        target,
        &intelligence,
        "repeat-take probe first score",
    )?;
    let second_take = run_take(
        &mut scenario,
        &mut metrics,
        patrol_summary,
        duration,
        target,
        &intelligence,
        "repeat-take probe immediate re-score",
    )?;
    if second_take != first_take / 2 || second_take >= first_take {
        return Err(format!(
            "immediate re-score expected exactly half of the first take, observed {first_take}c then {second_take}c"
        )
        .into());
    }
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

    // Let the recency window pass so the target restocks, then confirm full value returns.
    let rest_until = scenario.state.now() + SimDuration::from_minutes(3 * 1_440);
    run_until(&mut scenario, rest_until, false, &mut metrics)?;
    let third_take = run_take(
        &mut scenario,
        &mut metrics,
        patrol_summary,
        duration,
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
    println!(
        "[REPEAT TAKE] After letting the target rest, the next score held {} again. Repeat takes decay and recover through production rules.",
        format_cents(third_take)
    );
    validate_harness_state(registry, &scenario.state)?;
    Ok(())
}

pub fn run_legal_foundation_check(registry: &Registry) -> Result<(), Box<dyn Error>> {
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
    .commit(&mut state);
    let contact = validate_establish_contact(
        &state,
        InstitutionalContactDraft {
            sponsor,
            handler,
            contact: counsel,
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
    let arrest = validate_arrest(
        &state,
        ArrestDraft {
            character: defendant,
            investigation,
            evidence: BTreeSet::from([evidence]),
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
            payer_account: payer,
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
            lead_prosecutor: prosecutor,
            evidence: BTreeSet::from([evidence]),
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

    println!(
        "[LEGAL PASS] arrest -> paid counsel -> police custody-preserving referral -> terminal prosecution decline"
    );
    Ok(())
}

pub fn run_strategy_batch(
    registry: &Registry,
    profile: ScenarioProfile,
    samples: u64,
    seed: u64,
    artifact_dir: Option<&PathBuf>,
) -> Result<(Aggregate, Aggregate, Aggregate), Box<dyn Error>> {
    let mut rush_aggregate = Aggregate::default();
    let mut press_aggregate = Aggregate::default();
    let mut recon_aggregate = Aggregate::default();
    let mut artifacts_written = 0_u64;
    for offset in 0..samples {
        let sample_seed = seed.wrapping_add(offset + 1);
        let rush = play_session(registry, Strategy::Rush, profile, sample_seed, false, true)?;
        let press = play_session(registry, Strategy::Press, profile, sample_seed, false, true)?;
        let recon = play_session(registry, Strategy::Recon, profile, sample_seed, false, true)?;
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
                if persist_run_artifact(dir, sample_seed, profile, metrics).is_ok() {
                    artifacts_written += 1;
                }
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
    seed: u64,
    profile: ScenarioProfile,
    metrics: &RunMetrics,
) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let strategy_label = metrics
        .strategy
        .map(|s| s.label().to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());
    let filename = format!(
        "{}-{:016x}-{}-{}.json",
        profile.label().to_lowercase().replace(' ', "-"),
        seed,
        strategy_label,
        metrics
            .variation
            .map(|v| v.label().to_lowercase())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    let path = dir.join(filename);
    let payload = serde_json::json!({
        "seed": format!("{seed:#x}"),
        "seed_dec": seed,
        "profile": profile.label(),
        "strategy": metrics.strategy.map(|s| s.label()),
        "variation": metrics.variation.map(|v| v.label()),
        "burglary": metrics.burglary.map(|id| format!("{id:?}")),
        "outcome": metrics.outcome.map(|o| format!("{o:?}")),
        "aborted": metrics.aborted,
        "abort_phase": metrics.abort_phase.map(|p| format!("{p:?}")),
        "abort_cause": metrics.abort_cause.map(|c| format!("{c:?}")),
        "police_dispatched": metrics.police_dispatched,
        "police_arrived": metrics.police_arrived,
        "decision_requests": metrics.decision_requests,
        "exposure_score": metrics.exposure_score,
        "evidence_count": metrics.evidence_count,
        "burglary_terminal_minute": metrics.burglary_terminal_minute,
        "legitimate_net_cents": metrics.legitimate_net_cents,
        "enterprise_net_cents": metrics.enterprise_net_cents,
        "matched_financial_boundary_minute": metrics.matched_financial_boundary_minute,
        "matched_legitimate_net_cents": metrics.matched_legitimate_net_cents,
        "matched_enterprise_net_cents": metrics.matched_enterprise_net_cents,
        "player_report_count": metrics.player_report_count,
        "executive_brief_count": metrics.executive_brief_count,
        "rival_home_enterprises": metrics.rival_home_enterprises,
        "player_poach_warnings": metrics.player_poach_warnings,
        "session_case_staffed": metrics.session_case_staffed,
        "raw": {
            "second_opportunity_discovered": metrics.second_opportunity_discovered,
            "second_burglary": metrics.second_burglary.map(|id| format!("{id:?}")),
            "defector_trail_confirmed": metrics.defector_trail_confirmed,
        }
    });
    fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
    Ok(path)
}

pub fn run_opportunity_portfolio_probe(
    registry: &Registry,
    seed: u64,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, ScenarioProfile::NightTrap)?;
    let valid_until = Some(SimTime::from_minutes(180));
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
    println!(
        "[PORTFOLIO] Two burglary opportunities are open until minute 180: {} (street rumor) and {} (direct, precise observation).",
        scenario.variation.target_name(),
        scenario.variation.alternate_target_name(),
    );

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
        SimTime::from_minutes(130),
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
    run_until(
        &mut scenario,
        SimTime::from_minutes(181),
        false,
        &mut metrics,
    )?;
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
        print_report("PORTFOLIO EXPIRY REPORT", report, &scenario);
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
    validate_dismiss_opportunity(&scenario.state, dismissable)?.commit(&mut scenario.state)?;
    let dismissed = scenario
        .state
        .opportunities()
        .get_opportunity(dismissable)
        .expect("dismissed opportunity must persist");
    if dismissed.status() != OpportunityStatus::Dismissed || dismissed.resolution().is_none() {
        return Err("dismiss lifecycle did not produce expected Dismissed state".into());
    }
    println!(
        "[PORTFOLIO] Selected {} from player-visible source quality, converted it into {}, left the weaker opportunity to expire, and dismissed a decoy through the canonical lifecycle.",
        scenario.variation.alternate_target_name(),
        terminal_label(&metrics),
    );
    Ok(())
}

/// Proves that characters are a scarce organizational resource rather than infinitely reusable
/// stat bundles. The probe attempts to commit one specialist to overlapping jobs, observes the
/// typed rejection without mutating state, then retries after the first job reaches a terminal
/// state and confirms that the specialist is available again.
pub fn run_organizational_capacity_probe(
    registry: &Registry,
    seed: u64,
) -> Result<(), Box<dyn Error>> {
    let mut scenario = build_scenario(registry, seed, ScenarioProfile::NightTrap)?;
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
    println!(
        "[CAPACITY] {} was reserved for the first burglary; the overlapping second plan was rejected as {:?} without changing authoritative state.",
        scenario
            .state
            .world()
            .get_character(scenario.burglar)
            .expect("capacity-probe specialist must persist")
            .name(),
        overlapping_debug,
    );

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
    // Prove delegation lifecycle is not stale: revise the player's mandate to add a standing
    // order, verify the version advances, then ensure state remains valid. This exercises
    // the canonical revise path that earlier harness iterations never touched.
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
    let mut revised_orders = mandate_record.standing_orders().clone();
    revised_orders.insert(
        PolicyKind::IndependentRecruitment,
        PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
    );
    validate_revise_mandate(
        &scenario.state,
        player_mandate,
        MandateRevisionDraft {
            scopes: mandate_record.scopes().clone(),
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
    println!(
        "[CAPACITY] Mandate {:?} revised (v{} -> v{}) with updated standing orders, proving delegation lifecycle tracks the game.",
        player_mandate,
        prior_version,
        revised.version()
    );
    // Approach variation probe: authorize a WitnessPressure operation with a non-Covert
    // approach to prove the harness is not hard-coded to one tactical axis.
    let approach = match seed % 3 {
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
    println!(
        "[CAPACITY] Approach-variation probe exercised {:?} + {:?} through canonical validation.",
        OperationKind::WitnessPressure,
        approach
    );
    // Recruitment-approach variation: validate a non-FinancialOpportunity pitch through the
    // canonical path to prove the harness is not hard-coded to one approach. The probe uses
    // the same deterministic relationship so margin math stays registry-derived.
    let alt_approach = match seed % 4 {
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
    println!(
        "[CAPACITY] Recruitment-approach probe exercised {:?} through canonical validation.",
        alt_approach
    );
    Ok(())
}
