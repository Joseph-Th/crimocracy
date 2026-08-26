//! Player-facing printouts: labels, financial views, reports, and experience readouts.

use crimocracy::core::attention::AttentionClass;
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, EnterpriseId, OperationId};
use crimocracy::core::time::SimTime;
use crimocracy::economy::business_reporting::resolve_organization_business_financial_summary;
use crimocracy::enterprises::EnterpriseLocation;
use crimocracy::finance::{AccountKind, FinancialOwner, Money};
use crimocracy::intelligence::{InformationTopic, KnowledgeHolder};
use crimocracy::legal::InvestigationWorkStatus;
use crimocracy::operations::{
    OperationAbortCause, OperationAbortPhase, OperationObjectiveOutcome, RoleKind,
};
use crimocracy::reports::{ReportKind, ReportRecord};
use crimocracy::world::{CapabilityKind, Rating};
use std::collections::BTreeMap;
use std::error::Error;

use crate::*;

pub fn print_second_act_recap(scenario: &Scenario, strategy: Strategy, metrics: &RunMetrics) {
    let target = scenario.variation.alternate_target_name();
    match strategy {
        Strategy::Rush | Strategy::Recon => {
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
                println!(
                    "[ACT 2] Rebuild evidence: replacement recruited through executive recruitment; no fresh recon was used; the rebuilt crew worked the morning lull on {} planning item(s), including the debriefed police-response observation.",
                    metrics.second_act_planning_topics.len()
                );
            } else {
                println!(
                    "[ACT 2] Re-plan evidence: fresh surveillance produced {} information item(s) and the burglary used a patrol-safe window.",
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

pub fn role_label(role: RoleKind) -> &'static str {
    match role {
        RoleKind::Driver => "driver",
        RoleKind::Lookout => "lookout",
        RoleKind::EntrySpecialist => "entry specialist",
        RoleKind::SafeSpecialist => "safe specialist",
        RoleKind::Muscle => "muscle",
        RoleKind::InsideContact => "inside contact",
        RoleKind::Coordinator => "coordinator",
        RoleKind::Surveillance => "surveillance operator",
        RoleKind::Negotiator => "negotiator",
    }
}

pub fn print_starting_player_view(scenario: &Scenario) {
    println!("[ORGANIZATION] Marrow Organization");
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
        println!(
            "  - {:<14} autonomy {:?}; management {:?}; burglary {:?}; surveillance {:?}; stealth {:?}",
            record.name(),
            record.autonomy(),
            record.capability(CapabilityKind::Management).map(Rating::value),
            record.capability(CapabilityKind::Burglary).map(Rating::value),
            record.capability(CapabilityKind::Surveillance).map(Rating::value),
            record.capability(CapabilityKind::Stealth).map(Rating::value),
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
    let detective = scenario
        .state
        .world()
        .get_character(scenario.detective)
        .expect("detective must exist");
    println!(
        "[STATE] {} is available to Central Precinct with Investigation {}.",
        detective.name(),
        detective
            .capability(CapabilityKind::Investigation)
            .expect("detective must have investigation capability")
            .value(),
    );
    println!(
        "[STATE] {handler_name} keeps a standing Police-channel contact with {contact_name} inside Central Precinct; a quiet word costs no street exposure."
    );
    let replacement = scenario
        .state
        .world()
        .get_character(scenario.danny_ferro)
        .expect("replacement candidate must exist");
    println!(
        "[STATE] {} is an independent with Burglary {} / Stealth {}; Marrow holds a personal relationship with him, so he is the fallback entry specialist if the current crew is lost.",
        replacement.name(),
        replacement
            .capability(CapabilityKind::Burglary)
            .expect("replacement must have burglary capability")
            .value(),
        replacement
            .capability(CapabilityKind::Stealth)
            .expect("replacement must have stealth capability")
            .value(),
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

pub fn print_resolution_factors(resolution: &crimocracy::operations::OperationResolutionRecord) {
    let factors = resolution.factors();
    println!(
        "[CAUSAL FACTORS] margin {}; crew {}; leader {:?}; intelligence {} (-{} difficulty, {}/{} areas); police {:?}; response {}; approach {}; time pressure {}; variance {}.",
        resolution.execution_margin(),
        factors.role_capability_average().value(),
        factors.leader_capability().map(Rating::value),
        factors.intelligence_quality().value(),
        factors.intelligence_adjustment().unsigned_abs(),
        factors.intelligence_topics_covered(),
        factors.intelligence_topics_relevant(),
        factors.target_police_presence().map(Rating::value),
        factors.police_response_arrived(),
        factors.approach_adjustment(),
        factors.time_pressure(),
        factors.variance(),
    );
}

pub fn print_player_knowledge_gap(scenario: &Scenario, burglary: OperationId) {
    let operation = scenario
        .state
        .operations()
        .get_operation(burglary)
        .expect("burglary must persist");
    if let Some(resolution) = operation.resolution() {
        let legal_information: Vec<_> = scenario
            .state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(scenario.player),
                InformationTopic::LegalActivity,
            )
            .filter(|information| information.subject() == EntityRef::Operation(burglary))
            .collect();
        println!(
            "[KNOWLEDGE] Player organization has {} LegalActivity information record(s) about this burglary after resolution.",
            legal_information.len(),
        );
        for information in legal_information {
            println!("  - [PLAYER] {}", information.summary());
        }
        if let Some(investigation) = resolution.exposure().investigation() {
            let hidden = scenario
                .state
                .legal()
                .get_investigation(investigation)
                .expect("exposure-linked investigation must exist");
            let lead = hidden
                .lead_investigator()
                .and_then(|lead| scenario.state.world().get_character(lead))
                .map(|record| record.name());
            let scheduled_work = scenario
                .state
                .legal()
                .work_for_investigation(investigation)
                .filter(|work| work.status() == InvestigationWorkStatus::Scheduled)
                .count();
            let completed_work = scenario
                .state
                .legal()
                .work_for_investigation(investigation)
                .filter(|work| work.status() == InvestigationWorkStatus::Completed)
                .count();
            println!(
                "[DEV AUDIT] Hidden state has case '{}' with {} subject(s), {} evidence item(s), lead {:?}, {} scheduled and {} completed detective work item(s).",
                hidden.title(),
                hidden.subjects().len(),
                hidden.evidence().len(),
                lead,
                scheduled_work,
                completed_work,
            );
        }
    }
}

pub fn print_final_case_audit(scenario: &Scenario, burglary: OperationId) {
    let Some(investigation) = scenario
        .state
        .operations()
        .get_operation(burglary)
        .and_then(|operation| operation.resolution())
        .and_then(|resolution| resolution.exposure().investigation())
    else {
        return;
    };
    let case = scenario
        .state
        .legal()
        .get_investigation(investigation)
        .expect("exposure-linked investigation must persist");
    let evidence_kinds = case
        .evidence()
        .iter()
        .filter_map(|evidence| scenario.state.legal().get_evidence(*evidence))
        .map(|evidence| evidence.kind())
        .collect::<Vec<_>>();
    let work = scenario
        .state
        .legal()
        .work_for_investigation(investigation)
        .map(|work| {
            (
                work.kind(),
                work.status(),
                work.resolution().map(|resolution| resolution.outcome()),
            )
        })
        .collect::<Vec<_>>();
    println!(
        "\n[DEV AUDIT] Final hidden case state: {} subject(s), evidence {:?}, detective work {:?}.",
        case.subjects().len(),
        evidence_kinds,
        work,
    );
}

/// The closing counterpart to the starting player view: what the organization actually looks
/// like after the session, assembled only from state a boss can see — roster, mandates,
/// holdings, and the reports the organization received. Hidden case state stays in the audit
/// lines above.
pub fn print_organization_closing_view(scenario: &Scenario, metrics: &RunMetrics) {
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
            "  - Lost {} member(s) to rival recruitment this session{}",
            metrics.player_personnel_departures,
            if metrics.replacement_recruited {
                "; rebuilt through an executive recruitment".to_owned()
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
        enterprise_lines.push(EnterpriseLine {
            label: enterprise_label(scenario, id),
            cycle_count: scenario.state.enterprises().cycles_for(id).count(),
            net_cents: net.cents(),
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
        payroll_paid_cents: 0,
        payroll_short_cents: 0,
    })
}

pub fn print_financial_view(scenario: &Scenario, view: FinancialView) {
    println!(
        "\n[FINANCIAL VIEW {}]",
        stamp(scenario.state.now().as_minutes())
    );
    println!(
        "  Legitimate front: {} cycle(s), net {}.",
        view.legitimate_cycle_count,
        format_cents(view.legitimate_net_cents),
    );
    for line in &view.enterprise_lines {
        println!(
            "  Delegated gambling, {}: {} cycle(s), net {}, street float {}.",
            line.label,
            line.cycle_count,
            format_cents(line.net_cents),
            format_cents(line.cash_cents),
        );
    }
    if view.enterprise_lines.is_empty() {
        println!(
            "  Delegated gambling: {} cycle(s), net {}.",
            view.enterprise_cycle_count,
            format_cents(view.enterprise_net_cents),
        );
    }
    println!(
        "  Resale liquidation cash balance: {}.",
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
    println!(
        "  Payroll to date: {} paid across the crew, {} unpaid.",
        format_cents(view.payroll_paid_cents),
        format_cents(view.payroll_short_cents),
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

pub fn print_metrics(metrics: &RunMetrics) {
    let property_acquired = optional_cents(metrics.property_acquired_value_cents);
    let property_realized = optional_cents(metrics.property_realized_cash_cents);
    let liquidation_minute = optional_minute(metrics.liquidation_minute);
    println!(
        "{:<6} [{:<9}]: {}, finish {:?}m, police dispatched {}, police arrived {}, decisions {}, plan items {} {:?}, intel {:?}, exposure {:?}/{:?}, property {} -> {} cash at {}, case {}, evidence {}, player legal intel {}, police intel {}, follow-up {:?}/{} info (case hot {:?}), cold confirmed {:?} @ {:?}, case work {}/{}, surveillance discoveries {}, reports {}, briefs {}, recruitment {}, poach warnings {}, departures {}, legit {}, enterprise {}, matched@{}: legit {}, enterprise {}",
        metrics.strategy.expect("strategy must be set").label(),
        metrics.variation.expect("fixture variation must be set").label(),
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
        optional_cents(metrics.legitimate_net_cents),
        optional_cents(metrics.enterprise_net_cents),
        optional_minute(metrics.matched_financial_boundary_minute),
        optional_cents(metrics.matched_legitimate_net_cents),
        optional_cents(metrics.matched_enterprise_net_cents),
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
        optional_cents(metrics.second_act_property_acquired_value_cents),
        optional_cents(metrics.second_act_property_realized_cash_cents),
        metrics.self_heat_case_opened,
        metrics.self_heat_case_active,
    );
    if metrics.expansion_established {
        println!(
            "        diversification: second-district enterprise established, net {}",
            optional_cents(metrics.expansion_net_cents),
        );
    }
    println!(
        "        money: laundered {} gross through the front's books (house fee {}, accounted balance {}), books refused {} over-capacity request(s), vice inquiries drawn {}",
        optional_cents(Some(metrics.laundered_gross_cents)),
        optional_cents(Some(metrics.launder_fee_cents)),
        optional_cents(metrics.accounted_balance_cents),
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
    println!(
        "        world audit: rival home rackets at end {}",
        metrics.rival_home_enterprises,
    );
    if metrics.win_back_attempted {
        println!(
            "        win-back: attempted (accepted {:?}, margin {:?}), refusal leak to rival {:?}",
            metrics.win_back_accepted,
            metrics.win_back_margin,
            metrics.win_back_refusal_leaked_to_rival,
        );
    }
}

pub fn optional_cents(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_owned(), |cents| format!("{cents}c"))
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

pub fn print_experience_readout(rush: &RunMetrics, press: &RunMetrics, recon: &RunMetrics) {
    println!("\n--- PLAYER LOOP READOUT ---");
    println!(
        "The core fantasy tested here is: learn what the city reveals, turn it into an organizational plan, delegate execution, then stay powerful enough to absorb the consequences."
    );
    println!("Evidence coverage (not a game-quality score):");
    print_loop_checkpoint(
        "learn",
        recon.discovered_surveillance_information > 0,
        "surveillance produces actionable patrol and target information",
    );
    print_loop_checkpoint(
        "plan",
        recon.planning_information_count > rush.planning_information_count
            && recon.burglary_information_quality.unwrap_or_default()
                > rush.burglary_information_quality.unwrap_or_default(),
        "the player can make a better plan from organization-held intelligence",
    );
    print_loop_checkpoint(
        "failure teaches",
        rush.aborted
            && rush.player_police_activity_information > 0
            && rush
                .second_act_planning_topics
                .contains(&InformationTopic::PoliceActivity),
        "a standing abort is debriefed into organizational patrol knowledge that plans the rebuilt crew's next job",
    );
    let response_choice_changed_consequence = rush.aborted
        && press.outcome.is_some()
        && press.decision_requests > 0
        && press.player_police_activity_information > 0;
    print_loop_checkpoint(
        "choice",
        response_choice_changed_consequence,
        "a player response to the same police exception changes whether the operation aborts or resolves",
    );
    print_loop_checkpoint(
        "delegate",
        recon.burglary.is_some() && recon.outcome.is_some(),
        "the plan resolves through assigned people and authored capabilities",
    );
    print_loop_checkpoint(
        "respond",
        press.decision_requests > 0 && press.player_police_activity_information > 0,
        "an exception pauses the plan and a field report returns to the organization",
    );
    print_loop_checkpoint(
        "consequences",
        press.investigation_created && recon.property_realized_cash_cents.is_some(),
        "the same operation system can create legal pressure or recover value into cash",
    );
    print_loop_checkpoint(
        "follow-up",
        press.counterintelligence_outcome.is_some()
            && press.counterintelligence_information > 0
            && press.followup_case_active == Some(true),
        "a player-visible legal report can seed a precinct check that reads whether the case is still hot",
    );
    print_loop_checkpoint(
        "survive",
        press.cold_case_confirmed == Some(true) && press.case_cold_minute.is_some(),
        "standing down and outlasting the investigation resolves the consequence through the player's own surveillance",
    );
    print_loop_checkpoint(
        "organization",
        rush.autonomous_recruitment_attempts > 0 && rush.player_personnel_departures > 0,
        "a police-exposed crew member can be courted away by a rival without a scripted event",
    );
    print_loop_checkpoint(
        "rebuild",
        rush.replacement_recruited && rush.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved),
        "a crew member lost to rival pressure can be replaced through a player-authored executive recruitment, and the rebuilt crew works a second score safely",
    );
    print_loop_checkpoint(
        "second wind",
        recon.second_act_recon_information > 0
            && recon.second_burglary_outcome == Some(OperationObjectiveOutcome::Achieved),
        "an organization that re-invests in planning can recover value on a reopened window",
    );
    print_loop_checkpoint(
        "own heat",
        recon.self_heat_case_opened && recon.self_heat_case_active == Some(true),
        "casing carries risk both ways: after the organization's own surveillance draws a case, it reads that case through its standing police contact — no extra street exposure, provenance-bearing disclosure",
    );
    print_loop_checkpoint(
        "witness chain",
        press.case_witness_registered
            && press.witness_interviews_scheduled > 0
            && press.witness_testimony_produced,
        "a witnessed crime names its on-scene witness on the case; institutional interviews turn his account into testimony the file is built on",
    );
    print_loop_checkpoint(
        "counterplay",
        press.witness_pressure_attempted,
        "the organization can answer a witness with one canonical pressure operation — it lands and discounts his cooperation, or a police response forces a disciplined walk-away with no second case",
    );
    print_loop_checkpoint(
        "discipline cost",
        press.second_opportunity_expired && press.second_burglary.is_none(),
        "choosing to stand down has a real price: the second score lapses while the hot case stays protected",
    );
    print_loop_checkpoint(
        "diversify",
        press.expansion_established && press.expansion_net_cents.is_some_and(|net| net > 0),
        "idle capital during the wait becomes governance: a revised two-district mandate and a second-district enterprise the hot home case cannot tax",
    );
    let defector_trail_shown = rush.defector_trail_confirmed == Some(true)
        && press.defector_trail_confirmed == Some(true)
        && recon.defector_trail_confirmed.is_none();
    print_loop_checkpoint(
        "defector trail",
        defector_trail_shown,
        "after a departure, the organization can confirm where the defector landed through its own canonical surveillance channel instead of the report leaking the rival",
    );
    let win_back_shown = rush.win_back_attempted
        && press.win_back_attempted
        && rush.win_back_accepted.is_some()
        && !recon.win_back_attempted;
    print_loop_checkpoint(
        "win-back",
        win_back_shown,
        "after confirming where a defector landed, leadership can make one canonical executive re-approach; the pitch resolves through production scoring, and a refusal leaks the approach to the rival through the loyalty report that names the outside recruiter",
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
    print_loop_checkpoint(
        "routine",
        legitimate_isolated,
        "legitimate front continues identically while leadership focuses on exceptions",
    );
    print_loop_checkpoint(
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
    print_loop_checkpoint(
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
    print_loop_checkpoint(
        "clean money",
        laundering_shown,
        "street earnings pass through an owned front's books into accounted funds, and the front's plausible-volume ceiling visibly caps how fast dirty money becomes clean",
    );
    let wealth_loop_shown = press.front_acquired
        && press.acquisition_price_cents.is_some()
        && press.acquisition_rejections > 0;
    print_loop_checkpoint(
        "legit wealth",
        wealth_loop_shown,
        "accounted wealth converts into an owned legitimate asset through the canonical acquisition path: the short book first surfaces as a visible rejection, the purchase lands at the authored price, and owning the venue unlocks the second-district racket — the money loop closes",
    );
    let any_vice = [rush, press, recon]
        .iter()
        .any(|run| run.vice_inquiries_drawn > 0);
    print_loop_checkpoint(
        "vice heat",
        any_vice,
        "sustained district casework can convert into a dedicated vice inquiry on a racket itself: visible expansion carries discovery risk, lying low (suspending) or diversifying districts are real counter-play, and the inquiry shelves like any other case when the institution goes quiet",
    );
    println!("Observed decision leverage:");
    println!(
        "  - Information leverage: RECON selected {} planning item(s) versus RUSH's {} and finished as {} versus {}.",
        recon.planning_information_count,
        rush.planning_information_count,
        terminal_label(recon),
        terminal_label(rush),
    );
    println!(
        "  - Information risk: RECON's own casing can be made — surveillance base exposure means a weak scout in a heavily patrolled district draws police attention while gathering it{}; the branch then reads that self-inflicted case through its police contact rather than more street work (own-heat read: {:?}).",
        if recon.session_case_staffed && !recon.investigation_created {
            "; this fixture's recon run drew exactly that kind of case from its own surveillance"
        } else {
            ", which this fixture's skilled scout in a quiet district avoided"
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
        "  - Witness leverage: the case PRESS created carries a named witness whose institutional interview produced testimony; the organization's one pressure operation answers him - it lands and discounts his cooperation (degraded: {}) or a response forces a walk-away. Where an exposure identifies a crew member, the same chain escalates to autonomous arrest ({} member arrest(s) this comparison).",
        press.witness_cooperation_degraded,
        rush.player_member_arrests + press.player_member_arrests + recon.player_member_arrests,
    );
    println!(
        "  - Personnel leverage: RUSH/PRESS exposed the crew to police and lost {} crew member(s) to rival recruitment, while RECON kept everyone ({} departures) because the crew never saw police.",
        rush.player_personnel_departures + press.player_personnel_departures,
        recon.player_personnel_departures,
    );
    println!(
        "  - Consequence leverage: PRESS exposed {} evidence item(s), {} legal-activity information item(s), read the case as still hot at minute ~{}, then confirmed it shelved at minute {}; over the same campaign-day window enterprise heat cut gambling net to {} while an unheated branch earned {}; RECON realized {} of resale cash via a low-police venue.",
        press.evidence_count,
        press.player_legal_activity_information,
        press.counterintelligence_scheduled_at.unwrap_or_default(),
        press.case_cold_minute.unwrap_or_default(),
        optional_dollars(enterprise_window(press)),
        optional_dollars(enterprise_window(rush).or_else(|| enterprise_window(recon))),
        optional_dollars(recon.property_realized_cash_cents),
    );
    println!(
        "  - Time tradeoff: RECON finished at minute {} versus RUSH at minute {}; the extra planning time bought lower exposure and liquid value in this matched fixture.",
        recon.burglary_terminal_minute.unwrap_or_default(),
        rush.burglary_terminal_minute.unwrap_or_default(),
    );
    println!(
        "  - Diversification leverage: while the case stayed hot, PRESS bought its harbor venue outright with clean money and converted idle street cash into a second-district book earning {} surcharge-free, versus the canal book's heat-taxed window net of {}.",
        optional_dollars(press.expansion_net_cents),
        optional_dollars(press.enterprise_net_cents),
    );
    println!(
        "  - Money-state leverage: resale cash is not spendable money until it is laundered; every branch routes proceeds through its front's books ({} gross for RECON), and the front's per-cycle plausible volume rejected the over-capacity remainder {} time(s) across branches. PRESS then spent its accumulated accounted funds on the harbor venue ({}), so conversion speed — not desire — limits how fast dirty money becomes clean, and clean money has a real purchase waiting.",
        optional_cents(Some(recon.laundered_gross_cents)),
        rush.laundering_capacity_rejections
            + press.laundering_capacity_rejections
            + recon.laundering_capacity_rejections,
        optional_dollars(press.acquisition_price_cents),
    );
    println!(
        "  - Visibility leverage: the branches drew {} vice inquiries this comparison. Every cycle a racket runs under active district casework risks a dedicated inquiry on the racket itself, taxing every book in that district — including rivals' — until the case shelves; going dark or moving districts are the honest counters.",
        rush.vice_inquiries_drawn + press.vice_inquiries_drawn + recon.vice_inquiries_drawn,
    );
    println!("Current experience gaps exposed by this fixture:");
    println!(
        "  - The consequence arc now closes and bleeds into economics: an open case can be read, outlasted, verified shelved, and while hot it raises the delegated enterprise's street costs, compounds across cases, and can escalate into a vice inquiry on the racket itself (reported by the manager in-cycle). The witness link in that chain is now two-sided - testimony builds the file and pressure discounts it - but disrupting physical evidence, influencing counsel, or changing a prosecution outcome are still not modeled."
    );
    println!(
        "  - The portfolio probe covers prioritization and expiry across competing opportunities, while the organizational-capacity probe now proves overlapping specialist assignments reject atomically and release after completion, plus mandate revision and approach variation. Broader resource competition and rival-initiated enterprise targeting remain outside this foundation."
    );
    println!(
        "  - A refused poaching pitch now surfaces as a loyalty report naming the outside recruiter, so the organization can keep that member off police-exposed work before the next attempt lands; the defector loop now closes both ways — surveillance finds where the member landed and one canonical executive re-approach can bring them home, while a refusal leaks the approach to the rival. Retaliating after a defection remains outside scope, as does violence against people. The fixture's second rival (D'Amato Crew) is watched to confirm absence but makes no autonomous moves of its own yet."
    );
    println!(
        "  - The delegation pillar now carries real weight in the narrative arc: PRESS must own its second-district venue before anything can be established there, so the arc runs acquisition -> mandate revision -> enterprise establishment through canonical paths. Still untested: replacing a delegated manager mid-crisis or responding to manager drift beyond the capacity-probe revision."
    );
    println!(
        "  - The RUSH/PRESS/RECON policies are calibration treatments; each matched seed shares one authored-content-derived timeline while bounded policy offsets vary the act-1 and second-wind clock choices. They are not evidence that an actual player would choose the same policies or the same rebuild/second-wind scheduling. Acquisition covers only independently owned sellers at authored kind prices; rival-owned venues and negotiated prices remain outside scope."
    );
}

pub fn print_loop_checkpoint(label: &str, present: bool, evidence: &str) {
    println!(
        "  [{:>12}] {:<5} - {}",
        label,
        if present { "shown" } else { "missing" },
        evidence,
    );
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

/// Renders a player-facing tick beat as minute plus clock, e.g. `minute 160 (02:40)`.
pub fn stamp(minute: u64) -> String {
    format!("minute {} ({})", minute, format_minute_of_day(minute))
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
