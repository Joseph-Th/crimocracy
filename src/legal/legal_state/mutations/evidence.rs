//! Evidence insertion and case-subject promotion mutation/index maintenance.

use super::super::*;

impl LegalState {
    pub(in crate::legal) fn insert_evidence(
        &mut self,
        record: EvidenceRecord,
        activity_at: SimTime,
    ) {
        self.insert_evidence_with_work_exemption(record, activity_at, None);
    }

    pub(in crate::legal) fn insert_evidence_from_investigation_work(
        &mut self,
        record: EvidenceRecord,
        activity_at: SimTime,
        originating_work: InvestigationWorkId,
    ) {
        self.insert_evidence_with_work_exemption(record, activity_at, Some(originating_work));
    }

    fn insert_evidence_with_work_exemption(
        &mut self,
        record: EvidenceRecord,
        activity_at: SimTime,
        originating_work: Option<InvestigationWorkId>,
    ) {
        let investigation_id = record.investigation();
        let evidence_id = record.id();
        let actionable_character = if evidence_is_actionable_case_lead(&record) {
            record.subject().as_character()
        } else {
            None
        };
        let invalidated_witness = if let Some(character) = actionable_character {
            self.indexes
                .witnesses
                .case_witness_by_case_character
                .get(&(investigation_id, character))
                .copied()
        } else {
            None
        };
        let invalidated_lead = actionable_character.filter(|character| {
            self.investigations
                .get(&investigation_id)
                .is_some_and(|investigation| investigation.lead_investigator == Some(*character))
        });
        let invalidated_prosecution_cases: Vec<(ProsecutionCaseId, CharacterId)> =
            if let Some(prosecutor) = actionable_character {
                let cases: Vec<_> = self
                    .indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .get(&prosecutor)
                    .into_iter()
                    .flatten()
                    .copied()
                    .collect();
                cases
                    .into_iter()
                    .filter(|case| {
                        self.prosecution_cases
                            .get(case)
                            .expect("prosecutor-case index must reference a prosecution case")
                            .source_investigation()
                            == investigation_id
                    })
                    .map(|case| (case, prosecutor))
                    .collect()
            } else {
                Vec::new()
            };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before evidence commit");
        // Unusable or still-questionable material stays in the evidence graph without promoting
        // anyone to tracked-subject status. The investigation system owns the semantic threshold
        // because cold-case eligibility consumes the same definition of an actionable lead.
        if evidence_is_actionable_case_lead(&record) {
            investigation.subjects.insert(record.subject());
            self.indexes
                .investigations
                .investigations_by_subject
                .entry(record.subject())
                .or_default()
                .insert(record.investigation());
        }
        investigation.evidence.insert(record.id());
        investigation.version = advance_version_preflighted(investigation.version);
        for source in record.derived_from() {
            self.indexes
                .evidence
                .derived_evidence_by_source
                .entry(*source)
                .or_default()
                .insert(record.id());
        }
        let previous = self.evidence.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate evidence ID inserted"
        );
        if let Some(case_witness) = invalidated_witness {
            // Evidence is the causative case mutation and already advanced the investigation
            // version above. Cancel only the work record here so the composite state change has
            // one case revision while immediately releasing an interview that is now conflicted.
            self.cancel_scheduled_witness_interview_for_case_mutation(
                investigation_id,
                case_witness,
                activity_at,
                InvestigationWorkCancellationReason::WitnessBecameCaseSubject(evidence_id),
                originating_work,
            );
        }
        if let Some(investigator) = invalidated_lead {
            // The evidence insertion above already revised the case. Cancel any other pending
            // work and release the newly conflicted lead as parts of that same semantic change,
            // without manufacturing extra investigation revisions.
            self.cancel_scheduled_investigator_work_for_case_mutation(
                investigation_id,
                investigator,
                activity_at,
                InvestigationWorkCancellationReason::InvestigatorBecameCaseSubject(evidence_id),
                originating_work,
            );
            self.release_lead_investigator_for_case_mutation(investigation_id, investigator);
        }
        for (case, prosecutor) in invalidated_prosecution_cases {
            self.release_prosecution_case_prosecutor_runtime(case, prosecutor);
        }
        // Advance the case's last-activity instant to the commit minute, not the evidence's
        // discovery time: backdated evidence is legal (see validate_evidence_draft), but the case
        // still gained active work at the instant the evidence was actually added, so the
        // cold-case inactivity clock must reset to now.
        self.set_investigation_activity(investigation_id, activity_at);
    }
}
