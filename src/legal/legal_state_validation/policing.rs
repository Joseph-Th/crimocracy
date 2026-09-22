//! Index-consistency checks for jurisdiction, patrol, and police-response state.

use crate::legal::legal_state::LegalState;
use crate::legal::records::{PatrolDeploymentStatus, PoliceResponseStatus};

impl LegalState {
    pub(super) fn has_consistent_police_response_indexes(&self) -> bool {
        for response in self.police_responses.values() {
            let id = response.id();
            if self
                .indexes
                .police_responses
                .by_source_operation
                .get(&response.source_operation())
                != Some(&id)
            {
                return false;
            }
            let due_indexed = self
                .indexes
                .police_responses
                .dispatched_by_arrival_due
                .get(&response.arrival_due_at())
                .is_some_and(|ids| ids.contains(&id));
            if due_indexed != (response.status() == PoliceResponseStatus::Dispatched) {
                return false;
            }
        }
        for (operation, id) in &self.indexes.police_responses.by_source_operation {
            if !self
                .police_responses
                .get(id)
                .is_some_and(|record| record.source_operation() == *operation)
            {
                return false;
            }
        }
        for (due_at, ids) in &self.indexes.police_responses.dispatched_by_arrival_due {
            if ids.iter().any(|id| {
                !self.police_responses.get(id).is_some_and(|record| {
                    record.status() == PoliceResponseStatus::Dispatched
                        && record.arrival_due_at() == *due_at
                })
            }) {
                return false;
            }
        }
        true
    }

    pub(super) fn has_consistent_jurisdiction_indexes(&self) -> bool {
        for jurisdiction in self.jurisdictions.values() {
            let Some(current) = jurisdiction.revisions().last() else {
                return false;
            };
            for neighborhood in current.neighborhoods() {
                if !self
                    .indexes
                    .jurisdictions
                    .jurisdictions_by_neighborhood
                    .get(neighborhood)
                    .is_some_and(|organizations| {
                        organizations.contains(&jurisdiction.organization())
                    })
                {
                    return false;
                }
            }
        }
        for (neighborhood, organizations) in
            &self.indexes.jurisdictions.jurisdictions_by_neighborhood
        {
            for organization in organizations {
                if !self
                    .jurisdictions
                    .get(organization)
                    .and_then(|record| record.revisions().last())
                    .is_some_and(|revision| revision.neighborhoods().contains(neighborhood))
                {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn has_consistent_patrol_indexes(&self) -> bool {
        for deployment in self.patrol_deployments.values() {
            let Some(current) = deployment.revisions().last() else {
                return false;
            };
            let id = deployment.id();
            if !self
                .indexes
                .patrols
                .by_neighborhood
                .get(&deployment.neighborhood())
                .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            let active_pair = self
                .indexes
                .patrols
                .active_by_organization_neighborhood
                .get(&(deployment.organization(), deployment.neighborhood()));
            let active_neighborhood = self
                .indexes
                .patrols
                .active_by_neighborhood
                .get(&deployment.neighborhood())
                .is_some_and(|ids| ids.contains(&id));
            match current.status() {
                PatrolDeploymentStatus::Active
                    if active_pair != Some(&id) || !active_neighborhood =>
                {
                    return false;
                }
                PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired
                    if active_pair == Some(&id) || active_neighborhood =>
                {
                    return false;
                }
                PatrolDeploymentStatus::Active
                | PatrolDeploymentStatus::Suspended
                | PatrolDeploymentStatus::Retired => {}
            }
        }
        for (neighborhood, ids) in &self.indexes.patrols.by_neighborhood {
            for id in ids {
                if !self
                    .patrol_deployments
                    .get(id)
                    .is_some_and(|record| record.neighborhood() == *neighborhood)
                {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.patrols.active_by_organization_neighborhood {
            if !self.patrol_deployments.get(id).is_some_and(|record| {
                record
                    .revisions()
                    .last()
                    .is_some_and(|revision| revision.status() == PatrolDeploymentStatus::Active)
                    && (record.organization(), record.neighborhood()) == *key
            }) {
                return false;
            }
        }
        for (neighborhood, ids) in &self.indexes.patrols.active_by_neighborhood {
            for id in ids {
                if !self.patrol_deployments.get(id).is_some_and(|record| {
                    record
                        .revisions()
                        .last()
                        .is_some_and(|revision| revision.status() == PatrolDeploymentStatus::Active)
                        && record.neighborhood() == *neighborhood
                }) {
                    return false;
                }
            }
        }
        true
    }
}
