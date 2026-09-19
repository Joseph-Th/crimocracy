//! Owner withdrawals from legitimate business tills into organization accounted funds.

use crate::core::id::{BusinessId, FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_record_business_transaction,
};
use crate::finance::{
    AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money,
    helpers::format_money_cents,
};
use thiserror::Error;

/// An owner's draw on a business it owns: legitimate till cash becomes spendable organization
/// wealth. Laundering fees are excluded because they settle into the business's non-liquid
/// clearing account and are a real cost, not earned owner surplus.
#[derive(Clone, Debug)]
pub struct BusinessProfitSweepDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    pub destination: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessProfitSweepError {
    #[error("profit sweep amount must be positive")]
    NonPositiveAmount,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(BusinessId),
    #[error("business {0} has no operating economy to sweep till cash from")]
    MissingBusinessEconomy(BusinessId),
    #[error(
        "business {business} till account {account} is missing or not legitimate operating funds"
    )]
    CorruptTillAccount {
        business: BusinessId,
        account: FinancialAccountId,
    },
    #[error("destination account {0} does not exist")]
    MissingDestinationAccount(FinancialAccountId),
    #[error("destination account {account} is not owned by organization {organization}")]
    DestinationOwnerMismatch {
        account: FinancialAccountId,
        organization: OrganizationId,
    },
    #[error(
        "destination account {0} must hold accounted funds: till cash enters the books as legitimate wealth"
    )]
    InvalidDestinationKind(FinancialAccountId),
    #[error(
        "business {business} till holds {available_cents} cents and cannot sweep {requested_cents}"
    )]
    InsufficientTillCash {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error(
        "business {business} retained-capital basis belongs to ownership version {basis_version}, but current ownership is version {current_version}"
    )]
    StaleCapitalBasis {
        business: BusinessId,
        basis_version: u32,
        current_version: u32,
    },
    #[error(
        "business {business} has only {available_cents} cents of earned surplus above retained capital and cannot sweep {requested_cents}"
    )]
    InsufficientEarnedSurplus {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error("business {business} ownership changed after sweep validation")]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {business} economy changed after sweep validation")]
    StaleEconomy {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Finance(#[from] FinanceError),
}

pub struct ValidatedBusinessProfitSweep {
    transaction: ValidatedLedgerTransaction,
    business: BusinessId,
    organization: OrganizationId,
    expected_business_version: u32,
    expected_economy_version: u32,
}

impl std::fmt::Debug for ValidatedBusinessProfitSweep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedBusinessProfitSweep")
            .field("business", &self.business)
            .field("organization", &self.organization)
            .field("expected_business_version", &self.expected_business_version)
            .field("expected_economy_version", &self.expected_economy_version)
            .finish_non_exhaustive()
    }
}

impl ValidatedBusinessProfitSweep {
    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessProfitSweepError> {
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessProfitSweepError::MissingBusiness(self.business))?;
        if business_record.version() != self.expected_business_version {
            return Err(BusinessProfitSweepError::StaleBusiness {
                business: self.business,
                expected: self.expected_business_version,
                found: business_record.version(),
            });
        }
        if business_record.owner() != crate::world::BusinessOwner::Organization(self.organization) {
            return Err(BusinessProfitSweepError::ForeignBusiness(self.business));
        }
        let economy = state.economy.get_business_economy(self.business).ok_or(
            BusinessProfitSweepError::MissingBusinessEconomy(self.business),
        )?;
        if economy.version() != self.expected_economy_version {
            return Err(BusinessProfitSweepError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        self.transaction.commit(state)?;
        Ok(())
    }
}

pub fn validate_sweep_business_profits(
    state: &AppState,
    draft: BusinessProfitSweepDraft,
) -> Result<ValidatedBusinessProfitSweep, BusinessProfitSweepError> {
    if draft.amount <= Money::ZERO {
        return Err(BusinessProfitSweepError::NonPositiveAmount);
    }
    let context = resolve_business_profit_sweep_context(state, &draft)?;
    validate_business_profit_sweep_destination(state, &draft)?;
    validate_business_profit_sweep_capacity(
        &draft,
        context.operating_balance,
        context.capital_floor,
    )?;
    let transaction = validate_record_business_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Owner withdrawal of {} from {} till",
                format_money_cents(draft.amount.cents()),
                context.business_name,
            ),
            postings: vec![
                LedgerPosting {
                    account: context.operating_account,
                    amount: draft
                        .amount
                        .checked_neg()
                        .expect("positive sweep amount must negate"),
                },
                LedgerPosting {
                    account: draft.destination,
                    amount: draft.amount,
                },
            ],
            authorization: None,
        },
    )?;
    Ok(ValidatedBusinessProfitSweep {
        transaction,
        business: draft.business,
        organization: draft.organization,
        expected_business_version: context.business_version,
        expected_economy_version: context.economy_version,
    })
}

struct BusinessProfitSweepContext {
    operating_account: FinancialAccountId,
    operating_balance: Money,
    capital_floor: Money,
    business_name: String,
    business_version: u32,
    economy_version: u32,
}

fn resolve_business_profit_sweep_context(
    state: &AppState,
    draft: &BusinessProfitSweepDraft,
) -> Result<BusinessProfitSweepContext, BusinessProfitSweepError> {
    if state.world.get_organization(draft.organization).is_none() {
        return Err(BusinessProfitSweepError::MissingOrganization(
            draft.organization,
        ));
    }
    let business_record = state
        .world
        .get_business(draft.business)
        .ok_or(BusinessProfitSweepError::MissingBusiness(draft.business))?;
    if business_record.owner() != crate::world::BusinessOwner::Organization(draft.organization) {
        return Err(BusinessProfitSweepError::ForeignBusiness(draft.business));
    }
    let economy = state.economy.get_business_economy(draft.business).ok_or(
        BusinessProfitSweepError::MissingBusinessEconomy(draft.business),
    )?;
    let operating = state
        .finance
        .get_account(economy.operating_account())
        .ok_or(BusinessProfitSweepError::CorruptTillAccount {
            business: draft.business,
            account: economy.operating_account(),
        })?;
    if operating.owner() != FinancialOwner::Business(draft.business)
        || operating.kind() != AccountKind::LegitimateOperating
    {
        return Err(BusinessProfitSweepError::CorruptTillAccount {
            business: draft.business,
            account: economy.operating_account(),
        });
    }
    if economy.capital_floor_business_version() != business_record.version() {
        return Err(BusinessProfitSweepError::StaleCapitalBasis {
            business: draft.business,
            basis_version: economy.capital_floor_business_version(),
            current_version: business_record.version(),
        });
    }
    Ok(BusinessProfitSweepContext {
        operating_account: economy.operating_account(),
        operating_balance: operating.spendable_balance(),
        capital_floor: economy.operating_capital_floor(),
        business_name: business_record.name().to_owned(),
        business_version: business_record.version(),
        economy_version: economy.version(),
    })
}

fn validate_business_profit_sweep_destination(
    state: &AppState,
    draft: &BusinessProfitSweepDraft,
) -> Result<(), BusinessProfitSweepError> {
    let destination = state.finance.get_account(draft.destination).ok_or(
        BusinessProfitSweepError::MissingDestinationAccount(draft.destination),
    )?;
    if destination.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(BusinessProfitSweepError::DestinationOwnerMismatch {
            account: draft.destination,
            organization: draft.organization,
        });
    }
    if destination.kind() != AccountKind::AccountedFunds {
        return Err(BusinessProfitSweepError::InvalidDestinationKind(
            draft.destination,
        ));
    }
    Ok(())
}

fn validate_business_profit_sweep_capacity(
    draft: &BusinessProfitSweepDraft,
    operating_balance: Money,
    capital_floor: Money,
) -> Result<(), BusinessProfitSweepError> {
    if operating_balance < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientTillCash {
            business: draft.business,
            available_cents: operating_balance.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    let available_surplus = operating_balance
        .checked_sub(capital_floor)
        .ok_or(BusinessProfitSweepError::InsufficientEarnedSurplus {
            business: draft.business,
            available_cents: 0,
            requested_cents: draft.amount.cents(),
        })?
        .max(Money::ZERO);
    if available_surplus < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientEarnedSurplus {
            business: draft.business,
            available_cents: available_surplus.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    Ok(())
}
