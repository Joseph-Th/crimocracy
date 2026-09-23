//! Investigation-work scheduling, resolution, cancellation, and index maintenance.

use super::super::*;

impl LegalState {
    pub(in crate::legal) fn insert_investigation_work(&mut self, record: InvestigationWorkRecord) {
        let id = record.id();
        let investigation_id = record.investigation();
        let scheduled_at = record.scheduled_at();
        debug_assert_eq!(
            record.status(),
            InvestigationWorkStatus::Scheduled,
            "Lifecycle Validity: new investigation work must be scheduled"
        );
        self.indexes
            .work
            .work_by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        self.indexes
            .work
            .work_by_investigator
            .entry(record.investigator())
            .or_default()
            .insert(id);
        let previous_investigator = self
            .indexes
            .work
            .scheduled_work_by_investigator
            .insert(record.investigator(), id);
        debug_assert!(
            previous_investigator.is_none(),
            "Ownership Exclusivity: investigator received overlapping scheduled work"
        );
        if record.kind() == InvestigationWorkKind::EvidenceReview {
            let evidence = record
                .focus()
                .evidence_id()
                .expect("scheduled evidence review must have evidence focus");
            let evidence_record = self
                .evidence
                .get(&evidence)
                .expect("validated evidence review must reference persisted evidence");
            let previous_review = self
                .indexes
                .work
                .evidence_review_attempt_by_source
                .insert(evidence, id);
            debug_assert!(
                previous_review.is_none(),
                "Ownership Exclusivity: evidence received multiple live/completed review attempts"
            );
            let removed = self
                .indexes
                .work
                .unattempted_reviewable_evidence_by_investigation
                .get_mut(&record.investigation())
                .is_some_and(|unattempted| {
                    unattempted.remove(&(evidence_record.discovered_at(), evidence))
                });
            debug_assert!(
                removed,
                "validated evidence review must consume an unattempted reviewable source"
            );
            if self
                .indexes
                .work
                .unattempted_reviewable_evidence_by_investigation
                .get(&record.investigation())
                .is_some_and(BTreeSet::is_empty)
            {
                self.indexes
                    .work
                    .unattempted_reviewable_evidence_by_investigation
                    .remove(&record.investigation());
            }
        }
        self.indexes
            .work
            .scheduled_work_by_due_at
            .entry(record.due_at())
            .or_default()
            .insert(id);
        let previous_focus = self
            .indexes
            .work
            .scheduled_work_by_focus
            .insert((record.investigation(), record.kind(), record.focus()), id);
        debug_assert!(
            previous_focus.is_none(),
            "Ownership Exclusivity: duplicate scheduled investigation focus inserted"
        );
        let previous = self.investigation_work.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate investigation work ID inserted"
        );
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work insertion");
        investigation.version = advance_version_preflighted(investigation.version);
        self.set_investigation_activity(investigation_id, scheduled_at);
    }
    pub(in crate::legal) fn set_investigation_work_resolution(
        &mut self,
        id: InvestigationWorkId,
        resolution: InvestigationWorkResolution,
    ) {
        let resolved_at = resolution.resolved_at();
        let (due_at, focus_key, investigator) = {
            let record = self
                .investigation_work
                .get(&id)
                .expect("validated investigation work disappeared before completion");
            (
                record.due_at(),
                (record.investigation(), record.kind(), record.focus()),
                record.investigator(),
            )
        };
        if let Some(ids) = self.indexes.work.scheduled_work_by_due_at.get_mut(&due_at) {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes.work.scheduled_work_by_due_at.remove(&due_at);
            }
        }
        self.indexes.work.scheduled_work_by_focus.remove(&focus_key);
        let removed = self
            .indexes
            .work
            .scheduled_work_by_investigator
            .remove(&investigator);
        debug_assert_eq!(
            removed,
            Some(id),
            "completed work must own the investigator's scheduled-work slot"
        );
        let investigation_id = {
            let record = self
                .investigation_work
                .get_mut(&id)
                .expect("validated investigation work disappeared before completion");
            // Count a completed interview against its witness whether or not it produced a
            // statement, so scheduling can stop retrying witnesses who never open up.
            if record.kind() == InvestigationWorkKind::WitnessInterview
                && let Some(case_witness) = record.focus().witness_id()
            {
                let witness = self
                    .case_witnesses
                    .get_mut(&case_witness)
                    .expect("validated interview focus must reference an existing witness");
                witness.interview_attempts = witness
                    .interview_attempts
                    .checked_add(1)
                    .expect("interview attempt capacity must be preflighted before completion");
                witness.version = advance_version_preflighted(witness.version);
            }
            record.runtime.status = InvestigationWorkStatus::Completed;
            record.runtime.resolution = Some(resolution);
            record.runtime.cancellation = None;
            record.runtime.version = advance_version_preflighted(record.runtime.version);
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work completion");
        investigation.version = advance_version_preflighted(investigation.version);
        self.set_investigation_activity(investigation_id, resolved_at);
    }

    /// Cancels scheduled work while preserving the case's inactivity clock. Detention is an
    /// external availability event, not investigative progress; the investigation version still
    /// advances so every case-dependent plan observes the staffing/work change.
    pub(in crate::legal) fn set_investigation_work_cancellation(
        &mut self,
        id: InvestigationWorkId,
        cancellation: InvestigationWorkCancellation,
    ) {
        let investigation_id = self.set_investigation_work_cancellation_runtime(id, cancellation);
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work cancellation");
        investigation.version = advance_version_preflighted(investigation.version);
    }

    /// Cancels a witness interview whose invalidating cause is already advancing the case version
    /// in the same owner mutation. This prevents double-counting one semantic revision while still
    /// keeping work lifecycle/index state synchronized immediately.
    pub(super) fn cancel_scheduled_witness_interview_for_case_mutation(
        &mut self,
        investigation_id: InvestigationId,
        case_witness: CaseWitnessId,
        cancelled_at: SimTime,
        reason: InvestigationWorkCancellationReason,
        originating_work: Option<InvestigationWorkId>,
    ) {
        let focus = InvestigationWorkFocus::witness(case_witness);
        let Some(work) = self
            .indexes
            .work
            .scheduled_work_by_focus
            .get(&(
                investigation_id,
                InvestigationWorkKind::WitnessInterview,
                focus,
            ))
            .copied()
        else {
            return;
        };
        if Some(work) == originating_work {
            return;
        }
        self.set_investigation_work_cancellation_runtime(
            work,
            InvestigationWorkCancellation {
                cancelled_at,
                reason,
            },
        );
    }

    /// Cancels the investigator's scheduled work when the same case mutation makes that
    /// investigator a case subject. The causative evidence already advances the investigation.
    pub(super) fn cancel_scheduled_investigator_work_for_case_mutation(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
        cancelled_at: SimTime,
        reason: InvestigationWorkCancellationReason,
        originating_work: Option<InvestigationWorkId>,
    ) {
        let work = self
            .indexes
            .work
            .scheduled_work_by_investigator
            .get(&investigator)
            .copied()
            .filter(|work| {
                self.investigation_work
                    .get(work)
                    .is_some_and(|record| record.investigation() == investigation_id)
            });
        let Some(work) = work else {
            return;
        };
        if Some(work) == originating_work {
            return;
        }
        self.set_investigation_work_cancellation_runtime(
            work,
            InvestigationWorkCancellation {
                cancelled_at,
                reason,
            },
        );
    }

    /// Applies only the work-owned portion of cancellation and returns its investigation. Most
    /// callers use `set_investigation_work_cancellation`, which also advances the case. Composite
    /// witness/evidence mutations use this directly through the helper above because their
    /// causative mutation has already revised the same investigation.
    fn set_investigation_work_cancellation_runtime(
        &mut self,
        id: InvestigationWorkId,
        cancellation: InvestigationWorkCancellation,
    ) -> InvestigationId {
        let (due_at, focus_key, investigator, review_source) = {
            let record = self
                .investigation_work
                .get(&id)
                .expect("validated investigation work disappeared before cancellation");
            (
                record.due_at(),
                (record.investigation(), record.kind(), record.focus()),
                record.investigator(),
                (record.kind() == InvestigationWorkKind::EvidenceReview)
                    .then(|| record.focus().evidence_id())
                    .flatten(),
            )
        };
        if let Some(ids) = self.indexes.work.scheduled_work_by_due_at.get_mut(&due_at) {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes.work.scheduled_work_by_due_at.remove(&due_at);
            }
        }
        self.indexes.work.scheduled_work_by_focus.remove(&focus_key);
        let removed = self
            .indexes
            .work
            .scheduled_work_by_investigator
            .remove(&investigator);
        debug_assert_eq!(
            removed,
            Some(id),
            "cancelled work must own the investigator's scheduled-work slot"
        );
        if let Some(evidence) = review_source {
            let removed = self
                .indexes
                .work
                .evidence_review_attempt_by_source
                .remove(&evidence);
            debug_assert_eq!(
                removed,
                Some(id),
                "cancelled evidence review must own the source's attempt slot"
            );
            let evidence_record = self
                .evidence
                .get(&evidence)
                .expect("cancelled evidence review must retain its source evidence");
            self.indexes
                .work
                .unattempted_reviewable_evidence_by_investigation
                .entry(focus_key.0)
                .or_default()
                .insert((evidence_record.discovered_at(), evidence));
        }
        {
            let record = self
                .investigation_work
                .get_mut(&id)
                .expect("validated investigation work disappeared before cancellation");
            record.runtime.status = InvestigationWorkStatus::Cancelled;
            record.runtime.resolution = None;
            record.runtime.cancellation = Some(cancellation);
            record.runtime.version = advance_version_preflighted(record.runtime.version);
            record.investigation()
        }
    }
}
