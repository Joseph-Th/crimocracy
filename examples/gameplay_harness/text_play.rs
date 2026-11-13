//! Line-oriented player client. The scripted evaluator is not involved in command selection.
//!
//! The initial campaign uses the existing canonical authored scenario. All subsequent
//! actions run through the same production validation, commit and minute-tick owners
//! as the evaluation harness. Views contain only the player's staff, books, reports,
//! and organization-held intelligence; scenario diagnostics never enter this client.

use crate::{
    EvaluationSeeds, Scenario, ScenarioProfile, Strategy, authorize_burglary,
    authorize_surveillance_target, build_scenario,
};
use crimocracy::contacts::contact_system::{
    find_pending_disclosure_sources, validate_contact_disclosure,
};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::BusinessId;
use crimocracy::core::persistence::{SaveEnvelope, build_save, restore_save};
use crimocracy::core::simulation::run_tick;
use crimocracy::core::time::{DAY_MINUTES, SimDuration};
use crimocracy::decisions::DecisionResponse;
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::economy::business_economy_system::{
    BusinessProfitSweepDraft, validate_sweep_business_profits,
};
use crimocracy::finance::{AccountKind, FinancialOwner, Money};
use crimocracy::intelligence::KnowledgeHolder;
use crimocracy::operations::{OperationKind, OperationStatus};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
    validate_dismiss_opportunity,
};
use crimocracy::opportunities::{OperationOpportunityDraft, OpportunityStatus};
use crimocracy::registry::Registry;
use crimocracy::world::ALL_CAPABILITY_KINDS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};

const SAVE_DIR: &str = "target/text-play";
const MAX_ADVANCE: u64 = DAY_MINUTES * 7;

const HELP: &str = "Commands:
  brief                  Leadership's current situation
  people                 Your current organization members
  businesses             Owned storefronts and known leads
  finances               Your actual organization accounts
  intelligence           Information held by your organization
  opportunities          Known burglary leads and their deadlines
  operations             Your authorized and completed operations
  decisions              Pending decisions requiring leadership
  reports [count]        Most recent organization reports (default 8)
  report <id>            Read a complete organization report
  scout <business-id>    Order the scout to observe a known business
  burgle <lead-id> [abort|press]  Authorize a burglary of an open lead
  dismiss <lead-id>      Reject a lead through its canonical lifecycle
  contact                Ask the established police contact for one new disclosure
  decide <id> <continue|abort|approve|reject>
                         Resolve a pending decision
  sweep <business-id> <cents>  Withdraw earned storefront surplus to clean books
  advance <minutes|Nh|Nd>     Advance up to seven days, stopping for decisions
  save <slot>            Write a validated current-schema campaign save
  quit                   End this session

IDs are the numbers printed beside records, not guessed hidden database keys.
Commands can be typed or replayed with --script. Use --help for startup options.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignFile {
    world_seed: u64,
    policy_seed: u64,
    campaign: SaveEnvelope,
}

struct Options {
    seed: Option<u64>,
    policy_seed: Option<u64>,
    load: Option<String>,
    script: Option<PathBuf>,
    brief: bool,
    help: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        seed: None,
        policy_seed: None,
        load: None,
        script: None,
        brief: false,
        help: false,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = |flag: &str, args: &mut std::slice::Iter<'_, String>| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--help" | "-h" => options.help = true,
            "--seed" => {
                let raw = value("--seed", &mut args)?;
                options.seed =
                    Some(crate::parse_seed_value("--seed", &raw).map_err(|e| e.to_string())?);
            }
            "--policy-seed" => {
                let raw = value("--policy-seed", &mut args)?;
                options.policy_seed = Some(
                    crate::parse_seed_value("--policy-seed", &raw).map_err(|e| e.to_string())?,
                );
            }
            "--load" => options.load = Some(value("--load", &mut args)?),
            "--script" => options.script = Some(PathBuf::from(value("--script", &mut args)?)),
            "--brief" => options.brief = true,
            other => return Err(format!("unknown text-play option '{other}'")),
        }
    }
    if options.load.is_some() && (options.seed.is_some() || options.policy_seed.is_some()) {
        return Err("--load cannot be combined with seed options".to_owned());
    }
    Ok(options)
}

fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    std::time::SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    hasher.finish()
}

fn save_path(slot: &str) -> Result<PathBuf, String> {
    if slot.is_empty()
        || slot.len() > 64
        || !slot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("slot must have 1-64 ASCII letters, digits, '-' or '_'".to_owned());
    }
    Ok(Path::new(SAVE_DIR).join(format!("{slot}.sav")))
}

fn time_label(minutes: u64) -> String {
    let clock = minutes % DAY_MINUTES;
    format!(
        "day {} {:02}:{:02}",
        minutes / DAY_MINUTES + 1,
        clock / 60,
        clock % 60
    )
}

fn parse_number(raw: &str) -> Result<u32, String> {
    let raw = raw.rsplit_once('-').map_or(raw, |(_, digits)| digits);
    raw.parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("invalid record ID '{raw}'"))
}

fn parse_advance(raw: &str) -> Result<u64, String> {
    let (amount, factor) = if let Some(amount) = raw.strip_suffix('d') {
        (amount, DAY_MINUTES)
    } else if let Some(amount) = raw.strip_suffix('h') {
        (amount, 60)
    } else if let Some(amount) = raw.strip_suffix('m') {
        (amount, 1)
    } else {
        (raw, 1)
    };
    amount
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(factor))
        .filter(|minutes| (1..=MAX_ADVANCE).contains(minutes))
        .ok_or_else(|| "advance requires 1 to 10080 minutes (e.g. 45m, 6h, 2d)".to_owned())
}

struct Session<'a> {
    scenario: Scenario<'a>,
}

impl<'a> Session<'a> {
    fn new(registry: &'a Registry, seeds: EvaluationSeeds) -> Result<Self, String> {
        let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)
            .map_err(|e| e.to_string())?;
        for (target, source, summary, deadline) in [
            (
                scenario.target,
                scenario.opportunity_information,
                "Street word identifies an accessible store. The opening window is short.",
                Some(scenario.timeline.initial_opportunity_valid_until),
            ),
            (
                scenario.alternate_target,
                scenario.alternate_opportunity_information,
                "A second shop has an independently observed opening. This lead has no known expiry.",
                None,
            ),
        ] {
            validate_discover_operation_opportunity(
                registry,
                &scenario.state,
                OperationOpportunityDraft {
                    organization: scenario.player,
                    operation_kind: OperationKind::Burglary,
                    targets: BTreeSet::from([EntityRef::Business(target)]),
                    source_information: BTreeSet::from([source]),
                    summary: summary.to_owned(),
                    valid_until: deadline,
                },
            )
            .map_err(|e| e.to_string())?
            .commit(&mut scenario.state)
            .map_err(|e| e.to_string())?;
        }
        Ok(Self { scenario })
    }

    fn load(registry: &'a Registry, slot: &str) -> Result<Self, String> {
        let path = save_path(slot)?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let file: CampaignFile =
            bincode::deserialize(&bytes).map_err(|e| format!("invalid campaign file: {e}"))?;
        let seeds = EvaluationSeeds::new(file.world_seed, file.policy_seed);
        let mut scenario = build_scenario(registry, seeds, ScenarioProfile::NightTrap)
            .map_err(|e| e.to_string())?;
        let state = restore_save(registry, file.campaign).map_err(|e| e.to_string())?;
        if state.player_organization() != Some(scenario.player) {
            return Err("saved campaign belongs to another player organization".to_owned());
        }
        scenario.state = state;
        Ok(Self { scenario })
    }

    fn save(&self, slot: &str) -> Result<String, String> {
        let path = save_path(slot)?;
        let campaign =
            build_save(self.scenario.registry, &self.scenario.state).map_err(|e| e.to_string())?;
        let data = bincode::serialize(&CampaignFile {
            world_seed: self.scenario.seeds.world,
            policy_seed: self.scenario.seeds.policy,
            campaign,
        })
        .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(SAVE_DIR).map_err(|e| e.to_string())?;
        let temp = path.with_extension("sav.tmp");
        std::fs::write(&temp, data).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("cannot publish save: {error}"));
        }
        Ok(format!("Campaign saved to {}", path.display()))
    }

    fn organization_information(
        &self,
    ) -> impl Iterator<Item = &crimocracy::intelligence::InformationRecord> {
        self.scenario
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(self.scenario.player))
    }

    fn known_business(&self, raw: u32) -> Option<BusinessId> {
        let world = self.scenario.state.world();
        world
            .businesses_owned_by_organization(self.scenario.player)
            .map(|business| business.id())
            .chain(self.organization_information().filter_map(|information| {
                match information.subject() {
                    EntityRef::Business(business) => Some(business),
                    EntityRef::Organization(_)
                    | EntityRef::Character(_)
                    | EntityRef::Neighborhood(_)
                    | EntityRef::Operation(_)
                    | EntityRef::Investigation(_)
                    | EntityRef::Evidence(_)
                    | EntityRef::FinancialAccount(_)
                    | EntityRef::DecisionRequest(_)
                    | EntityRef::Mandate(_)
                    | EntityRef::Enterprise(_) => None,
                }
            }))
            .find(|id| id.raw() == raw)
    }

    fn known_opportunities(&self) -> Vec<&crimocracy::opportunities::OpportunityRecord> {
        self.scenario
            .state
            .reports()
            .reports_for(self.scenario.player)
            .filter_map(|report| {
                self.scenario
                    .state
                    .opportunities()
                    .opportunity_for_report(report.id())
            })
            .filter(|opportunity| opportunity.organization() == self.scenario.player)
            .map(|opportunity| (opportunity.id(), opportunity))
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    fn brief(&self) -> String {
        let state = &self.scenario.state;
        let player = self.scenario.player;
        let name = state
            .world()
            .get_organization(player)
            .map_or("Your organization", |o| o.name());
        let pending = state.decisions().pending_for_recipient(player).count();
        let active = state
            .operations()
            .operations_for_organization(player)
            .filter(|op| {
                !matches!(
                    op.status(),
                    OperationStatus::Completed | OperationStatus::Aborted
                )
            })
            .count();
        let leads = self
            .known_opportunities()
            .into_iter()
            .filter(|opp| opp.status() == OpportunityStatus::Open)
            .count();
        format!(
            "{name} | {}\nPending decisions: {pending} | Active operations: {active} | Open leads: {leads}\nUse 'reports', 'opportunities', 'people', or 'finances' to investigate.",
            time_label(state.now().as_minutes())
        )
    }

    fn execute(&mut self, command: &str) -> Result<(String, bool), String> {
        let words: Vec<_> = command.split_whitespace().collect();
        let Some(verb) = words.first().copied() else {
            return Ok((String::new(), false));
        };
        let state = &self.scenario.state;
        let mut out = String::new();
        match verb.to_ascii_lowercase().as_str() {
            "help" => out.push_str(HELP),
            "quit" | "exit" => {
                return Ok((
                    "Session ended. Save explicitly to continue later.".to_owned(),
                    true,
                ));
            }
            "brief" => out = self.brief(),
            "people" => {
                for person in state
                    .world()
                    .characters_in_organization(self.scenario.player)
                {
                    let supervisor = person
                        .supervisor()
                        .and_then(|id| state.world().get_character(id))
                        .map_or("none", |person| person.name());
                    writeln!(
                        out,
                        "{} {} | autonomy {:?} | reports to {}",
                        person.id().raw(),
                        person.name(),
                        person.autonomy(),
                        supervisor
                    )
                    .unwrap();
                    let capabilities = ALL_CAPABILITY_KINDS
                        .into_iter()
                        .filter_map(|kind| {
                            person
                                .capability(kind)
                                .map(|rating| format!("{kind:?} {}", rating.value()))
                        })
                        .collect::<Vec<_>>();
                    if !capabilities.is_empty() {
                        writeln!(out, "  Capabilities: {}", capabilities.join(", ")).unwrap();
                    }
                }
            }
            "businesses" => {
                out.push_str("Owned storefronts:\n");
                for business in state
                    .world()
                    .businesses_owned_by_organization(self.scenario.player)
                {
                    writeln!(
                        out,
                        "{} {} | {:?}",
                        business.id().raw(),
                        business.name(),
                        business.kind()
                    )
                    .unwrap();
                }
                out.push_str("Known business leads:\n");
                let mut seen = BTreeSet::new();
                for information in self.organization_information() {
                    if let EntityRef::Business(id) = information.subject()
                        && seen.insert(id)
                        && self.known_business(id.raw()).is_some()
                        && let Some(business) = state.world().get_business(id)
                    {
                        writeln!(
                            out,
                            "{} {} (source: {})",
                            id.raw(),
                            business.name(),
                            information.id().raw()
                        )
                        .unwrap();
                    }
                }
            }
            "finances" => {
                let mut totals = std::collections::BTreeMap::<AccountKind, i128>::new();
                for account in state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.scenario.player))
                {
                    *totals.entry(account.kind()).or_default() +=
                        i128::from(account.balance().cents());
                }
                for kind in [
                    AccountKind::StreetCash,
                    AccountKind::ConcealedCash,
                    AccountKind::AccountedFunds,
                ] {
                    writeln!(
                        out,
                        "{kind:?}: {} cents",
                        totals.get(&kind).copied().unwrap_or_default()
                    )
                    .unwrap();
                }
                out.push_str("Clearing/settlement balances are excluded; their ledger entries are not spendable reserves.");
            }
            "intelligence" => {
                for info in self.organization_information() {
                    writeln!(
                        out,
                        "{} {:?} on {:?} | observed {} | {:?}/{:?}: {}",
                        info.id().raw(),
                        info.topic(),
                        info.subject(),
                        time_label(info.observed_at().as_minutes()),
                        info.reliability(),
                        info.specificity(),
                        info.summary()
                    )
                    .unwrap();
                }
            }
            "opportunities" => {
                for lead in self.known_opportunities() {
                    let target = lead
                        .context()
                        .targets()
                        .iter()
                        .map(|entity| match entity {
                            EntityRef::Business(id) => state.world().get_business(*id).map_or_else(
                                || format!("business {}", id.raw()),
                                |business| format!("{} ({})", business.name(), id.raw()),
                            ),
                            other @ (EntityRef::Organization(_)
                            | EntityRef::Character(_)
                            | EntityRef::Neighborhood(_)
                            | EntityRef::Operation(_)
                            | EntityRef::Investigation(_)
                            | EntityRef::Evidence(_)
                            | EntityRef::FinancialAccount(_)
                            | EntityRef::DecisionRequest(_)
                            | EntityRef::Mandate(_)
                            | EntityRef::Enterprise(_)) => format!("{other:?}"),
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(
                        out,
                        "{} {:?} {:?} | target {} | expires {}\n  {}",
                        lead.id().raw(),
                        lead.status(),
                        lead.context().operation_kind(),
                        target,
                        lead.valid_until()
                            .map_or("not specified".to_owned(), |at| time_label(at.as_minutes())),
                        lead.summary()
                    )
                    .unwrap();
                }
            }
            "operations" => {
                for operation in state
                    .operations()
                    .operations_for_organization(self.scenario.player)
                {
                    writeln!(
                        out,
                        "{} {} | {:?} | scheduled {}",
                        operation.id().raw(),
                        operation.title(),
                        operation.status(),
                        time_label(operation.scheduled_for().as_minutes())
                    )
                    .unwrap();
                }
            }
            "decisions" => {
                for decision in state
                    .decisions()
                    .pending_for_recipient(self.scenario.player)
                {
                    writeln!(
                        out,
                        "{} {:?} | {} | options {:?}",
                        decision.id().raw(),
                        decision.attention(),
                        decision.summary(),
                        decision.options()
                    )
                    .unwrap();
                }
            }
            "reports" => {
                let count = words
                    .get(1)
                    .map_or(Ok(8_usize), |value| {
                        value
                            .parse::<usize>()
                            .map_err(|_| "reports needs a count".to_owned())
                    })?
                    .clamp(1, 40);
                for report in state
                    .reports()
                    .reports_for(self.scenario.player)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(count)
                {
                    writeln!(
                        out,
                        "{} {:?} {} | {}",
                        report.id().raw(),
                        report.kind(),
                        time_label(report.generated_at().as_minutes()),
                        report.title()
                    )
                    .unwrap();
                }
            }
            "report" => {
                let id = parse_number(words.get(1).ok_or("report needs an ID")?)?;
                let report = state
                    .reports()
                    .reports_for(self.scenario.player)
                    .find(|report| report.id().raw() == id)
                    .ok_or("no report with that ID for your organization")?;
                writeln!(
                    out,
                    "{} | {:?} | {}",
                    report.title(),
                    report.kind(),
                    time_label(report.generated_at().as_minutes())
                )
                .unwrap();
                for entry in report.entries() {
                    writeln!(out, "  [{:?}] {}", entry.attention, entry.summary).unwrap();
                }
            }
            "scout" => {
                let id = parse_number(words.get(1).ok_or("scout needs a known business ID")?)?;
                let business = self
                    .known_business(id)
                    .ok_or("no known business with that ID")?;
                let at = state
                    .now()
                    .checked_add(SimDuration::ONE_MINUTE)
                    .ok_or("simulation clock exhausted")?;
                let name = state
                    .world()
                    .get_business(business)
                    .ok_or("known business disappeared")?
                    .name()
                    .to_owned();
                let operation = authorize_surveillance_target(
                    &mut self.scenario,
                    EntityRef::Business(business),
                    &format!("Observe {name}"),
                    at,
                    BTreeSet::new(),
                )
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Scouting authorized as operation {}. Use 'advance' to see results.",
                    operation.raw()
                )
                .unwrap();
            }
            "burgle" => {
                let raw = parse_number(words.get(1).ok_or("burgle needs a lead ID")?)?;
                let posture = match words.get(2).copied().unwrap_or("abort") {
                    "abort" => Strategy::Rush,
                    "press" => Strategy::Press,
                    _ => return Err("use 'burgle <lead-id> [abort|press]'".to_owned()),
                };
                let lead = self
                    .known_opportunities()
                    .into_iter()
                    .find(|opp| opp.id().raw() == raw && opp.status() == OpportunityStatus::Open)
                    .ok_or("lead is not open or not known")?;
                if lead.context().operation_kind() != OperationKind::Burglary {
                    return Err("lead is not for burglary".to_owned());
                }
                let target = match lead.context().targets().iter().next() {
                    Some(EntityRef::Business(id)) if lead.context().targets().len() == 1 => *id,
                    _ => return Err("burglary requires one business target".to_owned()),
                };
                let lead_id = lead.id();
                let name = state
                    .world()
                    .get_business(target)
                    .ok_or("lead target disappeared")?
                    .name()
                    .to_owned();
                let neighborhood = state
                    .world()
                    .get_business(target)
                    .ok_or("lead target disappeared")?
                    .neighborhood();
                let relevant = self
                    .scenario
                    .registry
                    .get_operation(OperationKind::Burglary)
                    .execution()
                    .relevant_intelligence_topics();
                let intelligence: BTreeSet<_> = self
                    .organization_information()
                    .filter(|info| {
                        relevant.contains(&info.topic())
                            && (matches!(info.subject(),
                        EntityRef::Business(id) if id == target)
                                || matches!(info.subject(),
                        EntityRef::Neighborhood(id) if id == neighborhood))
                    })
                    .map(|info| info.id())
                    .collect();
                let information_count = intelligence.len();
                let at = state
                    .now()
                    .checked_add(SimDuration::ONE_MINUTE)
                    .ok_or("simulation clock exhausted")?;
                if lead.valid_until().is_some_and(|deadline| at >= deadline) {
                    return Err("lead expires before the proposed operation can begin".to_owned());
                }
                let entry_specialist = self.scenario.burglar;
                let operation = authorize_burglary(
                    &mut self.scenario,
                    posture,
                    target,
                    &format!("Burglary at {name}"),
                    at,
                    intelligence,
                    entry_specialist,
                )
                .map_err(|e| e.to_string())?;
                validate_convert_opportunity(&self.scenario.state, lead_id, operation)
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.scenario.state)
                    .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Burglary {} authorized with {} information item(s); lead {} converted. Contingency: {:?}.",
                    operation.raw(),
                    information_count,
                    lead_id.raw(),
                    posture
                )
                .unwrap();
            }
            "dismiss" => {
                let raw = parse_number(words.get(1).ok_or("dismiss needs a lead ID")?)?;
                let lead = self
                    .known_opportunities()
                    .into_iter()
                    .find(|opp| opp.id().raw() == raw)
                    .ok_or("unknown lead")?;
                validate_dismiss_opportunity(
                    self.scenario.registry,
                    &self.scenario.state,
                    lead.id(),
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.scenario.state)
                .map_err(|e| e.to_string())?;
                writeln!(out, "Lead {raw} dismissed.").unwrap();
            }
            "contact" => {
                let sources = find_pending_disclosure_sources(
                    &self.scenario.state,
                    self.scenario.police_contact,
                );
                match sources.first().copied() {
                    None => out.push_str("Your police contact has nothing new to disclose."),
                    Some(source) => {
                        let disclosure = validate_contact_disclosure(
                            &self.scenario.state,
                            self.scenario.police_contact,
                            source,
                        )
                        .map_err(|e| e.to_string())?
                        .commit(&mut self.scenario.state)
                        .map_err(|e| e.to_string())?;
                        let information = self
                            .scenario
                            .state
                            .contacts()
                            .get_disclosure(disclosure)
                            .ok_or("disclosure record missing")?
                            .disclosed_information();
                        let fact = self
                            .scenario
                            .state
                            .intelligence()
                            .get_information(information)
                            .ok_or("disclosed information missing")?;
                        writeln!(
                            out,
                            "Contact disclosed {}: {}",
                            fact.id().raw(),
                            fact.summary()
                        )
                        .unwrap();
                    }
                }
            }
            "decide" => {
                let raw = parse_number(words.get(1).ok_or("decide needs a decision ID")?)?;
                let response = match words.get(2).copied() {
                    Some("continue") => DecisionResponse::Continue,
                    Some("abort") => DecisionResponse::Abort,
                    Some("approve") => DecisionResponse::Approve,
                    Some("reject") => DecisionResponse::Reject,
                    _ => return Err("decide <id> <continue|abort|approve|reject>".to_owned()),
                };
                let decision = state
                    .decisions()
                    .pending_for_recipient(self.scenario.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("no pending decision for your organization")?
                    .id();
                validate_resolve_decision(
                    self.scenario.registry,
                    &self.scenario.state,
                    decision,
                    self.scenario.player,
                    response,
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.scenario.state)
                .map_err(|e| e.to_string())?;
                writeln!(out, "Decision {raw} resolved: {response:?}.").unwrap();
            }
            "sweep" => {
                let raw = parse_number(words.get(1).ok_or("sweep needs a business ID")?)?;
                let cents = words
                    .get(2)
                    .ok_or("sweep needs a positive amount in cents")?
                    .parse::<i64>()
                    .map_err(|_| "invalid cents amount")?;
                let business = state
                    .world()
                    .businesses_owned_by_organization(self.scenario.player)
                    .find(|b| b.id().raw() == raw)
                    .ok_or("not an owned business")?
                    .id();
                let destination = state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.scenario.player))
                    .find(|account| account.kind() == AccountKind::AccountedFunds)
                    .ok_or("organization has no accounted-funds account")?
                    .id();
                validate_sweep_business_profits(
                    &self.scenario.state,
                    BusinessProfitSweepDraft {
                        organization: self.scenario.player,
                        business,
                        destination,
                        amount: Money::from_cents(cents),
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.scenario.state)
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Withdrew {cents} cents of actual earned business surplus."
                )
                .unwrap();
            }
            "advance" | "wait" => {
                let span = parse_advance(words.get(1).ok_or("advance needs a duration")?)?;
                if self
                    .scenario
                    .state
                    .decisions()
                    .pending_for_recipient(self.scenario.player)
                    .next()
                    .is_some()
                {
                    return Err("resolve your pending decisions before advancing".to_owned());
                }
                let mut advanced = 0;
                let mut new_reports = 0;
                let mut reason = None;
                for _ in 0..span {
                    let previous = self
                        .scenario
                        .state
                        .reports()
                        .reports_for(self.scenario.player)
                        .last()
                        .map(|r| r.id());
                    let tick = run_tick(self.scenario.registry, &mut self.scenario.state)
                        .map_err(|e| e.to_string())?;
                    advanced += 1;
                    let reports: Vec<_> = self
                        .scenario
                        .state
                        .reports()
                        .reports_for_after(self.scenario.player, previous)
                        .collect();
                    new_reports += reports.len();
                    let urgent = reports.iter().any(|report| {
                        report.entries().iter().any(|entry| {
                            entry.attention.can_auto_pause()
                                && self
                                    .scenario
                                    .state
                                    .attention_settings()
                                    .is_auto_pause_enabled(entry.attention)
                        })
                    });
                    let operation_ended = tick
                        .resolved_operations
                        .iter()
                        .chain(&tick.aborted_operations)
                        .any(|id| {
                            self.scenario
                                .state
                                .operations()
                                .get_operation(*id)
                                .is_some_and(|op| {
                                    op.responsible_organization() == self.scenario.player
                                })
                        });
                    if self
                        .scenario
                        .state
                        .decisions()
                        .pending_for_recipient(self.scenario.player)
                        .next()
                        .is_some()
                    {
                        reason = Some("leadership decision pending");
                    } else if urgent {
                        reason = Some("high-priority report");
                    } else if operation_ended {
                        reason = Some("your operation reached a terminal outcome");
                    }
                    if reason.is_some() {
                        break;
                    }
                }
                writeln!(
                    out,
                    "Advanced {advanced} minute(s) to {}. {new_reports} new report(s).",
                    time_label(self.scenario.state.now().as_minutes())
                )
                .unwrap();
                if let Some(reason) = reason {
                    writeln!(out, "Paused: {reason}.").unwrap();
                }
                for report in self
                    .scenario
                    .state
                    .reports()
                    .reports_for(self.scenario.player)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(new_reports.min(6))
                {
                    writeln!(out, "  report {}: {}", report.id().raw(), report.title()).unwrap();
                }
            }
            "save" => out = self.save(words.get(1).ok_or("save needs a slot name")?)?,
            _ => return Err(format!("unknown command '{verb}'; type 'help'")),
        }
        Ok((out, false))
    }
}

pub fn run(registry: &Registry, args: &[String]) -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    run_with_io(registry, args, &mut input, &mut output)
}

fn run_with_io(
    registry: &Registry,
    args: &[String],
    input: &mut dyn BufRead,
    output: &mut dyn std::io::Write,
) -> Result<(), String> {
    let options = parse_options(args)?;
    if options.help {
        writeln!(output, "Text play: --seed N [--policy-seed N] [--script FILE] [--brief], or --load SLOT [--script FILE]\n{HELP}").map_err(|e| e.to_string())?;
        return Ok(());
    }
    let (mut session, label) = if let Some(slot) = options.load {
        (Session::load(registry, &slot)?, format!("Resumed '{slot}'"))
    } else {
        let world = options.seed.unwrap_or_else(fresh_seed);
        let seeds = EvaluationSeeds::new(world, options.policy_seed.unwrap_or(world));
        (
            Session::new(registry, seeds)?,
            "New authored city scenario".to_owned(),
        )
    };
    writeln!(
        output,
        "CRIMOCRACY TEXT PLAY | {label}\nWorld seed {} | policy seed {}\n{}",
        session.scenario.seeds.world,
        session.scenario.seeds.policy,
        session.brief()
    )
    .map_err(|e| e.to_string())?;
    if !options.brief {
        writeln!(output, "Type 'help' for commands.").map_err(|e| e.to_string())?;
    }
    if let Some(path) = options.script {
        let script = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read script {}: {e}", path.display()))?;
        for (index, line) in script.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            writeln!(output, "> {line}").map_err(|e| e.to_string())?;
            let (response, quit) = session
                .execute(line)
                .map_err(|e| format!("script line {}: {e}", index + 1))?;
            writeln!(output, "{response}").map_err(|e| e.to_string())?;
            if quit {
                break;
            }
        }
    } else {
        let mut line = String::new();
        loop {
            write!(output, "> ")
                .and_then(|_| output.flush())
                .map_err(|e| e.to_string())?;
            line.clear();
            if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                break;
            }
            match session.execute(&line) {
                Ok((response, quit)) => {
                    writeln!(output, "{response}").map_err(|e| e.to_string())?;
                    if quit {
                        break;
                    }
                }
                Err(error) => writeln!(output, "Rejected: {error}").map_err(|e| e.to_string())?,
            }
        }
    }
    output.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_play_script_runs_canonical_scout_and_reports() {
        let registry = crimocracy::build_registry();
        let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
        assert!(
            session
                .execute("opportunities")
                .unwrap()
                .0
                .contains("Burglary")
        );
        assert!(session.execute("scout 99999").is_err());
        let business = session.scenario.target.raw();
        assert!(
            session
                .execute(&format!("scout {business}"))
                .unwrap()
                .0
                .contains("authorized")
        );
        assert!(
            session
                .execute("advance 6h")
                .unwrap()
                .0
                .contains("Advanced")
        );
        assert!(!session.execute("reports").unwrap().0.is_empty());
    }

    #[test]
    fn text_play_ids_and_advancement_are_strict() {
        assert!(parse_number("0").is_err());
        assert_eq!(parse_number("decision-12").unwrap(), 12);
        assert_eq!(parse_advance("2h").unwrap(), 120);
        assert!(parse_advance("999d").is_err());
        assert!(save_path("../../escape").is_err());
        assert!(
            parse_options(&["--load".into(), "slot".into(), "--seed".into(), "1".into()]).is_err()
        );
    }

    #[test]
    fn text_play_script_reads_only_player_owned_reports() {
        let registry = crimocracy::build_registry();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let path = Path::new(SAVE_DIR).join("text-play-test-script.txt");
        std::fs::create_dir_all(SAVE_DIR).unwrap();
        std::fs::write(&path, "brief\nreports\npeople\nquit\n").unwrap();
        let result = run_with_io(
            &registry,
            &[
                "--seed".into(),
                "12".into(),
                "--script".into(),
                path.to_string_lossy().into_owned(),
            ],
            &mut input,
            &mut output,
        );
        let _ = std::fs::remove_file(&path);
        result.unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Pending decisions"));
        assert!(rendered.contains("Session ended"));
    }

    #[test]
    fn text_play_save_restore_continues_exact_simulation() {
        let registry = crimocracy::build_registry();
        let mut original = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
        original.execute("burgle 1 abort").unwrap();
        original.execute("advance 3h").unwrap();
        let slot = format!("text_play_roundtrip_{}", std::process::id());
        let path = save_path(&slot).unwrap();
        original.save(&slot).unwrap();
        original.save(&slot).unwrap(); // Updating an existing slot must work on Windows.
        let mut restored = Session::load(&registry, &slot).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(restored.scenario.state.now(), original.scenario.state.now());
        for _ in 0..45 {
            run_tick(&registry, &mut original.scenario.state).unwrap();
            run_tick(&registry, &mut restored.scenario.state).unwrap();
        }
        let before =
            bincode::serialize(&build_save(&registry, &original.scenario.state).unwrap()).unwrap();
        let after =
            bincode::serialize(&build_save(&registry, &restored.scenario.state).unwrap()).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn text_play_rejects_expiring_lead_before_authorization() {
        let registry = crimocracy::build_registry();
        let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
        let deadline = session
            .scenario
            .timeline
            .initial_opportunity_valid_until
            .as_minutes();
        while session.scenario.state.now().as_minutes() < deadline - 1 {
            run_tick(&registry, &mut session.scenario.state).unwrap();
        }
        let before =
            bincode::serialize(&build_save(&registry, &session.scenario.state).unwrap()).unwrap();
        assert!(
            session
                .execute("burgle 1 abort")
                .unwrap_err()
                .contains("expires before")
        );
        let after =
            bincode::serialize(&build_save(&registry, &session.scenario.state).unwrap()).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn text_play_reconnaissance_can_inform_a_later_burglary() {
        let registry = crimocracy::build_registry();
        let mut session = Session::new(&registry, EvaluationSeeds::new(12, 12)).unwrap();
        session
            .execute(&format!("scout {}", session.scenario.target.raw()))
            .unwrap();
        session.execute("advance 6h").unwrap();
        let confirmation = session.execute("burgle 1 abort").unwrap().0;
        assert!(confirmation.contains("information item(s)"));
        let operation = session
            .scenario
            .state
            .operations()
            .operations_for_organization(session.scenario.player)
            .find(|operation| operation.kind() == OperationKind::Burglary)
            .unwrap();
        assert!(
            operation.intelligence().len() > 1,
            "new scouting intelligence should contribute to the subsequent plan"
        );
    }
}
