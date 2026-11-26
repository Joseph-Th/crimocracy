//! Scheduled elections and deterministic integer ballot apportionment.

use super::{eligible_candidate, rival_pressure};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, is_day_boundary};
use crate::politics::{
    DistrictCampaign, MUNICIPAL_ELECTION_INTERVAL_DAYS, MayoralElection, OrganizationCampaign,
    WardBallot,
};
use crate::registry::Registry;
use crate::reputation::AudienceKind;
use crate::reputation::reputation_system::resolve_score;
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

/// Proportional integer ballots using largest remainders. Truncated divisions cannot
/// silently give all unallocated votes to the final competitor (including one with
/// no support). Equal remainders favor stronger candidates, then the stable ballot
/// order. Every vote is assigned once and zero-score contenders receive none.
fn apportion_ward_votes(electorate: u32, scores: [u32; 3]) -> [u32; 3] {
    let total: u64 = scores.iter().map(|&score| u64::from(score)).sum();
    // The incumbent always has a positive base score in an actual election.
    assert!(
        total > 0,
        "a ward ballot requires at least one supported contender"
    );
    let weighted = scores.map(|score| u64::from(electorate) * u64::from(score));
    let mut votes = weighted.map(|count| (count / total) as u32);
    let mut remainders = weighted.map(|count| count % total);
    let remaining = electorate - votes.iter().sum::<u32>();
    for _ in 0..remaining {
        let index = (0..scores.len())
            .max_by_key(|&index| (remainders[index], scores[index], std::cmp::Reverse(index)))
            .expect("a ward always has three ballot contenders");
        votes[index] += 1;
        remainders[index] = 0;
    }
    votes
}

pub(crate) fn apply_due_mayoral_elections(registry: &Registry, state: &mut AppState) {
    if !is_day_boundary(state.now()) {
        return;
    }
    let day = state.now().as_minutes() / DAY_MINUTES;
    if !day.is_multiple_of(MUNICIPAL_ELECTION_INTERVAL_DAYS) {
        return;
    }
    let Some(owner) = state.player_organization() else {
        return;
    };
    let Some(candidacy) = state
        .politics()
        .organization(owner)
        .and_then(OrganizationCampaign::candidacy)
        .filter(|entry| entry.election_day == day)
    else {
        return;
    };

    // Ballots use only current world state, the real campaign ledger, local
    // support and crime reputation. There is no preselected winning candidate.
    let incumbent = state
        .world()
        .organizations()
        .filter(|organization| organization.kind() == OrganizationKind::Political)
        .flat_map(|organization| state.world().characters_in_organization(organization.id()))
        .filter(|person| {
            // The filed nominee is a separate historical ballot participant even
            // if they subsequently leave the crew and join a council.
            person.id() != candidacy.candidate
                && person.supervisor().is_none()
                && state
                    .legal()
                    .active_arrest_for_character(person.id())
                    .is_none()
        })
        .max_by_key(|person| {
            let negotiating = person
                .capability(CapabilityKind::Negotiation)
                .map_or(0, |rating| rating.value());
            let access = person
                .capability(CapabilityKind::SocialAccess)
                .map_or(0, |rating| rating.value());
            (
                u32::from(negotiating)
                    + u32::from(access)
                    + u32::from(person.life().professional_standing()),
                std::cmp::Reverse(person.id()),
            )
        });
    let incumbent_id = incumbent.map(|person| person.id());
    let incumbent_strength = incumbent.map_or(175_u32, |person| {
        u32::from(
            person
                .capability(CapabilityKind::Negotiation)
                .map_or(0, |rating| rating.value()),
        ) + u32::from(
            person
                .capability(CapabilityKind::SocialAccess)
                .map_or(0, |rating| rating.value()),
        ) + u32::from(person.life().professional_standing())
    });
    let nominee = state
        .world()
        .get_character(candidacy.candidate)
        .expect("a filed mayoral candidate still exists");
    let nominee_strength = u32::from(
        nominee
            .capability(CapabilityKind::Negotiation)
            .map_or(0, |rating| rating.value()),
    ) / 2
        + u32::from(nominee.life().professional_standing()) / 2;
    let fear = resolve_score(registry, state.reputation(), owner, AudienceKind::Residents);
    let excess_fear = u32::from(fear.saturating_sub(registry.reputation().baseline()));
    let mut ballots = Vec::new();
    for ward in state.world().neighborhoods() {
        let district = ward.id();
        let office = state
            .politics()
            .organization(owner)
            .and_then(|campaign| campaign.district(district));
        let machine = u32::from(office.map_or(0, DistrictCampaign::influence));
        let grassroots = u32::from(office.map_or(0, DistrictCampaign::grassroots));
        let institutions = u32::from(office.map_or(0, DistrictCampaign::institutions));
        let local_rackets = state
            .enterprises()
            .enterprises_for_organization(owner)
            .filter(|racket| racket.status() == crate::enterprises::EnterpriseStatus::Active)
            .filter(|racket| match racket.location() {
                crate::enterprises::EnterpriseLocation::Neighborhood(id) => id == district,
                crate::enterprises::EnterpriseLocation::Business(id) => state
                    .world()
                    .get_business(id)
                    .is_some_and(|business| business.neighborhood() == district),
            })
            .count() as u32;
        let candidate_score =
            (machine * 50 + grassroots * 30 + institutions * 20) / 100 + nominee_strength;
        let candidate_score =
            candidate_score.saturating_sub(excess_fear * 5 + local_rackets.saturating_mul(25));
        let incumbent_score = 320_u32
            + incumbent_strength
            + u32::from(ward.profile().institutions.police_presence.value()) / 2;
        let rival_score = u32::from(rival_pressure(state, owner, district)) * 3 / 5;
        let electorate = 5_000_u32
            + u32::from(ward.profile().economy.wealth.value()) * 30
            + u32::from(ward.profile().economy.commercial_activity.value()) * 20;
        let [candidate_votes, incumbent_votes, rival_votes] =
            apportion_ward_votes(electorate, [candidate_score, incumbent_score, rival_score]);
        ballots.push(WardBallot {
            district,
            candidate_votes,
            incumbent_votes,
            rival_votes,
            ward_won: false,
        });
    }
    // Vote totals do not depend on the assignment of qualification directors.
    // Solve the complete citywide coalition once, prioritizing wards that won
    // their vote and meet the local readiness requirements.
    let ballot_winners = ballots
        .iter()
        .filter(|ward| {
            ward.candidate_votes > ward.incumbent_votes && ward.candidate_votes > ward.rival_votes
        })
        .map(|ward| ward.district)
        .collect::<BTreeSet<_>>();
    let Ok((progress, districts)) =
        crate::core::campaign_victory::ballot_progress(registry, state, &ballot_winners)
    else {
        return;
    };
    let mut shortfalls = crate::core::campaign_victory::mayoral_ballot_shortfalls(&progress);
    if !eligible_candidate(state, owner, candidacy.candidate)
        && progress.mayoral_leader_eligible
        && progress.established_political_contacts > 0
    {
        shortfalls.push("filed nominee no longer holds eligible executive authority".to_owned());
    }
    let eligible = shortfalls.is_empty();
    for ward in &mut ballots {
        ward.ward_won = ballot_winners.contains(&ward.district)
            && districts
                .iter()
                .any(|entry| entry.district == ward.district && entry.mayoral_ready);
    }
    let wards_won = ballots.iter().filter(|ballot| ballot.ward_won).count();
    let elected = eligible && wards_won >= progress.required_political_districts;
    let result = MayoralElection {
        candidate: candidacy.candidate,
        incumbent: incumbent_id,
        election_day: day,
        held_at: state.now(),
        wards_needed: progress.required_political_districts,
        wards_won,
        eligible,
        elected,
        ballots,
    };
    let campaign = state
        .politics
        .organizations
        .get_mut(&owner)
        .expect("nominated candidate has a campaign");
    campaign.candidacy = None;
    campaign.elections.push(result);

    // The public election notice is an ordinary Standing report; the full
    // verified ballot ledger remains inspectable even if report IDs run out.
    let ballot_at = state.now();
    let summary = if elected {
        format!(
            "Municipal election {ballot_at}: the nominated leader won {wards_won} of {} wards and is mayor-elect. Certify with 'claim-victory political'.",
            progress.total_districts
        )
    } else {
        let reason = if eligible {
            "Too few qualifying wards won the ballot.".to_owned()
        } else {
            format!("Citywide qualification failed: {}.", shortfalls.join("; "))
        };
        format!(
            "Municipal election {ballot_at}: the nominated leader won {wards_won} qualifying wards out of {} city wards; {} needed. {reason} The mayoral campaign was defeated; another election can be contested.",
            progress.total_districts, progress.required_political_districts
        )
    };
    let report = crate::reports::report_system::validate_record_report(
        state,
        crate::reports::ReportDraft {
            recipient: owner,
            kind: crate::reports::ReportKind::Standing,
            title: format!("Municipal election {ballot_at}"),
            entries: vec![crate::reports::ReportEntry {
                attention: if elected {
                    crate::core::attention::AttentionClass::Crisis
                } else {
                    crate::core::attention::AttentionClass::Notable
                },
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([crate::core::entity::EntityRef::Character(
                    candidacy.candidate,
                )]),
                decision: None,
            }],
        },
    );
    if let Ok(report) = report {
        let _ = report.commit(state);
    }
}

#[cfg(test)]
mod ballot_apportionment_tests {
    use super::apportion_ward_votes;

    #[test]
    fn unsupported_contenders_receive_no_rounding_votes() {
        assert_eq!(apportion_ward_votes(11, [2, 3, 0]), [4, 7, 0]);
        assert_eq!(apportion_ward_votes(9, [0, 4, 0]), [0, 9, 0]);
        assert_eq!(apportion_ward_votes(1, [0, 0, 8]), [0, 0, 1]);
    }

    #[test]
    fn proportional_rounding_allocates_every_vote_and_breaks_ties_stably() {
        assert_eq!(apportion_ward_votes(5, [1, 1, 1]), [2, 2, 1]);
        assert_eq!(apportion_ward_votes(1, [3, 3, 0]), [1, 0, 0]);
        assert_eq!(
            apportion_ward_votes(u32::MAX, [1, 1, 0]),
            [u32::MAX / 2 + 1, u32::MAX / 2, 0]
        );
        for electorate in 0..=100 {
            for scores in [[0, 1, 0], [1, 1, 0], [1, 2, 3], [97, 0, 4]] {
                let votes = apportion_ward_votes(electorate, scores);
                assert_eq!(votes.iter().sum::<u32>(), electorate);
                for index in 0..scores.len() {
                    if scores[index] == 0 {
                        assert_eq!(votes[index], 0);
                    }
                }
            }
        }
    }
}
