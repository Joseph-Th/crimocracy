//! Shared checked arithmetic for read-only financial-cycle reporting.

use crate::core::attention::AttentionClass;
use crate::finance::Money;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperatingCycleAmounts {
    pub(crate) gross_revenue: Money,
    pub(crate) operating_cost: Money,
    pub(crate) net_cash: Money,
}

/// Adds one settled operating cycle to a reporting accumulator. Business and enterprise
/// summaries expose different domain-specific total types, but the money/count arithmetic is
/// identical and must not drift between those reporting surfaces.
pub(crate) fn accumulate_cycle(
    cycle_count: &mut u32,
    notable_cycle_count: &mut u32,
    gross_total: &mut Money,
    operating_cost_total: &mut Money,
    net_cash_total: &mut Money,
    amounts: OperatingCycleAmounts,
    attention: AttentionClass,
) -> Option<()> {
    let next_cycle_count = cycle_count.checked_add(1)?;
    let next_notable_cycle_count = if attention == AttentionClass::Notable {
        notable_cycle_count.checked_add(1)?
    } else {
        *notable_cycle_count
    };
    let next_gross_total = gross_total.checked_add(amounts.gross_revenue)?;
    let next_operating_cost_total = operating_cost_total.checked_add(amounts.operating_cost)?;
    let next_net_cash_total = net_cash_total.checked_add(amounts.net_cash)?;

    *cycle_count = next_cycle_count;
    *notable_cycle_count = next_notable_cycle_count;
    *gross_total = next_gross_total;
    *operating_cost_total = next_operating_cost_total;
    *net_cash_total = next_net_cash_total;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_preserves_the_entire_accumulator() {
        let mut cycle_count = 4;
        let mut notable_cycle_count = 2;
        let mut gross_total = Money::from_cents(100);
        let mut operating_cost_total = Money::from_cents(60);
        let mut net_cash_total = Money::from_cents(i64::MAX);
        let before = (
            cycle_count,
            notable_cycle_count,
            gross_total,
            operating_cost_total,
            net_cash_total,
        );

        assert_eq!(
            accumulate_cycle(
                &mut cycle_count,
                &mut notable_cycle_count,
                &mut gross_total,
                &mut operating_cost_total,
                &mut net_cash_total,
                OperatingCycleAmounts {
                    gross_revenue: Money::from_cents(25),
                    operating_cost: Money::from_cents(10),
                    net_cash: Money::from_cents(1),
                },
                AttentionClass::Notable,
            ),
            None
        );
        assert_eq!(
            (
                cycle_count,
                notable_cycle_count,
                gross_total,
                operating_cost_total,
                net_cash_total,
            ),
            before
        );
    }
}
