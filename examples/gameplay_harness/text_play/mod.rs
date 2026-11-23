//! Line-oriented player client over production operations and minute ticks.
//!
//! Campaigns begin in a generated city or a complete authored opening.
//! Views contain only the player's staff, books, reports, and held intelligence.

use crate::campaign::{EvaluationSeeds, FixtureVariation, ScenarioProfile, parse_seed_value};
use crate::scenario::build_scenario;
use crimocracy::city_generation::{CityScale, generate_city};
use crimocracy::contacts::contact_system::{
    InstitutionalContactDraft, find_pending_disclosure_sources, validate_contact_disclosure,
    validate_establish_contact, validate_reassign_contact_handler, validate_terminate_contact,
};
use crimocracy::contacts::{ContactKind, ContactStatus};
use crimocracy::core::entity::EntityRef;
use crimocracy::core::id::{
    ArrestId, BusinessId, CharacterId, EnterpriseId, InvestigationId, OperationId, OrganizationId,
};
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
    BusinessAcquisitionDraft, BusinessAcquisitionError, acquisition_offer,
    validate_acquire_business, validate_offer_business,
};
use crimocracy::economy::business_economy_system::{
    BusinessProfitSweepDraft, validate_sweep_business_profits,
};
use crimocracy::economy::business_economy_system::{
    validate_resume_business_economy, validate_suspend_business_economy,
};
use crimocracy::enterprises::enterprise_execution::{
    validate_establish_enterprise, validate_resume_enterprise, validate_retire_enterprise,
    validate_suspend_enterprise,
};
use crimocracy::enterprises::{
    ALL_ENTERPRISE_KINDS, EnterpriseDraft, EnterpriseKind, EnterpriseLocation, EnterpriseStatus,
};
use crimocracy::finance::finance_system::{
    LaunderingDraft, StreetCapitalizationDraft, insert_account, remaining_laundering_capacity,
    validate_capitalize_street_cash, validate_launder_funds, validate_record_transaction,
};
use crimocracy::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crimocracy::intelligence::InformationTransferDraft;
use crimocracy::intelligence::intelligence_system::{
    IntelligenceError, validate_information_transfer,
};
use crimocracy::intelligence::{InformationSignal, KnowledgeHolder, LegalPersonStatusSignal};
use crimocracy::legal::legal_representation_system::{
    validate_end_legal_representation, validate_retain_legal_representation,
};
use crimocracy::legal::{
    LegalRepresentationDraft, LegalRepresentationEndReason, LegalRepresentationOrigin,
};
use crimocracy::operations::operation_system::{
    OperationTransition, apply_transition, participant_available_for_plan, planning_information,
    validate_authorize_operation,
};
use crimocracy::operations::property_disposition::{
    CashDispositionDraft, PropertyDispositionDraft, validate_deposit_operation_cash,
    validate_dispose_property,
};
use crimocracy::operations::{
    ALL_OPERATION_KINDS, OperationApproach, OperationConstraint, OperationContingency,
    OperationDraft, OperationKind, OperationObjective, OperationStatus, RoleKind,
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
    ALL_CAPABILITY_KINDS, ALL_POLICY_KINDS, ApprovalPolicy, LegalSupportPolicy, PolicyKind,
    PolicySetting,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const SAVE_DIR: &str = "target/text-play";
const MAX_ADVANCE: u64 = DAY_MINUTES * 30;
static NEXT_SAVE_STAGING_ID: AtomicU64 = AtomicU64::new(0);

/// Keep the leadership estimate aligned with the finance owner's per-storefront
/// rounding. Charging overhead once against an aggregate overstates the payout.
fn estimate_owner_distribution(
    surpluses: impl IntoIterator<Item = Money>,
    basis_points: u16,
) -> (i128, i128) {
    surpluses.into_iter().fold((0, 0), |(gross, net), amount| {
        let fee = crimocracy::economy::business_economy_system::portfolio_overhead_charge(
            amount,
            basis_points,
        );
        (
            gross + i128::from(amount.cents()),
            net + i128::from(amount.cents()) - i128::from(fee.cents()),
        )
    })
}

const HELP: &str = "Commands:
  brief                  Leadership's current situation
  victory                View mayoral, retirement, and city-underboss ending requirements
  political <district-N>  Inspect local property, influence, specialists and electoral challenges
  campaign               Inspect own municipal campaign, spending and district influence
  election               Inspect the official last mayoral election and ward ballot counts
  nominate-mayor <char-N> File your leader as a mayoral candidate at least 30 days before election
  campaign-event <district-N> <char-N> <meetings|services|labor|press>
                         Fund a local civic initiative via a mandated manager and real infrastructure
  buy-influence <district-N> <accounted-cents>
                         Pay clean campaign funds for influence (5000 cents per point)
  hire-specialist <district-N> <char-N> <organizer|publicist|strategist>
                         Hire a paid campaign specialist under a delegated local political manager
  fire-specialist <district-N> <organizer|publicist|strategist>
                         Terminate a specialist's ongoing daily salary
  claim-victory <political|retirement|underboss>
                         Choose an eligible permanent campaign ending
  staff-racket <enterprise-id> <associate-count>
                         Hire or release rank-and-file racket workers within the local staffing limit
  attention [exception|crisis on|off]  Inspect or configure optional auto-pauses
  people                 Your current organization members
  population             Total persistent named city population and public population
  freelancers            Freelance criminals personally known to current crew members
  contract <char-N> <char-M> <days>
                         Pay a known freelancer for 1-14 days under crew member char-M
  civics [district-N]     Public civic professions, shopkeepers and municipal offices
  civic-person <char-N>   Public work history and individually held business titles
  known-people           Your members, identified people, and approach-ready prospects
  known-targets          Identified organizations, operations, cases, and enterprises
  connections            Known institutional acquaintances with an existing staff relationship
  contacts               Your established institutional channels and their pending disclosures
  connect <handler-id> <known-person-id>  Establish a contact using a real relationship
  contact <contact-id> [ask|ask-recent|ask-all|end]
                         Hear one update, newest unseen fact per subject, up to 64, or end a channel
  contact <contact-id> reassign <member-id>
                         Transfer an active channel to a qualified member; retain case/disclosure history
  counsel                Current custody, prosecution, representation and unpaid fines
  counsel all            All known staff arrest matters, including quiet history
  counsel arrest-N       Full details for a known staff arrest
  recovery               Suspended rackets and storefronts, with actionable recovery blockers
  retain <arrest-id> <legal-contact-id> <fee-cents>
                         Retain legal counsel for a current member's known case
  end-counsel <representation-id> <concluded|replace|withdraw>
                         End representation through the legal owner
  policies               Organization standing policies
  policy <recruitment|legal> <approve|delegate|case|automatic>
                         Change organization-level governance
  districts              Public district names and IDs for delegated scopes
  turf                   Your active rackets and negotiated district standstills
  contest <enterprise-N>  Challenge an identified rival racket using local associates; the loser closes for seven days
  preview-contest <enterprise-N>
                         Inspect your local fighting strength without revealing enemy staffing
  negotiate <district-N> <org-N> [tribute=cents]
                         Propose a 30-day ceasefire; stronger rivals may demand actual street-cash tribute
  mandates               Current manager scopes, standing orders, and budgets
  delegate <member-id> <scope>  Assign a manager with delegated authority
  autonomy <member-id> <tight|guided|delegated|broad>  Set freedom to act independently
  supervise <member-id> <supervisor-id>  Reassign a subordinate within your organization
  mandate <id> <add|remove> <scope>  Adjust an active mandate's scope
  mandate <id> order <recruitment|legal> <approve|delegate|case|automatic|inherit>
  mandate <id> budget <daily|weekly> <account-id> <cents>
  mandate <id> budget clear
  mandate <id> revoke    Revoke authority when no active racket depends on it
  prospects              Executive contacts and prospects known to personnel managers
  recruit <id> [offer|advancement|protection|appeal]
                         Approach a known prospect
  directory              Public business names, kinds, districts, and standard prices
  market                 Purchasable businesses with advertised infrastructure and prices
  business-outlook <id>  Public competitive earnings and reference-offer estimate
  businesses             Owned storefronts and known leads
  business <business-id> <suspend|resume>
                         Pause or restart a player-owned legitimate storefront
  business-cycles <business-id> [count]
                         Settled revenue and costs under your ownership
  buy <business-id>      Acquire a listed business at its standard price using clean funds
  offer <business-id> <cents>  Make a binding offer for an organization-owned business
  launder <business-id> <cents|max>  Convert an amount or remaining affordable front capacity
  launder-all <street-reserve-cents>
                         One-time conversion through owned fronts, preserving a street-cash reserve
  enterprises            Your rackets and their operating status
  racket-kinds           Available racket kinds and venue requirements
  start-racket <kind> <business-id|district-id> <cash-id> <settlement-id> [support-business-id...] [manager=char-N]
                         Establish a delegated racket backed by owned infrastructure
  preview-racket <same arguments as start-racket>
                         Validate an investment and estimate earnings without establishing it
  racket <id> <suspend|resume|retire>  Change your racket's lifecycle
  resume-rackets <id> [id...]  Resume selected suspended rackets in one validated order
  cover-rackets <char-N> <id> [id...]  Reassign and resume selected suspended rackets atomically
  reassign-racket <id> <member-id>  Transfer management without replacing the racket
  racket-cycles <enterprise-id> [count]
                         Historical racket earnings and enforcement attention
  racket-outlook <enterprise-id>  Conditional earnings and shared working-cash estimate
  finances               Your actual organization accounts
  accounts               Inspect your organization account IDs and balances
  treasury [manual|daily]  Inspect or set daily earned-profit distribution before payroll
  ledger [count]          Recent transactions touching your books (default 10)
  transaction <id>        Inspect owned-account postings without rival books
  open-account <street|concealed|accounted|settlement>
                         Open an empty account in the organization's own books
  move <source-id> <dest-id> <cents>
                         Transfer between same-kind organization accounts
  fund-street <accounted-id> <street-id> <cents>
                         Withdraw clean funds into street operating cash (recorded, not laundering)
  case-brief             Recently disclosed case activity, with dates and provenance IDs
  intelligence [count] [page] [target-id]
                         Browse held observations, newest observed first (default 20 per page)
  intel <information-id> Inspect one held observation, typed finding and provenance
  debriefs               Your members and counts of unreported personal observations
  debrief <member-id>    Transfer one new member observation to organization knowledge
  operation-kinds        Authorized operation families and required crew roles
  plan <kind> <target-id> [approach=...] [leader=char-id] [Role=char-id...] [police=press|ask|abort]
                         [delay=Nh] [deadline=Nh] [need=InformationTopic]
                         Authorize with optional crew, contingency, schedule and knowledge gates.
                         Surveillance/infiltration also target
                         known org-, char-, district-, operation-, investigation-, enterprise-IDs
  preview <kind> <target-id> [same options as plan]
                         Validate and show crew and usable planning intelligence without authorizing
  opportunities          Known burglary leads and their deadlines
  discover <business-id> File a burglary lead from observed intelligence
  operations             Your authorized and completed operations
  operation <id>          Inspect your operation, crew, and observed outcome
  cancel <operation-id>  Withdraw an authorized or active operation; release crew bookings
  dispose <operation-id> <resale-business-id> <cash-id> <settlement-id>
                         Liquidate held property through an owned resale venue
  deposit <operation-id> <cash-id> <settlement-id>
                         Deposit held operation cash into the organization's books
  decisions              Pending decisions requiring leadership
  reports [count]        Most recent organization reports (default 8)
  report <id>            Read a complete organization report
  scout <business-id> [scout-id]  Assign a member to observe a known business
  burgle <lead-id> [entry-id] [coordinator-id] [abort|ask|press]
                         Authorize a burglary (default: abort before entry)
                         Police modes: abort before entry, ask leadership, or press on
  dismiss <lead-id>      Reject a lead through its canonical lifecycle
  decide <id> <continue|abort|approve|reject>
                         Resolve a pending decision
  sweep <business-id> <cents>  Withdraw earned storefront surplus to clean books
  advance <minutes|Nh|Nd> [uninterrupted]
                         Advance up to 30 days; uninterrupted skips optional pauses, never decisions or terminal outcomes, and summarizes deferred alerts
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
    generated: bool,
    city_size: Option<CityScale>,
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
        generated: false,
        city_size: None,
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
            "--generate" => {
                if options.generated {
                    return Err("--generate may only appear once".to_owned());
                }
                options.generated = true;
            }
            "--city-size" => {
                if options.city_size.is_some() {
                    return Err("--city-size may only appear once".to_owned());
                }
                let raw = value("--city-size", &mut args)?;
                options.city_size = Some(match raw.as_str() {
                    "small" => CityScale::Small,
                    "medium" => CityScale::Medium,
                    "large" => CityScale::Large,
                    _ => return Err("--city-size must be small, medium or large".to_owned()),
                });
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
        && (options.seed.is_some()
            || options.policy_seed.is_some()
            || options.opening.is_some()
            || options.generated
            || options.city_size.is_some())
    {
        return Err(
            "--load cannot be combined with generation, seed or opening options".to_owned(),
        );
    }
    if options.city_size.is_some() && !options.generated {
        return Err("--city-size requires --generate".to_owned());
    }
    if options.generated && (options.opening.is_some() || options.policy_seed.is_some()) {
        return Err(
            "--generate cannot be combined with an authored opening or policy seed".to_owned(),
        );
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

/// Publish only a complete save. A unique, exclusively created staging file prevents
/// overlapping sessions (and debris from an interrupted session) from replacing each
/// other's bytes before the final rename. The existing destination is never removed as
/// part of error handling, so failure to publish retains the last good campaign.
fn publish_save(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut staging = None;
    for _ in 0..16 {
        let sequence = NEXT_SAVE_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("sav.{}.{}.tmp", std::process::id(), sequence));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => {
                staging = Some((temporary, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot stage save: {error}")),
        }
    }
    let (temporary, mut file) = staging.ok_or("cannot allocate a distinct temporary save file")?;
    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    let published = written.and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = published {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot publish save: {error}"));
    }
    Ok(())
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
        .ok_or_else(|| "advance requires 1 to 43200 minutes (e.g. 45m, 6h, 30d)".to_owned())
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
        "help" | "quit" | "exit" | "brief" | "victory" | "campaign" | "election" | "people"
        | "population" | "freelancers" | "known-people" | "prospects" | "directory" | "market"
        | "businesses" | "enterprises" | "racket-kinds" | "finances" | "accounts"
        | "opportunities" | "operations" | "decisions" | "connections" | "contacts"
        | "policies" | "mandates" | "operation-kinds" | "districts" | "turf" | "known-targets"
        | "debriefs" | "case-brief" => 1..=1,
        "counsel" => 1..=2,
        "recovery" => 1..=1,
        "political" | "civic-person" => 2..=2,
        "civics" => 1..=2,
        "advance" => 2..=3,
        "launder-all" => 2..=2,
        "resume-rackets" => 2..=32,
        "cover-rackets" => 3..=32,
        "recruit" | "scout" => 2..=3,
        "start-racket" | "preview-racket" => 5..=usize::MAX,
        "burgle" => 2..=5,
        "racket" | "reassign-racket" | "staff-racket" | "launder" | "sweep" | "decide"
        | "business" => 3..=3,
        "move" | "deposit" | "fund-street" | "hire-specialist" | "campaign-event" | "contract" => {
            4..=4
        }
        "dispose" => 5..=5,
        "reports" | "treasury" => 1..=2,
        "intelligence" => 1..=4,
        "attention" => 1..=3,
        "ledger" => 1..=2,
        "business-cycles" | "racket-cycles" => 2..=3,
        "buy" | "open-account" | "discover" | "operation" | "report" | "dismiss"
        | "claim-victory" | "save" | "transaction" | "cancel" | "debrief" | "intel"
        | "racket-outlook" | "business-outlook" | "contest" | "preview-contest"
        | "nominate-mayor" => 2..=2,
        "policy" | "delegate" | "autonomy" | "supervise" | "offer" | "buy-influence"
        | "fire-specialist" => 3..=3,
        "negotiate" => 3..=4,
        "mandate" => 3..=6,
        "plan" | "preview" => 3..=32,
        "connect" => 3..=3,
        "contact" => 2..=4,
        "retain" => 4..=4,
        "end-counsel" => 3..=3,
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
    district: String,
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
        let state = build_scenario(registry, seeds, opening).map_err(|e| e.to_string())?;
        let player = state
            .player_organization()
            .ok_or("authored opening has no player organization")?;
        let variation = FixtureVariation::from_seed(seeds.world);
        Ok(Self {
            registry,
            state,
            player,
            seeds,
            district: variation.neighborhood_name().to_owned(),
            district_variant: variation.label(),
        })
    }

    fn new_generated(
        registry: &'a Registry,
        seeds: EvaluationSeeds,
        size: CityScale,
    ) -> Result<Self, String> {
        let city = generate_city(registry, size.config(seeds.world))
            .map_err(|error| format!("city generation failed: {error}"))?;
        let district = city
            .state
            .world()
            .get_neighborhood(city.home)
            .ok_or("generated home district missing")?
            .name()
            .to_owned();
        Ok(Self {
            registry,
            state: city.state,
            player: city.player,
            seeds,
            district,
            district_variant: "PROCEDURAL",
        })
    }

    fn load(registry: &'a Registry, slot: &str) -> Result<Self, String> {
        let path = save_path(slot)?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let file: CampaignFile =
            bincode::deserialize(&bytes).map_err(|e| format!("invalid campaign file: {e}"))?;
        // Derive presentation labels from saved metadata without recreating
        // the authored scenario or touching its persisted world state.
        let state = restore_save(registry, file.campaign).map_err(|e| e.to_string())?;
        let generated = state.world().neighborhoods().count() > 2;
        let variation = FixtureVariation::from_seed(file.world_seed);
        let district = if generated {
            state
                .world()
                .neighborhoods()
                .next()
                .ok_or("generated city has no districts")?
                .name()
                .to_owned()
        } else {
            variation.neighborhood_name().to_owned()
        };
        let player = state
            .player_organization()
            .ok_or("saved campaign has no player organization")?;
        Ok(Self {
            registry,
            state,
            player,
            seeds: EvaluationSeeds::new(file.world_seed, file.policy_seed),
            district,
            district_variant: if generated {
                "PROCEDURAL"
            } else {
                variation.label()
            },
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
        publish_save(&path, &data)?;
        Ok(format!("Campaign saved to {}", path.display()))
    }

    fn organization_information(
        &self,
    ) -> impl Iterator<Item = &crimocracy::intelligence::InformationRecord> {
        self.state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Organization(self.player))
    }

    /// Resolve a player-specified member; the canonical planner validates availability.
    fn requested_member(&self, raw: &str) -> Result<CharacterId, String> {
        let id = parse_number(raw, "char")?;
        self.state
            .world()
            .characters_in_organization(self.player)
            .find(|member| member.id().raw() == id)
            .map(|member| member.id())
            .ok_or_else(|| format!("character {id} is not a current member of your organization"))
    }

    /// Share candidate-specific routes between discovery and execution. When several
    /// available executives know a candidate, prefer the lowest character ID.
    fn player_prospects(&self) -> Result<BTreeMap<CharacterId, CharacterId>, String> {
        let mut routes = BTreeMap::new();
        for leader in self.state.world().characters_in_organization(self.player) {
            if leader.supervisor().is_some()
                || self
                    .state
                    .legal()
                    .active_arrest_for_character(leader.id())
                    .is_some()
            {
                continue;
            }
            for candidate in
                find_recruitment_candidates(self.registry, &self.state, self.player, leader.id())
                    .map_err(|e| e.to_string())?
            {
                routes
                    .entry(candidate)
                    .and_modify(|current: &mut CharacterId| *current = (*current).min(leader.id()))
                    .or_insert(leader.id());
            }
        }
        Ok(routes)
    }

    fn manager_prospects(&self) -> Result<Vec<(CharacterId, Vec<CharacterId>)>, String> {
        self.state
            .delegation()
            .active_for_scope(ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            ))
            .filter(|mandate| mandate.organization() == self.player)
            .filter(|mandate| {
                self.state
                    .legal()
                    .active_arrest_for_character(mandate.manager())
                    .is_none()
            })
            .map(|mandate| {
                let manager = mandate.manager();
                let candidates =
                    find_recruitment_candidates(self.registry, &self.state, self.player, manager)
                        .map_err(|error| error.to_string())?;
                Ok((manager, candidates))
            })
            .collect()
    }

    /// Choose a current mandate with authority over this location. The canonical
    /// enterprise establishment validator will check the entire authority state.
    fn enterprise_authority(
        &self,
        location: EnterpriseLocation,
        supporting_businesses: &BTreeSet<BusinessId>,
        requested_manager: Option<CharacterId>,
    ) -> Result<MandateAuthority, String> {
        let (district, business) = match location {
            EnterpriseLocation::Business(business) => (
                self.state
                    .world()
                    .get_business(business)
                    .ok_or("business not found")?
                    .neighborhood(),
                Some(business),
            ),
            EnterpriseLocation::Neighborhood(district) => (
                self.state
                    .world()
                    .get_neighborhood(district)
                    .ok_or("district not found")?
                    .id(),
                None,
            ),
        };
        // Prefer a specific venue or geographic assignment over a general remit,
        // but never select a scope that cannot cover the entire supporting network.
        let mut scopes = Vec::new();
        if let Some(business) = business
            && supporting_businesses
                .iter()
                .all(|support| *support == business)
        {
            scopes.push(ResponsibilityScope::Business(business));
        }
        if supporting_businesses.iter().all(|support| {
            self.state
                .world()
                .get_business(*support)
                .is_some_and(|record| record.neighborhood() == district)
        }) {
            scopes.push(ResponsibilityScope::Neighborhood(district));
        }
        scopes.push(ResponsibilityScope::Function(
            ResponsibilityFunction::Enterprise,
        ));
        scopes
            .into_iter()
            .find_map(|scope| {
                self.state
                    .delegation()
                    .active_for_scope(scope)
                    .filter(|record| {
                        record.organization() == self.player
                            && requested_manager.is_none_or(|manager| record.manager() == manager)
                    })
                    .map(|record| MandateAuthority {
                        mandate: record.id(),
                        manager: record.manager(),
                        scope,
                    })
                    .find(|authority| {
                        crimocracy::delegation::delegation_system::resolve_mandate_authority(
                            &self.state,
                            *authority,
                        )
                        .is_ok()
                    })
            })
            .ok_or_else(|| {
                if requested_manager.is_some() {
                    "selected manager has no active mandate covering the host and entire supporting network; assign enterprise scope for cross-district support".to_owned()
                } else {
                    "no active organization mandate covers the host and entire supporting network; cross-district support requires enterprise scope".to_owned()
                }
            })
    }

    fn known_business(&self, raw: u32) -> Option<BusinessId> {
        self.state
            .world()
            .public_business_directory()
            .find(|(id, _, _, _)| id.raw() == raw)
            .map(|(id, _, _, _)| id)
    }

    /// Public business/district names are available independently of hidden
    /// ownership and police intelligence.
    fn enterprise_location_label(&self, location: EnterpriseLocation) -> String {
        match location {
            EnterpriseLocation::Business(id) => self.state.world().get_business(id).map_or_else(
                || format!("business-{}", id.raw()),
                |business| format!("business-{} {}", id.raw(), business.name()),
            ),
            EnterpriseLocation::Neighborhood(id) => {
                self.state.world().get_neighborhood(id).map_or_else(
                    || format!("district-{}", id.raw()),
                    |district| format!("district-{} {}", id.raw(), district.name()),
                )
            }
        }
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
        known.extend(
            self.state
                .contacts()
                .contacts_for_sponsor(self.player)
                .map(|contact| contact.contact()),
        );
        // The recruitment owner has already confirmed a real, player-visible
        // relationship to every approach-ready prospect. Those individuals
        // must also be eligible for observation before the boss recruits them.
        if let Ok(prospects) = self.player_prospects() {
            known.extend(prospects.into_keys());
        }
        if let Ok(managers) = self.manager_prospects() {
            for (_, prospects) in managers {
                known.extend(prospects);
            }
        }
        for info in self.organization_information() {
            for entity in [Some(info.subject()), info.source_entity()]
                .into_iter()
                .flatten()
            {
                if let EntityRef::Character(id) = entity {
                    known.insert(id);
                }
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

    /// Organizational affiliation is not general player knowledge. Only our
    /// own identity, established contacts, or held information naming an
    /// institution make it a selectable reconnaissance target.
    fn known_organization_ids(&self) -> BTreeSet<OrganizationId> {
        let mut known = BTreeSet::from([self.player]);
        known.extend(
            self.state
                .contacts()
                .contacts_for_sponsor(self.player)
                .map(|contact| contact.institution()),
        );
        for info in self.organization_information() {
            for entity in [Some(info.subject()), info.source_entity()]
                .into_iter()
                .flatten()
            {
                if let EntityRef::Organization(id) = entity {
                    known.insert(id);
                }
            }
        }
        known
    }

    fn known_operation_ids(&self) -> BTreeSet<OperationId> {
        let mut known = self
            .state
            .operations()
            .operations_for_organization(self.player)
            .map(|operation| operation.id())
            .collect::<BTreeSet<_>>();
        known.extend(self.organization_information().filter_map(|info| {
            if let EntityRef::Operation(id) = info.subject() {
                Some(id)
            } else {
                None
            }
        }));
        known
    }

    fn known_enterprise_ids(&self) -> BTreeSet<EnterpriseId> {
        let mut known = self
            .state
            .enterprises()
            .enterprises_for_organization(self.player)
            .map(|enterprise| enterprise.id())
            .collect::<BTreeSet<_>>();
        known.extend(self.organization_information().filter_map(|info| {
            if let EntityRef::Enterprise(id) = info.subject() {
                Some(id)
            } else {
                None
            }
        }));
        known
    }

    fn known_investigation_ids(&self) -> BTreeSet<InvestigationId> {
        let mut known = BTreeSet::new();
        for info in self.organization_information() {
            if let EntityRef::Investigation(id) = info.subject() {
                known.insert(id);
            }
            if let Some(InformationSignal::LegalPersonStatus(
                LegalPersonStatusSignal::CaseWitness { investigation },
            )) = info.signal()
            {
                known.insert(*investigation);
            }
        }
        known
    }

    fn known_opportunities(&self) -> Vec<&crimocracy::opportunities::OpportunityRecord> {
        self.state
            .opportunities()
            .opportunities_for_organization(self.player)
            .collect()
    }

    fn brief(&self) -> String {
        if let Some(event) = self.state.campaign_victory()
            && let crimocracy::history::HistoryEventKind::CampaignVictory { path, .. } =
                event.kind()
        {
            return format!(
                "Campaign over: victory ({}). {}. Review the final reports.",
                path.label(),
                event.summary()
            );
        }
        if self.state.player_organization_defeated() {
            return "Campaign over: your organization dissolved. Review the final reports."
                .to_owned();
        }
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
        let public_prominence =
            crimocracy::world::organization_pressure::public_prominence_tier(state, player);
        let suspended_venues = state
            .world()
            .businesses_owned_by_organization(player)
            .filter(|business| {
                state
                    .economy()
                    .get_business_economy(business.id())
                    .is_some_and(|economy| {
                        economy.status() == crimocracy::economy::BusinessOperatingStatus::Suspended
                    })
            })
            .count();
        let rackets = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|enterprise| enterprise.status() != EnterpriseStatus::Retired)
            .count();
        let active_rackets = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
            .count();
        let overhead_basis_points =
            crimocracy::economy::business_economy_system::portfolio_overhead_basis_points(
                state, player,
            );
        let (surplus, estimated_owner_surplus) = estimate_owner_distribution(
            state
                .world()
                .businesses_owned_by_organization(player)
                .filter(|business| {
                    state
                        .economy()
                        .get_business_economy(business.id())
                        .is_some()
                })
                .map(|business| {
                    crimocracy::economy::business_economy_system::resolve_available_business_profit(
                        state,
                        player,
                        business.id(),
                    )
                    .expect("owned operating books must have a valid capital basis")
                }),
            overhead_basis_points,
        );
        let distribution = if state.automatic_profit_sweeps() {
            "daily"
        } else {
            "manual"
        };
        let mut street = 0_i128;
        let mut concealed = 0_i128;
        let mut clean = 0_i128;
        let mut street_spendable = 0_i128;
        let mut concealed_spendable = 0_i128;
        let mut clean_spendable = 0_i128;
        let mut operating_spendable = 0_i128;
        let mut deficits = 0_i128;
        for account in state
            .finance()
            .accounts_for(FinancialOwner::Organization(player))
        {
            let amount = i128::from(account.balance().cents());
            let spendable = i128::from(account.available_payment_balance().cents());
            match account.kind() {
                AccountKind::StreetCash => {
                    street += amount;
                    street_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::ConcealedCash => {
                    concealed += amount;
                    concealed_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::AccountedFunds => {
                    clean += amount;
                    clean_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::LegitimateOperating => {
                    operating_spendable += spendable;
                    deficits += (-amount).max(0);
                }
                AccountKind::Settlement => {}
            }
        }
        let (payroll, covered) = crimocracy::world::payroll_execution::payroll_funding_snapshot(
            self.registry,
            state,
            player,
        )
        .expect("valid player payroll estimate");
        let payroll = payroll.cents();
        let covered = covered.cents();
        // Snapshot only: every organization-owned liquid account can pay wages, but suspended
        // rackets, future shop earnings, counsel fees and fines make forecasts uncertain.
        let wage_liquidity =
            street_spendable + concealed_spendable + clean_spendable + operating_spendable;
        let wage_runway = if payroll > 0 {
            format!("{} full day(s)", wage_liquidity / i128::from(payroll))
        } else {
            "no current wage obligation".to_owned()
        };
        // Actual owner-settled cycles, not a forecast or an extrapolation of held cash.
        // The truncated opening window uses its real elapsed duration so a new campaign
        // doesn't pretend to have already earned 30 days of income.
        let window_minutes = state.now().as_minutes().min(30 * DAY_MINUTES);
        let window_start = crimocracy::core::time::SimTime::from_minutes(
            state.now().as_minutes() - window_minutes,
        );
        let days = window_minutes.div_ceil(DAY_MINUTES).max(1);
        let business_net = crimocracy::economy::business_reporting::resolve_organization_business_financial_summary(
            state, player, window_start, state.now(),
        ).expect("valid player business financial history").totals.net_cash.cents();
        let racket_net = crimocracy::enterprises::enterprise_reporting::resolve_organization_enterprise_financial_summary(
            state, player, window_start, state.now(),
        ).expect("valid player enterprise financial history").totals.net_cash.cents();
        let avg_daily_net = (i128::from(business_net) + i128::from(racket_net)) / i128::from(days);
        let payroll_gap = avg_daily_net - i128::from(payroll);
        let treasury_guidance = if !state.automatic_profit_sweeps()
            && surplus > 0
            && covered < payroll
        {
            "\nEarned storefront cash cannot pay wages until withdrawn. Use 'treasury daily' to distribute before payroll, or 'sweep' to withdraw from one storefront."
        } else {
            ""
        };
        let detained_members = state
            .world()
            .characters_in_organization(player)
            .filter(|member| {
                state
                    .legal()
                    .active_arrest_for_character(member.id())
                    .is_some()
            })
            .count();
        let active_associates: u64 = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|racket| racket.status() == EnterpriseStatus::Active)
            .map(|racket| u64::from(racket.associates()))
            .sum();
        let inactive_associates: u64 = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|racket| racket.status() == EnterpriseStatus::Suspended)
            .map(|racket| u64::from(racket.associates()))
            .sum();
        let incremental_labor_wages: u64 = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|racket| racket.status() == EnterpriseStatus::Active)
            .map(|racket| {
                u64::from(racket.associates() - racket.kind().baseline_associates())
                    * crimocracy::enterprises::enterprise_execution::ASSOCIATE_ADDITIONAL_DAILY_WAGE
                        .cents() as u64
            })
            .sum();
        let assigned_captains = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|racket| racket.status() == EnterpriseStatus::Active)
            .map(|racket| racket.manager())
            .collect::<BTreeSet<_>>();
        let command = state
            .world()
            .characters_in_organization(player)
            .filter_map(|person| {
                crimocracy::enterprises::enterprise_execution::manager_workload(state, person.id())
            })
            .filter(|load| load.required > 0)
            .collect::<Vec<_>>();
        let command_required: u64 = command.iter().map(|load| u64::from(load.required)).sum();
        let command_available: u64 = command.iter().map(|load| u64::from(load.available)).sum();
        let overstretched_leaders = command
            .iter()
            .filter(|load| load.required > load.available)
            .count();
        let active_captains = assigned_captains.len();
        let command_lines: u64 = command
            .iter()
            .map(|load| u64::from(load.command_lines))
            .sum();
        let latest = state
            .reports()
            .reports_for(player)
            .next_back()
            .map_or("No reports yet".to_owned(), |report| {
                format!("Report {}: {}", report.id().raw(), report.title())
            });
        format!(
            "{name} | {} | {} ({})\nPeople: {members} named | Associates: {active_associates} active, {inactive_associates} suspended | Venues: {venues} ({suspended_venues} suspended) | Rackets: {rackets} ({active_rackets} active, {} suspended)\nPublic prominence: tier {public_prominence}/6 | a wider commercial and political reach draws rival resistance and greater scrutiny of active rackets\nTreasury: street {street} cents | concealed {concealed} cents | accounted {clean} cents\nSpendable now: street {street_spendable} cents | concealed {concealed_spendable} cents | accounted {clean_spendable} cents | operating {operating_spendable} cents | separate account deficits {deficits} cents\nWithdrawable storefront surplus: {surplus} cents gross | owner net estimate {estimated_owner_surplus} cents after {overhead_basis_points} bp portfolio administration | distribution: {distribution}\nAssigned racket captains: {active_captains} | all-chain command duties: {command_required}/{command_available} available | direct reporting lines: {command_lines} | overloaded leaders: {overstretched_leaders} (see 'people' for individual workloads)\nNamed-member daily payroll: {payroll} cents | covered from current treasury: {covered} cents (before future revenue)
Gross payroll runway: {wage_runway} at current headcount and accessible funds, ignoring new income and other spending{treasury_guidance}
Additional associate wages: {incremental_labor_wages} cents/day through racket operating expenses (baseline workers included in fixed racket costs)\nRealized operating net before owner-distribution overhead (last {days} day(s)): legitimate {business_net} cents | rackets {racket_net} cents | averaged {avg_daily_net} cents/day | vs current payroll {payroll_gap:+} cents/day (historical income against today's wages; excludes laundering and asset trades)\nDetained staff: {detained_members} | suspended rackets: {} | suspended storefronts: {suspended_venues} | use 'counsel' and 'recovery' for actionable details\nPending decisions: {pending} | Active operations: {active} | Open leads: {leads}\nLatest: {latest}\nUse 'reports', 'opportunities', 'people', or 'accounts' to investigate.",
            time_label(state.now().as_minutes()),
            self.district,
            self.district_variant,
            rackets - active_rackets,
            rackets - active_rackets,
        )
    }
}

mod business_play;
#[cfg(test)]
mod cancellation_tests;
mod civic_play;
#[cfg(test)]
mod civic_play_tests;
mod commands;
mod contact_play;
mod debrief_play;
#[cfg(test)]
mod debrief_tests;
#[cfg(test)]
mod generation_tests;
mod governance;
#[cfg(test)]
mod legal_disclosure_tests;
mod legal_play;
mod planning;
#[cfg(test)]
mod planning_options_tests;
mod political_play;
#[cfg(test)]
mod political_play_tests;
mod racket_play;
mod reconnaissance;
#[cfg(test)]
mod reconnaissance_tests;
#[cfg(test)]
mod staff_matching_tests;
mod time_play;
mod treasury_play;

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
        writeln!(output, "Text play: --generate [--city-size small|medium|large] [--seed N] [--script FILE] [--brief]\nOr: --seed N [--policy-seed N] [--opening PROFILE] [--script FILE] [--brief]\nOr: --load SLOT [--script FILE]\nAuthored openings: night-trap, late-patrol, fleeting-window, veteran-crew, thin-crew, green-scout\n{HELP}").map_err(|e| e.to_string())?;
        return Ok(());
    }
    let (mut session, label) = if let Some(slot) = options.load {
        (Session::load(registry, &slot)?, format!("Resumed '{slot}'"))
    } else {
        let world = options.seed.unwrap_or_else(fresh_seed);
        let seeds = EvaluationSeeds::new(world, options.policy_seed.unwrap_or(world));
        if options.generated {
            let size = options.city_size.unwrap_or(CityScale::Medium);
            (
                Session::new_generated(registry, seeds, size)?,
                format!("New procedurally generated {} city", size.label()),
            )
        } else {
            let opening = options.opening.unwrap_or(ScenarioProfile::NightTrap);
            (
                if opening == ScenarioProfile::NightTrap {
                    Session::new(registry, seeds)?
                } else {
                    Session::new_with_opening(registry, seeds, opening)?
                },
                format!("New authored city opening: {}", opening.label()),
            )
        }
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
mod contact_legal_tests;
mod economy_history;
#[cfg(test)]
mod economy_play_tests;
mod finance_play;
#[cfg(test)]
mod finance_play_tests;
#[cfg(test)]
mod governance_tests;
#[cfg(test)]
mod legal_planning_tests;
#[cfg(test)]
mod openings_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod treasury_capitalization_tests;
