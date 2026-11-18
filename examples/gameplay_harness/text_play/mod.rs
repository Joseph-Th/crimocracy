//! Line-oriented player client. The scripted evaluator is not involved in command selection.
//!
//! Campaigns can begin in a generated city or an authored opening. All subsequent
//! actions run through the same production validation, commit and minute-tick owners
//! as the evaluation harness. Views contain only the player's staff, books, reports,
//! and organization-held intelligence; scenario diagnostics never enter this client.

use crate::campaign::{
    EvaluationSeeds, FixtureVariation, ScenarioProfile, build_scenario, parse_seed_value,
};
use crimocracy::city_generation::{CityScale, generate_city};
use crimocracy::contacts::contact_system::{
    InstitutionalContactDraft, find_pending_disclosure_sources, validate_contact_disclosure,
    validate_establish_contact, validate_terminate_contact,
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
    validate_acquire_business,
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
    LaunderingDraft, insert_account, remaining_laundering_capacity, validate_launder_funds,
    validate_record_transaction,
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
use std::io::BufRead;
use std::path::{Path, PathBuf};

const SAVE_DIR: &str = "target/text-play";
const MAX_ADVANCE: u64 = DAY_MINUTES * 7;

const HELP: &str = "Commands:
  brief                  Leadership's current situation
  attention [exception|crisis on|off]  Inspect or configure optional auto-pauses
  people                 Your current organization members
  known-people           Your members, identified people, and approach-ready prospects
  known-targets          Identified organizations, operations, cases, and enterprises
  connections            Known institutional acquaintances with an existing staff relationship
  contacts               Your established institutional channels and their pending disclosures
  connect <handler-id> <known-person-id>  Establish a contact using a real relationship
  contact [contact-id] [ask|ask-all|end]  Ask for one or up to 64 disclosures, or terminate a channel
  counsel                Known arrests of current staff and their retained lawyers
  retain <arrest-id> <legal-contact-id> <fee-cents>
                         Retain legal counsel for a current member's known case
  end-counsel <representation-id> <concluded|replace|withdraw>
                         End representation through the legal owner
  policies               Organization standing policies
  policy <recruitment|legal> <approve|delegate|case|automatic>
                         Change organization-level governance
  districts              Public district names and IDs for delegated scopes
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
  businesses             Owned storefronts and known leads
  business <business-id> <suspend|resume>
                         Pause or restart a player-owned legitimate storefront
  business-cycles <business-id> [count]
                         Settled revenue and costs under your ownership
  buy <business-id>      Acquire a business at its standard price using clean funds
  launder <business-id> <cents>  Convert street cash via an owned front
  enterprises            Your rackets and their operating status
  racket-kinds           Available racket kinds and venue requirements
  start-racket <kind> <business-id|district-id> <cash-id> <settlement-id> [support-business-id...]
                         Establish a delegated racket backed by owned infrastructure
  racket <id> <suspend|resume|retire>  Change your racket's lifecycle
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
  burgle <lead-id> [entry-id] [coordinator-id] [abort|press]
                         Authorize a burglary with your chosen crew
  dismiss <lead-id>      Reject a lead through its canonical lifecycle
  decide <id> <continue|abort|approve|reject>
                         Resolve a pending decision
  sweep <business-id> <cents>  Withdraw earned storefront surplus to clean books
  advance <minutes|Nh|Nd>     Advance up to seven days, stopping for decisions, crew custody, and critical reports
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
        | "directory" | "market" | "businesses" | "enterprises" | "racket-kinds" | "finances"
        | "accounts" | "opportunities" | "operations" | "decisions" | "connections"
        | "contacts" | "counsel" | "policies" | "mandates" | "operation-kinds" | "districts"
        | "known-targets" | "debriefs" => 1..=1,
        "recruit" | "scout" => 2..=3,
        "start-racket" => 5..=usize::MAX,
        "burgle" => 2..=5,
        "racket" | "launder" | "sweep" | "decide" | "business" => 3..=3,
        "move" | "deposit" => 4..=4,
        "dispose" => 5..=5,
        "reports" | "treasury" => 1..=2,
        "intelligence" => 1..=4,
        "attention" => 1..=3,
        "ledger" => 1..=2,
        "business-cycles" | "racket-cycles" => 2..=3,
        "buy" | "open-account" | "discover" | "operation" | "report" | "dismiss" | "advance"
        | "save" | "transaction" | "cancel" | "debrief" | "intel" | "racket-outlook" => 2..=2,
        "policy" | "delegate" | "autonomy" | "supervise" => 3..=3,
        "mandate" => 3..=6,
        "plan" | "preview" => 3..=32,
        "connect" => 3..=3,
        "contact" => 1..=3,
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
            district: scenario.variation.neighborhood_name().to_owned(),
            district_variant: scenario.variation.label(),
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
                    .filter(|record| record.organization() == self.player)
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
                "no active organization mandate covers enterprise work at that location".to_owned()
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
            .filter(|enterprise| enterprise.status() != EnterpriseStatus::Retired)
            .count();
        let active_rackets = state
            .enterprises()
            .enterprises_for_organization(player)
            .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
            .count();
        let surplus: i128 = state
            .world()
            .businesses_owned_by_organization(player)
            .filter(|business| state.economy().get_business_economy(business.id()).is_some())
            .map(|business| {
                i128::from(crimocracy::economy::business_economy_system::resolve_available_business_profit(
                    state, player, business.id(),
                ).expect("owned operating books must have a valid capital basis").cents())
            })
            .sum();
        let distribution = if state.automatic_profit_sweeps() {
            "daily"
        } else {
            "manual"
        };
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
        let (payroll, covered) = crimocracy::world::payroll_execution::payroll_funding_snapshot(
            self.registry,
            state,
            player,
        )
        .expect("valid player payroll estimate");
        let payroll = payroll.cents();
        let covered = covered.cents();
        let latest = state
            .reports()
            .reports_for(player)
            .next_back()
            .map_or("No reports yet".to_owned(), |report| {
                format!("Report {}: {}", report.id().raw(), report.title())
            });
        format!(
            "{name} | {} | {} ({})\nPeople: {members} | Venues: {venues} | Rackets: {rackets} ({active_rackets} active, {} suspended)\nTreasury: street {street} cents | concealed {concealed} cents | accounted {clean} cents\nWithdrawable storefront surplus: {surplus} cents | distribution: {distribution}\nDaily payroll: {payroll} cents | covered from current treasury: {covered} cents (before future revenue)\nPending decisions: {pending} | Active operations: {active} | Open leads: {leads}\nLatest: {latest}\nUse 'reports', 'opportunities', 'people', or 'accounts' to investigate.",
            time_label(state.now().as_minutes()),
            self.district,
            self.district_variant,
            rackets - active_rackets
        )
    }
}

#[cfg(test)]
mod cancellation_tests;
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
mod reconnaissance;
#[cfg(test)]
mod reconnaissance_tests;
#[cfg(test)]
mod staff_matching_tests;
mod time_play;

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
