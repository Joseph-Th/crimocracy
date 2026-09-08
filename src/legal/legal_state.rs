//! `LegalState` ownership, index-synchronizing mutators, and read-only observation.
//!
//! `LegalState` is the single owner of all legal records and their derived indexes
//! (see `records.rs` for the record definitions). Every mutator validates, resolves,
//! commits, and re-synchronizes the indexes in one atomic method; readers observe
//! through read-only getters, and sibling `legal_state_validation.rs` owns the
//! `has_consistent_*` projection checks over the same private fields.

use crate::core::entity::EntityRef;
use crate::core::id::IdKeyedBounds;
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, ContactId, EvidenceId, InformantDisclosureId,
    InformantId, InformationId, InvestigationId, InvestigationWorkId, LegalRepresentationId,
    NeighborhoodId, OperationId, OrganizationId, PatrolDeploymentId, PoliceResponseId,
    ProsecutionCaseId, ProsecutionReferralId, ReportId, WitnessStatementId,
};
use crate::core::time::SimTime;
use crate::legal::investigation_system::evidence_is_actionable_case_lead;
use crate::legal::records::{
    ArrestRecord, ArrestStatus, CaseWitnessRecord, EvidenceRecord, InformantDisclosureRecord,
    InformantRecord, InformantStatus, InvestigationRecord, InvestigationStatus,
    InvestigationWorkCancellation, InvestigationWorkFocus, InvestigationWorkKind,
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
        for informant in self.informants.values() {
            if informant.status() == InformantStatus::Active {
                self.indexes
                    .informants
                    .active_by_character_handler
                    .insert((informant.character(), informant.handler()), informant.id());
                self.indexes.informants.active.insert(informant.id());
            }
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
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                self.indexes
                    .jurisdictions
                    .jurisdictions_by_neighborhood
                    .entry(*neighborhood)
                    .or_default()
                    .insert(jurisdiction.organization());
            }
        }
        for patrol in self.patrol_deployments.values() {
            if patrol.status() == PatrolDeploymentStatus::Active {
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
    pub fn get_investigation(&self, id: InvestigationId) -> Option<&InvestigationRecord> {
        self.investigations.get(&id)
    }
    pub fn get_evidence(&self, id: EvidenceId) -> Option<&EvidenceRecord> {
        self.evidence.get(&id)
    }
    pub fn get_investigation_work(
        &self,
        id: InvestigationWorkId,
    ) -> Option<&InvestigationWorkRecord> {
        self.investigation_work.get(&id)
    }
    pub fn get_case_witness(&self, id: CaseWitnessId) -> Option<&CaseWitnessRecord> {
        self.case_witnesses.get(&id)
    }
    pub fn get_witness_statement(&self, id: WitnessStatementId) -> Option<&WitnessStatementRecord> {
        self.witness_statements.get(&id)
    }
    pub fn get_informant(&self, id: InformantId) -> Option<&InformantRecord> {
        self.informants.get(&id)
    }
    #[cfg(test)]
    pub fn get_informant_disclosure(
        &self,
        id: InformantDisclosureId,
    ) -> Option<&InformantDisclosureRecord> {
        self.informant_disclosures.get(&id)
    }
    pub fn active_informant_for(
        &self,
        character: CharacterId,
        handler: OrganizationId,
    ) -> Option<&InformantRecord> {
        self.indexes
            .informants
            .active_by_character_handler
            .get(&(character, handler))
            .map(|id| {
                self.informants
                    .get(id)
                    .expect("active informant index must reference an informant")
            })
    }
    pub(crate) fn informant_disclosure_for_case_information(
        &self,
        investigation: InvestigationId,
        information: InformationId,
    ) -> Option<&InformantDisclosureRecord> {
        self.indexes
            .informants
            .disclosure_by_case_information
            .get(&(investigation, information))
            .map(|id| {
                self.informant_disclosures
                    .get(id)
                    .expect("informant disclosure index must reference a disclosure")
            })
    }
    pub fn get_jurisdiction(&self, organization: OrganizationId) -> Option<&JurisdictionRecord> {
        self.jurisdictions.get(&organization)
    }
    pub fn get_patrol_deployment(&self, id: PatrolDeploymentId) -> Option<&PatrolDeploymentRecord> {
        self.patrol_deployments.get(&id)
    }
    pub fn get_police_response(&self, id: PoliceResponseId) -> Option<&PoliceResponseRecord> {
        self.police_responses.get(&id)
    }
    pub fn get_arrest(&self, id: ArrestId) -> Option<&ArrestRecord> {
        self.arrests.get(&id)
    }
    pub fn active_arrest_for_character(&self, character: CharacterId) -> Option<&ArrestRecord> {
        self.indexes
            .arrests
            .active_by_character
            .get(&character)
            .map(|id| {
                self.arrests
                    .get(id)
                    .expect("active-arrest index must reference an arrest")
            })
    }
    /// Test-only observation surface; production reads go through case-scoped getters.
    #[cfg(test)]
    pub fn arrests_for_character(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = &ArrestRecord> {
        self.arrests
            .values()
            .filter(move |record| record.character() == character)
    }
    pub fn arrests_for_investigation(
        &self,
        investigation: InvestigationId,
    ) -> impl Iterator<Item = &ArrestRecord> {
        self.indexes
            .arrests
            .by_investigation
            .get(&investigation)
            .into_iter()
            .flatten()
            .map(|id| {
                self.arrests
                    .get(id)
                    .expect("arrest-by-investigation index must reference an arrest")
            })
    }
    pub fn get_legal_representation(
        &self,
        id: LegalRepresentationId,
    ) -> Option<&LegalRepresentationRecord> {
        self.legal_representations.get(&id)
    }
    pub fn active_representation_for_arrest(
        &self,
        arrest: ArrestId,
    ) -> Option<&LegalRepresentationRecord> {
        self.indexes
            .representations
            .active_by_arrest
            .get(&arrest)
            .map(|id| {
                self.legal_representations
                    .get(id)
                    .expect("active-representation index must reference a representation")
            })
    }
    /// Test-only observation surface: production code reads representations through
    /// `active_representation_for_arrest`.
    #[cfg(test)]
    pub fn representations_for_arrest(
        &self,
        arrest: ArrestId,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.legal_representations
            .values()
            .filter(move |record| record.arrest() == arrest)
    }
    pub(crate) fn active_representations_for_contact(
        &self,
        contact: ContactId,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.indexes
            .representations
            .active_by_contact
            .get(&contact)
            .into_iter()
            .flatten()
            .map(|id| {
                self.legal_representations
                    .get(id)
                    .expect("representation-by-contact index must reference a representation")
            })
    }
    pub fn get_prosecution_case(&self, id: ProsecutionCaseId) -> Option<&ProsecutionCaseRecord> {
        self.prosecution_cases.get(&id)
    }
    pub fn get_prosecution_referral(
        &self,
        id: ProsecutionReferralId,
    ) -> Option<&ProsecutionReferralRecord> {
        self.prosecution_referrals.get(&id)
    }
    pub fn open_prosecution_case_for(
        &self,
        arrest: ArrestId,
        prosecutor_office: OrganizationId,
    ) -> Option<&ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .open_by_arrest_office
            .get(&(arrest, prosecutor_office))
            .map(|id| {
                self.prosecution_cases
                    .get(id)
                    .expect("open-prosecution index must reference a prosecution case")
            })
    }
    pub(crate) fn has_other_open_prosecution_case(
        &self,
        arrest: ArrestId,
        except: ProsecutionCaseId,
    ) -> bool {
        self.indexes
            .prosecutions
            .open_by_arrest_office
            .iter()
            .any(|((indexed_arrest, _), case)| *indexed_arrest == arrest && *case != except)
    }
    pub(crate) fn has_open_prosecution_case_for_arrest(&self, arrest: ArrestId) -> bool {
        self.indexes
            .prosecutions
            .open_by_arrest_office
            .keys()
            .any(|(indexed_arrest, _)| *indexed_arrest == arrest)
    }
    /// Test-only observation surface; production reads go through case-scoped getters.
    #[cfg(test)]
    pub fn prosecution_cases_for_arrest(
        &self,
        arrest: ArrestId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.prosecution_cases
            .values()
            .filter(move |record| record.arrest() == arrest)
    }
    pub fn reviewing_prosecution_cases_for_prosecutor(
        &self,
        prosecutor: CharacterId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .reviewing_cases_by_prosecutor
            .get(&prosecutor)
            .into_iter()
            .flatten()
            .map(|id| {
                self.prosecution_cases
                    .get(id)
                    .expect("prosecutor assignment index must reference a prosecution case")
            })
    }

    pub(crate) fn reviewing_prosecution_cases_without_prosecutor(
        &self,
    ) -> impl Iterator<Item = ProsecutionCaseId> + '_ {
        self.indexes
            .prosecutions
            .reviewing_without_prosecutor
            .iter()
            .copied()
    }
    pub fn police_response_for_operation(
        &self,
        operation: OperationId,
    ) -> Option<&PoliceResponseRecord> {
        self.indexes
            .police_responses
            .by_source_operation
            .get(&operation)
            .map(|id| {
                self.police_responses
                    .get(id)
                    .expect("police-response index must reference a response")
            })
    }
    pub(crate) fn find_police_responses_due_at_or_before(
        &self,
        now: SimTime,
    ) -> Vec<PoliceResponseId> {
        self.indexes
            .police_responses
            .dispatched_by_arrival_due
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
    pub fn active_patrol_deployments_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &PatrolDeploymentRecord> {
        self.indexes
            .patrols
            .active_by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .map(|id| {
                self.patrol_deployments
                    .get(id)
                    .expect("active-patrol index must reference a deployment")
            })
    }
    pub(crate) fn active_patrol_for(
        &self,
        organization: OrganizationId,
        neighborhood: NeighborhoodId,
    ) -> Option<&PatrolDeploymentRecord> {
        self.indexes
            .patrols
            .active_by_organization_neighborhood
            .get(&(organization, neighborhood))
            .map(|id| {
                self.patrol_deployments
                    .get(id)
                    .expect("active-patrol pair index must reference a deployment")
            })
    }
    pub fn jurisdictions_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &JurisdictionRecord> {
        self.indexes
            .jurisdictions
            .jurisdictions_by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .map(|organization| {
                self.jurisdictions
                    .get(organization)
                    .expect("jurisdiction-neighborhood index must reference a jurisdiction")
            })
    }
    /// Test-only observation surface: production code reads evidence through case-scoped
    /// getters, so this scans the record set instead of maintaining a by-origin index.
    #[cfg(test)]
    pub fn evidence_from_origin(&self, origin: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence
            .values()
            .filter(move |record| record.origin() == Some(origin))
    }
    pub fn derived_evidence_from(
        &self,
        source: EvidenceId,
    ) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .derived_evidence_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .map(|id| {
                self.evidence
                    .get(id)
                    .expect("derived-evidence index must reference evidence")
            })
    }
    /// Test-only observation surface over the maintained subject index.
    #[cfg(test)]
    pub fn investigations_for_subject(
        &self,
        subject: EntityRef,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes
            .investigations
            .investigations_by_subject
            .get(&subject)
            .into_iter()
            .flatten()
            .map(|id| {
                self.investigations
                    .get(id)
                    .expect("investigation-subject index must reference an investigation")
            })
    }
    pub fn case_witness_for(
        &self,
        investigation: InvestigationId,
        witness: CharacterId,
    ) -> Option<&CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witness_by_case_character
            .get(&(investigation, witness))
            .map(|id| {
                self.case_witnesses
                    .get(id)
                    .expect("case-witness index must reference a witness")
            })
    }
    pub fn case_witnesses_for_investigation(
        &self,
        investigation: InvestigationId,
    ) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witnesses_by_investigation
            .get(&investigation)
            .into_iter()
            .flatten()
            .map(|id| {
                self.case_witnesses
                    .get(id)
                    .expect("witness-by-investigation index must reference a witness")
            })
    }
    /// Canonical statement lookup: each testimony statement owns a unique derived evidence
    /// record, so the by-evidence index is the O(log n) authority for this relation.
    pub fn witness_statement_for_evidence(
        &self,
        evidence: EvidenceId,
    ) -> Option<&WitnessStatementRecord> {
        self.indexes
            .witnesses
            .witness_statement_by_evidence
            .get(&evidence)
            .map(|id| {
                self.witness_statements
                    .get(id)
                    .expect("statement-evidence index must reference a statement")
            })
    }
    pub fn work_for_investigation(
        &self,
        investigation: InvestigationId,
    ) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.indexes
            .work
            .work_by_investigation
            .get(&investigation)
            .into_iter()
            .flatten()
            .map(|id| {
                self.investigation_work
                    .get(id)
                    .expect("work-by-investigation index must reference investigation work")
            })
    }
    pub fn work_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.indexes
            .work
            .work_by_investigator
            .get(&investigator)
            .into_iter()
            .flatten()
            .map(|id| {
                self.investigation_work
                    .get(id)
                    .expect("work-by-investigator index must reference investigation work")
            })
    }
    pub(crate) fn scheduled_work_for_focus(
        &self,
        investigation: InvestigationId,
        kind: InvestigationWorkKind,
        focus: InvestigationWorkFocus,
    ) -> Option<&InvestigationWorkRecord> {
        self.indexes
            .work
            .scheduled_work_by_focus
            .get(&(investigation, kind, focus))
            .map(|id| {
                self.investigation_work
                    .get(id)
                    .expect("scheduled-work focus index must reference investigation work")
            })
    }
    pub(crate) fn find_investigation_work_due_at_or_before(
        &self,
        now: SimTime,
    ) -> Vec<InvestigationWorkId> {
        self.indexes
            .work
            .scheduled_work_by_due_at
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
    /// Test-only observation surface; production reads go through case-scoped getters.
    #[cfg(test)]
    pub fn evidence_of_kind(
        &self,
        kind: crate::legal::EvidenceKind,
    ) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence
            .values()
            .filter(move |record| record.kind() == kind)
    }
    pub fn investigations_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes
            .investigations
            .investigations_by_investigator
            .get(&investigator)
            .into_iter()
            .flatten()
            .map(|id| {
                self.investigations
                    .get(id)
                    .expect("investigator-case index must reference an investigation")
            })
    }
    pub fn investigations_for_owner(
        &self,
        owner: OrganizationId,
    ) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes
            .investigations
            .by_owner
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|id| {
                self.investigations
                    .get(id)
                    .expect("investigation-owner index must reference an investigation")
            })
    }
    pub(crate) fn active_investigation_for_investigator(
        &self,
        investigator: CharacterId,
    ) -> Option<&InvestigationRecord> {
        self.investigations_for_investigator(investigator)
            .find(|investigation| investigation.status() == InvestigationStatus::Active)
    }
    pub(crate) fn investigations(&self) -> impl Iterator<Item = &InvestigationRecord> {
        self.investigations.values()
    }
    pub(crate) fn active_investigations_without_lead(
        &self,
    ) -> impl Iterator<Item = InvestigationId> + '_ {
        self.indexes
            .investigations
            .active_without_lead
            .iter()
            .copied()
    }
    /// Every active case in id order; per-tick institutional passes scan this instead of
    /// the full case history.
    pub(crate) fn active_investigations(&self) -> impl Iterator<Item = &InvestigationRecord> {
        self.indexes.investigations.active.iter().map(|id| {
            self.investigations
                .get(id)
                .expect("active-investigation index must reference an investigation")
        })
    }
    pub(crate) fn investigation_work(&self) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.investigation_work.values()
    }
    pub(crate) fn case_witnesses(&self) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.case_witnesses.values()
    }
    /// Every case registration naming `character` as witness, in id order; witness-pressure
    /// authorization and resolution scan this instead of the full witness history.
    pub(crate) fn case_witnesses_for_character(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witnesses_by_character
            .get(&character)
            .into_iter()
            .flatten()
            .map(|id| {
                self.case_witnesses
                    .get(id)
                    .expect("witness-character index must reference a witness")
            })
    }
    pub(crate) fn witness_statements(&self) -> impl Iterator<Item = &WitnessStatementRecord> {
        self.witness_statements.values()
    }
    pub(crate) fn informants(&self) -> impl Iterator<Item = &InformantRecord> {
        self.informants.values()
    }
    /// Every active informant in id order; the disclosure pass scans this instead of the
    /// full terminated-and-active informant history.
    pub(crate) fn active_informants(&self) -> impl Iterator<Item = &InformantRecord> {
        self.indexes.informants.active.iter().map(|id| {
            self.informants
                .get(id)
                .expect("active-informant index must reference an informant")
        })
    }
    /// O(1) emptiness probe over the active-informant index, so per-tick passes that build
    /// cross-referenced views (handler-to-case maps) can skip that work entirely on quiet
    /// ticks without changing what they would have produced.
    pub(crate) fn has_active_informants(&self) -> bool {
        !self.indexes.informants.active.is_empty()
    }
    pub(crate) fn informant_disclosures(&self) -> impl Iterator<Item = &InformantDisclosureRecord> {
        self.informant_disclosures.values()
    }
    pub(crate) fn all_evidence(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.evidence.values()
    }
    pub(crate) fn jurisdictions(&self) -> impl Iterator<Item = &JurisdictionRecord> {
        self.jurisdictions.values()
    }
    pub(crate) fn patrol_deployments(&self) -> impl Iterator<Item = &PatrolDeploymentRecord> {
        self.patrol_deployments.values()
    }
    pub(crate) fn police_responses(&self) -> impl Iterator<Item = &PoliceResponseRecord> {
        self.police_responses.values()
    }
    pub(crate) fn arrests(&self) -> impl Iterator<Item = &ArrestRecord> {
        self.arrests.values()
    }
    /// Raw-id extremes of every id-keyed collection, read from key order; these feed
    /// allocator validation without walking full record histories.
    pub(crate) fn investigation_id_bounds(&self) -> Option<(u32, u32)> {
        self.investigations.id_bounds()
    }
    pub(crate) fn investigation_work_id_bounds(&self) -> Option<(u32, u32)> {
        self.investigation_work.id_bounds()
    }
    pub(crate) fn patrol_deployment_id_bounds(&self) -> Option<(u32, u32)> {
        self.patrol_deployments.id_bounds()
    }
    pub(crate) fn police_response_id_bounds(&self) -> Option<(u32, u32)> {
        self.police_responses.id_bounds()
    }
    pub(crate) fn case_witness_id_bounds(&self) -> Option<(u32, u32)> {
        self.case_witnesses.id_bounds()
    }
    pub(crate) fn witness_statement_id_bounds(&self) -> Option<(u32, u32)> {
        self.witness_statements.id_bounds()
    }
    pub(crate) fn informant_id_bounds(&self) -> Option<(u32, u32)> {
        self.informants.id_bounds()
    }
    pub(crate) fn informant_disclosure_id_bounds(&self) -> Option<(u32, u32)> {
        self.informant_disclosures.id_bounds()
    }
    pub(crate) fn evidence_id_bounds(&self) -> Option<(u32, u32)> {
        self.evidence.id_bounds()
    }
    pub(crate) fn arrest_id_bounds(&self) -> Option<(u32, u32)> {
        self.arrests.id_bounds()
    }
    pub(crate) fn legal_representation_id_bounds(&self) -> Option<(u32, u32)> {
        self.legal_representations.id_bounds()
    }
    pub(crate) fn prosecution_case_id_bounds(&self) -> Option<(u32, u32)> {
        self.prosecution_cases.id_bounds()
    }
    pub(crate) fn prosecution_referral_id_bounds(&self) -> Option<(u32, u32)> {
        self.prosecution_referrals.id_bounds()
    }
    /// Every currently detained arrest in id order; per-tick custody passes scan this
    /// instead of the full arrest history.
    pub(crate) fn detained_arrests(&self) -> impl Iterator<Item = &ArrestRecord> {
        self.indexes.arrests.detained.iter().map(|id| {
            self.arrests
                .get(id)
                .expect("detained-arrest index must reference an arrest")
        })
    }
    /// O(1) emptiness probes over the custody-cluster indexes, so per-tick passes can skip
    /// their cross-referenced scans entirely on ticks with no live custody work.
    pub(crate) fn has_detained_arrests(&self) -> bool {
        !self.indexes.arrests.detained.is_empty()
    }
    pub(crate) fn has_active_automatic_policy_representations(&self) -> bool {
        !self
            .indexes
            .representations
            .active_automatic_policy
            .is_empty()
    }
    pub(crate) fn legal_representations(&self) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.legal_representations.values()
    }
    /// Active representations retained through automatic policy, in id order; the custody
    /// sweep scans this instead of the full representation history.
    pub(crate) fn active_automatic_policy_representations(
        &self,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.indexes
            .representations
            .active_automatic_policy
            .iter()
            .map(|id| {
                self.legal_representations
                    .get(id)
                    .expect("automatic-representation index must reference a representation")
            })
    }
    pub(crate) fn prosecution_cases(&self) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.prosecution_cases.values()
    }
    pub(crate) fn prosecution_referrals(&self) -> impl Iterator<Item = &ProsecutionReferralRecord> {
        self.prosecution_referrals.values()
    }
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

    /// Adds incident-declared subject matter to an existing investigation while preserving the
    /// subject index. Incident intake uses this when a suspended originated shelf is resumed:
    /// opening a new file and continuing an old one must not disagree about what the same
    /// validated incident is about merely because some fresh evidence is still weak.
    pub(crate) fn extend_investigation_subjects(
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
            for subject in subjects {
                if investigation.subjects.insert(subject) {
                    added.push(subject);
                }
            }
            if added.is_empty() {
                return;
            }
            investigation.version = investigation
                .version
                .checked_add(1)
                .expect("investigation version counter exhausted");
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

    pub(crate) fn find_active_cases_inactive_since(&self, at: SimTime) -> Vec<InvestigationId> {
        let mut candidates = Vec::new();
        for (_, ids) in self
            .indexes
            .investigations
            .cases_by_last_activity
            .range(..=at)
        {
            candidates.extend(ids.iter().copied());
        }
        candidates
    }
    pub(crate) fn insert_informant(&mut self, record: InformantRecord) {
        let id = record.id();
        let key = (record.character(), record.handler());
        debug_assert_eq!(
            record.status(),
            InformantStatus::Active,
            "Lifecycle Validity: new informant relationships must be active"
        );
        let previous_active = self
            .indexes
            .informants
            .active_by_character_handler
            .insert(key, id);
        debug_assert!(
            previous_active.is_none(),
            "Ownership Exclusivity: duplicate active informant relationship inserted"
        );
        self.indexes.informants.active.insert(id);
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
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        let previous = self.case_witnesses.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate case witness ID inserted"
        );
        self.set_investigation_activity(investigation_id, activity_at);
    }
    /// Updates witness cooperation and resets the case's cold-case inactivity clock:
    /// cooperation directly drives future interview support scoring.
    pub(crate) fn set_witness_cooperation(
        &mut self,
        case_witness: CaseWitnessId,
        cooperation: WitnessCooperation,
        activity_at: SimTime,
    ) {
        let investigation_id = {
            let record = self
                .case_witnesses
                .get_mut(&case_witness)
                .expect("validated case witness disappeared before cooperation commit");
            record.cooperation = cooperation;
            record.version = record
                .version
                .checked_add(1)
                .expect("case witness version counter exhausted");
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before witness cooperation commit");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.set_investigation_activity(investigation_id, activity_at);
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
        witness.version = witness
            .version
            .checked_add(1)
            .expect("case witness version counter exhausted");
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before statement commit");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
                    .expect("witness interview attempt counter exhausted");
                witness.version = witness
                    .version
                    .checked_add(1)
                    .expect("case witness version counter exhausted");
            }
            record.runtime.status = InvestigationWorkStatus::Completed;
            record.runtime.resolution = Some(resolution);
            record.runtime.cancellation = None;
            record.runtime.version = record
                .runtime
                .version
                .checked_add(1)
                .expect("investigation work version counter exhausted");
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work completion");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.set_investigation_activity(investigation_id, resolved_at);
    }

    pub(crate) fn set_investigation_work_cancellation(
        &mut self,
        id: InvestigationWorkId,
        cancellation: InvestigationWorkCancellation,
    ) {
        let cancelled_at = cancellation.cancelled_at();
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
            record.runtime.version = record
                .runtime
                .version
                .checked_add(1)
                .expect("investigation work version counter exhausted");
            record.investigation()
        };
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before work cancellation");
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.set_investigation_activity(investigation_id, cancelled_at);
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
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        at: SimTime,
    ) {
        let record = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before detention staffing release");
        assert_eq!(record.status, InvestigationStatus::Active);
        assert_eq!(record.lead_investigator, Some(investigator));
        record.lead_investigator = None;
        record.version = record
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        self.set_investigation_activity(investigation_id, at);
    }
    pub(crate) fn set_jurisdiction(&mut self, record: JurisdictionRecord) {
        let organization = record.organization();
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
        for neighborhood in record.neighborhoods() {
            self.indexes
                .jurisdictions
                .jurisdictions_by_neighborhood
                .entry(*neighborhood)
                .or_default()
                .insert(organization);
        }
        self.jurisdictions.insert(organization, record);
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
        record.windows = windows;
        record.last_changed_at = changed_at;
        record.version = record
            .version
            .checked_add(1)
            .expect("patrol deployment version counter exhausted");
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
        record.status = status;
        record.last_changed_at = changed_at;
        record.version = record
            .version
            .checked_add(1)
            .expect("patrol deployment version counter exhausted");
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
        record.state.version = record
            .state
            .version
            .checked_add(1)
            .expect("police response version counter exhausted");
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
        record.version = record
            .version
            .checked_add(1)
            .expect("arrest version counter exhausted");
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
        record.version = record
            .version
            .checked_add(1)
            .expect("legal representation version counter exhausted");
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
        case.version = case
            .version
            .checked_add(1)
            .expect("prosecution case version counter exhausted");
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
        case.version = case
            .version
            .checked_add(1)
            .expect("prosecution case version counter exhausted");
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
        case.version = case
            .version
            .checked_add(1)
            .expect("prosecution case version counter exhausted");
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
        case.version = case
            .version
            .checked_add(1)
            .expect("prosecution case version counter exhausted");
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
