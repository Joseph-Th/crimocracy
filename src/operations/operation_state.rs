//! Operation state storage and index maintenance; sibling `operations` types define records.

use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    BusinessId, CharacterId, InformationId, OperationId, OrganizationId, PoliceResponseId,
};
use crate::core::time::{SimDuration, SimTime};
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
    #[serde(skip)]
    by_organization: BTreeMap<OrganizationId, BTreeSet<OperationId>>,
    /// Non-terminal operation bookings keyed by participant. This derived availability index
    /// serves double-booking, custody preemption, and reassignment checks.
    #[serde(skip)]
    active_by_participant: BTreeMap<CharacterId, BTreeSet<OperationId>>,
    #[serde(skip)]
    by_status: BTreeMap<OperationStatus, BTreeSet<OperationId>>,
    #[serde(skip)]
    by_discovered_information: BTreeMap<InformationId, OperationId>,
    #[serde(skip)]
    authorized_by_start: BTreeMap<SimTime, BTreeSet<OperationId>>,
    #[serde(skip)]
    in_progress_by_resolution_due: BTreeMap<SimTime, BTreeSet<OperationId>>,
    /// Successful property/cash takes per target business as (resolved_at, operation_id).
    /// Feeds recency-depletion economics without scanning the full completed bucket, which
    /// grows for the life of the campaign.
    #[serde(skip)]
    successful_takes_by_business: BTreeMap<BusinessId, BTreeSet<(SimTime, OperationId)>>,
}

impl OperationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.by_organization.clear();
        self.active_by_participant.clear();
        self.by_status.clear();
        self.by_discovered_information.clear();
        self.authorized_by_start.clear();
        self.in_progress_by_resolution_due.clear();
        self.successful_takes_by_business.clear();
        for record in self.records.values() {
            let id = record.id();
            self.by_organization
                .entry(record.responsible_organization())
                .or_default()
                .insert(id);
            self.by_status
                .entry(record.status())
                .or_default()
                .insert(id);
            if !matches!(
                record.status(),
                OperationStatus::Completed | OperationStatus::Aborted
            ) {
                for participant in record.participants() {
                    self.active_by_participant
                        .entry(participant)
                        .or_default()
                        .insert(id);
                }
            }
            if record.status() == OperationStatus::Authorized {
                self.authorized_by_start
                    .entry(record.scheduled_for())
                    .or_default()
                    .insert(id);
            }
            if record.status() == OperationStatus::InProgress
                && let Some(due_at) = record.resolution_due_at()
            {
                self.in_progress_by_resolution_due
                    .entry(due_at)
                    .or_default()
                    .insert(id);
            }
            if let Some(resolution) = record.resolution() {
                for information in resolution.discovered_information() {
                    self.by_discovered_information.insert(*information, id);
                }
                if record.status() == OperationStatus::Completed
                    && matches!(
                        resolution.objective_outcome(),
                        OperationObjectiveOutcome::Achieved | OperationObjectiveOutcome::Partial
                    )
                    && let Some(business) = record.objective().taken_business()
                {
                    self.successful_takes_by_business
                        .entry(business)
                        .or_default()
                        .insert((resolution.resolved_at(), id));
                }
            }
        }
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
            .map(|operation_id| {
                self.records
                    .get(operation_id)
                    .expect("operation organization index must reference an operation")
            })
    }

    /// Non-terminal operations holding a participant, in operation-id order.
    pub(crate) fn active_operations_for_participant(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.active_by_participant
            .get(&character)
            .into_iter()
            .flatten()
            .map(|operation_id| {
                self.records
                    .get(operation_id)
                    .expect("active participant index must reference an operation")
            })
    }

    pub(crate) fn active_operation_bookings(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = OperationId> + '_ {
        self.active_by_participant
            .get(&character)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Finds the smallest non-terminal operation holding the character as leader or role
    /// participant in O(log participants) lookup time.
    pub(crate) fn find_active_operation_booking(
        &self,
        character: CharacterId,
    ) -> Option<OperationId> {
        self.active_by_participant
            .get(&character)
            .and_then(|ids| ids.first().copied())
    }

    pub fn operation_for_discovered_information(
        &self,
        information: InformationId,
    ) -> Option<&OperationRecord> {
        self.by_discovered_information
            .get(&information)
            .map(|operation| {
                self.records
                    .get(operation)
                    .expect("discovered-information index must reference an operation")
            })
    }

    pub fn operations_with_status(
        &self,
        status: OperationStatus,
    ) -> impl Iterator<Item = &OperationRecord> {
        self.by_status
            .get(&status)
            .into_iter()
            .flatten()
            .map(|operation_id| {
                self.records
                    .get(operation_id)
                    .expect("operation status index must reference an operation")
            })
    }

    pub(crate) fn operations(&self) -> impl Iterator<Item = &OperationRecord> {
        self.records.values()
    }
    pub(crate) fn operation_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }

    pub(crate) fn find_due_authorized(&self, now: SimTime) -> Vec<OperationId> {
        // BTreeMap + BTreeSet iteration is already sorted by (time, id) but make the
        // contract explicit: due operations start in stable order regardless of backing
        // collection choice. Matches the sorted scan for missed deadlines.
        let mut due: Vec<OperationId> = self
            .authorized_by_start
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        due.sort_unstable();
        due
    }

    pub(crate) fn find_due_in_progress(&self, now: SimTime) -> Vec<OperationId> {
        let mut due: Vec<OperationId> = self
            .in_progress_by_resolution_due
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        due.sort_unstable();
        due
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
        for participant in record.participants() {
            self.active_by_participant
                .entry(participant)
                .or_default()
                .insert(id);
        }
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
        self.set_status(id, OperationStatus::InProgress);
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
        self.set_status(id, OperationStatus::AwaitingDecision);
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
        self.set_status(id, OperationStatus::InProgress);
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
        self.set_status(id, OperationStatus::Aborted);
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
        ) && let Some(business) = record.objective().taken_business()
        {
            // NOTE: deliberately unpruned. Load-time validation re-derives every
            // historical settlement's take economics against that settlement's own
            // recency window, so even entries older than the live window remain
            // load-bearing - same trade as the append-only ledger itself.
            self.successful_takes_by_business
                .entry(business)
                .or_default()
                .insert((resolution.resolved_at(), id));
        }
        self.set_status(id, OperationStatus::Completed);
    }

    /// Successful takes against `business` resolved inside the recency window before the
    /// current operation's `(resolved_at, id)` ordering position. The lower time boundary is
    /// open: a target is fully restocked exactly when the authored window elapses. Ordering by
    /// operation ID within one simulation minute mirrors the canonical due-operation pass and
    /// keeps historical re-derivation stable when several takes resolve at the same `SimTime`.
    pub(crate) fn recent_successful_takes(
        &self,
        business: BusinessId,
        at: SimTime,
        window: SimDuration,
        current_operation: OperationId,
    ) -> u32 {
        let at_minutes = at.as_minutes();
        let window_minutes = u64::from(window.as_minutes());
        let lower_bound = SimTime::from_minutes(at_minutes.saturating_sub(window_minutes));
        // Once a complete window exists, excluding `(lower_bound, MAX)` excludes every take at
        // the exact lower timestamp, not merely operation ID zero (real IDs start at one). Before
        // then the conceptual lower bound is before campaign time zero, so the range must be
        // unbounded below or a legitimate minute-zero take would disappear early.
        let lower_key = (lower_bound, OperationId::from_raw(u32::MAX));
        let lower_range_bound = if at_minutes >= window_minutes {
            std::ops::Bound::Excluded(&lower_key)
        } else {
            std::ops::Bound::Unbounded
        };
        // When re-deriving one operation, excluding its own `(at, id)` key also excludes later
        // same-minute IDs that had not yet committed when this operation was resolved by the
        // canonical ascending-ID pass.
        let upper_key = (at, current_operation);
        self.successful_takes_by_business
            .get(&business)
            .map(|takes| {
                takes
                    .range((lower_range_bound, std::ops::Bound::Excluded(&upper_key)))
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

    fn set_status(&mut self, id: OperationId, next: OperationStatus) {
        let (previous, participants) = {
            let record = self
                .records
                .get(&id)
                .expect("validated operation disappeared before status commit");
            (record.status(), record.participants())
        };
        if let Some(ids) = self.by_status.get_mut(&previous) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_status.remove(&previous);
            }
        }
        for participant in participants {
            if matches!(next, OperationStatus::Completed | OperationStatus::Aborted) {
                if let Some(ids) = self.active_by_participant.get_mut(&participant) {
                    ids.remove(&id);
                    if ids.is_empty() {
                        self.active_by_participant.remove(&participant);
                    }
                }
            } else {
                // Active-to-active transitions keep their membership; insertion is idempotent.
                self.active_by_participant
                    .entry(participant)
                    .or_default()
                    .insert(id);
            }
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
        // Forward direction plus exact-count agreement replaces per-entry reverse walks:
        // ids are unique and every index is a function of its record, so matching entry
        // totals prove no stale, duplicate, or foreign index membership survives.
        let mut expected_by_organization = 0_usize;
        let mut expected_by_status = 0_usize;
        let mut expected_active_participant_links = 0_usize;
        let mut expected_authorized = 0_usize;
        let mut expected_in_progress = 0_usize;
        let mut expected_takes = 0_usize;
        let mut expected_discovered_links = 0_usize;
        for (stored_id, record) in &self.records {
            if *stored_id != record.id() {
                return false;
            }
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
            let active = !matches!(
                record.status(),
                OperationStatus::Completed | OperationStatus::Aborted
            );
            let participants = record.participants();
            for participant in &participants {
                let active_indexed = self
                    .active_by_participant
                    .get(participant)
                    .is_some_and(|ids| ids.contains(&record.id()));
                if active_indexed != active {
                    return false;
                }
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
            expected_by_organization += 1;
            expected_by_status += 1;
            if active {
                expected_active_participant_links += participants.len();
            }
            if record.status() == OperationStatus::Authorized {
                expected_authorized += 1;
            }
            if record.status() == OperationStatus::InProgress {
                expected_in_progress += 1;
            }
            if let Some(resolution) = record.resolution() {
                for information in resolution.discovered_information() {
                    if self.by_discovered_information.get(information) != Some(&record.id()) {
                        return false;
                    }
                }
                expected_discovered_links += resolution.discovered_information().len();
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
                if should_index {
                    expected_takes += 1;
                }
            }
        }
        let indexed_by_organization: usize = self.by_organization.values().map(BTreeSet::len).sum();
        if indexed_by_organization != expected_by_organization {
            return false;
        }
        let indexed_by_status: usize = self.by_status.values().map(BTreeSet::len).sum();
        if indexed_by_status != expected_by_status {
            return false;
        }
        let indexed_active_participant_links: usize =
            self.active_by_participant.values().map(BTreeSet::len).sum();
        if indexed_active_participant_links != expected_active_participant_links {
            return false;
        }
        if self.by_discovered_information.len() != expected_discovered_links {
            return false;
        }
        let indexed_takes: usize = self
            .successful_takes_by_business
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_takes != expected_takes {
            return false;
        }
        let indexed_authorized: usize = self.authorized_by_start.values().map(BTreeSet::len).sum();
        if indexed_authorized != expected_authorized {
            return false;
        }
        let indexed_in_progress: usize = self
            .in_progress_by_resolution_due
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_in_progress != expected_in_progress {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_take_window_excludes_exact_lower_boundary_and_later_same_minute_ids() {
        let business = BusinessId::from_raw(7);
        let at = SimTime::from_minutes(5_000);
        let window = SimDuration::from_minutes(4_320);
        let lower = SimTime::from_minutes(680);
        let current = OperationId::from_raw(20);
        let mut state = OperationState::new();
        state.successful_takes_by_business.insert(
            business,
            BTreeSet::from([
                (lower, OperationId::from_raw(1)),
                (SimTime::from_minutes(681), OperationId::from_raw(2)),
                (at, OperationId::from_raw(10)),
                (at, current),
                (at, OperationId::from_raw(30)),
            ]),
        );

        assert_eq!(
            state.recent_successful_takes(business, at, window, current),
            2,
            "only interior-window takes that precede the current operation may deplete it"
        );

        let early = SimTime::from_minutes(100);
        let mut early_state = OperationState::new();
        early_state.successful_takes_by_business.insert(
            business,
            BTreeSet::from([
                (SimTime::from_minutes(0), OperationId::from_raw(1)),
                (early, OperationId::from_raw(2)),
            ]),
        );
        assert_eq!(
            early_state.recent_successful_takes(business, early, window, OperationId::from_raw(2),),
            1,
            "before a full window has elapsed, campaign-start takes are still recent"
        );
    }
}
