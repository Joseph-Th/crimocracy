//! Index-consistency checks for investigations, evidence, work, witnesses, and informants.

use crate::legal::legal_state::LegalState;
use crate::legal::records::{InvestigationStatus, InvestigationWorkKind, InvestigationWorkStatus};

impl LegalState {
    pub(super) fn has_consistent_informant_indexes(&self) -> bool {
        for informant in self.informants.values() {
            let id = informant.id();
            let pair_index = self
                .indexes
                .informants
                .by_character_handler
                .get(&(informant.character(), informant.handler()));
            if pair_index != Some(&id)
                || !self
                    .indexes
                    .informants
                    .by_handler
                    .get(&informant.handler())
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
        }
        for (key, id) in &self.indexes.informants.by_character_handler {
            if !self
                .informants
                .get(id)
                .is_some_and(|record| (record.character(), record.handler()) == *key)
            {
                return false;
            }
        }
        for (handler, ids) in &self.indexes.informants.by_handler {
            for id in ids {
                if !self
                    .informants
                    .get(id)
                    .is_some_and(|record| record.handler() == *handler)
                {
                    return false;
                }
            }
        }
        for disclosure in self.informant_disclosures.values() {
            if !self.informants.contains_key(&disclosure.informant())
                || !self.evidence.contains_key(&disclosure.evidence())
                || self
                    .indexes
                    .informants
                    .disclosure_by_case_information
                    .get(&(disclosure.investigation(), disclosure.source_information()))
                    != Some(&disclosure.id())
            {
                return false;
            }
        }
        for (key, disclosure) in &self.indexes.informants.disclosure_by_case_information {
            if !self
                .informant_disclosures
                .get(disclosure)
                .is_some_and(|record| (record.investigation(), record.source_information()) == *key)
            {
                return false;
            }
        }
        true
    }

    pub(super) fn has_consistent_witness_indexes(&self) -> bool {
        for witness in self.case_witnesses.values() {
            if self
                .indexes
                .witnesses
                .case_witness_by_case_character
                .get(&(witness.investigation(), witness.witness()))
                != Some(&witness.id())
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
                    .is_some_and(|ids| ids.contains(&witness.id()))
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id()))
            {
                return false;
            }
            for statement in witness.statements() {
                if !self
                    .witness_statements
                    .get(statement)
                    .is_some_and(|record| record.case_witness() == witness.id())
                {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.witnesses.case_witness_by_case_character {
            if !self
                .case_witnesses
                .get(id)
                .is_some_and(|record| (record.investigation(), record.witness()) == *key)
            {
                return false;
            }
        }
        for (investigation, ids) in &self.indexes.witnesses.case_witnesses_by_investigation {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        for (character, ids) in &self.indexes.witnesses.case_witnesses_by_character {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.witness() == *character)
                {
                    return false;
                }
            }
        }
        for statement in self.witness_statements.values() {
            if !self
                .case_witnesses
                .get(&statement.case_witness())
                .is_some_and(|witness| witness.statements().contains(&statement.id()))
                || self
                    .indexes
                    .witnesses
                    .witness_statement_by_evidence
                    .get(&statement.evidence())
                    != Some(&statement.id())
                || !self.evidence.contains_key(&statement.evidence())
            {
                return false;
            }
        }
        for (evidence, statement) in &self.indexes.witnesses.witness_statement_by_evidence {
            if !self
                .witness_statements
                .get(statement)
                .is_some_and(|record| record.evidence() == *evidence)
            {
                return false;
            }
        }
        true
    }

    pub(super) fn has_consistent_investigation_indexes(&self) -> bool {
        self.investigation_forward_indexes_are_consistent()
            && self.active_investigation_indexes_are_consistent()
            && self.suspended_originated_index_is_consistent()
            && self.investigation_activity_index_is_consistent()
            && self.investigation_owner_index_is_consistent()
            && self.investigation_subject_index_is_consistent()
            && self.investigator_index_is_consistent()
    }

    /// Every investigation must appear in each projection implied by its authoritative record.
    fn investigation_forward_indexes_are_consistent(&self) -> bool {
        for investigation in self.investigations.values() {
            if !self
                .indexes
                .investigations
                .by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
            for subject in investigation.subjects() {
                if !self
                    .indexes
                    .investigations
                    .investigations_by_subject
                    .get(subject)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
                {
                    return false;
                }
            }
            for evidence_id in investigation.evidence() {
                if !self
                    .evidence
                    .get(evidence_id)
                    .is_some_and(|record| record.investigation() == investigation.id())
                {
                    return false;
                }
            }
            let should_need_lead = investigation.status() == InvestigationStatus::Active
                && investigation.lead_investigator().is_none();
            if self
                .indexes
                .investigations
                .active_without_lead
                .contains(&investigation.id())
                != should_need_lead
            {
                return false;
            }
            if self
                .indexes
                .investigations
                .active
                .contains(&investigation.id())
                != (investigation.status() == InvestigationStatus::Active)
            {
                return false;
            }
            if self
                .indexes
                .investigations
                .active_by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
                != (investigation.status() == InvestigationStatus::Active)
            {
                return false;
            }
            if self
                .indexes
                .investigations
                .suspended_originated_by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
                != (investigation.status() == InvestigationStatus::Suspended
                    && investigation.origin().is_some())
            {
                return false;
            }
            if let Some(investigator) = investigation.lead_investigator()
                && !self
                    .indexes
                    .investigations
                    .investigations_by_investigator
                    .get(&investigator)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
        }
        for (owner, investigations) in &self.indexes.investigations.active_by_owner {
            for investigation in investigations {
                if !self
                    .investigations
                    .get(investigation)
                    .is_some_and(|record| {
                        record.status() == InvestigationStatus::Active && record.owner() == *owner
                    })
                {
                    return false;
                }
            }
        }
        true
    }

    /// Incident continuation may consult only suspended files with real operation/enterprise
    /// provenance, grouped under the institution that owns the shelf.
    fn suspended_originated_index_is_consistent(&self) -> bool {
        for (owner, ids) in &self.indexes.investigations.suspended_originated_by_owner {
            if ids.is_empty() {
                return false;
            }
            for id in ids {
                if !self.investigations.get(id).is_some_and(|record| {
                    record.owner() == *owner
                        && record.status() == InvestigationStatus::Suspended
                        && record.origin().is_some()
                }) {
                    return false;
                }
            }
        }
        true
    }

    /// Active-case membership and the empty-lead subset must resolve back to matching records.
    fn active_investigation_indexes_are_consistent(&self) -> bool {
        for investigation in &self.indexes.investigations.active_without_lead {
            if !self
                .investigations
                .get(investigation)
                .is_some_and(|record| {
                    record.status() == InvestigationStatus::Active
                        && record.lead_investigator().is_none()
                })
            {
                return false;
            }
        }
        for investigation in &self.indexes.investigations.active {
            if !self
                .investigations
                .get(investigation)
                .is_some_and(|record| record.status() == InvestigationStatus::Active)
            {
                return false;
            }
        }
        true
    }

    /// The activity schedule is bidirectional and contains active investigations only.
    fn investigation_activity_index_is_consistent(&self) -> bool {
        for (at, ids) in &self.indexes.investigations.cases_by_last_activity {
            for id in ids {
                if !self.investigations.get(id).is_some_and(|record| {
                    record.status() == InvestigationStatus::Active
                        && record.last_activity_at() == *at
                }) {
                    return false;
                }
            }
        }
        for investigation in self.investigations.values() {
            if investigation.status() != InvestigationStatus::Active {
                continue;
            }
            if !self
                .indexes
                .investigations
                .cases_by_last_activity
                .get(&investigation.last_activity_at())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
        }
        true
    }

    /// Reverse owner entries must resolve to investigations owned by that institution.
    fn investigation_owner_index_is_consistent(&self) -> bool {
        for (owner, ids) in &self.indexes.investigations.by_owner {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.owner() == *owner)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse subject entries must agree with each investigation's effective subject set.
    fn investigation_subject_index_is_consistent(&self) -> bool {
        for (subject, ids) in &self.indexes.investigations.investigations_by_subject {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.subjects().contains(subject))
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse investigator entries must agree with current lead assignments.
    fn investigator_index_is_consistent(&self) -> bool {
        for (investigator, ids) in &self.indexes.investigations.investigations_by_investigator {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.lead_investigator() == Some(*investigator))
                {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn has_consistent_evidence_indexes(&self) -> bool {
        for evidence in self.evidence.values() {
            if !self
                .investigations
                .get(&evidence.investigation())
                .is_some_and(|investigation| investigation.evidence().contains(&evidence.id()))
            {
                return false;
            }
            for source in evidence.derived_from() {
                if !self
                    .indexes
                    .evidence
                    .derived_evidence_by_source
                    .get(source)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
        }
        for (source, ids) in &self.indexes.evidence.derived_evidence_by_source {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.derived_from().contains(source))
                {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn has_consistent_investigation_work_indexes(&self) -> bool {
        self.investigation_work_forward_indexes_are_consistent()
            && self.work_by_investigation_index_is_consistent()
            && self.work_by_investigator_index_is_consistent()
            && self.scheduled_work_investigator_index_is_consistent()
            && self.evidence_review_attempt_index_is_consistent()
            && self.scheduled_work_due_index_is_consistent()
            && self.scheduled_work_focus_index_is_consistent()
    }

    /// Every work record must agree with its owner/investigator projections and lifecycle-only
    /// schedule indexes.
    fn investigation_work_forward_indexes_are_consistent(&self) -> bool {
        for work in self.investigation_work.values() {
            if !self
                .indexes
                .work
                .work_by_investigation
                .get(&work.investigation())
                .is_some_and(|ids| ids.contains(&work.id()))
                || !self
                    .indexes
                    .work
                    .work_by_investigator
                    .get(&work.investigator())
                    .is_some_and(|ids| ids.contains(&work.id()))
            {
                return false;
            }
            let due_indexed = self
                .indexes
                .work
                .scheduled_work_by_due_at
                .get(&work.due_at())
                .is_some_and(|ids| ids.contains(&work.id()));
            let focus_indexed = self.indexes.work.scheduled_work_by_focus.get(&(
                work.investigation(),
                work.kind(),
                work.focus(),
            )) == Some(&work.id());
            let investigator_scheduled = self
                .indexes
                .work
                .scheduled_work_by_investigator
                .get(&work.investigator())
                == Some(&work.id());
            let review_attempt_indexed = work.focus().evidence_id().is_some_and(|evidence| {
                self.indexes
                    .work
                    .evidence_review_attempt_by_source
                    .get(&evidence)
                    == Some(&work.id())
            });
            if review_attempt_indexed
                != (work.kind() == InvestigationWorkKind::EvidenceReview
                    && work.status() != InvestigationWorkStatus::Cancelled)
            {
                return false;
            }
            match work.status() {
                InvestigationWorkStatus::Scheduled => {
                    if work.resolution().is_some()
                        || work.cancellation().is_some()
                        || !due_indexed
                        || !focus_indexed
                        || !investigator_scheduled
                    {
                        return false;
                    }
                }
                InvestigationWorkStatus::Completed => {
                    if work.resolution().is_none()
                        || work.cancellation().is_some()
                        || due_indexed
                        || focus_indexed
                        || investigator_scheduled
                    {
                        return false;
                    }
                }
                InvestigationWorkStatus::Cancelled => {
                    if work.resolution().is_some()
                        || work.cancellation().is_none()
                        || due_indexed
                        || focus_indexed
                        || investigator_scheduled
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Reverse case-work entries must resolve to work owned by that investigation.
    fn work_by_investigation_index_is_consistent(&self) -> bool {
        for (investigation, ids) in &self.indexes.work.work_by_investigation {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse investigator-work entries must resolve to work assigned to that investigator.
    fn work_by_investigator_index_is_consistent(&self) -> bool {
        for (investigator, ids) in &self.indexes.work.work_by_investigator {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigator() == *investigator)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Scheduled investigator capacity is exclusive and points only to live scheduled work.
    fn scheduled_work_investigator_index_is_consistent(&self) -> bool {
        for (investigator, id) in &self.indexes.work.scheduled_work_by_investigator {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.status() == InvestigationWorkStatus::Scheduled
                    && work.investigator() == *investigator
            }) {
                return false;
            }
        }
        true
    }

    /// Every indexed review attempt is a scheduled/completed evidence review of that exact source.
    fn evidence_review_attempt_index_is_consistent(&self) -> bool {
        for (evidence, id) in &self.indexes.work.evidence_review_attempt_by_source {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.kind() == InvestigationWorkKind::EvidenceReview
                    && work.status() != InvestigationWorkStatus::Cancelled
                    && work.focus().evidence_id() == Some(*evidence)
            }) {
                return false;
            }
        }
        true
    }

    /// Only scheduled work may occupy the due-time index, at its exact authored due time.
    fn scheduled_work_due_index_is_consistent(&self) -> bool {
        for (time, ids) in &self.indexes.work.scheduled_work_by_due_at {
            for id in ids {
                if !self.investigation_work.get(id).is_some_and(|work| {
                    work.status() == InvestigationWorkStatus::Scheduled && work.due_at() == *time
                }) {
                    return false;
                }
            }
        }
        true
    }

    /// The scheduled focus index is exclusive and must point to the exact active work tuple.
    fn scheduled_work_focus_index_is_consistent(&self) -> bool {
        for (key, id) in &self.indexes.work.scheduled_work_by_focus {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.status() == InvestigationWorkStatus::Scheduled
                    && (work.investigation(), work.kind(), work.focus()) == *key
            }) {
                return false;
            }
        }
        true
    }
}
