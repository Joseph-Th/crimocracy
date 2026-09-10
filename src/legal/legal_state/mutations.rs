//! Authoritative legal-record mutation and synchronized derived-index maintenance.

use super::*;

impl LegalState {
    pub(crate) fn insert_investigation(&mut self, record: InvestigationRecord) {
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

    /// Extends an existing investigation with the full context of a later validated incident:
    /// declared subjects remain indexed even while some fresh evidence is still weak, and every
    /// explicit external notification recipient retains the same case-visibility sightline they
    /// would have received if the incident had opened a new file.
    pub(crate) fn extend_investigation_incident_context(
        &mut self,
        investigation_id: InvestigationId,
        subjects: BTreeSet<EntityRef>,
        notified_organizations: BTreeSet<OrganizationId>,
    ) {
        let mut added = Vec::new();
        {
            let investigation = self
                .investigations
                .get_mut(&investigation_id)
                .expect("validated investigation disappeared before subject merge");
            for subject in subjects {
                if investigation.subjects.insert(subject) {
                    added.push(subject);
                }
            }
            let mut notifications_changed = false;
            for organization in notified_organizations {
                notifications_changed |= investigation.notified_organizations.insert(organization);
            }
            if added.is_empty() && !notifications_changed {
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
    pub(crate) fn set_investigation_activity(
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

    pub(crate) fn insert_informant(&mut self, record: InformantRecord) {
        let id = record.id();
        let key = (record.character(), record.handler());
        let previous_pair = self.indexes.informants.by_character_handler.insert(key, id);
        debug_assert!(
            previous_pair.is_none(),
            "Ownership Exclusivity: duplicate informant relationship inserted"
        );
        let previous = self.informants.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate informant ID inserted"
        );
    }
    pub(crate) fn insert_informant_disclosure(
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
    pub(crate) fn insert_evidence(&mut self, record: EvidenceRecord, activity_at: SimTime) {
        let investigation_id = record.investigation();
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
        // Advance the case's last-activity instant to the commit minute, not the evidence's
        // discovery time: backdated evidence is legal (see validate_evidence_draft), but the case
        // still gained active work at the instant the evidence was actually added, so the
        // cold-case inactivity clock must reset to now.
        self.set_investigation_activity(investigation_id, activity_at);
    }
    /// Registers a case witness and resets the case's cold-case inactivity clock: witness
    /// registration is consequence-bearing (cooperation drives future interview support).
    pub(crate) fn insert_case_witness(&mut self, record: CaseWitnessRecord, activity_at: SimTime) {
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
    pub(crate) fn set_witness_cooperation(
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
    pub(crate) fn insert_witness_statement(&mut self, record: WitnessStatementRecord) {
        let id = record.id();
        let evidence = record.evidence();
        let case_witness = record.case_witness();
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
    }
    pub(crate) fn insert_investigation_work(&mut self, record: InvestigationWorkRecord) {
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
    pub(crate) fn set_investigation_work_resolution(
        &mut self,
        id: InvestigationWorkId,
        resolution: InvestigationWorkResolution,
    ) {
        let resolved_at = resolution.resolved_at();
        let (due_at, focus_key) = {
            let record = self
                .investigation_work
                .get(&id)
                .expect("validated investigation work disappeared before completion");
            (
                record.due_at(),
                (record.investigation(), record.kind(), record.focus()),
            )
        };
        if let Some(ids) = self.indexes.work.scheduled_work_by_due_at.get_mut(&due_at) {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes.work.scheduled_work_by_due_at.remove(&due_at);
            }
        }
        self.indexes.work.scheduled_work_by_focus.remove(&focus_key);
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
    pub(crate) fn set_investigation_work_cancellation(
        &mut self,
        id: InvestigationWorkId,
        cancellation: InvestigationWorkCancellation,
    ) {
        let (due_at, focus_key) = {
            let record = self
                .investigation_work
                .get(&id)
                .expect("validated investigation work disappeared before cancellation");
            (
                record.due_at(),
                (record.investigation(), record.kind(), record.focus()),
            )
        };
        if let Some(ids) = self.indexes.work.scheduled_work_by_due_at.get_mut(&due_at) {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes.work.scheduled_work_by_due_at.remove(&due_at);
            }
        }
        self.indexes.work.scheduled_work_by_focus.remove(&focus_key);
        let investigation_id = {
            let record = self
                .investigation_work
                .get_mut(&id)
                .expect("validated investigation work disappeared before cancellation");
            record.runtime.status = InvestigationWorkStatus::Cancelled;
            record.runtime.resolution = None;
            record.runtime.cancellation = Some(cancellation);
            record.runtime.version = advance_version_preflighted(record.runtime.version);
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work cancellation");
        investigation.version = advance_version_preflighted(investigation.version);
    }
    pub(crate) fn set_investigation_status(
        &mut self,
        investigation_id: InvestigationId,
        status: InvestigationStatus,
        at: SimTime,
    ) {
        let previous_status = self
            .investigations
            .get(&investigation_id)
            .expect("validated investigation disappeared before lifecycle commit")
            .status;
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before lifecycle commit");
        investigation.status = status;
        investigation.version = advance_version_preflighted(investigation.version);
        // Shelving or closing a case releases its lead: a case nobody works holds no
        // institutional attention, so its detective is free for other casework and a resumed
        // case re-enters the unstaffed index and is staffed again from available detectives.
        if status != InvestigationStatus::Active
            && let Some(released) = investigation.lead_investigator.take()
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
        let needs_lead = investigation.status == InvestigationStatus::Active
            && investigation.lead_investigator.is_none();
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
        let is_active = investigation.status == InvestigationStatus::Active;
        if is_active && !was_active {
            self.indexes.investigations.active.insert(investigation_id);
        } else if was_active && !is_active {
            self.indexes.investigations.active.remove(&investigation_id);
        }
        match (previous_status, status) {
            // Suspending or closing an active case shelves it: it leaves the cold-decay index.
            // Resuming re-engages institutional interest, so the cold window restarts from the
            // resume instant rather than the stale pre-suspension activity.
            (InvestigationStatus::Active, InvestigationStatus::Suspended)
            | (InvestigationStatus::Active, InvestigationStatus::Closed) => {
                let key = investigation.last_activity_at;
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
    /// Promotes an investigator to the case's lead seat. Staffing is single-seat: every
    /// canonical producer assigns exactly one lead, and support-investigator bookkeeping does
    /// not exist, so the seat is only ever filled, never demoted in place.
    pub(crate) fn set_lead_investigator(
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
    pub(crate) fn release_lead_investigator_for_detention(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        let record = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before detention staffing release");
        assert_eq!(record.status, InvestigationStatus::Active);
        assert_eq!(record.lead_investigator, Some(investigator));
        record.lead_investigator = None;
        record.version = advance_version_preflighted(record.version);
        if let Some(cases) = self
            .indexes
            .investigations
            .investigations_by_investigator
            .get_mut(&investigator)
        {
            let removed = cases.remove(&investigation_id);
            debug_assert!(
                removed,
                "detained lead must be present in investigator index"
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
    pub(crate) fn set_jurisdiction(
        &mut self,
        organization: OrganizationId,
        neighborhoods: BTreeSet<NeighborhoodId>,
        case_intake_priority: crate::world::Rating,
        changed_at: SimTime,
    ) {
        let previous_neighborhoods = self
            .jurisdictions
            .get(&organization)
            .map(|previous| previous.neighborhoods().iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for neighborhood in previous_neighborhoods {
            if let Some(organizations) = self
                .indexes
                .jurisdictions
                .jurisdictions_by_neighborhood
                .get_mut(&neighborhood)
            {
                organizations.remove(&organization);
                if organizations.is_empty() {
                    self.indexes
                        .jurisdictions
                        .jurisdictions_by_neighborhood
                        .remove(&neighborhood);
                }
            }
        }
        for neighborhood in &neighborhoods {
            self.indexes
                .jurisdictions
                .jurisdictions_by_neighborhood
                .entry(*neighborhood)
                .or_default()
                .insert(organization);
        }
        if let Some(record) = self.jurisdictions.get_mut(&organization) {
            let version = advance_version_preflighted(record.version());
            record.revisions.push(crate::legal::JurisdictionRevision {
                changed_at,
                neighborhoods,
                case_intake_priority,
                version,
            });
        } else {
            self.jurisdictions.insert(
                organization,
                JurisdictionRecord {
                    organization,
                    revisions: vec![crate::legal::JurisdictionRevision {
                        changed_at,
                        neighborhoods,
                        case_intake_priority,
                        version: 1,
                    }],
                },
            );
        }
    }
    pub(crate) fn insert_patrol_deployment(&mut self, record: PatrolDeploymentRecord) {
        let id = record.id();
        let organization = record.organization();
        let neighborhood = record.neighborhood();
        debug_assert_eq!(
            record.status(),
            PatrolDeploymentStatus::Active,
            "Lifecycle Validity: new patrol deployments must be active"
        );
        self.indexes
            .patrols
            .by_neighborhood
            .entry(neighborhood)
            .or_default()
            .insert(id);
        let previous_active = self
            .indexes
            .patrols
            .active_by_organization_neighborhood
            .insert((organization, neighborhood), id);
        debug_assert!(
            previous_active.is_none(),
            "Ownership Exclusivity: duplicate active patrol deployment inserted"
        );
        self.indexes
            .patrols
            .active_by_neighborhood
            .entry(neighborhood)
            .or_default()
            .insert(id);
        let previous = self.patrol_deployments.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate patrol deployment ID inserted"
        );
    }
    pub(crate) fn revise_patrol_deployment(
        &mut self,
        id: PatrolDeploymentId,
        windows: Vec<PatrolWindow>,
        changed_at: SimTime,
    ) {
        let record = self
            .patrol_deployments
            .get_mut(&id)
            .expect("validated patrol deployment disappeared before revision commit");
        let status = record.status();
        let version = advance_version_preflighted(record.version());
        record
            .revisions
            .push(crate::legal::PatrolDeploymentRevision {
                changed_at,
                windows,
                status,
                version,
            });
    }
    pub(crate) fn set_patrol_deployment_status(
        &mut self,
        id: PatrolDeploymentId,
        status: PatrolDeploymentStatus,
        changed_at: SimTime,
    ) {
        let (organization, neighborhood, previous_status) = {
            let record = self
                .patrol_deployments
                .get(&id)
                .expect("validated patrol deployment disappeared before lifecycle commit");
            (
                record.organization(),
                record.neighborhood(),
                record.status(),
            )
        };
        debug_assert_ne!(
            previous_status, status,
            "Lifecycle Validity: patrol transition must change status"
        );
        if previous_status == PatrolDeploymentStatus::Active {
            let removed = self
                .indexes
                .patrols
                .active_by_organization_neighborhood
                .remove(&(organization, neighborhood));
            debug_assert_eq!(
                removed,
                Some(id),
                "Derived Data Consistency: active patrol index changed before lifecycle commit"
            );
            if let Some(ids) = self
                .indexes
                .patrols
                .active_by_neighborhood
                .get_mut(&neighborhood)
            {
                let removed = ids.remove(&id);
                debug_assert!(
                    removed,
                    "Derived Data Consistency: neighborhood active patrol index changed before lifecycle commit"
                );
                if ids.is_empty() {
                    self.indexes
                        .patrols
                        .active_by_neighborhood
                        .remove(&neighborhood);
                }
            }
        }
        if status == PatrolDeploymentStatus::Active {
            let previous = self
                .indexes
                .patrols
                .active_by_organization_neighborhood
                .insert((organization, neighborhood), id);
            debug_assert!(
                previous.is_none(),
                "Ownership Exclusivity: patrol resume collided with another active deployment"
            );
            self.indexes
                .patrols
                .active_by_neighborhood
                .entry(neighborhood)
                .or_default()
                .insert(id);
        }
        let record = self
            .patrol_deployments
            .get_mut(&id)
            .expect("validated patrol deployment disappeared before lifecycle commit");
        let windows = record.windows().to_vec();
        let version = advance_version_preflighted(record.version());
        record
            .revisions
            .push(crate::legal::PatrolDeploymentRevision {
                changed_at,
                windows,
                status,
                version,
            });
    }
    pub(crate) fn insert_police_response(&mut self, record: PoliceResponseRecord) {
        let id = record.id();
        let previous_operation = self
            .indexes
            .police_responses
            .by_source_operation
            .insert(record.source_operation(), id);
        debug_assert!(
            previous_operation.is_none(),
            "Ownership Exclusivity: operation has multiple police responses"
        );
        self.indexes
            .police_responses
            .dispatched_by_arrival_due
            .entry(record.arrival_due_at())
            .or_default()
            .insert(id);
        let previous = self.police_responses.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate police response ID inserted"
        );
    }
    pub(crate) fn set_police_response_arrived(&mut self, id: PoliceResponseId, at: SimTime) {
        let due_at = self
            .police_responses
            .get(&id)
            .expect("validated police response disappeared before arrival commit")
            .arrival_due_at();
        if let Some(ids) = self
            .indexes
            .police_responses
            .dispatched_by_arrival_due
            .get_mut(&due_at)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.indexes
                    .police_responses
                    .dispatched_by_arrival_due
                    .remove(&due_at);
            }
        }
        let record = self
            .police_responses
            .get_mut(&id)
            .expect("validated police response disappeared before arrival commit");
        record.state.status = PoliceResponseStatus::Arrived;
        record.timing.arrived_at = Some(at);
        record.state.version = advance_version_preflighted(record.state.version);
    }
    pub(crate) fn insert_arrest(&mut self, record: ArrestRecord) {
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
    pub(crate) fn release_arrest(&mut self, id: ArrestId, released_at: SimTime) {
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
    pub(crate) fn insert_legal_representation(&mut self, record: LegalRepresentationRecord) {
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
    pub(crate) fn end_legal_representation(
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
    pub(crate) fn insert_prosecution_case(
        &mut self,
        case: ProsecutionCaseRecord,
        referral: ProsecutionReferralRecord,
    ) {
        let case_id = case.id();
        let referral_id = referral.id();
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert_eq!(referral.prosecution_case(), case_id);
        debug_assert_eq!(case.initial_referral(), referral_id);
        debug_assert_eq!(case.referrals(), &BTreeSet::from([referral_id]));
        debug_assert_eq!(case.evidence(), referral.evidence());
        if let Some(prosecutor) = case.assigned_prosecutor() {
            self.indexes
                .prosecutions
                .reviewing_cases_by_prosecutor
                .entry(prosecutor)
                .or_default()
                .insert(case_id);
        } else {
            self.indexes
                .prosecutions
                .reviewing_without_prosecutor
                .insert(case_id);
        }
        let previous_open = self
            .indexes
            .prosecutions
            .open_by_arrest_office
            .insert((case.arrest(), case.prosecutor_office()), case_id);
        debug_assert!(previous_open.is_none());
        self.indexes
            .prosecutions
            .referrals_by_case
            .entry(case_id)
            .or_default()
            .insert(referral_id);
        let previous_case = self.prosecution_cases.insert(case_id, case);
        let previous_referral = self.prosecution_referrals.insert(referral_id, referral);
        debug_assert!(previous_case.is_none());
        debug_assert!(previous_referral.is_none());
    }
    pub(crate) fn add_prosecution_referral(&mut self, referral: ProsecutionReferralRecord) {
        let referral_id = referral.id();
        let case_id = referral.prosecution_case();
        let case = self
            .prosecution_cases
            .get_mut(&case_id)
            .expect("validated prosecution case disappeared before referral commit");
        for evidence in referral.evidence() {
            let inserted = case.referrals.evidence.insert(*evidence);
            debug_assert!(inserted, "supplemental referral must add new evidence");
        }
        case.referrals.referrals.insert(referral_id);
        case.version = advance_version_preflighted(case.version);
        self.indexes
            .prosecutions
            .referrals_by_case
            .entry(case_id)
            .or_default()
            .insert(referral_id);
        let previous = self.prosecution_referrals.insert(referral_id, referral);
        debug_assert!(previous.is_none());
    }
    pub(crate) fn apply_prosecution_resolution(
        &mut self,
        id: ProsecutionCaseId,
        resolution: ProsecutionCaseResolution,
        resolved_at: SimTime,
        prosecutor: CharacterId,
        information: InformationId,
        report: ReportId,
    ) {
        let (arrest, office, assigned) = {
            let case = self
                .prosecution_cases
                .get(&id)
                .expect("validated prosecution case disappeared before resolution commit");
            debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
            (
                case.arrest(),
                case.prosecutor_office(),
                case.assigned_prosecutor(),
            )
        };
        let removed = self
            .indexes
            .prosecutions
            .open_by_arrest_office
            .remove(&(arrest, office));
        debug_assert_eq!(removed, Some(id));
        match assigned {
            Some(assigned) => {
                debug_assert_eq!(assigned, prosecutor);
                if let Some(cases) = self
                    .indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .get_mut(&assigned)
                {
                    let removed = cases.remove(&id);
                    debug_assert!(removed);
                    if cases.is_empty() {
                        self.indexes
                            .prosecutions
                            .reviewing_cases_by_prosecutor
                            .remove(&assigned);
                    }
                }
            }
            None => {
                debug_assert!(
                    false,
                    "validated prosecution resolution must have an assignee"
                );
                self.indexes
                    .prosecutions
                    .reviewing_without_prosecutor
                    .remove(&id);
            }
        }
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before resolution commit");
        case.context.assigned_prosecutor = None;
        case.lifecycle.status = resolution.status();
        case.lifecycle.resolved_at = Some(resolved_at);
        case.resolution_artifacts.resolution_information = Some(information);
        case.resolution_artifacts.resolution_report = Some(report);
        case.resolution_artifacts.resolution_prosecutor = Some(prosecutor);
        case.version = advance_version_preflighted(case.version);
    }

    pub(crate) fn set_prosecution_case_prosecutor(
        &mut self,
        id: ProsecutionCaseId,
        prosecutor: CharacterId,
    ) {
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before staffing commit");
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert!(case.assigned_prosecutor().is_none());
        case.context.assigned_prosecutor = Some(prosecutor);
        case.version = advance_version_preflighted(case.version);
        let removed = self
            .indexes
            .prosecutions
            .reviewing_without_prosecutor
            .remove(&id);
        debug_assert!(removed);
        self.indexes
            .prosecutions
            .reviewing_cases_by_prosecutor
            .entry(prosecutor)
            .or_default()
            .insert(id);
    }

    pub(crate) fn release_prosecution_case_prosecutor_for_detention(
        &mut self,
        id: ProsecutionCaseId,
        prosecutor: CharacterId,
    ) {
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before detention staffing release");
        debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
        debug_assert_eq!(case.assigned_prosecutor(), Some(prosecutor));
        case.context.assigned_prosecutor = None;
        case.version = advance_version_preflighted(case.version);
        if let Some(cases) = self
            .indexes
            .prosecutions
            .reviewing_cases_by_prosecutor
            .get_mut(&prosecutor)
        {
            let removed = cases.remove(&id);
            debug_assert!(removed);
            if cases.is_empty() {
                self.indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .remove(&prosecutor);
            }
        }
        self.indexes
            .prosecutions
            .reviewing_without_prosecutor
            .insert(id);
    }
}
