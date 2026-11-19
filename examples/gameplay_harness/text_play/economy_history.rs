//! Read-only settlement chronology for assets the player actually owns.
//! Previously independent businesses' pre-acquisition books are not revealed.

use super::*;
use crimocracy::world::BusinessOwner;

impl Session<'_> {
    pub(super) fn execute_economy_history(&self, words: &[&str]) -> Result<String, String> {
        let count = words
            .get(2)
            .map_or(Ok(8), |value| {
                value
                    .parse::<usize>()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or("cycle count must be a positive integer")
            })?
            .min(40);
        let mut out = String::new();
        match words[0] {
            "business-cycles" => {
                let raw = parse_number(words[1], "business")?;
                let business = self
                    .state
                    .world()
                    .businesses_owned_by_organization(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("business does not belong to your organization")?;
                writeln!(
                    out,
                    "business-{} {} | settled operating cycles:",
                    business.id().raw(),
                    business.name()
                )
                .unwrap();
                let cycles = self
                    .state
                    .economy()
                    .cycles_for(business.id())
                    .rev()
                    .filter(|cycle| cycle.owner() == BusinessOwner::Organization(self.player))
                    .take(count)
                    .collect::<Vec<_>>();
                if cycles.is_empty() {
                    out.push_str("  No settled operating cycles under your ownership.");
                }
                for cycle in cycles {
                    writeln!(out,
                        "  business-cycle-{} | {} | gross {} cents | cost {} cents | net {:+} cents | disrupted: {} | market: {:.1}% of normal potential",
                        cycle.id().raw(),
                        time_label(cycle.occurred_at().as_minutes()),
                        cycle.gross_revenue().cents(),
                        cycle.operating_cost().cents(),
                        cycle.net_cash().cents(),
                        cycle.disrupted(),
                        f64::from(cycle.market_gross_basis_points(self.registry)) / 100.0,
                    ).unwrap();
                }
            }
            "racket-cycles" => {
                let raw = parse_number(words[1], "enterprise")?;
                let enterprise = self
                    .state
                    .enterprises()
                    .enterprises_for_organization(self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or("enterprise does not belong to your organization")?;
                writeln!(
                    out,
                    "enterprise-{} {:?} at {} | settled cycles:",
                    enterprise.id().raw(),
                    enterprise.kind(),
                    self.enterprise_location_label(enterprise.location())
                )
                .unwrap();
                let cycles = self
                    .state
                    .enterprises()
                    .cycles_for(enterprise.id())
                    .rev()
                    .take(count)
                    .collect::<Vec<_>>();
                if cycles.is_empty() {
                    out.push_str("  No settled cycles for this enterprise.");
                }
                for cycle in cycles {
                    writeln!(out,
                        "  enterprise-cycle-{} | {} | gross {} cents | cost {} cents | net {:+} cents | heat {} cents | enforcement: {}",
                        cycle.id().raw(),
                        time_label(cycle.occurred_at().as_minutes()),
                        cycle.gross_revenue().cents(),
                        cycle.operating_cost().cents(),
                        cycle.net_cash().cents(),
                        cycle.investigation_heat().cents(),
                        cycle.drew_enforcement_attention(),
                    ).unwrap();
                }
            }
            _ => return Err("unknown economy-history command".to_owned()),
        }
        Ok(out)
    }
}
