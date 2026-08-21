//! Explicit police-to-prosecutor evidence referral and prosecution-case intake.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId, OrganizationId,
    ProsecutionCaseId, ProsecutionReferralId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::{
    ProsecutionCaseDraft, ProsecutionCaseRecord, ProsecutionCaseResolution, ProsecutionCaseStatus,
    ProsecutionReferralDraft, ProsecutionReferralRecord,
};
use crate::reports::report_system::{validate_record_report, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, Lifecycle, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProsecutionError {
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("source investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("source authority {0} does not exist")]
    MissingSourceAuthority(OrganizationId),
    #[error("source authority {0} is not an active law-enforcement organization")]
    InvalidSourceAuthority(OrganizationId),
    #[error("prosecutor office {0} does not exist")]
    MissingProsecutorOffice(OrganizationId),
    #[error("organization {0} is not an active prosecutor office")]
    InvalidProsecutorOffice(OrganizationId),
    #[error("lead prosecutor {0} does not exist")]
    MissingLeadProsecutor(CharacterId),
    #[error("lead prosecutor {prosecutor} is not an active member of office {office}")]
    InvalidLeadProsecutor {
        prosecutor: CharacterId,
        office: OrganizationId,
    },
    #[error("lead prosecutor {0} is detained")]
    DetainedLeadProsecutor(CharacterId),
    #[error("lead prosecutor {0} has no LegalKnowledge capability")]
    MissingLegalKnowledge(CharacterId),
    #[error("defendant {0} does not exist")]
    MissingDefendant(CharacterId),
    #[error("defendant {0} is not active")]
    InactiveDefendant(CharacterId),
    #[error("arrest {arrest} already has open prosecution case {case} in office {office}")]
    DuplicateOpenCase {
        arrest: ArrestId,
        office: OrganizationId,
        case: ProsecutionCaseId,
    },
    #[error("prosecution referral must contain at least one evidence record")]
    NoEvidence,
    #[error("initial prosecution referral must include arrest evidence {0}")]
    MissingArrestEvidence(EvidenceId),
    #[error("evidence {0} does not exist")]
    MissingEvidence(EvidenceId),
    #[error("evidence {evidence} does not belong to source investigation {investigation}")]
    EvidenceInvestigationMismatch {
        evidence: EvidenceId,
        investigation: InvestigationId,
    },
    #[error("evidence {evidence} is not held by source authority {authority}")]
    EvidenceCustodianMismatch {
        evidence: EvidenceId,
        authority: OrganizationId,
    },
    #[error("prosecution case {0} does not exist")]
    MissingProsecutionCase(ProsecutionCaseId),
    #[error("prosecution case {case} is not open for prosecutorial action")]
    CaseNotOpen { case: ProsecutionCaseId },
    #[error("evidence {evidence} is already available to prosecution case {case}")]
    EvidenceAlreadyReferred {
        case: ProsecutionCaseId,
        evidence: EvidenceId,
    },
    #[error(
        "prosecution referral was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("arrest {arrest} changed after referral validation; expected version {expected}, found {found}")]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error("source investigation {investigation} changed after referral validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("lead prosecutor {prosecutor} changed after referral validation; expected version {expected}, found {found}")]
    StaleLeadProsecutor {
        prosecutor: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("prosecution case {case} changed after referral validation; expected version {expected}, found {found}")]
    StaleProsecutionCase {
        case: ProsecutionCaseId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Clone, Copy, Debug)]
struct ReferralDependencies {
    defendant: CharacterId,
    source_investigation: InvestigationId,
    source_authority: OrganizationId,
    prosecutor_office: OrganizationId,
    lead_prosecutor: CharacterId,
    arrest_version: u32,
    investigation_version: u32,
    lead_version: u32,
}

pub struct ValidatedProsecutionCaseOpening {
    draft: ProsecutionCaseDraft,
    dependencies: ReferralDependencies,
    referred_at: SimTime,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedProsecutionCaseOpening {
    pub fn commit(self, state: &mut AppState) -> Result<ProsecutionCaseId, ProsecutionError> {
        state.ids.reserve_many(&[
            (IdKind::Information, 1),
            (IdKind::Report, 1),
            (IdKind::ProsecutionCase, 1),
            (IdKind::ProsecutionReferral, 1),
        ])?;
        validate_time(state, self.referred_at)?;
        validate_opening_versions(state, self.draft.arrest, self.dependencies)?;
        let current = validate_opening_dependencies(state, &self.draft)?;
        debug_assert_eq!(current.defendant, self.dependencies.defendant);
        debug_assert_eq!(
            current.source_investigation,
            self.dependencies.source_investigation
        );
        debug_assert_eq!(current.source_authority, self.dependencies.source_authority);
        debug_assert_eq!(
            current.prosecutor_office,
            self.dependencies.prosecutor_office
        );

        let information = self.information.commit(state)?;
        let report = self.report.commit(state)?;
        let case_id = state.ids.next_prosecution_case()?;
        let referral_id = state.ids.next_prosecution_referral()?;
        state.legal.insert_prosecution_case(
            ProsecutionCaseRecord {
                id: case_id,
                context: super::ProsecutionCaseContext {
                    arrest: self.draft.arrest,
                    defendant: self.dependencies.defendant,
                    source_investigation: self.dependencies.source_investigation,
                    source_authority: self.dependencies.source_authority,
                    prosecutor_office: self.draft.prosecutor_office,
                    lead_prosecutor: self.draft.lead_prosecutor,
                },
                referrals: super::ProsecutionCaseReferrals {
                    evidence: self.draft.evidence.clone(),
                    initial_referral: referral_id,
                    referrals: BTreeSet::from([referral_id]),
                },
                lifecycle: super::ProsecutionCaseLifecycle {
                    opened_at: self.referred_at,
                    resolved_at: None,
                    status: ProsecutionCaseStatus::Reviewing,
                },
                resolution_artifacts: super::ProsecutionCaseResolutionArtifacts {
                    resolution_information: None,
                    resolution_report: None,
                },
                version: 1,
            },
            ProsecutionReferralRecord {
                id: referral_id,
                prosecution_case: case_id,
                source_investigation: self.dependencies.source_investigation,
                source_authority: self.dependencies.source_authority,
                prosecutor_office: self.draft.prosecutor_office,
                evidence: self.draft.evidence,
                referred_at: self.referred_at,
                information,
                report,
            },
        );
        Ok(case_id)
    }
}

pub fn validate_open_prosecution_case(
    state: &AppState,
    draft: ProsecutionCaseDraft,
) -> Result<ValidatedProsecutionCaseOpening, ProsecutionError> {
    let dependencies = validate_opening_dependencies(state, &draft)?;
    let referred_at = state.now();
    let (information, report) = validate_referral_artifacts(
        state,
        ReferralArtifactContext {
            defendant: dependencies.defendant,
            source_investigation: dependencies.source_investigation,
            source_authority: dependencies.source_authority,
            prosecutor_office: draft.prosecutor_office,
            lead_prosecutor: draft.lead_prosecutor,
            evidence: &draft.evidence,
            referred_at,
            initial: true,
        },
    )?;
    Ok(ValidatedProsecutionCaseOpening {
        draft,
        dependencies,
        referred_at,
        information,
        report,
    })
}

fn validate_opening_dependencies(
    state: &AppState,
    draft: &ProsecutionCaseDraft,
) -> Result<ReferralDependencies, ProsecutionError> {
    let arrest = state
        .legal
        .get_arrest(draft.arrest)
        .ok_or(ProsecutionError::MissingArrest(draft.arrest))?;
    // The defendant is the subject of every artifact this case produces; opening a case against
    // an inactive person would assert the office reviewed someone who no longer exists.
    let defendant = state
        .world
        .get_character(arrest.character())
        .ok_or(ProsecutionError::MissingDefendant(arrest.character()))?;
    if defendant.lifecycle() != Lifecycle::Active {
        return Err(ProsecutionError::InactiveDefendant(arrest.character()));
    }
    if let Some(existing) = state
        .legal
        .open_prosecution_case_for(draft.arrest, draft.prosecutor_office)
    {
        return Err(ProsecutionError::DuplicateOpenCase {
            arrest: draft.arrest,
            office: draft.prosecutor_office,
            case: existing.id(),
        });
    }
    validate_evidence_set(
        state,
        arrest.investigation(),
        arrest.authority(),
        &draft.evidence,
    )?;
    for evidence in arrest.evidence() {
        if !draft.evidence.contains(evidence) {
            return Err(ProsecutionError::MissingArrestEvidence(*evidence));
        }
    }
    let investigation = state
        .legal
        .get_investigation(arrest.investigation())
        .ok_or(ProsecutionError::MissingInvestigation(
            arrest.investigation(),
        ))?;
    let source_authority = state
        .world
        .get_organization(arrest.authority())
        .ok_or(ProsecutionError::MissingSourceAuthority(arrest.authority()))?;
    if source_authority.lifecycle() != Lifecycle::Active
        || source_authority.kind() != OrganizationKind::LawEnforcement
        || investigation.owner() != arrest.authority()
    {
        return Err(ProsecutionError::InvalidSourceAuthority(arrest.authority()));
    }
    let office = state
        .world
        .get_organization(draft.prosecutor_office)
        .ok_or(ProsecutionError::MissingProsecutorOffice(
            draft.prosecutor_office,
        ))?;
    if office.lifecycle() != Lifecycle::Active || office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            draft.prosecutor_office,
        ));
    }
    let lead = validate_lead_prosecutor(state, draft.prosecutor_office, draft.lead_prosecutor)?;
    Ok(ReferralDependencies {
        defendant: arrest.character(),
        source_investigation: arrest.investigation(),
        source_authority: arrest.authority(),
        prosecutor_office: draft.prosecutor_office,
        lead_prosecutor: draft.lead_prosecutor,
        arrest_version: arrest.version(),
        investigation_version: investigation.version(),
        lead_version: lead.version(),
    })
}

pub struct ValidatedProsecutionReferral {
    draft: ProsecutionReferralDraft,
    expected_case_version: u32,
    expected_investigation_version: u32,
    expected_lead_version: u32,
    referred_at: SimTime,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedProsecutionReferral {
    pub fn commit(self, state: &mut AppState) -> Result<ProsecutionReferralId, ProsecutionError> {
        state.ids.reserve_many(&[
            (IdKind::Information, 1),
            (IdKind::Report, 1),
            (IdKind::ProsecutionReferral, 1),
        ])?;
        validate_time(state, self.referred_at)?;
        let case = state
            .legal
            .get_prosecution_case(self.draft.prosecution_case)
            .ok_or(ProsecutionError::MissingProsecutionCase(
                self.draft.prosecution_case,
            ))?;
        if case.version() != self.expected_case_version {
            return Err(ProsecutionError::StaleProsecutionCase {
                case: case.id(),
                expected: self.expected_case_version,
                found: case.version(),
            });
        }
        let investigation = state
            .legal
            .get_investigation(case.source_investigation())
            .ok_or(ProsecutionError::MissingInvestigation(
                case.source_investigation(),
            ))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(ProsecutionError::StaleInvestigation {
                investigation: investigation.id(),
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        let lead = state.world.get_character(case.lead_prosecutor()).ok_or(
            ProsecutionError::MissingLeadProsecutor(case.lead_prosecutor()),
        )?;
        if lead.version() != self.expected_lead_version {
            return Err(ProsecutionError::StaleLeadProsecutor {
                prosecutor: lead.id(),
                expected: self.expected_lead_version,
                found: lead.version(),
            });
        }
        validate_supplement_dependencies(state, &self.draft)?;
        let source_investigation = case.source_investigation();
        let source_authority = case.source_authority();
        let prosecutor_office = case.prosecutor_office();
        let information = self.information.commit(state)?;
        let report = self.report.commit(state)?;
        let referral_id = state.ids.next_prosecution_referral()?;
        state
            .legal
            .add_prosecution_referral(ProsecutionReferralRecord {
                id: referral_id,
                prosecution_case: self.draft.prosecution_case,
                source_investigation,
                source_authority,
                prosecutor_office,
                evidence: self.draft.evidence,
                referred_at: self.referred_at,
                information,
                report,
            });
        Ok(referral_id)
    }
}

pub fn validate_supplement_prosecution_case(
    state: &AppState,
    draft: ProsecutionReferralDraft,
) -> Result<ValidatedProsecutionReferral, ProsecutionError> {
    let case = validate_supplement_dependencies(state, &draft)?;
    let investigation = state
        .legal
        .get_investigation(case.source_investigation())
        .expect("validated source investigation must exist");
    let lead = state
        .world
        .get_character(case.lead_prosecutor())
        .expect("validated lead prosecutor must exist");
    let referred_at = state.now();
    let (information, report) = validate_referral_artifacts(
        state,
        ReferralArtifactContext {
            defendant: case.defendant(),
            source_investigation: case.source_investigation(),
            source_authority: case.source_authority(),
            prosecutor_office: case.prosecutor_office(),
            lead_prosecutor: case.lead_prosecutor(),
            evidence: &draft.evidence,
            referred_at,
            initial: false,
        },
    )?;
    Ok(ValidatedProsecutionReferral {
        draft,
        expected_case_version: case.version(),
        expected_investigation_version: investigation.version(),
        expected_lead_version: lead.version(),
        referred_at,
        information,
        report,
    })
}

fn validate_supplement_dependencies<'a>(
    state: &'a AppState,
    draft: &ProsecutionReferralDraft,
) -> Result<&'a ProsecutionCaseRecord, ProsecutionError> {
    let case = state
        .legal
        .get_prosecution_case(draft.prosecution_case)
        .ok_or(ProsecutionError::MissingProsecutionCase(
            draft.prosecution_case,
        ))?;
    if !matches!(case.status(), ProsecutionCaseStatus::Reviewing) {
        return Err(ProsecutionError::CaseNotOpen { case: case.id() });
    }
    validate_source_case_and_office(state, case)?;
    validate_evidence_set(
        state,
        case.source_investigation(),
        case.source_authority(),
        &draft.evidence,
    )?;
    for evidence in &draft.evidence {
        if case.evidence().contains(evidence) {
            return Err(ProsecutionError::EvidenceAlreadyReferred {
                case: case.id(),
                evidence: *evidence,
            });
        }
    }
    Ok(case)
}

fn validate_source_case_and_office(
    state: &AppState,
    case: &ProsecutionCaseRecord,
) -> Result<(), ProsecutionError> {
    let source = state
        .world
        .get_organization(case.source_authority())
        .ok_or(ProsecutionError::MissingSourceAuthority(
            case.source_authority(),
        ))?;
    if source.lifecycle() != Lifecycle::Active || source.kind() != OrganizationKind::LawEnforcement
    {
        return Err(ProsecutionError::InvalidSourceAuthority(
            case.source_authority(),
        ));
    }
    let investigation = state
        .legal
        .get_investigation(case.source_investigation())
        .ok_or(ProsecutionError::MissingInvestigation(
            case.source_investigation(),
        ))?;
    if investigation.owner() != case.source_authority() {
        return Err(ProsecutionError::InvalidSourceAuthority(
            case.source_authority(),
        ));
    }
    let office = state
        .world
        .get_organization(case.prosecutor_office())
        .ok_or(ProsecutionError::MissingProsecutorOffice(
            case.prosecutor_office(),
        ))?;
    if office.lifecycle() != Lifecycle::Active || office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            case.prosecutor_office(),
        ));
    }
    validate_lead_prosecutor(state, case.prosecutor_office(), case.lead_prosecutor())?;
    Ok(())
}

fn validate_lead_prosecutor(
    state: &AppState,
    office: OrganizationId,
    prosecutor: CharacterId,
) -> Result<&crate::world::CharacterRecord, ProsecutionError> {
    let lead = state
        .world
        .get_character(prosecutor)
        .ok_or(ProsecutionError::MissingLeadProsecutor(prosecutor))?;
    if lead.lifecycle() != Lifecycle::Active || lead.organization() != Some(office) {
        return Err(ProsecutionError::InvalidLeadProsecutor { prosecutor, office });
    }
    if state
        .legal
        .active_arrest_for_character(prosecutor)
        .is_some()
    {
        return Err(ProsecutionError::DetainedLeadProsecutor(prosecutor));
    }
    if lead.capability(CapabilityKind::LegalKnowledge).is_none() {
        return Err(ProsecutionError::MissingLegalKnowledge(prosecutor));
    }
    Ok(lead)
}

fn validate_evidence_set(
    state: &AppState,
    investigation: InvestigationId,
    source_authority: OrganizationId,
    evidence_ids: &BTreeSet<EvidenceId>,
) -> Result<(), ProsecutionError> {
    if evidence_ids.is_empty() {
        return Err(ProsecutionError::NoEvidence);
    }
    for evidence_id in evidence_ids {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ProsecutionError::MissingEvidence(*evidence_id))?;
        if evidence.investigation() != investigation {
            return Err(ProsecutionError::EvidenceInvestigationMismatch {
                evidence: *evidence_id,
                investigation,
            });
        }
        if evidence.custodian() != source_authority {
            return Err(ProsecutionError::EvidenceCustodianMismatch {
                evidence: *evidence_id,
                authority: source_authority,
            });
        }
    }
    Ok(())
}

fn validate_opening_versions(
    state: &AppState,
    arrest: ArrestId,
    expected: ReferralDependencies,
) -> Result<(), ProsecutionError> {
    let arrest_record = state
        .legal
        .get_arrest(arrest)
        .ok_or(ProsecutionError::MissingArrest(arrest))?;
    if arrest_record.version() != expected.arrest_version {
        return Err(ProsecutionError::StaleArrest {
            arrest,
            expected: expected.arrest_version,
            found: arrest_record.version(),
        });
    }
    let investigation = state
        .legal
        .get_investigation(expected.source_investigation)
        .ok_or(ProsecutionError::MissingInvestigation(
            expected.source_investigation,
        ))?;
    if investigation.version() != expected.investigation_version {
        return Err(ProsecutionError::StaleInvestigation {
            investigation: expected.source_investigation,
            expected: expected.investigation_version,
            found: investigation.version(),
        });
    }
    let lead = state.world.get_character(expected.lead_prosecutor).ok_or(
        ProsecutionError::MissingLeadProsecutor(expected.lead_prosecutor),
    )?;
    if lead.version() != expected.lead_version {
        return Err(ProsecutionError::StaleLeadProsecutor {
            prosecutor: expected.lead_prosecutor,
            expected: expected.lead_version,
            found: lead.version(),
        });
    }
    Ok(())
}

fn validate_time(state: &AppState, expected: SimTime) -> Result<(), ProsecutionError> {
    if state.now() != expected {
        return Err(ProsecutionError::StaleTime {
            expected,
            found: state.now(),
        });
    }
    Ok(())
}

pub struct ValidatedProsecutionCaseResolution {
    case: ProsecutionCaseId,
    resolution: ProsecutionCaseResolution,
    expected_case_version: u32,
    expected_lead_version: u32,
    resolved_at: SimTime,
    information: ValidatedInformation,
    report: ValidatedReport,
}

impl ValidatedProsecutionCaseResolution {
    pub fn commit(self, state: &mut AppState) -> Result<(), ProsecutionError> {
        state
            .ids
            .reserve_many(&[(IdKind::Information, 1), (IdKind::Report, 1)])?;
        validate_time(state, self.resolved_at)?;
        let case = state
            .legal
            .get_prosecution_case(self.case)
            .ok_or(ProsecutionError::MissingProsecutionCase(self.case))?;
        if case.version() != self.expected_case_version {
            return Err(ProsecutionError::StaleProsecutionCase {
                case: self.case,
                expected: self.expected_case_version,
                found: case.version(),
            });
        }
        let lead = state.world.get_character(case.lead_prosecutor()).ok_or(
            ProsecutionError::MissingLeadProsecutor(case.lead_prosecutor()),
        )?;
        if lead.version() != self.expected_lead_version {
            return Err(ProsecutionError::StaleLeadProsecutor {
                prosecutor: lead.id(),
                expected: self.expected_lead_version,
                found: lead.version(),
            });
        }
        validate_resolution_dependencies(state, self.case)?;

        let information = self.information.commit(state)?;
        let report = self.report.commit(state)?;
        state.legal.resolve_prosecution_case(
            self.case,
            self.resolution,
            self.resolved_at,
            information,
            report,
        );
        Ok(())
    }
}

pub fn validate_decline_prosecution_case(
    state: &AppState,
    case: ProsecutionCaseId,
) -> Result<ValidatedProsecutionCaseResolution, ProsecutionError> {
    validate_prosecution_case_resolution(state, case, ProsecutionCaseResolution::Declined)
}

pub fn validate_close_prosecution_case(
    state: &AppState,
    case: ProsecutionCaseId,
) -> Result<ValidatedProsecutionCaseResolution, ProsecutionError> {
    validate_prosecution_case_resolution(state, case, ProsecutionCaseResolution::Closed)
}

fn validate_prosecution_case_resolution(
    state: &AppState,
    case_id: ProsecutionCaseId,
    resolution: ProsecutionCaseResolution,
) -> Result<ValidatedProsecutionCaseResolution, ProsecutionError> {
    let case = validate_resolution_dependencies(state, case_id)?;
    let lead = state
        .world
        .get_character(case.lead_prosecutor())
        .expect("validated lead prosecutor must exist");
    let resolved_at = state.now();
    let (information, report) =
        validate_resolution_artifacts(state, case, resolution, resolved_at)?;
    Ok(ValidatedProsecutionCaseResolution {
        case: case_id,
        resolution,
        expected_case_version: case.version(),
        expected_lead_version: lead.version(),
        resolved_at,
        information,
        report,
    })
}

fn validate_resolution_dependencies(
    state: &AppState,
    case_id: ProsecutionCaseId,
) -> Result<&ProsecutionCaseRecord, ProsecutionError> {
    let case = state
        .legal
        .get_prosecution_case(case_id)
        .ok_or(ProsecutionError::MissingProsecutionCase(case_id))?;
    if case.status() != ProsecutionCaseStatus::Reviewing {
        return Err(ProsecutionError::CaseNotOpen { case: case_id });
    }
    let office = state
        .world
        .get_organization(case.prosecutor_office())
        .ok_or(ProsecutionError::MissingProsecutorOffice(
            case.prosecutor_office(),
        ))?;
    if office.lifecycle() != Lifecycle::Active || office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            case.prosecutor_office(),
        ));
    }
    validate_lead_prosecutor(state, case.prosecutor_office(), case.lead_prosecutor())?;
    // Resolving the case emits defendant-named artifacts; an inactive defendant cannot be
    // meaningfully reviewed, so resolution must not proceed against one.
    let defendant = state
        .world
        .get_character(case.defendant())
        .ok_or(ProsecutionError::MissingDefendant(case.defendant()))?;
    if defendant.lifecycle() != Lifecycle::Active {
        return Err(ProsecutionError::InactiveDefendant(case.defendant()));
    }
    Ok(case)
}

fn validate_resolution_artifacts(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    resolution: ProsecutionCaseResolution,
    resolved_at: SimTime,
) -> Result<(ValidatedInformation, ValidatedReport), ProsecutionError> {
    let defendant_name = state
        .world
        .get_character(case.defendant())
        .expect("validated prosecution defendant must exist")
        .name();
    let office_name = state
        .world
        .get_organization(case.prosecutor_office())
        .expect("validated prosecutor office must exist")
        .name();
    let lead_name = state
        .world
        .get_character(case.lead_prosecutor())
        .expect("validated lead prosecutor must exist")
        .name();
    let (title, summary) = match resolution {
        ProsecutionCaseResolution::Declined => (
            "Prosecution declined",
            format!(
                "{} declined prosecution of {} after review by {}.",
                office_name, defendant_name, lead_name
            ),
        ),
        ProsecutionCaseResolution::Closed => (
            "Prosecution review closed",
            format!(
                "{} closed its prosecution review of {} after review by {}.",
                office_name, defendant_name, lead_name
            ),
        ),
    };
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(case.prosecutor_office()),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(case.lead_prosecutor())),
            subject: EntityRef::Character(case.defendant()),
            observed_at: resolved_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: case.prosecutor_office(),
            kind: ReportKind::Legal,
            title: title.to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(case.defendant()),
                    EntityRef::Organization(case.source_authority()),
                    EntityRef::Organization(case.prosecutor_office()),
                    EntityRef::Character(case.lead_prosecutor()),
                    EntityRef::Investigation(case.source_investigation()),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok((information, report))
}

struct ReferralArtifactContext<'a> {
    defendant: CharacterId,
    source_investigation: InvestigationId,
    source_authority: OrganizationId,
    prosecutor_office: OrganizationId,
    lead_prosecutor: CharacterId,
    evidence: &'a BTreeSet<EvidenceId>,
    referred_at: SimTime,
    initial: bool,
}

fn validate_referral_artifacts(
    state: &AppState,
    context: ReferralArtifactContext<'_>,
) -> Result<(ValidatedInformation, ValidatedReport), ProsecutionError> {
    let ReferralArtifactContext {
        defendant,
        source_investigation,
        source_authority,
        prosecutor_office,
        lead_prosecutor,
        evidence,
        referred_at,
        initial,
    } = context;
    let defendant_name = state
        .world
        .get_character(defendant)
        .expect("validated defendant must exist")
        .name();
    let source_name = state
        .world
        .get_organization(source_authority)
        .expect("validated source authority must exist")
        .name();
    let office_name = state
        .world
        .get_organization(prosecutor_office)
        .expect("validated prosecutor office must exist")
        .name();
    let summary = if initial {
        format!(
            "{} referred the arrest matter for {} to {}, sharing {} evidence record(s).",
            source_name,
            defendant_name,
            office_name,
            evidence.len(),
        )
    } else {
        format!(
            "{} supplemented the prosecution matter for {} with {} additional evidence record(s).",
            source_name,
            defendant_name,
            evidence.len(),
        )
    };
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(prosecutor_office),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Organization(source_authority)),
            subject: EntityRef::Character(defendant),
            observed_at: referred_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: prosecutor_office,
            kind: ReportKind::Legal,
            title: if initial {
                "Prosecution case referral".to_owned()
            } else {
                "Prosecution evidence supplement".to_owned()
            },
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(defendant),
                    EntityRef::Organization(source_authority),
                    EntityRef::Organization(prosecutor_office),
                    EntityRef::Character(lead_prosecutor),
                    EntityRef::Investigation(source_investigation),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok((information, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
    use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
    use crate::legal::{
        Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability,
        EvidenceStrength, InvestigationDraft,
    };
    use crate::registry::Registry;
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, Rating};
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        registry: Registry,
        state: AppState,
        police: OrganizationId,
        office: OrganizationId,
        defendant: CharacterId,
        lead: CharacterId,
        investigation: InvestigationId,
        arrest: ArrestId,
        arrest_evidence: EvidenceId,
        supplemental_evidence: EvidenceId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn add_evidence(
        state: &mut AppState,
        police: OrganizationId,
        investigation: InvestigationId,
        defendant: CharacterId,
        kind: EvidenceKind,
    ) -> EvidenceId {
        validate_add_evidence(
            state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(defendant),
                origin: None,
                kind,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("fixture evidence should validate")
        .commit(state)
        .expect("fixture evidence should commit")
    }

    fn fixture() -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0xCA5E_1931);
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Canal Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Canal Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let office = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "District Prosecutor".to_owned(),
                kind: OrganizationKind::Prosecutor,
            },
        )
        .expect("prosecutor office should validate");
        let defendant = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Case Defendant".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("defendant fixture should validate");
        let lead = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Lead Prosecutor".to_owned(),
                organization: Some(office),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(86))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("lead prosecutor fixture should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Canal arrest case".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(defendant)]),
            },
        )
        .expect("source investigation should validate")
        .commit(&mut state)
        .expect("source investigation should commit");
        let arrest_evidence = add_evidence(
            &mut state,
            police,
            investigation,
            defendant,
            EvidenceKind::Document,
        );
        let arrest = validate_arrest(
            &state,
            ArrestDraft {
                character: defendant,
                investigation,
                evidence: BTreeSet::from([arrest_evidence]),
            },
        )
        .expect("arrest should validate")
        .commit(&mut state)
        .expect("arrest should commit");
        let supplemental_evidence = add_evidence(
            &mut state,
            police,
            investigation,
            defendant,
            EvidenceKind::FinancialRecord,
        );
        Fixture {
            registry,
            state,
            police,
            office,
            defendant,
            lead,
            investigation,
            arrest,
            arrest_evidence,
            supplemental_evidence,
        }
    }

    fn opening_draft(fixture: &Fixture) -> ProsecutionCaseDraft {
        ProsecutionCaseDraft {
            arrest: fixture.arrest,
            prosecutor_office: fixture.office,
            lead_prosecutor: fixture.lead,
            evidence: BTreeSet::from([fixture.arrest_evidence]),
        }
    }

    fn open_case(fixture: &mut Fixture) -> ProsecutionCaseId {
        validate_open_prosecution_case(&fixture.state, opening_draft(fixture))
            .expect("prosecution case should validate")
            .commit(&mut fixture.state)
            .expect("prosecution case should commit")
    }

    #[test]
    fn referral_preserves_police_custody_and_survives_save_before_supplement() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        let record = fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should persist");
        assert_eq!(record.status(), ProsecutionCaseStatus::Reviewing);
        assert_eq!(record.defendant(), fixture.defendant);
        assert_eq!(record.source_investigation(), fixture.investigation);
        assert_eq!(record.source_authority(), fixture.police);
        assert_eq!(record.prosecutor_office(), fixture.office);
        assert_eq!(record.lead_prosecutor(), fixture.lead);
        assert_eq!(
            record.evidence(),
            &BTreeSet::from([fixture.arrest_evidence])
        );
        assert_eq!(record.version(), 1);
        let initial_referral = record.initial_referral();
        let referral = fixture
            .state
            .legal()
            .get_prosecution_referral(initial_referral)
            .expect("initial referral should persist");
        assert_eq!(referral.evidence(), record.evidence());
        assert_eq!(
            fixture
                .state
                .legal()
                .get_evidence(fixture.arrest_evidence)
                .expect("source evidence should persist")
                .custodian(),
            fixture.police
        );
        assert_eq!(
            fixture
                .state
                .intelligence()
                .get_information(referral.information())
                .expect("referral information should persist")
                .holder(),
            KnowledgeHolder::Organization(fixture.office)
        );
        validate_state(&fixture.state).expect("initial prosecution referral should validate");
        validate_invariants(&fixture.state);

        let save = build_save(&fixture.registry, &fixture.state)
            .expect("prosecution referral should build a save");
        let bytes = bincode::serialize(&save).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let mut restored =
            restore_save(&fixture.registry, decoded).expect("prosecution referral should restore");
        let supplemental = validate_supplement_prosecution_case(
            &restored,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        )
        .expect("supplemental referral should validate after restore")
        .commit(&mut restored)
        .expect("supplemental referral should commit after restore");
        assert_ne!(supplemental, initial_referral);
        let updated = restored
            .legal()
            .get_prosecution_case(case)
            .expect("supplemented prosecution case should persist");
        assert_eq!(updated.version(), 2);
        assert_eq!(updated.referrals().len(), 2);
        assert_eq!(
            updated.evidence(),
            &BTreeSet::from([fixture.arrest_evidence, fixture.supplemental_evidence])
        );
        assert_eq!(
            restored
                .legal()
                .get_evidence(fixture.supplemental_evidence)
                .expect("supplemental source evidence should persist")
                .custodian(),
            fixture.police
        );
        validate_state(&restored).expect("supplemented restored prosecution case should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn initial_referral_must_include_every_evidence_record_that_supported_arrest() {
        let fixture = fixture();
        let error = match validate_open_prosecution_case(
            &fixture.state,
            ProsecutionCaseDraft {
                arrest: fixture.arrest,
                prosecutor_office: fixture.office,
                lead_prosecutor: fixture.lead,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        ) {
            Ok(_) => panic!("prosecution intake must not omit arrest evidence"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ProsecutionError::MissingArrestEvidence(fixture.arrest_evidence)
        );
        assert!(fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none());
        validate_state(&fixture.state).expect("rejected referral should preserve valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn supplemental_referral_stales_when_source_police_case_changes() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        let stale = validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        )
        .expect("supplement should initially validate");
        add_evidence(
            &mut fixture.state,
            fixture.police,
            fixture.investigation,
            fixture.defendant,
            EvidenceKind::Surveillance,
        );
        let error = stale
            .commit(&mut fixture.state)
            .expect_err("source case mutation must stale supplemental referral");
        assert!(matches!(error, ProsecutionError::StaleInvestigation { .. }));
        let record = fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("prosecution case should remain");
        assert_eq!(record.version(), 1);
        assert!(!record.evidence().contains(&fixture.supplemental_evidence));
        assert_eq!(record.referrals().len(), 1);
        validate_state(&fixture.state).expect("stale referral rejection should be atomic");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn open_case_is_unique_per_office_but_other_prosecutor_office_may_receive_referral() {
        let mut fixture = fixture();
        let first = open_case(&mut fixture);
        let duplicate =
            match validate_open_prosecution_case(&fixture.state, opening_draft(&fixture)) {
                Ok(_) => panic!("same office must not open duplicate case for one arrest"),
                Err(error) => error,
            };
        assert_eq!(
            duplicate,
            ProsecutionError::DuplicateOpenCase {
                arrest: fixture.arrest,
                office: fixture.office,
                case: first,
            }
        );

        let second_office = insert_organization(
            &fixture.registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "State Prosecutor".to_owned(),
                kind: OrganizationKind::Prosecutor,
            },
        )
        .expect("second prosecutor office should validate");
        let second_lead = insert_character(
            &fixture.registry,
            &mut fixture.state,
            CharacterDraft {
                name: "State Prosecutor Lead".to_owned(),
                organization: Some(second_office),
                supervisor: None,
                autonomy: AutonomyLevel::Broad,
                capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(91))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("second prosecutor should validate");
        let second = validate_open_prosecution_case(
            &fixture.state,
            ProsecutionCaseDraft {
                arrest: fixture.arrest,
                prosecutor_office: second_office,
                lead_prosecutor: second_lead,
                evidence: BTreeSet::from([fixture.arrest_evidence]),
            },
        )
        .expect("different prosecutor office may receive same arrest referral")
        .commit(&mut fixture.state)
        .expect("second office case should commit");
        assert_ne!(first, second);
        assert_eq!(
            fixture
                .state
                .legal()
                .prosecution_cases_for_arrest(fixture.arrest)
                .count(),
            2
        );
        validate_state(&fixture.state).expect("multiple-office referral state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn open_prosecution_case_blocks_lead_transfer_but_not_formal_case_persistence() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        let error = validate_reassign_character(&fixture.state, fixture.lead, None, None)
            .expect_err("open prosecution assignment must block office transfer");
        assert_eq!(
            error,
            WorldError::ActiveProsecutionAssignment {
                character: fixture.lead,
                case,
            }
        );
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.lead)
                .expect("lead should persist")
                .organization(),
            Some(fixture.office)
        );
        validate_state(&fixture.state).expect("rejected lead transfer should preserve valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn declining_case_releases_lead_assignment_and_ends_referral_access() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        fixture
            .state
            .advance_clock(crate::core::time::SimDuration::from_minutes(15));

        validate_decline_prosecution_case(&fixture.state, case)
            .expect("reviewing case should be eligible for decline")
            .commit(&mut fixture.state)
            .expect("decline should commit atomically");
        let record = fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("declined prosecution case should persist");
        assert_eq!(record.status(), ProsecutionCaseStatus::Declined);
        assert_eq!(record.resolved_at(), Some(fixture.state.now()));
        assert!(record.resolution_information().is_some());
        assert!(record.resolution_report().is_some());
        assert_eq!(record.version(), 2);
        assert!(fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none());

        let supplement_error = match validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        ) {
            Ok(_) => panic!("declined case must reject later evidence referral"),
            Err(error) => error,
        };
        assert_eq!(supplement_error, ProsecutionError::CaseNotOpen { case });

        validate_reassign_character(&fixture.state, fixture.lead, None, None)
            .expect("terminal prosecution case must release lead organization lock")
            .commit(&mut fixture.state)
            .expect("released lead should be able to leave prosecutor office");
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(fixture.lead)
                .expect("lead prosecutor should persist")
                .organization(),
            None
        );
        validate_state(&fixture.state).expect("declined historical case should remain valid");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn closed_case_survives_save_and_allows_later_reconsideration() {
        let mut fixture = fixture();
        let first = open_case(&mut fixture);
        fixture
            .state
            .advance_clock(crate::core::time::SimDuration::from_minutes(30));
        validate_close_prosecution_case(&fixture.state, first)
            .expect("reviewing case should be eligible for closure")
            .commit(&mut fixture.state)
            .expect("case closure should commit");

        let save = build_save(&fixture.registry, &fixture.state)
            .expect("closed prosecution case should build a save");
        let bytes = bincode::serialize(&save).expect("save should serialize");
        let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should deserialize");
        let mut restored = restore_save(&fixture.registry, decoded)
            .expect("closed prosecution case should restore");
        let historical = restored
            .legal()
            .get_prosecution_case(first)
            .expect("closed prosecution case should survive restore");
        assert_eq!(historical.status(), ProsecutionCaseStatus::Closed);
        assert_eq!(historical.resolved_at(), Some(restored.now()));
        assert!(historical.resolution_information().is_some());
        assert!(historical.resolution_report().is_some());
        assert!(restored
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_none());

        let second = validate_open_prosecution_case(&restored, opening_draft(&fixture))
            .expect("terminal case should permit later reconsideration")
            .commit(&mut restored)
            .expect("reconsidered prosecution case should commit");
        assert_ne!(first, second);
        assert_eq!(
            restored
                .legal()
                .open_prosecution_case_for(fixture.arrest, fixture.office)
                .expect("new prosecution review should own open index")
                .id(),
            second
        );
        assert_eq!(
            restored
                .legal()
                .prosecution_cases_for_arrest(fixture.arrest)
                .count(),
            2
        );
        validate_state(&restored).expect("reconsidered prosecution state should validate");
        validate_invariants(&restored);
    }

    #[test]
    fn prosecution_resolution_token_stales_after_new_referral_without_partial_resolution() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        let stale_resolution = validate_decline_prosecution_case(&fixture.state, case)
            .expect("decline should initially validate");
        validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        )
        .expect("supplement should validate before terminal disposition")
        .commit(&mut fixture.state)
        .expect("supplement should commit before stale decline token");

        assert_eq!(
            stale_resolution
                .commit(&mut fixture.state)
                .expect_err("case mutation must stale prior disposition token"),
            ProsecutionError::StaleProsecutionCase {
                case,
                expected: 1,
                found: 2,
            }
        );
        let record = fixture
            .state
            .legal()
            .get_prosecution_case(case)
            .expect("case should remain after stale resolution rejection");
        assert_eq!(record.status(), ProsecutionCaseStatus::Reviewing);
        assert_eq!(record.resolved_at(), None);
        assert_eq!(record.resolution_information(), None);
        assert_eq!(record.resolution_report(), None);
        assert!(fixture
            .state
            .legal()
            .open_prosecution_case_for(fixture.arrest, fixture.office)
            .is_some_and(|open| open.id() == case));
        validate_state(&fixture.state).expect("stale disposition rejection should be atomic");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn detained_lead_keeps_formal_case_assignment_but_cannot_refer_new_evidence() {
        let mut fixture = fixture();
        let case = open_case(&mut fixture);
        let lead_investigation = validate_open_investigation(
            &fixture.state,
            InvestigationDraft {
                owner: fixture.police,
                title: "Prosecutor misconduct inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(fixture.lead)]),
            },
        )
        .expect("lead investigation should validate")
        .commit(&mut fixture.state)
        .expect("lead investigation should commit");
        let lead_evidence = add_evidence(
            &mut fixture.state,
            fixture.police,
            lead_investigation,
            fixture.lead,
            EvidenceKind::Document,
        );
        let lead_arrest = validate_arrest(
            &fixture.state,
            ArrestDraft {
                character: fixture.lead,
                investigation: lead_investigation,
                evidence: BTreeSet::from([lead_evidence]),
            },
        )
        .expect("lead prosecutor may be arrested without erasing formal case assignment")
        .commit(&mut fixture.state)
        .expect("lead arrest should commit");
        assert_eq!(
            fixture
                .state
                .legal()
                .get_prosecution_case(case)
                .expect("prosecution case should persist")
                .lead_prosecutor(),
            fixture.lead
        );
        validate_state(&fixture.state)
            .expect("detained lead should leave formal prosecution case structurally valid");
        validate_invariants(&fixture.state);

        let error = match validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        ) {
            Ok(_) => panic!("detained lead must not perform new prosecutorial work"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            ProsecutionError::DetainedLeadProsecutor(fixture.lead)
        );
        assert_eq!(
            validate_decline_prosecution_case(&fixture.state, case)
                .err()
                .expect("detained lead must not resolve prosecution case"),
            ProsecutionError::DetainedLeadProsecutor(fixture.lead)
        );

        validate_release_arrest(&fixture.state, lead_arrest)
            .expect("lead detention should release")
            .commit(&mut fixture.state)
            .expect("lead release should commit");
        validate_supplement_prosecution_case(
            &fixture.state,
            ProsecutionReferralDraft {
                prosecution_case: case,
                evidence: BTreeSet::from([fixture.supplemental_evidence]),
            },
        )
        .expect("released lead should resume prosecutorial work")
        .commit(&mut fixture.state)
        .expect("supplement should commit after lead release");
        validate_state(&fixture.state).expect("released lead prosecution state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn private_legal_services_and_generic_legal_authority_cannot_act_as_prosecutor_office() {
        for kind in [
            OrganizationKind::LegalServices,
            OrganizationKind::LegalAuthority,
        ] {
            let mut fixture = fixture();
            let invalid_office = insert_organization(
                &fixture.registry,
                &mut fixture.state,
                OrganizationDraft {
                    name: format!("Invalid prosecution office {kind:?}"),
                    kind,
                },
            )
            .expect("invalid prosecution fixture organization should still be creatable");
            let invalid_lead = insert_character(
                &fixture.registry,
                &mut fixture.state,
                CharacterDraft {
                    name: "Invalid Prosecutor".to_owned(),
                    organization: Some(invalid_office),
                    supervisor: None,
                    autonomy: AutonomyLevel::Broad,
                    capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(80))]),
                    traits: BTreeSet::new(),
                    drives: BTreeMap::new(),
                },
            )
            .expect("invalid lead fixture should validate as a character");
            let error = match validate_open_prosecution_case(
                &fixture.state,
                ProsecutionCaseDraft {
                    arrest: fixture.arrest,
                    prosecutor_office: invalid_office,
                    lead_prosecutor: invalid_lead,
                    evidence: BTreeSet::from([fixture.arrest_evidence]),
                },
            ) {
                Ok(_) => panic!("non-prosecutor institution must not open prosecution case"),
                Err(error) => error,
            };
            assert_eq!(
                error,
                ProsecutionError::InvalidProsecutorOffice(invalid_office)
            );
            validate_state(&fixture.state)
                .expect("rejected prosecutor office should preserve state");
            validate_invariants(&fixture.state);
        }
    }
}
