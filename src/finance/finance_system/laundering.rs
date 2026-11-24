//! Laundering through legitimate cash-intensive business fronts.
//!
//! The parent finance system remains the canonical financial mutation owner. This child owns
//! laundering-specific validation and composes the parent ledger transaction path atomically
//! with the economy-owned plausibility-capacity mutation.

use super::{FinanceError, ValidatedLedgerTransaction, validate_record_business_transaction};
use crate::core::id::{FinancialAccountId, LedgerTransactionId};
use crate::core::state::AppState;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::economy::business_economy_system::resolve_business_current_gross;
use crate::economy::{BusinessEconomyRecord, BusinessOperatingStatus};
use crate::finance::{
    AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money,
    helpers::apply_basis_point_multiplier,
};
use crate::registry::Registry;
use crate::world::{BusinessOwner, BusinessRecord, OrganizationKind};
use thiserror::Error;

/// A dirty-to-accounted funds conversion routed through an owned cash-intensive front.
///
/// The canonical laundering path: street cash leaves a StreetCash account, arrives in the
/// organization's AccountedFunds minus the authored laundering fee, and the fee lands in the
/// front's non-liquid settlement account as an external cost. The fee must leave organization-
/// controlled liquidity permanently; routing it into the front's operating till would let the
/// owner sweep it back later and turn an authored cost into a temporary bookkeeping delay.
/// Plausibility is enforced against the front's legitimate gross potential, so volume requires
/// larger or additional fronts.
#[derive(Clone, Debug)]
pub struct LaunderingDraft {
    pub organization: crate::core::id::OrganizationId,
    pub street_account: FinancialAccountId,
    pub business: crate::core::id::BusinessId,
    pub accounted_account: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LaunderingError {
    #[error("laundering amount must be positive")]
    NonPositiveAmount,
    #[error("laundering amount is too small to produce both a laundering fee and accounted funds")]
    AmountTooSmallForSplit,
    #[error("laundering organization {0} does not exist")]
    MissingOrganization(crate::core::id::OrganizationId),
    #[error("laundering organization {0} is not a criminal organization")]
    InvalidOrganizationKind(crate::core::id::OrganizationId),
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("account {account} is not owned by organization {organization}")]
    AccountOwnerMismatch {
        account: FinancialAccountId,
        organization: crate::core::id::OrganizationId,
    },
    #[error("street-cash source account {0} must hold street cash")]
    InvalidStreetAccountKind(FinancialAccountId),
    #[error("destination account {0} must hold accounted funds")]
    InvalidAccountedAccountKind(FinancialAccountId),
    #[error("business {0} does not exist")]
    MissingBusiness(crate::core::id::BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(crate::core::id::BusinessId),
    #[error("business {business} changed after laundering validation")]
    StaleBusiness {
        business: crate::core::id::BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {0} lacks the cash-intensive function required to absorb illicit cash")]
    NotCashIntensive(crate::core::id::BusinessId),
    #[error("business {0} has no active operating economy to route laundered revenue through")]
    MissingBusinessEconomy(crate::core::id::BusinessId),
    #[error("business {0}'s operating economy is suspended and cannot route laundered revenue")]
    EconomySuspended(crate::core::id::BusinessId),
    #[error(
        "street-cash account {account} holds {balance_cents} cents and cannot launder {requested_cents}"
    )]
    InsufficientStreetCash {
        account: FinancialAccountId,
        balance_cents: i64,
        requested_cents: i64,
    },
    #[error(
        "amount {requested_cents} exceeds business {business}'s plausible laundering capacity {capacity_cents}"
    )]
    CapacityExceeded {
        business: crate::core::id::BusinessId,
        requested_cents: i64,
        capacity_cents: i64,
    },
    #[error("laundering arithmetic overflowed")]
    ArithmeticOverflow,
    #[error("business {business}'s operating economy changed after laundering validation")]
    StaleEconomy {
        business: crate::core::id::BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    BusinessEconomy(#[from] crate::economy::business_economy_system::BusinessEconomyError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

pub struct ValidatedLaundering<'r> {
    registry: &'r Registry,
    transaction: ValidatedLedgerTransaction,
    business: crate::core::id::BusinessId,
    organization: crate::core::id::OrganizationId,
    expected_business_version: u32,
    amount: Money,
    expected_economy_version: u32,
}

impl ValidatedLaundering<'_> {
    pub fn commit(self, state: &mut AppState) -> Result<LedgerTransactionId, LaunderingError> {
        let business = state
            .world
            .get_business(self.business)
            .ok_or(LaunderingError::MissingBusiness(self.business))?;
        if business.version() != self.expected_business_version {
            return Err(LaunderingError::StaleBusiness {
                business: self.business,
                expected: self.expected_business_version,
                found: business.version(),
            });
        }
        if business.owner() != BusinessOwner::Organization(self.organization) {
            return Err(LaunderingError::ForeignBusiness(self.business));
        }
        // The capacity decision rested on the front's economy at validation time. A cycle
        // settling in between resets the plausibility budget (and a chronic-loss suspension
        // deactivates the front), so the version pin must be re-checked before anything mutates.
        let economy = state
            .economy
            .get_business_economy(self.business)
            .ok_or(LaunderingError::MissingBusinessEconomy(self.business))?;
        if economy.version() != self.expected_economy_version {
            return Err(LaunderingError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        ensure_version_can_advance(economy.version(), "business economy")?;
        // A local competitor can open or recover during the same minute without changing
        // this front's version. Re-prove the live earning-power ceiling before moving money.
        let new_cycle_total = validate_laundering_capacity(
            self.registry,
            state,
            self.business,
            self.amount,
            economy,
        )?;
        let id = self.transaction.commit(state)?;
        // Both the ledger and the economy's cumulative volume are preflighted.
        crate::economy::business_economy_system::apply_laundering_capacity_preflighted(
            state,
            self.business,
            id,
            new_cycle_total,
        );
        Ok(id)
    }

    /// The front business that absorbed the transfer; callers use this for reporting.
    pub fn business(&self) -> crate::core::id::BusinessId {
        self.business
    }
}

pub(super) fn resolve_laundering_split(
    amount: Money,
    fee_basis_points: u32,
) -> Result<(Money, Money), LaunderingError> {
    let fee = apply_basis_point_multiplier(amount, fee_basis_points)
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    let credited = amount
        .checked_sub(fee)
        .ok_or(LaunderingError::ArithmeticOverflow)?;
    if fee <= Money::ZERO || credited <= Money::ZERO {
        return Err(LaunderingError::AmountTooSmallForSplit);
    }
    Ok((fee, credited))
}

/// Read-only planning query: how much more street cash the front's books can plausibly
/// absorb this cycle. Returns `None` when the business is missing, foreign-owned, not
/// cash-intensive, or has no active operating economy — the same conditions that make
/// `validate_launder_funds` reject — and `Some(Money::ZERO)` when the budget is spent.
/// Leadership can size a transfer from its own books before drafting, instead of learning
/// the ceiling only through a rejected over-capacity attempt.
pub fn remaining_laundering_capacity(
    registry: &Registry,
    state: &AppState,
    organization: crate::core::id::OrganizationId,
    business: crate::core::id::BusinessId,
) -> Option<Money> {
    let record = state.world.get_business(business)?;
    if record.owner() != BusinessOwner::Organization(organization) {
        return None;
    }
    if !record
        .functions()
        .contains(&crate::world::BusinessFunction::CashIntensive)
    {
        return None;
    }
    let economy = state.economy.get_business_economy(business)?;
    if economy.status() != BusinessOperatingStatus::Active {
        return None;
    }
    let capacity = laundering_ceiling(registry, state, business).ok()?;
    Some(unspent_laundering_capacity(
        capacity,
        economy.laundered_this_cycle(),
    ))
}

/// A front can become over its current plausibility ceiling after sabotage without
/// retroactively invalidating valid transfers. That leaves no *additional* room, not
/// a negative allowance that leadership could misinterpret as an available amount.
fn unspent_laundering_capacity(capacity: Money, consumed: Money) -> Money {
    capacity
        .checked_sub(consumed)
        .unwrap_or(Money::ZERO)
        .max(Money::ZERO)
}

pub fn validate_launder_funds<'r>(
    registry: &'r Registry,
    state: &AppState,
    draft: LaunderingDraft,
) -> Result<ValidatedLaundering<'r>, LaunderingError> {
    if draft.amount.cents() <= 0 {
        return Err(LaunderingError::NonPositiveAmount);
    }
    validate_laundering_organization(state, draft.organization)?;
    validate_laundering_street_account(state, &draft)?;
    validate_laundering_accounted_account(state, &draft)?;
    let (business_record, economy) = validate_laundering_front(state, &draft)?;
    // Plausibility: the front can hide only the authored fraction of what it legitimately
    // earns per cycle, and the budget is cumulative — a front that already absorbed volume
    // this cycle has less plausible room left, so volume requires larger or additional fronts
    // rather than many small transfers. The basis is the front's current earning power, so a
    // sabotage-disrupted front cannot hide cash its degraded books cannot explain.
    validate_laundering_capacity(registry, state, draft.business, draft.amount, economy)?;
    // Both legs must remain material after cent rounding. A zero fee would fail to prove which
    // front absorbed the transfer, while a zero accounted credit would call a pure front-revenue
    // transfer "laundering" without cleaning any money.
    let transaction =
        validate_laundering_transaction(registry, state, &draft, business_record, economy)?;
    Ok(ValidatedLaundering {
        registry,
        transaction,
        business: draft.business,
        organization: draft.organization,
        expected_business_version: business_record.version(),
        amount: draft.amount,
        expected_economy_version: economy.version(),
    })
}

fn validate_laundering_organization(
    state: &AppState,
    organization: crate::core::id::OrganizationId,
) -> Result<(), LaunderingError> {
    let record = state
        .world
        .get_organization(organization)
        .ok_or(LaunderingError::MissingOrganization(organization))?;
    if record.kind() != OrganizationKind::Criminal {
        return Err(LaunderingError::InvalidOrganizationKind(organization));
    }
    Ok(())
}

fn validate_laundering_street_account(
    state: &AppState,
    draft: &LaunderingDraft,
) -> Result<(), LaunderingError> {
    let street = state
        .finance
        .get_account(draft.street_account)
        .ok_or(LaunderingError::MissingAccount(draft.street_account))?;
    if street.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.street_account,
            organization: draft.organization,
        });
    }
    if street.kind() != AccountKind::StreetCash {
        return Err(LaunderingError::InvalidStreetAccountKind(
            draft.street_account,
        ));
    }
    // The source must actually hold the cash being cleaned: debiting a phantom balance would
    // mint accounted funds out of nothing, so laundering gates on available street cash the
    // same way every other spend path does.
    if street.balance().cents() < draft.amount.cents() {
        return Err(LaunderingError::InsufficientStreetCash {
            account: draft.street_account,
            balance_cents: street.balance().cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    Ok(())
}

fn validate_laundering_accounted_account(
    state: &AppState,
    draft: &LaunderingDraft,
) -> Result<(), LaunderingError> {
    let accounted = state
        .finance
        .get_account(draft.accounted_account)
        .ok_or(LaunderingError::MissingAccount(draft.accounted_account))?;
    if accounted.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(LaunderingError::AccountOwnerMismatch {
            account: draft.accounted_account,
            organization: draft.organization,
        });
    }
    if accounted.kind() != AccountKind::AccountedFunds {
        return Err(LaunderingError::InvalidAccountedAccountKind(
            draft.accounted_account,
        ));
    }
    Ok(())
}

fn validate_laundering_front<'a>(
    state: &'a AppState,
    draft: &LaunderingDraft,
) -> Result<(&'a BusinessRecord, &'a BusinessEconomyRecord), LaunderingError> {
    let business = state
        .world
        .get_business(draft.business)
        .ok_or(LaunderingError::MissingBusiness(draft.business))?;
    if business.owner() != BusinessOwner::Organization(draft.organization) {
        return Err(LaunderingError::ForeignBusiness(draft.business));
    }
    if !business
        .functions()
        .contains(&crate::world::BusinessFunction::CashIntensive)
    {
        return Err(LaunderingError::NotCashIntensive(draft.business));
    }
    let economy = state
        .economy
        .get_business_economy(draft.business)
        .ok_or(LaunderingError::MissingBusinessEconomy(draft.business))?;
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(LaunderingError::EconomySuspended(draft.business));
    }
    ensure_version_can_advance(economy.version(), "business economy")?;
    Ok((business, economy))
}

fn laundering_ceiling(
    registry: &Registry,
    state: &AppState,
    business: crate::core::id::BusinessId,
) -> Result<Money, LaunderingError> {
    let gross_potential = resolve_business_current_gross(registry, state, business)?;
    apply_basis_point_multiplier(
        gross_potential,
        registry.laundering().plausibility_gross_basis_points(),
    )
    .ok_or(LaunderingError::ArithmeticOverflow)
}

fn validate_laundering_capacity(
    registry: &Registry,
    state: &AppState,
    business: crate::core::id::BusinessId,
    amount: Money,
    economy: &BusinessEconomyRecord,
) -> Result<Money, LaunderingError> {
    let capacity = laundering_ceiling(registry, state, business)?;
    let already_laundered = economy.laundered_this_cycle();
    let remaining = unspent_laundering_capacity(capacity, already_laundered);
    if amount > remaining {
        return Err(LaunderingError::CapacityExceeded {
            business,
            requested_cents: amount.cents(),
            capacity_cents: remaining.cents(),
        });
    }
    already_laundered
        .checked_add(amount)
        .ok_or(LaunderingError::ArithmeticOverflow)
}

fn validate_laundering_transaction(
    registry: &Registry,
    state: &AppState,
    draft: &LaunderingDraft,
    business: &BusinessRecord,
    economy: &BusinessEconomyRecord,
) -> Result<ValidatedLedgerTransaction, LaunderingError> {
    let (fee, credited) =
        resolve_laundering_split(draft.amount, registry.laundering().fee_basis_points())?;
    let postings = vec![
        LedgerPosting {
            account: draft.street_account,
            amount: Money::ZERO
                .checked_sub(draft.amount)
                .ok_or(LaunderingError::ArithmeticOverflow)?,
        },
        LedgerPosting {
            account: draft.accounted_account,
            amount: credited,
        },
        LedgerPosting {
            account: economy.settlement_account(),
            amount: fee,
        },
    ];
    Ok(validate_record_business_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Laundered {} through {}",
                crate::finance::helpers::format_money_cents(draft.amount.cents()),
                business.name(),
            ),
            postings,
            authorization: None,
        },
    )?)
}
