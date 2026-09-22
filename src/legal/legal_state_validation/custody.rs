//! Index-consistency checks for custody, representation, and prosecution state.

use crate::legal::legal_state::LegalState;
use crate::legal::records::{
    ArrestStatus, LegalRepresentationOrigin, LegalRepresentationStatus, ProsecutionCaseStatus,
};

impl LegalState {
    pub(super) fn has_consistent_prosecution_indexes(&self) -> bool {
        for case in self.prosecution_cases.values() {
            let id = case.id();
            let assignment_indexed = match (case.status(), case.assigned_prosecutor()) {
                (ProsecutionCaseStatus::Reviewing, Some(prosecutor)) => self
                    .indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .get(&prosecutor)
                    .is_some_and(|ids| ids.contains(&id)),
                (ProsecutionCaseStatus::Reviewing, None) => self
                    .indexes
                    .prosecutions
                    .reviewing_without_prosecutor
                    .contains(&id),
                (ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed, None) => true,
                (ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed, Some(_)) => false,
            };
            if !assignment_indexed
                || case.referrals().iter().any(|referral| {
                    !self
                        .indexes
                        .prosecutions
                        .referrals_by_case
                        .get(&id)
                        .is_some_and(|ids| ids.contains(referral))
                })
            {
                return false;
            }
            let open = self
                .indexes
                .prosecutions
                .open_by_arrest_office
                .get(&(case.arrest(), case.prosecutor_office()));
            match case.status() {
                ProsecutionCaseStatus::Reviewing if open != Some(&id) => return false,
                ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed
                    if open == Some(&id) =>
                {
                    return false;
                }
                ProsecutionCaseStatus::Reviewing
                | ProsecutionCaseStatus::Declined
                | ProsecutionCaseStatus::Closed => {}
            }
        }
        for referral in self.prosecution_referrals.values() {
            let case = match self.prosecution_cases.get(&referral.prosecution_case()) {
                Some(case) => case,
                None => return false,
            };
            if !case.referrals().contains(&referral.id())
                || !referral.evidence().is_subset(case.evidence())
                || !self
                    .indexes
                    .prosecutions
                    .referrals_by_case
                    .get(&case.id())
                    .is_some_and(|ids| ids.contains(&referral.id()))
            {
                return false;
            }
        }
        for (key, id) in &self.indexes.prosecutions.open_by_arrest_office {
            if !self.prosecution_cases.get(id).is_some_and(|case| {
                (case.arrest(), case.prosecutor_office()) == *key
                    && case.status() == ProsecutionCaseStatus::Reviewing
            }) {
                return false;
            }
        }
        for (prosecutor, ids) in &self.indexes.prosecutions.reviewing_cases_by_prosecutor {
            if ids.iter().any(|id| {
                !self.prosecution_cases.get(id).is_some_and(|case| {
                    case.status() == ProsecutionCaseStatus::Reviewing
                        && case.assigned_prosecutor() == Some(*prosecutor)
                })
            }) {
                return false;
            }
        }
        for id in &self.indexes.prosecutions.reviewing_without_prosecutor {
            if !self.prosecution_cases.get(id).is_some_and(|case| {
                case.status() == ProsecutionCaseStatus::Reviewing
                    && case.assigned_prosecutor().is_none()
            }) {
                return false;
            }
        }
        true
    }
    pub(super) fn has_consistent_legal_representation_indexes(&self) -> bool {
        for record in self.legal_representations.values() {
            let id = record.id();
            let arrest_active = self
                .indexes
                .representations
                .active_by_arrest
                .get(&record.arrest());
            let contact_active = self
                .indexes
                .representations
                .active_by_contact
                .get(&record.contact())
                .is_some_and(|ids| ids.contains(&id));
            match record.status() {
                LegalRepresentationStatus::Active
                    if arrest_active != Some(&id) || !contact_active =>
                {
                    return false;
                }
                LegalRepresentationStatus::Ended
                    if arrest_active == Some(&id) || contact_active =>
                {
                    return false;
                }
                LegalRepresentationStatus::Active | LegalRepresentationStatus::Ended => {}
            }
            if self
                .indexes
                .representations
                .active_automatic_policy
                .contains(&id)
                != (record.status() == LegalRepresentationStatus::Active
                    && record.origin() == LegalRepresentationOrigin::AutomaticPolicy)
            {
                return false;
            }
        }
        for (arrest, id) in &self.indexes.representations.active_by_arrest {
            if !self.legal_representations.get(id).is_some_and(|record| {
                record.arrest() == *arrest && record.status() == LegalRepresentationStatus::Active
            }) {
                return false;
            }
        }
        for id in &self.indexes.representations.active_automatic_policy {
            if !self.legal_representations.get(id).is_some_and(|record| {
                record.status() == LegalRepresentationStatus::Active
                    && record.origin() == LegalRepresentationOrigin::AutomaticPolicy
            }) {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.representations.active_by_contact {
            if ids.iter().any(|id| {
                !self.legal_representations.get(id).is_some_and(|record| {
                    record.contact() == *contact
                        && record.status() == LegalRepresentationStatus::Active
                })
            }) {
                return false;
            }
        }
        true
    }
    pub(super) fn has_consistent_arrest_indexes(&self) -> bool {
        for arrest in self.arrests.values() {
            let id = arrest.id();
            if !self
                .indexes
                .arrests
                .by_investigation
                .get(&arrest.investigation())
                .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            let active = self
                .indexes
                .arrests
                .active_by_character
                .get(&arrest.character());
            match arrest.status() {
                ArrestStatus::Detained if active != Some(&id) => return false,
                ArrestStatus::Released if active == Some(&id) => return false,
                ArrestStatus::Detained | ArrestStatus::Released => {}
            }
            if self.indexes.arrests.detained.contains(&id)
                != (arrest.status() == ArrestStatus::Detained)
            {
                return false;
            }
            let chronology_indexed = self
                .indexes
                .arrests
                .detained_by_arrested_at
                .get(&arrest.arrested_at())
                .is_some_and(|ids| ids.contains(&id));
            if chronology_indexed != (arrest.status() == ArrestStatus::Detained) {
                return false;
            }
        }
        for (investigation, ids) in &self.indexes.arrests.by_investigation {
            if ids.iter().any(|id| {
                !self
                    .arrests
                    .get(id)
                    .is_some_and(|record| record.investigation() == *investigation)
            }) {
                return false;
            }
        }
        for (character, id) in &self.indexes.arrests.active_by_character {
            if !self.arrests.get(id).is_some_and(|record| {
                record.character() == *character && record.status() == ArrestStatus::Detained
            }) {
                return false;
            }
        }
        for id in &self.indexes.arrests.detained {
            if !self
                .arrests
                .get(id)
                .is_some_and(|record| record.status() == ArrestStatus::Detained)
            {
                return false;
            }
        }
        for (arrested_at, ids) in &self.indexes.arrests.detained_by_arrested_at {
            if ids.iter().any(|id| {
                !self.arrests.get(id).is_some_and(|record| {
                    record.status() == ArrestStatus::Detained
                        && record.arrested_at() == *arrested_at
                })
            }) {
                return false;
            }
        }
        true
    }
}
