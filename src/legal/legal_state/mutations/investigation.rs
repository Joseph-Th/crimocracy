//! Investigation record, activity, lifecycle, and lead-assignment mutation/index maintenance.

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

    pub(super) fn release_lead_investigator_for_case_mutation(
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
