//! Player-only finance history. Every row is selected via the finance owner's
//! account indexes, and counterparties' posting details remain private.

use super::*;
use crimocracy::finance::LedgerTransactionRecord;

impl Session<'_> {
    fn own_postings<'a>(
        &'a self,
        transaction: &'a LedgerTransactionRecord,
    ) -> impl Iterator<Item = (AccountKind, &'a LedgerPosting)> + 'a {
        let owner = FinancialOwner::Organization(self.player);
        transaction.postings().iter().filter_map(move |posting| {
            self.state
                .finance()
                .get_account(posting.account)
                .filter(|account| account.owner() == owner)
                .map(|account| (account.kind(), posting))
        })
    }

    pub(super) fn execute_finance_history(&self, words: &[&str]) -> Result<String, String> {
        let owner = FinancialOwner::Organization(self.player);
        let mut out = String::new();
        match words[0] {
            "ledger" => {
                let count = words
                    .get(1)
                    .map_or(Ok(10), |value| {
                        value
                            .parse::<usize>()
                            .ok()
                            .filter(|count| *count > 0)
                            .ok_or("ledger count must be a positive integer")
                    })?
                    .min(40);
                let transactions = self
                    .state
                    .finance()
                    .recent_transactions_for_owner(owner, count);
                if transactions.is_empty() {
                    return Ok("No transactions have touched your organization's books.".to_owned());
                }
                for transaction in transactions {
                    let (debits, credits) = self.own_postings(transaction).fold(
                        (0_i128, 0_i128),
                        |(debits, credits), (_, posting)| {
                            let cents = i128::from(posting.amount.cents());
                            if cents < 0 {
                                (debits - cents, credits)
                            } else {
                                (debits, credits + cents)
                            }
                        },
                    );
                    writeln!(
                        out,
                        "transaction-{} | {} | debits {} cents, credits {} cents | {}",
                        transaction.id().raw(),
                        time_label(transaction.occurred_at().as_minutes()),
                        debits,
                        credits,
                        transaction.memo(),
                    )
                    .unwrap();
                }
            }
            "transaction" => {
                let raw = parse_number(words[1], "transaction")?;
                // An arbitrary valid ID is not authority to inspect rivals'
                // settlements. Rejection also hides whether it exists at all.
                let transaction = self
                    .state
                    .finance()
                    .transaction_by_number_for_owner(owner, raw)
                    .ok_or("no transaction with that ID in your organization's books")?;
                writeln!(
                    out,
                    "transaction-{} | {} | {}",
                    transaction.id().raw(),
                    time_label(transaction.occurred_at().as_minutes()),
                    transaction.memo()
                )
                .unwrap();
                let mut net = 0_i128;
                for (kind, posting) in self.own_postings(transaction) {
                    let cents = i128::from(posting.amount.cents());
                    net += cents;
                    writeln!(
                        out,
                        "  account-{} {:?}: {cents:+} cents",
                        posting.account.raw(),
                        kind
                    )
                    .unwrap();
                }
                writeln!(out,
                    "Net change across your accounts: {net:+} cents. External counterparties' accounts are not displayed."
                ).unwrap();
            }
            _ => return Err("unknown finance-history command".to_owned()),
        }
        Ok(out)
    }
}
