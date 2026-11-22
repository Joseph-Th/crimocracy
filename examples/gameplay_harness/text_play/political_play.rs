//! Own-books political coalition dossier, never a hidden rival/intelligence feed.
use super::*;
use crimocracy::core::campaign_victory::{
    VICTORY_PROFITABLE_CYCLES, political_business_progress, political_district_progress,
};
use crimocracy::politics::political_system::{
    dismiss_specialist, initiative_available, next_municipal_election_day, specialist_output,
    validate_civic_initiative, validate_hire_specialist, validate_invest_influence,
    validate_nominate_mayor,
};
use crimocracy::politics::{CivicInitiative, SpecialistKind};

fn specialist_kind(raw: &str) -> Result<SpecialistKind, String> {
    match raw {
        "organizer" => Ok(SpecialistKind::Organizer),
        "publicist" => Ok(SpecialistKind::Publicist),
        "strategist" => Ok(SpecialistKind::Strategist),
        _ => Err("specialist must be organizer, publicist, or strategist".to_owned()),
    }
}

fn pressure_label(score: u16) -> &'static str {
    match score {
        0 => "none observed",
        1..=249 => "limited",
        250..=599 => "organized",
        _ => "intense",
    }
}

fn initiative_kind(raw: &str) -> Result<CivicInitiative, String> {
    match raw {
        "meetings" => Ok(CivicInitiative::PublicMeetings),
        "services" => Ok(CivicInitiative::NeighborhoodServices),
        "labor" => Ok(CivicInitiative::LaborCoalition),
        "press" => Ok(CivicInitiative::LocalPress),
        _ => Err("initiative must be meetings, services, labor or press".to_owned()),
    }
}

impl Session<'_> {
    pub(super) fn campaign_readiness(&self) -> Result<String, String> {
        let progress = crimocracy::core::campaign_victory::progress(self.registry, &self.state)
            .map_err(|e| e.to_string())?;
        let rows =
            political_district_progress(self.registry, &self.state).map_err(|e| e.to_string())?;
        let mut out = format!(
            "Mayoral campaign | lifetime spend {} / {} cents | clean reserve {} / {} cents | sustained electoral districts {} / {}\n",
            progress.campaign_spent_cents,
            progress.required_campaign_spent_cents,
            progress.accounted_net.cents(),
            progress.required_mayoral_reserve_cents,
            progress.mayoral_majority_districts,
            progress.required_political_districts,
        );
        writeln!(
            out,
            "Next election day {} | nomination {:?} | latest election {:?}, won {}",
            next_municipal_election_day(self.state.now()),
            progress.mayoral_nominated_for_day,
            progress.latest_election_day,
            progress.latest_election_won
        )
        .unwrap();
        for district in self.state.world().neighborhoods() {
            let own = self
                .state
                .politics()
                .organization(self.player)
                .and_then(|campaign| campaign.district(district.id()));
            let row = rows.iter().find(|row| row.district == district.id());
            let rival = crimocracy::politics::political_system::rival_pressure(
                &self.state,
                self.player,
                district.id(),
            );
            if own.is_none() && row.is_none() {
                continue;
            }
            writeln!(out,
                "  district-{} {} | machine {}/1000 | grassroots {}/1000 | institutions {}/1000 | rival activity {} | majority held {} day(s) | election-ready {}",
                district.id().raw(), district.name(),
                own.map_or(0, |office| office.influence()),
                own.map_or(0, |office| office.grassroots()),
                own.map_or(0, |office| office.institutions()),
                pressure_label(rival),
                row.map_or(0, |row| row.majority_held_days),
                row.is_some_and(|row| row.mayoral_ready),
            ).unwrap();
            if let Some(office) = own {
                for (kind, manager) in office.specialists() {
                    let (machine, grassroots, institutions) =
                        specialist_output(&self.state, *manager, *kind);
                    writeln!(out, "    {:?} under char-{} | salary {} cents/day | funded daily +{} machine, +{} grassroots, +{} institutions",
                        kind, manager.raw(), kind.daily_salary(), machine, grassroots, institutions).unwrap();
                }
            }
        }
        out.push_str("A ward needs 800 machine, 300 grassroots and 300 institutional support, plus a 3:2 lead over rival machine influence for 30 days. All three erode daily. Filing costs 2000000 cents, requires an established political contact and must precede a municipal ballot by 30 days. Elections occur every 180 days; resident fear and active local rackets cost votes.");
        Ok(out)
    }

    pub(super) fn election_brief(&self) -> Result<String, String> {
        let Some(result) = self
            .state
            .politics()
            .organization(self.player)
            .and_then(|campaign| campaign.latest_election())
        else {
            return Ok(format!(
                "No mayoral ballot yet. Next election day {}. File your leader at least 30 days in advance using 'nominate-mayor char-N'.",
                next_municipal_election_day(self.state.now())
            ));
        };
        let mut out = format!(
            "Municipal election day {} | nominee char-{} | qualified {} | wards won {}/{} | elected {}\n",
            result.election_day,
            result.candidate.raw(),
            result.eligible,
            result.wards_won,
            result.wards_needed,
            result.elected
        );
        for ballot in &result.ballots {
            let ward = self
                .state
                .world()
                .get_neighborhood(ballot.district)
                .map_or("unknown", |record| record.name());
            writeln!(
                out,
                "  district-{} {} | candidate {} votes | incumbent {} votes | rival {} votes | {}",
                ballot.district.raw(),
                ward,
                ballot.candidate_votes,
                ballot.incumbent_votes,
                ballot.rival_votes,
                if ballot.ward_won { "won" } else { "lost" }
            )
            .unwrap();
        }
        Ok(out)
    }

    pub(super) fn execute_campaign_order(&mut self, words: &[&str]) -> Result<String, String> {
        if words[0] == "nominate-mayor" {
            let raw = parse_number(words[1], "char")?;
            let candidate = self
                .state
                .world()
                .characters_in_organization(self.player)
                .find(|person| person.id().raw() == raw)
                .ok_or("candidate must be a named member")?
                .id();
            validate_nominate_mayor(&self.state, candidate)
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
            let day = self
                .state
                .politics()
                .organization(self.player)
                .and_then(|campaign| campaign.candidacy())
                .expect("nomination was recorded")
                .election_day;
            return Ok(format!(
                "char-{raw} nominated for the mayoral election on day {day}. Filing fee: 2000000 accounted cents."
            ));
        }
        let raw = parse_number(words[1], "district")?;
        let district = self
            .state
            .world()
            .neighborhoods()
            .find(|district| district.id().raw() == raw)
            .ok_or("unknown public city district")?
            .id();
        match words[0] {
            "buy-influence" => {
                let cents = words[2]
                    .parse::<i64>()
                    .map_err(|_| "investment must be an integer number of accounted cents")?;
                validate_invest_influence(&self.state, district, cents)
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Spent {cents} clean cents on district-{raw} political influence."
                ))
            }
            "hire-specialist" => {
                let manager_raw = parse_number(words[2], "char")?;
                let manager = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|person| person.id().raw() == manager_raw)
                    .ok_or("manager must be an existing named member")?
                    .id();
                let kind = specialist_kind(words[3])?;
                validate_hire_specialist(&self.state, district, manager, kind)
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Hired {kind:?} in district-{raw} under char-{manager_raw} for {} cents; daily salary {} cents.",
                    kind.signing_cost(),
                    kind.daily_salary()
                ))
            }
            "fire-specialist" => {
                let kind = specialist_kind(words[2])?;
                dismiss_specialist(&mut self.state, district, kind).map_err(|e| e.to_string())?;
                Ok(format!(
                    "Released {kind:?} from district-{raw}; future salary ended."
                ))
            }
            "campaign-event" => {
                let manager_raw = parse_number(words[2], "char")?;
                let manager = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|person| person.id().raw() == manager_raw)
                    .ok_or("campaign organizer must be a named member")?
                    .id();
                let kind = initiative_kind(words[3])?;
                validate_civic_initiative(&self.state, district, manager, kind)
                    .map_err(|e| e.to_string())?
                    .commit(&mut self.state)
                    .map_err(|e| e.to_string())?;
                let (machine, grassroots, institutions) = kind.boosts();
                Ok(format!(
                    "{:?} in district-{raw}: {} cents paid; +{machine} machine, +{grassroots} grassroots, +{institutions} institutions.",
                    kind,
                    kind.cost_cents()
                ))
            }
            _ => Err("unknown campaign order".to_owned()),
        }
    }

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
            writeln!(
                out,
                "Electoral contest: machine {}/1000 | grassroots {}/1000 | institutions {}/1000 | rival pressure {} | majority held {} day(s) / 30 | mayoral-ready {}",
                local.own_influence, local.grassroots_support, local.institutional_support,
                pressure_label(local.rival_influence), local.majority_held_days, local.mayoral_ready,
            ).unwrap();
            for (kind, command) in [
                (CivicInitiative::PublicMeetings, "meetings"),
                (CivicInitiative::NeighborhoodServices, "services"),
                (CivicInitiative::LaborCoalition, "labor"),
                (CivicInitiative::LocalPress, "press"),
            ] {
                let office = self
                    .state
                    .politics()
                    .organization(self.player)
                    .and_then(|campaign| campaign.district(district.id()));
                let last = office.and_then(|office| office.initiative_last_held(kind));
                let days_left = last.map_or(0, |at| {
                    (7 * DAY_MINUTES)
                        .saturating_sub(
                            self.state
                                .now()
                                .as_minutes()
                                .saturating_sub(at.as_minutes()),
                        )
                        .div_ceil(DAY_MINUTES)
                });
                writeln!(
                    out,
                    "  {command}: {} | cost {} cents | cooldown {} day(s)",
                    if initiative_available(&self.state, self.player, district.id(), kind) {
                        "venue/contact ready"
                    } else {
                        "needs compatible venue or labor contact"
                    },
                    kind.cost_cents(),
                    days_left
                )
                .unwrap();
            }
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
