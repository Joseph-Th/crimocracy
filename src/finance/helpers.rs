//! Shared financial arithmetic helpers used by enterprise and business economy cycles.

use crate::core::id::FinancialAccountId;
use crate::finance::{LedgerPosting, Money};

/// Weighted contribution of a rating point value without overflow.
pub fn weighted_rating(per_point: Money, rating: u8) -> Option<Money> {
    let cents = per_point.cents().checked_mul(i64::from(rating))?;
    Some(Money::from_cents(cents))
}

/// Applies a basis-point variance (-10000..+10000 maps to 0..200%) to an amount.
pub fn apply_basis_point_variance(amount: Money, basis_points: i16) -> Option<Money> {
    let factor = 10_000_i128 + i128::from(basis_points);
    let adjusted = i128::from(amount.cents()).checked_mul(factor)? / 10_000_i128;
    let cents = i64::try_from(adjusted).ok()?;
    Some(Money::from_cents(cents))
}

/// Builds a balanced two-posting settlement for a net cash amount.
/// The settlement account is the fictitious counterparty.
pub fn build_settlement_postings(
    cash: FinancialAccountId,
    settlement: FinancialAccountId,
    net: Money,
) -> Option<[LedgerPosting; 2]> {
    let negated = net.cents().checked_neg()?;
    Some([
        LedgerPosting {
            account: cash,
            amount: net,
        },
        LedgerPosting {
            account: settlement,
            amount: Money::from_cents(negated),
        },
    ])
}
