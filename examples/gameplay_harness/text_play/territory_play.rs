//! District views and territorial orders, gated by held reconnaissance.
use super::*;
use crimocracy::core::id::NeighborhoodId;
use crimocracy::world::territory_conflict::{
    active_district_force, local_defensive_forces, validate_territory_clash,
};
use crimocracy::world::territory_influence::{
    active_standstill, validate_negotiate_standstill_with_tribute,
};

impl Session<'_> {
    pub(super) fn execute_territory_commands(&mut self, words: &[&str]) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        match words[0] {
            "districts" => {
                for district in state.world().neighborhoods() {
                    writeln!(
                        out,
                        "district-{} | {}",
                        district.id().raw(),
                        district.name()
                    )
                    .unwrap();
                }
            }
            "turf" => {
                let defenses = local_defensive_forces(state, self.player);
                let mut by_district = BTreeMap::<NeighborhoodId, Vec<_>>::new();
                for (id, info) in self.enterprise_observations() {
                    if let Some(record) = state.enterprises().get_enterprise(id)
                        && record.organization() != self.player
                        && let Some(district) = self.observed_enterprise_district(info)
                    {
                        by_district.entry(district).or_default().push((
                            id,
                            record.organization(),
                            info,
                        ));
                    }
                }
                for district in state.world().neighborhoods() {
                    let footprint = defenses.get(&district.id()).copied().unwrap_or_default();
                    let defense = footprint.total();
                    writeln!(out, "district-{} {} | own active rackets: {} | active associates: {} | operating shops: {} | local defense: {} (including {} distributed named enforcers) | rival influence requires reconnaissance",
                        district.id().raw(), district.name(), footprint.active_rackets, footprint.associates,
                        footprint.operating_shops, defense, footprint.named_enforcers).unwrap();
                    if footprint.operating_shops > 0 && defense == 0 {
                        writeln!(out, "  Unprotected commercial holdings: vulnerable to rival rackets and protection demands.").unwrap();
                    }
                    if let Some(standstill) = active_standstill(state, self.player, district.id()) {
                        let rival = state
                            .world()
                            .get_organization(standstill.counterparty)
                            .expect("agreed counterpart exists");
                        writeln!(out, "  Standstill with {} (org-{}) until {} | no new rackets or offensive business operations",
                            rival.name(), rival.id().raw(), time_label(standstill.expires_at.as_minutes())).unwrap();
                    }
                    if let Some(sightings) = by_district.get(&district.id()) {
                        for (id, organization, info) in sightings {
                            writeln!(out, "  Observed enterprise-{} (org-{}) | {} | {} (information-{}) | current status and staffing unconfirmed",
                                id.raw(), organization.raw(), time_label(info.observed_at().as_minutes()), info.summary(), info.id().raw()).unwrap();
                        }
                    }
                }
            }
            "negotiate" => {
                let district_number = parse_number(words[1], "district")?;
                let district = state
                    .world()
                    .neighborhoods()
                    .find(|item| item.id().raw() == district_number)
                    .ok_or("unknown district")?
                    .id();
                let rival_number = parse_number(words[2], "org")?;
                let rival = self
                    .known_organization_ids()
                    .into_iter()
                    .find(|id| id.raw() == rival_number && *id != self.player)
                    .ok_or("rival organization is not identified; use reconnaissance")?;
                // Enterprise affiliation is fixed at establishment; a typed
                // surveillance sighting names its operator and locates the racket.
                if !self
                    .enterprise_observations()
                    .into_iter()
                    .any(|(id, info)| {
                        self.observed_enterprise_district(info) == Some(district)
                            && state
                                .enterprises()
                                .get_enterprise(id)
                                .is_some_and(|record| record.organization() == rival)
                    })
                {
                    return Err(
                        "no identified rival racket in this district; surveil its operations first"
                            .to_owned(),
                    );
                }
                let tribute_cents = if let Some(raw) = words.get(3) {
                    raw.strip_prefix("tribute=")
                        .ok_or("optional fourth argument must be tribute=<positive cents>")?
                        .parse::<i64>()
                        .ok()
                        .filter(|amount| *amount > 0)
                        .ok_or("tribute must be a positive whole number of cents")?
                } else {
                    0
                };
                let history = validate_negotiate_standstill_with_tribute(
                    state,
                    self.player,
                    rival,
                    district,
                    crimocracy::finance::Money::from_cents(tribute_cents),
                )
                .map_err(|error| error.to_string())?
                .commit(&mut self.state)
                .map_err(|error| error.to_string())?;
                writeln!(out, "Agreed a 30-day district standstill with org-{rival_number}. {}Recorded as {history}. Existing rackets continue; both gangs refrain from new rackets and offensive business operations until expiry.",
                    if tribute_cents > 0 { format!("Transferred {tribute_cents} cents street-cash tribute. ") } else { String::new() }).unwrap();
            }
            "contest" | "preview-contest" => {
                let number = parse_number(words[1], "enterprise")?;
                let target = self
                    .known_enterprise_ids()
                    .into_iter()
                    .find(|id| id.raw() == number)
                    .ok_or("rival racket not identified; surveil the district first")?;
                let racket = state
                    .enterprises()
                    .get_enterprise(target)
                    .ok_or("racket no longer exists")?;
                if racket.organization() == self.player {
                    return Err(
                        "contest needs an identified rival racket, not one of your own".to_owned(),
                    );
                }
                let observations = self.enterprise_observations();
                let info = observations
                    .get(&target)
                    .ok_or("rival racket location unverified; surveil its operations first")?;
                let district = self
                    .observed_enterprise_district(info)
                    .ok_or("rival racket location unverified; surveil its operations first")?;
                let own_force = active_district_force(state, self.player, district);
                let prepared = validate_territory_clash(state, self.player, target)
                    .map_err(|error| error.to_string())?;
                if words[0] == "preview-contest" {
                    writeln!(out, "Challenge enterprise-{number} in district-{} | active associates: {own_force}. Defenders have a positional advantage; their current force is unverified. Defeat suspends your leading racket for seven days. No action committed.", district.raw()).unwrap();
                } else {
                    let result = prepared
                        .commit(&mut self.state)
                        .map_err(|error| error.to_string())?;
                    writeln!(out, "District-{} confrontation: {}. Racket {} suspended for seven days; next challenge allowed in seven days. History: {}. Review 'reports'.",
                        district.raw(),
                        if result.attacker_won { "your gang displaced a rival operation" } else { "your assault was repelled" },
                        result.suspended_racket, result.history).unwrap();
                }
            }
            _ => unreachable!("dispatcher only routes territory commands"),
        }
        Ok(out)
    }
}
