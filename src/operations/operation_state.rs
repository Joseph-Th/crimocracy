//! Operation state storage and index maintenance; sibling `operations` types define records.

use crate::core::id::{BusinessId, InformationId, OperationId, OrganizationId, PoliceResponseId};
use crate::core::time::SimTime;
use crate::operations::{
    OperationAbortPhase, OperationAbortRecord, OperationCashDispositionRecord,
    OperationObjectiveOutcome, OperationPropertyDispositionRecord, OperationRecord,
    OperationResolutionRecord, OperationStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Minutes elapsed between a decision pause's start and its resumption instant. Shared by
/// resume validation and commit so pause arithmetic cannot drift between the two phases.
pub(crate) fn pause_duration_minutes(paused_at: SimTime, resumed_at: SimTime) -> u64 {
    resumed_at
        .as_minutes()
        .checked_sub(paused_at.as_minutes())
        .expect("operation cannot resume before its decision pause began")
}

/// Shifts a scheduled time forward across a decision pause; overflow breaks the simulation's
/// minute-count invariant. `what` names the shifted field for the panic message.
pub(crate) fn shift_past_pause(time: SimTime, paused_minutes: u64, what: &str) -> SimTime {
    SimTime::from_minutes(
        time.as_minutes()
            .checked_add(paused_minutes)
            .unwrap_or_else(|| panic!("operation {what} overflowed u64 minutes")),
    )
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OperationState {
    records: BTreeMap<OperationId, OperationRecord>,
    by_organization: BTreeMap<OrganizationId, BTreeSet<OperationId>>,
    /// Non-terminal operations per organization. Participant double-booking checks scan
    /// this instead of the organization's full operation history, which grows forever.
    active_by_organization: BTreeMap<OrganizationId, BTreeSet<OperationId>>,
    by_status: BTreeMap<OperationStatus, BTreeSet<OperationId>>,
    by_discovered_information: BTreeMap<InformationId, OperationId>,
    authorized_by_start: BTreeMap<SimTime, BTreeSet<OperationId>>,
    in_progress_by_resolution_due: BTreeMap<SimTime, BTreeSet<OperationId>>,
    /// Successful property/cash takes per target business as (resolved_at, operation_id).
    /// Feeds recency-depletion economics without scanning the full completed bucket, which
    /// grows for the life of the campaign.
    successful_takes_by_business: BTreeMap<BusinessId, BTreeSet<(SimTime, OperationId)>>,
}

impl OperationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn get_operation(&self, id: OperationId) -> Option<&OperationRecord> {
        self.records.get(&id)
    }

    pub fn operations_for_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.by_organization
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|operation_id| self.records.get(operation_id))
    }

    /// Non-terminal operations of an organization, in id order. This is the scan surface for
    /// participant availability: terminal operations release their participants and never
    /// block new bookings.
    pub(crate) fn active_operations_for_organization(
        &self,
        id: OrganizationId,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.active_by_organization
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|operation_id| self.records.get(operation_id))
    }

    pub fn operation_for_discovered_information(
        &self,
        information: InformationId,
    ) -> Option<&OperationRecord> {
        self.by_discovered_information
            .get(&information)
            .and_then(|operation| self.records.get(operation))
    }

    pub fn operations_with_status(
        &self,
        status: OperationStatus,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.by_status
            .get(&status)
            .into_iter()
            .flatten()
            .filter_map(|operation_id| self.records.get(operation_id))
    }

    pub(crate) fn operations(&self) -> impl Iterator<Item = &OperationRecord> {
        self.records.values()
    }

    pub(crate) fn due_authorized_at_or_before(&self, now: SimTime) -> Vec<OperationId> {
        self.authorized_by_start
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn due_in_progress_at_or_before(&self, now: SimTime) -> Vec<OperationId> {
        self.in_progress_by_resolution_due
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    pub(crate) fn insert(&mut self, record: OperationRecord) {
        let id = record.id();
        // Guard before any index mutation so a duplicate ID cannot pollute derived state in
        // a debug build; release builds rely on the monotonic ID allocator for uniqueness.
        debug_assert!(
            !self.records.contains_key(&id),
            "Index Uniqueness: duplicate operation ID inserted"
        );
        debug_assert_eq!(
            record.status(),
            OperationStatus::Authorized,
            "new operations must enter state as authorized"
        );
        self.by_organization
            .entry(record.responsible_organization())
            .or_default()
            .insert(id);
        self.active_by_organization
            .entry(record.responsible_organization())
            .or_default()
            .insert(id);
        self.by_status
            .entry(record.status())
            .or_default()
            .insert(id);
        self.authorized_by_start
            .entry(record.scheduled_for())
            .or_default()
            .insert(id);
        let previous = self.records.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "unreachable after the contains_key guard"
        );
    }

    pub(crate) fn begin(
        &mut self,
        id: OperationId,
        started_at: SimTime,
        resolution_due_at: SimTime,
        entry_at: Option<SimTime>,
        police_response: Option<PoliceResponseId>,
    ) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before begin commit");
        assert_eq!(
            record.status(),
            OperationStatus::Authorized,
            "only authorized operations may begin"
        );
        let scheduled_for = record.scheduled_for();
        Self::remove_schedule_index(&mut self.authorized_by_start, scheduled_for, id);
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before begin commit");
            record.runtime.started_at = Some(started_at);
            record.runtime.resolution_due_at = Some(resolution_due_at);
            record.runtime.entry_at = entry_at;
            record.runtime.police_response = police_response;
            record.runtime.awaiting_decision_since = None;
        }
        self.change_status(id, OperationStatus::InProgress);
        self.in_progress_by_resolution_due
            .entry(resolution_due_at)
            .or_default()
            .insert(id);
    }

    pub(crate) fn set_awaiting_decision(&mut self, id: OperationId, paused_at: SimTime) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before decision wait commit");
        assert_eq!(
            record.status(),
            OperationStatus::InProgress,
            "only in-progress operations may await a decision"
        );
        let due_at = record
            .resolution_due_at()
            .expect("in-progress operation must have a resolution due time");
        Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
        self.records
            .get_mut(&id)
            .expect("validated operation disappeared before decision wait commit")
            .runtime
            .awaiting_decision_since = Some(paused_at);
        self.change_status(id, OperationStatus::AwaitingDecision);
    }

    pub(crate) fn resume(&mut self, id: OperationId, resumed_at: SimTime) {
        let (due_at, entry_at, paused_at) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before resume commit");
            assert_eq!(
                record.status(),
                OperationStatus::AwaitingDecision,
                "only decision-blocked operations may resume"
            );
            (
                record
                    .resolution_due_at()
                    .expect("awaiting operation must retain its resolution due time"),
                record.entry_at(),
                record
                    .awaiting_decision_since()
                    .expect("awaiting operation must retain its pause time"),
            )
        };
        let paused_minutes = pause_duration_minutes(paused_at, resumed_at);
        let shifted_due_at = shift_past_pause(due_at, paused_minutes, "resolution time");
        let shifted_entry_at = entry_at.map(|entry_at| {
            if entry_at > paused_at {
                shift_past_pause(entry_at, paused_minutes, "entry time")
            } else {
                entry_at
            }
        });
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before resume commit");
            record.runtime.resolution_due_at = Some(shifted_due_at);
            record.runtime.entry_at = shifted_entry_at;
            record.runtime.awaiting_decision_since = None;
        }
        self.change_status(id, OperationStatus::InProgress);
        self.in_progress_by_resolution_due
            .entry(shifted_due_at)
            .or_default()
            .insert(id);
    }

    pub(crate) fn abort(&mut self, id: OperationId, abort: OperationAbortRecord) {
        let (status, scheduled_for, due_at) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before abort commit");
            (
                record.status(),
                record.scheduled_for(),
                record.resolution_due_at(),
            )
        };
        assert!(
            matches!(
                status,
                OperationStatus::Authorized
                    | OperationStatus::InProgress
                    | OperationStatus::AwaitingDecision
            ),
            "only active operations may abort"
        );
        match status {
            OperationStatus::Authorized => {
                Self::remove_schedule_index(&mut self.authorized_by_start, scheduled_for, id);
            }
            OperationStatus::InProgress => {
                let due_at = due_at.expect("in-progress operation must have a resolution due time");
                Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
            }
            OperationStatus::AwaitingDecision
            | OperationStatus::Completed
            | OperationStatus::Aborted => {}
        }
        if abort.phase() != OperationAbortPhase::AwaitingDecision {
            self.records
                .get_mut(&id)
                .expect("validated operation disappeared before abort commit")
                .runtime
                .awaiting_decision_since = None;
        }
        self.records
            .get_mut(&id)
            .expect("validated operation disappeared before abort commit")
            .runtime
            .abort = Some(abort);
        self.change_status(id, OperationStatus::Aborted);
    }

    pub(crate) fn complete(&mut self, id: OperationId, resolution: OperationResolutionRecord) {
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before completion commit");
        assert_eq!(
            record.status(),
            OperationStatus::InProgress,
            "only in-progress operations may complete"
        );
        assert!(
            record.abort_record().is_none(),
            "completed operations cannot retain an abort record"
        );
        let due_at = record
            .resolution_due_at()
            .expect("in-progress operation must have a resolution due time");
        for information in resolution.discovered_information() {
            let previous = self.by_discovered_information.insert(*information, id);
            debug_assert!(
                previous.is_none(),
                "Ownership Exclusivity: discovered information is linked to multiple operations"
            );
        }
        Self::remove_schedule_index(&mut self.in_progress_by_resolution_due, due_at, id);
        {
            let record = self
                .records
                .get_mut(&id)
                .expect("validated operation disappeared before completion commit");
            record.runtime.resolution = Some(resolution);
            record.runtime.awaiting_decision_since = None;
        }
        // A successful take against a business enters the recency-depletion index at its own
        // resolution instant, so later takes price the target without a full-history scan.
        let record = self
            .records
            .get(&id)
            .expect("validated operation disappeared before completion commit");
        let resolution = record
            .resolution()
            .expect("just-attached resolution must be present");
        if matches!(
            resolution.objective_outcome(),
            OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
        ) {
            if let Some(business) = record.objective().taken_business() {
                self.successful_takes_by_business
                    .entry(business)
                    .or_default()
                    .insert((resolution.resolved_at(), id));
            }
        }
        self.change_status(id, OperationStatus::Completed);
    }

    /// Successful takes against `business` resolved inside the recency window before `at`,
    /// excluding `exclude` itself. Served from the depletion index, ordered by resolution time.
    pub(crate) fn recent_successful_takes(
        &self,
        business: BusinessId,
        at: SimTime,
        window_minutes: i64,
        exclude: Option<OperationId>,
    ) -> u32 {
        let at_minutes = i64::try_from(at.as_minutes()).unwrap_or(i64::MAX);
        let lower_bound =
            SimTime::from_minutes(at_minutes.saturating_sub(window_minutes).max(0) as u64);
        self.successful_takes_by_business
            .get(&business)
            .map(|takes| {
                takes
                    .range((
                        std::ops::Bound::Excluded(&(lower_bound, OperationId::from_raw(0))),
                        std::ops::Bound::Included(&(at, OperationId::from_raw(u32::MAX))),
                    ))
                    .filter(|(_, operation_id)| Some(*operation_id) != exclude)
                    .count() as u32
            })
            .unwrap_or(0)
    }

    pub(crate) fn set_property_disposition(
        &mut self,
        id: OperationId,
        disposition: OperationPropertyDispositionRecord,
    ) {
        let record = self
            .records
            .get_mut(&id)
            .expect("validated operation disappeared before property disposition commit");
        assert_eq!(
            record.status(),
            OperationStatus::Completed,
            "only completed operations may dispose acquired property"
        );
        assert!(
            record
                .resolution()
                .and_then(OperationResolutionRecord::property_proceeds)
                .is_some(),
            "property disposition requires persisted property proceeds"
        );
        assert!(
            record.runtime.property_disposition.is_none(),
            "operation property may only be disposed once"
        );
        record.runtime.property_disposition = Some(disposition);
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("operation version counter exhausted");
    }

    pub(crate) fn set_cash_disposition(
        &mut self,
        id: OperationId,
        disposition: OperationCashDispositionRecord,
    ) {
        let record = self
            .records
            .get_mut(&id)
            .expect("validated operation disappeared before cash disposition commit");
        assert_eq!(
            record.status(),
            OperationStatus::Completed,
            "only completed operations may deposit taken cash"
        );
        assert!(
            record
                .resolution()
                .and_then(OperationResolutionRecord::cash_proceeds)
                .is_some(),
            "cash disposition requires persisted cash proceeds"
        );
        assert!(
            record.runtime.cash_disposition.is_none(),
            "operation cash may only be deposited once"
        );
        record.runtime.cash_disposition = Some(disposition);
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("operation version counter exhausted");
    }

    fn change_status(&mut self, id: OperationId, next: OperationStatus) {
        let (previous, organization) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before status commit");
            (record.status(), record.responsible_organization())
        };
        if let Some(ids) = self.by_status.get_mut(&previous) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_status.remove(&previous);
            }
        }
        let active_entry = self.active_by_organization.entry(organization).or_default();
        if matches!(next, OperationStatus::Completed | OperationStatus::Aborted) {
            active_entry.remove(&id);
        } else {
            // Active-to-active transitions keep their membership; insertion is idempotent.
            active_entry.insert(id);
        }
        if active_entry.is_empty() {
            self.active_by_organization.remove(&organization);
        }
        let record = self
            .records
            .get_mut(&id)
            .expect("validated operation disappeared before status commit");
        record.runtime.status = next;
        record.runtime.version = record
            .runtime
            .version
            .checked_add(1)
            .expect("operation version counter exhausted");
        self.by_status.entry(next).or_default().insert(id);
    }

    fn remove_schedule_index(
        index: &mut BTreeMap<SimTime, BTreeSet<OperationId>>,
        time: SimTime,
        id: OperationId,
    ) {
        if let Some(ids) = index.get_mut(&time) {
            ids.remove(&id);
            if ids.is_empty() {
                index.remove(&time);
            }
        }
    }

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        for record in self.records.values() {
            if !self
                .by_organization
                .get(&record.responsible_organization())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            if !self
                .by_status
                .get(&record.status())
                .is_some_and(|ids| ids.contains(&record.id()))
            {
                return false;
            }
            let active_indexed = self
                .active_by_organization
                .get(&record.responsible_organization())
                .is_some_and(|ids| ids.contains(&record.id()));
            if active_indexed
                != !matches!(
                    record.status(),
                    OperationStatus::Completed | OperationStatus::Aborted
                )
            {
                return false;
            }
            let authorized_indexed = self
                .authorized_by_start
                .get(&record.scheduled_for())
                .is_some_and(|ids| ids.contains(&record.id()));
            if authorized_indexed != (record.status() == OperationStatus::Authorized) {
                return false;
            }
            let resolution_indexed = record.resolution_due_at().is_some_and(|due_at| {
                self.in_progress_by_resolution_due
                    .get(&due_at)
                    .is_some_and(|ids| ids.contains(&record.id()))
            });
            if resolution_indexed != (record.status() == OperationStatus::InProgress) {
                return false;
            }
            if let Some(resolution) = record.resolution() {
                for information in resolution.discovered_information() {
                    if self.by_discovered_information.get(information) != Some(&record.id()) {
                        return false;
                    }
                }
                // Recency-depletion index membership must match exactly: a completed
                // successful business take is indexed; everything else is not.
                let taken_business = record.objective().taken_business();
                let should_index = taken_business.is_some()
                    && record.status() == OperationStatus::Completed
                    && matches!(
                        resolution.objective_outcome(),
                        OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    );
                let indexed = taken_business
                    .and_then(|business| self.successful_takes_by_business.get(&business))
                    .is_some_and(|takes| takes.contains(&(resolution.resolved_at(), record.id())));
                if indexed != should_index {
                    return false;
                }
            }
        }
        for (organization, ids) in &self.by_organization {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.responsible_organization() == *organization)
                {
                    return false;
                }
            }
        }
        for (information, operation) in &self.by_discovered_information {
            if !self.records.get(operation).is_some_and(|record| {
                record.resolution().is_some_and(|resolution| {
                    resolution.discovered_information().contains(information)
                })
            }) {
                return false;
            }
        }
        for (status, ids) in &self.by_status {
            for id in ids {
                if !self
                    .records
                    .get(id)
                    .is_some_and(|record| record.status() == *status)
                {
                    return false;
                }
            }
        }
        for (organization, ids) in &self.active_by_organization {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.responsible_organization() == *organization
                        && !matches!(
                            record.status(),
                            OperationStatus::Completed | OperationStatus::Aborted
                        )
                }) {
                    return false;
                }
            }
        }
        for (time, ids) in &self.authorized_by_start {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == OperationStatus::Authorized
                        && record.scheduled_for() == *time
                }) {
                    return false;
                }
            }
        }
        for (time, ids) in &self.in_progress_by_resolution_due {
            for id in ids {
                if !self.records.get(id).is_some_and(|record| {
                    record.status() == OperationStatus::InProgress
                        && record.resolution_due_at() == Some(*time)
                }) {
                    return false;
                }
            }
        }
        true
    }

    /// Debug builds re-derive the full index consistency check on every mutation boundary;
    /// `has_consistent_indexes` is the single authority so the two can never drift apart.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: operation indexes disagree with source records"
        );
    }
}
