//! Player-facing printouts: labels, financial views, reports, and experience readouts.

use crimocracy::core::attention::AttentionClass;
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, EnterpriseId, OperationId};
use crimocracy::core::time::SimTime;
use crimocracy::economy::business_reporting::resolve_organization_business_financial_summary;
use crimocracy::enterprises::EnterpriseLocation;
use crimocracy::finance::{AccountKind, FinancialOwner, Money};
use crimocracy::intelligence::{InformationTopic, KnowledgeHolder};
use crimocracy::operations::{
    OperationAbortCause, OperationAbortPhase, OperationExposureLevel, OperationObjectiveBlocker,
    OperationObjectiveOutcome,
};
use crimocracy::reports::{ReportKind, ReportRecord};
use crimocracy::world::{CapabilityKind, OrganizationKind, Rating};
use std::collections::BTreeMap;
use std::error::Error;

use crate::*;

pub fn print_second_act_recap(scenario: &Scenario, strategy: Strategy, metrics: &RunMetrics) {
    let target = scenario.variation.alternate_target_name();
    match strategy {
        Strategy::Rush | Strategy::Recon => {
            if strategy == Strategy::Recon
                && metrics.self_heat_check_required
                && metrics.self_heat_case_active != Some(false)
                && metrics.second_burglary.is_none()
            {
                let lapsed_at = metrics
                    .second_opportunity
                    .and_then(|opportunity| {
                        scenario.state.opportunities().get_opportunity(opportunity)
                    })
                    .and_then(|record| record.resolution())
                    .map(|resolution| resolution.at().as_minutes().to_string())
                    .unwrap_or_else(|| "-".to_owned());
                let case_read = if metrics.self_heat_case_active == Some(true) {
                    "the police contact confirmed it remained active"
                } else {
                    "the police contact could not give a dependable clearing read"
                };
                println!(
                    "\n[ACT 2] {target} second score lapsed at minute {lapsed_at}: fresh surveillance reported exposure, {case_read}, and RECON declined to compound the heat."
                );
                println!(
                    "[ACT 2] Re-plan evidence: fresh surveillance produced {} information item(s); its legal consequence changed the decision before another burglary was authorized.",
                    metrics.second_act_recon_information
                );
                return;
            }
            let outcome = metrics
                .second_burglary_outcome
                .map(|outcome| format!("{outcome:?}"))
                .unwrap_or_else(|| "no resolution".to_owned());
            let realized = optional_dollars(metrics.second_act_property_realized_cash_cents);
            println!(
                "\n[ACT 2] {target} second score: {} at minute {}, liquidating {}.",
                outcome,
                metrics
                    .second_burglary_terminal_minute
                    .map(|minute| minute.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                realized
            );
            if strategy == Strategy::Rush {
                if metrics.replacement_recruited {
                    println!(
                        "[ACT 2] Rebuild evidence: replacement recruited through executive recruitment; no fresh recon was used; the retry moved away from the failed overnight hour and carried {} planning topic(s), including the debriefed police-response observation.",
                        metrics.second_act_planning_topics.len()
                    );
                } else {
                    println!(
                        "[ACT 2] Crew evidence: the win-back restored the original crew before the second score; no fresh recon was used; the retry moved away from the failed overnight hour and carried {} planning topic(s), including the debriefed police-response observation.",
                        metrics.second_act_planning_topics.len()
                    );
                }
            } else {
                println!(
                    "[ACT 2] Re-plan evidence: fresh surveillance produced {} information item(s); no uncleared exposure check blocked the burglary, which used a patrol-safe window.",
                    metrics.second_act_recon_information
                );
            }
        }
        Strategy::Press => {
            let lapsed_at = metrics
                .second_opportunity
                .and_then(|opportunity| scenario.state.opportunities().get_opportunity(opportunity))
                .and_then(|record| record.resolution())
                .map(|resolution| resolution.at().as_minutes().to_string())
                .unwrap_or_else(|| "-".to_owned());
            println!(
                "\n[ACT 2] {target} second score deliberately lapsed at minute {lapsed_at} while the case stayed hot; the standing-down cost the organization the value it refused to risk."
            );
        }
    }
}

pub fn print_starting_player_view(scenario: &Scenario) {
    println!("[ORGANIZATION] Marrow Organization");
    println!(
        "  (Capabilities show as values; traits and drives are what the organization knows about its own people and its personal contacts. Recruitment pitches land when the approach matches what the candidate wants.)"
    );
    for character in [
        scenario.boss,
        scenario.lieutenant,
        scenario.burglar,
        scenario.scout,
    ] {
        let record = scenario
            .state
            .world()
            .get_character(character)
            .expect("scenario character must exist");
        let traits = crimocracy::world::ALL_TRAIT_KINDS
            .iter()
            .filter(|kind| record.has_trait(**kind))
            .map(|kind| format!("{kind:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let drives = crimocracy::world::ALL_DRIVE_KINDS
            .iter()
            .filter_map(|kind| {
                record
                    .drive(*kind)
                    .map(|rating| format!("{kind:?} {}", rating.value()))
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  - {:<14} autonomy {:?}; management {:?}; burglary {:?}; surveillance {:?}; stealth {:?}",
            record.name(),
            record.autonomy(),
            record
                .capability(CapabilityKind::Management)
                .map(Rating::value),
            record
                .capability(CapabilityKind::Burglary)
                .map(Rating::value),
            record
                .capability(CapabilityKind::Surveillance)
                .map(Rating::value),
            record
                .capability(CapabilityKind::Stealth)
                .map(Rating::value),
        );
        println!(
            "      traits [{}]; drives [{}]",
            if traits.is_empty() {
                "-".to_owned()
            } else {
                traits
            },
            if drives.is_empty() {
                "-".to_owned()
            } else {
                drives
            },
        );
    }
    println!(
        "[WORLD] In {}: player fronts {} and {}; the target is {}; {} holds jurisdiction; two rivals operate: {} and {}.",
        scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("neighborhood must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("front must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.resale_venue)
            .expect("resale venue must exist")
            .name(),
        scenario
            .state
            .world()
            .get_business(scenario.target)
            .expect("target must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.police)
            .expect("police must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.rival)
            .expect("rival must exist")
            .name(),
        scenario
            .state
            .world()
            .get_organization(scenario.second_rival)
            .expect("second rival must exist")
            .name(),
    );
    println!(
        "[DELEGATION] Carlo manages a gambling enterprise at {}; routine cycles are delegated.",
        scenario
            .state
            .world()
            .get_business(scenario.front)
            .expect("front must exist")
            .name(),
    );
    let (contact_name, handler_name) = {
        let record = scenario
            .state
            .contacts()
            .get_contact(scenario.police_contact)
            .expect("police contact must persist");
        (
            scenario
                .state
                .world()
                .get_character(record.contact())
                .expect("contact character must exist")
                .name()
                .to_owned(),
            scenario
                .state
                .world()
                .get_character(record.handler())
                .expect("handler character must exist")
                .name()
                .to_owned(),
        )
    };
    let police_name = scenario
        .state
        .world()
        .get_organization(scenario.police)
        .expect("police organization must exist")
        .name()
        .to_owned();
    let detective = scenario
        .state
        .world()
        .get_character(scenario.detective)
        .expect("detective must exist");
    println!(
        "[STATE] {} is available to {police_name} with Investigation {}.",
        detective.name(),
        detective
            .capability(CapabilityKind::Investigation)
            .expect("detective must have investigation capability")
            .value(),
    );
    println!(
        "[STATE] {handler_name} keeps a standing Police-channel contact with {contact_name} inside {police_name}; a quiet word costs no street exposure."
    );
    // Counsel posture from contacts leadership actually holds: an arrest without a
    // legal channel means scrambling for representation under custody pressure, so a
    // boss should know before the cuffs whether that channel exists.
    {
        let has_legal_contact = scenario
            .state
            .contacts()
            .contacts_for_sponsor(scenario.player)
            .any(|record| {
                scenario
                    .state
                    .world()
                    .get_character(record.contact())
                    .and_then(|character| character.organization())
                    .and_then(|organization| scenario.state.world().get_organization(organization))
                    .is_some_and(|organization| {
                        organization.kind() == OrganizationKind::LegalServices
                    })
            });
        if has_legal_contact {
            println!(
                "[STATE] The organization holds a legal channel: counsel can be retained through production representation if anyone is taken."
            );
        } else {
            println!(
                "[STATE] No standing legal contact exists: an arrest would force direct retention from scratch while the detainee's one-time cooperation decision ticks. A single job rarely carries custody on its own - repeated heat is what builds a file - but leadership has no counsel lined up."
            );
        }
    }
    let replacement = scenario
        .state
        .world()
        .get_character(scenario.danny_ferro)
        .expect("replacement candidate must exist");
    let replacement_traits = crimocracy::world::ALL_TRAIT_KINDS
        .iter()
        .filter(|kind| replacement.has_trait(**kind))
        .map(|kind| format!("{kind:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let replacement_drives = crimocracy::world::ALL_DRIVE_KINDS
        .iter()
        .filter_map(|kind| {
            replacement
                .drive(*kind)
                .map(|rating| format!("{kind:?} {}", rating.value()))
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "[STATE] {} is an independent with Burglary {} / Stealth {}; traits [{}]; drives [{}]. Marrow holds a personal relationship with him, so he is the fallback entry specialist if the current crew is lost. A Money-driven, Greedy candidate answers a FinancialOpportunity pitch; a frightened, Safety-driven crew member answers Protection.",
        replacement.name(),
        replacement
            .capability(CapabilityKind::Burglary)
            .expect("replacement must have burglary capability")
            .value(),
        replacement
            .capability(CapabilityKind::Stealth)
            .expect("replacement must have stealth capability")
            .value(),
        if replacement_traits.is_empty() {
            "-".to_owned()
        } else {
            replacement_traits
        },
        if replacement_drives.is_empty() {
            "-".to_owned()
        } else {
            replacement_drives
        },
    );
    let burglar_record = scenario
        .state
        .world()
        .get_character(scenario.burglar)
        .expect("burglar must exist");
    let burglar_traits = crimocracy::world::ALL_TRAIT_KINDS
        .iter()
        .filter(|kind| burglar_record.has_trait(**kind))
        .map(|kind| format!("{kind:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let burglar_drives = crimocracy::world::ALL_DRIVE_KINDS
        .iter()
        .filter_map(|kind| {
            burglar_record
                .drive(*kind)
                .map(|rating| format!("{kind:?} {}", rating.value()))
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "[STATE] {} carries traits [{}] and drives [{}]; if police exposure ever makes him a poaching target, leadership will need the approach that speaks to what he fears and wants, not a random pitch.",
        burglar_record.name(),
        if burglar_traits.is_empty() {
            "-".to_owned()
        } else {
            burglar_traits
        },
        if burglar_drives.is_empty() {
            "-".to_owned()
        } else {
            burglar_drives
        },
    );
}

pub fn print_planning_inputs(scenario: &Scenario, operation: OperationId) {
    let record = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("planning operation must persist");
    for information_id in record.intelligence() {
        let information = scenario
            .state
            .intelligence()
            .get_information(*information_id)
            .expect("selected planning information must persist");
        println!(
            "[PLAN INPUT] {:?} ({:?}/{:?}): {}",
            information.topic(),
            information.reliability(),
            information.specificity(),
            information.summary(),
        );
    }
}

pub fn print_player_knowledge_gap(scenario: &Scenario, burglary: OperationId) {
    let operation = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must persist");
    if operation.resolution().is_some() {
        let legal_information: Vec<_> = scenario
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(scenario.player),
                InformationTopic::LegalActivity,
            )
            .filter(|information| information.subject() == EntityRef::Operation(burglary))
            .collect();
        // A clean resolution teaches nothing about the case file, so reporting a zero here
        // is noise. The gap matters only when the crew saw or left evidence that could
        // support a case: then leadership must learn it through a channel, not the debrief.
        let exposed = operation.resolution().is_some_and(|resolution| {
            matches!(
                resolution.exposure().level(),
                OperationExposureLevel::Witnessed | OperationExposureLevel::Identifying
            )
        });
        if legal_information.is_empty() && !exposed {
            return;
        }
        println!(
            "[KNOWLEDGE] Player organization has {} LegalActivity information record(s) about this burglary after resolution.",
            legal_information.len(),
        );
        for information in &legal_information {
            println!("  - [PLAYER] {}", information.summary());
        }
        if exposed && legal_information.is_empty() {
            println!(
                "  - [PLAYER] No case fact arrived with the resolution itself; any case knowledge must come through a contact or casing channel."
            );
        }
    }
}

/// The closing counterpart to the starting player view: what the organization actually looks
/// like after the session, assembled only from state a boss can see - roster, mandates,
/// holdings, and the reports the organization received.
pub fn print_organization_closing_view(
    scenario: &Scenario,
    metrics: &RunMetrics,
    financials: &FinancialView,
) {
    let members = scenario
        .state
        .world()
        .characters_in_organization(scenario.player)
        .map(|record| record.name().to_owned())
        .collect::<Vec<_>>();
    println!(
        "\n[ORGANIZATION NOW] {} member(s): {}",
        members.len(),
        members.join(", ")
    );
    if metrics.player_personnel_departures > 0 {
        println!(
            "  - Lost {} member(s) to rival recruitment this session{}{}",
            metrics.player_personnel_departures,
            if metrics.replacement_recruited {
                "; rebuilt through an executive recruitment".to_owned()
            } else if metrics.win_back_accepted == Some(true) {
                "; the departed member subsequently returned after a successful win-back".to_owned()
            } else {
                String::new()
            },
            if metrics.police_arrived {
                ". The departed crew saw police at the score, and fear made the outside offer land where loyalty might otherwise have held".to_owned()
            } else {
                String::new()
            },
        );
    }
    for business in scenario
        .state
        .world()
        .businesses_owned_by_organization(scenario.player)
    {
        let kind = format!("{:?}", business.kind());
        println!(
            "  - Owns {} ({}, {})",
            business.name(),
            kind,
            host_district_label(scenario, business.id()),
        );
    }
    for record in scenario
        .state
        .enterprises()
        .enterprises_for_organization(scenario.player)
    {
        let cycles = scenario.state.enterprises().cycles_for(record.id()).count();
        println!(
            "  - Runs a {:?} enterprise at {}: {} settled cycle(s)",
            record.kind(),
            enterprise_label(scenario, record.id()),
            cycles,
        );
    }
    // Delegated authority a boss can actually inspect: who holds the lieutenant's mandate,
    // at what version, over which scopes. PRESS revises this to two districts; the other
    // branches keep the single home-district grant.
    if let Some(mandate) = scenario
        .state
        .delegation()
        .get_mandate(scenario.lieutenant_mandate)
    {
        let manager = scenario
            .state
            .world()
            .get_character(mandate.manager())
            .map(|record| record.name().to_owned())
            .unwrap_or_else(|| "?".to_owned());
        let mut scopes: Vec<String> = mandate
            .scopes()
            .iter()
            .map(|scope| match scope {
                crimocracy::delegation::ResponsibilityScope::Neighborhood(id) => scenario
                    .state
                    .world()
                    .get_neighborhood(*id)
                    .map(|record| record.name().to_owned())
                    .unwrap_or_else(|| "unknown district".to_owned()),
                crimocracy::delegation::ResponsibilityScope::Business(id) => scenario
                    .state
                    .world()
                    .get_business(*id)
                    .map(|record| record.name().to_owned())
                    .unwrap_or_else(|| "unknown venue".to_owned()),
                crimocracy::delegation::ResponsibilityScope::Function(function) => {
                    format!("{function:?}")
                }
            })
            .collect();
        scopes.sort();
        println!(
            "  - Mandate v{} ({:?}) held by {} over {}.",
            mandate.version(),
            mandate.status(),
            manager,
            if scopes.is_empty() {
                "no scopes".to_owned()
            } else {
                scopes.join(", ")
            },
        );
    }
    // What the organization actually knows: held information grouped by topic, with the
    // patrol pattern spelled out when the organization holds one. Reports narrate beats;
    // this is the accumulated stock a boss plans from.
    {
        let mut by_topic: BTreeMap<InformationTopic, usize> = BTreeMap::new();
        let mut patrol_windows: Vec<(u64, u64)> = Vec::new();
        for record in scenario
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(scenario.player))
        {
            *by_topic.entry(record.topic()).or_default() += 1;
            if record.topic() == InformationTopic::PoliceActivity
                && let Some(signal) = record.signal()
                && let crimocracy::intelligence::InformationSignal::PatrolPattern { intervals } =
                    signal
            {
                for interval in intervals {
                    patrol_windows.push((
                        u64::from(interval.start_minute()),
                        u64::from(interval.end_minute()),
                    ));
                }
            }
        }
        let held: Vec<String> = by_topic
            .iter()
            .map(|(topic, count)| format!("{count}x {topic:?}"))
            .collect();
        println!(
            "  - Holds {} information item(s){}.",
            by_topic.values().sum::<usize>(),
            if held.is_empty() {
                String::new()
            } else {
                format!(": {}", held.join(", "))
            },
        );
        if !patrol_windows.is_empty() {
            patrol_windows.sort();
            patrol_windows.dedup();
            println!(
                "  - Known patrol rhythm: {}.",
                format_patrol_windows(&patrol_windows)
            );
        }
    }
    let standing_reports = scenario
        .state
        .reports()
        .reports_for(scenario.player)
        .filter(|report| report.kind() == ReportKind::Standing)
        .count();
    if standing_reports > 0 {
        println!(
            "  - Word on the street moved {standing_reports} time(s) this session (Standing reports)."
        );
    }
    // Rival posture from the player-visible territory surface: the underworld the
    // organization actually competes with, not hidden rival books. Rival growth (or its
    // absence) is background by scope, but a boss can always see who holds the home district.
    {
        let home_district = scenario
            .state
            .world()
            .get_neighborhood(scenario.neighborhood)
            .expect("home neighborhood must persist")
            .name()
            .to_owned();
        let rival_name = scenario
            .state
            .world()
            .get_organization(scenario.rival)
            .expect("rival must persist")
            .name()
            .to_owned();
        let second_rival_name = scenario
            .state
            .world()
            .get_organization(scenario.second_rival)
            .expect("second rival must persist")
            .name()
            .to_owned();
        println!(
            "  - Underworld around {home_district}: {rival_name} and {second_rival_name} operate {} racket(s) between them in this district.",
            metrics.rival_home_enterprises,
        );
    }
    // Wage runway from the books the organization actually holds: headcount is a standing
    // carrying cost, and growth (or heat-taxed income) is what makes it bind. Early sessions
    // run slack; every added member and every heat surcharge narrows the cover.
    {
        let member_count = members.len().max(1);
        let daily_wage =
            scenario.registry.upkeep().per_member_daily().cents() * member_count as i64;
        let (front_daily, racket_daily, unsettled) = latest_daily_earnings(scenario);
        let daily_net = front_daily + racket_daily;
        println!(
            "  - Latest settled active books: fronts {} /day + rackets {} /day; {} active book(s) not yet settled. Estimates, not guaranteed income or cash available for wages.",
            format_cents(front_daily),
            format_cents(racket_daily),
            unsettled,
        );
        let cover = daily_net as f64 / daily_wage.max(1) as f64;
        println!(
            "  - Wages {} /day across {} member(s); recent books net ~{} /day ({:.1}x cover) - {}.",
            format_cents(daily_wage),
            member_count,
            format_cents(daily_net),
            cover,
            if financials.payroll_short_cents > 0 {
                "payroll already slipped and the crew remembers"
            } else if cover >= 2.0 {
                "payroll comfortable at this headcount"
            } else if cover >= 1.0 {
                "payroll covered but growth or heat would tighten it"
            } else {
                "payroll uncovered at this burn rate"
            },
        );
    }
    // Player-visible street standing so a leader can see what the city thinks.
    // Only touched impressions exist; absent means unremarkable baseline. Scores print as
    // qualitative bands, never exact numerics: the player learns standing through Standing
    // report prose, not through a hidden-subsystem dashboard.
    let registry = scenario.registry;
    for audience in [
        crimocracy::reputation::AudienceKind::Underworld,
        crimocracy::reputation::AudienceKind::Police,
        crimocracy::reputation::AudienceKind::Businesses,
    ] {
        for dimension in [
            crimocracy::reputation::ReputationDimension::Competence,
            crimocracy::reputation::ReputationDimension::Fear,
            crimocracy::reputation::ReputationDimension::Reliability,
            crimocracy::reputation::ReputationDimension::Treachery,
        ] {
            let score = crimocracy::reputation::reputation_system::resolve_score(
                registry,
                scenario.state.reputation(),
                scenario.player,
                audience,
                dimension,
            );
            let baseline = registry.reputation().baseline();
            if let Some(band) = standing_band(score, baseline) {
                println!("  - Standing {audience:?}/{dimension:?}: {band}.");
            }
        }
    }
}

/// Presentation-only distance from the authored baseline on the 0..=100 score scale.
/// A few ordinary successes are a slight shift, not an extreme standing. These bands
/// are not gameplay thresholds; neutral wording works for both competence and fear.
fn standing_band(score: u8, baseline: u8) -> Option<&'static str> {
    match (score.cmp(&baseline), score.abs_diff(baseline)) {
        (std::cmp::Ordering::Equal, _) => None,
        (std::cmp::Ordering::Greater, 1..=9) => Some("slightly above baseline"),
        (std::cmp::Ordering::Greater, 10..=24) => Some("noticeably above baseline"),
        (std::cmp::Ordering::Greater, _) => Some("far above baseline"),
        (std::cmp::Ordering::Less, 1..=9) => Some("slightly below baseline"),
        (std::cmp::Ordering::Less, 10..=24) => Some("noticeably below baseline"),
        (std::cmp::Ordering::Less, _) => Some("far below baseline"),
    }
}

#[cfg(test)]
mod standing_tests {
    use super::*;
    use crimocracy::reputation::reputation_system::{apply_reputation_delta, resolve_score};
    use crimocracy::reputation::{AudienceKind, ReputationDimension};

    #[test]
    fn standing_stays_slight_when_three_authored_success_shifts_accumulate() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let baseline = registry.reputation().baseline();
        let initial = resolve_score(
            &registry,
            scenario.state.reputation(),
            scenario.player,
            AudienceKind::Underworld,
            ReputationDimension::Competence,
        );
        assert_eq!(initial, baseline);
        assert_eq!(standing_band(initial, baseline), None);

        // Exercise the canonical score owner, not a fabricated reputation record.
        // This isolates presentation of authored shifts from operation RNG and decay.
        let shift = registry.reputation().achieved_underworld_competence();
        for successes in 1..=3 {
            apply_reputation_delta(
                &registry,
                &mut scenario.state,
                scenario.player,
                AudienceKind::Underworld,
                ReputationDimension::Competence,
                shift,
            )
            .unwrap();
            let score = resolve_score(
                &registry,
                scenario.state.reputation(),
                scenario.player,
                AudienceKind::Underworld,
                ReputationDimension::Competence,
            );
            assert_eq!(
                i16::from(score) - i16::from(baseline),
                successes * i16::from(shift)
            );
            assert_eq!(
                standing_band(score, baseline),
                Some("slightly above baseline")
            );
        }
    }

    #[test]
    fn standing_bands_use_baseline_distance_at_both_boundaries_and_score_rails() {
        for baseline in [40_u8, 50] {
            assert_eq!(standing_band(baseline, baseline), None);
            for (distance, above, below) in [
                (1, "slightly above baseline", "slightly below baseline"),
                (9, "slightly above baseline", "slightly below baseline"),
                (10, "noticeably above baseline", "noticeably below baseline"),
                (24, "noticeably above baseline", "noticeably below baseline"),
                (25, "far above baseline", "far below baseline"),
            ] {
                assert_eq!(standing_band(baseline + distance, baseline), Some(above));
                assert_eq!(standing_band(baseline - distance, baseline), Some(below));
            }
            assert_eq!(standing_band(100, baseline), Some("far above baseline"));
            assert_eq!(standing_band(0, baseline), Some("far below baseline"));
        }
    }
}

/// Normalize each active book separately: parallel books are additive, not extra elapsed days.
/// Only cycles settled under this owner contribute; an acquired seller's history is not our income.
pub fn latest_daily_earnings(scenario: &Scenario) -> (i64, i64, usize) {
    let mut fronts = 0;
    let mut rackets = 0;
    let mut unsettled = 0;
    for business in scenario
        .state
        .world()
        .businesses_owned_by_organization(scenario.player)
    {
        let Some(economy) = scenario.state.economy().get_business_economy(business.id()) else {
            continue;
        };
        if economy.status() != crimocracy::economy::BusinessOperatingStatus::Active {
            continue;
        }
        if let Some(cycle) = scenario
            .state
            .economy()
            .latest_cycle(business.id())
            .filter(|cycle| {
                cycle.owner() == crimocracy::world::BusinessOwner::Organization(scenario.player)
            })
        {
            fronts += daily_rate(
                cycle.net_cash().cents(),
                scenario
                    .registry
                    .get_business(business.kind())
                    .economics()
                    .cycle()
                    .as_minutes()
                    .into(),
            );
        } else {
            unsettled += 1;
        }
    }
    for enterprise in scenario
        .state
        .enterprises()
        .enterprises_for_organization(scenario.player)
    {
        if enterprise.status() != crimocracy::enterprises::EnterpriseStatus::Active {
            continue;
        }
        if let Some(cycle) = scenario.state.enterprises().latest_cycle(enterprise.id()) {
            rackets += daily_rate(
                cycle.net_cash().cents(),
                scenario
                    .registry
                    .get_enterprise(enterprise.kind())
                    .economics()
                    .cycle()
                    .as_minutes()
                    .into(),
            );
        } else {
            unsettled += 1;
        }
    }
    (fronts, rackets, unsettled)
}

#[cfg(test)]
mod earnings_tests {
    use super::*;

    #[test]
    fn rates_normalize_duration_without_treating_parallel_books_as_days() {
        assert_eq!(daily_rate(10_000, 720) + daily_rate(15_000, 1_440), 35_000);
        assert_eq!(daily_rate(-5_000, 2_880), -2_500);
    }

    #[test]
    fn latest_owned_books_include_every_active_front_and_racket() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        assert_eq!(latest_daily_earnings(&scenario), (0, 0, 2));
        let mut metrics = RunMetrics::default();
        run_until(
            &mut scenario,
            SimTime::from_minutes(1_440),
            false,
            &mut metrics,
        )
        .unwrap();
        let front = scenario
            .state
            .economy()
            .latest_cycle(scenario.front)
            .unwrap()
            .net_cash()
            .cents();
        let racket = scenario
            .state
            .enterprises()
            .latest_cycle(scenario.enterprise)
            .unwrap()
            .net_cash()
            .cents();
        assert_eq!(latest_daily_earnings(&scenario), (front, racket, 0));
    }
}

fn daily_rate(cents: i64, cycle_minutes: u64) -> i64 {
    assert!(
        cycle_minutes > 0,
        "authored cycles must have positive duration"
    );
    i64::try_from(i128::from(cents) * 1_440 / i128::from(cycle_minutes))
        .expect("daily earnings must fit money range")
}

pub fn enterprise_label(scenario: &Scenario, enterprise: EnterpriseId) -> String {
    let record = scenario
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .expect("labeled enterprise must persist");
    match record.location() {
        EnterpriseLocation::Business(business) => scenario
            .state
            .world()
            .get_business(business)
            .map(|record| {
                format!(
                    "{} ({})",
                    record.name(),
                    host_district_label(scenario, business)
                )
            })
            .unwrap_or_else(|| "enterprise".to_owned()),
        EnterpriseLocation::Neighborhood(_) => "district enterprise".to_owned(),
    }
}

pub fn host_district_label(scenario: &Scenario, business: BusinessId) -> String {
    scenario
        .state
        .world()
        .get_business(business)
        .and_then(|record| {
            scenario
                .state
                .world()
                .get_neighborhood(record.neighborhood())
        })
        .map(|neighborhood| neighborhood.name().to_owned())
        .unwrap_or_else(|| "unknown district".to_owned())
}

pub fn resolve_financial_view(
    scenario: &Scenario,
    metrics: &RunMetrics,
) -> Result<FinancialView, Box<dyn Error>> {
    let business_summary = resolve_organization_business_financial_summary(
        &scenario.state,
        scenario.player,
        SimTime::ZERO,
        scenario.state.now(),
    )?;
    let enterprise_net = scenario
        .state
        .enterprises()
        .cycles_for(scenario.enterprise)
        .try_fold(Money::ZERO, |sum, cycle| sum.checked_add(cycle.net_cash()))
        .expect("scenario enterprise totals must fit money range");
    let mut enterprise_lines = Vec::new();
    for record in scenario
        .state
        .enterprises()
        .enterprises_for_organization(scenario.player)
    {
        let id = record.id();
        let net = scenario
            .state
            .enterprises()
            .cycles_for(id)
            .try_fold(Money::ZERO, |sum, cycle| sum.checked_add(cycle.net_cash()))
            .expect("enterprise totals must fit money range");
        let heat = scenario
            .state
            .enterprises()
            .cycles_for(id)
            .try_fold(Money::ZERO, |sum, cycle| {
                sum.checked_add(cycle.investigation_heat())
            })
            .expect("enterprise heat totals must fit money range");
        enterprise_lines.push(EnterpriseLine {
            label: enterprise_label(scenario, id),
            cash_kind: scenario
                .state
                .finance()
                .get_account(record.cash_account())
                .expect("enterprise cash account must exist")
                .kind(),
            cycle_count: scenario.state.enterprises().cycles_for(id).count(),
            net_cents: net.cents(),
            heat_cents: heat.cents(),
            cash_cents: scenario
                .state
                .finance()
                .get_account(record.cash_account())
                .expect("enterprise cash account must exist")
                .balance()
                .cents(),
        });
    }
    let liquidation_cash = scenario
        .state
        .finance()
        .get_account(scenario.liquidation_cash)
        .expect("liquidation cash account must exist")
        .balance();
    let (held_property_operations, held_property_value) = scenario
        .state
        .operations()
        .operations_for_organization(scenario.player)
        .filter(|operation| operation.property_disposition().is_none())
        .filter_map(|operation| operation.resolution())
        .filter_map(|resolution| resolution.property_proceeds())
        .try_fold((0_u32, Money::ZERO), |(count, total), proceeds| {
            Some((
                count.checked_add(1)?,
                total.checked_add(proceeds.estimated_value())?,
            ))
        })
        .expect("scenario held-property totals must fit numeric bounds");
    let (liquidated_property_operations, liquidated_property_cash) = scenario
        .state
        .operations()
        .operations_for_organization(scenario.player)
        .filter_map(|operation| operation.property_disposition())
        .try_fold((0_u32, Money::ZERO), |(count, total), disposition| {
            Some((
                count.checked_add(1)?,
                total.checked_add(disposition.realized_value())?,
            ))
        })
        .expect("scenario liquidated-property totals must fit numeric bounds");
    let mut cash_kinds: BTreeMap<AccountKind, i64> = BTreeMap::new();
    for account in scenario
        .state
        .finance()
        .accounts_for(FinancialOwner::Organization(scenario.player))
    {
        // Settlement accounts are ledger counterparties, not governable cash; a boss
        // reads their cash position from what they actually hold.
        if account.kind() != AccountKind::Settlement {
            *cash_kinds.entry(account.kind()).or_default() += account.balance().cents();
        }
    }
    let cash_position: Vec<_> = cash_kinds.into_iter().collect();
    Ok(FinancialView {
        legitimate_cycle_count: business_summary.totals.cycle_count,
        legitimate_net_cents: business_summary.totals.net_cash.cents(),
        enterprise_cycle_count: scenario
            .state
            .enterprises()
            .cycles_for(scenario.enterprise)
            .count(),
        enterprise_net_cents: enterprise_net.cents(),
        enterprise_lines,
        liquidation_cash_cents: liquidation_cash.cents(),
        held_property_operations,
        held_property_value_cents: held_property_value.cents(),
        liquidated_property_operations,
        liquidated_property_cash_cents: liquidated_property_cash.cents(),
        cash_position,
        laundered_gross_cents: metrics.laundered_gross_cents,
        launder_fee_cents: metrics.launder_fee_cents,
        laundering_capacity_rejections: metrics.laundering_capacity_rejections,
        payroll_paid_cents: metrics.payroll_paid_cents,
        payroll_short_cents: metrics.payroll_short_cents,
    })
}

pub fn print_financial_view(scenario: &Scenario, view: FinancialView) {
    println!(
        "\n[FINANCIAL VIEW {}]",
        stamp(scenario.state.now().as_minutes())
    );
    println!(
        "  Cash states: street cash spends on the street but cannot buy legitimacy; accounted funds are washed money that can buy businesses; legitimate operating cash belongs to the fronts' own books."
    );
    let member_count = scenario
        .state
        .world()
        .characters_in_organization(scenario.player)
        .count();
    let per_member = scenario.registry.upkeep().per_member_daily();
    let daily_wage = per_member.cents() * member_count as i64;
    println!(
        "  Legitimate businesses: {} settled cycle(s), total net {}. Parallel business cycles are not elapsed campaign days.",
        view.legitimate_cycle_count,
        format_cents(view.legitimate_net_cents),
    );
    for line in &view.enterprise_lines {
        println!(
            "  Enterprise at {}: {} cycle(s), net {}, racket till ({:?}) {} (avg {} /settled cycle){}.",
            line.label,
            line.cycle_count,
            format_cents(line.net_cents),
            line.cash_kind,
            format_cents(line.cash_cents),
            format_cents(line.net_cents / (line.cycle_count.max(1) as i64)),
            if line.heat_cents > 0 {
                format!(
                    " including {} of district-heat surcharge",
                    format_cents(line.heat_cents)
                )
            } else {
                String::new()
            },
        );
    }
    if view.enterprise_lines.is_empty() {
        println!("  No enterprise books.");
    }
    println!(
        "  Fence proceeds (street cash, awaiting wash): {}.",
        format_cents(view.liquidation_cash_cents),
    );
    println!(
        "  Held operation property: {} operation(s), estimated value {}, unliquidated.",
        view.held_property_operations,
        format_cents(view.held_property_value_cents),
    );
    println!(
        "  Liquidated operation property: {} disposition(s), realized {}.",
        view.liquidated_property_operations,
        format_cents(view.liquidated_property_cash_cents),
    );
    if !view.cash_position.is_empty() {
        let total: i64 = view.cash_position.iter().map(|(_, cents)| cents).sum();
        let lines: Vec<_> = view
            .cash_position
            .iter()
            .map(|(kind, cents)| format!("{} {}", account_kind_label(*kind), format_cents(*cents)))
            .collect();
        println!(
            "  Cash position (total {}): {}.",
            format_cents(total),
            lines.join(", "),
        );
    }
    if view.laundered_gross_cents > 0 || view.laundering_capacity_rejections > 0 {
        println!(
            "  Laundered to date: {} gross through the front's books, {} kept as booked revenue{}; the books refused {} over-capacity request(s).",
            format_cents(view.laundered_gross_cents),
            format_cents(view.launder_fee_cents),
            if view.laundered_gross_cents > 0 {
                format!(
                    ", {} now accounted",
                    format_cents(view.laundered_gross_cents - view.launder_fee_cents)
                )
            } else {
                String::new()
            },
            view.laundering_capacity_rejections,
        );
    }
    let payroll_status = if view.payroll_short_cents > 0 {
        format!(
            "SHORTFALL — crew resentment rising ({} unpaid)",
            format_cents(view.payroll_short_cents)
        )
    } else {
        "all wages met".to_owned()
    };
    println!(
        "  Payroll to date: {} paid across {} member(s) ({} /day), {} unpaid — {}.",
        format_cents(view.payroll_paid_cents),
        member_count,
        format_cents(daily_wage),
        format_cents(view.payroll_short_cents),
        payroll_status,
    );
}

/// Short leader-readable label for an organization account kind in the financial view.
pub fn account_kind_label(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::StreetCash => "street cash",
        AccountKind::ConcealedCash => "concealed cash",
        AccountKind::AccountedFunds => "accounted funds",
        AccountKind::LegitimateOperating => "legitimate operating",
        AccountKind::Settlement => "settlement",
    }
}

pub fn print_report(label: &str, report: &ReportRecord, scenario: &Scenario) {
    println!(
        "[{label}] minute {}: {}",
        report.generated_at().as_minutes(),
        report.title()
    );
    for entry in report.entries() {
        let marker = match entry.attention {
            AttentionClass::Routine => "routine",
            AttentionClass::Notable => "notable",
            AttentionClass::Exception => "EXCEPTION",
            AttentionClass::Crisis => "CRISIS",
        };
        let context = entry.entities.iter().find_map(|entity| {
            if let EntityRef::Operation(operation) = entity {
                return scenario
                    .state
                    .operations()
                    .get_operation(*operation)
                    .map(|record| record.title().to_owned());
            }
            None
        });
        // After-action and abort summaries already lead with the operation title, so the
        // entity-derived context would only echo it; keep it for entries that do not.
        let context = context.filter(|title| {
            !entry.summary.starts_with(title.as_str())
                && !entry.summary.starts_with(format!("{title}: ").as_str())
        });
        if let Some(context) = context {
            println!("  - [{marker}] [operation: {context}] {}", entry.summary);
        } else {
            println!("  - [{marker}] {}", entry.summary);
        }
    }
}

/// Condensed report rendering for routine briefs: header plus only the entries that need a
/// leader's attention. Full after-action text stays on the [AFTER-ACTION]/[ABORT REPORT] beats so
/// the interesting consequence text is not drowned in repeated boilerplate. A day with nothing
/// above routine attention is summarized in its header instead of printing an empty entry list.
pub fn print_report_condensed(label: &str, report: &ReportRecord) {
    let entries = report.entries();
    let attention_worthy: Vec<_> = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.attention,
                AttentionClass::Notable | AttentionClass::Exception | AttentionClass::Crisis
            )
        })
        .collect();
    if attention_worthy.is_empty() {
        println!(
            "[{label}] minute {}: {} (quiet; all {} entr{} routine)",
            report.generated_at().as_minutes(),
            report.title(),
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
        );
        return;
    }
    println!(
        "[{label}] minute {}: {} ({} entr{})",
        report.generated_at().as_minutes(),
        report.title(),
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
    );
    for entry in attention_worthy {
        let marker = match entry.attention {
            AttentionClass::Routine => "routine",
            AttentionClass::Notable => "notable",
            AttentionClass::Exception => "EXCEPTION",
            AttentionClass::Crisis => "CRISIS",
        };
        println!("  - [{marker}] {}", entry.summary);
    }
}

/// Closing brief history without one header for every quiet campaign day. Briefs that need
/// leadership attention retain their chronology and authored entries; all-routine days collapse
/// into one count because they add no new decision information.
pub fn print_executive_briefs<'a>(reports: impl Iterator<Item = &'a ReportRecord>) {
    let reports = reports.collect::<Vec<_>>();
    let has_attention = |report: &ReportRecord| {
        report.entries().iter().any(|entry| {
            matches!(
                entry.attention,
                AttentionClass::Notable | AttentionClass::Exception | AttentionClass::Crisis
            )
        })
    };
    let quiet_count = reports
        .iter()
        .copied()
        .filter(|report| !has_attention(report))
        .count();
    println!("\n[EXECUTIVE BRIEFS]");
    for report in reports
        .iter()
        .copied()
        .filter(|report| has_attention(report))
    {
        print_report_condensed("BRIEF", report);
    }
    if quiet_count > 0 {
        println!("[BRIEF] {quiet_count} additional daily brief(s) contained only routine entries.");
    }
}

pub fn print_metrics(metrics: &RunMetrics) {
    let property_acquired = optional_dollars(metrics.property_acquired_value_cents);
    let property_realized = optional_dollars(metrics.property_realized_cash_cents);
    let liquidation_minute = optional_minute(metrics.liquidation_minute);
    println!(
        "{:<6} [{:<9}]: {}, finish {:?}m, police dispatched {}, police arrived {}, decisions {}, plan items {} {:?}, intel {:?}, exposure {:?}/{:?}, property {} -> {} cash at {}, case {}, evidence {}, player legal intel {}, police intel {}, follow-up {:?}/{} info (follow-up hot {:?}), cold confirmed {:?} @ {:?}, case work {}/{}, surveillance discoveries {}, reports {}, briefs {}, recruitment {}, poach warnings {}, departures {}, legit {}, enterprise {}, matched@{}: legit {}, enterprise {}",
        metrics.strategy.expect("strategy must be set").label(),
        metrics
            .variation
            .expect("fixture variation must be set")
            .label(),
        terminal_label(metrics),
        metrics.burglary_terminal_minute,
        metrics.police_dispatched,
        metrics.police_arrived,
        metrics.decision_requests,
        metrics.planning_information_count,
        metrics.planning_information_topics,
        metrics.burglary_information_quality,
        metrics.exposure_level,
        metrics.exposure_score,
        property_acquired,
        property_realized,
        liquidation_minute,
        metrics.investigation_created,
        metrics.evidence_count,
        metrics.player_legal_activity_information,
        metrics.player_police_activity_information,
        metrics.counterintelligence_outcome,
        metrics.counterintelligence_information,
        metrics.followup_case_active,
        metrics.cold_case_confirmed,
        metrics.case_cold_minute,
        metrics.investigation_work_scheduled,
        metrics.investigation_work_resolved,
        metrics.discovered_surveillance_information,
        metrics.player_report_count,
        metrics.executive_brief_count,
        metrics.autonomous_recruitment_attempts,
        metrics.player_poach_warnings,
        metrics.player_personnel_departures,
        optional_dollars(metrics.legitimate_net_cents),
        optional_dollars(metrics.enterprise_net_cents),
        optional_minute(metrics.matched_financial_boundary_minute),
        optional_dollars(metrics.matched_legitimate_net_cents),
        optional_dollars(metrics.matched_enterprise_net_cents),
    );
    println!(
        "        act 2: second score discovered {}, expired {}, replacement {}, second burglary {} @ {} (outcome {:?}, aborted {}), recon info {}, property {} -> {}, self-heat case opened {} read {:?}",
        metrics.second_opportunity_discovered,
        metrics.second_opportunity_expired,
        metrics.replacement_recruited,
        metrics.second_burglary.is_some(),
        optional_minute(metrics.second_burglary_terminal_minute),
        metrics.second_burglary_outcome,
        metrics.second_burglary_aborted,
        metrics.second_act_recon_information,
        optional_dollars(metrics.second_act_property_acquired_value_cents),
        optional_dollars(metrics.second_act_property_realized_cash_cents),
        metrics.self_heat_case_opened,
        metrics.self_heat_case_active,
    );
    if metrics.expansion_established {
        println!(
            "        diversification: second-district enterprise established, net {}, unrelated-case heat {}",
            optional_dollars(metrics.expansion_net_cents),
            optional_dollars(metrics.expansion_heat_cents),
        );
    }
    println!(
        "        money: laundered {} gross through the front's books (house fee {}, accounted-payroll spend {}, accounted balance {}), books refused {} over-capacity request(s), vice inquiries drawn {}",
        optional_dollars(Some(metrics.laundered_gross_cents)),
        optional_dollars(Some(metrics.launder_fee_cents)),
        optional_dollars(Some(metrics.payroll_accounted_spent_cents)),
        optional_dollars(metrics.accounted_balance_cents),
        metrics.laundering_capacity_rejections,
        metrics.vice_inquiries_drawn,
    );
    if metrics.case_witness_registered
        || metrics.witness_pressure_attempted
        || metrics.player_member_arrests > 0
    {
        println!(
            "        witness chain: named case witness {}, interviews scheduled {}, testimony produced {}, pressure run {} (outcome {:?}, degraded {}), member arrests {}",
            metrics.case_witness_registered,
            metrics.witness_interviews_scheduled,
            metrics.witness_testimony_produced,
            metrics.witness_pressure_attempted,
            metrics.witness_pressure_outcome,
            metrics.witness_cooperation_degraded,
            metrics.player_member_arrests,
        );
    }
    if metrics.win_back_attempted {
        println!(
            "        win-back: attempted (accepted {:?}, margin {:?})",
            metrics.win_back_accepted, metrics.win_back_margin,
        );
    }
}

/// A sensitivity profile earns its place by making at least one policy treatment behave
/// differently from the others. When every strategy converges on the same outcome mix with no
/// police pressure, say so explicitly: that is evidence about the scenario's discrimination, not
/// a failure, but a reader should not have to infer it from three identical blocks.
pub fn print_convergence_observation(
    profile: ScenarioProfile,
    rush: &Aggregate,
    press: &Aggregate,
    recon: &Aggregate,
) {
    let outcome_mix = |aggregate: &Aggregate| {
        (
            aggregate.achieved,
            aggregate.partial,
            aggregate.failed,
            aggregate.aborted,
        )
    };
    let converged = outcome_mix(rush) == outcome_mix(press)
        && outcome_mix(press) == outcome_mix(recon)
        && rush.police_arrived == press.police_arrived
        && press.police_arrived == recon.police_arrived;
    if converged {
        println!(
            "[OBSERVATION] {}: all strategies converged ({}/{} achieved, {} police arrivals). Under this scenario the patrol timing removes the information decision, so policy choice carries no leverage here; treat this block as a control, not a contrast.",
            profile.label(),
            rush.achieved,
            rush.samples,
            rush.police_arrived,
        );
    }
}

pub fn optional_minute(value: Option<u64>) -> String {
    value.map_or_else(|| "-".to_owned(), |minute| format!("{minute}m"))
}

pub fn print_experience_readout(
    rush: &RunMetrics,
    press: &RunMetrics,
    recon: &RunMetrics,
    vice_demonstrated: bool,
) {
    println!("\n--- PLAYER LOOP READOUT ---");
    println!(
        "Core fantasy: learn what the city reveals, turn it into an organizational plan, delegate execution, then stay powerful enough to absorb the consequences."
    );
    println!(
        "Read this as a decision trace, not a quality grade: compare what leadership knew, what it chose, what it paid, and what remains unresolved."
    );
    if rush.win_back_accepted == Some(true) {
        println!(
            "[WATCH] RUSH recovered its specialist through a drive-matched pitch. A replacement is unnecessary unless that recovery fails; adding headcount would add wages, not repair a missing capability."
        );
    }
    if press.cold_case_confirmed == Some(true) {
        println!(
            "[WATCH] The burglary file cooled. That is not a district-wide all-clear; racket warnings and street surcharges have their own continuation."
        );
    }
    if !recon.self_heat_case_opened {
        println!(
            "[WATCH] No casing case was disclosed to RECON in this run. That is missing knowledge, not proof that surveillance left no trace."
        );
    }
    println!("Evidence coverage (not a game-quality score):");
    let mut missing = 0u32;
    let mut checkpoint = |label: &str, present: bool, evidence: &str| {
        missing += u32::from(!present);
        print_loop_checkpoint(label, present, evidence);
    };
    checkpoint(
        "learn",
        recon.discovered_surveillance_information > 0,
        "surveillance produces actionable patrol and target information",
    );
    checkpoint(
        "plan",
        recon.planning_information_count > rush.planning_information_count
            && recon.outcome == Some(OperationObjectiveOutcome::Achieved),
        "the player can make a better plan from organization-held intelligence",
    );
    checkpoint(
        "failure teaches",
        rush.aborted
            && rush.player_police_activity_information > 0
            && rush
                .second_act_planning_topics
                .contains(&InformationTopic::PoliceActivity),
        "a standing abort is debriefed into organizational police-response knowledge that informs the rebuilt crew's next job without inventing a patrol pattern the crew never observed",
    );
    let response_choice_changed_consequence = rush.aborted
        && press.outcome.is_some()
        && press.decision_requests > 0
        && press.player_police_activity_information > 0;
    checkpoint(
        "choice",
        response_choice_changed_consequence,
        "a player response to the same police exception changes whether the operation aborts or resolves",
    );
    checkpoint(
        "delegate",
        recon.burglary.is_some() && recon.outcome.is_some(),
        "the plan resolves through assigned people and authored capabilities",
    );
    checkpoint(
        "respond",
        press.decision_requests > 0 && press.player_police_activity_information > 0,
        "an exception pauses the plan and a field report returns to the organization",
    );
    checkpoint(
        "consequences",
        press.player_legal_activity_information > 0 && recon.property_realized_cash_cents.is_some(),
        "the same operation system can create legal pressure or recover value into cash",
    );
    checkpoint(
        "follow-up",
        press.counterintelligence_outcome.is_some()
            && press.counterintelligence_information > 0
            && press.followup_case_active == Some(true),
        "a player-visible legal report can seed a precinct check that reads whether the case is still hot",
    );
    checkpoint(
        "survive",
        press.cold_case_confirmed == Some(true),
        "the contact confirms the burglary file shelved; this does not clear separate racket pressure",
    );
    checkpoint(
        "organization",
        rush.player_personnel_departures > 0,
        "a police-exposed crew member can be courted away by a rival without a scripted event",
    );
    checkpoint(
        "rebuild",
        (rush.replacement_recruited || rush.win_back_accepted == Some(true))
            && rush.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved),
        "leadership restores its entry capability through win-back or replacement, then works the next score without a redundant hire",
    );
    checkpoint(
        "second wind",
        recon.second_act_recon_information > 0
            && (recon.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved)
                || (recon.self_heat_case_opened
                    && recon.self_heat_case_active != Some(false)
                    && recon.second_burglary.is_none()
                    && recon.second_opportunity_expired)),
        "fresh planning changes the next move: RECON takes the reopened score when clear and gives it up when its own casing creates a case the channel cannot affirmatively clear",
    );
    checkpoint(
        "own heat",
        recon.self_heat_case_opened
            && recon.self_heat_case_active != Some(false)
            && recon.second_burglary.is_none(),
        "casing carries risk both ways: when the crew's own casing reports exposure, leadership asks its standing police contact whether a file exists and stands down unless the channel explicitly says the matter is shelved",
    );
    checkpoint(
        "counterplay",
        press.witness_pressure_attempted,
        "a witnessed after-action can trigger a player-authored pressure operation against the publicly known shopkeeper; it either lands or visibly encounters enough police risk to justify walking away",
    );
    checkpoint(
        "discipline cost",
        press.second_opportunity_expired && press.second_burglary.is_none(),
        "choosing to stand down has a real price: the second score lapses while the hot case stays protected",
    );
    checkpoint(
        "diversify",
        press.expansion_established
            && press.expansion_net_cents.is_some_and(|net| net > 0)
            && press.expansion_heat_cents == Some(0),
        "idle capital during the wait becomes governance: a revised two-district mandate and a second-district enterprise the hot home case cannot tax",
    );
    let defector_trail_shown = rush.defector_trail_confirmed == Some(true)
        && press.defector_trail_confirmed == Some(true)
        && recon.defector_trail_confirmed.is_none();
    checkpoint(
        "defector trail",
        defector_trail_shown,
        "after a departure, the organization can confirm where the defector landed through its own canonical surveillance channel instead of the report leaking the rival",
    );
    let win_back_shown = rush.win_back_attempted
        && press.win_back_attempted
        && rush.win_back_accepted.is_some()
        && !recon.win_back_attempted;
    checkpoint(
        "win-back",
        win_back_shown,
        "after confirming where a defector landed, leadership can make one canonical executive re-approach and the pitch resolves through production recruitment scoring",
    );
    // Window honesty: compare branches at their shared campaign-day boundary when both captured
    // it, because the PRESS narrative arc deliberately runs longer than RUSH/RECON and raw
    // cumulative totals over different observation lengths are not comparable.
    let same_window = rush.matched_legitimate_net_cents.is_some()
        && press.matched_legitimate_net_cents.is_some()
        && recon.matched_legitimate_net_cents.is_some();
    let legitimate_isolated = if same_window {
        rush.matched_legitimate_net_cents == press.matched_legitimate_net_cents
            && press.matched_legitimate_net_cents == recon.matched_legitimate_net_cents
    } else {
        rush.legitimate_net_cents == press.legitimate_net_cents
            && press.legitimate_net_cents == recon.legitimate_net_cents
    };
    // Any branch whose case lived across the whole matched window pays the street surcharge
    // on every cycle in it; branches whose cases appeared later (or never) pay less over the
    // same window. With casing risk live, nearly every branch draws some case, so the honest
    // signal is differential: an early-opened case must cost more than a cleaner branch earned.
    let boundary = |run: &RunMetrics| run.matched_financial_boundary_minute.unwrap_or(u64::MAX);
    let enterprise_window = |run: &RunMetrics| {
        run.matched_enterprise_net_cents
            .or(run.enterprise_net_cents)
    };
    let all_runs = [rush, press, recon];
    let all_nets: Vec<i64> = all_runs
        .iter()
        .filter_map(|run| enterprise_window(run))
        .collect();
    let long_case_nets: Vec<i64> = all_runs
        .iter()
        .filter_map(|run| {
            let case_lived_across_window = run.investigation_created
                && run
                    .case_open_minute
                    .is_some_and(|open| open < boundary(run));
            case_lived_across_window
                .then_some(())
                .and_then(|_| enterprise_window(run))
        })
        .collect();
    let enterprise_heat_shown = !long_case_nets.is_empty()
        && long_case_nets
            .iter()
            .any(|heated| all_nets.iter().any(|net| net > heated));
    checkpoint(
        "routine",
        legitimate_isolated,
        "legitimate front continues identically while leadership focuses on exceptions",
    );
    checkpoint(
        "heat cost",
        enterprise_heat_shown,
        "a case that stayed open across the whole matched window taxes the delegated enterprise every cycle, visibly earning less than branches whose districts stayed clean longer",
    );
    let liquidation_varies = rush
        .second_act_property_realized_cash_cents
        .zip(recon.second_act_property_realized_cash_cents)
        .map(|(a, b)| a != b)
        .unwrap_or(false)
        || recon.property_realized_cash_cents.is_some()
            && press.property_realized_cash_cents.is_none();
    checkpoint(
        "venue choice",
        liquidation_varies || recon.property_realized_cash_cents.is_some(),
        "liquidated resale value reflects the venue's district police presence",
    );
    let laundering_shown = rush.laundered_gross_cents > 0
        && recon.laundered_gross_cents > 0
        && press.laundered_gross_cents > 0
        && [rush, press, recon]
            .iter()
            .all(|run| run.laundered_gross_cents - run.launder_fee_cents > 0);
    checkpoint(
        "clean money",
        laundering_shown,
        "street earnings pass through an owned front's books into accounted funds, and the front's plausible-volume ceiling visibly caps how fast dirty money becomes clean",
    );
    let wealth_loop_shown = press.front_acquired
        && press.acquisition_price_cents.is_some()
        && press.acquisition_rejections > 0;
    checkpoint(
        "legit wealth",
        wealth_loop_shown,
        "accounted wealth converts into an owned legitimate asset through the canonical acquisition path: the short book first surfaces as a visible rejection, the purchase lands at the authored price, and owning the venue unlocks the second-district racket - the money loop closes",
    );
    let any_vice = vice_demonstrated
        || [rush, press, recon]
            .iter()
            .any(|run| run.vice_inquiries_drawn > 0);
    checkpoint(
        "vice heat",
        any_vice,
        "sustained district casework can convert into a dedicated vice inquiry on a racket itself: the manager reports that new pressure, while lying low or diversifying districts remain available counters",
    );
    if missing > 0 {
        println!(
            "[NOTE] {missing} checkpoint(s) absent in this comparison. Check rotated runs and explicit probes; absence here is neither a failure nor proof of coverage elsewhere."
        );
    }
    println!("Observed decision leverage:");
    println!(
        "  - Information leverage: RECON selected {} planning item(s) versus RUSH's {} and finished as {} versus {}.",
        recon.planning_information_count,
        rush.planning_information_count,
        terminal_label(recon),
        terminal_label(rush),
    );
    println!(
        "  - Information risk: {} (contact case read: {:?}). No disclosed file is not proof of no institutional attention.",
        if recon.self_heat_check_required {
            "the second casing reported exposure, so RECON checked its contact before authorizing another score"
        } else {
            "the second casing reported no exposure; RECON had no crew-observed trigger for a case query"
        },
        recon.self_heat_case_active,
    );
    println!(
        "  - Exception leverage: PRESS chose Continue on the score's police exception at {} surfaced decision(s) and Abort on every later one, producing {} versus {}.",
        press.decision_requests,
        terminal_label(press),
        terminal_label(rush),
    );
    println!(
        "  - Witness counterplay: PRESS's after-action says the score was witnessed, leadership answers with one pressure operation against the publicly known shopkeeper, and that operation visibly {}.",
        if press.witness_pressure_aborted {
            "aborts when another police response arrives - a blind guess in a watched district risks a response whether or not the case machinery is active, so walking away is the disciplined play. This pressure attempt does not establish how RECON's separate casing resolved"
        } else {
            match press.witness_pressure_outcome {
                Some(OperationObjectiveOutcome::Achieved) => "achieves its objective",
                Some(OperationObjectiveOutcome::Partial) => "partially achieves its objective",
                Some(OperationObjectiveOutcome::Failed) => "fails",
                None => "ends without a resolved objective",
            }
        },
    );
    println!(
        "  - Personnel leverage: RUSH/PRESS exposed the crew to police and lost {} crew member(s) to rival recruitment, while RECON kept everyone ({} departures) because the crew never saw police.",
        rush.player_personnel_departures + press.player_personnel_departures,
        recon.player_personnel_departures,
    );
    println!(
        "  - Consequence leverage: PRESS received {} legal-activity information item(s), read the burglary case as still hot at {}, then later confirmed that same case shelved through its police channel; over the matched campaign window the heated gambling book earned {} versus {} in the cleaner comparison branch; RECON realized {} of resale cash via a low-police venue.",
        press.player_legal_activity_information,
        press
            .counterintelligence_scheduled_at
            .map(stamp)
            .unwrap_or_else(|| "-".to_owned()),
        optional_dollars(enterprise_window(press)),
        optional_dollars(enterprise_window(rush).or_else(|| enterprise_window(recon))),
        optional_dollars(recon.property_realized_cash_cents),
    );
    println!(
        "  - Time tradeoff: RECON finished at {} versus RUSH at {}; the extra planning time bought lower exposure and liquid value in this matched fixture.",
        recon
            .burglary_terminal_minute
            .map(stamp)
            .unwrap_or_else(|| "-".to_owned()),
        rush.burglary_terminal_minute
            .map(stamp)
            .unwrap_or_else(|| "-".to_owned()),
    );
    println!(
        "  - Diversification leverage: while the case stayed hot, PRESS bought its harbor venue outright with clean money and converted idle street cash into a second-district book earning {} with {} of unrelated-case heat, versus the canal book's heat-taxed window net of {}.",
        optional_dollars(press.expansion_net_cents),
        optional_dollars(press.expansion_heat_cents),
        optional_dollars(press.enterprise_net_cents),
    );
    println!(
        "  - Money-state leverage: resale cash can pay wages and capitalize rackets, but only accounted funds can buy legitimate businesses; every branch routes proceeds through its front's books ({} gross for RECON), and the front's per-cycle plausible volume rejected the over-capacity remainder {} time(s) across branches. PRESS then spent its accumulated accounted funds on the harbor venue ({}), so conversion speed - not desire - limits how fast dirty money becomes clean, and clean money has a real purchase waiting.",
        optional_dollars(Some(recon.laundered_gross_cents)),
        rush.laundering_capacity_rejections
            + press.laundering_capacity_rejections
            + recon.laundering_capacity_rejections,
        optional_dollars(press.acquisition_price_cents),
    );
    println!(
        "  - Visibility leverage: the branches drew {} vice inquiries this comparison, and the vice-heat probe demonstrates the full chain deterministically every run - clean districts never roll attention; sustained casework compounds a per-case street surcharge onto every cycle and can convert into a dedicated inquiry on the racket itself, taxing every book in that district (including rivals') until it shelves. Going dark or moving districts are the honest counters.",
        rush.vice_inquiries_drawn + press.vice_inquiries_drawn + recon.vice_inquiries_drawn,
    );
    println!(
        "Player attention load: RUSH {} surfaced decision(s), PRESS {}, RECON {}; player reports {}/{}/{}, executive briefs {}/{}/{}.",
        rush.decision_requests,
        press.decision_requests,
        recon.decision_requests,
        rush.player_report_count,
        press.player_report_count,
        recon.player_report_count,
        rush.executive_brief_count,
        press.executive_brief_count,
        recon.executive_brief_count,
    );
}

pub fn print_loop_checkpoint(label: &str, present: bool, evidence: &str) -> bool {
    println!(
        "  [{:>12}] {:<5} - {}",
        label,
        if present { "shown" } else { "missing" },
        evidence,
    );
    present
}

pub fn terminal_label(metrics: &RunMetrics) -> String {
    if metrics.aborted {
        let phase = metrics
            .abort_phase
            .map(abort_phase_label)
            .unwrap_or("at unknown phase");
        let cause = metrics
            .abort_cause
            .map(abort_cause_label)
            .unwrap_or_else(|| "unknown cause".to_owned());
        format!("aborted {phase} by {cause}")
    } else {
        format!(
            "completed {}",
            objective_label(metrics.outcome).unwrap_or("unresolved outcome")
        )
    }
}

pub fn abort_phase_label(phase: OperationAbortPhase) -> &'static str {
    match phase {
        OperationAbortPhase::BeforeStart => "before start",
        OperationAbortPhase::InProgress => "in progress",
        OperationAbortPhase::AwaitingDecision => "while awaiting decision",
    }
}

pub fn abort_cause_label(cause: OperationAbortCause) -> String {
    match cause {
        OperationAbortCause::AuthorityOrder => "authority order".to_owned(),
        OperationAbortCause::Decision(id) => format!("decision request {id}"),
        OperationAbortCause::PoliceArrival(id) => format!("police arrival {id}"),
        OperationAbortCause::DeadlineMissed => "missed deadline".to_owned(),
        OperationAbortCause::OpportunityExpired(id) => format!("opportunity {id} expired"),
        OperationAbortCause::ObjectiveUnavailable(blocker) => match blocker {
            OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
                "objective unavailable: target ownership changed".to_owned()
            }
            OperationObjectiveBlocker::TargetEconomyInactive => {
                "objective unavailable: target business inactive".to_owned()
            }
            OperationObjectiveBlocker::NoPressureableWitnessCase => {
                "objective unavailable: witness pressure no longer actionable".to_owned()
            }
            OperationObjectiveBlocker::ExtractionCustodyEnded => {
                "objective unavailable: target custody ended".to_owned()
            }
        },
        OperationAbortCause::ParticipantDetained(id) => format!("participant detention {id}"),
    }
}

/// Renders an objective outcome as a lowercase label; `None` means the operation
/// never reached a terminal objective state.
pub fn objective_label(outcome: Option<OperationObjectiveOutcome>) -> Option<&'static str> {
    match outcome {
        None => None,
        Some(OperationObjectiveOutcome::Achieved) => Some("achieved"),
        Some(OperationObjectiveOutcome::Partial) => Some("partial"),
        Some(OperationObjectiveOutcome::Failed) => Some("failed"),
    }
}

/// Renders an optional tri-state as `yes` / `no` / `-` for one-line readouts.
pub fn tri_state(value: Option<bool>) -> &'static str {
    match value {
        None => "-",
        Some(true) => "yes",
        Some(false) => "no",
    }
}

/// Renders an optional scalar as its value or `-` when absent.
pub fn optional_scalar<T: std::fmt::Display>(value: Option<T>) -> String {
    match value {
        None => "-".to_owned(),
        Some(value) => value.to_string(),
    }
}

/// Renders an absolute campaign minute as the clock time the player would see on a report.
pub fn format_minute_of_day(minute: u64) -> String {
    let minute_of_day = minute % 1_440;
    format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// Renders an absolute campaign minute as the day-anchored clock time the player would
/// see on a report, e.g. `minute 160, Day 1 02:40`. Multi-day arcs (the PRESS stand-down)
/// stay temporally anchored instead of collapsing to a bare clock time.
pub fn format_day_minute(minute: u64) -> String {
    format!(
        "Day {} {}",
        minute / 1_440 + 1,
        format_minute_of_day(minute)
    )
}

/// Renders patrol windows as the clock ranges a player reads in a surveillance report,
/// e.g. `01:00-04:30, 20:00-23:00`, instead of raw minute tuples.
pub fn format_patrol_windows(windows: &[(u64, u64)]) -> String {
    windows
        .iter()
        .map(|(start, end)| {
            format!(
                "{}-{}",
                format_minute_of_day(*start),
                format_minute_of_day(*end)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a player-facing tick beat as minute plus day-anchored clock, e.g.
/// `minute 160, Day 1 02:40`.
pub fn stamp(minute: u64) -> String {
    format!("minute {}, {}", minute, format_day_minute(minute))
}

/// Renders cents as a player-facing dollar amount, e.g. `23019` -> `$230.19`.
pub fn format_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let magnitude = cents.unsigned_abs();
    format!("{sign}${}.{:02}", magnitude / 100, magnitude % 100)
}

pub fn optional_dollars(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), format_cents)
}
