//! Public human-scale city directory. Real names on registered titles and
//! public institutional rosters are public; private stress, finances, case
//! assignments and unknown criminal membership are never exposed.

use super::*;
use crimocracy::world::freelance_contracts::{
    freelance_contact_available, validate_hire_freelancer,
};
use crimocracy::world::{CivicActivity, CivicProfession, OrganizationKind};
use std::collections::BTreeMap;

impl Session<'_> {
    pub(super) fn civic_directory(&self, district: Option<&str>) -> Result<String, String> {
        let ward = district
            .map(|raw| {
                let raw = parse_number(raw, "district")?;
                self.state
                    .world()
                    .neighborhoods()
                    .find(|ward| ward.id().raw() == raw)
                    .map(|record| record.id())
                    .ok_or_else(|| "unknown public district".to_owned())
            })
            .transpose()?;
        let mut out = if let Some(ward) = ward {
            format!("Public civic roster in district-{}\n", ward.raw())
        } else {
            "Public civic city roster\n".to_owned()
        };
        if ward.is_none() {
            let mut counts = BTreeMap::new();
            for person in self.state.world().public_civic_people() {
                *counts
                    .entry(format!(
                        "{:?}",
                        self.state
                            .world()
                            .civic_profession(person.id())
                            .expect("listed person exists")
                    ))
                    .or_insert(0_usize) += 1;
            }
            for (profession, count) in counts {
                writeln!(out, "  {profession}: {count} named people").unwrap();
            }
            out.push_str("Use 'civics district-N' for registered local proprietors and their public officials.\n");
            return Ok(out);
        }
        let ward = ward.expect("matched district filter");
        for business in self.state.world().businesses_in_neighborhood(ward) {
            if let crimocracy::world::BusinessOwner::Character(owner) = business.owner() {
                let person = self
                    .state
                    .world()
                    .get_character(owner)
                    .expect("registered owner must exist");
                writeln!(
                    out,
                    "  char-{} {} | proprietor | business-{} {} | career {} day(s)",
                    owner.raw(),
                    person.name(),
                    business.id().raw(),
                    business.name(),
                    person.life().career_days()
                )
                .unwrap();
            }
        }
        for person in self.state.world().public_civic_people() {
            let profession = self
                .state
                .world()
                .civic_profession(person.id())
                .expect("listed person exists");
            if person.organization().is_none() {
                if person.life().home() == Some(ward)
                    && matches!(
                        profession,
                        CivicProfession::BusinessEmployee
                            | CivicProfession::Journalist
                            | CivicProfession::IndependentLawyer
                    )
                {
                    writeln!(
                        out,
                        "  char-{} {} | {:?} | independent local career {} day(s)",
                        person.id().raw(),
                        person.name(),
                        profession,
                        person.life().career_days()
                    )
                    .unwrap();
                }
                continue;
            }
            let Some(organization) = person
                .organization()
                .and_then(|id| self.state.world().get_organization(id))
            else {
                continue;
            };
            let serves_ward = match organization.kind() {
                OrganizationKind::LawEnforcement => self
                    .state
                    .legal()
                    .get_jurisdiction(organization.id())
                    .is_some_and(|jurisdiction| jurisdiction.neighborhoods().contains(&ward)),
                OrganizationKind::Political
                | OrganizationKind::Labor
                | OrganizationKind::Commercial
                | OrganizationKind::Civic
                | OrganizationKind::LegalAuthority
                | OrganizationKind::LegalServices
                | OrganizationKind::Prosecutor => true,
                OrganizationKind::Criminal => false,
            };
            if serves_ward && person.life().home().is_none_or(|home| home == ward) {
                writeln!(
                    out,
                    "  char-{} {} | {:?} | {} | career {} day(s)",
                    person.id().raw(),
                    person.name(),
                    profession,
                    organization.name(),
                    person.life().career_days()
                )
                .unwrap();
            }
        }
        Ok(out)
    }

    pub(super) fn civic_character(&self, raw: &str) -> Result<String, String> {
        let raw = parse_number(raw, "char")?;
        let person = self
            .state
            .world()
            .public_civic_people()
            .find(|person| person.id().raw() == raw)
            .or_else(|| {
                self.state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|person| person.id().raw() == raw)
            })
            .or_else(|| {
                self.known_freelancers()
                    .into_iter()
                    .find(|(id, _)| id.raw() == raw)
                    .and_then(|(id, _)| self.state.world().get_character(id))
            })
            .ok_or("person is not publicly registered or known as one of your members")?;
        let profession = self
            .state
            .world()
            .civic_profession(person.id())
            .expect("identified person exists");
        let own_member = person.organization() == Some(self.player);
        let owner = person
            .organization()
            .and_then(|owner| self.state.world().get_organization(owner))
            .filter(|organization| own_member || organization.kind() != OrganizationKind::Criminal);
        // A registered title is public, but a criminal affiliation is not. An
        // unaffiliated proprietor and a hidden rival use the same neutral label.
        let public_affiliation = owner.map_or(
            if profession == CivicProfession::BusinessOwner {
                "not disclosed"
            } else {
                "none"
            },
            |organization| organization.name(),
        );
        let criminal_only = own_member && profession == CivicProfession::Criminal;
        let mut out = if criminal_only {
            format!(
                "char-{} {} | Criminal | organization {} | civilian workdays not applicable\n",
                person.id().raw(),
                person.name(),
                public_affiliation
            )
        } else {
            format!(
                "char-{} {} | {:?} | organization {} | workdays {}\n",
                person.id().raw(),
                person.name(),
                profession,
                public_affiliation,
                person.life().career_days()
            )
        };
        if own_member {
            let operations = self
                .state
                .operations()
                .operations_for_organization(self.player)
                .filter(|operation| {
                    operation.leader() == person.id()
                        || operation.roles().values().any(|id| *id == person.id())
                });
            let (completed, achieved) = operations.fold((0_usize, 0_usize), |(done, won), op| {
                let resolution = op.resolution();
                (
                    done + usize::from(resolution.is_some()),
                    won + usize::from(resolution.is_some_and(|result| {
                        result.objective_outcome()
                            == crimocracy::operations::OperationObjectiveOutcome::Achieved
                    })),
                )
            });
            let rackets = self
                .state
                .enterprises()
                .enterprises_for_organization(self.player)
                .filter(|racket| {
                    racket.manager() == person.id()
                        && racket.status() != crimocracy::enterprises::EnterpriseStatus::Retired
                })
                .count();
            let channels = self
                .state
                .contacts()
                .contacts_for_sponsor(self.player)
                .filter(|contact| {
                    contact.handler() == person.id()
                        && contact.status() == crimocracy::contacts::ContactStatus::Active
                })
                .count();
            writeln!(
                out,
                "  Recorded crew activity: {completed} completed operation(s), {achieved} objectives achieved; {rackets} currently managed racket(s); {channels} active institutional channel(s)"
            )
            .unwrap();
        }
        for business in self
            .state
            .world()
            .businesses_owned_by_character(person.id())
        {
            writeln!(
                out,
                "  Registered owner: business-{} {} in district-{}",
                business.id().raw(),
                business.name(),
                business.neighborhood().raw()
            )
            .unwrap();
        }
        if let Some((at, action)) = person.life().last_activity().filter(|(_, action)| {
            own_member
                || profession == CivicProfession::FreelanceCriminal
                || !matches!(action, CivicActivity::FreelanceWork(_))
        }) {
            let public_action = match action {
                CivicActivity::BusinessManagement(business) => {
                    format!("personally managed business-{}", business.raw())
                }
                CivicActivity::Politics(_) => "served in municipal government".to_owned(),
                CivicActivity::PolicePatrol(_) => "worked a police patrol shift".to_owned(),
                CivicActivity::DetectiveWork(_) => {
                    "performed police casework (case confidential)".to_owned()
                }
                CivicActivity::LegalService(_) | CivicActivity::Prosecution(_) => {
                    "performed legal duties".to_owned()
                }
                CivicActivity::LaborAdvocacy(_) => "represented organized labor".to_owned(),
                CivicActivity::CommercialAdvocacy(_) => "represented local commerce".to_owned(),
                CivicActivity::Community => "participated in neighborhood affairs".to_owned(),
                CivicActivity::BusinessShift(_) => "worked at a local business".to_owned(),
                CivicActivity::FreelanceWork(_) => "pursued independent assignments".to_owned(),
                CivicActivity::Journalism(_) => "worked as a local journalist".to_owned(),
                CivicActivity::IndependentLegalPractice(_) => {
                    "served independent legal clients".to_owned()
                }
            };
            writeln!(
                out,
                "  Last recorded duty: {public_action}, at {}",
                time_label(at.as_minutes())
            )
            .unwrap();
        } else if !criminal_only {
            out.push_str("  No disclosed civilian professional duty yet.\n");
        }
        if own_member {
            writeln!(
                out,
                "  Own member condition: professional standing {}/100, fatigue {}/100",
                person.life().professional_standing(),
                person.life().fatigue()
            )
            .unwrap();
        } else {
            out.push_str("  Private condition and personal finances are not disclosed.\n");
        }
        Ok(out)
    }

    fn known_freelancers(&self) -> BTreeMap<CharacterId, Vec<CharacterId>> {
        let mut available = BTreeMap::new();
        for member in self.state.world().characters_in_organization(self.player) {
            for relationship in self.state.social().relationships_to(member.id()) {
                if freelance_contact_available(
                    &self.state,
                    self.player,
                    relationship.from(),
                    member.id(),
                ) {
                    available
                        .entry(relationship.from())
                        .or_insert_with(Vec::new)
                        .push(member.id());
                }
            }
        }
        available
    }

    pub(super) fn population_dossier(&self) -> String {
        let population = self.state.world().population_size();
        let public = self.state.world().public_civic_people().count();
        let wards = self.state.world().neighborhoods().count();
        format!(
            "Persistently simulated named people: {population} in {wards} districts.\nPublic registered proprietors and civic professionals: {public}.\nOther people include residents, unaffiliated workers, criminal crews and private freelance contacts; identities require actual relationships or intelligence.\nUse 'civics', 'civics district-N' and 'freelancers'.\n"
        )
    }

    pub(super) fn freelancer_directory(&self) -> String {
        let known = self.known_freelancers();
        let mut out = format!(
            "Personally known, currently available freelancers: {}\n",
            known.len()
        );
        for (id, contacts) in known {
            let person = self
                .state
                .world()
                .get_character(id)
                .expect("known freelance relationship has a real person");
            let names: Vec<_> = contacts
                .iter()
                .map(|manager| format!("char-{}", manager.raw()))
                .collect();
            let skills: Vec<_> = crimocracy::world::ALL_CAPABILITY_KINDS
                .iter()
                .filter(|kind| person.capability(**kind).is_some())
                .map(|kind| format!("{kind:?}"))
                .collect();
            writeln!(out, "  char-{} {} | district-{} | {} | contacts {}\n    contract char-{} {} <days> (1-14)",
                id.raw(), person.name(), person.life().home().map_or(0, |home| home.raw()),
                skills.join(", "), names.join(", "), id.raw(), names[0]).unwrap();
        }
        out
    }

    pub(super) fn execute_freelance_contract(&mut self, words: &[&str]) -> Result<String, String> {
        let freelancer_raw = parse_number(words[1], "char")?;
        let manager_raw = parse_number(words[2], "char")?;
        let days = words[3]
            .parse::<u8>()
            .map_err(|_| "contract days must be an integer between 1 and 14")?;
        let freelancer = self
            .known_freelancers()
            .into_iter()
            .find(|(person, contacts)| {
                person.raw() == freelancer_raw
                    && contacts.iter().any(|manager| manager.raw() == manager_raw)
            })
            .map(|(person, _)| person)
            .ok_or("freelancer is not a current personal acquaintance of that crew member")?;
        let manager = self
            .state
            .world()
            .characters_in_organization(self.player)
            .find(|member| member.id().raw() == manager_raw)
            .map(|member| member.id())
            .ok_or("contract manager does not belong to your crew")?;
        let token = validate_hire_freelancer(&self.state, self.player, freelancer, manager, days)
            .map_err(|error| error.to_string())?;
        let fee = token.fee_cents();
        token
            .commit(&mut self.state)
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "Contracted char-{} under char-{} for {} days, paid {} street cents to the freelancer's own account. Normal operation and payroll rules apply.\n",
            freelancer.raw(),
            manager.raw(),
            days,
            fee
        ))
    }
}
