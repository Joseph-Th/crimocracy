//! Operation-state derived-index consistency auditing.
//!
//! The parent operation state owns storage and index mutation. This child owns the inverse audit
//! that proves every derived index is an exact projection of authoritative operation records.

use super::{OperationState, resolution_has_positive_take};
use crate::operations::{OperationRecord, OperationResolutionRecord, OperationStatus};
use std::collections::BTreeSet;

impl OperationState {
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        // Forward direction plus exact-count agreement replaces per-entry reverse walks:
        // ids are unique and every index is a function of its record, so matching entry
        // totals prove no stale, duplicate, or foreign index membership survives.
        let mut expected = OperationIndexExpectations::default();
        for (stored_id, record) in &self.records {
            if *stored_id != record.id()
                || !self.record_indexes_are_consistent(record, &mut expected)
            {
                return false;
            }
        }
        self.index_entry_counts_match(expected)
    }

    fn record_indexes_are_consistent(
        &self,
        record: &OperationRecord,
        expected: &mut OperationIndexExpectations,
    ) -> bool {
        if !self
            .by_organization
            .get(&record.responsible_organization())
            .is_some_and(|ids| ids.contains(&record.id()))
            || !self
                .by_status
                .get(&record.status())
                .is_some_and(|ids| ids.contains(&record.id()))
        {
            return false;
        }
        expected.by_organization += 1;
        expected.by_status += 1;

        let active = !matches!(
            record.status(),
            OperationStatus::Completed | OperationStatus::Aborted
        );
        let mut participant_count = 0_usize;
        for participant in record.participant_ids() {
            participant_count += 1;
            if self
                .active_by_participant
                .get(&participant)
                .is_some_and(|ids| ids.contains(&record.id()))
                != active
            {
                return false;
            }
        }
        if active {
            expected.active_participant_links += participant_count;
        }

        let deadline_indexed = record.completion_deadline().is_some_and(|deadline| {
            self.active_by_completion_deadline
                .get(&deadline)
                .is_some_and(|ids| ids.contains(&record.id()))
        });
        let should_index_deadline = active && record.completion_deadline().is_some();
        if deadline_indexed != should_index_deadline {
            return false;
        }
        expected.active_deadlines += usize::from(should_index_deadline);

        let authorized = record.status() == OperationStatus::Authorized;
        if self
            .authorized_by_start
            .get(&record.scheduled_for())
            .is_some_and(|ids| ids.contains(&record.id()))
            != authorized
        {
            return false;
        }
        expected.authorized += usize::from(authorized);

        let in_progress = record.status() == OperationStatus::InProgress;
        let resolution_indexed = record.resolution_due_at().is_some_and(|due_at| {
            self.in_progress_by_resolution_due
                .get(&due_at)
                .is_some_and(|ids| ids.contains(&record.id()))
        });
        if resolution_indexed != in_progress {
            return false;
        }
        expected.in_progress += usize::from(in_progress);

        record.resolution().is_none_or(|resolution| {
            self.resolution_indexes_are_consistent(record, resolution, expected)
        })
    }

    fn resolution_indexes_are_consistent(
        &self,
        record: &OperationRecord,
        resolution: &OperationResolutionRecord,
        expected: &mut OperationIndexExpectations,
    ) -> bool {
        for information in resolution.discovered_information() {
            if self.by_discovered_information.get(information) != Some(&record.id()) {
                return false;
            }
        }
        expected.discovered_links += resolution.discovered_information().len();

        // Recency-depletion index membership must match exactly: a completed successful
        // business take is indexed; everything else is not.
        let taken_business = record.objective().taken_business();
        let should_index = taken_business.is_some()
            && record.status() == OperationStatus::Completed
            && resolution_has_positive_take(resolution);
        let indexed = taken_business
            .and_then(|business| {
                self.successful_takes_by_business_kind
                    .get(&(business, record.kind()))
            })
            .is_some_and(|takes| takes.contains(&(resolution.resolved_at(), record.id())));
        if indexed != should_index {
            return false;
        }
        expected.takes += usize::from(should_index);

        let financial = record.status() == OperationStatus::Completed
            && resolution_has_positive_take(resolution);
        let organization = record.responsible_organization();
        let resolution_financial_indexed = self
            .financial_resolutions_by_organization
            .get(&organization)
            .is_some_and(|events| events.contains(&(resolution.resolved_at(), record.id())));
        if resolution_financial_indexed != financial {
            return false;
        }
        expected.financial_resolutions += usize::from(financial);

        let property_held = financial
            && resolution.property_proceeds().is_some()
            && record.property_disposition().is_none();
        let property_held_indexed = self
            .held_property_by_organization
            .get(&organization)
            .is_some_and(|ids| ids.contains(&record.id()));
        if property_held_indexed != property_held {
            return false;
        }
        expected.held_property += usize::from(property_held);
        let property_disposed = financial && record.property_disposition().is_some();
        let property_disposition_indexed =
            record.property_disposition().is_some_and(|disposition| {
                self.property_dispositions_by_organization
                    .get(&organization)
                    .is_some_and(|events| {
                        events.contains(&(disposition.disposed_at(), record.id()))
                    })
            });
        if property_disposition_indexed != property_disposed {
            return false;
        }
        expected.property_dispositions += usize::from(property_disposed);

        let cash_held = financial
            && resolution.cash_proceeds().is_some()
            && record.cash_disposition().is_none();
        let cash_held_indexed = self
            .held_cash_by_organization
            .get(&organization)
            .is_some_and(|ids| ids.contains(&record.id()));
        if cash_held_indexed != cash_held {
            return false;
        }
        expected.held_cash += usize::from(cash_held);
        let cash_disposed = financial && record.cash_disposition().is_some();
        let cash_disposition_indexed = record.cash_disposition().is_some_and(|disposition| {
            self.cash_dispositions_by_organization
                .get(&organization)
                .is_some_and(|events| events.contains(&(disposition.disposed_at(), record.id())))
        });
        if cash_disposition_indexed != cash_disposed {
            return false;
        }
        expected.cash_dispositions += usize::from(cash_disposed);
        true
    }

    fn index_entry_counts_match(&self, expected: OperationIndexExpectations) -> bool {
        let indexed_by_organization: usize = self.by_organization.values().map(BTreeSet::len).sum();
        if indexed_by_organization != expected.by_organization {
            return false;
        }
        let indexed_by_status: usize = self.by_status.values().map(BTreeSet::len).sum();
        if indexed_by_status != expected.by_status {
            return false;
        }
        let indexed_active_participant_links: usize =
            self.active_by_participant.values().map(BTreeSet::len).sum();
        if indexed_active_participant_links != expected.active_participant_links {
            return false;
        }
        if self.by_discovered_information.len() != expected.discovered_links {
            return false;
        }
        let indexed_takes: usize = self
            .successful_takes_by_business_kind
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_takes != expected.takes {
            return false;
        }
        let indexed_financial_resolutions: usize = self
            .financial_resolutions_by_organization
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_financial_resolutions != expected.financial_resolutions {
            return false;
        }
        let indexed_property_dispositions: usize = self
            .property_dispositions_by_organization
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_property_dispositions != expected.property_dispositions {
            return false;
        }
        let indexed_cash_dispositions: usize = self
            .cash_dispositions_by_organization
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_cash_dispositions != expected.cash_dispositions {
            return false;
        }
        let indexed_held_property: usize = self
            .held_property_by_organization
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_held_property != expected.held_property {
            return false;
        }
        let indexed_held_cash: usize = self
            .held_cash_by_organization
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_held_cash != expected.held_cash {
            return false;
        }
        let indexed_authorized: usize = self.authorized_by_start.values().map(BTreeSet::len).sum();
        if indexed_authorized != expected.authorized {
            return false;
        }
        let indexed_in_progress: usize = self
            .in_progress_by_resolution_due
            .values()
            .map(BTreeSet::len)
            .sum();
        if indexed_in_progress != expected.in_progress {
            return false;
        }
        let indexed_active_deadlines: usize = self
            .active_by_completion_deadline
            .values()
            .map(BTreeSet::len)
            .sum();
        indexed_active_deadlines == expected.active_deadlines
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct OperationIndexExpectations {
    by_organization: usize,
    by_status: usize,
    active_participant_links: usize,
    authorized: usize,
    in_progress: usize,
    active_deadlines: usize,
    takes: usize,
    discovered_links: usize,
    financial_resolutions: usize,
    property_dispositions: usize,
    cash_dispositions: usize,
    held_property: usize,
    held_cash: usize,
}
