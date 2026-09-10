//! Read-only observation and index-backed query surface for `LegalState`.

use super::*;

impl LegalState {
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
    pub fn informant_for(
        &self,
        character: CharacterId,
        handler: OrganizationId,
    ) -> Option<&InformantRecord> {
        self.indexes
            .informants
            .by_character_handler
            .get(&(character, handler))
            .map(|id| {
                self.informants
                    .get(id)
                    .expect("informant pair index must reference an informant")
            })
    }
    pub(crate) fn patrol_deployments_for_neighborhood(
        &self,
        neighborhood: NeighborhoodId,
    ) -> impl Iterator<Item = &PatrolDeploymentRecord> {
        self.indexes
            .patrols
            .by_neighborhood
            .get(&neighborhood)
            .into_iter()
            .flatten()
            .map(|id| {
                self.patrol_deployments
                    .get(id)
                    .expect("patrol-neighborhood index must reference a deployment")
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
    /// O(1) emptiness probe over the authoritative informant map, so per-tick passes that build
    /// cross-referenced views (handler-to-case maps) can skip that work entirely on quiet
    /// ticks without changing what they would have produced.
    pub(crate) fn has_informants(&self) -> bool {
        !self.informants.is_empty()
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
}
