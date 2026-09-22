//! Case-witness and witness-statement mutation/index maintenance.

use super::super::*;

impl LegalState {
    /// Registers a case witness and resets the case's cold-case inactivity clock: witness
    /// registration is consequence-bearing (cooperation drives future interview support).
    pub(in crate::legal) fn insert_case_witness(
        &mut self,
        record: CaseWitnessRecord,
        activity_at: SimTime,
    ) {
        let id = record.id();
        let investigation_id = record.investigation();
        let key = (investigation_id, record.witness());
        let previous_key = self
            .indexes
            .witnesses
            .case_witness_by_case_character
            .insert(key, id);
        debug_assert!(
            previous_key.is_none(),
            "Ownership Exclusivity: duplicate witness registration inserted for one investigation"
        );
        self.indexes
            .witnesses
            .case_witnesses_by_investigation
            .entry(investigation_id)
            .or_default()
            .insert(id);
        self.indexes
            .witnesses
            .case_witnesses_by_character
            .entry(record.witness())
            .or_default()
            .insert(id);
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before witness registration");
        investigation.version = advance_version_preflighted(investigation.version);
        let previous = self.case_witnesses.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate case witness ID inserted"
        );
        self.set_investigation_activity(investigation_id, activity_at);
    }
    /// Updates witness cooperation without manufacturing institutional case activity. Cooperation
    /// changes can be caused externally (for example by intimidation), so they invalidate
    /// case-dependent plans through the investigation version but do not refresh `last_activity_at`.
    pub(in crate::legal) fn set_witness_cooperation(
        &mut self,
        case_witness: CaseWitnessId,
        cooperation: WitnessCooperation,
    ) {
        let investigation_id = {
            let record = self
                .case_witnesses
                .get_mut(&case_witness)
                .expect("validated case witness disappeared before cooperation commit");
            record.cooperation = cooperation;
            record.version = advance_version_preflighted(record.version);
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before witness cooperation commit");
        investigation.version = advance_version_preflighted(investigation.version);
    }
    pub(in crate::legal) fn insert_witness_statement(&mut self, record: WitnessStatementRecord) {
        self.insert_witness_statement_with_work_exemption(record, None);
    }

    pub(in crate::legal) fn insert_witness_statement_from_investigation_work(
        &mut self,
        record: WitnessStatementRecord,
        originating_work: InvestigationWorkId,
    ) {
        self.insert_witness_statement_with_work_exemption(record, Some(originating_work));
    }

    fn insert_witness_statement_with_work_exemption(
        &mut self,
        record: WitnessStatementRecord,
        originating_work: Option<InvestigationWorkId>,
    ) {
        let id = record.id();
        let evidence = record.evidence();
        let case_witness = record.case_witness();
        let recorded_at = record.recorded_at();
        let witness = self
            .case_witnesses
            .get_mut(&case_witness)
            .expect("validated case witness disappeared before statement commit");
        let investigation_id = witness.investigation();
        witness.statements.insert(id);
        witness.version = advance_version_preflighted(witness.version);
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before statement commit");
        investigation.version = advance_version_preflighted(investigation.version);
        let previous_evidence = self
            .indexes
            .witnesses
            .witness_statement_by_evidence
            .insert(evidence, id);
        debug_assert!(
            previous_evidence.is_none(),
            "Ownership Exclusivity: evidence is linked to multiple witness statements"
        );
        let previous = self.witness_statements.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate witness statement ID inserted"
        );
        // A statement entered outside a scheduled interview makes any pending interview for the
        // same witness redundant. Statement insertion already advanced the investigation version,
        // so cancel only the work record as part of this one composite case mutation.
        self.cancel_scheduled_witness_interview_for_case_mutation(
            investigation_id,
            case_witness,
            recorded_at,
            InvestigationWorkCancellationReason::WitnessStatementRecorded(id),
            originating_work,
        );
    }
}
