//! Public human-scale city directory. Real names on registered titles and
//! public institutional rosters are public; private stress, finances, case
//! assignments and unknown criminal membership are never exposed.

use super::*;
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
        let mut out = if let Some(ward) = ward {
            format!("Public civic roster in district-{}\n", ward.raw())
        } else {
            "Public civic city roster\n".to_owned()
        };
        if ward.is_none() {
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
            if serves_ward {
                writeln!(
                    out,
                    "  char-{} {} | {:?} | {} | career {} day(s)",
                    person.id().raw(),
                    person.name(),
                    self.state
                        .world()
                        .civic_profession(person.id())
                        .unwrap_or(CivicProfession::Resident),
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
            .ok_or("person is not publicly registered or known as one of your members")?;
        let profession = self
            .state
            .world()
            .civic_profession(person.id())
            .expect("identified person exists");
        let owner = person
            .organization()
            .and_then(|owner| self.state.world().get_organization(owner));
        let mut out = format!(
            "char-{} {} | {:?} | organization {} | workdays {}\n",
            person.id().raw(),
            person.name(),
            profession,
            owner.map_or("none", |organization| organization.name()),
            person.life().career_days()
        );
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
        if let Some((at, action)) = person.life().last_activity() {
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
            };
            writeln!(
                out,
                "  Last recorded duty: {public_action}, at {}",
                time_label(at.as_minutes())
            )
            .unwrap();
        } else {
            out.push_str("  No recorded professional duty yet.\n");
        }
        if person.organization() == Some(self.player) {
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
}
