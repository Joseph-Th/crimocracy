//! Player business acquisition, operation, laundering, and profit withdrawal commands.

use super::*;
use crimocracy::intelligence::InformationTopic;
use crimocracy::world::BusinessOwner;

impl Session<'_> {
    fn spendable_street_cash(&self) -> i128 {
        self.state
            .finance()
            .accounts_for(FinancialOwner::Organization(self.player))
            .filter(|account| account.kind() == AccountKind::StreetCash)
            .map(|account| i128::from(account.available_payment_balance().cents()))
            .sum()
    }

    fn settle_laundering(&mut self, business: BusinessId, cents: i64) -> Result<u32, String> {
        let owner = FinancialOwner::Organization(self.player);
        let accounted_account = self
            .state
            .finance()
            .accounts_for(owner)
            .find(|account| account.kind() == AccountKind::AccountedFunds)
            .ok_or("no accounted-funds account")?
            .id();
        let street_accounts = self
            .state
            .finance()
            .accounts_for(owner)
            .filter(|account| account.kind() == AccountKind::StreetCash)
            .map(|account| account.id())
            .collect();
        validate_launder_funds(
            self.registry,
            &self.state,
            LaunderingDraft {
                organization: self.player,
                street_accounts,
                business,
                accounted_account,
                amount: Money::from_cents(cents),
            },
        )
        .map_err(|error| error.to_string())?
        .commit(&mut self.state)
        .map(|transaction| transaction.raw())
        .map_err(|error| error.to_string())
    }

    pub(super) fn execute_business_commands(&mut self, words: &[&str]) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        match words[0] {
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
                let validated = if words[0] == "offer" {
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
            "business-outlook" => {
                let raw = parse_number(words[1], "business")?;
                let business = self
                    .known_business(raw)
                    .ok_or("property is not in the public directory")?;
                let appraisal = crimocracy::economy::business_acquisition::appraise_business(
                    self.registry,
                    state,
                    business,
                )
                .map_err(|error| error.to_string())?;
                writeln!(out, "Public appraisal for business-{raw} (all directory competitors operating normally):").unwrap();
                let owned = state.world().get_business(business).is_some_and(|record| {
                    record.owner() == BusinessOwner::Organization(self.player)
                });
                if owned {
                    out.push_str("  Purchase status: already owned by your organization.\n");
                } else if acquisition_offer(self.registry, state, self.player, business).is_some() {
                    writeln!(out, "  Purchase status: independently listed; use 'buy {raw}' (funds checked at purchase).").unwrap();
                } else {
                    writeln!(out, "  Purchase status: not available for direct purchase; 'offer {raw} <cents>' may work for a privately held property, but seller consent is not public.").unwrap();
                }
                writeln!(
                    out,
                    "  Base property price (before portfolio premium): {} cents",
                    appraisal.standard_price.cents()
                )
                .unwrap();
                writeln!(
                    out,
                    "  Forecast net: {:+} cents per {} minutes",
                    appraisal.expected_cycle_net.cents(),
                    appraisal.cycle_minutes
                )
                .unwrap();
                if let Some((quote, _)) =
                    acquisition_offer(self.registry, state, self.player, business)
                {
                    writeln!(out, "  Your current listed purchase quote: {} cents, including portfolio consolidation premium.", quote.cents()).unwrap();
                }
                if owned {
                    out.push_str(
                        "  Negotiated offer: not applicable to a property you already own.\n",
                    );
                } else {
                    writeln!(
                        out,
                        "  Negotiated reference offer: {} cents before your consolidation premium; seller consent is not guaranteed.",
                        appraisal.reference_offer_cents
                    )
                    .unwrap();
                }
                out.push_str("  Standalone normal-operations estimate, not incremental profit to your existing portfolio. Another nearby business can lose sales after acquisition. This estimate excludes actual books, disruptions, variance, owner obligations, and private seller commitments.");
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
                    out.push_str("No properties currently available for direct purchase. Use directory to identify properties and offer <business-id> <cents> to negotiate with a private owner.");
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
                let raw = parse_number(words[1], "business")?;
                let business = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|business| business.id().raw() == raw)
                    .ok_or("not an owned business")?
                    .id();
                let cents = if words[2] == "max" {
                    let capacity = remaining_laundering_capacity(
                        self.registry,
                        &self.state,
                        self.player,
                        business,
                    )
                    .ok_or("business has no active cash-intensive laundering channel")?
                    .cents();
                    let street_cash = self.spendable_street_cash();
                    let amount = i128::from(capacity).min(street_cash) as i64;
                    if amount <= 0 {
                        return Err(format!(
                            "no available laundering amount: front room {capacity} cents, total spendable street cash {street_cash} cents"
                        ));
                    }
                    amount
                } else {
                    words[2]
                        .parse::<i64>()
                        .ok()
                        .filter(|cents| *cents > 0)
                        .ok_or("launder needs a positive amount in cents or 'max'")?
                };
                let transaction = self.settle_laundering(business, cents)?;
                writeln!(out, "Laundered {cents} cents at business-{raw} as transaction-{transaction}. Fees and accounted proceeds are recorded in the ledger.").unwrap();
            }
            "launder-all" => {
                let reserve = words[1]
                    .parse::<i64>()
                    .ok()
                    .filter(|amount| *amount >= 0)
                    .ok_or("launder-all needs a nonnegative street-cash reserve in cents")?;
                // One explicit player order, not a standing autonomous policy. Every storefront
                // is independently settled through the finance owner at this same minute.
                let fronts = state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .filter(|business| {
                        remaining_laundering_capacity(
                            self.registry,
                            state,
                            self.player,
                            business.id(),
                        )
                        .is_some_and(|amount| amount.cents() > 0)
                    })
                    .map(|business| business.id())
                    .collect::<Vec<_>>();
                let mut converted = 0_i128;
                let mut settled = 0_usize;
                let mut skipped = 0_usize;
                for business in fronts {
                    let total_spendable = self.spendable_street_cash();
                    let budget = (total_spendable - i128::from(reserve))
                        .max(0)
                        .min(i128::from(i64::MAX)) as i64;
                    if budget == 0 {
                        break;
                    }
                    let capacity = remaining_laundering_capacity(
                        self.registry,
                        &self.state,
                        self.player,
                        business,
                    )
                    .map_or(0, |room| room.cents());
                    let amount = budget.min(capacity);
                    if amount <= 0 {
                        continue;
                    }
                    match self.settle_laundering(business, amount) {
                        Ok(transaction) => {
                            converted += i128::from(amount);
                            settled += 1;
                            writeln!(
                                out,
                                "  business-{}: {} street cents through transaction-{}",
                                business.raw(),
                                amount,
                                transaction
                            )
                            .unwrap();
                        }
                        Err(error) => {
                            skipped += 1;
                            writeln!(out, "  business-{}: skipped, {}", business.raw(), error)
                                .unwrap();
                        }
                    }
                }
                if settled == 0 && skipped == 0 {
                    out.push_str("  No eligible front capacity or street cash above the requested reserve.\n");
                }
                writeln!(
                    out,
                    "Laundering order complete: {converted} street cents converted at {settled} front(s), {skipped} failed; requested street-cash reserve {reserve} cents. Fees reduce accounted proceeds. Review 'accounts' and 'ledger'."
                )
                .unwrap();
            }
            "businesses" => {
                let availability =
                    crimocracy::economy::business_economy_system::organization_profit_availability(
                        state,
                        self.player,
                    )
                    .map_err(|error| error.to_string())?;
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
                        let withdrawable = availability.gross_by_business[&business.id()].cents();
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
                let outcome = validate_sweep_business_profits(
                    &self.state,
                    BusinessProfitSweepDraft {
                        organization: self.player,
                        business,
                        amount: Money::from_cents(cents),
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "Withdrew {} cents of actual earned business surplus; received {} cents, administration {} cents (transaction-{}).",
                    outcome.gross.cents(), outcome.received.cents(), outcome.administration.cents(), outcome.transaction.raw()
                )
                .unwrap();
            }
            _ => unreachable!("command routed to the wrong handler"),
        }
        Ok(out)
    }
}
