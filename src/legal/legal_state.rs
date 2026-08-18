//! `LegalState` ownership, index-synchronizing mutators, and read-only observation.
//!
//! `LegalState` is the single owner of all legal records and their derived indexes
//! (see `records.rs` for the record definitions). Every mutator validates, resolves,
//! commits, and re-synchronizes the indexes in one atomic method; readers observe
//! through read-only getters or the `has_consistent_*` projection checks.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CaseWitnessId, CharacterId, ContactId, EvidenceId, InformantDisclosureId,
    InformantId, InformationId, InvestigationId, InvestigationWorkId, LegalRepresentationId,
    NeighborhoodId, OperationId, OrganizationId, PatrolDeploymentId, PoliceResponseId,
    ProsecutionCaseId, ProsecutionReferralId, ReportId, WitnessStatementId,
};
use crate::core::time::SimTime;
use crate::legal::records::{
    ArrestRecord, ArrestStatus, CaseWitnessRecord, EvidenceKind, EvidenceRecord,
    InformantDisclosureRecord, InformantRecord, InformantStatus, InvestigationRecord,
    InvestigationStatus, InvestigationWorkFocus, InvestigationWorkKind, InvestigationWorkRecord,
    InvestigationWorkResolution, InvestigationWorkStatus, InvestigatorRole, JurisdictionRecord,
    LegalIndexes, LegalRepresentationEndReason, LegalRepresentationRecord,
    LegalRepresentationStatus, PatrolDeploymentRecord, PatrolDeploymentStatus, PatrolWindow,
    PoliceResponseRecord, PoliceResponseStatus, ProsecutionCaseRecord, ProsecutionCaseResolution,
    ProsecutionCaseStatus, ProsecutionReferralRecord, WitnessCooperation, WitnessStatementRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LegalState {
    investigations: BTreeMap<InvestigationId, InvestigationRecord>,
    investigation_work: BTreeMap<InvestigationWorkId, InvestigationWorkRecord>,
    case_witnesses: BTreeMap<CaseWitnessId, CaseWitnessRecord>,
    witness_statements: BTreeMap<WitnessStatementId, WitnessStatementRecord>,
    informants: BTreeMap<InformantId, InformantRecord>,
    informant_disclosures: BTreeMap<InformantDisclosureId, InformantDisclosureRecord>,
    evidence: BTreeMap<EvidenceId, EvidenceRecord>,
    jurisdictions: BTreeMap<OrganizationId, JurisdictionRecord>,
    patrol_deployments: BTreeMap<PatrolDeploymentId, PatrolDeploymentRecord>,
    police_responses: BTreeMap<PoliceResponseId, PoliceResponseRecord>,
    arrests: BTreeMap<ArrestId, ArrestRecord>,
    legal_representations: BTreeMap<LegalRepresentationId, LegalRepresentationRecord>,
    prosecution_cases: BTreeMap<ProsecutionCaseId, ProsecutionCaseRecord>,
    prosecution_referrals: BTreeMap<ProsecutionReferralId, ProsecutionReferralRecord>,
    indexes: LegalIndexes,
}

impl LegalState {
    pub(crate) fn new() -> Self {
        Self::default()
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
            .and_then(|id| self.informants.get(id))
    }
    pub fn informants_for_character(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = &InformantRecord> {
        self.indexes
            .informants
            .by_character
            .get(&character)
            .into_iter()
            .flatten()
            .filter_map(|id| self.informants.get(id))
    }
    pub fn informants_for_handler(
        &self,
        handler: OrganizationId,
    ) -> impl Iterator<Item = &InformantRecord> {
        self.indexes
            .informants
            .by_handler
            .get(&handler)
            .into_iter()
            .flatten()
            .filter_map(|id| self.informants.get(id))
    }
    pub fn disclosures_for_informant(
        &self,
        informant: InformantId,
    ) -> impl Iterator<Item = &InformantDisclosureRecord> {
        self.indexes
            .informants
            .disclosures_by_informant
            .get(&informant)
            .into_iter()
            .flatten()
            .filter_map(|id| self.informant_disclosures.get(id))
    }
    pub fn informant_disclosure_for_evidence(
        &self,
        evidence: EvidenceId,
    ) -> Option<&InformantDisclosureRecord> {
        self.indexes
            .informants
            .disclosure_by_evidence
            .get(&evidence)
            .and_then(|id| self.informant_disclosures.get(id))
    }
    pub fn informant_disclosures_from_information(
        &self,
        information: InformationId,
    ) -> impl Iterator<Item = &InformantDisclosureRecord> {
        self.indexes
            .informants
            .disclosures_by_information
            .get(&information)
            .into_iter()
            .flatten()
            .filter_map(|id| self.informant_disclosures.get(id))
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
            .and_then(|id| self.informant_disclosures.get(id))
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
            .and_then(|id| self.arrests.get(id))
    }
    pub fn arrests_for_character(
        &self,
        character: CharacterId,
    ) -> impl Iterator<Item = &ArrestRecord> {
        self.indexes
            .arrests
            .by_character
            .get(&character)
            .into_iter()
            .flatten()
            .filter_map(|id| self.arrests.get(id))
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
            .filter_map(|id| self.arrests.get(id))
    }
    pub fn arrests_for_authority(
        &self,
        authority: OrganizationId,
    ) -> impl Iterator<Item = &ArrestRecord> {
        self.indexes
            .arrests
            .by_authority
            .get(&authority)
            .into_iter()
            .flatten()
            .filter_map(|id| self.arrests.get(id))
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
            .and_then(|id| self.legal_representations.get(id))
    }
    pub fn representations_for_arrest(
        &self,
        arrest: ArrestId,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.indexes
            .representations
            .by_arrest
            .get(&arrest)
            .into_iter()
            .flatten()
            .filter_map(|id| self.legal_representations.get(id))
    }
    pub fn representations_for_defendant(
        &self,
        defendant: CharacterId,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.indexes
            .representations
            .by_defendant
            .get(&defendant)
            .into_iter()
            .flatten()
            .filter_map(|id| self.legal_representations.get(id))
    }
    pub fn representations_for_sponsor(
        &self,
        sponsor: OrganizationId,
    ) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.indexes
            .representations
            .by_sponsor
            .get(&sponsor)
            .into_iter()
            .flatten()
            .filter_map(|id| self.legal_representations.get(id))
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
            .filter_map(|id| self.legal_representations.get(id))
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
            .and_then(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_cases_for_arrest(
        &self,
        arrest: ArrestId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .cases_by_arrest
            .get(&arrest)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_cases_for_defendant(
        &self,
        defendant: CharacterId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .cases_by_defendant
            .get(&defendant)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_cases_for_office(
        &self,
        office: OrganizationId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .cases_by_office
            .get(&office)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_cases_for_lead(
        &self,
        lead: CharacterId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .cases_by_lead
            .get(&lead)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_cases_with_evidence(
        &self,
        evidence: EvidenceId,
    ) -> impl Iterator<Item = &ProsecutionCaseRecord> {
        self.indexes
            .prosecutions
            .cases_by_evidence
            .get(&evidence)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_cases.get(id))
    }
    pub fn prosecution_referrals_for_case(
        &self,
        case: ProsecutionCaseId,
    ) -> impl Iterator<Item = &ProsecutionReferralRecord> {
        self.indexes
            .prosecutions
            .referrals_by_case
            .get(&case)
            .into_iter()
            .flatten()
            .filter_map(|id| self.prosecution_referrals.get(id))
    }
    pub fn police_response_for_operation(
        &self,
        operation: OperationId,
    ) -> Option<&PoliceResponseRecord> {
        self.indexes
            .police_responses
            .by_source_operation
            .get(&operation)
            .and_then(|id| self.police_responses.get(id))
    }
    pub fn police_responses_for_authority(
        &self,
        authority: OrganizationId,
    ) -> impl Iterator<Item = &PoliceResponseRecord> {
        self.indexes
            .police_responses
            .by_authority
            .get(&authority)
            .into_iter()
            .flatten()
            .filter_map(|id| self.police_responses.get(id))
    }
    pub fn police_responses_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &PoliceResponseRecord> {
        self.indexes
            .police_responses
            .by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .filter_map(|id| self.police_responses.get(id))
    }
    pub(crate) fn due_police_responses_at_or_before(&self, now: SimTime) -> Vec<PoliceResponseId> {
        self.indexes
            .police_responses
            .dispatched_by_arrival_due
            .range(..=now)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }
    pub fn patrol_deployments_for_organization(
        &self,
        organization: OrganizationId,
    ) -> impl Iterator<Item = &PatrolDeploymentRecord> {
        self.indexes
            .patrols
            .by_organization
            .get(&organization)
            .into_iter()
            .flatten()
            .filter_map(|id| self.patrol_deployments.get(id))
    }
    pub fn patrol_deployments_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &PatrolDeploymentRecord> {
        self.indexes
            .patrols
            .by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .filter_map(|id| self.patrol_deployments.get(id))
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
            .filter_map(|id| self.patrol_deployments.get(id))
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
            .and_then(|id| self.patrol_deployments.get(id))
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
            .filter_map(|organization| self.jurisdictions.get(organization))
    }
    pub fn evidence_from_origin(&self, origin: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_origin
            .get(&origin)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }
    pub fn evidence_from_source(&self, source: EntityRef) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
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
            .filter_map(|id| self.evidence.get(id))
    }
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
            .filter_map(|id| self.investigations.get(id))
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
            .and_then(|id| self.case_witnesses.get(id))
    }
    pub fn case_witnesses_for_character(
        &self,
        witness: CharacterId,
    ) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.indexes
            .witnesses
            .case_witnesses_by_character
            .get(&witness)
            .into_iter()
            .flatten()
            .filter_map(|id| self.case_witnesses.get(id))
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
            .filter_map(|id| self.case_witnesses.get(id))
    }
    pub fn witness_statement_for_evidence(
        &self,
        evidence: EvidenceId,
    ) -> Option<&WitnessStatementRecord> {
        self.indexes
            .witnesses
            .witness_statement_by_evidence
            .get(&evidence)
            .and_then(|id| self.witness_statements.get(id))
    }
    pub fn statements_for_case_witness(
        &self,
        case_witness: CaseWitnessId,
    ) -> impl Iterator<Item = &WitnessStatementRecord> {
        self.case_witnesses
            .get(&case_witness)
            .into_iter()
            .flat_map(|witness| witness.statements().iter())
            .filter_map(|id| self.witness_statements.get(id))
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
            .filter_map(|id| self.investigation_work.get(id))
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
            .filter_map(|id| self.investigation_work.get(id))
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
            .and_then(|id| self.investigation_work.get(id))
    }
    pub(crate) fn due_investigation_work_at_or_before(
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
    pub fn evidence_of_kind(&self, kind: EvidenceKind) -> impl Iterator<Item = &EvidenceRecord> {
        self.indexes
            .evidence
            .evidence_by_kind
            .get(&kind)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
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
            .filter_map(|id| self.investigations.get(id))
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
            .filter_map(|id| self.investigations.get(id))
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
    pub(crate) fn investigation_work(&self) -> impl Iterator<Item = &InvestigationWorkRecord> {
        self.investigation_work.values()
    }
    pub(crate) fn case_witnesses(&self) -> impl Iterator<Item = &CaseWitnessRecord> {
        self.case_witnesses.values()
    }
    pub(crate) fn witness_statements(&self) -> impl Iterator<Item = &WitnessStatementRecord> {
        self.witness_statements.values()
    }
    pub(crate) fn informants(&self) -> impl Iterator<Item = &InformantRecord> {
        self.informants.values()
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
    pub(crate) fn legal_representations(&self) -> impl Iterator<Item = &LegalRepresentationRecord> {
        self.legal_representations.values()
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

    /// Advances a case's last-activity instant and re-synchronizes the cold-decay index in one
    /// atomic step. Called by every consequence-bearing legal transition: incident intake,
    /// evidence insertion, investigation-work scheduling, and investigation-work resolution.
    pub(crate) fn note_investigation_activity(
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

    pub(crate) fn active_case_ids_with_last_activity_at_or_before(
        &self,
        at: SimTime,
    ) -> Vec<InvestigationId> {
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
        self.indexes
            .informants
            .by_character
            .entry(record.character())
            .or_default()
            .insert(id);
        self.indexes
            .informants
            .by_handler
            .entry(record.handler())
            .or_default()
            .insert(id);
        let previous = self.informants.insert(id, record);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate informant ID inserted"
        );
    }
    pub(crate) fn terminate_informant(&mut self, id: InformantId, terminated_at: SimTime) {
        let (character, handler) = {
            let record = self
                .informants
                .get_mut(&id)
                .expect("validated informant disappeared before termination commit");
            record.status = InformantStatus::Terminated;
            record.terminated_at = Some(terminated_at);
            record.version = record
                .version
                .checked_add(1)
                .expect("informant version counter exhausted");
            (record.character(), record.handler())
        };
        let removed = self
            .indexes
            .informants
            .active_by_character_handler
            .remove(&(character, handler));
        debug_assert_eq!(
            removed,
            Some(id),
            "Derived Data Consistency: active informant index changed before termination"
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
        self.indexes
            .informants
            .disclosures_by_informant
            .entry(disclosure.informant())
            .or_default()
            .insert(id);
        let previous_evidence = self
            .indexes
            .informants
            .disclosure_by_evidence
            .insert(disclosure.evidence(), id);
        debug_assert!(
            previous_evidence.is_none(),
            "Ownership Exclusivity: evidence is linked to multiple informant disclosures"
        );
        self.indexes
            .informants
            .disclosures_by_information
            .entry(disclosure.source_information())
            .or_default()
            .insert(id);
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
        investigation.subjects.insert(record.subject());
        investigation.evidence.insert(record.id());
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
        self.indexes
            .investigations
            .investigations_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.investigation());
        self.indexes
            .evidence
            .evidence_by_subject
            .entry(record.subject())
            .or_default()
            .insert(record.id());
        if let Some(origin) = record.origin() {
            self.indexes
                .evidence
                .evidence_by_origin
                .entry(origin)
                .or_default()
                .insert(record.id());
        }
        if let Some(source) = record.source() {
            self.indexes
                .evidence
                .evidence_by_source
                .entry(source)
                .or_default()
                .insert(record.id());
        }
        self.indexes
            .evidence
            .evidence_by_kind
            .entry(record.kind())
            .or_default()
            .insert(record.id());
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
        self.note_investigation_activity(investigation_id, activity_at);
    }
    pub(crate) fn insert_case_witness(&mut self, record: CaseWitnessRecord) {
        let id = record.id();
        let key = (record.investigation(), record.witness());
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
            .case_witnesses_by_character
            .entry(record.witness())
            .or_default()
            .insert(id);
        self.indexes
            .witnesses
            .case_witnesses_by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        let investigation = self
            .investigations
            .get_mut(&record.investigation())
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
    }
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
        self.note_investigation_activity(investigation_id, scheduled_at);
    }
    pub(crate) fn complete_investigation_work(
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
            record.runtime.status = InvestigationWorkStatus::Completed;
            record.runtime.resolution = Some(resolution);
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
        self.note_investigation_activity(investigation_id, resolved_at);
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
    pub(crate) fn set_investigator_role(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
        role: InvestigatorRole,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing commit");
        investigation.assigned_investigators.insert(investigator);
        match role {
            InvestigatorRole::Lead => investigation.lead_investigator = Some(investigator),
            InvestigatorRole::Investigator => {
                if investigation.lead_investigator == Some(investigator) {
                    investigation.lead_investigator = None;
                }
            }
        }
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        self.indexes
            .investigations
            .investigations_by_investigator
            .entry(investigator)
            .or_default()
            .insert(investigation_id);
    }
    pub(crate) fn remove_investigator(
        &mut self,
        investigation_id: InvestigationId,
        investigator: CharacterId,
    ) {
        let investigation = self
            .investigations
            .get_mut(&investigation_id)
            .expect("validated investigation disappeared before staffing commit");
        let removed = investigation.assigned_investigators.remove(&investigator);
        debug_assert!(
            removed,
            "validated investigator assignment disappeared before commit"
        );
        if investigation.lead_investigator == Some(investigator) {
            investigation.lead_investigator = None;
        }
        investigation.version = investigation
            .version
            .checked_add(1)
            .expect("investigation version counter exhausted");
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
        if let Some(investigations) = self
            .indexes
            .investigations
            .investigations_by_investigator
            .get_mut(&investigator)
        {
            investigations.remove(&investigation_id);
            if investigations.is_empty() {
                self.indexes
                    .investigations
                    .investigations_by_investigator
                    .remove(&investigator);
            }
        }
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
        self.indexes
            .patrols
            .by_organization
            .entry(organization)
            .or_default()
            .insert(id);
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
        self.indexes
            .police_responses
            .by_authority
            .entry(record.authority())
            .or_default()
            .insert(id);
        self.indexes
            .police_responses
            .by_neighborhood
            .entry(record.neighborhood())
            .or_default()
            .insert(id);
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
    pub(crate) fn mark_police_response_arrived(&mut self, id: PoliceResponseId, at: SimTime) {
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
            .by_character
            .entry(record.character())
            .or_default()
            .insert(id);
        self.indexes
            .arrests
            .by_investigation
            .entry(record.investigation())
            .or_default()
            .insert(id);
        self.indexes
            .arrests
            .by_authority
            .entry(record.authority())
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
        self.indexes
            .representations
            .by_arrest
            .entry(record.arrest())
            .or_default()
            .insert(id);
        self.indexes
            .representations
            .by_defendant
            .entry(record.defendant())
            .or_default()
            .insert(id);
        self.indexes
            .representations
            .by_sponsor
            .entry(record.sponsor())
            .or_default()
            .insert(id);
        self.indexes
            .representations
            .by_counsel
            .entry(record.counsel())
            .or_default()
            .insert(id);
        self.indexes
            .representations
            .by_contact
            .entry(record.contact())
            .or_default()
            .insert(id);
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
        let (arrest, contact) = {
            let record = self
                .legal_representations
                .get(&id)
                .expect("validated legal representation disappeared before end commit");
            (record.arrest(), record.contact())
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
        self.indexes
            .prosecutions
            .cases_by_arrest
            .entry(case.arrest())
            .or_default()
            .insert(case_id);
        self.indexes
            .prosecutions
            .cases_by_defendant
            .entry(case.defendant())
            .or_default()
            .insert(case_id);
        self.indexes
            .prosecutions
            .cases_by_source_investigation
            .entry(case.source_investigation())
            .or_default()
            .insert(case_id);
        self.indexes
            .prosecutions
            .cases_by_office
            .entry(case.prosecutor_office())
            .or_default()
            .insert(case_id);
        self.indexes
            .prosecutions
            .cases_by_lead
            .entry(case.lead_prosecutor())
            .or_default()
            .insert(case_id);
        for evidence in case.evidence() {
            self.indexes
                .prosecutions
                .cases_by_evidence
                .entry(*evidence)
                .or_default()
                .insert(case_id);
            self.indexes
                .prosecutions
                .referrals_by_evidence
                .entry(*evidence)
                .or_default()
                .insert(referral_id);
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
            self.indexes
                .prosecutions
                .cases_by_evidence
                .entry(*evidence)
                .or_default()
                .insert(case_id);
            self.indexes
                .prosecutions
                .referrals_by_evidence
                .entry(*evidence)
                .or_default()
                .insert(referral_id);
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
    pub(crate) fn resolve_prosecution_case(
        &mut self,
        id: ProsecutionCaseId,
        resolution: ProsecutionCaseResolution,
        resolved_at: SimTime,
        information: InformationId,
        report: ReportId,
    ) {
        let (arrest, office) = {
            let case = self
                .prosecution_cases
                .get(&id)
                .expect("validated prosecution case disappeared before resolution commit");
            debug_assert_eq!(case.status(), ProsecutionCaseStatus::Reviewing);
            (case.arrest(), case.prosecutor_office())
        };
        let removed = self
            .indexes
            .prosecutions
            .open_by_arrest_office
            .remove(&(arrest, office));
        debug_assert_eq!(removed, Some(id));
        let case = self
            .prosecution_cases
            .get_mut(&id)
            .expect("validated prosecution case disappeared before resolution commit");
        case.lifecycle.status = resolution.status();
        case.lifecycle.resolved_at = Some(resolved_at);
        case.resolution_artifacts.resolution_information = Some(information);
        case.resolution_artifacts.resolution_report = Some(report);
        case.version = case
            .version
            .checked_add(1)
            .expect("prosecution case version counter exhausted");
    }
    fn has_consistent_prosecution_indexes(&self) -> bool {
        for case in self.prosecution_cases.values() {
            let id = case.id();
            if !self
                .indexes
                .prosecutions
                .cases_by_arrest
                .get(&case.arrest())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .prosecutions
                    .cases_by_defendant
                    .get(&case.defendant())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .prosecutions
                    .cases_by_source_investigation
                    .get(&case.source_investigation())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .prosecutions
                    .cases_by_office
                    .get(&case.prosecutor_office())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .prosecutions
                    .cases_by_lead
                    .get(&case.lead_prosecutor())
                    .is_some_and(|ids| ids.contains(&id))
                || case.evidence().iter().any(|evidence| {
                    !self
                        .indexes
                        .prosecutions
                        .cases_by_evidence
                        .get(evidence)
                        .is_some_and(|ids| ids.contains(&id))
                })
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
                    return false
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
                || referral.evidence().iter().any(|evidence| {
                    !self
                        .indexes
                        .prosecutions
                        .referrals_by_evidence
                        .get(evidence)
                        .is_some_and(|ids| ids.contains(&referral.id()))
                })
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
        true
    }
    fn has_consistent_legal_representation_indexes(&self) -> bool {
        for record in self.legal_representations.values() {
            let id = record.id();
            if !self
                .indexes
                .representations
                .by_arrest
                .get(&record.arrest())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .representations
                    .by_defendant
                    .get(&record.defendant())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .representations
                    .by_sponsor
                    .get(&record.sponsor())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .representations
                    .by_counsel
                    .get(&record.counsel())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .representations
                    .by_contact
                    .get(&record.contact())
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
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
                    return false
                }
                LegalRepresentationStatus::Ended
                    if arrest_active == Some(&id) || contact_active =>
                {
                    return false
                }
                LegalRepresentationStatus::Active | LegalRepresentationStatus::Ended => {}
            }
        }
        for (arrest, id) in &self.indexes.representations.active_by_arrest {
            if !self.legal_representations.get(id).is_some_and(|record| {
                record.arrest() == *arrest && record.status() == LegalRepresentationStatus::Active
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
        for (arrest, ids) in &self.indexes.representations.by_arrest {
            if ids.iter().any(|id| {
                !self
                    .legal_representations
                    .get(id)
                    .is_some_and(|record| record.arrest() == *arrest)
            }) {
                return false;
            }
        }
        for (defendant, ids) in &self.indexes.representations.by_defendant {
            if ids.iter().any(|id| {
                !self
                    .legal_representations
                    .get(id)
                    .is_some_and(|record| record.defendant() == *defendant)
            }) {
                return false;
            }
        }
        for (sponsor, ids) in &self.indexes.representations.by_sponsor {
            if ids.iter().any(|id| {
                !self
                    .legal_representations
                    .get(id)
                    .is_some_and(|record| record.sponsor() == *sponsor)
            }) {
                return false;
            }
        }
        for (counsel, ids) in &self.indexes.representations.by_counsel {
            if ids.iter().any(|id| {
                !self
                    .legal_representations
                    .get(id)
                    .is_some_and(|record| record.counsel() == *counsel)
            }) {
                return false;
            }
        }
        for (contact, ids) in &self.indexes.representations.by_contact {
            if ids.iter().any(|id| {
                !self
                    .legal_representations
                    .get(id)
                    .is_some_and(|record| record.contact() == *contact)
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
                .by_character
                .get(&arrest.character())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .arrests
                    .by_investigation
                    .get(&arrest.investigation())
                    .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .arrests
                    .by_authority
                    .get(&arrest.authority())
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
        }
        for (character, ids) in &self.indexes.arrests.by_character {
            if ids.iter().any(|id| {
                !self
                    .arrests
                    .get(id)
                    .is_some_and(|record| record.character() == *character)
            }) {
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
        for (authority, ids) in &self.indexes.arrests.by_authority {
            if ids.iter().any(|id| {
                !self
                    .arrests
                    .get(id)
                    .is_some_and(|record| record.authority() == *authority)
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
        true
    }
    fn has_consistent_police_response_indexes(&self) -> bool {
        for response in self.police_responses.values() {
            let id = response.id();
            if !self
                .indexes
                .police_responses
                .by_authority
                .get(&response.authority())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .police_responses
                    .by_neighborhood
                    .get(&response.neighborhood())
                    .is_some_and(|ids| ids.contains(&id))
                || self
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
        for (authority, ids) in &self.indexes.police_responses.by_authority {
            if ids.iter().any(|id| {
                !self
                    .police_responses
                    .get(id)
                    .is_some_and(|record| record.authority() == *authority)
            }) {
                return false;
            }
        }
        for (neighborhood, ids) in &self.indexes.police_responses.by_neighborhood {
            if ids.iter().any(|id| {
                !self
                    .police_responses
                    .get(id)
                    .is_some_and(|record| record.neighborhood() == *neighborhood)
            }) {
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
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        if !self.has_consistent_arrest_indexes()
            || !self.has_consistent_legal_representation_indexes()
            || !self.has_consistent_prosecution_indexes()
            || !self.has_consistent_police_response_indexes()
        {
            return false;
        }
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
            if investigation
                .lead_investigator()
                .is_some_and(|lead| !investigation.assigned_investigators().contains(&lead))
            {
                return false;
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
            for investigator in investigation.assigned_investigators() {
                if !self
                    .indexes
                    .investigations
                    .investigations_by_investigator
                    .get(investigator)
                    .is_some_and(|ids| ids.contains(&investigation.id()))
                {
                    return false;
                }
            }
        }
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
        for informant in self.informants.values() {
            let id = informant.id();
            if !self
                .indexes
                .informants
                .by_character
                .get(&informant.character())
                .is_some_and(|ids| ids.contains(&id))
                || !self
                    .indexes
                    .informants
                    .by_handler
                    .get(&informant.handler())
                    .is_some_and(|ids| ids.contains(&id))
            {
                return false;
            }
            let active_index = self
                .indexes
                .informants
                .active_by_character_handler
                .get(&(informant.character(), informant.handler()));
            match informant.status() {
                InformantStatus::Active if active_index != Some(&id) => return false,
                InformantStatus::Terminated if active_index == Some(&id) => return false,
                InformantStatus::Active | InformantStatus::Terminated => {}
            }
        }
        for (key, id) in &self.indexes.informants.active_by_character_handler {
            if !self.informants.get(id).is_some_and(|record| {
                record.status() == InformantStatus::Active
                    && (record.character(), record.handler()) == *key
            }) {
                return false;
            }
        }
        for (character, ids) in &self.indexes.informants.by_character {
            for id in ids {
                if !self
                    .informants
                    .get(id)
                    .is_some_and(|record| record.character() == *character)
                {
                    return false;
                }
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
                || !self
                    .indexes
                    .informants
                    .disclosures_by_informant
                    .get(&disclosure.informant())
                    .is_some_and(|ids| ids.contains(&disclosure.id()))
                || self
                    .indexes
                    .informants
                    .disclosure_by_evidence
                    .get(&disclosure.evidence())
                    != Some(&disclosure.id())
                || !self
                    .indexes
                    .informants
                    .disclosures_by_information
                    .get(&disclosure.source_information())
                    .is_some_and(|ids| ids.contains(&disclosure.id()))
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
        for (informant, ids) in &self.indexes.informants.disclosures_by_informant {
            for id in ids {
                if !self
                    .informant_disclosures
                    .get(id)
                    .is_some_and(|record| record.informant() == *informant)
                {
                    return false;
                }
            }
        }
        for (evidence, disclosure) in &self.indexes.informants.disclosure_by_evidence {
            if !self
                .informant_disclosures
                .get(disclosure)
                .is_some_and(|record| record.evidence() == *evidence)
            {
                return false;
            }
        }
        for (information, ids) in &self.indexes.informants.disclosures_by_information {
            for id in ids {
                if !self
                    .informant_disclosures
                    .get(id)
                    .is_some_and(|record| record.source_information() == *information)
                {
                    return false;
                }
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
        for (source, ids) in &self.indexes.evidence.evidence_by_source {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.source() == Some(*source))
                {
                    return false;
                }
            }
        }
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
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id()))
                || !self
                    .indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
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
        for evidence in self.evidence.values() {
            if !self
                .investigations
                .get(&evidence.investigation())
                .is_some_and(|investigation| investigation.evidence().contains(&evidence.id()))
            {
                return false;
            }
            if !self
                .indexes
                .evidence
                .evidence_by_subject
                .get(&evidence.subject())
                .is_some_and(|ids| ids.contains(&evidence.id()))
            {
                return false;
            }
            if let Some(origin) = evidence.origin() {
                if !self
                    .indexes
                    .evidence
                    .evidence_by_origin
                    .get(&origin)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
            if let Some(source) = evidence.source() {
                if !self
                    .indexes
                    .evidence
                    .evidence_by_source
                    .get(&source)
                    .is_some_and(|ids| ids.contains(&evidence.id()))
                {
                    return false;
                }
            }
            if !self
                .indexes
                .evidence
                .evidence_by_kind
                .get(&evidence.kind())
                .is_some_and(|ids| ids.contains(&evidence.id()))
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
        for (subject, ids) in &self.indexes.evidence.evidence_by_subject {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.subject() == *subject)
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
                    if work.resolution().is_some() || !due_indexed || !focus_indexed {
                        return false;
                    }
                }
                InvestigationWorkStatus::Completed => {
                    if work.resolution().is_none() || due_indexed || focus_indexed {
                        return false;
                    }
                }
            }
        }
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
        for (time, ids) in &self.indexes.work.scheduled_work_by_due_at {
            for id in ids {
                if !self.investigation_work.get(id).is_some_and(|work| {
                    work.status() == InvestigationWorkStatus::Scheduled && work.due_at() == *time
                }) {
                    return false;
                }
            }
        }
        for (key, id) in &self.indexes.work.scheduled_work_by_focus {
            if !self.investigation_work.get(id).is_some_and(|work| {
                work.status() == InvestigationWorkStatus::Scheduled
                    && (work.investigation(), work.kind(), work.focus()) == *key
            }) {
                return false;
            }
        }
        for (kind, ids) in &self.indexes.evidence.evidence_by_kind {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.kind() == *kind)
                {
                    return false;
                }
            }
        }
        for (origin, ids) in &self.indexes.evidence.evidence_by_origin {
            for id in ids {
                if !self
                    .evidence
                    .get(id)
                    .is_some_and(|record| record.origin() == Some(*origin))
                {
                    return false;
                }
            }
        }
        for (investigator, ids) in &self.indexes.investigations.investigations_by_investigator {
            for id in ids {
                if !self
                    .investigations
                    .get(id)
                    .is_some_and(|record| record.assigned_investigators().contains(investigator))
                {
                    return false;
                }
            }
        }
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
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
                    .is_some_and(|record| record.neighborhoods().contains(neighborhood))
                {
                    return false;
                }
            }
        }
        for deployment in self.patrol_deployments.values() {
            let id = deployment.id();
            if !self
                .indexes
                .patrols
                .by_organization
                .get(&deployment.organization())
                .is_some_and(|ids| ids.contains(&id))
                || !self
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
            match deployment.status() {
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
        for (organization, ids) in &self.indexes.patrols.by_organization {
            for id in ids {
                if !self
                    .patrol_deployments
                    .get(id)
                    .is_some_and(|record| record.organization() == *organization)
                {
                    return false;
                }
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
                record.status() == PatrolDeploymentStatus::Active
                    && (record.organization(), record.neighborhood()) == *key
            }) {
                return false;
            }
        }
        for (neighborhood, ids) in &self.indexes.patrols.active_by_neighborhood {
            for id in ids {
                if !self.patrol_deployments.get(id).is_some_and(|record| {
                    record.status() == PatrolDeploymentStatus::Active
                        && record.neighborhood() == *neighborhood
                }) {
                    return false;
                }
            }
        }
        true
    }
    pub(crate) fn debug_validate_indexes(&self) {
        debug_assert!(
            self.has_consistent_indexes(),
            "Derived Data Consistency: legal indexes disagree with source records"
        );
        for investigation in self.investigations.values() {
            debug_assert!(
                self.indexes
                    .investigations
                    .by_owner
                    .get(&investigation.owner())
                    .is_some_and(|ids| ids.contains(&investigation.id())),
                "Index Completeness: investigation owner index is missing a case"
            );
            for subject in investigation.subjects() {
                debug_assert!(
                    self.indexes
                        .investigations
                        .investigations_by_subject
                        .get(subject)
                        .is_some_and(|ids| ids.contains(&investigation.id())),
                    "Index Completeness: investigation subject index is missing a case"
                );
            }
            for evidence in investigation.evidence() {
                let record = self
                    .evidence
                    .get(evidence)
                    .expect("Record Reference Validity: investigation references missing evidence");
                debug_assert_eq!(
                    record.investigation(),
                    investigation.id(),
                    "Ownership Exclusivity: evidence belongs to a different investigation"
                );
            }
            if let Some(lead) = investigation.lead_investigator() {
                debug_assert!(
                    investigation.assigned_investigators().contains(&lead),
                    "Derived Data Consistency: investigation lead is not assigned to the case"
                );
            }
            debug_assert_eq!(
                self.indexes
                    .investigations
                    .active_without_lead
                    .contains(&investigation.id()),
                investigation.status() == InvestigationStatus::Active
                    && investigation.lead_investigator().is_none(),
                "Derived Data Consistency: unstaffed active investigation index disagrees with case"
            );
            for investigator in investigation.assigned_investigators() {
                debug_assert!(
                    self.indexes
                        .investigations
                        .investigations_by_investigator
                        .get(investigator)
                        .is_some_and(|ids| ids.contains(&investigation.id())),
                    "Index Completeness: investigator reverse index is missing an assigned case"
                );
            }
            let indexed_activity = self
                .indexes
                .investigations
                .cases_by_last_activity
                .get(&investigation.last_activity_at())
                .is_some_and(|ids| ids.contains(&investigation.id()));
            debug_assert_eq!(
                indexed_activity,
                investigation.status() == InvestigationStatus::Active,
                "Derived Data Consistency: cold-decay activity index disagrees with case status"
            );
        }
        for arrest in self.arrests.values() {
            debug_assert!(
                self.indexes
                    .arrests
                    .by_character
                    .get(&arrest.character())
                    .is_some_and(|ids| ids.contains(&arrest.id())),
                "Index Completeness: character arrest index is missing an arrest"
            );
            debug_assert!(
                self.indexes
                    .arrests
                    .by_investigation
                    .get(&arrest.investigation())
                    .is_some_and(|ids| ids.contains(&arrest.id())),
                "Index Completeness: investigation arrest index is missing an arrest"
            );
            debug_assert!(
                self.indexes
                    .arrests
                    .by_authority
                    .get(&arrest.authority())
                    .is_some_and(|ids| ids.contains(&arrest.id())),
                "Index Completeness: authority arrest index is missing an arrest"
            );
            let active = self
                .indexes
                .arrests
                .active_by_character
                .get(&arrest.character());
            match arrest.status() {
                ArrestStatus::Detained => debug_assert_eq!(active, Some(&arrest.id())),
                ArrestStatus::Released => debug_assert_ne!(active, Some(&arrest.id())),
            }
        }
        for evidence in self.evidence.values() {
            debug_assert!(
                self.indexes
                    .evidence
                    .evidence_by_subject
                    .get(&evidence.subject())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence subject index is missing evidence"
            );
            if let Some(origin) = evidence.origin() {
                debug_assert!(
                    self.indexes
                        .evidence
                        .evidence_by_origin
                        .get(&origin)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence origin index is missing evidence"
                );
            }
            if let Some(source) = evidence.source() {
                debug_assert!(
                    self.indexes
                        .evidence
                        .evidence_by_source
                        .get(&source)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence source index is missing evidence"
                );
            }
            debug_assert!(
                self.indexes
                    .evidence
                    .evidence_by_kind
                    .get(&evidence.kind())
                    .is_some_and(|ids| ids.contains(&evidence.id())),
                "Index Completeness: evidence kind index is missing evidence"
            );
            for source in evidence.derived_from() {
                debug_assert!(
                    self.indexes.evidence.derived_evidence_by_source
                        .get(source)
                        .is_some_and(|ids| ids.contains(&evidence.id())),
                    "Index Completeness: evidence provenance reverse index is missing derived evidence"
                );
            }
        }
        for work in self.investigation_work.values() {
            debug_assert!(
                self.indexes
                    .work
                    .work_by_investigation
                    .get(&work.investigation())
                    .is_some_and(|ids| ids.contains(&work.id())),
                "Index Completeness: investigation work case index is missing work"
            );
            debug_assert!(
                self.indexes
                    .work
                    .work_by_investigator
                    .get(&work.investigator())
                    .is_some_and(|ids| ids.contains(&work.id())),
                "Index Completeness: investigation work investigator index is missing work"
            );
            match work.status() {
                InvestigationWorkStatus::Scheduled => {
                    debug_assert!(work.resolution().is_none());
                    debug_assert!(
                        self.indexes.work.scheduled_work_by_due_at
                            .get(&work.due_at())
                            .is_some_and(|ids| ids.contains(&work.id())),
                        "Index Completeness: scheduled investigation work due index is missing work"
                    );
                    debug_assert_eq!(
                        self.indexes.work.scheduled_work_by_focus.get(&(
                            work.investigation(),
                            work.kind(),
                            work.focus(),
                        )),
                        Some(&work.id()),
                        "Index Completeness: scheduled investigation work focus index is missing work"
                    );
                }
                InvestigationWorkStatus::Completed => {
                    debug_assert!(work.resolution().is_some());
                }
            }
        }
        for witness in self.case_witnesses.values() {
            debug_assert_eq!(
                self.indexes
                    .witnesses
                    .case_witness_by_case_character
                    .get(&(witness.investigation(), witness.witness())),
                Some(&witness.id()),
                "Index Completeness: case-witness uniqueness index is missing witness"
            );
            debug_assert!(
                self.indexes
                    .witnesses
                    .case_witnesses_by_character
                    .get(&witness.witness())
                    .is_some_and(|ids| ids.contains(&witness.id())),
                "Index Completeness: character witness index is missing case witness"
            );
            debug_assert!(
                self.indexes
                    .witnesses
                    .case_witnesses_by_investigation
                    .get(&witness.investigation())
                    .is_some_and(|ids| ids.contains(&witness.id())),
                "Index Completeness: investigation witness index is missing case witness"
            );
            for statement in witness.statements() {
                debug_assert!(
                    self.witness_statements
                        .get(statement)
                        .is_some_and(|record| record.case_witness() == witness.id()),
                    "Record Reference Validity: case witness references missing or foreign statement"
                );
            }
        }
        for statement in self.witness_statements.values() {
            debug_assert!(
                self.case_witnesses
                    .get(&statement.case_witness())
                    .is_some_and(|witness| witness.statements().contains(&statement.id())),
                "Record Reference Validity: witness statement is not owned by its case witness"
            );
            debug_assert_eq!(
                self.indexes
                    .witnesses
                    .witness_statement_by_evidence
                    .get(&statement.evidence()),
                Some(&statement.id()),
                "Index Completeness: witness statement evidence index is missing statement"
            );
        }
        for informant in self.informants.values() {
            debug_assert!(
                self.indexes
                    .informants
                    .by_character
                    .get(&informant.character())
                    .is_some_and(|ids| ids.contains(&informant.id())),
                "Index Completeness: character informant index is missing a relationship"
            );
            debug_assert!(
                self.indexes
                    .informants
                    .by_handler
                    .get(&informant.handler())
                    .is_some_and(|ids| ids.contains(&informant.id())),
                "Index Completeness: handler informant index is missing a relationship"
            );
            let active = self
                .indexes
                .informants
                .active_by_character_handler
                .get(&(informant.character(), informant.handler()));
            match informant.status() {
                InformantStatus::Active => debug_assert_eq!(active, Some(&informant.id())),
                InformantStatus::Terminated => debug_assert_ne!(active, Some(&informant.id())),
            }
        }
        for disclosure in self.informant_disclosures.values() {
            debug_assert!(
                self.indexes
                    .informants
                    .disclosures_by_informant
                    .get(&disclosure.informant())
                    .is_some_and(|ids| ids.contains(&disclosure.id())),
                "Index Completeness: informant disclosure index is missing a disclosure"
            );
            debug_assert_eq!(
                self.indexes
                    .informants
                    .disclosure_by_evidence
                    .get(&disclosure.evidence()),
                Some(&disclosure.id()),
                "Index Completeness: informant evidence index is missing a disclosure"
            );
            debug_assert_eq!(
                self.indexes
                    .informants
                    .disclosure_by_case_information
                    .get(&(disclosure.investigation(), disclosure.source_information())),
                Some(&disclosure.id()),
                "Index Completeness: informant case-information index is missing a disclosure"
            );
        }
        for jurisdiction in self.jurisdictions.values() {
            for neighborhood in jurisdiction.neighborhoods() {
                debug_assert!(
                    self.indexes.jurisdictions.jurisdictions_by_neighborhood
                        .get(neighborhood)
                        .is_some_and(|organizations| {
                            organizations.contains(&jurisdiction.organization())
                        }),
                    "Index Completeness: legal jurisdiction neighborhood index is missing authority"
                );
            }
        }
        for deployment in self.patrol_deployments.values() {
            debug_assert!(
                self.indexes
                    .patrols
                    .by_organization
                    .get(&deployment.organization())
                    .is_some_and(|ids| ids.contains(&deployment.id())),
                "Index Completeness: patrol organization index is missing deployment"
            );
            debug_assert!(
                self.indexes
                    .patrols
                    .by_neighborhood
                    .get(&deployment.neighborhood())
                    .is_some_and(|ids| ids.contains(&deployment.id())),
                "Index Completeness: patrol neighborhood index is missing deployment"
            );
            let active = self
                .indexes
                .patrols
                .active_by_organization_neighborhood
                .get(&(deployment.organization(), deployment.neighborhood()));
            match deployment.status() {
                PatrolDeploymentStatus::Active => {
                    debug_assert_eq!(active, Some(&deployment.id()));
                    debug_assert!(self
                        .indexes
                        .patrols
                        .active_by_neighborhood
                        .get(&deployment.neighborhood())
                        .is_some_and(|ids| ids.contains(&deployment.id())));
                }
                PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired => {
                    debug_assert_ne!(active, Some(&deployment.id()));
                }
            }
        }
    }
}
