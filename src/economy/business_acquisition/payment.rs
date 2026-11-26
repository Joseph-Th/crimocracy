//! Purchase funding and settlement-account planning. The acquisition coordinator owns the
//! combined ID reservation and commits title, ledger and economy changes together.

use super::{BusinessAcquisitionError, ValidatedBusinessAcquisition};
use crate::core::id::{FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::economy::business_economy_system::{
    self, ValidatedComposedBusinessEconomyEstablishment,
};
use crate::economy::{BusinessEconomyDraft, BusinessEconomyRecord};
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use std::collections::BTreeSet;

impl ValidatedBusinessAcquisition {
    /// Predict fresh account IDs without consuming allocator state. The ledger token owns
    /// openings, so neither an unfunded purchase nor another failed leg leaves accounts behind.
    pub(super) fn plan_payment(
        &self,
        state: &AppState,
        existing_economy: Option<&BusinessEconomyRecord>,
    ) -> Result<
        (
            ValidatedLedgerTransaction,
            Option<ValidatedComposedBusinessEconomyEstablishment>,
        ),
        BusinessAcquisitionError,
    > {
        let draft = |destination| {
            acquisition_payment_draft(
                state,
                &self.funding_accounts,
                destination,
                self.price,
                &self.business_name,
            )
        };
        let mut establishment = None;
        let payment = if let Some(seller) = self.negotiated_seller {
            // The seller actually receives spendable purchase consideration. Select a viable
            // account deterministically or open one atomically with the transfer.
            let destination = state
                .finance
                .accounts_for(seller)
                .find(|account| {
                    account.kind() == AccountKind::AccountedFunds
                        && account.can_receive_liquid_payment(self.price)
                })
                .map(|account| account.id());
            if let Some(destination) = destination {
                validate_record_transaction(state, draft(destination))?
            } else {
                let openings = validate_open_accounts(
                    state,
                    vec![FinancialAccountDraft {
                        owner: seller,
                        kind: AccountKind::AccountedFunds,
                    }],
                )?;
                let destination = openings
                    .account_id(0)
                    .expect("seller account opening must expose its ID");
                validate_record_transaction_with_openings(state, openings, draft(destination))?
            }
        } else if let Some(economy) = existing_economy {
            validate_record_transaction(state, draft(economy.settlement_account()))?
        } else {
            let openings = validate_open_accounts(
                state,
                vec![
                    FinancialAccountDraft {
                        owner: FinancialOwner::Business(self.business),
                        kind: AccountKind::LegitimateOperating,
                    },
                    FinancialAccountDraft {
                        owner: FinancialOwner::Business(self.business),
                        kind: AccountKind::Settlement,
                    },
                ],
            )?;
            let operating_account = openings
                .account_id(0)
                .expect("two-account acquisition plan must expose its operating account");
            let settlement_account = openings
                .account_id(1)
                .expect("two-account acquisition plan must expose its settlement account");
            establishment = Some(
                business_economy_system::validate_composed_business_economy_establishment(
                    state,
                    BusinessEconomyDraft {
                        business: self.business,
                        operating_account,
                        settlement_account,
                    },
                    self.economy_cycle_duration,
                    &openings,
                )?,
            );
            validate_record_transaction_with_openings(state, openings, draft(settlement_account))?
        };
        Ok((payment, establishment))
    }
}

fn acquisition_payment_draft(
    state: &AppState,
    funding_accounts: &BTreeSet<FinancialAccountId>,
    seller_settlement_account: FinancialAccountId,
    price: Money,
    business_name: &str,
) -> LedgerTransactionDraft {
    let mut postings = crate::finance::helpers::payment_debits(
        funding_accounts.iter().map(|account| {
            state
                .finance
                .get_account(*account)
                .expect("validated acquisition funding account must exist")
        }),
        price,
    )
    .expect("acquisition funding validation proved the exact price is payable");
    postings.push(LedgerPosting {
        account: seller_settlement_account,
        amount: price,
    });
    LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!("Business acquisition of {business_name}"),
        postings,
        authorization: None,
    }
}

pub(super) fn validate_funding_accounts(
    state: &AppState,
    organization: OrganizationId,
    funding_accounts: &BTreeSet<FinancialAccountId>,
    price: Money,
) -> Result<(), BusinessAcquisitionError> {
    if funding_accounts.is_empty() {
        return Err(BusinessAcquisitionError::NoFundingAccounts);
    }
    let mut available_cents = 0_i128;
    for account in funding_accounts {
        let funding = state
            .finance
            .get_account(*account)
            .ok_or(BusinessAcquisitionError::MissingFundingAccount(*account))?;
        if funding.owner() != FinancialOwner::Organization(organization) {
            return Err(BusinessAcquisitionError::FundingAccountOwnerMismatch {
                account: *account,
                organization,
            });
        }
        if funding.kind() != AccountKind::AccountedFunds {
            return Err(BusinessAcquisitionError::InvalidFundingAccountKind(
                *account,
            ));
        }
        available_cents = (available_cents
            + i128::from(funding.available_payment_balance().cents()))
        .min(i128::from(price.cents()));
    }
    if available_cents < i128::from(price.cents()) {
        return Err(BusinessAcquisitionError::InsufficientFunds {
            available_cents: i64::try_from(available_cents)
                .expect("available acquisition funding is bounded by price"),
            price_cents: price.cents(),
        });
    }
    Ok(())
}
