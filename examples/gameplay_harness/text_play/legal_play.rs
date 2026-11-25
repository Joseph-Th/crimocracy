//! Player-controlled legal defense. New retainers require a current member;
//! existing sponsored engagements remain manageable after a member leaves.
//! Counsel comes from an actual player-established legal contact, and all
//! payments and closures are committed by the production legal owner.

use super::*;
use crimocracy::legal::legal_representation_system::{
    sponsored_active_representations, sponsored_representation_matter_open,
};

impl Session<'_> {
    pub(super) fn known_member_arrests(&self) -> BTreeSet<ArrestId> {
        crimocracy::legal::arrest_system::member_arrest_history(&self.state, self.player)
            .into_iter()
            .collect()
    }

    pub(super) fn execute_legal(&mut self, words: &[&str]) -> Result<String, String> {
        let mut out = String::new();
        match words[0] {
            "counsel" => {
                let members = self.known_member_arrests();
                let mut known = members.clone();
                known.extend(
                    sponsored_active_representations(&self.state, self.player)
                        .map(|representation| representation.arrest()),
                );
                let selected = match words.get(1).copied() {
                    None | Some("all") => None,
                    Some(raw) => {
                        let id = parse_number(raw, "arrest")?;
                        Some(
                            known
                                .iter()
                                .copied()
                                .find(|arrest| arrest.raw() == id)
                                .ok_or("arrest is not a current member matter or an active sponsored engagement")?,
                        )
                    }
                };
                let show_all = words.get(1) == Some(&"all");
                let mut quiet = 0_usize;
                if known.is_empty() {
                    out.push_str("No arrests of current members or active sponsored engagements are known to leadership.\n");
                }
                for id in known {
                    if selected.is_some_and(|requested| requested != id) {
                        continue;
                    }
                    let record = self
                        .state
                        .legal()
                        .get_arrest(id)
                        .ok_or("known arrest record disappeared")?;
                    let current_member = members.contains(&id);
                    let representation = self.state.legal().active_representation_for_arrest(id);
                    let sponsored = representation.filter(|record| record.sponsor() == self.player);
                    let detained = record.status() == crimocracy::legal::ArrestStatus::Detained;
                    let open_review = if current_member {
                        crimocracy::legal::prosecution_system::member_prosecution_under_review(
                            &self.state,
                            self.player,
                            id,
                        )
                    } else {
                        sponsored.is_some_and(|rep| {
                            !detained
                                && sponsored_representation_matter_open(
                                    &self.state,
                                    self.player,
                                    rep.id(),
                                )
                                .unwrap_or(false)
                        })
                    };
                    let judgments = crimocracy::legal::prosecution_system::member_judgments(
                        &self.state,
                        self.player,
                        id,
                    );
                    let actionable = detained
                        || open_review
                        || sponsored.is_some()
                        || judgments
                            .iter()
                            .any(|judgment| judgment.outstanding.cents() > 0);
                    if !show_all && selected.is_none() && !actionable {
                        quiet += 1;
                        continue;
                    }
                    let defendant = self
                        .state
                        .world()
                        .get_character(record.character())
                        .ok_or("known defendant disappeared")?;
                    writeln!(
                        out,
                        "arrest-{} | {} (char-{}){} | {:?} | observed from {}",
                        id.raw(),
                        defendant.name(),
                        defendant.id().raw(),
                        if current_member {
                            ""
                        } else {
                            " | former member; sponsored engagement"
                        },
                        record.status(),
                        time_label(record.arrested_at().as_minutes())
                    )
                    .unwrap();
                    if open_review {
                        out.push_str("  Prosecutorial review remains open; release does not end the legal matter.\n");
                    }
                    for judgment in judgments {
                        writeln!(
                            out,
                            "  Court: {:?} on {} | personal fine: {} cents | outstanding: {} cents",
                            judgment.status,
                            time_label(judgment.resolved_at.as_minutes()),
                            judgment.fine.cents(),
                            judgment.outstanding.cents()
                        )
                        .unwrap();
                    }
                    if let Some(representation) = sponsored {
                        let counsel = self
                            .state
                            .world()
                            .get_character(representation.counsel())
                            .ok_or("counsel disappeared")?;
                        writeln!(
                            out,
                            "  representation-{}: {} | retained for {} cents",
                            representation.id().raw(),
                            counsel.name(),
                            representation.fee().cents()
                        )
                        .unwrap();
                        if !detained && !open_review {
                            writeln!(out, "  Legal matter no longer needs active defense. This retainer remains assigned until leadership uses 'end-counsel representation-{} concluded'.", representation.id().raw()).unwrap();
                        }
                    } else if representation.is_some() {
                        out.push_str("  Existing counsel retained outside your organization; this engagement is not yours to end.\n");
                    } else if detained || open_review {
                        out.push_str(
                            "  No retained counsel. Use 'retain arrest-N contact-N cents'.\n",
                        );
                    }
                }
                if quiet > 0 {
                    writeln!(out, "{quiet} other known arrest matter(s) have no active custody, open review, sponsored retainer, or unpaid fine. Use 'counsel all' or 'counsel arrest-N' for history.").unwrap();
                } else if out.is_empty() {
                    out.push_str("No known current member matters require legal attention.\n");
                }
                for contact in self
                    .state
                    .contacts()
                    .contacts_for_sponsor(self.player)
                    .filter(|contact| {
                        contact.kind() == ContactKind::Legal
                            && contact.status() == ContactStatus::Active
                    })
                {
                    let counsel = self
                        .state
                        .world()
                        .get_character(contact.contact())
                        .ok_or("legal contact person disappeared")?;
                    writeln!(
                        out,
                        "Legal channel: contact-{} {}",
                        contact.id().raw(),
                        counsel.name()
                    )
                    .unwrap();
                }
            }
            "retain" => {
                let raw = parse_number(words[1], "arrest")?;
                let arrest = self
                    .known_member_arrests()
                    .into_iter()
                    .find(|id| id.raw() == raw)
                    .ok_or("arrest is not known for a current organization member")?;
                if self
                    .state
                    .legal()
                    .active_representation_for_arrest(arrest)
                    .is_some_and(|record| record.sponsor() != self.player)
                {
                    return Err(
                        "this member already has counsel retained outside your organization"
                            .to_owned(),
                    );
                }
                let contact_raw = parse_number(words[2], "contact")?;
                let contact = self
                    .state
                    .contacts()
                    .contacts_for_sponsor(self.player)
                    .find(|contact| {
                        contact.id().raw() == contact_raw
                            && contact.kind() == ContactKind::Legal
                            && contact.status() == ContactStatus::Active
                    })
                    .ok_or("legal contact is not active for your organization")?;
                let provider = contact.institution();
                let provider_account = self
                    .state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(provider))
                    .find(|account| account.kind() == AccountKind::LegitimateOperating)
                    .ok_or("legal-services provider has no operating account")?
                    .id();
                let fee_cents = words[3]
                    .parse::<i64>()
                    .ok()
                    .filter(|fee| *fee > 0)
                    .ok_or("retainer must be a positive amount in cents")?;
                let payer_accounts = self
                    .state
                    .finance()
                    .accounts_for(FinancialOwner::Organization(self.player))
                    .filter(|account| account.kind().is_liquid())
                    .map(|account| account.id())
                    .collect();
                let draft = LegalRepresentationDraft {
                    arrest,
                    sponsor: self.player,
                    contact: contact.id(),
                    fee: Money::from_cents(fee_cents),
                    payer_accounts,
                    provider_account,
                    authorization: None,
                    origin: LegalRepresentationOrigin::DirectRetention,
                };
                let id = validate_retain_legal_representation(&self.state, draft)
                    .map_err(|error| error.to_string())?
                    .commit(&mut self.state)
                    .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Retained counsel as representation-{} for arrest-{}; paid {} cents.",
                    id.raw(),
                    arrest.raw(),
                    fee_cents
                )
                .unwrap();
            }
            "end-counsel" => {
                let raw = parse_number(words[1], "representation")?;
                let representation = sponsored_active_representations(&self.state, self.player)
                    .find(|record| record.id().raw() == raw)
                    .ok_or(
                        "representation is not an active engagement sponsored by your organization",
                    )?
                    .id();
                let reason = match words[2] {
                    "concluded" => LegalRepresentationEndReason::MatterConcluded,
                    "replace" => LegalRepresentationEndReason::Replaced,
                    "withdraw" => LegalRepresentationEndReason::SponsorWithdrawn,
                    _ => {
                        return Err(
                            "end-counsel <representation-id> <concluded|replace|withdraw>"
                                .to_owned(),
                        );
                    }
                };
                validate_end_legal_representation(&self.state, representation, reason)
                    .map_err(|error| error.to_string())?
                    .commit(&mut self.state)
                    .map_err(|error| error.to_string())?;
                writeln!(
                    out,
                    "Representation {} ended: {:?}.",
                    representation.raw(),
                    reason
                )
                .unwrap();
            }
            _ => return Err("unknown legal command".to_owned()),
        }
        Ok(out)
    }
}
