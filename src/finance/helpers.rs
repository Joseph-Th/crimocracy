//! Shared financial arithmetic used by enterprise and business economy cycles.

use crate::core::id::FinancialAccountId;
use crate::core::time::SimTime;
use crate::finance::{LedgerPosting, Money};

/// Weighted contribution of a rating point value without overflow.
pub fn weighted_rating(per_point: Money, rating: u8) -> Option<Money> {
    per_point.checked_mul(i64::from(rating))
}

/// Applies a basis-point variance (-10000..+10000 maps to 0..200%) to an amount.
/// Rounds half away from zero so upside and downside variances are symmetric.
pub fn resolve_basis_point_variance(amount: Money, basis_points: i16) -> Option<Money> {
    let factor = 10_000_i128 + i128::from(basis_points);
    let scaled = i128::from(amount.cents()).checked_mul(factor)?;
    let sign = if scaled < 0 { -1 } else { 1 };
    let adjusted = (scaled.abs() + 5_000) / 10_000 * sign;
    let cents = i64::try_from(adjusted).ok()?;
    Some(Money::from_cents(cents))
}

/// Reduces an amount to its basis-point share (`0..=10_000` maps to 0..100%), rounded half
/// away from zero to match the crate's single rounding convention.
pub fn resolve_basis_point_share(amount: Money, basis_points: u32) -> Option<Money> {
    let scaled = i128::from(amount.cents()).checked_mul(i128::from(basis_points))?;
    let negative = scaled < 0;
    let adjusted = (scaled.abs() + 5_000) / 10_000;
    let adjusted = if negative { -adjusted } else { adjusted };
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
    Some([
        LedgerPosting {
            account: cash,
            amount: net,
        },
        LedgerPosting {
            account: settlement,
            amount: net.checked_neg()?,
        },
    ])
}

/// Renders a cents amount as leader-readable dollars (`"$1,234.56"`, `"-$12.30"`).
/// Player-facing reports quote people talking about money, so raw cent counts stay
/// confined to diagnostics and ledger internals.
pub fn format_money_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let whole = abs / 100;
    let fraction = abs % 100;
    let digits = whole.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len().div_ceil(3));
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}${grouped}.{fraction:02}")
}

/// Describes a gross-variance draw as leader-readable language instead of basis points.
/// Cycle reports are how managers and accountants talk, so small draws read as "close to
/// plan" and material ones as an approximate percentage over or under expectations.
pub fn describe_gross_variance(basis_points: i16) -> String {
    let magnitude = i32::from(basis_points).unsigned_abs();
    if magnitude < 500 {
        "gross came in close to plan".to_owned()
    } else {
        let percent = magnitude as f64 / 100.0;
        let direction = if basis_points > 0 { "over" } else { "under" };
        format!("gross ran about {percent:.1}% {direction} plan")
    }
}

/// Consecutive most-recent settled cycles whose net cash was negative, capped at `limit`.
/// `newest_first` must be ordered newest settlement first — cycle indexes are id-ordered and
/// cycle IDs are allocated sequentially, so a reversed `cycles_for` scan provides that
/// directly and the scan touches at most `limit + 1` records no matter how much history
/// accumulates. Cycles settled at or before the loss-streak anchor predate the current grace
/// window (a resumed operation starts counting fresh) and end the scan, because everything
/// older is older still. Shared verbatim by enterprise and business economies so chronic-loss
/// semantics can never drift between them.
pub fn count_trailing_losing_cycles<T>(
    newest_first: &[T],
    occurred_at: impl Fn(&T) -> SimTime,
    net_cash: impl Fn(&T) -> Money,
    anchor: Option<SimTime>,
    limit: u8,
) -> u32 {
    let mut losing = 0u32;
    for cycle in newest_first {
        if losing >= u32::from(limit) || net_cash(cycle) >= Money::ZERO {
            break;
        }
        if anchor.is_some_and(|anchor| occurred_at(cycle) <= anchor) {
            break;
        }
        losing += 1;
    }
    losing
}

#[cfg(test)]
mod tests {
    use super::{describe_gross_variance, format_money_cents};

    #[test]
    fn money_format_groups_thousands_and_preserves_sign() {
        assert_eq!(format_money_cents(0), "$0.00");
        assert_eq!(format_money_cents(5), "$0.05");
        assert_eq!(format_money_cents(75), "$0.75");
        assert_eq!(format_money_cents(55320), "$553.20");
        assert_eq!(format_money_cents(1_234_567), "$12,345.67");
        assert_eq!(format_money_cents(-12_500), "-$125.00");
    }

    #[test]
    fn variance_description_reads_like_a_manager_not_a_ledger() {
        assert_eq!(describe_gross_variance(0), "gross came in close to plan");
        assert_eq!(describe_gross_variance(-300), "gross came in close to plan");
        assert_eq!(describe_gross_variance(499), "gross came in close to plan");
        assert_eq!(
            describe_gross_variance(1082),
            "gross ran about 10.8% over plan"
        );
        assert_eq!(
            describe_gross_variance(-1128),
            "gross ran about 11.3% under plan"
        );
    }
}
