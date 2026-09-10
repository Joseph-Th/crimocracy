//! `LegalState` durable record ownership and deterministic derived-index reconstruction.
//!
//! `LegalState` is the single owner of all legal records and their derived indexes
//! (see `records.rs` for the record definitions). Child `mutations` owns authoritative
//! record writes plus synchronized index maintenance, child `queries` owns read-only
//! observation, and sibling `legal_state_validation.rs` owns the `has_consistent_*`
//! projection checks over the same private fields.

use crate::core::entity::EntityRef;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, ContactId, EvidenceId, InformantDisclosureId,
    InformantId, InformationId, InvestigationId, InvestigationWorkId, LegalRepresentationId,
    NeighborhoodId, OperationId, OrganizationId, PatrolDeploymentId, PoliceResponseId,
    ProsecutionCaseId, ProsecutionReferralId, ReportId, WitnessStatementId,
};
use crate::core::time::SimTime;
use crate::core::version::advance_version_preflighted;
use crate::legal::investigation_system::evidence_is_actionable_case_lead;
use crate::legal::records::{
    ArrestRecord, ArrestStatus, CaseWitnessRecord, EvidenceRecord, InformantDisclosureRecord,
    InformantRecord, InvestigationRecord, InvestigationStatus, InvestigationWorkCancellation,
    InvestigationWorkCancellationReason, InvestigationWorkFocus, InvestigationWorkKind,
    InvestigationWorkRecord, InvestigationWorkResolution, InvestigationWorkStatus,
    JurisdictionRecord, LegalIndexes, LegalRepresentationEndReason, LegalRepresentationOrigin,
    LegalRepresentationRecord, LegalRepresentationStatus, PatrolDeploymentRecord,
    PatrolDeploymentStatus, PatrolWindow, PoliceResponseRecord, PoliceResponseStatus,
    ProsecutionCaseRecord, ProsecutionCaseResolution, ProsecutionCaseStatus,
    ProsecutionReferralRecord, WitnessCooperation, WitnessStatementRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LegalState {
    pub(super) investigations: BTreeMap<InvestigationId, InvestigationRecord>,
    pub(super) investigation_work: BTreeMap<InvestigationWorkId, InvestigationWorkRecord>,
    pub(super) case_witnesses: BTreeMap<CaseWitnessId, CaseWitnessRecord>,
    pub(super) witness_statements: BTreeMap<WitnessStatementId, WitnessStatementRecord>,
    pub(super) informants: BTreeMap<InformantId, InformantRecord>,
    pub(super) informant_disclosures: BTreeMap<InformantDisclosureId, InformantDisclosureRecord>,
    pub(super) evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    pub(super) jurisdictions: BTreeMap<OrganizationId, JurisdictionRecord>,
    pub(super) patrol_deployments: BTreeMap<PatrolDeploymentId, PatrolDeploymentRecord>,
    pub(super) police_responses: BTreeMap<PoliceResponseId, PoliceResponseRecord>,
    pub(super) arrests: BTreeMap<ArrestId, ArrestRecord>,
    pub(super) legal_representations: BTreeMap<LegalRepresentationId, LegalRepresentationRecord>,
    pub(super) prosecution_cases: BTreeMap<ProsecutionCaseId, ProsecutionCaseRecord>,
    pub(super) prosecution_referrals: BTreeMap<ProsecutionReferralId, ProsecutionReferralRecord>,
    #[serde(skip)]
    pub(super) indexes: LegalIndexes,
}

impl LegalState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.indexes = LegalIndexes::default();
        self.rebuild_investigation_indexes();
        self.rebuild_evidence_indexes();
        self.rebuild_witness_indexes();
        self.rebuild_informant_indexes();
        self.rebuild_investigation_work_indexes();
        self.rebuild_jurisdiction_indexes();
        self.rebuild_patrol_indexes();
        self.rebuild_police_response_indexes();
        self.rebuild_arrest_indexes();
        self.rebuild_representation_indexes();
        self.rebuild_prosecution_indexes();
    }

    fn rebuild_investigation_indexes(&mut self) {
        for investigation in self.investigations.values() {
            let id = investigation.id();
            self.indexes
                .investigations
                .by_owner
                .entry(investigation.owner())
                .or_default()
                .insert(id);
            for subject in investigation.subjects() {
                self.indexes
                    .investigations
                    .investigations_by_subject
                    .entry(*subject)
                    .or_default()
                    .insert(id);
            }
            if let Some(investigator) = investigation.lead_investigator() {
                self.indexes
                    .investigations
                    .investigations_by_investigator
                    .entry(investigator)
                    .or_default()
                    .insert(id);
            }
            if investigation.status() == InvestigationStatus::Active {
                self.indexes.investigations.active.insert(id);
                self.indexes
                    .investigations
                    .cases_by_last_activity
                    .entry(investigation.last_activity_at())
                    .or_default()
                    .insert(id);
                if investigation.lead_investigator().is_none() {
                    self.indexes.investigations.active_without_lead.insert(id);
                }
            }
        }
    }

    fn rebuild_evidence_indexes(&mut self) {
        for evidence in self.evidence.values() {
            for source in evidence.derived_from() {
                self.indexes
                    .evidence
                    .derived_evidence_by_source
                    .entry(*source)
                    .or_default()
                    .insert(evidence.id());
            }
        }
    }

    fn rebuild_witness_indexes(&mut self) {
        for witness in self.case_witnesses.values() {
            self.indexes
                .witnesses
                .case_witness_by_case_character
                .insert((witness.investigation(), witness.witness()), witness.id());
            self.indexes
                .witnesses
                .case_witnesses_by_investigation
                .entry(witness.investigation())
                .or_default()
                .insert(witness.id());
            self.indexes
                .witnesses
                .case_witnesses_by_character
                .entry(witness.witness())
                .or_default()
                .insert(witness.id());
        }
        for statement in self.witness_statements.values() {
            self.indexes
                .witnesses
                .witness_statement_by_evidence
                .insert(statement.evidence(), statement.id());
        }
    }

    fn rebuild_informant_indexes(&mut self) {
        for informant in self.informants.values() {
            self.indexes
                .informants
                .by_character_handler
                .insert((informant.character(), informant.handler()), informant.id());
        }
        for disclosure in self.informant_disclosures.values() {
            self.indexes
                .informants
                .disclosure_by_case_information
                .insert(
                    (disclosure.investigation(), disclosure.source_information()),
                    disclosure.id(),
                );
        }
    }

    fn rebuild_investigation_work_indexes(&mut self) {
        for work in self.investigation_work.values() {
            let id = work.id();
            self.indexes
                .work
                .work_by_investigation
                .entry(work.investigation())
                .or_default()
                .insert(id);
            self.indexes
                .work
                .work_by_investigator
                .entry(work.investigator())
                .or_default()
                .insert(id);
            if work.status() == InvestigationWorkStatus::Scheduled {
                self.indexes
                    .work
                    .scheduled_work_by_due_at
                    .entry(work.due_at())
                    .or_default()
                    .insert(id);
                self.indexes
                    .work
                    .scheduled_work_by_focus
                    .insert((work.investigation(), work.kind(), work.focus()), id);
            }
        }
    }

    fn rebuild_jurisdiction_indexes(&mut self) {
        for jurisdiction in self.jurisdictions.values() {
            let Some(current) = jurisdiction.revisions().last() else {
                continue;
            };
            for neighborhood in current.neighborhoods() {
                self.indexes
                    .jurisdictions
                    .jurisdictions_by_neighborhood
                    .entry(*neighborhood)
                    .or_default()
                    .insert(jurisdiction.organization());
            }
        }
    }

    fn rebuild_patrol_indexes(&mut self) {
        for patrol in self.patrol_deployments.values() {
            let Some(current) = patrol.revisions().last() else {
                continue;
            };
            self.indexes
                .patrols
                .by_neighborhood
                .entry(patrol.neighborhood())
                .or_default()
                .insert(patrol.id());
            if current.status() == PatrolDeploymentStatus::Active {
                self.indexes
                    .patrols
                    .active_by_organization_neighborhood
                    .insert((patrol.organization(), patrol.neighborhood()), patrol.id());
                self.indexes
                    .patrols
                    .active_by_neighborhood
                    .entry(patrol.neighborhood())
                    .or_default()
                    .insert(patrol.id());
            }
        }
    }

    fn rebuild_police_response_indexes(&mut self) {
        for response in self.police_responses.values() {
            self.indexes
                .police_responses
                .by_source_operation
                .insert(response.source_operation(), response.id());
            if response.status() == PoliceResponseStatus::Dispatched {
                self.indexes
                    .police_responses
                    .dispatched_by_arrival_due
                    .entry(response.arrival_due_at())
                    .or_default()
                    .insert(response.id());
            }
        }
    }

    fn rebuild_arrest_indexes(&mut self) {
        for arrest in self.arrests.values() {
            self.indexes
                .arrests
                .by_investigation
                .entry(arrest.investigation())
                .or_default()
                .insert(arrest.id());
            if arrest.status() == ArrestStatus::Detained {
                self.indexes
                    .arrests
                    .active_by_character
                    .insert(arrest.character(), arrest.id());
                self.indexes.arrests.detained.insert(arrest.id());
            }
        }
    }

    fn rebuild_representation_indexes(&mut self) {
        for representation in self.legal_representations.values() {
            if representation.status() == LegalRepresentationStatus::Active {
                self.indexes
                    .representations
                    .active_by_arrest
                    .insert(representation.arrest(), representation.id());
                self.indexes
                    .representations
                    .active_by_contact
                    .entry(representation.contact())
                    .or_default()
                    .insert(representation.id());
                if representation.origin() == LegalRepresentationOrigin::AutomaticPolicy {
                    self.indexes
                        .representations
                        .active_automatic_policy
                        .insert(representation.id());
                }
            }
        }
    }

    fn rebuild_prosecution_indexes(&mut self) {
        for case in self.prosecution_cases.values() {
            if case.status() == ProsecutionCaseStatus::Reviewing {
                self.indexes
                    .prosecutions
                    .open_by_arrest_office
                    .insert((case.arrest(), case.prosecutor_office()), case.id());
                if let Some(prosecutor) = case.assigned_prosecutor() {
                    self.indexes
                        .prosecutions
                        .reviewing_cases_by_prosecutor
                        .entry(prosecutor)
                        .or_default()
                        .insert(case.id());
                } else {
                    self.indexes
                        .prosecutions
                        .reviewing_without_prosecutor
                        .insert(case.id());
                }
            }
        }
        for referral in self.prosecution_referrals.values() {
            self.indexes
                .prosecutions
                .referrals_by_case
                .entry(referral.prosecution_case())
                .or_default()
                .insert(referral.id());
        }
    }
}

mod mutations;
mod queries;
