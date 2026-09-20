//! Jurisdiction, patrol, and police-response mutation/index maintenance.

use super::super::*;

impl LegalState {
    pub(in crate::legal) fn set_jurisdiction(
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
    pub(in crate::legal) fn insert_patrol_deployment(&mut self, record: PatrolDeploymentRecord) {
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
    pub(in crate::legal) fn revise_patrol_deployment(
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
    pub(in crate::legal) fn set_patrol_deployment_status(
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
    pub(in crate::legal) fn insert_police_response(&mut self, record: PoliceResponseRecord) {
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
    pub(in crate::legal) fn set_police_response_arrived(
        &mut self,
        id: PoliceResponseId,
        at: SimTime,
    ) {
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
}
