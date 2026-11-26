//! Victory certification reports: persisted election evidence and explicitly current context.

use super::{
    POLITICAL_CONTACT_DAYS, PoliticalDistrictProgress, RETIREMENT_CRIME_WINDOW_DAYS,
    VICTORY_INCOME_WINDOW_DAYS, VictoryError, VictoryPath, VictoryProgress, established_for,
};
use crate::contacts::ContactKind;
use crate::contacts::contact_system::usable_contacts_of_kind;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::OrganizationId;
use crate::core::state::AppState;
use crate::reports::report_system::{ValidatedReport, validate_report_draft};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn prepare_victory_report(
    state: &AppState,
    owner: OrganizationId,
    path: VictoryPath,
    standing: &VictoryProgress,
    districts: &[PoliticalDistrictProgress],
) -> Result<ValidatedReport, VictoryError> {
    let mut entries = vec![ReportEntry {
        attention: AttentionClass::Crisis,
        summary: match path {
            VictoryPath::PoliticalPower => {
                let election = state
                    .politics()
                    .organization(owner)
                    .and_then(|campaign| campaign.latest_election())
                    .filter(|election| election.elected)
                    .ok_or(VictoryError::PoliticalNotReady)?;
                let candidate = state
                    .world()
                    .get_character(election.candidate)
                    .ok_or(VictoryError::PoliticalNotReady)?;
                format!(
                    "{} was elected mayor on {} after winning {} independently organized districts ({} required). At certification, {} districts currently meet the coalition requirements; established legitimate income is {} cents/day against {} cents/day in named-member wages.",
                    candidate.name(),
                    election.held_at,
                    election.wards_won,
                    election.wards_needed,
                    standing.mayoral_majority_districts,
                    standing.legitimate_daily_net.cents(),
                    standing.daily_payroll.cents(),
                )
            }
            VictoryPath::FundedRetirement => format!(
                "The leader retired after {} profitable racket settlements and {} cents in net criminal proceeds over the last {} days, with a funded nest egg; each subordinate received 180 days of wages in an accounted personal account.",
                standing.retirement_profitable_racket_cycles,
                standing.retirement_racket_net.cents(),
                RETIREMENT_CRIME_WINDOW_DAYS,
            ),
            VictoryPath::CityUnderboss => format!(
                "City underboss: {} independently commanded, defended and institutionally backed mature districts, {} proven associates, {} available named personnel, {} established institutional channel types and {} cents of liquid reserves. The organization demonstrated a durable citywide command, not just profitable fronts.",
                standing.influential_underboss_districts,
                standing.mature_associates,
                standing.available_underboss_personnel,
                standing.underboss_contact_kinds,
                standing.underboss_liquid_reserve.cents(),
            ),
        },
        sources: Vec::new(),
        entities: BTreeSet::from([EntityRef::Organization(owner)]),
        decision: None,
    }];
    if path == VictoryPath::PoliticalPower {
        let election = state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.latest_election())
            .expect("eligible political certification has a recorded ballot");
        entries[0]
            .entities
            .insert(EntityRef::Character(election.candidate));
        if let Some(sponsor) =
            usable_contacts_of_kind(state, owner, ContactKind::Political).find(|contact| {
                established_for(
                    state.now(),
                    contact.established_at(),
                    POLITICAL_CONTACT_DAYS,
                )
            })
        {
            let contact = state
                .world()
                .get_character(sponsor.contact())
                .expect("established institutional contact has a real person");
            entries.push(ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!(
                    "Political sponsor: {} (contact {}), currently usable and established at least {} days ago.",
                    contact.name(), sponsor.id(), POLITICAL_CONTACT_DAYS,
                ),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::Character(contact.id())]),
                decision: None,
            });
        }
        let current: BTreeMap<_, _> = districts
            .iter()
            .map(|district| (district.district, district))
            .collect();
        for ward in election.ballots.iter().filter(|ward| ward.ward_won) {
            let name = state
                .world()
                .get_neighborhood(ward.district)
                .expect("a certified ward exists")
                .name();
            let mut entities = BTreeSet::from([EntityRef::Neighborhood(ward.district)]);
            let mut summary = format!(
                "{}: winning ballot on {} ({} nominee, {} incumbent, {} rival votes).",
                name,
                election.held_at,
                ward.candidate_votes,
                ward.incumbent_votes,
                ward.rival_votes,
            );
            if let Some(district) = current
                .get(&ward.district)
                .filter(|district| district.mayoral_ready)
            {
                let organizer = district.steward.expect("qualifying district has organizer");
                let manager = state
                    .world()
                    .get_character(organizer)
                    .expect("assigned political organizer exists");
                entities.insert(EntityRef::Character(organizer));
                summary.push_str(&format!(
                    " At certification, {} coordinates {} established business sectors with {} cents in mature local earnings over {} days and a sustained majority.",
                    manager.name(), district.established_kinds, district.mature_window_net.cents(),
                    VICTORY_INCOME_WINDOW_DAYS,
                ));
            } else {
                summary.push_str(" At certification, this ward no longer meets the current coalition requirements.");
            }
            entries.push(ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities,
                decision: None,
            });
        }
    }
    Ok(validate_report_draft(
        state,
        ReportDraft {
            recipient: owner,
            kind: ReportKind::CampaignVictory,
            title: format!("Campaign victory: {}", path.label()),
            entries,
        },
    )?)
}
