//! Player command dispatch through canonical production owners.

use super::planning::PoliceMode;
use super::*;
use crimocracy::world::{AutonomyLevel, CharacterRecord};

/// The recruitment owner has established an existing personal acquaintance. Show
/// its vocational profile, not private motives, numeric relationship scores or
/// anticipated acceptance odds.
fn write_prospect_skills(out: &mut String, person: &CharacterRecord) {
    let skills = ALL_CAPABILITY_KINDS
        .into_iter()
        .filter_map(|kind| person.capability(kind).map(|_| format!("{kind:?}")))
        .collect::<Vec<_>>();
    if !skills.is_empty() {
        writeln!(out, "  Familiar skills: {}", skills.join(", ")).unwrap();
    }
}

/// Terminal campaigns remain inspectable and saveable, but never actionable.
/// Admit only verbs whose every supported argument form is read-only. In particular,
/// `contact <id>` defaults to `ask`, which records a new disclosure.
pub(super) fn terminal_command_is_read_only(verb: &str) -> bool {
    matches!(
        verb,
        "help"
            | "quit"
            | "exit"
            | "brief"
            | "victory"
            | "save"
            | "people"
            | "known-people"
            | "known-targets"
            | "connections"
            | "contacts"
            | "counsel"
            | "policies"
            | "districts"
            | "turf"
            | "mandates"
            | "prospects"
            | "directory"
            | "market"
            | "businesses"
            | "business-outlook"
            | "business-cycles"
            | "enterprises"
            | "racket-kinds"
            | "racket-outlook"
            | "finances"
            | "accounts"
            | "ledger"
            | "transaction"
            | "case-brief"
            | "intelligence"
            | "intel"
            | "debriefs"
            | "operation-kinds"
            | "opportunities"
            | "operations"
            | "operation"
            | "decisions"
            | "reports"
            | "report"
            | "racket-cycles"
            | "recovery"
    )
}

impl Session<'_> {
    pub(super) fn execute(&mut self, command: &str) -> Result<(String, bool), String> {
        let mut words: Vec<_> = command.split_whitespace().collect();
        let Some(raw_verb) = words.first().copied() else {
            return Ok((String::new(), false));
        };
        let normalized = raw_verb.to_ascii_lowercase();
        let verb = normalized.as_str();
        words[0] = verb;
        validate_command_arity(verb, words.len())?;
        let terminal_reason = if self.state.player_organization_defeated() {
            Some("Campaign over: your organization dissolved")
        } else if self.state.campaign_victory().is_some() {
            Some("Campaign over: victory has already been claimed")
        } else {
            None
        };
        if let Some(reason) = terminal_reason
            && !terminal_command_is_read_only(verb)
        {
            return Err(reason.to_owned());
        }
        let state = &self.state;
        let mut out = String::new();
        match verb {
            "help" => out.push_str(HELP),
            "quit" | "exit" => {
                return Ok((
                    "Session ended. Save explicitly to continue later.".to_owned(),
                    true,
                ));
            }
            "brief" => out = self.brief(),
            "victory" => {
                let standing =
                    crimocracy::core::campaign_victory::progress(self.registry, &self.state)
                        .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Optional endings (claim-victory <political|retirement|underboss>):"
                )
                .unwrap();
                writeln!(out, "  Political: {}/{} contacts held for 30 days; owned businesses in {} districts, {} productive mature districts ({} required of {} city districts)",
                    standing.established_political_contacts, standing.active_political_contacts,
                    standing.owned_operating_business_districts, standing.established_business_districts,
                    standing.required_political_districts, standing.total_districts).unwrap();
                writeln!(out, "    Named political staff: {} / {} required ({} headquarters and one independent local delegate per required district); staffed mature districts: {} / {}",
                    standing.key_personnel, standing.required_political_personnel,
                    crimocracy::core::campaign_victory::POLITICAL_HEADQUARTERS_PERSONNEL,
                    standing.staffed_political_districts, standing.required_political_districts).unwrap();
                out.push_str("    Each district needs two distinct kinds of mature profitable businesses and its own active, undetained delegate with both a neighborhood mandate and Political function scope, at Delegated or Broad autonomy. One manager cannot count for two districts; the leader cannot serve as a district delegate.\n");
                writeln!(out, "    Mature legitimate income: {} cents/day realized over 30 days; daily payroll: {} cents; covered: {}",
                    standing.legitimate_daily_net.cents(), standing.daily_payroll.cents(),
                    if standing.legitimate_self_sufficient { "yes" } else { "no" }).unwrap();
                let local_floor = i128::from(self.registry.upkeep().per_member_daily().cents())
                    * i128::from(crimocracy::core::campaign_victory::VICTORY_INCOME_WINDOW_DAYS);
                let districts = crimocracy::core::campaign_victory::political_district_progress(
                    self.registry,
                    &self.state,
                )
                .map_err(|error| error.to_string())?;
                out.push_str("    Owned political district readiness (own books only):\n");
                if districts.is_empty() {
                    out.push_str("      No operating or suspended owned storefronts.\n");
                }
                for district in districts {
                    let name = state
                        .world()
                        .get_neighborhood(district.district)
                        .map_or("unknown", |record| record.name());
                    let reason = if district.qualifies {
                        "locally qualified"
                    } else if district.active_owned == 0 {
                        "no active storefronts; check 'recovery'"
                    } else if district.mature_active == 0 {
                        "needs an active storefront continuously owned for 90 days"
                    } else if district.established_active == 0 {
                        "needs a mature storefront with 20 profitable cycles in 30 days and a recent settlement"
                    } else if district.established_kinds
                        < crimocracy::core::campaign_victory::POLITICAL_BUSINESS_KINDS_PER_DISTRICT
                    {
                        "needs two different established business kinds"
                    } else if !district.economically_qualified {
                        "mature storefront income below one member's 30-day wages"
                    } else if district.steward.is_none() {
                        "needs an independent eligible political district delegate"
                    } else {
                        "not locally qualified"
                    };
                    let steward = district
                        .steward
                        .map_or("none".to_owned(), |id| format!("char-{}", id.raw()));
                    writeln!(
                        out,
                        "      district-{} {name}: {reason} | {} active, {} suspended, {} mature, {} settlement-qualified, {} kinds | {} available delegates, assigned {steward} | mature 30d net {} / {} cents required",
                        district.district.raw(),
                        district.active_owned,
                        district.suspended_owned,
                        district.mature_active,
                        district.established_active,
                        district.established_kinds,
                        district.available_stewards,
                        district.mature_window_net.cents(),
                        local_floor
                    )
                    .unwrap();
                }
                writeln!(out, "  Retirement: {} cents net accounted funds; {} cents required ({} dollars for the leader plus {} days' severance per subordinate)",
                    standing.accounted_net.cents(), standing.retirement_total.cents(),
                    crimocracy::core::campaign_victory::RETIREMENT_NEST_EGG.cents() / 100,
                    crimocracy::core::campaign_victory::RETIREMENT_SEVERANCE_DAYS).unwrap();
                writeln!(out, "  Underboss: active rackets in {} districts, {} productive mature districts ({} required); local majority and superior strength relative to rivals required",
                    standing.own_active_racket_districts, standing.established_racket_districts,
                    standing.required_underboss_districts).unwrap();
                writeln!(out, "    Named command staff: {} / {} required; independently captained mature districts: {} / {} required ({} distinct proven active-racket managers per district, not shared across districts)",
                    standing.key_personnel, standing.required_underboss_personnel,
                    standing.staffed_underboss_districts, standing.required_underboss_districts,
                    crimocracy::core::campaign_victory::UNDERBOSS_CAPTAINS_PER_DISTRICT).unwrap();
                writeln!(out, "    Proven rank-and-file associates: {} across mature rackets; economically uncontested districts with at least {} proven associates and more active associates than rivals combined: {} / {}",
                    standing.mature_associates,
                    crimocracy::core::campaign_victory::UNDERBOSS_ASSOCIATES_PER_DISTRICT,
                    standing.workforce_ready_districts, standing.required_underboss_districts).unwrap();
                writeln!(out, "    Mature racket income: {} cents/day realized over 30 days; daily payroll: {} cents; covered: {}",
                    standing.racket_daily_net.cents(), standing.daily_payroll.cents(),
                    if standing.racket_self_sufficient { "yes" } else { "no" }).unwrap();
                out.push_str("    A captain must direct 20 profitable settlements in 30 days; associate strength must have operated for 20 profitable cycles. Named personnel lead rank-and-file workers, whose compensation is paid through racket operating costs.\n");
                writeln!(out, "  Productive mature districts require 90 days of ownership/operation, 20 profitable settlements in the last 30 days, a settlement within 7 days, and 30-day net income sufficient for one crew member's daily wage ({} cents/day).",
                    self.registry.upkeep().per_member_daily().cents()).unwrap();
                out.push_str(
                    "Reaching a threshold does not end sandbox play until you claim the victory.",
                );
            }
            "claim-victory" => {
                use crimocracy::core::campaign_victory::{VictoryPath, validate_claim_victory};
                let path = match words[1] {
                    "political" => VictoryPath::PoliticalPower,
                    "retirement" => VictoryPath::FundedRetirement,
                    "underboss" => VictoryPath::CityUnderboss,
                    _ => return Err("claim-victory <political|retirement|underboss>".to_owned()),
                };
                let result = validate_claim_victory(self.registry, &self.state, path)
                    .map_err(|error| error.to_string())?
                    .commit(self.registry, &mut self.state)
                    .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Campaign over: victory ({}). History {}, report {}.",
                    result.path.label(),
                    result.history,
                    result.report
                )
                .unwrap();
                if let Some(payment) = result.settlement {
                    writeln!(
                        out,
                        "Final retirement payout recorded in transaction {payment}."
                    )
                    .unwrap();
                }
            }
            "attention" => {
                use crimocracy::core::attention::AttentionClass;
                match words.as_slice() {
                    [_] => {}
                    [_, class, enabled] => {
                        let class = match *class {
                            "exception" => AttentionClass::Exception,
                            "crisis" => AttentionClass::Crisis,
                            _ => return Err("attention [exception|crisis on|off]".to_owned()),
                        };
                        let enabled = match *enabled {
                            "on" => true,
                            "off" => false,
                            _ => return Err("attention [exception|crisis on|off]".to_owned()),
                        };
                        self.state
                            .set_auto_pause(class, enabled)
                            .map_err(|error| error.to_string())?;
                    }
                    _ => return Err("attention [exception|crisis on|off]".to_owned()),
                }
                for class in [AttentionClass::Exception, AttentionClass::Crisis] {
                    writeln!(
                        out,
                        "{class:?} auto-pause: {}",
                        if self.state.attention_settings().is_auto_pause_enabled(class) {
                            "on"
                        } else {
                            "off"
                        }
                    )
                    .unwrap();
                }
                out.push_str("Pending decisions, own-member custody changes, and completed or aborted operations always pause.");
            }
            "treasury" | "accounts" | "open-account" | "move" | "fund-street" | "finances" => {
                out = self.execute_treasury_commands(&words)?;
            }
            "debriefs" | "debrief" => out = self.execute_debrief(&words)?,
            "ledger" | "transaction" => out = self.execute_finance_history(&words)?,
            "business-cycles" | "racket-cycles" => {
                out = self.execute_economy_history(&words)?;
            }
            "connections" | "contacts" | "connect" | "contact" => {
                out = self.execute_contacts(&words)?;
            }
            "counsel" | "retain" | "end-counsel" => {
                out = self.execute_legal(&words)?;
            }
            "recovery" => out = self.recovery(),
            "policies" | "policy" | "mandates" | "delegate" | "mandate" | "autonomy"
            | "supervise" => {
                out = self.execute_governance(&words)?;
            }
            "districts" => {
                for district in state.world().neighborhoods() {
                    writeln!(
                        out,
                        "district-{} | {}",
                        district.id().raw(),
                        district.name()
                    )
                    .unwrap();
                }
            }
            "turf" => {
                use crimocracy::world::territory_influence::{
                    active_standstill, resolve_neighborhood_influence,
                };
                for district in state.world().neighborhoods() {
                    let summary = resolve_neighborhood_influence(state, district.id())
                        .map_err(|error| error.to_string())?;
                    let own = summary
                        .standings
                        .iter()
                        .find(|entry| entry.organization == self.player)
                        .map_or(0, |entry| entry.active_enterprises);
                    let associates = crimocracy::world::territory_conflict::active_district_force(
                        state,
                        self.player,
                        district.id(),
                    );
                    writeln!(out, "district-{} {} | own active rackets: {} | active associates: {} | rival influence requires reconnaissance",
                        district.id().raw(), district.name(), own, associates).unwrap();
                    if let Some(standstill) = active_standstill(state, self.player, district.id()) {
                        let rival = state
                            .world()
                            .get_organization(standstill.counterparty)
                            .expect("agreed counterpart exists");
                        writeln!(out, "  Standstill with {} (org-{}) until {} | no new rackets or offensive business operations",
                            rival.name(), rival.id().raw(), time_label(standstill.expires_at.as_minutes())).unwrap();
                    }
                    for rival in self.known_enterprise_ids().into_iter().filter_map(|id| {
                        let record = state.enterprises().get_enterprise(id)?;
                        if record.organization() == self.player {
                            return None;
                        }
                        let location = match record.location() {
                            EnterpriseLocation::Neighborhood(ward) => Some(ward),
                            EnterpriseLocation::Business(venue) => state
                                .world()
                                .get_business(venue)
                                .map(|business| business.neighborhood()),
                        };
                        (location == Some(district.id())).then_some((record.organization(), id))
                    }) {
                        if let Some(organization) = state.world().get_organization(rival.0) {
                            writeln!(out, "  Identified rival: {} (org-{}) at enterprise-{} | current status and staffing unconfirmed",
                                organization.name(), organization.id().raw(), rival.1.raw()).unwrap();
                        }
                    }
                }
            }
            "negotiate" => {
                use crimocracy::world::territory_influence::validate_negotiate_standstill_with_tribute;
                let district_number = parse_number(words[1], "district")?;
                let district = state
                    .world()
                    .neighborhoods()
                    .find(|item| item.id().raw() == district_number)
                    .ok_or("unknown district")?
                    .id();
                let rival_number = parse_number(words[2], "org")?;
                let rival = self
                    .known_organization_ids()
                    .into_iter()
                    .find(|id| id.raw() == rival_number && *id != self.player)
                    .ok_or("rival organization is not identified; use reconnaissance")?;
                if !self.known_enterprise_ids().into_iter().any(|id| {
                    state
                        .enterprises()
                        .get_enterprise(id)
                        .is_some_and(|record| {
                            let racket_district = match record.location() {
                                EnterpriseLocation::Neighborhood(id) => Some(id),
                                EnterpriseLocation::Business(id) => state
                                    .world()
                                    .get_business(id)
                                    .map(|business| business.neighborhood()),
                            };
                            record.organization() == rival && racket_district == Some(district)
                        })
                }) {
                    return Err(
                        "no identified rival racket in this district; surveil its operations first"
                            .to_owned(),
                    );
                }
                let tribute_cents = if let Some(raw) = words.get(3) {
                    raw.strip_prefix("tribute=")
                        .ok_or("optional fourth argument must be tribute=<positive cents>")?
                        .parse::<i64>()
                        .ok()
                        .filter(|amount| *amount > 0)
                        .ok_or("tribute must be a positive whole number of cents")?
                } else {
                    0
                };
                let history = validate_negotiate_standstill_with_tribute(
                    state,
                    self.player,
                    rival,
                    district,
                    crimocracy::finance::Money::from_cents(tribute_cents),
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(out, "Agreed a 30-day district standstill with org-{rival_number}. {}Recorded as {history}. Existing rackets continue; both gangs refrain from new rackets and offensive business operations until expiry.",
                    if tribute_cents > 0 { format!("Transferred {tribute_cents} cents street-cash tribute. ") } else { String::new() }).unwrap();
            }
            "contest" | "preview-contest" => {
                use crimocracy::world::territory_conflict::{
                    active_district_force, district_of_racket, validate_territory_clash,
                };
                let number = parse_number(words[1], "enterprise")?;
                let target = self
                    .known_enterprise_ids()
                    .into_iter()
                    .find(|id| id.raw() == number)
                    .ok_or("rival racket not identified; surveil the district first")?;
                let racket = state
                    .enterprises()
                    .get_enterprise(target)
                    .ok_or("racket no longer exists")?;
                if racket.organization() == self.player {
                    return Err(
                        "contest needs an identified rival racket, not one of your own".to_owned(),
                    );
                }
                let district = district_of_racket(state, racket.location())
                    .ok_or("racket location no longer exists")?;
                let own_force = active_district_force(state, self.player, district);
                let prepared = validate_territory_clash(state, self.player, target)
                    .map_err(|error| error.to_string())?;
                if verb == "preview-contest" {
                    writeln!(out, "Challenge enterprise-{number} in district-{} | active associates: {own_force}. Defenders have a positional advantage; their current force is unverified. Defeat suspends your leading racket for seven days. No action committed.", district.raw()).unwrap();
                } else {
                    let result = prepared
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                    writeln!(out, "District-{} confrontation: {}. Racket {} suspended for seven days; next challenge allowed in seven days. History: {}. Review 'reports'.",
                        district.raw(),
                        if result.attacker_won { "your gang displaced a rival operation" } else { "your assault was repelled" },
                        result.suspended_racket, result.history).unwrap();
                }
            }
            "operation-kinds" | "known-targets" | "plan" | "preview" => {
                out = self.execute_planning(&words)?;
            }
            "people" => {
                for person in state.world().characters_in_organization(self.player) {
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
            "known-people" => {
                for id in self.known_character_ids() {
                    let person = state
                        .world()
                        .get_character(id)
                        .ok_or("known person disappeared")?;
                    let relationship = if person.organization() == Some(self.player) {
                        "organization member"
                    } else {
                        "identified outside person"
                    };
                    writeln!(
                        out,
                        "char-{} {} | {}",
                        id.raw(),
                        person.name(),
                        relationship
                    )
                    .unwrap();
                    for information in self
                        .organization_information()
                        .filter(|info| info.subject() == EntityRef::Character(id))
                    {
                        if let Some(InformationSignal::LegalPersonStatus(status)) =
                            information.signal()
                        {
                            let description = match status {
                                LegalPersonStatusSignal::CaseWitness { .. } => {
                                    "reported case witness; current legal status is not confirmed"
                                        .to_owned()
                                }
                                LegalPersonStatusSignal::Detained { arrest }
                                    if person.organization() == Some(self.player) =>
                                {
                                    let ending = state
                                        .legal()
                                        .get_arrest(*arrest)
                                        .and_then(|episode| episode.released_at())
                                        .map(|at| {
                                            format!("released {}", time_label(at.as_minutes()))
                                        })
                                        .unwrap_or_else(|| "custody ongoing".to_owned());
                                    format!(
                                        "reported detention (arrest-{}; {ending})",
                                        arrest.raw()
                                    )
                                }
                                LegalPersonStatusSignal::Detained { .. } => {
                                    "reported detention; current legal status is not confirmed"
                                        .to_owned()
                                }
                            };
                            writeln!(
                                out,
                                "  Learned {description} via information {} (observed {})",
                                information.id().raw(),
                                time_label(information.observed_at().as_minutes())
                            )
                            .unwrap();
                        }
                    }
                }
            }
            "prospects" => {
                if words.len() != 1 {
                    return Err("prospects takes no arguments".to_owned());
                }
                let prospects = self.player_prospects()?;
                for (id, recruiter) in prospects {
                    let person = state
                        .world()
                        .get_character(id)
                        .ok_or("prospect disappeared")?;
                    let leader = state
                        .world()
                        .get_character(recruiter)
                        .ok_or("executive disappeared")?;
                    writeln!(
                        out,
                        "{} {} | established connection to leadership via {} (char-{})",
                        id.raw(),
                        person.name(),
                        leader.name(),
                        recruiter.raw()
                    )
                    .unwrap();
                    write_prospect_skills(&mut out, person);
                }
                let mut approval_refusals = None;
                for (manager, candidates) in self.manager_prospects()? {
                    let member = state
                        .world()
                        .get_character(manager)
                        .ok_or("manager disappeared")?;
                    let status = match member.autonomy() {
                        AutonomyLevel::Delegated | AutonomyLevel::Broad => {
                            "may act at the daily recruitment pass, subject to policy"
                        }
                        AutonomyLevel::Guided | AutonomyLevel::Tight => {
                            "requires delegated or broad autonomy for independent recruitment"
                        }
                    };
                    let policy =
                        crimocracy::delegation::delegation_system::resolve_policy_for_manager(
                            state,
                            manager,
                            crimocracy::world::PolicyKind::IndependentRecruitment,
                        )
                        .map_err(|error| error.to_string())?;
                    for candidate in candidates {
                        let status = if policy.independent_recruitment_approval()
                            == crimocracy::world::ApprovalPolicy::RequireApproval
                            && approval_refusals
                                .get_or_insert_with(|| {
                                    crimocracy::recruitment::recruitment_system::recent_recruitment_approval_refusals(
                                        self.registry, state,
                                    )
                                })
                                .contains(&(self.player, candidate))
                        {
                            "leadership declined the approach; manager will reconsider after recruitment cooldown"
                        } else {
                            status
                        };
                        let person = state
                            .world()
                            .get_character(candidate)
                            .ok_or("prospect disappeared")?;
                        writeln!(
                            out,
                            "{} {} | known to manager {} (char-{}); {status}",
                            candidate.raw(),
                            person.name(),
                            member.name(),
                            manager.raw()
                        )
                        .unwrap();
                        write_prospect_skills(&mut out, person);
                    }
                }
                if out.is_empty() {
                    out.push_str("No currently approachable contacts.");
                }
            }
            "recruit" => {
                if !(2..=3).contains(&words.len()) {
                    return Err(
                        "recruit <character-id> [offer|advancement|protection|appeal]".to_owned(),
                    );
                }
                let raw = parse_number(words[1], "char")?;
                let (candidate, recruiter) = self
                    .player_prospects()?
                    .into_iter()
                    .find(|(id, _)| id.raw() == raw)
                    .ok_or("candidate is not a currently approachable contact")?;
                let approach = match words.get(2).copied().unwrap_or("appeal") {
                    "offer" => RecruitmentApproach::FinancialOpportunity,
                    "advancement" => RecruitmentApproach::Advancement,
                    "protection" => RecruitmentApproach::Protection,
                    "appeal" => RecruitmentApproach::PersonalAppeal,
                    _ => {
                        return Err(
                            "recruit <character-id> [offer|advancement|protection|appeal]"
                                .to_owned(),
                        );
                    }
                };
                let attempt = validate_recruitment_attempt(
                    self.registry,
                    &self.state,
                    RecruitmentDraft {
                        target_organization: self.player,
                        recruiter,
                        candidate,
                        approach,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                let record = self
                    .state
                    .recruitment()
                    .get_attempt(attempt)
                    .ok_or("recruitment attempt missing")?;
                writeln!(
                    out,
                    "Recruitment attempt {}: {:?}.",
                    attempt.raw(),
                    record.outcome()
                )
                .unwrap();
            }
            "directory" | "buy" | "offer" | "market" | "business" | "launder" | "launder-all"
            | "businesses" | "sweep" | "business-outlook" => {
                out = self.execute_business_commands(&words)?;
            }
            "enterprises" | "racket-outlook" | "racket-kinds" | "start-racket"
            | "preview-racket" | "racket" | "resume-rackets" | "cover-rackets"
            | "reassign-racket" | "staff-racket" => {
                out = self.execute_racket_commands(&words)?;
            }
            "case-brief" => {
                let mut records = self
                    .organization_information()
                    .filter(|info| {
                        matches!(
                            info.signal(),
                            Some(crimocracy::intelligence::InformationSignal::CaseActivity(_))
                        )
                    })
                    .collect::<Vec<_>>();
                records.sort_unstable_by_key(|info| {
                    std::cmp::Reverse((info.observed_at(), info.id()))
                });
                writeln!(
                    out,
                    "Learned case activity (historical observations, not a current case census):"
                )
                .unwrap();
                if records.is_empty() {
                    writeln!(
                        out,
                        "  No disclosed case activity. Ask police contacts or gather intelligence."
                    )
                    .unwrap();
                }
                for info in records.iter().take(20) {
                    writeln!(
                        out,
                        "  intel {} | {} | {:?} | {}",
                        info.id().raw(),
                        time_label(info.observed_at().as_minutes()),
                        info.reliability(),
                        info.summary()
                    )
                    .unwrap();
                }
                if records.len() > 20 {
                    writeln!(
                        out,
                        "  {} older observations omitted; use intelligence for history.",
                        records.len() - 20
                    )
                    .unwrap();
                }
            }
            "intelligence" => {
                let count = words.get(1).map_or(Ok(20_usize), |value| {
                    value
                        .parse::<usize>()
                        .map_err(|_| "intelligence needs a positive count")
                })?;
                if !(1..=100).contains(&count) {
                    return Err("intelligence count must be 1 to 100".to_owned());
                }
                let page = words.get(2).map_or(Ok(1_usize), |value| {
                    value
                        .parse::<usize>()
                        .map_err(|_| "intelligence needs a positive page number")
                })?;
                let offset = page
                    .checked_sub(1)
                    .and_then(|n| n.checked_mul(count))
                    .ok_or("invalid intelligence page")?;
                let target = words
                    .get(3)
                    .map(|raw| self.reconnaissance_target(raw).map(|(entity, _, _)| entity))
                    .transpose()?;
                let mut records = self
                    .organization_information()
                    .filter(|info| target.is_none_or(|entity| info.subject() == entity))
                    .collect::<Vec<_>>();
                records.sort_unstable_by_key(|info| {
                    std::cmp::Reverse((info.observed_at(), info.id()))
                });
                writeln!(out,"Page {page} | showing {} of {} organization-held observations, newest observed first.", count.min(records.len().saturating_sub(offset)), records.len()).unwrap();
                for info in records.into_iter().skip(offset).take(count) {
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
            "intel" => {
                let raw = parse_number(words[1], "information")?;
                let fact = self
                    .organization_information()
                    .find(|fact| fact.id().raw() == raw)
                    .ok_or("information is not held by your organization")?;
                writeln!(
                    out,
                    "information-{} | {:?} | {:?} on {:?}",
                    fact.id().raw(),
                    fact.source_kind(),
                    fact.topic(),
                    fact.subject(),
                )
                .unwrap();
                writeln!(
                    out,
                    "Observed {} | recorded {} | {:?}/{:?}",
                    time_label(fact.observed_at().as_minutes()),
                    time_label(fact.recorded_at().as_minutes()),
                    fact.reliability(),
                    fact.specificity(),
                )
                .unwrap();
                writeln!(out, "Source: {:?}", fact.source_entity()).unwrap();
                if let Some(signal) = fact.signal() {
                    writeln!(out, "Typed finding: {signal:?}").unwrap();
                }
                if !fact.derived_from().is_empty() {
                    writeln!(
                        out,
                        "Source information: {:?}",
                        fact.derived_from()
                            .iter()
                            .map(|id| format!("information-{}", id.raw()))
                            .collect::<Vec<_>>()
                    )
                    .unwrap();
                }
                writeln!(out, "{}", fact.summary()).unwrap();
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
            "discover" => {
                if words.len() != 2 {
                    return Err("discover <business-id>".to_owned());
                }
                let raw = parse_number(words[1], "business")?;
                let target = self
                    .known_business(raw)
                    .ok_or("business is not in the public directory")?;
                let information = crimocracy::opportunities::opportunity_system::latest_usable_opportunity_information(
                    self.registry,
                    state,
                    self.player,
                    OperationKind::Burglary,
                    EntityRef::Business(target),
                )
                .ok_or("no qualifying organization-held information about this business; scout first")?;
                let summary = format!(
                    "{}: {}",
                    state
                        .world()
                        .get_business(target)
                        .ok_or("business disappeared")?
                        .name(),
                    information.summary()
                );
                let source = information.id();
                let opportunity = validate_discover_operation_opportunity(
                    self.registry,
                    &self.state,
                    OperationOpportunityDraft {
                        organization: self.player,
                        operation_kind: OperationKind::Burglary,
                        targets: BTreeSet::from([EntityRef::Business(target)]),
                        source_information: BTreeSet::from([source]),
                        summary,
                        valid_until: None,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Burglary lead {} recorded from information {}.",
                    opportunity.raw(),
                    source.raw()
                )
                .unwrap();
            }
            "operations" => {
                for operation in state.operations().operations_for_organization(self.player) {
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
            "operation" => {
                if words.len() != 2 {
                    return Err("operation <id>".to_owned());
                }
                let raw = parse_number(words[1], "operation")?;
                let op = state
                    .operations()
                    .operations_for_organization(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("no operation with that ID for your organization")?;
                writeln!(
                    out,
                    "{} | {:?} | {:?} | {}",
                    op.title(),
                    op.kind(),
                    op.status(),
                    time_label(op.scheduled_for().as_minutes())
                )
                .unwrap();
                writeln!(
                    out,
                    "Objective: {:?} | approach: {:?}",
                    op.objective(),
                    op.approach()
                )
                .unwrap();
                for (role, person) in op.roles() {
                    let name = state
                        .world()
                        .get_character(*person)
                        .map_or("unknown member", |person| person.name());
                    writeln!(out, "  {role:?}: {name} (char-{})", person.raw()).unwrap();
                }
                writeln!(
                    out,
                    "Planning intelligence: {} record(s); constraints: {:?}; contingencies: {:?}",
                    op.intelligence().len(),
                    op.constraints(),
                    op.contingencies()
                )
                .unwrap();
                if let Some(resolution) = op.resolution() {
                    writeln!(
                        out,
                        "Outcome: {:?} | debrief report {}",
                        resolution.objective_outcome(),
                        resolution.after_action_report().raw()
                    )
                    .unwrap();
                    if let Some(property) = resolution.property_proceeds() {
                        if op.property_disposition().is_some() {
                            out.push_str("Held property: liquidated.\n");
                        } else {
                            writeln!(out, "Held property estimate: {} cents; 'dispose' through an owned resale venue.",
                                property.estimated_value().cents()).unwrap();
                        }
                    }
                    if let Some(cash) = resolution.cash_proceeds() {
                        if op.cash_disposition().is_some() {
                            out.push_str("Held cash: deposited.\n");
                        } else {
                            writeln!(
                                out,
                                "Held cash: {} cents; use 'deposit' to bank proceeds.",
                                cash.amount().cents()
                            )
                            .unwrap();
                        }
                    }
                }
                if op.abort_record().is_some() {
                    out.push_str("Operation was aborted. See the reports for the crew's debrief.");
                }
            }
            "cancel" => {
                let raw = parse_number(words[1], "operation")?;
                let operation = state
                    .operations()
                    .operations_for_organization(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("operation is not owned by your organization")?
                    .id();
                apply_transition(
                    self.registry,
                    &mut self.state,
                    operation,
                    OperationTransition::Abort,
                )
                .map_err(|error| error.to_string())?;
                writeln!(out,
                    "Withdrew operation-{}. Its authoritative abort report was filed and assigned crew were released.",
                    operation.raw()).unwrap();
            }
            "dispose" => {
                if words.len() != 5 {
                    return Err("dispose <operation-id> <resale-business-id> <cash-account-id> <settlement-account-id>".to_owned());
                }
                let operation_raw = parse_number(words[1], "operation")?;
                let venue_raw = parse_number(words[2], "business")?;
                let cash_raw = parse_number(words[3], "account")?;
                let settle_raw = parse_number(words[4], "account")?;
                let operation = state
                    .operations()
                    .operations_for_organization(self.player)
                    .find(|record| record.id().raw() == operation_raw)
                    .ok_or("no owned operation with that ID")?
                    .id();
                let venue = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|record| record.id().raw() == venue_raw)
                    .ok_or("no owned resale venue with that ID")?
                    .id();
                let owner = FinancialOwner::Organization(self.player);
                let cash_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == cash_raw)
                    .ok_or("cash account is not owned by your organization")?
                    .id();
                let settlement_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == settle_raw)
                    .ok_or("settlement account is not owned by your organization")?
                    .id();
                let outcome = validate_dispose_property(
                    self.registry,
                    &self.state,
                    PropertyDispositionDraft {
                        operation,
                        venue,
                        cash_account,
                        settlement_account,
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Liquidated operation {operation_raw} property for {} cents (transaction {}).",
                    outcome.realized_value.cents(),
                    outcome.transaction.raw()
                )
                .unwrap();
            }
            "deposit" => {
                if words.len() != 4 {
                    return Err(
                        "deposit <operation-id> <cash-account-id> <settlement-account-id>"
                            .to_owned(),
                    );
                }
                let operation_raw = parse_number(words[1], "operation")?;
                let cash_raw = parse_number(words[2], "account")?;
                let settle_raw = parse_number(words[3], "account")?;
                let operation = state
                    .operations()
                    .operations_for_organization(self.player)
                    .find(|record| record.id().raw() == operation_raw)
                    .ok_or("no owned operation with that ID")?
                    .id();
                let owner = FinancialOwner::Organization(self.player);
                let cash_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == cash_raw)
                    .ok_or("cash account is not owned by your organization")?
                    .id();
                let settlement_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == settle_raw)
                    .ok_or("settlement account is not owned by your organization")?
                    .id();
                let outcome = validate_deposit_operation_cash(
                    &self.state,
                    CashDispositionDraft {
                        operation,
                        cash_account,
                        settlement_account,
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Deposited {} cents from operation {operation_raw} (transaction {}).",
                    outcome.deposited_value.cents(),
                    outcome.transaction.raw()
                )
                .unwrap();
            }
            "decisions" => {
                for decision in state.decisions().pending_for_recipient(self.player) {
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
                for report in state.reports().reports_for(self.player).rev().take(count) {
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
                let id = parse_number(words.get(1).ok_or("report needs an ID")?, "report")?;
                let report = state
                    .reports()
                    .reports_for(self.player)
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
                if words.len() > 3 {
                    return Err("scout <business-id> [scout-character-id]".to_owned());
                }
                let id = parse_number(
                    words.get(1).ok_or("scout needs a known business ID")?,
                    "business",
                )?;
                let business = self
                    .known_business(id)
                    .ok_or("no known business with that ID")?;
                let at = state
                    .now()
                    .checked_add(SimDuration::ONE_MINUTE)
                    .ok_or("simulation clock exhausted")?;
                let mut explicit = BTreeMap::new();
                if let Some(raw) = words.get(2) {
                    explicit.insert(RoleKind::Surveillance, self.requested_member(raw)?);
                }
                let roles =
                    self.select_operation_roles(OperationKind::Surveillance, &explicit, at, &[])?;
                let scout = roles[&RoleKind::Surveillance];
                let name = state
                    .world()
                    .get_business(business)
                    .ok_or("known business disappeared")?
                    .name()
                    .to_owned();
                let operation = validate_authorize_operation(
                    self.registry,
                    &self.state,
                    OperationDraft {
                        title: format!("Observe {name}"),
                        kind: OperationKind::Surveillance,
                        responsible_organization: self.player,
                        leader: scout,
                        objective: OperationObjective::GatherInformation {
                            target: EntityRef::Business(business),
                        },
                        approach: OperationApproach::Covert,
                        roles,
                        intelligence: BTreeSet::new(),
                        constraints: Vec::new(),
                        contingencies: Vec::new(),
                        scheduled_for: at,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Scouting authorized as operation {}. Use 'advance' to see results.",
                    operation.raw()
                )
                .unwrap();
            }
            "burgle" => {
                if words.len() > 5 {
                    return Err("burgle <lead-id> [entry-character-id] [coordinator-character-id] [abort|ask|press]".to_owned());
                }
                let raw =
                    parse_number(words.get(1).ok_or("burgle needs a lead ID")?, "opportunity")?;
                let mut params = &words[2..];
                let police_mode =
                    if let Some(mode) = params.last().and_then(|raw| PoliceMode::parse(raw).ok()) {
                        params = &params[..params.len() - 1];
                        mode
                    } else {
                        PoliceMode::Abort
                    };
                if params.len() > 2 {
                    return Err("burgle <lead-id> [entry-character-id] [coordinator-character-id] [abort|ask|press]".to_owned());
                }
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
                let at = state
                    .now()
                    .checked_add(SimDuration::ONE_MINUTE)
                    .ok_or("simulation clock exhausted")?;
                if lead.valid_until().is_some_and(|deadline| at >= deadline) {
                    return Err("lead expires before the proposed operation can begin".to_owned());
                }
                let mut explicit = BTreeMap::new();
                for (raw, role) in params
                    .iter()
                    .zip([RoleKind::EntrySpecialist, RoleKind::Coordinator])
                {
                    explicit.insert(role, self.requested_member(raw)?);
                }
                let roles =
                    self.select_operation_roles(OperationKind::Burglary, &explicit, at, &[])?;
                let coordinator = roles[&RoleKind::Coordinator];
                let objective = OperationObjective::AcquireProperty {
                    target: EntityRef::Business(target),
                };
                let coverage = planning_information(
                    self.registry,
                    &self.state,
                    self.player,
                    OperationKind::Burglary,
                    &objective,
                    at,
                );
                let information_count = coverage.len();
                let intelligence = coverage.values().map(|(id, _)| *id).collect();
                let contingencies = police_mode.contingencies();
                let operation = validate_authorize_operation(
                    self.registry,
                    &self.state,
                    OperationDraft {
                        title: format!("Burglary at {name}"),
                        kind: OperationKind::Burglary,
                        responsible_organization: self.player,
                        leader: coordinator,
                        objective,
                        approach: OperationApproach::Covert,
                        roles,
                        intelligence,
                        constraints: Vec::new(),
                        contingencies,
                        scheduled_for: at,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                validate_convert_opportunity(&self.state, lead_id, operation)
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Burglary {} authorized with {} information item(s); lead {} converted. Contingency: {}.",
                    operation.raw(),
                    information_count,
                    lead_id.raw(),
                    police_mode.label()
                )
                .unwrap();
            }
            "dismiss" => {
                let raw = parse_number(
                    words.get(1).ok_or("dismiss needs a lead ID")?,
                    "opportunity",
                )?;
                let lead = self
                    .known_opportunities()
                    .into_iter()
                    .find(|opp| opp.id().raw() == raw)
                    .ok_or("unknown lead")?;
                validate_dismiss_opportunity(self.registry, &self.state, lead.id())
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                writeln!(out, "Lead {raw} dismissed.").unwrap();
            }
            "decide" => {
                let raw = parse_number(
                    words.get(1).ok_or("decide needs a decision ID")?,
                    "decision",
                )?;
                let response = match words.get(2).copied() {
                    Some("continue") => DecisionResponse::Continue,
                    Some("abort") => DecisionResponse::Abort,
                    Some("approve") => DecisionResponse::Approve,
                    Some("reject") => DecisionResponse::Reject,
                    _ => return Err("decide <id> <continue|abort|approve|reject>".to_owned()),
                };
                let decision = state
                    .decisions()
                    .pending_for_recipient(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("no pending decision for your organization")?
                    .id();
                validate_resolve_decision(
                    self.registry,
                    &self.state,
                    decision,
                    self.player,
                    response,
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(out, "Decision {raw} resolved: {response:?}.").unwrap();
                if let Some(attempt) = self
                    .state
                    .recruitment()
                    .get_attempt_for_approval_decision(decision)
                {
                    let information = self
                        .state
                        .intelligence()
                        .get_information(attempt.outcome_information())
                        .expect("resolved recruitment attempt must have outcome information");
                    writeln!(out, "  {}", information.summary()).unwrap();
                }
            }
            "advance" => out = self.execute_advance(words[1])?,
            "save" => out = self.save(words.get(1).ok_or("save needs a slot name")?)?,
            _ => return Err(format!("unknown command '{verb}'; type 'help'")),
        }
        Ok((out, false))
    }
}
