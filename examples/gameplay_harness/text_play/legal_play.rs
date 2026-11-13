//! Player-controlled legal defense. An arrest is selectable only when it affects
//! a current member and is either current custody or known to the organization
//! through typed personal detention information. Counsel comes from an actual
//! player-established legal contact, and all payments and closures are committed
//! by the production legal-representation owner.

use super::*;

impl Session<'_> {
    pub(super) fn known_member_arrests(&self) -> BTreeSet<ArrestId> {
        let members: BTreeSet<_> = self
            .state
            .world()
            .characters_in_organization(self.player)
            .map(|member| member.id())
            .collect();
        let mut arrests = BTreeSet::new();
        for member in &members {
            if let Some(arrest) = self.state.legal().active_arrest_for_character(*member) {
                arrests.insert(arrest.id());
            }
        }
        for information in self.organization_information() {
            if let EntityRef::Character(member) = information.subject()
                && members.contains(&member)
                && let Some(InformationSignal::LegalPersonStatus(
                    LegalPersonStatusSignal::Detained { arrest },
                )) = information.signal()
            {
                arrests.insert(*arrest);
            }
        }
        arrests
    }

    pub(super) fn execute_legal(&mut self, words: &[&str]) -> Result<String, String> {
        let mut out = String::new();
        match words[0] {
            "counsel" => {
                let known = self.known_member_arrests();
                if known.is_empty() {
                    out.push_str("No arrests of current members are known to leadership.\n");
                }
                for id in known {
                    let record = self
                        .state
                        .legal()
                        .get_arrest(id)
                        .ok_or("known arrest record disappeared")?;
                    let defendant = self
                        .state
                        .world()
                        .get_character(record.character())
                        .ok_or("known defendant disappeared")?;
                    writeln!(
                        out,
                        "arrest-{} | {} (char-{}) | {:?} | observed from {}",
                        id.raw(),
                        defendant.name(),
                        defendant.id().raw(),
                        record.status(),
                        time_label(record.arrested_at().as_minutes())
                    )
                    .unwrap();
                    if let Some(representation) =
                        self.state.legal().active_representation_for_arrest(id)
                    {
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
                    } else {
                        out.push_str("  No retained counsel. Use 'retain arrest-N contact-N cents' if the legal matter remains open.\n");
                    }
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
                let representation = self
                    .known_member_arrests()
                    .into_iter()
                    .filter_map(|arrest| {
                        self.state.legal().active_representation_for_arrest(arrest)
                    })
                    .find(|record| record.id().raw() == raw && record.sponsor() == self.player)
                    .ok_or("representation is not active for one of your known member cases")?
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
