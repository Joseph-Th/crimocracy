//! `LegalState` index-consistency validation; sibling `legal_state` owns records and mutators.
//!
//! These projection checks re-derive every legal index from the authoritative records and
//! must agree with them after any mutation, save restoration, or invariant audit.

mod casework;
mod custody;
mod policing;

use crate::legal::legal_state::LegalState;

impl LegalState {
    fn has_consistent_primary_keys(&self) -> bool {
        self.investigations
            .iter()
            .all(|(id, record)| *id == record.id())
            && self
                .investigation_work
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .case_witnesses
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .witness_statements
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .informants
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .informant_disclosures
                .iter()
                .all(|(id, record)| *id == record.id())
            && self.evidence.iter().all(|(id, record)| *id == record.id())
            && self
                .jurisdictions
                .iter()
                .all(|(organization, record)| *organization == record.organization())
            && self
                .patrol_deployments
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .police_responses
                .iter()
                .all(|(id, record)| *id == record.id())
            && self.arrests.iter().all(|(id, record)| *id == record.id())
            && self
                .legal_representations
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .prosecution_cases
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .prosecution_referrals
                .iter()
                .all(|(id, record)| *id == record.id())
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        self.has_consistent_primary_keys()
            && self.has_consistent_investigation_indexes()
            && self.has_consistent_investigation_work_indexes()
            && self.has_consistent_evidence_indexes()
            && self.has_consistent_witness_indexes()
            && self.has_consistent_informant_indexes()
            && self.has_consistent_arrest_indexes()
            && self.has_consistent_legal_representation_indexes()
            && self.has_consistent_prosecution_indexes()
            && self.has_consistent_jurisdiction_indexes()
            && self.has_consistent_patrol_indexes()
            && self.has_consistent_police_response_indexes()
    }
}
