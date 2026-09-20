//! Investigation, evidence, witness, informant, and investigation-work mutation/index maintenance.

use super::super::*;

struct InvestigationStatusTransitionSnapshot {
    previous_status: InvestigationStatus,
    owner: OrganizationId,
    originated: bool,
    last_activity_at: SimTime,
    released_lead: Option<CharacterId>,
    needs_lead: bool,
}

impl LegalState {
    pub(in crate::legal) fn insert_investigation(&mut self, record: InvestigationRecord) {
        if record.status() == InvestigationStatus::Active && record.lead_investigator().is_none() {
            self.indexes
                .investigations
                .active_without_lead
                .insert(record.id());
        }
        self.indexes
            .investigations
            .by_owner
            .entry(record.owner())
            .or_default()
            .insert(record.id());
        for subject in record.subjects() {
            self.indexes
                .investigations
                .investigations_by_subject
                .entry(*subject)
                .or_default()
                .insert(record.id());
        }
        if record.status() == InvestigationStatus::Active {
            self.indexes.investigations.active.insert(record.id());
            self.indexes
                .investigations
                .active_by_owner
                .entry(record.owner())
                .or_default()
                .insert(record.id());
            self.indexes
                .investigations
                .cases_by_last_activity
                .entry(record.last_activity_at())
                .or_default()
                .insert(record.id());
        }
        let previous = self.investigations.insert(record.id(), record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate investigation ID inserted"
        );
    }

    /// Extends an existing investigation with the subject matter of a later validated incident.
    /// Declared subjects remain indexed even while some fresh evidence is still weak.
    pub(in crate::legal) fn extend_investigation_incident_subjects(
        &mut self,
        investigation_id: InvestigationId,
        subjects: BTreeSet<EntityRef>,
    ) {
        let mut added = Vec::new();
        {
            let investigation = self
                .investigations
                .get_mut(&investigation_id)
                .expect("validated investigation disappeared before subject merge");
            let mut declared_changed = false;
            for subject in subjects {
                declared_changed |= investigation.declared_subjects.insert(subject);
                if investigation.subjects.insert(subject) {
                    added.push(subject);
                }
            }
            if !declared_changed {
                return;
            }
            investigation.version = advance_version_preflighted(investigation.version);
        }
        for subject in added {
            self.indexes
                .investigations
                .investigations_by_subject
                .entry(subject)
                .or_default()
                .insert(investigation_id);
        }
    }

    /// Advances a case's last-activity instant and re-synchronizes the cold-decay index in one
    /// atomic step. Called by every consequence-bearing legal transition: incident intake,
    /// evidence insertion, investigation-work scheduling, and investigation-work resolution.
    pub(in crate::legal) fn set_investigation_activity(
        &mut self,
        investigation_id: InvestigationId,
        at: SimTime,
    ) {
        let previous_key = {
            let record = self
                .investigations
                .get_mut(&investigation_id)
                .expect("validated investigation disappeared before activity update");
            if record.status() != InvestigationStatus::Active || at <= record.last_activity_at {
                return;
            }
            let previous_key = record.last_activity_at;
            record.last_activity_at = at;
            previous_key
        };
        let ids = self
            .indexes
            .investigations
            .cases_by_last_activity
            .get_mut(&previous_key)
            .expect("active investigation must be indexed at its last activity instant");
        ids.remove(&investigation_id);
        if ids.is_empty() {
            self.indexes
                .investigations
                .cases_by_last_activity
                .remove(&previous_key);
        }
        self.indexes
            .investigations
            .cases_by_last_activity
            .entry(at)
            .or_default()
            .insert(investigation_id);
    }

    pub(in crate::legal) fn insert_informant(&mut self, record: InformantRecord) {
        let id = record.id();
        let key = (record.character(), record.handler());
        let handler = record.handler();
        let previous_pair = self.indexes.informants.by_character_handler.insert(key, id);
        debug_assert!(
            previous_pair.is_none(),
            "Ownership Exclusivity: duplicate informant relationship inserted"
        );
        self.indexes
            .informants
            .by_handler
            .entry(handler)
            .or_default()
            .insert(id);
        let previous = self.informants.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate informant ID inserted"
        );
    }
    pub(in crate::legal) fn insert_informant_disclosure(
        &mut self,
        evidence: EvidenceRecord,
        disclosure: InformantDisclosureRecord,
        activity_at: SimTime,
    ) {
        debug_assert_eq!(
            evidence.id(),
            disclosure.evidence(),
            "Record Reference Validity: informant disclosure evidence ID mismatch"
        );
        debug_assert_eq!(
            evidence.investigation(),
            disclosure.investigation(),
            "Ownership Exclusivity: informant disclosure belongs to a different case than its evidence"
        );
        self.insert_evidence(evidence, activity_at);
        let id = disclosure.id();
        let previous_case_information = self
            .indexes
            .informants
            .disclosure_by_case_information
            .insert(
                (disclosure.investigation(), disclosure.source_information()),
                id,
            );
        debug_assert!(
            previous_case_information.is_none(),
            "Ownership Exclusivity: source information was disclosed twice into one investigation"
        );
        let previous = self.informant_disclosures.insert(id, disclosure);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate informant disclosure ID inserted"
        );
    }
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
            let previous_review = self
                .indexes
                .work
                .evidence_review_attempt_by_source
                .insert(evidence, id);
            debug_assert!(
                previous_review.is_none(),
                "Ownership Exclusivity: evidence received multiple live/completed review attempts"
            );
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
    fn cancel_scheduled_witness_interview_for_case_mutation(
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
    fn cancel_scheduled_investigator_work_for_case_mutation(
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
    fn apply_investigation_status_record_transition(
        &mut self,
        investigation_id: InvestigationId,
        status: InvestigationStatus,
    ) -> InvestigationStatusTransitionSnapshot {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before lifecycle commit");
        let previous_status = investigation.status;
        let owner = investigation.owner;
        let originated = investigation.origin.is_some();
        let last_activity_at = investigation.last_activity_at;
        investigation.status = status;
        investigation.version = advance_version_preflighted(investigation.version);
        // Shelving or closing a case releases its lead: a case nobody works holds no
        // institutional attention, so its detective is free for other casework and a resumed
        // case re-enters the unstaffed index and is staffed again from available detectives.
        let released_lead = if status != InvestigationStatus::Active {
            investigation.lead_investigator.take()
        } else {
            None
        };
        let needs_lead = investigation.status == InvestigationStatus::Active
            && investigation.lead_investigator.is_none();
        InvestigationStatusTransitionSnapshot {
            previous_status,
            owner,
            originated,
            last_activity_at,
            released_lead,
            needs_lead,
        }
    }

    fn sync_released_investigation_lead(
        &mut self,
        investigation_id: InvestigationId,
        released_lead: Option<CharacterId>,
    ) {
        if let Some(released) = released_lead
            && let Some(cases) = self
                .indexes
                .investigations
                .investigations_by_investigator
                .get_mut(&released)
        {
            cases.remove(&investigation_id);
            if cases.is_empty() {
                self.indexes
                    .investigations
                    .investigations_by_investigator
                    .remove(&released);
            }
        }
    }

    fn sync_active_investigation_indexes(
        &mut self,
        investigation_id: InvestigationId,
        owner: OrganizationId,
        previous_status: InvestigationStatus,
        status: InvestigationStatus,
        needs_lead: bool,
    ) {
        if needs_lead {
            self.indexes
                .investigations
                .active_without_lead
                .insert(investigation_id);
        } else {
            self.indexes
                .investigations
                .active_without_lead
                .remove(&investigation_id);
        }
        let was_active = previous_status == InvestigationStatus::Active;
        let is_active = status == InvestigationStatus::Active;
        if is_active && !was_active {
            self.indexes.investigations.active.insert(investigation_id);
            self.indexes
                .investigations
                .active_by_owner
                .entry(owner)
                .or_default()
                .insert(investigation_id);
        } else if was_active && !is_active {
            self.indexes.investigations.active.remove(&investigation_id);
            if let Some(ids) = self.indexes.investigations.active_by_owner.get_mut(&owner) {
                ids.remove(&investigation_id);
                if ids.is_empty() {
                    self.indexes.investigations.active_by_owner.remove(&owner);
                }
            }
        }
    }

    fn sync_suspended_originated_investigation_index(
        &mut self,
        investigation_id: InvestigationId,
        owner: OrganizationId,
        previous_status: InvestigationStatus,
        status: InvestigationStatus,
        originated: bool,
    ) {
        if previous_status == InvestigationStatus::Active
            && status == InvestigationStatus::Suspended
            && originated
        {
            self.indexes
                .investigations
                .suspended_originated_by_owner
                .entry(owner)
                .or_default()
                .insert(investigation_id);
        } else if previous_status == InvestigationStatus::Suspended
            && status != InvestigationStatus::Suspended
            && originated
            && let Some(ids) = self
                .indexes
                .investigations
                .suspended_originated_by_owner
                .get_mut(&owner)
        {
            ids.remove(&investigation_id);
            if ids.is_empty() {
                self.indexes
                    .investigations
                    .suspended_originated_by_owner
                    .remove(&owner);
            }
        }
    }

    fn sync_investigation_activity_index_for_status(
        &mut self,
        investigation_id: InvestigationId,
        previous_status: InvestigationStatus,
        status: InvestigationStatus,
        at: SimTime,
        last_activity_at: SimTime,
    ) {
        match (previous_status, status) {
            // Suspending or closing an active case shelves it: it leaves the cold-decay index.
            // Resuming re-engages institutional interest, so the cold window restarts from the
            // resume instant rather than the stale pre-suspension activity.
            (InvestigationStatus::Active, InvestigationStatus::Suspended)
            | (InvestigationStatus::Active, InvestigationStatus::Closed) => {
                let key = last_activity_at;
                if let Some(ids) = self
                    .indexes
                    .investigations
                    .cases_by_last_activity
                    .get_mut(&key)
                {
                    ids.remove(&investigation_id);
                    if ids.is_empty() {
                        self.indexes
                            .investigations
                            .cases_by_last_activity
                            .remove(&key);
                    }
                }
            }
            (InvestigationStatus::Suspended, InvestigationStatus::Active) => {
                self.investigations
                    .get_mut(&investigation_id)
                    .expect("validated investigation disappeared before resume commit")
                    .last_activity_at = at;
                self.indexes
                    .investigations
                    .cases_by_last_activity
                    .entry(at)
                    .or_default()
                    .insert(investigation_id);
            }
            // Closing a suspended case adds nothing: it left the cold-decay index when suspended.
            (InvestigationStatus::Suspended, InvestigationStatus::Closed) => {}
            // Every other combination is rejected by validate_investigation_transition_dependencies
            // (closed cases are terminal, and no canonical path rewrites an unchanged status).
            (previous, target) => debug_assert!(
                false,
                "unreachable investigation transition {previous:?} -> {target:?}"
            ),
        }
    }

    pub(in crate::legal) fn set_investigation_status(
        &mut self,
        investigation_id: InvestigationId,
        status: InvestigationStatus,
        at: SimTime,
    ) {
        let snapshot = self.apply_investigation_status_record_transition(investigation_id, status);
        self.sync_released_investigation_lead(investigation_id, snapshot.released_lead);
        self.sync_active_investigation_indexes(
            investigation_id,
            snapshot.owner,
            snapshot.previous_status,
            status,
            snapshot.needs_lead,
        );
        self.sync_suspended_originated_investigation_index(
            investigation_id,
            snapshot.owner,
            snapshot.previous_status,
            status,
            snapshot.originated,
        );
        self.sync_investigation_activity_index_for_status(
            investigation_id,
            snapshot.previous_status,
            status,
            at,
            snapshot.last_activity_at,
        );
    }
    /// Promotes an investigator to the case's lead seat. Staffing is single-seat: every
    /// canonical producer assigns exactly one lead, and support-investigator bookkeeping does
    /// not exist, so the seat is only ever filled, never demoted in place.
    pub(in crate::legal) fn set_lead_investigator(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing commit");
        investigation.lead_investigator = Some(investigator);
        investigation.version = advance_version_preflighted(investigation.version);
        self.indexes
            .investigations
            .active_without_lead
            .remove(&investigation_id);
        self.indexes
            .investigations
            .investigations_by_investigator
            .entry(investigator)
            .or_default()
            .insert(investigation_id);
    }

    /// Releases the single active lead seat because custody made that investigator unavailable.
    /// The case itself remains active and immediately re-enters the unstaffed index so normal
    /// institutional staffing can assign another eligible detective on the same simulation tick.
    pub(in crate::legal) fn release_lead_investigator_for_detention(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        self.release_lead_investigator_runtime(investigation_id, investigator, true);
    }

    fn release_lead_investigator_for_case_mutation(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        self.release_lead_investigator_runtime(investigation_id, investigator, false);
    }

    fn release_lead_investigator_runtime(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
        advance_case_version: bool,
    ) {
        let record = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing release");
        assert_eq!(record.status, InvestigationStatus::Active);
        assert_eq!(record.lead_investigator, Some(investigator));
        record.lead_investigator = None;
        if advance_case_version {
            record.version = advance_version_preflighted(record.version);
        }
        if let Some(cases) = self
            .indexes
            .investigations
            .investigations_by_investigator
            .get_mut(&investigator)
        {
            let removed = cases.remove(&investigation_id);
            debug_assert!(
                removed,
                "released lead must be present in investigator index"
            );
            if cases.is_empty() {
                self.indexes
                    .investigations
                    .investigations_by_investigator
                    .remove(&investigator);
            }
        }
        self.indexes
            .investigations
            .active_without_lead
            .insert(investigation_id);
    }
}
