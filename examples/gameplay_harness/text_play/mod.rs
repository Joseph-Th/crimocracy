//! Line-oriented player client. The scripted evaluator is not involved in command selection.
//!
//! The initial campaign uses the existing canonical authored scenario. All subsequent
//! actions run through the same production validation, commit and minute-tick owners
//! as the evaluation harness. Views contain only the player's staff, books, reports,
//! and organization-held intelligence; scenario diagnostics never enter this client.

use crate::campaign::{EvaluationSeeds, ScenarioProfile, build_scenario, parse_seed_value};
use crimocracy::contacts::contact_system::{
    find_pending_disclosure_sources, validate_contact_disclosure,
};
use crimocracy::contacts::{ContactKind, ContactStatus};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{BusinessId, CharacterId, OrganizationId};
use crimocracy::core::persistence::{SaveEnvelope, build_save, restore_save};
use crimocracy::core::simulation::run_tick;
use crimocracy::core::state::AppState;
use crimocracy::core::time::{DAY_MINUTES, SimDuration};
use crimocracy::decisions::DecisionResponse;
use crimocracy::decisions::decision_system::validate_resolve_decision;
use crimocracy::delegation::delegation_system::{
    MandateRevisionDraft, validate_assign_mandate, validate_revise_mandate,
    validate_revoke_mandate, validate_set_policy,
};
use crimocracy::delegation::{BudgetAuthority, BudgetPeriod, MandateDraft};
use crimocracy::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crimocracy::economy::business_acquisition::{
    BusinessAcquisitionDraft, BusinessAcquisitionError, validate_acquire_business,
};
use crimocracy::economy::business_economy_system::{
    BusinessProfitSweepDraft, validate_sweep_business_profits,
};
use crimocracy::enterprises::enterprise_execution::{
    validate_establish_enterprise, validate_resume_enterprise, validate_retire_enterprise,
    validate_suspend_enterprise,
};
use crimocracy::enterprises::{
    ALL_ENTERPRISE_KINDS, EnterpriseDraft, EnterpriseKind, EnterpriseLocation,
};
use crimocracy::finance::finance_system::{
    LaunderingDraft, insert_account, remaining_laundering_capacity, validate_launder_funds,
    validate_record_transaction,
};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::{InformationSignal, KnowledgeHolder, LegalPersonStatusSignal};
use crimocracy::operations::operation_system::validate_authorize_operation;
use crimocracy::operations::property_disposition::{
    CashDispositionDraft, PropertyDispositionDraft, validate_deposit_operation_cash,
    validate_dispose_property,
};
use crimocracy::operations::{
    ALL_OPERATION_KINDS, OperationApproach, OperationContingency, OperationDraft, OperationKind,
    OperationObjective, OperationStatus, RoleKind,
};
use crimocracy::opportunities::opportunity_system::{
    validate_convert_opportunity, validate_discover_operation_opportunity,
    validate_dismiss_opportunity,
};
use crimocracy::opportunities::{OperationOpportunityDraft, OpportunityStatus};
use crimocracy::recruitment::recruitment_system::{
    find_recruitment_candidates, validate_recruitment_attempt,
};
use crimocracy::recruitment::{RecruitmentApproach, RecruitmentDraft};
use crimocracy::registry::Registry;
use crimocracy::world::{
    ALL_CAPABILITY_KINDS, ALL_POLICY_KINDS, ApprovalPolicy, CapabilityKind, LegalSupportPolicy,
    PolicyKind, PolicySetting,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};

const SAVE_DIR: &str = "target/text-play";
const MAX_ADVANCE: u64 = DAY_MINUTES * 7;

const HELP: &str = "Commands:
  brief                  Leadership's current situation
  people                 Your current organization members
  known-people           People identified by your intelligence or current membership
  policies               Organization standing policies
  policy <recruitment|legal> <approve|delegate|case|automatic>
                         Change organization-level governance
  districts              Public district names and IDs for delegated scopes
  mandates               Current manager scopes, standing orders, and budgets
  delegate <member-id> <scope>  Assign a manager with delegated authority
  mandate <id> <add|remove> <scope>  Adjust an active mandate's scope
  mandate <id> order <recruitment|legal> <approve|delegate|case|automatic|inherit>
  mandate <id> budget <daily|weekly> <account-id> <cents>
  mandate <id> budget clear
  mandate <id> revoke    Revoke authority when no active racket depends on it
  prospects              Contacts available for executive recruitment
  recruit <id> [offer|advancement|protection|appeal]
                         Approach a known prospect
  directory              Public city business names, kinds, and standard prices
  businesses             Owned storefronts and known leads
  buy <business-id>      Acquire a business at its standard price using clean funds
  launder <business-id> <cents>  Convert street cash via an owned front
  enterprises            Your rackets and their operating status
  racket-kinds           Available racket kinds and venue requirements
  start-racket <kind> <business-id> <cash-id> <settlement-id> [support-id]
                         Establish a racket with active delegated authority
  racket <id> <suspend|resume|retire>  Change your racket's lifecycle
  finances               Your actual organization accounts
  accounts               Inspect your organization account IDs and balances
  open-account <street|concealed|accounted|settlement>
                         Open an empty account in the organization's own books
  move <source-id> <dest-id> <cents>
                         Transfer between same-kind organization accounts
  intelligence           Information held by your organization
  operation-kinds        Authorized operation families and required crew roles
  plan <kind> <business-id|char-id> [approach=covert] [leader=char-id] [Role=char-id...]
                         Authorize an operation with optional explicit crew assignments
  opportunities          Known burglary leads and their deadlines
  discover <business-id> File a burglary lead from observed intelligence
  operations             Your authorized and completed operations
  operation <id>          Inspect your operation, crew, and observed outcome
  dispose <operation-id> <resale-business-id> <cash-id> <settlement-id>
                         Liquidate held property through an owned resale venue
  deposit <operation-id> <cash-id> <settlement-id>
                         Deposit held operation cash into the organization's books
  decisions              Pending decisions requiring leadership
  reports [count]        Most recent organization reports (default 8)
  report <id>            Read a complete organization report
  scout <business-id> [scout-id]  Assign a member to observe a known business
  burgle <lead-id> [entry-id] [coordinator-id] [abort|press]
                         Authorize a burglary with your chosen crew
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
    opening: Option<ScenarioProfile>,
    load: Option<String>,
    script: Option<PathBuf>,
    brief: bool,
    help: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        seed: None,
        policy_seed: None,
        opening: None,
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
                if options.seed.is_some() {
                    return Err("--seed may only appear once".to_owned());
                }
                let raw = value("--seed", &mut args)?;
                options.seed = Some(parse_seed_value("--seed", &raw).map_err(|e| e.to_string())?);
            }
            "--policy-seed" => {
                if options.policy_seed.is_some() {
                    return Err("--policy-seed may only appear once".to_owned());
                }
                let raw = value("--policy-seed", &mut args)?;
                options.policy_seed =
                    Some(parse_seed_value("--policy-seed", &raw).map_err(|e| e.to_string())?);
            }
            "--opening" => {
                if options.opening.is_some() {
                    return Err("--opening may only appear once".to_owned());
                }
                let raw = value("--opening", &mut args)?;
                options.opening = Some(parse_opening(&raw)?);
            }
            "--load" => {
                if options.load.is_some() {
                    return Err("--load may only appear once".to_owned());
                }
                options.load = Some(value("--load", &mut args)?);
            }
            "--script" => {
                if options.script.is_some() {
                    return Err("--script may only appear once".to_owned());
                }
                options.script = Some(PathBuf::from(value("--script", &mut args)?));
            }
            "--brief" => options.brief = true,
            other => return Err(format!("unknown text-play option '{other}'")),
        }
    }
    if options.load.is_some()
        && (options.seed.is_some() || options.policy_seed.is_some() || options.opening.is_some())
    {
        return Err("--load cannot be combined with seed or opening options".to_owned());
    }
    Ok(options)
}

fn parse_opening(raw: &str) -> Result<ScenarioProfile, String> {
    match raw {
        "night-trap" => Ok(ScenarioProfile::NightTrap),
        "late-patrol" => Ok(ScenarioProfile::LatePatrol),
        "fleeting-window" => Ok(ScenarioProfile::FleetingWindow),
        "veteran-crew" => Ok(ScenarioProfile::VeteranCrew),
        "thin-crew" => Ok(ScenarioProfile::ThinCrew),
        "green-scout" => Ok(ScenarioProfile::GreenScout),
        _ => Err("opening must be night-trap, late-patrol, fleeting-window, veteran-crew, thin-crew or green-scout".to_owned()),
    }
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

fn parse_number(raw: &str, kind: &str) -> Result<u32, String> {
    let digits = if let Some((prefix, digits)) = raw.rsplit_once('-') {
        if prefix != kind {
            return Err(format!("expected a {kind} ID, got '{raw}'"));
        }
        digits
    } else {
        raw
    };
    digits
        .parse::<u32>()
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

fn parse_enterprise_kind(raw: &str) -> Result<EnterpriseKind, String> {
    let normalized = raw
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    ALL_ENTERPRISE_KINDS
        .into_iter()
        .find(|kind| format!("{kind:?}").eq_ignore_ascii_case(&normalized))
        .ok_or_else(|| format!("unknown racket kind '{raw}'; use 'racket-kinds'"))
}

fn parse_account_kind(raw: &str) -> Result<AccountKind, String> {
    match raw {
        "street" => Ok(AccountKind::StreetCash),
        "concealed" => Ok(AccountKind::ConcealedCash),
        "accounted" => Ok(AccountKind::AccountedFunds),
        "settlement" => Ok(AccountKind::Settlement),
        _ => Err("open-account <street|concealed|accounted|settlement>".to_owned()),
    }
}

/// Scripted commands reject extraneous arguments instead of silently ignoring them.
fn validate_command_arity(command: &str, words: usize) -> Result<(), String> {
    let range = match command {
        "help" | "quit" | "exit" | "brief" | "people" | "known-people" | "prospects"
        | "directory" | "businesses" | "enterprises" | "racket-kinds" | "finances" | "accounts"
        | "intelligence" | "opportunities" | "operations" | "decisions" | "contact"
        | "policies" | "mandates" | "operation-kinds" | "districts" => 1..=1,
        "recruit" | "scout" => 2..=3,
        "start-racket" => 5..=6,
        "burgle" => 2..=5,
        "racket" | "launder" | "sweep" | "decide" => 3..=3,
        "move" | "deposit" => 4..=4,
        "dispose" => 5..=5,
        "reports" => 1..=2,
        "buy" | "open-account" | "discover" | "operation" | "report" | "dismiss" | "advance"
        | "wait" | "save" => 2..=2,
        "policy" | "delegate" => 3..=3,
        "mandate" => 3..=6,
        "plan" => 3..=14,
        _ => return Ok(()),
    };
    if range.contains(&words) {
        Ok(())
    } else {
        Err(format!(
            "invalid argument count for '{command}'; see 'help'"
        ))
    }
}

struct Session<'a> {
    registry: &'a Registry,
    state: AppState,
    player: OrganizationId,
    seeds: EvaluationSeeds,
    district: &'static str,
    district_variant: &'static str,
}

impl<'a> Session<'a> {
    fn new(registry: &'a Registry, seeds: EvaluationSeeds) -> Result<Self, String> {
        Self::new_with_opening(registry, seeds, ScenarioProfile::NightTrap)
    }

    fn new_with_opening(
        registry: &'a Registry,
        seeds: EvaluationSeeds,
        opening: ScenarioProfile,
    ) -> Result<Self, String> {
        let mut scenario = build_scenario(registry, seeds, opening).map_err(|e| e.to_string())?;
        for (target, source, summary, deadline) in [
            (
                scenario.target,
                scenario.opportunity_information,
                scenario.variation.opportunity_summary(),
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
        Ok(Self {
            registry,
            state: scenario.state,
            player: scenario.player,
            seeds,
            district: scenario.variation.neighborhood_name(),
            district_variant: scenario.variation.label(),
        })
    }

    fn load(registry: &'a Registry, slot: &str) -> Result<Self, String> {
        let path = save_path(slot)?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let file: CampaignFile =
            bincode::deserialize(&bytes).map_err(|e| format!("invalid campaign file: {e}"))?;
        let state = restore_save(registry, file.campaign).map_err(|e| e.to_string())?;
        let player = state
            .player_organization()
            .ok_or("saved campaign has no player organization")?;
        Ok(Self {
            registry,
            state,
            player,
            seeds: EvaluationSeeds::new(file.world_seed, file.policy_seed),
            district: "the saved campaign's district",
            district_variant: "saved",
        })
    }

    fn save(&self, slot: &str) -> Result<String, String> {
        let path = save_path(slot)?;
        let campaign = build_save(self.registry, &self.state).map_err(|e| e.to_string())?;
        let data = bincode::serialize(&CampaignFile {
            world_seed: self.seeds.world,
            policy_seed: self.seeds.policy,
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
        self.state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(self.player))
    }

    /// Select from current organizational members, not fixture-held characters whose
    /// membership may have changed. The fallback prefers publicly known staff skills,
    /// with stable character-id ties; the player can always name an explicit member.
    fn member_for_role(
        &self,
        requested: Option<&str>,
        skill: CapabilityKind,
        prefer_supervised: bool,
    ) -> Result<CharacterId, String> {
        let members: Vec<_> = self
            .state
            .world()
            .characters_in_organization(self.player)
            .collect();
        if let Some(raw) = requested {
            let id = parse_number(raw, "char")?;
            return members
                .iter()
                .find(|member| member.id().raw() == id)
                .map(|member| member.id())
                .ok_or_else(|| {
                    format!("character {id} is not a current member of your organization")
                });
        }
        let best = |require_supervision: bool| {
            members
                .iter()
                .filter(|person| !require_supervision || person.supervisor().is_some())
                .max_by_key(|person| {
                    (
                        person.capability(skill).map_or(0, |rating| rating.value()),
                        std::cmp::Reverse(person.id().raw()),
                    )
                })
                .map(|person| person.id())
        };
        best(prefer_supervised)
            .or_else(|| best(false))
            .ok_or_else(|| "your organization has no current members".to_owned())
    }

    fn executive_recruiter(&self) -> Result<CharacterId, String> {
        self.state
            .world()
            .characters_in_organization(self.player)
            .find(|person| person.supervisor().is_none())
            .map(|person| person.id())
            .ok_or_else(|| "your organization has no unsupervised leader".to_owned())
    }

    fn player_prospects(&self) -> Result<Vec<CharacterId>, String> {
        let leader = self.executive_recruiter()?;
        find_recruitment_candidates(self.registry, &self.state, self.player, leader)
            .map_err(|e| e.to_string())
    }

    /// Choose a current mandate with authority over this venue. The canonical
    /// enterprise establishment validator will check the entire authority state.
    fn enterprise_authority(&self, business: BusinessId) -> Result<MandateAuthority, String> {
        let district = self
            .state
            .world()
            .get_business(business)
            .ok_or("business not found")?
            .neighborhood();
        [
            ResponsibilityScope::Function(ResponsibilityFunction::Enterprise),
            ResponsibilityScope::Neighborhood(district),
            ResponsibilityScope::Business(business),
        ]
        .into_iter()
        .find_map(|scope| {
            self.state
                .delegation()
                .active_for_scope(scope)
                .find(|record| record.organization() == self.player)
                .map(|record| MandateAuthority {
                    mandate: record.id(),
                    manager: record.manager(),
                    scope,
                })
        })
        .ok_or_else(|| {
            "no active organization mandate covers enterprise work at that property".to_owned()
        })
    }

    fn known_business(&self, raw: u32) -> Option<BusinessId> {
        self.state
            .world()
            .public_business_directory()
            .find(|(id, _, _, _)| id.raw() == raw)
            .map(|(id, _, _, _)| id)
    }

    /// Read the organization's member roster and typed personnel information,
    /// never the world's hidden character index.
    fn known_character_ids(&self) -> BTreeSet<CharacterId> {
        let mut known: BTreeSet<_> = self
            .state
            .world()
            .characters_in_organization(self.player)
            .map(|member| member.id())
            .collect();
        for info in self.organization_information() {
            if let EntityRef::Character(id) = info.subject() {
                known.insert(id);
            }
            if let Some(InformationSignal::PersonnelPresence { characters }) = info.signal() {
                known.extend(characters.iter().copied());
            }
        }
        known
    }

    /// A guessed world character ID cannot open a legal or field operation.
    fn known_character(&self, raw: u32) -> Option<CharacterId> {
        self.known_character_ids()
            .into_iter()
            .find(|id| id.raw() == raw)
    }

    fn known_opportunities(&self) -> Vec<&crimocracy::opportunities::OpportunityRecord> {
        self.state
            .reports()
            .reports_for(self.player)
            .filter_map(|report| {
                self.state
                    .opportunities()
                    .opportunity_for_report(report.id())
            })
            .filter(|opportunity| opportunity.organization() == self.player)
            .map(|opportunity| (opportunity.id(), opportunity))
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    fn brief(&self) -> String {
        let state = &self.state;
        let player = self.player;
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
        let members = state.world().characters_in_organization(player).count();
        let venues = state
            .world()
            .businesses_owned_by_organization(player)
            .count();
        let rackets = state
            .enterprises()
            .enterprises_for_organization(player)
            .count();
        let mut street = 0_i128;
        let mut concealed = 0_i128;
        let mut clean = 0_i128;
        for account in state
            .finance()
            .accounts_for(FinancialOwner::Organization(player))
        {
            let amount = i128::from(account.balance().cents());
            match account.kind() {
                AccountKind::StreetCash => street += amount,
                AccountKind::ConcealedCash => concealed += amount,
                AccountKind::AccountedFunds => clean += amount,
                AccountKind::LegitimateOperating | AccountKind::Settlement => {}
            }
        }
        let latest = state
            .reports()
            .reports_for(player)
            .last()
            .map_or("No reports yet".to_owned(), |report| {
                format!("Report {}: {}", report.id().raw(), report.title())
            });
        format!(
            "{name} | {} | {} ({})\nPeople: {members} | Venues: {venues} | Rackets: {rackets}\nTreasury: street {street} cents | concealed {concealed} cents | accounted {clean} cents\nPending decisions: {pending} | Active operations: {active} | Open leads: {leads}\nLatest: {latest}\nUse 'reports', 'opportunities', 'people', or 'accounts' to investigate.",
            time_label(state.now().as_minutes()),
            self.district,
            self.district_variant
        )
    }
}

mod commands;
mod governance;
mod planning;

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
        writeln!(output, "Text play: --seed N [--policy-seed N] [--opening PROFILE] [--script FILE] [--brief], or --load SLOT [--script FILE]\nOpenings: night-trap, late-patrol, fleeting-window, veteran-crew, thin-crew, green-scout\n{HELP}").map_err(|e| e.to_string())?;
        return Ok(());
    }
    let (mut session, label) = if let Some(slot) = options.load {
        (Session::load(registry, &slot)?, format!("Resumed '{slot}'"))
    } else {
        let world = options.seed.unwrap_or_else(fresh_seed);
        let seeds = EvaluationSeeds::new(world, options.policy_seed.unwrap_or(world));
        let opening = options.opening.unwrap_or(ScenarioProfile::NightTrap);
        (
            if opening == ScenarioProfile::NightTrap {
                Session::new(registry, seeds)?
            } else {
                Session::new_with_opening(registry, seeds, opening)?
            },
            format!("New authored city opening: {}", opening.label()),
        )
    };
    writeln!(
        output,
        "CRIMOCRACY TEXT PLAY | {label}\nWorld seed {} | policy seed {}\n{}",
        session.seeds.world,
        session.seeds.policy,
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
mod governance_tests;
#[cfg(test)]
mod legal_planning_tests;
#[cfg(test)]
mod openings_tests;
#[cfg(test)]
mod tests;
