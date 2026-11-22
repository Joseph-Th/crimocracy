//! Player racket establishment, outlook, and lifecycle commands.

use super::*;
use crimocracy::enterprises::enterprise_reporting::EnterpriseOutlook;

fn write_enterprise_outlook(out: &mut String, outlook: EnterpriseOutlook) {
    writeln!(
        out,
        "  Expected cycle net with no street-heat surcharge: {} cents",
        outlook.baseline_net_cash.cents()
    )
    .unwrap();
    match outlook.nonpositive_at_cases {
        Some(0) => writeln!(out, "  Expected net is nonpositive even without case pressure.").unwrap(),
        Some(cases) => writeln!(out, "  Expected net becomes nonpositive at {cases} own-operation case equivalent(s); each adds {} cents to cycle cost.", outlook.heat_surcharge_per_case.cents()).unwrap(),
        None => writeln!(out, "  No authored per-case operating surcharge.").unwrap(),
    }
    match outlook.observed_pressure {
        Some((at, cases)) => writeln!(out, "  Last observed own-operation pressure in district: {cases} case equivalent(s) at {}", time_label(at.as_minutes())).unwrap(),
        None => writeln!(out, "  Own-operation pressure unknown: no recent owned racket settlement; estimate assumes no surcharge").unwrap(),
    }
    writeln!(
        out,
        "  Conditional operating cost: {} cents | net: {} cents (before variance)",
        outlook.estimated_operating_cost.cents(),
        outlook.estimated_net_cash.cents()
    )
    .unwrap();
    // Show a one- and two-case sensitivity without disclosing the actual police case graph.
    // These scenarios keep gross and staffing fixed; they are not omniscient forecasts.
    let surcharge = i128::from(outlook.heat_surcharge_per_case.cents());
    if surcharge > 0 {
        let conditional_net = i128::from(outlook.estimated_net_cash.cents());
        writeln!(
            out,
            "  If district case pressure rises by 1: {} cents net; by 2: {} cents net (before variance)",
            conditional_net - surcharge,
            conditional_net - 2 * surcharge
        )
        .unwrap();
    }
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

impl Session<'_> {
    /// A read-only recovery register. The player chooses each resumption; it never creates
    /// an autonomous policy or silently restarts a suspended criminal operation.
    pub(super) fn recovery(&self) -> String {
        let mut out = String::from("Suspended racket recovery (read-only):\n");
        let mut count = 0_usize;
        for enterprise in self
            .state
            .enterprises()
            .enterprises_for_organization(self.player)
            .filter(|enterprise| enterprise.status() == EnterpriseStatus::Suspended)
        {
            count += 1;
            let manager = self
                .state
                .world()
                .get_character(enterprise.manager())
                .expect("owned racket manager must exist");
            let detained = self
                .state
                .legal()
                .active_arrest_for_character(manager.id())
                .is_some();
            writeln!(
                out,
                "  enterprise-{} {:?} at {} | manager {} (char-{}, {})",
                enterprise.id().raw(),
                enterprise.kind(),
                self.enterprise_location_label(enterprise.location()),
                manager.name(),
                manager.id().raw(),
                if detained { "detained" } else { "available" }
            )
            .unwrap();
            if let Some(cycle) = self.state.enterprises().latest_cycle(enterprise.id()) {
                writeln!(
                    out,
                    "    Last settlement: {} | net {} cents",
                    time_label(cycle.occurred_at().as_minutes()),
                    cycle.net_cash().cents()
                )
                .unwrap();
            }
            match crimocracy::enterprises::enterprise_reporting::resolve_enterprise_outlook(
                self.registry,
                &self.state,
                self.player,
                enterprise.id(),
            ) {
                Ok(outlook) => {
                    writeln!(out, "    Estimated next cycle: {} cents net | shared spendable cash: {} cents{}",
                        outlook.estimated_net_cash.cents(), outlook.shared_spendable_cash.cents(),
                        if outlook.observed_pressure.is_none() { " | district pressure unobserved (zero assumed)" } else { "" }).unwrap();
                    if outlook.shared_spendable_cash < outlook.estimated_operating_cost {
                        writeln!(out, "    Warning: shared cash is below one estimated cycle's operating cost ({} cents); reopening can be valid but unfunded.", outlook.estimated_operating_cost.cents()).unwrap();
                    }
                }
                Err(error) => writeln!(out, "    Outlook unavailable: {error}").unwrap(),
            }
            match validate_resume_enterprise(self.registry, &self.state, enterprise.id()) {
                Ok(_) => writeln!(
                    out,
                    "    Can resume manually: racket {} resume",
                    enterprise.id().raw()
                )
                .unwrap(),
                Err(error) => writeln!(out, "    Resumption blocked: {error}").unwrap(),
            }
        }
        if count == 0 {
            out.push_str("  No suspended rackets.\n");
        }
        out.push_str("Suspended storefront recovery (read-only):\n");
        let mut storefronts = 0_usize;
        for business in self
            .state
            .world()
            .businesses_owned_by_organization(self.player)
        {
            let Some(economy) = self.state.economy().get_business_economy(business.id()) else {
                continue;
            };
            if economy.status() != crimocracy::economy::BusinessOperatingStatus::Suspended {
                continue;
            }
            storefronts += 1;
            writeln!(
                out,
                "  business-{} {} | district-{}",
                business.id().raw(),
                business.name(),
                business.neighborhood().raw()
            )
            .unwrap();
            if let Some(cycle) =
                self.state
                    .economy()
                    .cycles_for(business.id())
                    .rev()
                    .find(|cycle| {
                        cycle.owner() == crimocracy::world::BusinessOwner::Organization(self.player)
                    })
            {
                writeln!(
                    out,
                    "    Last settlement: {} | net {:+} cents | disrupted: {}",
                    time_label(cycle.occurred_at().as_minutes()),
                    cycle.net_cash().cents(),
                    cycle.disrupted()
                )
                .unwrap();
            }
            match validate_resume_business_economy(self.registry, &self.state, business.id()) {
                Ok(_) => writeln!(
                    out,
                    "    Can resume manually: business {} resume",
                    business.id().raw()
                )
                .unwrap(),
                Err(error) => writeln!(out, "    Resumption blocked: {error}").unwrap(),
            }
        }
        if storefronts == 0 {
            out.push_str("  No suspended storefronts.\n");
        }
        out.push_str("Rackets and storefronts remain suspended until resumed by the player. Use 'cover-rackets <char-N> <id> [id...]' to nominate a replacement manager and reopen selected rackets atomically, or 'resume-rackets <id> [id...]' when no handoff is needed. Repeated shared-account balances are not separate reserves. Outlooks are estimates, not forecasts of undisclosed police cases.");
        out
    }

    pub(super) fn execute_racket_commands(&mut self, words: &[&str]) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        match words[0] {
            "enterprises" => {
                if words.len() != 1 {
                    return Err("enterprises takes no arguments".to_owned());
                }
                let mut active_by_manager = BTreeMap::<CharacterId, Vec<EnterpriseId>>::new();
                for enterprise in state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                {
                    if enterprise.status() == EnterpriseStatus::Active {
                        active_by_manager
                            .entry(enterprise.manager())
                            .or_default()
                            .push(enterprise.id());
                    }
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
                    writeln!(out, "  Rank-and-file associates: {} (baseline {}, maximum {}); additional hires earn 1500 cents/day each in operating costs",
                        enterprise.associates(), enterprise.kind().baseline_associates(),
                        enterprise.kind().max_associates()).unwrap();
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
                for (manager, ids) in active_by_manager {
                    if ids.len() > 1 {
                        let name = state
                            .world()
                            .get_character(manager)
                            .ok_or("manager disappeared")?
                            .name();
                        let rackets = ids
                            .iter()
                            .map(|id| id.raw().to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(out, "Management concentration: {name} (char-{}) oversees {} active rackets ({rackets}). Detention would suspend all of them. Use delegate and reassign-racket to distribute authority.", manager.raw(), ids.len()).unwrap();
                    }
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
                if words[0] == "preview-racket" {
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
                    let existing = state
                        .enterprises()
                        .enterprises_for_organization(self.player)
                        .filter(|record| {
                            record.status() == EnterpriseStatus::Active
                                && record.manager() == authority.manager
                        })
                        .count();
                    if existing > 0 {
                        writeln!(out, "  Management concentration: this investment would put {} active rackets under {manager_label}. Detention would suspend them together.", existing + 1).unwrap();
                    }
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
            "cover-rackets" => {
                // One explicit player order covers a nominated set of suspended rackets with
                // one named manager. Execute canonical reassignment and resumption validators
                // against a projected state. All failures discard that projection, so neither
                // a manager-capacity block nor an invalid second racket leaves a partial handoff.
                let manager = self.requested_member(words[1])?;
                let mut selected = BTreeSet::new();
                for raw in &words[2..] {
                    let raw_id = parse_number(raw, "enterprise")?;
                    let id = self
                        .state
                        .enterprises()
                        .enterprises_for_organization(self.player)
                        .find(|record| record.id().raw() == raw_id)
                        .ok_or(format!("enterprise-{raw_id} is not yours"))?
                        .id();
                    if !selected.insert(id) {
                        return Err(format!("enterprise-{raw_id} was selected more than once"));
                    }
                }
                let mut projected = self.state.clone();
                for id in &selected {
                    let racket = projected
                        .enterprises()
                        .get_enterprise(*id)
                        .expect("selected enterprise must exist");
                    let authority = self.enterprise_authority(
                        racket.location(),
                        racket.supporting_businesses(),
                        Some(manager),
                    )?;
                    if racket.manager() != manager {
                        crimocracy::enterprises::enterprise_execution::validate_reassign_enterprise(
                            &projected,
                            *id,
                            authority,
                        )
                        .map_err(|error| format!("enterprise-{}: {error}", id.raw()))?
                        .commit(&mut projected)
                        .map_err(|error| format!("enterprise-{}: {error}", id.raw()))?;
                    }
                    validate_resume_enterprise(self.registry, &projected, *id)
                        .map_err(|error| format!("enterprise-{}: {error}", id.raw()))?
                        .commit(&mut projected)
                        .map_err(|error| format!("enterprise-{}: {error}", id.raw()))?;
                }
                self.state = projected;
                writeln!(
                    out,
                    "Assigned char-{} and resumed {} racket(s): {}. All changes committed together.",
                    manager.raw(),
                    selected.len(),
                    selected
                        .iter()
                        .map(|id| format!("enterprise-{}", id.raw()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ).unwrap();
            }
            "resume-rackets" => {
                // The player explicitly selects every racket. Validate the full cohort
                // before resuming any of them, so a blocked manager cannot cause a
                // partial batch. Independent version-pinned status changes then commit
                // in stable enterprise-ID order without changing shared finances.
                let mut selected = BTreeSet::new();
                for raw in &words[1..] {
                    let id = parse_number(raw, "enterprise")?;
                    let enterprise = state
                        .enterprises()
                        .enterprises_for_organization(self.player)
                        .find(|enterprise| enterprise.id().raw() == id)
                        .ok_or(format!("enterprise-{id} is not yours"))?
                        .id();
                    if !selected.insert(enterprise) {
                        return Err(format!("enterprise-{id} was selected more than once"));
                    }
                }
                let plans = selected
                    .iter()
                    .map(|id| {
                        validate_resume_enterprise(self.registry, &self.state, *id)
                            .map_err(|error| format!("enterprise-{}: {error}", id.raw()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for plan in plans {
                    plan.commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                }
                writeln!(
                    out,
                    "Resumed {} selected racket(s): {}. Every requested resumption passed validation before the first change.",
                    selected.len(),
                    selected.iter().map(|id| format!("enterprise-{}", id.raw())).collect::<Vec<_>>().join(", ")
                ).unwrap();
            }
            "reassign-racket" => {
                let raw = parse_number(words[1], "enterprise")?;
                let record = state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                    .find(|enterprise| enterprise.id().raw() == raw)
                    .ok_or("no owned enterprise with that ID")?;
                let manager = self.requested_member(words[2])?;
                let authority = self.enterprise_authority(
                    record.location(),
                    record.supporting_businesses(),
                    Some(manager),
                )?;
                let old_manager = state
                    .world()
                    .get_character(record.manager())
                    .ok_or("manager disappeared")?
                    .name()
                    .to_owned();
                let new_manager = state
                    .world()
                    .get_character(manager)
                    .ok_or("manager disappeared")?
                    .name()
                    .to_owned();
                let status = record.status();
                crimocracy::enterprises::enterprise_execution::validate_reassign_enterprise(
                    state,
                    record.id(),
                    authority,
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(out, "Enterprise {raw}: {old_manager} handed responsibility to {new_manager}. Existing accounts and cycle history retained.").unwrap();
                if status == crimocracy::enterprises::EnterpriseStatus::Suspended {
                    writeln!(
                        out,
                        "Still suspended; use 'racket {raw} resume' when ready."
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
            "staff-racket" => {
                let raw = parse_number(words[1], "enterprise")?;
                let associates = words[2]
                    .parse::<u16>()
                    .map_err(|_| "associate count must be a nonnegative integer".to_owned())?;
                let enterprise = state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                    .find(|enterprise| enterprise.id().raw() == raw)
                    .ok_or("no owned enterprise with that ID")?
                    .id();
                crimocracy::enterprises::enterprise_execution::validate_staff_enterprise(
                    &self.state,
                    enterprise,
                    associates,
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(out, "Racket enterprise-{raw}: {associates} rank-and-file associates assigned. Extra pay and local-demand output take effect at the next settlement.").unwrap();
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
        Ok(out)
    }
}
