//! Player recruitment routes and explicit orders through the canonical recruitment owner.

use super::*;
use crimocracy::delegation::delegation_system::resolve_policy_for_manager;
use crimocracy::recruitment::{
    reconsider_recruitment_pitch, recruitment_system::recent_recruitment_approval_refusals,
};
use crimocracy::world::{AutonomyLevel, CharacterRecord};

/// The recruitment owner has established an existing personal acquaintance. Show
/// its vocational profile, not private motives, numeric relationship scores or
/// anticipated acceptance odds.
fn write_prospect_skills(out: &mut String, person: &CharacterRecord) {
    let skills = ALL_CAPABILITY_KINDS
        .into_iter()
        .filter_map(|kind| person.capability(kind).map(|_| format!("{kind:?}")))
        .collect::<Vec<_>>();
    if !skills.is_empty() {
        writeln!(out, "  Familiar skills: {}", skills.join(", ")).unwrap();
    }
}

impl Session<'_> {
    pub(super) fn player_prospects(&self) -> Result<BTreeMap<CharacterId, CharacterId>, String> {
        let mut routes = BTreeMap::new();
        for leader in self.state.world().characters_in_organization(self.player) {
            if leader.supervisor().is_some()
                || self
                    .state
                    .legal()
                    .active_arrest_for_character(leader.id())
                    .is_some()
            {
                continue;
            }
            for candidate in
                find_recruitment_candidates(self.registry, &self.state, self.player, leader.id())
                    .map_err(|e| e.to_string())?
            {
                routes
                    .entry(candidate)
                    .and_modify(|current: &mut CharacterId| *current = (*current).min(leader.id()))
                    .or_insert(leader.id());
            }
        }
        Ok(routes)
    }

    pub(super) fn manager_prospects(&self) -> Result<Vec<(CharacterId, Vec<CharacterId>)>, String> {
        self.state
            .delegation()
            .active_for_scope(ResponsibilityScope::Function(
                ResponsibilityFunction::Personnel,
            ))
            .filter(|mandate| mandate.organization() == self.player)
            .filter(|mandate| {
                self.state
                    .legal()
                    .active_arrest_for_character(mandate.manager())
                    .is_none()
            })
            .map(|mandate| {
                let manager = mandate.manager();
                let candidates =
                    find_recruitment_candidates(self.registry, &self.state, self.player, manager)
                        .map_err(|error| error.to_string())?;
                Ok((manager, candidates))
            })
            .collect()
    }

    pub(super) fn execute_recruitment(&mut self, words: &[&str]) -> Result<String, String> {
        let state = &self.state;
        let mut out = String::new();
        match words[0] {
            "prospects" => {
                let prospects = self.player_prospects()?;
                for (id, recruiter) in prospects {
                    let person = state
                        .world()
                        .get_character(id)
                        .ok_or("prospect disappeared")?;
                    let leader = state
                        .world()
                        .get_character(recruiter)
                        .ok_or("executive disappeared")?;
                    writeln!(
                        out,
                        "{} {} | established connection to leadership via {} (char-{})",
                        id.raw(),
                        person.name(),
                        leader.name(),
                        recruiter.raw()
                    )
                    .unwrap();
                    write_prospect_skills(&mut out, person);
                }
                let mut approval_refusals = None;
                let roster = state
                    .world()
                    .characters_in_organization(self.player)
                    .count();
                for (manager, candidates) in self.manager_prospects()? {
                    let member = state
                        .world()
                        .get_character(manager)
                        .ok_or("manager disappeared")?;
                    let approval = resolve_policy_for_manager(
                        state,
                        manager,
                        PolicyKind::IndependentRecruitment,
                    )
                    .map_err(|error| error.to_string())?
                    .independent_recruitment_approval();
                    let blocker = match approval.recruitment_limit() {
                        Some(0) => Some("independent recruitment paused by effective policy"),
                        Some(limit) if roster >= usize::from(limit) => {
                            Some("independent recruitment at named roster cap")
                        }
                        _ if !matches!(
                            member.autonomy(),
                            AutonomyLevel::Delegated | AutonomyLevel::Broad
                        ) =>
                        {
                            Some("requires delegated or broad autonomy for independent recruitment")
                        }
                        _ => None,
                    };
                    for candidate in candidates {
                        let leadership_declined = blocker.is_none()
                            && approval == ApprovalPolicy::RequireApproval
                            && approval_refusals
                                .get_or_insert_with(|| {
                                    recent_recruitment_approval_refusals(self.registry, state)
                                })
                                .contains(&(self.player, candidate));
                        let status = if let Some(blocker) = blocker {
                            blocker
                        } else if leadership_declined {
                            "leadership declined the approach; manager will reconsider after recruitment cooldown"
                        } else if reconsider_recruitment_pitch(
                            self.registry.recruitment(),
                            state,
                            self.player,
                            member,
                            candidate,
                        )
                        .is_none()
                        {
                            "all recent pitches were refused; manager is waiting for a changed relationship or an older refusal to expire"
                        } else {
                            "may act at the daily recruitment pass, subject to policy"
                        };
                        let person = state
                            .world()
                            .get_character(candidate)
                            .ok_or("prospect disappeared")?;
                        writeln!(
                            out,
                            "{} {} | known to manager {} (char-{}); {status}",
                            candidate.raw(),
                            person.name(),
                            member.name(),
                            manager.raw()
                        )
                        .unwrap();
                        write_prospect_skills(&mut out, person);
                    }
                }
                if out.is_empty() {
                    out.push_str("No currently approachable contacts.");
                }
            }
            "recruit" => {
                let raw = parse_number(words[1], "char")?;
                let (candidate, recruiter) = self
                    .player_prospects()?
                    .into_iter()
                    .find(|(id, _)| id.raw() == raw)
                    .ok_or("candidate is not a currently approachable contact")?;
                let approach = match words.get(2).copied().unwrap_or("appeal") {
                    "offer" => RecruitmentApproach::FinancialOpportunity,
                    "advancement" => RecruitmentApproach::Advancement,
                    "protection" => RecruitmentApproach::Protection,
                    "appeal" => RecruitmentApproach::PersonalAppeal,
                    _ => {
                        return Err(
                            "recruit <character-id> [offer|advancement|protection|appeal]"
                                .to_owned(),
                        );
                    }
                };
                let attempt = validate_recruitment_attempt(
                    self.registry,
                    &self.state,
                    RecruitmentDraft {
                        target_organization: self.player,
                        recruiter,
                        candidate,
                        approach,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                let record = self
                    .state
                    .recruitment()
                    .get_attempt(attempt)
                    .ok_or("recruitment attempt missing")?;
                writeln!(
                    out,
                    "Recruitment attempt {}: {:?}.",
                    attempt.raw(),
                    record.outcome()
                )
                .unwrap();
            }
            _ => unreachable!("command routed to wrong recruitment handler"),
        }
        Ok(out)
    }
}
