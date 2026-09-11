//! `LegalState` index-consistency validation; sibling `legal_state` owns records and mutators.
//!
//! These projection checks re-derive every legal index from the authoritative records and
//! must agree with them after any mutation, save restoration, or invariant audit.

use crate::legal::legal_state::LegalState;
use crate::legal::records::{
    ArrestStatus, InvestigationStatus, InvestigationWorkStatus, LegalRepresentationOrigin,
    LegalRepresentationStatus, PatrolDeploymentStatus, PoliceResponseStatus, ProsecutionCaseStatus,
};

impl LegalState {
    fn has_consistent_primary_keys(&self) -> bool {
        self.investigations
            .iter()
            .all(|(id, record)| *id == record.id())
            && self
                .investigation_work
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .case_witnesses
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .witness_statements
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .informants
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .informant_disclosures
                .iter()
                .all(|(id, record)| *id == record.id())
            && self.evidence.iter().all(|(id, record)| *id == record.id())
            && self
                .jurisdictions
                .iter()
                .all(|(organization, record)| *organization == record.organization())
            && self
                .patrol_deployments
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .police_responses
                .iter()
                .all(|(id, record)| *id == record.id())
            && self.arrests.iter().all(|(id, record)| *id == record.id())
            && self
                .legal_representations
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .prosecution_cases
                .iter()
                .all(|(id, record)| *id == record.id())
            && self
                .prosecution_referrals
                .iter()
                .all(|(id, record)| *id == record.id())
    }

    fn has_consistent_prosecution_indexes(&self) -> bool {
        for case in self.prosecution_cases.values() {
            let id = case.id();
            let assignment_indexed = match (case.status(), case.assigned_prosecutor()) {
                (ProsecutionCaseStatus::Reviewing, Some(prosecutor)) => self
                    .indexes
                    .prosecutions
                    .reviewing_cases_by_prosecutor
                    .get(&prosecutor)
                    .is_some_and(|ids| ids.contains(&id)),
                (ProsecutionCaseStatus::Reviewing, None) => self
                    .indexes
                    .prosecutions
                    .reviewing_without_prosecutor
                    .contains(&id),
                (ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed, None) => true,
                (ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed, Some(_)) => false,
            };
            if !assignment_indexed
                || case.referrals().iter().any(|referral| {
                    !self
                        .indexes
                        .prosecutions
                        .referrals_by_case
                        .get(&id)
                        .is_some_and(|ids| ids.contains(referral))
                })
            {
                return false;
            }
            let open = self
                .indexes
                .prosecutions
                .open_by_arrest_office
                .get(&(case.arrest(), case.prosecutor_office()));
            match case.status() {
                ProsecutionCaseStatus::Reviewing if open != Some(&id) => return false,
                ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed
                    if open == Some(&id) =>
                {
                    return false;
                }
                ProsecutionCaseStatus::Reviewing
                | ProsecutionCaseStatus::Declined
                | ProsecutionCaseStatus::Closed => {}
            }
        }
        for referral in self.prosecution_referrals.values() {
            let case = match self.prosecution_cases.get(&referral.prosecution_case()) {
                Some(case) => case,
                None => return false,
            };
            if !case.referrals().contains(&referral.id())
                || !referral.evidence().is_subset(case.evidence())
                || !self
                    .indexes
                    .prosecutions
                    .referrals_by_case
                    .get(&case.id())
                    .is_some_and(|ids| ids.contains(&referral.id()))
            {
                return false;
            }
        }
        for (key, id) in &self.indexes.prosecutions.open_by_arrest_office {
            if !self.prosecution_cases.get(id).is_some_and(|case| {
                (case.arrest(), case.prosecutor_office()) == *key
                    && case.status() == ProsecutionCaseStatus::Reviewing
            }) {
                return false;
            }
        }
        for (prosecutor, ids) in &self.indexes.prosecutions.reviewing_cases_by_prosecutor {
            if ids.iter().any(|id| {
                !self.prosecution_cases.get(id).is_some_and(|case| {
                    case.status() == ProsecutionCaseStatus::Reviewing
                        && case.assigned_prosecutor() == Some(*prosecutor)
                })
            }) {
                return false;
            }
        }
        for id in &self.indexes.prosecutions.reviewing_without_prosecutor {
            if !self.prosecution_cases.get(id).is_some_and(|case| {
                case.status() == ProsecutionCaseStatus::Reviewing
                    && case.assigned_prosecutor().is_none()
            }) {
                return false;
            }
        }
        true
    }
    fn has_consistent_legal_representation_indexes(&self) -> bool {
        for record in self.legal_representations.values() {
            let id = record.id();
            let arrest_active = self
                .indexes
                .representations
                .active_by_arrest
                .get(&record.arrest());
            let contact_active = self
                .indexes
                .representations
                .active_by_contact
                .get(&record.contact())
                .is_some_and(|ids| ids.contains(&id));
            match record.status() {
                LegalRepresentationStatus::Active
                    if arrest_active != Some(&id) || !contact_active =>
                {
                    return false;
                }
                LegalRepresentationStatus::Ended
                    if arrest_active == Some(&id) || contact_active =>
                {
                    return false;
                }
                LegalRepresentationStatus::Active | LegalRepresentationStatus::Ended => {}
            }
            if self
                .indexes
                .representations
                .active_automatic_policy
                .contains(&id)
                != (record.status() == LegalRepresentationStatus::Active
                    && record.origin() == LegalRepresentationOrigin::AutomaticPolicy)
            {
                return false;
            }
        }
        for (arrest, id) in &self.indexes.representations.active_by_arrest {
            if !self.legal_representations.get(id).is_some_and(|record| {
                record.arrest() == *arrest && record.status() == LegalRepresentationStatus::Active
            }) {
                return false;
            }
        }
        for id in &self.indexes.representations.active_automatic_policy {
            if !self.legal_representations.get(id).is_some_and(|record| {
                record.status() == LegalRepresentationStatus::Active
                    && record.origin() == LegalRepresentationOrigin::AutomaticPolicy
            }) {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.representations.active_by_contact {
            if ids.iter().any(|id| {
                !self.legal_representations.get(id).is_some_and(|record| {
                    record.contact() == *contact
                        && record.status() == LegalRepresentationStatus::Active
                })
            }) {
                return false;
            }
        }
        true
    }
    fn has_consistent_arrest_indexes(&self) -> bool {
        for arrest in self.arrests.values() {
            let id = arrest.id();
            if !self
                .indexes
                .arrests
                .by_investigation
                .get(&arrest.investigation())
                .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            let active = self
                .indexes
                .arrests
                .active_by_character
                .get(&arrest.character());
            match arrest.status() {
                ArrestStatus::Detained if active != Some(&id) => return false,
                ArrestStatus::Released if active == Some(&id) => return false,
                ArrestStatus::Detained | ArrestStatus::Released => {}
            }
            if self.indexes.arrests.detained.contains(&id)
                != (arrest.status() == ArrestStatus::Detained)
            {
                return false;
            }
        }
        for (investigation, ids) in &self.indexes.arrests.by_investigation {
            if ids.iter().any(|id| {
                !self
                    .arrests
                    .get(id)
                    .is_some_and(|record| record.investigation() == *investigation)
            }) {
                return false;
            }
        }
        for (character, id) in &self.indexes.arrests.active_by_character {
            if !self.arrests.get(id).is_some_and(|record| {
                record.character() == *character && record.status() == ArrestStatus::Detained
            }) {
                return false;
            }
        }
        for id in &self.indexes.arrests.detained {
            if !self
                .arrests
                .get(id)
                .is_some_and(|record| record.status() == ArrestStatus::Detained)
            {
                return false;
            }
        }
        true
    }
    fn has_consistent_police_response_indexes(&self) -> bool {
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

    fn has_consistent_informant_indexes(&self) -> bool {
        for informant in self.informants.values() {
            let id = informant.id();
            let pair_index = self
                .indexes
                .informants
                .by_character_handler
                .get(&(informant.character(), informant.handler()));
            if pair_index != Some(&id)
                || !self
                    .indexes
                    .informants
                    .by_handler
                    .get(&informant.handler())
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
        }
        for (key, id) in &self.indexes.informants.by_character_handler {
            if !self
                .informants
                .get(id)
                .is_some_and(|record| (record.character(), record.handler()) == *key)
            {
                return false;
            }
        }
        for (handler, ids) in &self.indexes.informants.by_handler {
            for id in ids {
                if !self
                    .informants
                    .get(id)
                    .is_some_and(|record| record.handler() == *handler)
                {
                    return false;
                }
            }
        }
        for disclosure in self.informant_disclosures.values() {
            if !self.informants.contains_key(&disclosure.informant())
                || !self.evidence.contains_key(&disclosure.evidence())
                || self
                    .indexes
                    .informants
                    .disclosure_by_case_information
                    .get(&(disclosure.investigation(), disclosure.source_information()))
                    != Some(&disclosure.id())
            {
                return false;
            }
        }
        for (key, disclosure) in &self.indexes.informants.disclosure_by_case_information {
            if !self
                .informant_disclosures
                .get(disclosure)
                .is_some_and(|record| (record.investigation(), record.source_information()) == *key)
            {
                return false;
            }
        }
        true
    }

    fn has_consistent_witness_indexes(&self) -> bool {
        for witness in self.case_witnesses.values() {
            if self
                .indexes
                .witnesses
                .case_witness_by_case_character
                .get(&(witness.investigation(), witness.witness()))
                != Some(&witness.id())
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
                    .is_some_and(|ids| ids.contains(&witness.id()))
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id()))
            {
                return false;
            }
            for statement in witness.statements() {
                if !self
                    .witness_statements
                    .get(statement)
                    .is_some_and(|record| record.case_witness() == witness.id())
                {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.witnesses.case_witness_by_case_character {
            if !self
                .case_witnesses
                .get(id)
                .is_some_and(|record| (record.investigation(), record.witness()) == *key)
            {
                return false;
            }
        }
        for (investigation, ids) in &self.indexes.witnesses.case_witnesses_by_investigation {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        for (character, ids) in &self.indexes.witnesses.case_witnesses_by_character {
            for id in ids {
                if !self
                    .case_witnesses
                    .get(id)
                    .is_some_and(|record| record.witness() == *character)
                {
                    return false;
                }
            }
        }
        for statement in self.witness_statements.values() {
            if !self
                .case_witnesses
                .get(&statement.case_witness())
                .is_some_and(|witness| witness.statements().contains(&statement.id()))
                || self
                    .indexes
                    .witnesses
                    .witness_statement_by_evidence
                    .get(&statement.evidence())
                    != Some(&statement.id())
                || !self.evidence.contains_key(&statement.evidence())
            {
                return false;
            }
        }
        for (evidence, statement) in &self.indexes.witnesses.witness_statement_by_evidence {
            if !self
                .witness_statements
                .get(statement)
                .is_some_and(|record| record.evidence() == *evidence)
            {
                return false;
            }
        }
        true
    }

    fn has_consistent_investigation_indexes(&self) -> bool {
        self.investigation_forward_indexes_are_consistent()
            && self.active_investigation_indexes_are_consistent()
            && self.investigation_activity_index_is_consistent()
            && self.investigation_owner_index_is_consistent()
            && self.investigation_subject_index_is_consistent()
            && self.investigator_index_is_consistent()
    }

    /// Every investigation must appear in each projection implied by its authoritative record.
    fn investigation_forward_indexes_are_consistent(&self) -> bool {
        for investigation in self.investigations.values() {
            if !self
                .indexes
                .investigations
                .by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
            for subject in investigation.subjects() {
                if !self
                    .indexes
                    .investigations
                    .investigations_by_subject
                    .get(subject)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
                {
                    return false;
                }
            }
            for evidence_id in investigation.evidence() {
                if !self
                    .evidence
                    .get(evidence_id)
                    .is_some_and(|record| record.investigation() == investigation.id())
                {
                    return false;
                }
            }
            let should_need_lead = investigation.status() == InvestigationStatus::Active
                && investigation.lead_investigator().is_none();
            if self
                .indexes
                .investigations
                .active_without_lead
                .contains(&investigation.id())
                != should_need_lead
            {
                return false;
            }
            if self
                .indexes
                .investigations
                .active
                .contains(&investigation.id())
                != (investigation.status() == InvestigationStatus::Active)
            {
                return false;
            }
            if self
                .indexes
                .investigations
                .active_by_owner
                .get(&investigation.owner())
                .is_some_and(|ids| ids.contains(&investigation.id()))
                != (investigation.status() == InvestigationStatus::Active)
            {
                return false;
            }
            if let Some(investigator) = investigation.lead_investigator()
                && !self
                    .indexes
                    .investigations
                    .investigations_by_investigator
                    .get(&investigator)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
        }
        for (owner, investigations) in &self.indexes.investigations.active_by_owner {
            for investigation in investigations {
                if !self
                    .investigations
                    .get(investigation)
                    .is_some_and(|record| {
                        record.status() == InvestigationStatus::Active && record.owner() == *owner
                    })
                {
                    return false;
                }
            }
        }
        true
    }

    /// Active-case membership and the empty-lead subset must resolve back to matching records.
    fn active_investigation_indexes_are_consistent(&self) -> bool {
        for investigation in &self.indexes.investigations.active_without_lead {
            if !self
                .investigations
                .get(investigation)
                .is_some_and(|record| {
                    record.status() == InvestigationStatus::Active
                        && record.lead_investigator().is_none()
                })
            {
                return false;
            }
        }
        for investigation in &self.indexes.investigations.active {
            if !self
                .investigations
                .get(investigation)
                .is_some_and(|record| record.status() == InvestigationStatus::Active)
            {
                return false;
            }
        }
        true
    }

    /// The activity schedule is bidirectional and contains active investigations only.
    fn investigation_activity_index_is_consistent(&self) -> bool {
        for (at, ids) in &self.indexes.investigations.cases_by_last_activity {
            for id in ids {
                if !self.investigations.get(id).is_some_and(|record| {
                    record.status() == InvestigationStatus::Active
                        && record.last_activity_at() == *at
                }) {
                    return false;
                }
            }
        }
        for investigation in self.investigations.values() {
            if investigation.status() != InvestigationStatus::Active {
                continue;
            }
            if !self
                .indexes
                .investigations
                .cases_by_last_activity
                .get(&investigation.last_activity_at())
                .is_some_and(|ids| ids.contains(&investigation.id()))
            {
                return false;
            }
        }
        true
    }

    /// Reverse owner entries must resolve to investigations owned by that institution.
    fn investigation_owner_index_is_consistent(&self) -> bool {
        for (owner, ids) in &self.indexes.investigations.by_owner {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.owner() == *owner)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse subject entries must agree with each investigation's effective subject set.
    fn investigation_subject_index_is_consistent(&self) -> bool {
        for (subject, ids) in &self.indexes.investigations.investigations_by_subject {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.subjects().contains(subject))
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse investigator entries must agree with current lead assignments.
    fn investigator_index_is_consistent(&self) -> bool {
        for (investigator, ids) in &self.indexes.investigations.investigations_by_investigator {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.lead_investigator() == Some(*investigator))
                {
                    return false;
                }
            }
        }
        true
    }

    fn has_consistent_evidence_indexes(&self) -> bool {
        for evidence in self.evidence.values() {
            if !self
                .investigations
                .get(&evidence.investigation())
                .is_some_and(|investigation| investigation.evidence().contains(&evidence.id()))
            {
                return false;
            }
            for source in evidence.derived_from() {
                if !self
                    .indexes
                    .evidence
                    .derived_evidence_by_source
                    .get(source)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
        }
        for (source, ids) in &self.indexes.evidence.derived_evidence_by_source {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.derived_from().contains(source))
                {
                    return false;
                }
            }
        }
        true
    }

    fn has_consistent_investigation_work_indexes(&self) -> bool {
        self.investigation_work_forward_indexes_are_consistent()
            && self.work_by_investigation_index_is_consistent()
            && self.work_by_investigator_index_is_consistent()
            && self.scheduled_work_due_index_is_consistent()
            && self.scheduled_work_focus_index_is_consistent()
    }

    /// Every work record must agree with its owner/investigator projections and lifecycle-only
    /// schedule indexes.
    fn investigation_work_forward_indexes_are_consistent(&self) -> bool {
        for work in self.investigation_work.values() {
            if !self
                .indexes
                .work
                .work_by_investigation
                .get(&work.investigation())
                .is_some_and(|ids| ids.contains(&work.id()))
                || !self
                    .indexes
                    .work
                    .work_by_investigator
                    .get(&work.investigator())
                    .is_some_and(|ids| ids.contains(&work.id()))
            {
                return false;
            }
            let due_indexed = self
                .indexes
                .work
                .scheduled_work_by_due_at
                .get(&work.due_at())
                .is_some_and(|ids| ids.contains(&work.id()));
            let focus_indexed = self.indexes.work.scheduled_work_by_focus.get(&(
                work.investigation(),
                work.kind(),
                work.focus(),
            )) == Some(&work.id());
            match work.status() {
                InvestigationWorkStatus::Scheduled => {
                    if work.resolution().is_some()
                        || work.cancellation().is_some()
                        || !due_indexed
                        || !focus_indexed
                    {
                        return false;
                    }
                }
                InvestigationWorkStatus::Completed => {
                    if work.resolution().is_none()
                        || work.cancellation().is_some()
                        || due_indexed
                        || focus_indexed
                    {
                        return false;
                    }
                }
                InvestigationWorkStatus::Cancelled => {
                    if work.resolution().is_some()
                        || work.cancellation().is_none()
                        || due_indexed
                        || focus_indexed
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Reverse case-work entries must resolve to work owned by that investigation.
    fn work_by_investigation_index_is_consistent(&self) -> bool {
        for (investigation, ids) in &self.indexes.work.work_by_investigation {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigation() == *investigation)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Reverse investigator-work entries must resolve to work assigned to that investigator.
    fn work_by_investigator_index_is_consistent(&self) -> bool {
        for (investigator, ids) in &self.indexes.work.work_by_investigator {
            for id in ids {
                if !self
                    .investigation_work
                    .get(id)
                    .is_some_and(|work| work.investigator() == *investigator)
                {
                    return false;
                }
            }
        }
        true
    }

    /// Only scheduled work may occupy the due-time index, at its exact authored due time.
    fn scheduled_work_due_index_is_consistent(&self) -> bool {
        for (time, ids) in &self.indexes.work.scheduled_work_by_due_at {
            for id in ids {
                if !self.investigation_work.get(id).is_some_and(|work| {
                    work.status() == InvestigationWorkStatus::Scheduled && work.due_at() == *time
                }) {
                    return false;
                }
            }
        }
        true
    }

    /// The scheduled focus index is exclusive and must point to the exact active work tuple.
    fn scheduled_work_focus_index_is_consistent(&self) -> bool {
        for (key, id) in &self.indexes.work.scheduled_work_by_focus {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.status() == InvestigationWorkStatus::Scheduled
                    && (work.investigation(), work.kind(), work.focus()) == *key
            }) {
                return false;
            }
        }
        true
    }

    fn has_consistent_jurisdiction_indexes(&self) -> bool {
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

    fn has_consistent_patrol_indexes(&self) -> bool {
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

    pub(crate) fn has_consistent_indexes(&self) -> bool {
        self.has_consistent_primary_keys()
            && self.has_consistent_investigation_indexes()
            && self.has_consistent_investigation_work_indexes()
            && self.has_consistent_evidence_indexes()
            && self.has_consistent_witness_indexes()
            && self.has_consistent_informant_indexes()
            && self.has_consistent_arrest_indexes()
            && self.has_consistent_legal_representation_indexes()
            && self.has_consistent_prosecution_indexes()
            && self.has_consistent_jurisdiction_indexes()
            && self.has_consistent_patrol_indexes()
            && self.has_consistent_police_response_indexes()
    }
}
