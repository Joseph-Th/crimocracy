//! Registry-directed player operation orders.
//!
//! The client selects a player-known business, crew, and approach. All objective
//! family, role, target, participant, booking, and intelligence validation remains
//! with the production operation owner. The client cannot learn unseen ownership,
//! economy status or law-enforcement state simply by naming a public address:
//! foreign-business plans require target-specific organization-held information.

use super::*;
use crimocracy::world::BusinessOwner;

fn normalized(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_operation_kind(raw: &str) -> Result<OperationKind, String> {
    let key = normalized(raw);
    ALL_OPERATION_KINDS
        .into_iter()
        .find(|kind| normalized(&format!("{kind:?}")) == key)
        .ok_or_else(|| format!("unknown operation kind '{raw}'; use 'operation-kinds'"))
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
            let target = if kind == OperationKind::Burglary {
                "use 'burgle' to consume a known lead"
            } else if kind == OperationKind::Extraction || kind == OperationKind::WitnessPressure {
                "known char-N with learned legal-status basis"
            } else {
                "business target"
            };
            writeln!(
                output,
                "{kind:?} | roles: {roles} | approaches: {approaches} | {}m | {target}",
                definition.execution().duration().as_minutes()
            )
            .unwrap();
        }
        output
    }

    /// Assign the scarcest role first, then prefer proficiency over hierarchy.
    /// A modest supervised-member preference avoids defaulting to the boss when
    /// a competent subordinate is close in skill. Do not silently appoint an
    /// untrained person to a field role: the player must explicitly accept that
    /// risk with a role assignment.
    fn select_operation_roles(
        &self,
        kind: OperationKind,
        explicit: &BTreeMap<RoleKind, CharacterId>,
    ) -> Result<BTreeMap<RoleKind, CharacterId>, String> {
        let definition = self.registry.get_operation(kind);
        let roster: Vec<_> = self
            .state
            .world()
            .characters_in_organization(self.player)
            .collect();
        let mut used: BTreeSet<_> = explicit.values().copied().collect();
        if used.len() != explicit.len() {
            return Err("the same character cannot fill two operation roles".to_owned());
        }
        let mut roles = explicit.clone();
        let mut missing = definition
            .required_roles()
            .iter()
            .filter(|role| !roles.contains_key(role))
            .copied()
            .collect::<Vec<_>>();
        missing.sort_by_key(|role| {
            let skill = definition.execution().capability_for_role(*role);
            let trained = roster
                .iter()
                .filter(|member| {
                    !used.contains(&member.id())
                        && skill.is_some_and(|skill| {
                            member
                                .capability(skill)
                                .is_some_and(|rating| rating.value() > 0)
                        })
                })
                .count();
            (trained, *role)
        });
        for role in missing {
            let skill = definition
                .execution()
                .capability_for_role(role)
                .ok_or("operation role has no authored capability")?;
            let candidate = roster
                .iter()
                .filter(|member| !used.contains(&member.id()))
                .filter(|member| member.capability(skill).is_some_and(|rating| rating.value() > 0))
                .max_by_key(|member| {
                    let skill =
                        i32::from(member.capability(skill).map_or(0, |rating| rating.value()));
                    let supervisory_bonus = i32::from(member.supervisor().is_some()) * 120;
                    (
                        skill * 10 + supervisory_bonus,
                        std::cmp::Reverse(member.id().raw()),
                    )
                })
                .map(|member| member.id())
                .ok_or_else(|| format!(
                    "no unassigned member has trained {skill:?} for {role:?}; use {role:?}=char-N to choose an untrained member explicitly, or recruit a specialist"
                ))?;
            used.insert(candidate);
            roles.insert(role, candidate);
        }
        Ok(roles)
    }

    pub(super) fn execute_planning(&mut self, words: &[&str]) -> Result<String, String> {
        if words[0] == "operation-kinds" {
            return Ok(self.operation_kinds());
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
        let (target, name, location) = if person_target {
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
            let owned = target.owner() == BusinessOwner::Organization(self.player);
            // A public address is not an operational opening. Foreign targets
            // require organization-held information before authorization can
            // query the owner's hidden economy or ownership dependencies.
            if kind != OperationKind::Surveillance
                && !owned
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
        for assignment in &words[3..] {
            let (key, value) = assignment
                .split_once('=')
                .ok_or("plan options must be approach=covert, leader=char-N or Role=char-N")?;
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
        let roles = self.select_operation_roles(kind, &explicit)?;
        let leader = requested_leader
            .or_else(|| roles.get(&RoleKind::Coordinator).copied())
            .or_else(|| {
                let skill = definition.execution().leader_capability();
                self.state
                    .world()
                    .characters_in_organization(self.player)
                    .max_by_key(|person| {
                        (
                            person.capability(skill).map_or(0, |rating| rating.value()),
                            std::cmp::Reverse(person.id().raw()),
                        )
                    })
                    .map(|person| person.id())
            })
            .ok_or("your organization has no available operation leader")?;
        let relevant = definition.execution().relevant_intelligence_topics();
        let information: BTreeSet<_> = self
            .organization_information()
            .filter(|info| relevant.contains(&info.topic()))
            .filter(|info| {
                info.subject() == target
                    || location
                        .is_some_and(|district| info.subject() == EntityRef::Neighborhood(district))
            })
            .map(|info| info.id())
            .collect();
        let at = self
            .state
            .now()
            .checked_add(SimDuration::ONE_MINUTE)
            .ok_or("simulation clock exhausted")?;
        let objective = kind
            .objective_for_target(target)
            .ok_or("this operation requires a different target type")?;
        let operation = validate_authorize_operation(
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
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for: at,
            },
        )
        .map_err(|error| error.to_string())?
        .commit(&mut self.state)
        .map_err(|error| error.to_string())?;
        Ok(format!(
            "Authorized {kind:?} operation {} against {name}. See 'operation {}' for crew and planning details.",
            operation.raw(),
            operation.raw()
        ))
    }
}
