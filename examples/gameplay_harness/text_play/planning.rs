//! Registry-directed player operation orders.
//!
//! The client selects a known or public target, crew, approach, timing and
//! contingencies. Objective family, role, target, booking and intelligence
//! validation remains with the production operation owner. The client cannot
//! learn hidden ownership, economic status, or law-enforcement records by
//! naming a public address or guessing an administrative ID. Foreign-business
//! actions require target-specific organization-held information; recon orders
//! gather it by actually running an operation in simulated time.

use super::*;

fn normalized(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Clone, Copy)]
pub(super) enum PoliceMode {
    Press,
    Ask,
    Abort,
}

impl PoliceMode {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        match normalized(raw).as_str() {
            "press" => Ok(Self::Press),
            "ask" => Ok(Self::Ask),
            "abort" => Ok(Self::Abort),
            _ => Err("police must be press, ask, or abort".to_owned()),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Ask => "ask",
            Self::Abort => "abort",
        }
    }

    pub(super) fn contingencies(self) -> Vec<OperationContingency> {
        match self {
            Self::Press => Vec::new(),
            Self::Ask => vec![OperationContingency::RequestDecisionOnPoliceArrival],
            Self::Abort => vec![
                OperationContingency::AbortOnPoliceArrivalBeforeEntry,
                OperationContingency::RequestDecisionOnPoliceArrival,
            ],
        }
    }
}

fn parse_operation_kind(raw: &str) -> Result<OperationKind, String> {
    let key = normalized(raw);
    ALL_OPERATION_KINDS
        .into_iter()
        .find(|kind| normalized(&format!("{kind:?}")) == key)
        .ok_or_else(|| format!("unknown operation kind '{raw}'; use 'operation-kinds'"))
}

#[derive(Clone)]
struct StaffMatch {
    total_score: i64,
    // An occupied role stores a real member; unfilled slots remain None.
    by_role: Vec<Option<CharacterId>>,
}

/// Maximum-weight one-person-per-role assignment over the *entire* required
/// crew, not a sequence of locally greedy hires. Each member can take at most
/// one role. With at most nine authored RoleKind values, the bitmask DP is
/// bounded by 512 masks per current member and uses no random tie-breaking.
///
/// Scores are authored by the caller from trained capabilities and its
/// supervisory preference; None means the candidate is untrained for that
/// role. Equal-score crews break ties by lowest role-ordered character IDs.
pub(super) fn optimal_staff_match(
    required_role_count: usize,
    candidate_scores: &[(CharacterId, Vec<Option<i32>>)],
) -> Option<Vec<CharacterId>> {
    if required_role_count == 0 {
        return Some(Vec::new());
    }
    if required_role_count > 9 {
        return None;
    }
    let size = 1 << required_role_count;
    let mut matches: Vec<Option<StaffMatch>> = vec![None; size];
    matches[0] = Some(StaffMatch {
        total_score: 0,
        by_role: vec![None; required_role_count],
    });
    for (person, scores) in candidate_scores {
        let mut next = matches.clone();
        for (mask, current) in matches.iter().enumerate() {
            let Some(current) = current else {
                continue;
            };
            for role in 0..required_role_count {
                if mask & (1 << role) != 0 {
                    continue;
                }
                let Some(score) = scores.get(role).and_then(|score| *score) else {
                    continue;
                };
                let next_mask = mask | (1 << role);
                let mut choice = current.clone();
                choice.total_score += i64::from(score);
                choice.by_role[role] = Some(*person);
                let better = match &next[next_mask] {
                    None => true,
                    Some(other) => {
                        choice.total_score > other.total_score
                            || (choice.total_score == other.total_score
                                && choice.by_role < other.by_role)
                    }
                };
                if better {
                    next[next_mask] = Some(choice);
                }
            }
        }
        matches = next;
    }
    matches[size - 1].take().map(|match_| {
        match_
            .by_role
            .into_iter()
            .map(|member| member.expect("complete role mask must have a member in every slot"))
            .collect()
    })
}

impl Session<'_> {
    fn operation_kinds(&self) -> String {
        let mut output = String::new();
        for kind in ALL_OPERATION_KINDS {
            let definition = self.registry.get_operation(kind);
            let approaches = definition
                .supported_approaches()
                .iter()
                .map(|approach| format!("{approach:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let roles = definition
                .required_roles()
                .iter()
                .map(|role| {
                    let skill = definition
                        .execution()
                        .capability_for_role(*role)
                        .map_or("Unspecified".to_owned(), |skill| format!("{skill:?}"));
                    format!("{role:?} ({skill})")
                })
                .collect::<Vec<_>>()
                .join(", ");
            let topics = definition
                .execution()
                .relevant_intelligence_topics()
                .iter()
                .map(|topic| format!("{topic:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let target = if kind == OperationKind::Burglary {
                "use 'burgle' to consume a known lead"
            } else if kind == OperationKind::Extraction || kind == OperationKind::WitnessPressure {
                "known char-N with learned legal-status basis"
            } else if matches!(
                kind,
                OperationKind::Surveillance | OperationKind::Infiltration
            ) {
                "public business-N/district-N; known char-N/org-N/operation-N/investigation-N/enterprise-N"
            } else {
                "business target"
            };
            writeln!(
                output,
                "{kind:?} | roles: {roles} | approaches: {approaches} | intelligence: {topics} | {}m | {target}",
                definition.execution().duration().as_minutes()
            )
            .unwrap();
        }
        output
    }

    /// Match the whole crew for maximum trained capability and a modest
    /// subordinate preference. Never appoint an untrained member silently;
    /// the player must explicitly accept that risk with a role assignment.
    pub(super) fn select_operation_roles(
        &self,
        kind: OperationKind,
        explicit: &BTreeMap<RoleKind, CharacterId>,
        at: crimocracy::core::time::SimTime,
        constraints: &[OperationConstraint],
    ) -> Result<BTreeMap<RoleKind, CharacterId>, String> {
        let definition = self.registry.get_operation(kind);
        let roster: Vec<_> = self
            .state
            .world()
            .characters_in_organization(self.player)
            .filter(|person| {
                participant_available_for_plan(
                    self.registry,
                    &self.state,
                    person.id(),
                    kind,
                    at,
                    constraints,
                )
            })
            .collect();
        let used: BTreeSet<_> = explicit.values().copied().collect();
        if used.len() != explicit.len() {
            return Err("the same character cannot fill two operation roles".to_owned());
        }
        let mut roles = explicit.clone();
        let missing = definition
            .required_roles()
            .iter()
            .filter(|role| !roles.contains_key(role))
            .copied()
            .collect::<Vec<_>>();
        let capabilities = missing
            .iter()
            .map(|role| {
                definition
                    .execution()
                    .capability_for_role(*role)
                    .ok_or("operation role has no authored capability")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let candidate_scores = roster
            .iter()
            .filter(|member| !used.contains(&member.id()))
            .map(|member| {
                let scores = capabilities
                    .iter()
                    .map(|skill| {
                        member
                            .capability(*skill)
                            .filter(|rating| rating.value() > 0)
                            .map(|rating| {
                                i32::from(rating.value()) * 10
                                    + i32::from(member.supervisor().is_some()) * 120
                            })
                    })
                    .collect();
                (member.id(), scores)
            })
            .collect::<Vec<_>>();
        let picked = optimal_staff_match(missing.len(), &candidate_scores).ok_or_else(|| {
            let untrained = missing.iter().zip(&capabilities).find(|(_, skill)| {
                !roster.iter().any(|member| !used.contains(&member.id())
                    && member.capability(**skill).is_some_and(|rating| rating.value() > 0))
            });
            if let Some((role, skill)) = untrained {
                format!(
                    "no available unassigned member has trained {skill:?} for {role:?}; use {role:?}=char-N to choose an untrained member explicitly, reschedule, or recruit a specialist"
                )
            } else {
                "not enough distinct trained people to cover all roles; explicitly choose a member for a role or recruit specialists".to_owned()
            }
        })?;
        for (role, member) in missing.into_iter().zip(picked) {
            roles.insert(role, member);
        }
        Ok(roles)
    }

    pub(super) fn execute_planning(&mut self, words: &[&str]) -> Result<String, String> {
        if words[0] == "operation-kinds" {
            return Ok(self.operation_kinds());
        }
        if words[0] == "known-targets" {
            return Ok(self.known_surveillance_targets());
        }
        let kind = parse_operation_kind(words[1])?;
        if kind == OperationKind::Burglary {
            return Err(
                "use 'burgle <lead-id>' to authorize and convert a known burglary lead".to_owned(),
            );
        }
        let person_target = matches!(
            kind,
            OperationKind::Extraction | OperationKind::WitnessPressure
        );
        let recon = matches!(
            kind,
            OperationKind::Surveillance | OperationKind::Infiltration
        );
        let (target, name, _) = if person_target {
            if !words[2].starts_with("char-") {
                return Err(
                    "legal-person operations require an identified char-N from 'known-people'"
                        .to_owned(),
                );
            }
            let raw = parse_number(words[2], "char")?;
            let character = self.known_character(raw).ok_or(
                "character is not an organization member or identified by its intelligence",
            )?;
            let name = self
                .state
                .world()
                .get_character(character)
                .ok_or("known character disappeared")?
                .name()
                .to_owned();
            (EntityRef::Character(character), name, None)
        } else if recon {
            self.reconnaissance_target(words[2])?
        } else {
            let raw = parse_number(words[2], "business")?;
            let business = self
                .known_business(raw)
                .ok_or("business is not listed in the public directory")?;
            let target = self
                .state
                .world()
                .get_business(business)
                .ok_or("business disappeared")?;
            let owned = self
                .state
                .world()
                .businesses_owned_by_organization(self.player)
                .any(|record| record.id() == business);
            // A public address is not an operational opening. Foreign targets
            // require organization-held information before authorization can
            // query the owner's hidden economy or ownership dependencies.
            if !owned
                && !self
                    .organization_information()
                    .any(|info| info.subject() == EntityRef::Business(business))
            {
                return Err(
                    "no organization-held target information; scout this business first".to_owned(),
                );
            }
            (
                EntityRef::Business(business),
                target.name().to_owned(),
                Some(target.neighborhood()),
            )
        };
        let definition = self.registry.get_operation(kind);
        let mut approach = definition
            .supported_approaches()
            .get(&OperationApproach::Covert)
            .copied()
            .or_else(|| definition.supported_approaches().iter().next().copied())
            .ok_or("operation kind has no available approach")?;
        let mut approach_specified = false;
        let mut requested_leader = None;
        let mut explicit = BTreeMap::new();
        let mut requested_delay = None;
        let mut requested_deadline = None;
        let mut police_mode = None;
        let mut required_topics = BTreeSet::new();
        for assignment in &words[3..] {
            let (key, value) = assignment
                .split_once('=')
                .ok_or("plan options must be key=value, e.g. approach=covert, police=ask, delay=2h or Role=char-N")?;
            let key = normalized(key);
            if key == "approach" {
                if approach_specified {
                    return Err("approach may only be specified once".to_owned());
                }
                approach_specified = true;
                let value = normalized(value);
                approach = definition
                    .supported_approaches()
                    .iter()
                    .copied()
                    .find(|candidate| normalized(&format!("{candidate:?}")) == value)
                    .ok_or("unsupported approach; see 'operation-kinds'")?;
                continue;
            }
            if key == "police" {
                let response = PoliceMode::parse(value)?;
                if police_mode.replace(response).is_some() {
                    return Err("police response may only be specified once".to_owned());
                }
                continue;
            }
            if key == "delay" {
                let minutes = parse_advance(value)
                    .ok()
                    .filter(|minutes| *minutes <= DAY_MINUTES * 7)
                    .ok_or_else(|| "delay must be 1m to 7d (e.g. delay=2h)".to_owned())?;
                if requested_delay.replace(minutes).is_some() {
                    return Err("delay may only be specified once".to_owned());
                }
                continue;
            }
            if key == "deadline" {
                let minutes = parse_advance(value)
                    .ok()
                    .filter(|minutes| *minutes <= DAY_MINUTES * 7)
                    .ok_or_else(|| "deadline must be 1m to 7d after the current time".to_owned())?;
                if requested_deadline.replace(minutes).is_some() {
                    return Err("deadline may only be specified once".to_owned());
                }
                continue;
            }
            if key == "need" {
                let topic = definition
                    .execution()
                    .relevant_intelligence_topics()
                    .iter().copied()
                    .find(|topic| normalized(&format!("{topic:?}")) == normalized(value))
                    .ok_or("required intelligence must be an operation-relevant topic; see 'intelligence'")?;
                if !required_topics.insert(topic) {
                    return Err("an intelligence topic may only be required once".to_owned());
                }
                continue;
            }
            let role = if key == "leader" {
                None
            } else {
                Some(
                    *definition
                        .required_roles()
                        .iter()
                        .find(|role| normalized(&format!("{role:?}")) == key)
                        .ok_or_else(|| format!("role '{key}' is not required for {kind:?}"))?,
                )
            };
            let id = parse_number(value, "char")?;
            let participant = self
                .state
                .world()
                .characters_in_organization(self.player)
                .find(|person| person.id().raw() == id)
                .ok_or_else(|| format!("character {id} is not in your organization"))?
                .id();
            if key == "leader" {
                if requested_leader.replace(participant).is_some() {
                    return Err("leader may only be assigned once".to_owned());
                }
                continue;
            }
            let role = role.ok_or("missing operation role")?;
            if explicit.insert(role, participant).is_some() {
                return Err(format!("role {role:?} was assigned more than once"));
            }
        }
        let at = self
            .state
            .now()
            .checked_add(SimDuration::from_minutes(
                u32::try_from(requested_delay.unwrap_or(1))
                    .expect("command delay is at most seven days"),
            ))
            .ok_or("simulation clock exhausted")?;
        let mut constraints = required_topics
            .into_iter()
            .map(OperationConstraint::RequireIntelligenceTopic)
            .collect::<Vec<_>>();
        if let Some(minutes) = requested_deadline {
            let deadline = self
                .state
                .now()
                .checked_add(SimDuration::from_minutes(
                    u32::try_from(minutes).expect("command deadline is at most seven days"),
                ))
                .ok_or("deadline exceeds the simulation clock")?;
            constraints.push(OperationConstraint::CompleteBy(deadline));
        }
        let contingencies = police_mode.unwrap_or(PoliceMode::Press).contingencies();
        let objective = kind
            .objective_for_target(target)
            .ok_or("this operation requires a different target type")?;
        let roles = self.select_operation_roles(kind, &explicit, at, &constraints)?;
        let leader = requested_leader
            .or_else(|| roles.get(&RoleKind::Coordinator).copied())
            .or_else(|| {
                let skill = definition.execution().leader_capability();
                self.state
                    .world()
                    .characters_in_organization(self.player)
                    .filter(|person| {
                        participant_available_for_plan(
                            self.registry,
                            &self.state,
                            person.id(),
                            kind,
                            at,
                            &constraints,
                        )
                    })
                    .max_by_key(|person| {
                        (
                            person.capability(skill).map_or(0, |rating| rating.value()),
                            roles.values().any(|id| *id == person.id()),
                            std::cmp::Reverse(person.id().raw()),
                        )
                    })
                    .map(|person| person.id())
            })
            .ok_or("your organization has no available operation leader")?;
        let coverage = planning_information(
            self.registry,
            &self.state,
            self.player,
            kind,
            &objective,
            at,
        );
        let information = coverage.values().map(|(id, _)| *id).collect();
        let mut detail = format!(
            "{kind:?} at {name} | {approach:?} | starts {}\n",
            time_label(at.as_minutes())
        );
        let leader_name = self
            .state
            .world()
            .get_character(leader)
            .ok_or("leader disappeared")?
            .name();
        writeln!(detail, "  Leader: {leader_name} (char-{})", leader.raw()).unwrap();
        for (role, id) in &roles {
            let member = self
                .state
                .world()
                .get_character(*id)
                .ok_or("participant disappeared")?;
            let skill = definition
                .execution()
                .capability_for_role(*role)
                .ok_or("missing role capability")?;
            let rating = member.capability(skill).map_or(0, |rating| rating.value());
            writeln!(
                detail,
                "  {role:?}: {} (char-{}) | {skill:?} {rating}",
                member.name(),
                id.raw()
            )
            .unwrap();
        }
        let relevant = definition.execution().relevant_intelligence_topics();
        writeln!(
            detail,
            "  Planning intelligence: {}/{} usable topics at scheduled start",
            coverage.len(),
            relevant.len()
        )
        .unwrap();
        for topic in relevant {
            if let Some((id, score)) = coverage.get(topic) {
                writeln!(
                    detail,
                    "    {topic:?}: information-{} (quality {score}/100)",
                    id.raw()
                )
                .unwrap();
            } else {
                writeln!(detail, "    {topic:?}: missing usable information").unwrap();
            }
        }
        let validated = validate_authorize_operation(
            self.registry,
            &self.state,
            OperationDraft {
                title: format!("{kind:?}: {name}"),
                kind,
                responsible_organization: self.player,
                leader,
                objective,
                approach,
                roles,
                intelligence: information,
                constraints,
                contingencies,
                scheduled_for: at,
            },
        )
        .map_err(|error| error.to_string())?;
        if words[0].eq_ignore_ascii_case("preview") {
            return Ok(format!("Preview only; no operation authorized.\n{detail}"));
        }
        let operation = validated
            .commit(&mut self.state)
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "Authorized {kind:?} operation {} against {name}. See 'operation {}' for crew and planning details.\n{detail}",
            operation.raw(),
            operation.raw()
        ))
    }
}
