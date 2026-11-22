//! Own-books political coalition dossier, never a hidden rival/intelligence feed.
use super::*;
use crimocracy::core::campaign_victory::{
    VICTORY_PROFITABLE_CYCLES, political_business_progress, political_district_progress,
};

impl Session<'_> {
    pub(super) fn political_readiness(&self, raw: &str) -> Result<String, String> {
        let id = parse_number(raw, "district")?;
        let district = self
            .state
            .world()
            .neighborhoods()
            .find(|district| district.id().raw() == id)
            .ok_or("no public district with that ID")?;
        let progress = political_district_progress(self.registry, &self.state)
            .map_err(|error| error.to_string())?;
        let local = progress.iter().find(|row| row.district == district.id());
        let mut out = String::new();
        writeln!(
            out,
            "Political dossier: {} (district-{id})",
            district.name()
        )
        .unwrap();
        if let Some(local) = local {
            writeln!(out,
                "Current readiness: {} | {} mature owned shops, {} recently proven, {} distinct proven kinds | citywide counted organizer: {}",
                if local.qualifies { "qualified" } else { "incomplete" },
                local.mature_active,
                local.established_active,
                local.established_kinds,
                local.steward.map_or("not assigned".to_owned(), |id| format!("char-{}", id.raw())),
            ).unwrap();
        }
        let political_mandates = self
            .state
            .delegation()
            .active_for_scope(ResponsibilityScope::Neighborhood(district.id()))
            .filter(|mandate| {
                mandate.organization() == self.player
                    && mandate.scopes().contains(&ResponsibilityScope::Function(
                        ResponsibilityFunction::Political,
                    ))
            })
            .collect::<Vec<_>>();
        if political_mandates.is_empty() {
            out.push_str("No local Political-function mandate assigned yet.\n");
        }
        for mandate in political_mandates {
            let manager = self
                .state
                .world()
                .get_character(mandate.manager())
                .expect("valid active mandate references a known person");
            writeln!(out,
                "  Local Political-function mandate {}: {} (char-{}) | autonomy {:?} | counted only when local commerce qualifies and the organizer passes citywide matching",
                mandate.id(), manager.name(), manager.id().raw(), manager.autonomy(),
            ).unwrap();
        }
        let properties = political_business_progress(&self.state, district.id())
            .map_err(|error| error.to_string())?;
        if properties.is_empty() {
            out.push_str(
                "No owned storefronts here. Check 'market' for available local property.\n",
            );
        }
        for property in properties {
            let name = self
                .state
                .world()
                .get_business(property.business)
                .expect("owned business from world index exists")
                .name();
            let blocker = if !property.active {
                "storefront suspended; resume before it can qualify".to_owned()
            } else if property.days_until_mature > 0 {
                format!(
                    "title matures in {} day(s), provided ownership continues",
                    property.days_until_mature
                )
            } else if property.profitable_cycles < VICTORY_PROFITABLE_CYCLES {
                format!(
                    "needs {} more profitable settlements in the last 30 days",
                    VICTORY_PROFITABLE_CYCLES - property.profitable_cycles
                )
            } else if property.established {
                "established and qualifying".to_owned()
            } else {
                "needs a recent settlement within seven days".to_owned()
            };
            writeln!(out,
                "  business-{} {} | {:?} | title age {} full day(s) | profitable settlements {}/{} | last owned settlement: {} | 30d own net {} cents | {}",
                property.business.raw(), name, property.kind, property.owned_days,
                property.profitable_cycles, VICTORY_PROFITABLE_CYCLES,
                property.last_owned_settlement.map_or("none".to_owned(),
                    |time| time_label(time.as_minutes())),
                property.recent_net.cents(), blocker,
            ).unwrap();
        }
        out.push_str("Title age, recent profits and settlement dates are actual own records. A fresh purchase cannot inherit the previous owner's political establishment. Mandates and citywide independent-organizer matching are shown by 'victory'.");
        Ok(out)
    }
}
