//! Player command dispatch through canonical production owners.

use super::*;

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
            "prospects" => {
                if words.len() != 1 {
                    return Err("prospects takes no arguments".to_owned());
                }
                let prospects = self.player_prospects()?;
                if prospects.is_empty() {
                    out.push_str("No currently approachable contacts.");
                }
                for id in prospects {
                    if let Some(person) = state.world().get_character(id) {
                        writeln!(
                            out,
                            "{} {} | established connection to leadership",
                            id.raw(),
                            person.name()
                        )
                        .unwrap();
                    }
                }
            }
            "recruit" => {
                if !(2..=3).contains(&words.len()) {
                    return Err(
                        "recruit <character-id> [offer|advancement|protection|appeal]".to_owned(),
                    );
                }
                let raw = parse_number(words[1], "char")?;
                let candidate = self
                    .player_prospects()?
                    .into_iter()
                    .find(|id| id.raw() == raw)
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
                let recruiter = self.executive_recruiter()?;
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
                        "{} {} | {:?} | {} | standard acquisition price {} cents",
                        id.raw(),
                        name,
                        kind,
                        ward,
                        price.cents()
                    )
                    .unwrap();
                }
            }
            "buy" => {
                if words.len() != 2 {
                    return Err("buy <business-id>".to_owned());
                }
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
                let validated = validate_acquire_business(
                    self.registry,
                    &self.state,
                    BusinessAcquisitionDraft {
                        organization: self.player,
                        business,
                        funding_accounts,
                    },
                )
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
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                writeln!(out, "Acquired business {} for {} accounted cents; initialized operating economy: {}.",
                    acquired.business.raw(), acquired.price.cents(), acquired.established_economy).unwrap();
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
                    if let Some(economy) = state.economy().get_business_economy(business.id()) {
                        let till = state
                            .finance()
                            .get_account(economy.operating_account())
                            .ok_or("operating books reference a missing account")?;
                        let withdrawable = till
                            .balance()
                            .cents()
                            .saturating_sub(economy.operating_capital_floor().cents())
                            .max(0);
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
                        "{} {:?} | {:?} at {:?} | next settlement {}",
                        enterprise.id().raw(),
                        enterprise.kind(),
                        enterprise.status(),
                        enterprise.location(),
                        enterprise
                            .next_cycle_at()
                            .map_or("none".to_owned(), |at| time_label(at.as_minutes()),)
                    )
                    .unwrap();
                    let book = state
                        .finance()
                        .get_account(enterprise.cash_account())
                        .ok_or("enterprise cash account is missing")?;
                    writeln!(out, "  Book cash: {} cents", book.balance().cents()).unwrap();
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
            "racket-kinds" => {
                if words.len() != 1 {
                    return Err("racket-kinds takes no arguments".to_owned());
                }
                for kind in ALL_ENTERPRISE_KINDS {
                    let definition = self.registry.get_enterprise(kind);
                    writeln!(
                        out,
                        "{kind:?} | host needs {:?} | network needs {:?}",
                        definition.required_business_functions(),
                        definition.required_network_functions()
                    )
                    .unwrap();
                }
            }
            "start-racket" => {
                if !(5..=6).contains(&words.len()) {
                    return Err("start-racket <kind> <business-id> <cash-account-id> <settlement-account-id> [support-business-id]".to_owned());
                }
                let kind = parse_enterprise_kind(words[1])?;
                let venue = parse_number(words[2], "business")?;
                let business = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|record| record.id().raw() == venue)
                    .ok_or("venue is not one of your owned businesses")?
                    .id();
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
                let supporting_businesses = if let Some(raw) = words.get(5) {
                    let support = parse_number(raw, "business")?;
                    BTreeSet::from([state
                        .world()
                        .businesses_owned_by_organization(self.player)
                        .find(|record| record.id().raw() == support)
                        .ok_or("support property is not owned by your organization")?
                        .id()])
                } else {
                    BTreeSet::new()
                };
                let authority = self.enterprise_authority(business)?;
                let established = validate_establish_enterprise(
                    self.registry,
                    &self.state,
                    EnterpriseDraft {
                        kind,
                        organization: self.player,
                        authority,
                        location: EnterpriseLocation::Business(business),
                        supporting_businesses,
                        cash_account,
                        settlement_account,
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Established {kind:?} enterprise {} at business {}.",
                    established.raw(),
                    business.raw()
                )
                .unwrap();
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
                    "Planning intelligence: {} record(s); contingencies: {:?}",
                    op.intelligence().len(),
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
                for report in state
                    .reports()
                    .reports_for(self.player)
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
                let scout = self.member_for_role(
                    words.get(2).copied(),
                    CapabilityKind::Surveillance,
                    false,
                )?;
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
                        roles: BTreeMap::from([(RoleKind::Surveillance, scout)]),
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
                    return Err("burgle <lead-id> [entry-character-id] [coordinator-character-id] [abort|press]".to_owned());
                }
                let raw =
                    parse_number(words.get(1).ok_or("burgle needs a lead ID")?, "opportunity")?;
                let mut params = &words[2..];
                let press = match params.last().copied() {
                    Some("press") => {
                        params = &params[..params.len() - 1];
                        true
                    }
                    Some("abort") => {
                        params = &params[..params.len() - 1];
                        false
                    }
                    _ => false,
                };
                if params.len() > 2 {
                    return Err("burgle <lead-id> [entry-character-id] [coordinator-character-id] [abort|press]".to_owned());
                }
                let entry_specialist =
                    self.member_for_role(params.first().copied(), CapabilityKind::Burglary, false)?;
                let coordinator =
                    self.member_for_role(params.get(1).copied(), CapabilityKind::Management, true)?;
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
                let contingencies = if press {
                    vec![OperationContingency::RequestDecisionOnPoliceArrival]
                } else {
                    vec![
                        OperationContingency::AbortOnPoliceArrivalBeforeEntry,
                        OperationContingency::RequestDecisionOnPoliceArrival,
                    ]
                };
                let operation = validate_authorize_operation(
                    self.registry,
                    &self.state,
                    OperationDraft {
                        title: format!("Burglary at {name}"),
                        kind: OperationKind::Burglary,
                        responsible_organization: self.player,
                        leader: coordinator,
                        objective: OperationObjective::AcquireProperty {
                            target: EntityRef::Business(target),
                        },
                        approach: OperationApproach::Covert,
                        roles: BTreeMap::from([
                            (RoleKind::Coordinator, coordinator),
                            (RoleKind::EntrySpecialist, entry_specialist),
                        ]),
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
                    "Burglary {} authorized with {} information item(s); lead {} converted. Contingency: {:?}.",
                    operation.raw(),
                    information_count,
                    lead_id.raw(),
                    if press { "press" } else { "abort" }
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
            "contact" => {
                let contact = self
                    .state
                    .contacts()
                    .contacts_for_sponsor(self.player)
                    .find(|contact| {
                        contact.kind() == ContactKind::Police
                            && contact.status() == ContactStatus::Active
                    })
                    .ok_or("your organization has no active police contact")?
                    .id();
                let sources = find_pending_disclosure_sources(&self.state, contact);
                match sources.first().copied() {
                    None => out.push_str("Your police contact has nothing new to disclose."),
                    Some(source) => {
                        let disclosure = validate_contact_disclosure(&self.state, contact, source)
                            .map_err(|e| e.to_string())?
                            .commit(&mut self.state)
                            .map_err(|e| e.to_string())?;
                        let information = self
                            .state
                            .contacts()
                            .get_disclosure(disclosure)
                            .ok_or("disclosure record missing")?
                            .disclosed_information();
                        let fact = self
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
            "advance" | "wait" => {
                let span = parse_advance(words.get(1).ok_or("advance needs a duration")?)?;
                if self
                    .state
                    .decisions()
                    .pending_for_recipient(self.player)
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
                        .state
                        .reports()
                        .reports_for(self.player)
                        .last()
                        .map(|r| r.id());
                    let tick =
                        run_tick(self.registry, &mut self.state).map_err(|e| e.to_string())?;
                    advanced += 1;
                    let reports: Vec<_> = self
                        .state
                        .reports()
                        .reports_for_after(self.player, previous)
                        .collect();
                    new_reports += reports.len();
                    let urgent = reports.iter().any(|report| {
                        report.entries().iter().any(|entry| {
                            entry.attention.can_auto_pause()
                                && self
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
                            self.state
                                .operations()
                                .get_operation(*id)
                                .is_some_and(|op| op.responsible_organization() == self.player)
                        });
                    if self
                        .state
                        .decisions()
                        .pending_for_recipient(self.player)
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
                    time_label(self.state.now().as_minutes())
                )
                .unwrap();
                if let Some(reason) = reason {
                    writeln!(out, "Paused: {reason}.").unwrap();
                }
                for report in self
                    .state
                    .reports()
                    .reports_for(self.player)
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
