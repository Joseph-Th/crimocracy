//! Acquiring an independently owned business as an organizational asset.
//!
//! The canonical purchase path: an organization buys a business outright at its authored
//! kind price, paid in full from accounted funds. Dirty street cash cannot become a
//! storefront — legitimate expansion is gated on laundering throughput, so the money
//! loop closes: illicit proceeds are laundered into accounted wealth, and accounted
//! wealth converts into earning capacity (and, for cash-intensive fronts, more
//! laundering capacity). Purchase consideration leaves the buyer's spendable control:
//! the acquired business's settlement account is the ledger counterparty representing
//! payment to the outside seller, never a capitalization of the asset's operating cash.
//!
//! Commit composes three canonical paths in one validated step: world ownership
//! transfer, business-economy establishment when the target has never operated (or
//! restart under the new owner when books already exist), and a balanced ledger
//! payment. Every fallible condition is validated before any mutation; the records
//! constructed during commit reference freshly reserved accounts whose kinds and owners
//! are guaranteed by construction.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessId, FinancialAccountId, IdExhaustionError, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::economy::{BusinessEconomyDraft, business_economy_system};
use crate::finance::finance_system::{
    FinanceError, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::helpers::format_money_cents;
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::registry::Registry;
use crate::reports::report_system::{ReportError, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::BusinessOwner;
use crate::world::world_system::WorldError;
use crate::world::world_system::validate_transfer_business_ownership;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessAcquisitionError {
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business {business} is not independently owned; it is owned by {owner:?}")]
    NotIndependentlyOwned {
        business: BusinessId,
        owner: BusinessOwner,
    },
    #[error("funding account {0} does not exist")]
    MissingFundingAccount(FinancialAccountId),
    #[error("funding account {account} is not owned by organization {organization}")]
    FundingAccountOwnerMismatch {
        account: FinancialAccountId,
        organization: OrganizationId,
    },
    #[error(
        "funding account {0} must hold accounted funds: dirty street cash cannot buy legitimacy"
    )]
    InvalidFundingAccountKind(FinancialAccountId),
    #[error(
        "accounted funds {available_cents} cannot cover the {price_cents}-cent acquisition price"
    )]
    InsufficientFunds {
        available_cents: i64,
        price_cents: i64,
    },
    #[error("business acquisition must name at least one accounted-funds account")]
    NoFundingAccounts,
    #[error(transparent)]
    World(#[from] WorldError),
    #[error(transparent)]
    Economy(#[from] business_economy_system::BusinessEconomyError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Debug)]
pub struct BusinessAcquisitionDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    /// Organization-owned accounted-funds accounts permitted to fund the purchase. The ledger
    /// records the exact deterministic debit allocation, so the acquisition itself stores no
    /// duplicate payment-source fact.
    pub funding_accounts: BTreeSet<FinancialAccountId>,
}

/// What one committed acquisition actually did, quoted from production state rather than
/// recomputed by callers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquiredBusiness {
    pub business: BusinessId,
    pub price: Money,
    /// True when commit also opened the acquired business's first operating economy.
    pub established_economy: bool,
}

#[derive(Debug)]
pub struct ValidatedBusinessAcquisition {
    /// Authored cycle duration for the economy commit opens; captured at validation time.
    economy_cycle_duration: crate::core::time::SimDuration,
    funding_accounts: BTreeSet<FinancialAccountId>,
    price: Money,
    business: BusinessId,
    business_name: String,
    organization: OrganizationId,
}

impl ValidatedBusinessAcquisition {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<AcquiredBusiness, BusinessAcquisitionError> {
        // ---- Phase 1: re-validate every fallible condition against live state. --------
        // A validated token can be held across intervening mutations, so solvency,
        // ownership, and hosting conflicts are re-checked before anything mutates.
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessAcquisitionError::MissingBusiness(self.business))?;
        if business_record.owner() != BusinessOwner::Independent {
            return Err(BusinessAcquisitionError::NotIndependentlyOwned {
                business: self.business,
                owner: business_record.owner(),
            });
        }
        validate_funding_accounts(state, self.organization, &self.funding_accounts, self.price)?;
        // Hosting conflicts (active enterprise venues/supporters) reject through the same
        // canonical read the ownership transfer enforces. Keep the validated token so commit
        // does not create a second validation path later in the composite operation.
        let transfer = validate_transfer_business_ownership(
            state,
            self.business,
            BusinessOwner::Organization(self.organization),
        )?;
        let existing_economy = state.economy.get_business_economy(self.business);
        // The establishment/restart decision follows live state, not the validation-time
        // snapshot: a token held across someone else's establishment adopts those books instead
        // of colliding. Every existing economy restarts from the title-transfer instant so the
        // buyer cannot inherit a nearly due seller cycle or the seller's chronic-loss streak.
        let establish_now = existing_economy.is_none();

        // ---- Phase 2: plan fresh books and pre-validate every remaining leg. ---------
        // Fresh account IDs are predicted without consuming allocator state. The payment token
        // owns that plan and opens the accounts only when the whole acquisition is ready to
        // commit, so no rollback path exists and every rejection is truly state-neutral.
        let mut fresh_account_count = 0_u32;
        let mut establishment = None;
        let payment = if let Some(economy) = existing_economy {
            validate_record_transaction(
                state,
                acquisition_payment_draft(
                    state,
                    &self.funding_accounts,
                    economy.settlement_account(),
                    self.price,
                    &self.business_name,
                ),
            )?
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
            fresh_account_count = u32::try_from(openings.len())
                .expect("validated acquisition account count must fit u32");
            validate_record_transaction_with_openings(
                state,
                openings,
                acquisition_payment_draft(
                    state,
                    &self.funding_accounts,
                    settlement_account,
                    self.price,
                    &self.business_name,
                ),
            )?
        };
        let restart = if existing_economy.is_some() {
            Some(business_economy_system::validate_acquisition_restart(
                state,
                self.business,
                self.economy_cycle_duration,
            )?)
        } else {
            None
        };
        let announcement = validate_record_report(
            state,
            ReportDraft {
                recipient: self.organization,
                kind: ReportKind::Financial,
                title: "Business acquisition".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: format!(
                        "The organization purchased {} outright for {}, paid in full from accounted funds.",
                        self.business_name,
                        format_money_cents(self.price.cents())
                    ),
                    sources: Vec::new(),
                    entities: std::collections::BTreeSet::from([EntityRef::Business(
                        self.business,
                    )]),
                    decision: None,
                }],
            },
        )?;

        // ---- Phase 3: reserve the complete persistent-ID budget, then commit. --------
        // No mutation occurs before this aggregate preflight. From this point onward each
        // fallible canonical commit is guarded by dependencies that this function alone controls.
        state.ids.reserve_many(&[
            (IdKind::BusinessOwnershipChange, 1),
            (IdKind::FinancialAccount, fresh_account_count),
            (IdKind::LedgerTransaction, 1),
            (IdKind::Report, 1),
        ])?;
        transfer
            .commit(state)
            .expect("a just-revalidated ownership transfer must commit atomically");
        payment
            .commit(state)
            .expect("a payment pre-validated against untouched account versions must commit");
        if let Some(establishment) = establishment {
            establishment.commit_after_preflight(state);
        } else if let Some(restart) = restart {
            restart
                .commit(state)
                .expect("prevalidated acquisition restart must remain current during commit");
        }
        announcement
            .commit(state)
            .expect("a validated acquisition report about live entities must commit");
        Ok(AcquiredBusiness {
            business: self.business,
            price: self.price,
            established_economy: establish_now,
        })
    }
}

fn acquisition_payment_draft(
    state: &AppState,
    funding_accounts: &BTreeSet<FinancialAccountId>,
    seller_settlement_account: FinancialAccountId,
    price: Money,
    business_name: &str,
) -> LedgerTransactionDraft {
    let mut postings = Vec::with_capacity(funding_accounts.len() + 1);
    let mut remaining = price;
    for account in funding_accounts {
        if remaining == Money::ZERO {
            break;
        }
        let spendable = state
            .finance
            .get_account(*account)
            .expect("validated acquisition funding account must exist")
            .spendable_balance();
        if spendable == Money::ZERO {
            continue;
        }
        let debit = spendable.min(remaining);
        postings.push(LedgerPosting {
            account: *account,
            amount: debit
                .checked_neg()
                .expect("positive acquisition debit must negate"),
        });
        remaining = remaining
            .checked_sub(debit)
            .expect("acquisition debit cannot exceed remaining price");
    }
    debug_assert_eq!(remaining, Money::ZERO);
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

fn validate_funding_accounts(
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
        available_cents = (available_cents + i128::from(funding.spendable_balance().cents()))
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

pub fn validate_acquire_business(
    registry: &Registry,
    state: &AppState,
    draft: BusinessAcquisitionDraft,
) -> Result<ValidatedBusinessAcquisition, BusinessAcquisitionError> {
    if state.world.get_organization(draft.organization).is_none() {
        return Err(BusinessAcquisitionError::MissingOrganization(
            draft.organization,
        ));
    }
    let business_record = state
        .world
        .get_business(draft.business)
        .ok_or(BusinessAcquisitionError::MissingBusiness(draft.business))?;
    if business_record.owner() != BusinessOwner::Independent {
        return Err(BusinessAcquisitionError::NotIndependentlyOwned {
            business: draft.business,
            owner: business_record.owner(),
        });
    }
    let price = registry
        .get_business(business_record.kind())
        .economics()
        .acquisition_cost();
    validate_funding_accounts(state, draft.organization, &draft.funding_accounts, price)?;
    // Ownership conflicts (active enterprise hosts/supports) reject through the canonical
    // transfer path; unchanged ownership is impossible because an Independent owner never
    // equals an Organization owner. Commit re-runs this read against live state, so the
    // token itself carries no transfer snapshot.
    validate_transfer_business_ownership(
        state,
        draft.business,
        BusinessOwner::Organization(draft.organization),
    )?;
    // Existing books do not block a purchase. Acquisition changes the responsible owner, so
    // commit restarts the cycle after ownership transfer: suspended books reopen, active books
    // re-anchor their next settlement, and neither state inherits the seller's loss streak.
    Ok(ValidatedBusinessAcquisition {
        economy_cycle_duration: registry
            .get_business(business_record.kind())
            .economics()
            .cycle(),
        funding_accounts: draft.funding_accounts,
        price,
        business: draft.business,
        business_name: business_record.name().to_owned(),
        organization: draft.organization,
    })
}

#[cfg(test)]
mod tests;
