//! Player command dispatch through canonical production owners.

use super::planning::PoliceMode;
use super::*;
use crimocracy::decisions::DecisionContext;
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
            | "political"
            | "campaign"
            | "election"
            | "population"
            | "freelancers"
            | "civics"
            | "civic-person"
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
            "political" => out = self.political_readiness(words[1])?,
            "campaign" => out = self.campaign_readiness()?,
            "election" => out = self.election_brief()?,
            "civics" => out = self.civic_directory(words.get(1).copied())?,
            "civic-person" => out = self.civic_character(words[1])?,
            "population" => out = self.population_dossier(),
            "freelancers" => out = self.freelancer_directory(),
            "contract" => out = self.execute_freelance_contract(&words)?,
            "buy-influence" | "hire-specialist" | "fire-specialist" | "nominate-mayor"
            | "campaign-event" => {
                out = self.execute_campaign_order(&words)?;
            }
            "victory" => out = self.victory_readiness()?,
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
            "districts" | "turf" | "negotiate" | "contest" | "preview-contest" => {
                out = self.execute_territory_commands(&words)?;
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
                    if let Some(contract) = state.world().active_freelance_contract(person.id()) {
                        let expiry = contract.expires_at();
                        if expiry <= state.now() {
                            writeln!(
                                out,
                                "  Freelance contract expired {expiry}; release pending"
                            )
                            .unwrap();
                        } else {
                            writeln!(out, "  Freelance contract expires {expiry}").unwrap();
                        }
                    }
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
                    if let Some(load) =
                        crimocracy::enterprises::enterprise_execution::manager_workload(
                            state,
                            person.id(),
                        )
                        && load.required > 0
                    {
                        writeln!(
                            out,
                            "  Command: {} / {} duties | {} directly managed rackets, {} associates, {} operating reporting lines ({} supervision duties), {} districts | {}",
                            load.required,
                            load.available,
                            load.rackets,
                            load.associates,
                            load.command_lines,
                            load.reporting_duties,
                            load.districts,
                            if load.required > load.available {
                                "OVERSTRETCHED"
                            } else {
                                "within capacity"
                            }
                        ).unwrap();
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
            "prospects" | "recruit" => {
                out = self.execute_recruitment(&words)?;
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
                let objective = OperationObjective::GatherInformation {
                    target: EntityRef::Business(business),
                };
                let roles = self.select_operation_roles(
                    OperationKind::Surveillance,
                    &objective,
                    &explicit,
                    at,
                    &[],
                )?;
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
                        objective,
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
                    .known_opportunity(raw)
                    .filter(|opp| opp.status() == OpportunityStatus::Open)
                    .ok_or("lead is not open or not known")?;
                if lead.context().operation_kind() != OperationKind::Burglary {
                    return Err("lead is not for burglary".to_owned());
                }
                let mut businesses =
                    lead.context()
                        .targets()
                        .iter()
                        .filter_map(|target| match target {
                            EntityRef::Business(id) => Some(*id),
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
                        });
                let target = businesses
                    .next()
                    .ok_or("burglary requires one business target")?;
                if businesses.next().is_some() {
                    return Err("burglary lead has multiple business targets; this shortcut requires exactly one".to_owned());
                }
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
                let mut explicit = BTreeMap::new();
                for (raw, role) in params
                    .iter()
                    .zip([RoleKind::EntrySpecialist, RoleKind::Coordinator])
                {
                    explicit.insert(role, self.requested_member(raw)?);
                }
                let objective = OperationObjective::AcquireProperty {
                    target: EntityRef::Business(target),
                };
                let roles = self.select_operation_roles(
                    OperationKind::Burglary,
                    &objective,
                    &explicit,
                    at,
                    &[],
                )?;
                let coordinator = roles[&RoleKind::Coordinator];
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
                let operation = validate_authorize_from_opportunity(
                    self.registry,
                    &self.state,
                    lead_id,
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
                let lead = self.known_opportunity(raw).ok_or("unknown lead")?;
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
                let request = state
                    .decisions()
                    .pending_for_recipient(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("no pending decision for your organization")?;
                let decision = request.id();
                let protection = match request.context() {
                    DecisionContext::ProtectionDemand(context) => Some(context),
                    DecisionContext::RecruitmentApproval(_)
                    | DecisionContext::OperationPoliceArrival { .. } => None,
                };
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
                if let Some(context) = protection {
                    let gang = self
                        .state
                        .world()
                        .get_organization(context.attacker())
                        .expect("protection demand references an existing gang");
                    let district = self
                        .state
                        .world()
                        .get_neighborhood(context.district())
                        .expect("protection demand references an existing ward");
                    match response {
                        DecisionResponse::Approve => {
                            writeln!(out,
                                "  Paid {} cents to {} for 30 days' protection from that gang's new attacks in {}.",
                                context.amount().cents(), gang.name(), district.name(),
                            ).unwrap();
                        }
                        DecisionResponse::Reject => {
                            writeln!(out,
                                "  Refused {}'s demand in {}. Their scouts and strike crews remain a threat.",
                                gang.name(), district.name(),
                            ).unwrap();
                        }
                        DecisionResponse::Continue | DecisionResponse::Abort => {
                            unreachable!("protection decisions support approve or reject only")
                        }
                    }
                }
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
            "advance" => {
                let uninterrupted = match words.get(2).copied() {
                    None => false,
                    Some("uninterrupted") => true,
                    _ => return Err("advance <duration> [uninterrupted]".to_owned()),
                };
                out = self.execute_advance(words[1], uninterrupted)?;
            }
            "save" => out = self.save(words.get(1).ok_or("save needs a slot name")?)?,
            _ => return Err(format!("unknown command '{verb}'; type 'help'")),
        }
        Ok((out, false))
    }
}
