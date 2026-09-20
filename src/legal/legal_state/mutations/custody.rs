//! Arrest and legal-representation mutation/index maintenance.

use super::super::*;

impl LegalState {
    pub(in crate::legal) fn insert_arrest(&mut self, record: ArrestRecord) {
        let id = record.id();
        debug_assert_eq!(
            record.status(),
            ArrestStatus::Detained,
            "Lifecycle Validity: new arrest records must begin in detention"
        );
        self.indexes
            .arrests
            .by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        let previous_active = self
            .indexes
            .arrests
            .active_by_character
            .insert(record.character(), id);
        debug_assert!(
            previous_active.is_none(),
            "Ownership Exclusivity: character has multiple active detentions"
        );
        self.indexes.arrests.detained.insert(id);
        let previous = self.arrests.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate arrest ID inserted"
        );
    }
    pub(in crate::legal) fn release_arrest(&mut self, id: ArrestId, released_at: SimTime) {
        let character = self
            .arrests
            .get(&id)
            .expect("validated arrest disappeared before release commit")
            .character();
        let removed = self.indexes.arrests.active_by_character.remove(&character);
        debug_assert_eq!(
            removed,
            Some(id),
            "Derived Data Consistency: active detention index changed before release"
        );
        let removed_detained = self.indexes.arrests.detained.remove(&id);
        debug_assert!(
            removed_detained,
            "Derived Data Consistency: released arrest was not indexed as detained"
        );
        let record = self
            .arrests
            .get_mut(&id)
            .expect("validated arrest disappeared before release commit");
        record.status = ArrestStatus::Released;
        record.released_at = Some(released_at);
        record.version = advance_version_preflighted(record.version);
    }
    pub(in crate::legal) fn insert_legal_representation(
        &mut self,
        record: LegalRepresentationRecord,
    ) {
        let id = record.id();
        debug_assert_eq!(
            record.status(),
            LegalRepresentationStatus::Active,
            "Lifecycle Validity: new legal representation must begin active"
        );
        if record.origin() == LegalRepresentationOrigin::AutomaticPolicy {
            self.indexes
                .representations
                .active_automatic_policy
                .insert(id);
        }
        let previous = self
            .indexes
            .representations
            .active_by_arrest
            .insert(record.arrest(), id);
        debug_assert!(
            previous.is_none(),
            "Ownership Exclusivity: arrest has multiple active legal representations"
        );
        self.indexes
            .representations
            .active_by_contact
            .entry(record.contact())
            .or_default()
            .insert(id);
        let previous = self.legal_representations.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate legal representation ID inserted"
        );
    }
    pub(in crate::legal) fn end_legal_representation(
        &mut self,
        id: LegalRepresentationId,
        ended_at: SimTime,
        reason: LegalRepresentationEndReason,
        information: InformationId,
        report: ReportId,
    ) {
        let (arrest, contact, origin) = {
            let record = self
                .legal_representations
                .get(&id)
                .expect("validated legal representation disappeared before end commit");
            (record.arrest(), record.contact(), record.origin())
        };
        let removed = self
            .indexes
            .representations
            .active_by_arrest
            .remove(&arrest);
        debug_assert_eq!(removed, Some(id));
        if let Some(ids) = self
            .indexes
            .representations
            .active_by_contact
            .get_mut(&contact)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes
                    .representations
                    .active_by_contact
                    .remove(&contact);
            }
        }
        if origin == LegalRepresentationOrigin::AutomaticPolicy {
            let removed_automatic = self
                .indexes
                .representations
                .active_automatic_policy
                .remove(&id);
            debug_assert!(
                removed_automatic,
                "Derived Data Consistency: ended automatic-policy representation was not indexed"
            );
        }
        let record = self
            .legal_representations
            .get_mut(&id)
            .expect("validated legal representation disappeared before end commit");
        record.lifecycle.status = LegalRepresentationStatus::Ended;
        record.lifecycle.ended_at = Some(ended_at);
        record.lifecycle.end_reason = Some(reason);
        record.artifacts.ended_information = Some(information);
        record.artifacts.ended_report = Some(report);
        record.version = advance_version_preflighted(record.version);
    }
}
