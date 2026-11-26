//! Player-visible victory readiness and explanatory details over the core assessment.

use super::*;

impl Session<'_> {
    pub(super) fn victory_readiness(&self) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        let (standing, districts) =
            crimocracy::core::campaign_victory::progress_and_political_districts(
                self.registry,
                &self.state,
            )
            .map_err(|error| error.to_string())?;
        writeln!(
            out,
            "Optional endings (claim-victory <political|retirement|underboss>):"
        )
        .unwrap();
        writeln!(out, "  Mayoral election: {}/{} contacts held for 180 days; owned businesses in {} districts, {} productive mature districts ({} required of {} city districts)",
            standing.established_political_contacts, standing.active_political_contacts,
            standing.owned_operating_business_districts, standing.established_business_districts,
            standing.required_political_districts, standing.total_districts).unwrap();
        writeln!(out, "    Named political staff: {} / {} required (permanent crew, {} headquarters and two independent local managers per required district); staffed mature districts: {} / {}",
            standing.key_personnel, standing.required_political_personnel,
            crimocracy::core::campaign_victory::POLITICAL_HEADQUARTERS_PERSONNEL,
            standing.staffed_political_districts, standing.required_political_districts).unwrap();
        writeln!(
            out,
            "    Legitimate transition: {} | uninterrupted racket-free days: {} (365 required)",
            standing.legitimate_transition_ready,
            standing
                .days_since_last_racket_activity
                .map_or_else(|| "never operated".to_owned(), |days| days.to_string())
        )
        .unwrap();
        out.push_str("    Each district needs at least three mature profitable businesses across two kinds, plus two different, active, undetained managers: one Neighborhood + Political delegate and one Neighborhood + Finance director. Both need Delegated or Broad autonomy; neither may represent another qualifying district.\n");
        writeln!(out, "    Mature legitimate income after {} bp administrative overhead: {} cents/day realized over 30 days; daily payroll: {} cents; covered: {}",
            standing.portfolio_overhead_basis_points, standing.legitimate_daily_net.cents(), standing.daily_payroll.cents(),
            if standing.legitimate_self_sufficient { "yes" } else { "no" }).unwrap();
        writeln!(out, "    Sustained contested electoral majorities: {} / {} districts (800/1000 local influence, stronger than rivals for 90 days)",
            standing.mayoral_majority_districts, standing.required_political_districts).unwrap();
        writeln!(
            out,
            "    Nomination filed for: {}; latest election: {}, elected: {}.",
            election_date_label(standing.mayoral_nominated_for_day),
            election_date_label(standing.latest_election_day),
            standing.latest_election_won
        )
        .unwrap();
        writeln!(
            out,
            "    Political certification: {}.",
            standing.political_certification.label()
        )
        .unwrap();
        writeln!(out, "    Campaign spending: {} / {} cents; remaining clean reserve: {} / {} cents; leader eligible: {}",
            standing.campaign_spent_cents, standing.required_campaign_spent_cents,
            standing.accounted_net.cents(), standing.required_mayoral_reserve_cents,
            standing.sole_executive_available).unwrap();
        out.push_str("    Machine influence, grassroots voters and institutional support all need maintenance. Local initiatives, manager skills and election-day reputation matter. Use 'campaign', 'campaign-event', 'nominate-mayor' and 'election' for the actual political campaign.\n");
        let local_floor = i128::from(self.registry.upkeep().per_member_daily().cents())
            * i128::from(crimocracy::core::campaign_victory::VICTORY_INCOME_WINDOW_DAYS);
        let districts = districts
            .into_iter()
            .filter(|row| {
                row.active_owned > 0
                    || row.suspended_owned > 0
                    || row.mature_window_net != Money::ZERO
            })
            .collect::<Vec<_>>();
        out.push_str("    Owned political district readiness (own books only):\n");
        if districts.is_empty() {
            out.push_str("      No operating or suspended owned storefronts.\n");
        }
        for district in districts {
            let name = state
                .world()
                .get_neighborhood(district.district)
                .map_or("unknown", |record| record.name());
            let reason = if district.qualifies {
                "locally qualified"
            } else if district.active_owned == 0 {
                "no active storefronts; check 'recovery'"
            } else if district.mature_active
                < crimocracy::core::campaign_victory::POLITICAL_BUSINESSES_PER_DISTRICT
            {
                "needs three storefront titles held for 1080 days (inspect 'political district-N')"
            } else if district.established_active
                < crimocracy::core::campaign_victory::POLITICAL_BUSINESSES_PER_DISTRICT
            {
                "needs three mature storefronts with 20 profitable cycles in 30 days and recent settlements"
            } else if district.established_kinds
                < crimocracy::core::campaign_victory::POLITICAL_BUSINESS_KINDS_PER_DISTRICT
            {
                "needs two different established business kinds"
            } else if !district.economically_qualified {
                "mature net storefront income below one member's 30-day wages"
            } else if district.available_stewards == 0 {
                "needs an independent eligible political district delegate"
            } else if district.available_financial_stewards == 0 {
                "needs a separate eligible financial district director"
            } else if district.steward.is_none() || district.financial_steward.is_none() {
                "no independent complete two-person team in the citywide assignment"
            } else {
                "not locally qualified"
            };
            let steward = district
                .steward
                .map_or("none".to_owned(), |id| format!("char-{}", id.raw()));
            writeln!(
                out,
                "      district-{} {name}: {reason} | {} active, {} suspended, {} mature, {} settlement-qualified, {} kinds | {} political candidates, organizer {steward}, {} finance candidates, director {:?} | mature 30d net {} / {} cents required",
                district.district.raw(),
                district.active_owned,
                district.suspended_owned,
                district.mature_active,
                district.established_active,
                district.established_kinds,
                district.available_stewards,
                district.available_financial_stewards,
                district.financial_steward.map(|manager| format!("char-{}", manager.raw())),
                district.mature_window_net.cents(),
                local_floor
            )
            .unwrap();
        }
        out.push_str("    Inspect 'political district-N' for owned title ages, remaining maturation days and profitable settlements.\n");
        writeln!(out, "  Retirement: {} cents net accounted funds; {} cents required ({} dollars for the leader plus {} days' severance per subordinate)",
            standing.accounted_net.cents(), standing.retirement_total.cents(),
            crimocracy::core::campaign_victory::RETIREMENT_NEST_EGG.cents() / 100,
            crimocracy::core::campaign_victory::RETIREMENT_SEVERANCE_DAYS).unwrap();
        writeln!(out, "    Criminal career: {} / {} profitable racket cycles and {} / {} cents net racket proceeds over {} days; permanent named crew {} / {} required. Passive storefronts alone cannot earn retirement.",
            standing.retirement_profitable_racket_cycles,
            crimocracy::core::campaign_victory::RETIREMENT_MIN_PROFITABLE_RACKET_CYCLES,
            standing.retirement_racket_net.cents(),
            crimocracy::core::campaign_victory::RETIREMENT_MIN_RACKET_NET.cents(),
            crimocracy::core::campaign_victory::RETIREMENT_CRIME_WINDOW_DAYS,
            standing.key_personnel,
            crimocracy::core::campaign_victory::RETIREMENT_MIN_KEY_PERSONNEL).unwrap();
        if standing.contracted_personnel > 0 {
            writeln!(out, "    Retirement blocked: {} open freelance contract(s); finish the terms and obtain lawful release before settling severance (see 'people').", standing.contracted_personnel).unwrap();
        }
        if !standing.sole_executive_available {
            out.push_str("    Retirement blocked: a sole undetained executive must authorize the final settlement.\n");
        }
        writeln!(out, "  Underboss: active rackets in {} districts, {} productive mature districts ({} required); local majority and superior strength relative to rivals required",
            standing.own_active_racket_districts, standing.established_racket_districts,
            standing.required_underboss_districts).unwrap();
        use crimocracy::core::campaign_victory::{
            UNDERBOSS_ASSOCIATES_PER_DISTRICT, UNDERBOSS_CAPTAINS_PER_DISTRICT,
            UNDERBOSS_CONTACT_DAYS, UNDERBOSS_CONTACT_HANDLERS, UNDERBOSS_CONTACT_KINDS,
            UNDERBOSS_ENFORCERS_PER_DISTRICT, UNDERBOSS_LIQUID_RUNWAY_DAYS,
            UNDERBOSS_LOCAL_INSTITUTIONAL_SUPPORT, UNDERBOSS_LOCAL_MACHINE_INFLUENCE,
            UNDERBOSS_OPERATING_RUNWAY_DAYS, UNDERBOSS_RACKET_KINDS_PER_DISTRICT,
            UNDERBOSS_SENIOR_MANAGEMENT_RATING,
        };
        writeln!(out, "    Available named command staff: {} / {} required (permanent crew; {} on payroll, including {} contractors); independent captains in defended districts: {} / {} required ({} proven captains per district, never shared)",
            standing.available_underboss_personnel, standing.required_underboss_personnel, standing.key_personnel + standing.contracted_personnel, standing.contracted_personnel,
            standing.staffed_underboss_districts, standing.required_underboss_districts,
            UNDERBOSS_CAPTAINS_PER_DISTRICT).unwrap();
        writeln!(out, "    Proven associates: {}; uncontested districts with at least {} proven associates across {} racket kinds and more workers than all local rivals combined: {} / {}",
            standing.mature_associates, UNDERBOSS_ASSOCIATES_PER_DISTRICT, UNDERBOSS_RACKET_KINDS_PER_DISTRICT,
            standing.workforce_ready_districts, standing.required_underboss_districts).unwrap();
        writeln!(out, "    Defended mature districts with at least {} available permanent named enforcers each: {} / {}",
            UNDERBOSS_ENFORCERS_PER_DISTRICT, standing.defended_underboss_districts, standing.required_underboss_districts).unwrap();
        writeln!(out, "    Fully backed districts: {} / {} required (same defended and independently commanded districts, with {} machine influence and {} institutional support each; see 'campaign')",
            standing.influential_underboss_districts, standing.required_underboss_districts,
            UNDERBOSS_LOCAL_MACHINE_INFLUENCE, UNDERBOSS_LOCAL_INSTITUTIONAL_SUPPORT).unwrap();
        writeln!(out, "    Senior commanders: {} / 1 required (non-boss delegated manager, Management {}+, two selected direct-report captains in different districts of the same fully backed coalition)",
            standing.senior_underboss_commands, UNDERBOSS_SENIOR_MANAGEMENT_RATING).unwrap();
        writeln!(out, "    Institutional reach: {} / {} contact kinds (legal, political, labor, professional); {} / {} independent handlers; active usable contacts held at least {} days",
            standing.underboss_contact_kinds, UNDERBOSS_CONTACT_KINDS,
            standing.underboss_contact_handlers, UNDERBOSS_CONTACT_HANDLERS, UNDERBOSS_CONTACT_DAYS).unwrap();
        out.push_str("    Citywide, your mature rackets and proven workforce must exceed all rivals combined, with a free executive leader. Rival positions require reconnaissance.\n");
        writeln!(out, "    Mature racket income: {} cents/day realized over 30 days; daily payroll: {} cents; covered: {}",
            standing.racket_daily_net.cents(), standing.daily_payroll.cents(),
            if standing.racket_self_sufficient { "yes" } else { "no" }).unwrap();
        writeln!(out, "    Underboss operating reserve: {} / {} cents ({} days of named payroll plus {} days of current racket costs at maximum modeled heat: {} cents; campaign contracts: {} cents/day, {} cents reserved for same operating runway; net of treasury deficits)",
            standing.underboss_liquid_reserve.cents(),
            standing.required_underboss_liquid_reserve.cents(),
            UNDERBOSS_LIQUID_RUNWAY_DAYS, UNDERBOSS_OPERATING_RUNWAY_DAYS, standing.underboss_operating_buffer.cents(),
            standing.campaign_daily_payroll.cents(), standing.underboss_campaign_buffer.cents()).unwrap();
        out.push_str("    Both route income figures deduct recent net losses from ventures that are not eligible to contribute positive mature earnings, even after closure or title transfer.\n");
        out.push_str("    An independent captain must be an undetained non-executive with Delegated or Broad autonomy and direct 20 profitable settlements in 30 days. Proven associate strength is the largest staffing complement demonstrated in at least 20 such settlements, capped by current staffing; extra hires need their own operating proof. Named personnel lead rank-and-file workers, whose compensation is paid through racket operating costs.\n");
        writeln!(out, "  Underboss rackets require 90 days of operation; mayoral businesses require 1080 days of ownership. Both require 20 profitable settlements in the last 30 days, a settlement within 7 days, and 30-day net income sufficient for one crew member's daily wage ({} cents/day).",
            self.registry.upkeep().per_member_daily().cents()).unwrap();
        out.push_str("Reaching a threshold does not end sandbox play until you claim the victory.");
        Ok(out)
    }
}
