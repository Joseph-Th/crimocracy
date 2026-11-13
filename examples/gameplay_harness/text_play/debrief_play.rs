//! A member's personal observations are not organization intelligence until
//! transferred. Debrief orders use the production intelligence owner's
//! single-source provenance-preserving transfer, one record per command.

use super::*;
use crimocracy::core::id::InformationId;

impl Session<'_> {
    /// Decide which of this member's own observations are not already reported,
    /// without inspecting or exposing a foreign holder's information. An
    /// internal-report loop with the same root cannot be debriefed twice.
    fn pending_member_notes(&self, member: CharacterId) -> Result<Vec<InformationId>, String> {
        let recipient = KnowledgeHolder::Organization(self.player);
        let mut notes = Vec::new();
        for fact in self
            .state
            .intelligence()
            .information_for_holder(KnowledgeHolder::Character(member))
        {
            match validate_information_transfer(
                &self.state,
                InformationTransferDraft {
                    source: fact.id(),
                    recipient,
                },
            ) {
                Ok(_) => notes.push(fact.id()),
                Err(IntelligenceError::DuplicateTransfer { .. }) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(notes)
    }

    pub(super) fn execute_debrief(&mut self, words: &[&str]) -> Result<String, String> {
        if words[0] == "debriefs" {
            let mut out = String::new();
            for member in self.state.world().characters_in_organization(self.player) {
                let detained = self
                    .state
                    .legal()
                    .active_arrest_for_character(member.id())
                    .is_some();
                if detained {
                    writeln!(
                        out,
                        "char-{} {} | in custody, unavailable for debrief",
                        member.id().raw(),
                        member.name()
                    )
                    .unwrap();
                } else {
                    let pending = self.pending_member_notes(member.id())?.len();
                    writeln!(
                        out,
                        "char-{} {} | {} unreported observation(s)",
                        member.id().raw(),
                        member.name(),
                        pending
                    )
                    .unwrap();
                }
            }
            return Ok(out);
        }
        let raw = parse_number(words[1], "char")?;
        let member = self
            .state
            .world()
            .characters_in_organization(self.player)
            .find(|member| member.id().raw() == raw)
            .ok_or("debrief requires a current organization member")?;
        let member_id = member.id();
        let member_name = member.name().to_owned();
        if self
            .state
            .legal()
            .active_arrest_for_character(member_id)
            .is_some()
        {
            return Err("a detained member is unavailable for debrief".to_owned());
        }
        let Some(source) = self.pending_member_notes(member_id)?.first().copied() else {
            return Ok(format!("{member_name} has no new observations to report."));
        };
        let recorded = validate_information_transfer(
            &self.state,
            InformationTransferDraft {
                source,
                recipient: KnowledgeHolder::Organization(self.player),
            },
        )
        .map_err(|error| error.to_string())?
        .commit(&mut self.state)
        .map_err(|error| error.to_string())?;
        let information = self
            .state
            .intelligence()
            .get_information(recorded)
            .ok_or("transferred observation disappeared")?;
        Ok(format!(
            "{member_name} debriefed information-{} from personal source information-{}: {}",
            recorded.raw(),
            source.raw(),
            information.summary()
        ))
    }
}
