//! Player command dispatch through canonical production owners.

use super::planning::PoliceMode;
use super::*;
use crimocracy::enterprises::enterprise_reporting::EnterpriseOutlook;
use crimocracy::intelligence::InformationTopic;

fn write_enterprise_outlook(out: &mut String, outlook: EnterpriseOutlook) {
    writeln!(
        out,
        "  Expected cycle net with no street-heat surcharge: {} cents",
        outlook.baseline_net_cash.cents()
    )
    .unwrap();
    match outlook.nonpositive_at_cases {
        Some(0) => writeln!(out, "  Expected net is nonpositive even without case pressure.").unwrap(),
        Some(cases) => writeln!(out, "  Expected net becomes nonpositive at {cases} active-case equivalent(s); each adds {} cents to cycle cost.", outlook.heat_surcharge_per_case.cents()).unwrap(),
        None => writeln!(out, "  No authored per-case operating surcharge.").unwrap(),
    }
    match outlook.observed_pressure {
        Some((at, cases)) => writeln!(out, "  Last observed district pressure: {cases} active-case equivalent(s) at {}", time_label(at.as_minutes())).unwrap(),
        None => writeln!(out, "  District pressure unknown: no recent owned racket settlement; estimate assumes no surcharge").unwrap(),
    }
    writeln!(
        out,
        "  Conditional operating cost: {} cents | net: {} cents (before variance)",
        outlook.estimated_operating_cost.cents(),
        outlook.estimated_net_cash.cents()
    )
    .unwrap();
    writeln!(
        out,
        "  Shared account balance: {} cents | spendable cash: {} cents",
        outlook.shared_cash_balance.cents(),
        outlook.shared_spendable_cash.cents()
    )
    .unwrap();
    if outlook.shared_cash_balance < Money::ZERO {
        writeln!(
            out,
            "  Account has an operating deficit, not spendable funds."
        )
        .unwrap();
    }
    if outlook.shared_spendable_cash < outlook.estimated_operating_cost {
        writeln!(
            out,
            "  Shared balance is below one estimated cycle's operating cost."
        )
        .unwrap();
    }
    writeln!(
        out,
        "  Not a guarantee of current enforcement conditions or other rackets' cash needs."
    )
    .unwrap();
}

use crimocracy::world::{AutonomyLevel, BusinessOwner, CharacterRecord};

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

impl Session<'_> {
    pub(super) fn execute(&mut self, command: &str) -> Result<(String, bool), String> {
        let words: Vec<_> = command.split_whitespace().collect();
        let Some(verb) = words.first().copied() else {
            return Ok((String::new(), false));
        };
        validate_command_arity(&verb.to_ascii_lowercase(), words.len())?;
        let state = &self.state;
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
            "treasury" => {
                match words.get(1).copied() {
                    None => {}
                    Some("manual") => self.state.set_automatic_profit_sweeps(false),
                    Some("daily") => self.state.set_automatic_profit_sweeps(true),
                    Some(_) => return Err("treasury [manual|daily]".to_owned()),
                }
                out = if self.state.automatic_profit_sweeps() {
                    "Treasury: daily distribution of earned business surplus before payroll; retained operating capital stays protected."
                } else {
                    "Treasury: manual owner withdrawals; use 'businesses' and 'sweep' to distribute earned surplus."
                }.to_owned();
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
                            && crimocracy::recruitment::recruitment_system::recruitment_approval_is_deferred(
                                self.registry, state, self.player, candidate,
                            )
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
            "directory" => {
                if words.len() != 1 {
                    return Err("directory takes no arguments".to_owned());
                }
                for (id, name, kind, district) in state.world().public_business_directory() {
                    let ward = state
                        .world()
                        .get_neighborhood(district)
                        .map_or("unknown district", |ward| ward.name());
                    let price = self
                        .registry
                        .get_business(kind)
                        .economics()
                        .acquisition_cost();
                    writeln!(
                        out,
                        "{} {} | {:?} | district-{} {} | standard acquisition price {} cents",
                        id.raw(),
                        name,
                        kind,
                        district.raw(),
                        ward,
                        price.cents()
                    )
                    .unwrap();
                }
            }
            "buy" | "offer" => {
                let raw = parse_number(words[1], "business")?;
                let business = self
                    .known_business(raw)
                    .ok_or("property is not in the public directory")?;
                let funding_accounts: BTreeSet<_> = state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                    .filter(|account| account.kind() == AccountKind::AccountedFunds)
                    .map(|account| account.id())
                    .collect();
                let draft = BusinessAcquisitionDraft {
                    organization: self.player,
                    business,
                    funding_accounts,
                };
                let validated = if verb.eq_ignore_ascii_case("offer") {
                    let cents = words[2]
                        .parse::<i64>()
                        .ok()
                        .filter(|cents| *cents > 0)
                        .ok_or("offer needs a positive amount in cents")?;
                    validate_offer_business(
                        self.registry,
                        &self.state,
                        draft,
                        Money::from_cents(cents),
                    )
                } else {
                    validate_acquire_business(self.registry, &self.state, draft)
                }
                .map_err(|error| {
                    if matches!(
                        &error,
                        BusinessAcquisitionError::NotIndependentlyOwned { .. }
                    ) {
                        "property is not available for direct purchase".to_owned()
                    } else {
                        error.to_string()
                    }
                })?;
                let acquired = validated
                    .commit(self.registry, &mut self.state)
                    .map_err(|e| e.to_string())?;
                writeln!(out, "Acquired business {} for {} accounted cents; initialized operating economy: {}.",
                    acquired.business.raw(), acquired.price.cents(), acquired.established_economy).unwrap();
                for opportunity in acquired.dismissed_opportunities {
                    writeln!(
                        out,
                        "Lead {} dismissed: you now own its target.",
                        opportunity.raw()
                    )
                    .unwrap();
                }
            }
            "market" => {
                for (id, name, kind, district) in state.world().public_business_directory() {
                    let Some((price, functions)) =
                        acquisition_offer(self.registry, state, self.player, id)
                    else {
                        continue;
                    };
                    let ward = state
                        .world()
                        .get_neighborhood(district)
                        .map_or("unknown district", |ward| ward.name());
                    writeln!(out, "business-{} {name} | {kind:?} | district-{} {ward} | {} cents | functions {functions:?}", id.raw(), district.raw(), price.cents()).unwrap();
                }
                if out.is_empty() {
                    out.push_str("No properties currently available for direct purchase. Use directory to identify properties and offer <business-id> <cents> to negotiate with an existing organization owner.");
                }
            }
            "business" => {
                let raw = parse_number(words[1], "business")?;
                let id = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|business| business.id().raw() == raw)
                    .ok_or("business is not owned by your organization")?
                    .id();
                match words[2] {
                    "suspend" => validate_suspend_business_economy(&self.state, id)
                        .map_err(|error| error.to_string())?
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?,
                    "resume" => validate_resume_business_economy(self.registry, &self.state, id)
                        .map_err(|error| error.to_string())?
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?,
                    _ => return Err("business <business-id> <suspend|resume>".to_owned()),
                }
                let economy = self
                    .state
                    .economy()
                    .get_business_economy(id)
                    .ok_or("owned business operating books disappeared")?;
                writeln!(out, "Business {raw}: {:?}.", economy.status()).unwrap();
            }
            "launder" => {
                if words.len() != 3 {
                    return Err("launder <business-id> <cents>".to_owned());
                }
                let raw = parse_number(words[1], "business")?;
                let cents = words[2]
                    .parse::<i64>()
                    .ok()
                    .filter(|cents| *cents > 0)
                    .ok_or("launder needs a positive amount in cents")?;
                let business = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|business| business.id().raw() == raw)
                    .ok_or("not an owned business")?
                    .id();
                let accounted_account = state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                    .find(|account| account.kind() == AccountKind::AccountedFunds)
                    .ok_or("no accounted-funds account")?
                    .id();
                let street_account = state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                    .filter(|account| {
                        account.kind() == AccountKind::StreetCash
                            && account.balance().cents() >= cents
                    })
                    .max_by_key(|account| {
                        (
                            account.balance().cents(),
                            std::cmp::Reverse(account.id().raw()),
                        )
                    })
                    .ok_or("no street-cash account holds the requested amount")?
                    .id();
                let capacity = remaining_laundering_capacity(
                    self.registry,
                    &self.state,
                    self.player,
                    business,
                )
                .ok_or("business has no active cash-intensive laundering channel")?;
                if capacity.cents() < cents {
                    return Err(format!(
                        "front can absorb only {} more cents this cycle",
                        capacity.cents()
                    ));
                }
                let transaction = validate_launder_funds(
                    self.registry,
                    &self.state,
                    LaunderingDraft {
                        organization: self.player,
                        street_account,
                        business,
                        accounted_account,
                        amount: Money::from_cents(cents),
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(out, "Laundering settled as transaction {}. Fees and accounted proceeds are recorded in the ledger.", transaction.raw()).unwrap();
            }
            "businesses" => {
                out.push_str("Owned storefronts:\n");
                for business in state.world().businesses_owned_by_organization(self.player) {
                    writeln!(
                        out,
                        "{} {} | {:?}",
                        business.id().raw(),
                        business.name(),
                        business.kind()
                    )
                    .unwrap();
                    writeln!(out, "  Infrastructure: {:?}", business.functions()).unwrap();
                    if let Some(economy) = state.economy().get_business_economy(business.id()) {
                        let till = state
                            .finance()
                            .get_account(economy.operating_account())
                            .ok_or("operating books reference a missing account")?;
                        let withdrawable = crimocracy::economy::business_economy_system::resolve_available_business_profit(
                            state, self.player, business.id(),
                        ).map_err(|error| error.to_string())?.cents();
                        let laundering = remaining_laundering_capacity(
                            self.registry,
                            state,
                            self.player,
                            business.id(),
                        )
                        .map_or(0, |money| money.cents());
                        writeln!(out,
                            "  {:?} | till {} cents | protected capital {} cents | available owner draw {} cents | laundering room {} cents",
                            economy.status(), till.balance().cents(),
                            economy.operating_capital_floor().cents(), withdrawable, laundering,
                        ).unwrap();
                    }
                }
                out.push_str("Known business leads:\n");
                let mut seen = BTreeSet::new();
                for information in self.organization_information() {
                    if let EntityRef::Business(id) = information.subject()
                        && matches!(
                            information.topic(),
                            InformationTopic::TargetSecurity | InformationTopic::MarketAccess
                        )
                        && seen.insert(id)
                        && let Some(business) = state.world().get_business(id)
                        && business.owner() != BusinessOwner::Organization(self.player)
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
            "enterprises" => {
                if words.len() != 1 {
                    return Err("enterprises takes no arguments".to_owned());
                }
                for enterprise in state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                {
                    writeln!(
                        out,
                        "{} {:?} | {:?} at {} | next settlement {}",
                        enterprise.id().raw(),
                        enterprise.kind(),
                        enterprise.status(),
                        self.enterprise_location_label(enterprise.location()),
                        enterprise
                            .next_cycle_at()
                            .map_or("none".to_owned(), |at| time_label(at.as_minutes()),)
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "  Manager char-{} | cash account-{} | settlement account-{} | supports {:?}",
                        enterprise.manager().raw(),
                        enterprise.cash_account().raw(),
                        enterprise.settlement_account().raw(),
                        enterprise
                            .supporting_businesses()
                            .iter()
                            .map(|id| format!("business-{}", id.raw()))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    let book = state
                        .finance()
                        .get_account(enterprise.cash_account())
                        .ok_or("enterprise cash account is missing")?;
                    writeln!(out, "  Shared cash account balance: {} cents (not an individual racket reserve)", book.balance().cents()).unwrap();
                    if let Some(cycle) = state.enterprises().latest_cycle(enterprise.id()) {
                        writeln!(
                            out,
                            "  Last cycle: {} | net cash {} cents | enforcement attention: {}",
                            time_label(cycle.occurred_at().as_minutes()),
                            cycle.net_cash().cents(),
                            cycle.drew_enforcement_attention()
                        )
                        .unwrap();
                    }
                }
                if out.is_empty() {
                    out.push_str("No enterprises owned.");
                }
            }
            "racket-outlook" => {
                let raw = parse_number(words[1], "enterprise")?;
                let record = state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                    .find(|enterprise| enterprise.id().raw() == raw)
                    .ok_or("no owned enterprise with that ID")?;
                let outlook =
                    crimocracy::enterprises::enterprise_reporting::resolve_enterprise_outlook(
                        self.registry,
                        state,
                        self.player,
                        record.id(),
                    )
                    .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Enterprise-{raw} {:?} | {:?}",
                    record.kind(),
                    record.status()
                )
                .unwrap();
                write_enterprise_outlook(&mut out, outlook);
            }
            "racket-kinds" => {
                if words.len() != 1 {
                    return Err("racket-kinds takes no arguments".to_owned());
                }
                for kind in ALL_ENTERPRISE_KINDS {
                    let definition = self.registry.get_enterprise(kind);
                    let location_hint = if definition.required_business_functions().is_empty() {
                        "business-N or district-N (district uses its supporting network)"
                    } else {
                        "business-N (physical host required)"
                    };
                    writeln!(
                        out,
                        "{kind:?} | host needs {:?} | network needs {:?} | {location_hint}",
                        definition.required_business_functions(),
                        definition.required_network_functions()
                    )
                    .unwrap();
                }
            }
            "start-racket" | "preview-racket" => {
                let kind = parse_enterprise_kind(words[1])?;
                let location = if words[2].starts_with("district-") {
                    if !self
                        .registry
                        .get_enterprise(kind)
                        .required_business_functions()
                        .is_empty()
                    {
                        return Err(format!(
                            "{kind:?} requires a physical host with authored functions; use business-N"
                        ));
                    }
                    let raw = parse_number(words[2], "district")?;
                    let district = state
                        .world()
                        .neighborhoods()
                        .find(|record| record.id().raw() == raw)
                        .ok_or("district is not in the public city directory")?
                        .id();
                    EnterpriseLocation::Neighborhood(district)
                } else {
                    let raw = parse_number(words[2], "business")?;
                    let business = state
                        .world()
                        .businesses_owned_by_organization(self.player)
                        .find(|record| record.id().raw() == raw)
                        .ok_or("venue is not one of your owned businesses")?
                        .id();
                    EnterpriseLocation::Business(business)
                };
                let cash = parse_number(words[3], "account")?;
                let settlement = parse_number(words[4], "account")?;
                let owner = FinancialOwner::Organization(self.player);
                let cash_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|account| account.id().raw() == cash)
                    .ok_or("cash account is not owned by your organization")?
                    .id();
                let settlement_account = state
                    .finance()
                    .accounts_for(owner)
                    .find(|account| account.id().raw() == settlement)
                    .ok_or("settlement account is not owned by your organization")?
                    .id();
                let mut supporting_businesses = BTreeSet::new();
                let mut requested_manager = None;
                for raw in &words[5..] {
                    if let Some(raw_manager) = raw.strip_prefix("manager=") {
                        if requested_manager.is_some() {
                            return Err("manager was specified more than once".to_owned());
                        }
                        requested_manager = Some(self.requested_member(raw_manager)?);
                        continue;
                    }
                    let support = parse_number(raw, "business")?;
                    let business = state
                        .world()
                        .businesses_owned_by_organization(self.player)
                        .find(|record| record.id().raw() == support)
                        .ok_or("support property is not owned by your organization")?
                        .id();
                    if !supporting_businesses.insert(business) {
                        return Err("support business was listed more than once".to_owned());
                    }
                }
                let authority =
                    self.enterprise_authority(location, &supporting_businesses, requested_manager)?;
                let manager_label = state
                    .world()
                    .get_character(authority.manager)
                    .ok_or("manager disappeared")?
                    .name()
                    .to_owned();
                let support_count = supporting_businesses.len();
                let draft = EnterpriseDraft {
                    kind,
                    organization: self.player,
                    authority,
                    location,
                    supporting_businesses,
                    cash_account,
                    settlement_account,
                };
                if verb.eq_ignore_ascii_case("preview-racket") {
                    let outlook = crimocracy::enterprises::enterprise_reporting::preview_enterprise_establishment(
                        self.registry, state, &draft,
                    ).map_err(|error| error.to_string())?;
                    writeln!(
                        out,
                        "Preview only: {kind:?} at {}.",
                        self.enterprise_location_label(location)
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "  Manager: {manager_label} (char-{}) | scope {:?}",
                        authority.manager.raw(),
                        authority.scope
                    )
                    .unwrap();
                    write_enterprise_outlook(&mut out, outlook);
                } else {
                    let established = validate_establish_enterprise(self.registry, state, draft)
                        .map_err(|error| error.to_string())?
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                    writeln!(
                        out,
                        "Established {kind:?} enterprise {} at {}, using {support_count} supporting business(es). Manager: {manager_label} (char-{}).",
                        established.raw(),
                        self.enterprise_location_label(location),
                        authority.manager.raw(),
                    )
                    .unwrap();
                }
            }
            "racket" => {
                if words.len() != 3 {
                    return Err("racket <enterprise-id> <suspend|resume|retire>".to_owned());
                }
                let raw = parse_number(words[1], "enterprise")?;
                let id = state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                    .find(|enterprise| enterprise.id().raw() == raw)
                    .ok_or("no owned enterprise with that ID")?
                    .id();
                let validated = match words[2] {
                    "suspend" => validate_suspend_enterprise(&self.state, id),
                    "resume" => validate_resume_enterprise(self.registry, &self.state, id),
                    "retire" => validate_retire_enterprise(&self.state, id),
                    _ => return Err("racket <enterprise-id> <suspend|resume|retire>".to_owned()),
                }
                .map_err(|e| e.to_string())?;
                validated
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Enterprise {raw}: {:?}.",
                    self.state
                        .enterprises()
                        .get_enterprise(id)
                        .ok_or("enterprise disappeared")?
                        .status()
                )
                .unwrap();
            }
            "accounts" => {
                if words.len() != 1 {
                    return Err("accounts takes no arguments".to_owned());
                }
                for account in state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                {
                    writeln!(
                        out,
                        "{} {:?} | {} cents{}",
                        account.id().raw(),
                        account.kind(),
                        account.balance().cents(),
                        if account.kind() == AccountKind::Settlement {
                            " (clearing only)"
                        } else {
                            ""
                        }
                    )
                    .unwrap();
                }
            }
            "open-account" => {
                if words.len() != 2 {
                    return Err("open-account <street|concealed|accounted|settlement>".to_owned());
                }
                let kind = parse_account_kind(words[1])?;
                let id = insert_account(
                    &mut self.state,
                    FinancialAccountDraft {
                        owner: FinancialOwner::Organization(self.player),
                        kind,
                    },
                )
                .map_err(|error| error.to_string())?;
                writeln!(out, "Opened zero-balance {kind:?} account {}.", id.raw()).unwrap();
            }
            "move" => {
                if words.len() != 4 {
                    return Err(
                        "move <source-account-id> <destination-account-id> <cents>".to_owned()
                    );
                }
                let source_raw = parse_number(words[1], "account")?;
                let dest_raw = parse_number(words[2], "account")?;
                let cents = words[3]
                    .parse::<i64>()
                    .ok()
                    .filter(|cents| *cents > 0)
                    .ok_or("move amount must be a positive number of cents")?;
                let owner = FinancialOwner::Organization(self.player);
                let source = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == source_raw)
                    .ok_or("source is not a player-owned account")?;
                let dest = state
                    .finance()
                    .accounts_for(owner)
                    .find(|record| record.id().raw() == dest_raw)
                    .ok_or("destination is not a player-owned account")?;
                if source.id() == dest.id() {
                    return Err("source and destination must differ".to_owned());
                }
                if source.kind() != dest.kind()
                    || !matches!(
                        source.kind(),
                        AccountKind::StreetCash
                            | AccountKind::ConcealedCash
                            | AccountKind::AccountedFunds
                    )
                {
                    return Err("move only transfers within the same spendable money kind; laundering requires the 'launder' command".to_owned());
                }
                if source.balance().cents() < cents {
                    return Err(format!(
                        "source holds only {} cents",
                        source.balance().cents()
                    ));
                }
                let transaction = validate_record_transaction(
                    &self.state,
                    LedgerTransactionDraft {
                        occurred_at: self.state.now(),
                        memo: "Internal treasury allocation".to_owned(),
                        postings: vec![
                            LedgerPosting {
                                account: source.id(),
                                amount: Money::from_cents(-cents),
                            },
                            LedgerPosting {
                                account: dest.id(),
                                amount: Money::from_cents(cents),
                            },
                        ],
                        authorization: None,
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Allocated {cents} cents in transaction {}.",
                    transaction.raw()
                )
                .unwrap();
            }
            "finances" => {
                let mut totals = std::collections::BTreeMap::<AccountKind, i128>::new();
                for account in state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
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
                let relevant = self
                    .registry
                    .get_operation(OperationKind::Burglary)
                    .execution()
                    .relevant_intelligence_topics();
                let information = self.organization_information()
                    .filter(|info| info.subject() == EntityRef::Business(target)
                        && relevant.contains(&info.topic()))
                    .max_by_key(|info| (info.observed_at(), info.id()))
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
            "sweep" => {
                let raw =
                    parse_number(words.get(1).ok_or("sweep needs a business ID")?, "business")?;
                let cents = words
                    .get(2)
                    .ok_or("sweep needs a positive amount in cents")?
                    .parse::<i64>()
                    .map_err(|_| "invalid cents amount")?;
                let business = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|b| b.id().raw() == raw)
                    .ok_or("not an owned business")?
                    .id();
                let destination = state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                    .find(|account| account.kind() == AccountKind::AccountedFunds)
                    .ok_or("organization has no accounted-funds account")?
                    .id();
                validate_sweep_business_profits(
                    &self.state,
                    BusinessProfitSweepDraft {
                        organization: self.player,
                        business,
                        destination,
                        amount: Money::from_cents(cents),
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Withdrew {cents} cents of actual earned business surplus."
                )
                .unwrap();
            }
            "advance" => out = self.execute_advance(words[1])?,
            "save" => out = self.save(words.get(1).ok_or("save needs a slot name")?)?,
            _ => return Err(format!("unknown command '{verb}'; type 'help'")),
        }
        Ok((out, false))
    }
}
