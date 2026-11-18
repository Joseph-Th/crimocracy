//! Player-created institutional links and their provenance-bearing disclosures.
//! The potential-connection view is restricted to characters the organization has
//! already identified and direct social edges involving current staff; no world
//! roster, contact-held sources, or hidden case details are offered for free.

use super::*;

impl Session<'_> {
    pub(super) fn execute_contacts(&mut self, words: &[&str]) -> Result<String, String> {
        let mut out = String::new();
        match words[0] {
            "connections" => {
                let staff: Vec<_> = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .map(|person| person.id())
                    .collect();
                for outsider in self.known_character_ids() {
                    if staff.contains(&outsider) {
                        continue;
                    }
                    let person = self
                        .state
                        .world()
                        .get_character(outsider)
                        .ok_or("known person disappeared")?;
                    let institution = person
                        .organization()
                        .and_then(|org| self.state.world().get_organization(org));
                    for handler in &staff {
                        // A detainee cannot manage or establish a channel. This
                        // is current information about our own member, not an
                        // observation of the outsider's private legal state.
                        if self
                            .state
                            .legal()
                            .active_arrest_for_character(*handler)
                            .is_some()
                        {
                            continue;
                        }
                        if self
                            .state
                            .social()
                            .get_relationship(*handler, outsider)
                            .is_none()
                            && self
                                .state
                                .social()
                                .get_relationship(outsider, *handler)
                                .is_none()
                        {
                            continue;
                        }
                        let member = self
                            .state
                            .world()
                            .get_character(*handler)
                            .ok_or("staff member disappeared")?;
                        let employer = institution.map_or("unaffiliated", |org| org.name());
                        writeln!(
                            out,
                            "char-{} {} | {} | known to char-{} {}",
                            outsider.raw(),
                            person.name(),
                            employer,
                            handler.raw(),
                            member.name(),
                        )
                        .unwrap();
                    }
                }
                if out.is_empty() {
                    out.push_str(
                        "No identified institutional acquaintances with staff relationships.",
                    );
                }
            }
            "contacts" => {
                for contact in self.state.contacts().contacts_for_sponsor(self.player) {
                    let person = self
                        .state
                        .world()
                        .get_character(contact.contact())
                        .ok_or("contact character disappeared")?;
                    let handler = self
                        .state
                        .world()
                        .get_character(contact.handler())
                        .ok_or("contact handler disappeared")?;
                    let institution = self
                        .state
                        .world()
                        .get_organization(contact.institution())
                        .ok_or("contact institution disappeared")?;
                    let pending = if contact.status() == ContactStatus::Active {
                        find_pending_disclosure_sources(&self.state, contact.id()).len()
                    } else {
                        0
                    };
                    writeln!(out,
                        "contact-{} {:?} {:?} | {} at {} | handler {} (char-{}) | {} undisclosed source(s)",
                        contact.id().raw(), contact.kind(), contact.status(),
                        person.name(), institution.name(), handler.name(), handler.id().raw(),
                        pending,
                    ).unwrap();
                }
                if out.is_empty() {
                    out.push_str("No established institutional contacts.");
                }
            }
            "connect" => {
                let handler_raw = parse_number(words[1], "char")?;
                let outsider_raw = parse_number(words[2], "char")?;
                let handler = self
                    .state
                    .world()
                    .characters_in_organization(self.player)
                    .find(|member| member.id().raw() == handler_raw)
                    .ok_or("handler must be a current organization member")?
                    .id();
                let contact = self
                    .known_character(outsider_raw)
                    .ok_or("contact must already be identified by your organization")?;
                if self
                    .state
                    .world()
                    .get_character(contact)
                    .is_some_and(|person| person.organization() == Some(self.player))
                {
                    return Err(
                        "institutional contacts must be outside your organization".to_owned()
                    );
                }
                let id = validate_establish_contact(
                    &self.state,
                    InstitutionalContactDraft {
                        sponsor: self.player,
                        handler,
                        contact,
                    },
                )
                .map_err(|e| e.to_string())?
                .commit(&mut self.state)
                .map_err(|e| e.to_string())?;
                let record = self
                    .state
                    .contacts()
                    .get_contact(id)
                    .ok_or("established contact disappeared")?;
                writeln!(
                    out,
                    "Established {:?} contact-{} via char-{}.",
                    record.kind(),
                    id.raw(),
                    handler.raw()
                )
                .unwrap();
            }
            "contact" => {
                let (id, action) = if words.len() == 1 {
                    let contact = self
                        .state
                        .contacts()
                        .contacts_for_sponsor(self.player)
                        .find(|record| {
                            record.kind() == ContactKind::Police
                                && record.status() == ContactStatus::Active
                        })
                        .ok_or("your organization has no active police contact; use 'contacts'")?;
                    (contact.id(), "ask")
                } else {
                    let raw = parse_number(words[1], "contact")?;
                    let id = self
                        .state
                        .contacts()
                        .contacts_for_sponsor(self.player)
                        .find(|contact| contact.id().raw() == raw)
                        .ok_or("contact does not belong to your organization")?
                        .id();
                    (id, words.get(2).copied().unwrap_or("ask"))
                };
                match action {
                    "ask" | "ask-all" => {
                        let active = self
                            .state
                            .contacts()
                            .get_contact(id)
                            .is_some_and(|record| record.status() == ContactStatus::Active);
                        if !active {
                            return Err("contact is no longer active".to_owned());
                        }
                        let sources = find_pending_disclosure_sources(&self.state, id);
                        let limit = if action == "ask-all" { 64 } else { 1 };
                        if sources.is_empty() {
                            out.push_str(
                                "This contact has no new available information to disclose.",
                            );
                        }
                        for (index, source) in sources.iter().copied().take(limit).enumerate() {
                            // Each disclosure remains a separate canonical transaction. A later
                            // capacity failure must acknowledge earlier successful disclosures.
                            let result = validate_contact_disclosure(&self.state, id, source)
                                .and_then(|token| token.commit(&mut self.state));
                            let disclosure = match result {
                                Ok(disclosure) => disclosure,
                                Err(error) if index == 0 => return Err(error.to_string()),
                                Err(error) => {
                                    writeln!(
                                        out,
                                        "Briefing stopped after {index} disclosures: {error}"
                                    )
                                    .unwrap();
                                    return Ok(out);
                                }
                            };
                            let information = self
                                .state
                                .contacts()
                                .get_disclosure(disclosure)
                                .ok_or("disclosure disappeared")?
                                .disclosed_information();
                            let fact = self
                                .state
                                .intelligence()
                                .get_information(information)
                                .ok_or("disclosed information disappeared")?;
                            writeln!(
                                out,
                                "Contact disclosed {} (observed minute {}): {}",
                                fact.id().raw(),
                                fact.observed_at().as_minutes(),
                                fact.summary()
                            )
                            .unwrap();
                        }
                        if action == "ask-all" && sources.len() > limit {
                            writeln!(
                                out,
                                "{} more disclosures available; ask-all again.",
                                sources.len() - limit
                            )
                            .unwrap();
                        }
                    }
                    "end" => {
                        validate_terminate_contact(&self.state, id)
                            .map_err(|e| e.to_string())?
                            .commit(&mut self.state)
                            .map_err(|e| e.to_string())?;
                        writeln!(out, "Contact {} terminated.", id.raw()).unwrap();
                    }
                    _ => return Err("contact [contact-id] [ask|ask-all|end]".to_owned()),
                }
            }
            _ => return Err("unknown contact command".to_owned()),
        }
        Ok(out)
    }
}
