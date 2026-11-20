//! Player business acquisition, operation, laundering, and profit withdrawal commands.

use super::*;
use crimocracy::intelligence::InformationTopic;
use crimocracy::world::BusinessOwner;

impl Session<'_> {
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
            _ => unreachable!("command routed to the wrong handler"),
        }
        Ok(out)
    }
}
