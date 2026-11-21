//! Player treasury policy, account management, and cash movement commands.

use super::*;
impl Session<'_> {
    pub(super) fn execute_treasury_commands(&mut self, words: &[&str]) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        match words[0] {
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
                            " (clearing only)".to_owned()
                        } else {
                            format!(
                                " (spendable {} cents)",
                                account.available_payment_balance().cents()
                            )
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
            "fund-street" => {
                if words.len() != 4 {
                    return Err("fund-street <accounted-id> <street-id> <cents>".to_owned());
                }
                let source = parse_number(words[1], "account")?;
                let destination = parse_number(words[2], "account")?;
                let cents = words[3]
                    .parse::<i64>()
                    .ok()
                    .filter(|amount| *amount > 0)
                    .ok_or("fund-street amount must be positive cents")?;
                let owner = FinancialOwner::Organization(self.player);
                let resolve = |raw| {
                    state
                        .finance()
                        .accounts_for(owner)
                        .find(|account| account.id().raw() == raw)
                        .map(|account| account.id())
                        .ok_or("account is not owned by your organization")
                };
                let transaction = validate_capitalize_street_cash(
                    state,
                    StreetCapitalizationDraft {
                        organization: self.player,
                        accounted_account: resolve(source)?,
                        street_account: resolve(destination)?,
                        amount: Money::from_cents(cents),
                    },
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Allocated {cents} cents of working cash in transaction {}.",
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
            _ => unreachable!("command routed to the wrong handler"),
        }
        Ok(out)
    }
}
