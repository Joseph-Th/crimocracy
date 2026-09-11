//! Explicit police-to-prosecutor evidence referral and prosecution-case intake.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId, OrganizationId,
    ProsecutionCaseId, ProsecutionReferralId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::arrest_system::{ArrestError, validate_release_arrest};
use crate::legal::{
    ProsecutionCaseDraft, ProsecutionCaseRecord, ProsecutionCaseResolution, ProsecutionCaseStatus,
    ProsecutionReferralDraft, ProsecutionReferralRecord,
};
use crate::reports::report_system::{ReportError, ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
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
    #[error("prosecutor {0} does not exist")]
    MissingProsecutor(CharacterId),
    #[error("prosecutor {prosecutor} is not an active member of office {office}")]
    InvalidProsecutor {
        prosecutor: CharacterId,
        office: OrganizationId,
    },
    #[error("prosecutor {prosecutor} cannot prosecute defendant {defendant}")]
    ProsecutorIsDefendant {
        prosecutor: CharacterId,
        defendant: CharacterId,
    },
    #[error("prosecutor {prosecutor} is a named witness in source investigation {investigation}")]
    ProsecutorIsCaseWitness {
        prosecutor: CharacterId,
        investigation: InvestigationId,
    },
    #[error(
        "prosecutor {prosecutor} is an actionable subject of source investigation {investigation}"
    )]
    ProsecutorIsCaseSubject {
        prosecutor: CharacterId,
        investigation: InvestigationId,
    },
    #[error("prosecutor {0} is detained")]
    DetainedProsecutor(CharacterId),
    #[error("prosecutor {0} has no LegalKnowledge capability")]
    MissingLegalKnowledge(CharacterId),
    #[error("defendant {0} does not exist")]
    MissingDefendant(CharacterId),
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
    #[error("evidence {evidence} does not concern prosecution defendant {defendant}")]
    EvidenceDefendantMismatch {
        evidence: EvidenceId,
        defendant: CharacterId,
    },
    #[error("prosecution case {0} does not exist")]
    MissingProsecutionCase(ProsecutionCaseId),
    #[error("prosecution case {case} is not open for prosecutorial action")]
    CaseNotOpen { case: ProsecutionCaseId },
    #[error("prosecution case {case} has no currently assigned prosecutor")]
    CaseUnstaffed { case: ProsecutionCaseId },
    #[error("evidence {evidence} is already available to prosecution case {case}")]
    EvidenceAlreadyReferred {
        case: ProsecutionCaseId,
        evidence: EvidenceId,
    },
    #[error(
        "prosecution referral was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(
        "arrest {arrest} changed after referral validation; expected version {expected}, found {found}"
    )]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error(
        "source investigation {investigation} changed after referral validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "lead prosecutor {prosecutor} changed after referral validation; expected version {expected}, found {found}"
    )]
    StaleProsecutor {
        prosecutor: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "prosecution case {case} changed after referral validation; expected version {expected}, found {found}"
    )]
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
    Custody(#[from] ArrestError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProsecutionStaffingError {
    #[error("prosecution case {0} does not exist")]
    MissingCase(ProsecutionCaseId),
    #[error("prosecution case {0} is not under review")]
    CaseNotReviewing(ProsecutionCaseId),
    #[error("prosecution case {case} already has prosecutor {prosecutor} assigned")]
    CaseAlreadyStaffed {
        case: ProsecutionCaseId,
        prosecutor: CharacterId,
    },
    #[error("character {0} is not an eligible prosecutor for this case")]
    InvalidProsecutor(CharacterId),
    #[error("character {prosecutor} cannot prosecute themself in case {case}")]
    ProsecutorIsDefendant {
        case: ProsecutionCaseId,
        prosecutor: CharacterId,
    },
    #[error(
        "character {prosecutor} is a named witness in the source investigation for case {case}"
    )]
    ProsecutorIsCaseWitness {
        case: ProsecutionCaseId,
        prosecutor: CharacterId,
    },
    #[error(
        "character {prosecutor} is an actionable subject of the source investigation for case {case}"
    )]
    ProsecutorIsCaseSubject {
        case: ProsecutionCaseId,
        prosecutor: CharacterId,
    },
    #[error("character {0} is detained and cannot staff a prosecution case")]
    DetainedProsecutor(CharacterId),
    #[error("prosecution case {case} changed after staffing validation")]
    StaleCase { case: ProsecutionCaseId },
    #[error("prosecutor {prosecutor} changed after staffing validation")]
    StaleProsecutor { prosecutor: CharacterId },
    #[error("prosecutor {prosecutor}'s reviewing assignments changed after detention preflight")]
    DetentionAssignmentsChanged { prosecutor: CharacterId },
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

#[derive(Debug)]
pub(crate) struct ValidatedProsecutorDetentionRelease {
    prosecutor: CharacterId,
    assignments: Vec<(ProsecutionCaseId, u32)>,
}

impl ValidatedProsecutorDetentionRelease {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), ProsecutionStaffingError> {
        let current: Vec<_> = state
            .legal
            .reviewing_prosecution_cases_for_prosecutor(self.prosecutor)
            .filter(|case| case.status() == ProsecutionCaseStatus::Reviewing)
            .map(|case| (case.id(), case.version()))
            .collect();
        if current != self.assignments {
            return Err(ProsecutionStaffingError::DetentionAssignmentsChanged {
                prosecutor: self.prosecutor,
            });
        }
        for (_, version) in &self.assignments {
            ensure_version_can_advance(*version, "prosecution case")?;
        }
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) {
        for (case, _) in self.assignments {
            state
                .legal
                .release_prosecution_case_prosecutor_for_detention(case, self.prosecutor);
        }
    }
}

pub(crate) fn validate_release_prosecution_cases_for_detention(
    state: &AppState,
    prosecutor: CharacterId,
) -> Result<ValidatedProsecutorDetentionRelease, ProsecutionStaffingError> {
    let assignments: Vec<_> = state
        .legal
        .reviewing_prosecution_cases_for_prosecutor(prosecutor)
        .filter(|case| case.status() == ProsecutionCaseStatus::Reviewing)
        .map(|case| (case.id(), case.version()))
        .collect();
    for (_, version) in &assignments {
        ensure_version_can_advance(*version, "prosecution case")?;
    }
    // Keep an empty token too. A prosecutor can acquire a reviewing assignment after arrest
    // validation without changing their character version; the empty snapshot is what makes
    // that newly acquired responsibility stale the arrest instead of surviving custody.
    Ok(ValidatedProsecutorDetentionRelease {
        prosecutor,
        assignments,
    })
}

#[derive(Debug)]
pub struct ValidatedProsecutorAssignment {
    case: ProsecutionCaseId,
    prosecutor: CharacterId,
    expected_case_version: u32,
    expected_prosecutor_version: u32,
}

impl ValidatedProsecutorAssignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), ProsecutionStaffingError> {
        let case = state
            .legal
            .get_prosecution_case(self.case)
            .ok_or(ProsecutionStaffingError::MissingCase(self.case))?;
        if case.version() != self.expected_case_version
            || case.status() != ProsecutionCaseStatus::Reviewing
            || case.assigned_prosecutor().is_some()
        {
            return Err(ProsecutionStaffingError::StaleCase { case: self.case });
        }
        ensure_version_can_advance(case.version(), "prosecution case")?;
        let prosecutor = state
            .world
            .get_character(self.prosecutor)
            .ok_or(ProsecutionStaffingError::InvalidProsecutor(self.prosecutor))?;
        if prosecutor.version() != self.expected_prosecutor_version {
            return Err(ProsecutionStaffingError::StaleProsecutor {
                prosecutor: self.prosecutor,
            });
        }
        validate_prosecutor_assignment_dependencies(state, self.case, self.prosecutor)?;
        state
            .legal
            .set_prosecution_case_prosecutor(self.case, self.prosecutor);
        Ok(())
    }
}

pub fn validate_assign_prosecutor(
    state: &AppState,
    case: ProsecutionCaseId,
    prosecutor: CharacterId,
) -> Result<ValidatedProsecutorAssignment, ProsecutionStaffingError> {
    validate_prosecutor_assignment_dependencies(state, case, prosecutor)?;
    let case_record = state
        .legal
        .get_prosecution_case(case)
        .expect("validated prosecution case must exist");
    ensure_version_can_advance(case_record.version(), "prosecution case")?;
    let prosecutor_record = state
        .world
        .get_character(prosecutor)
        .expect("validated prosecutor must exist");
    Ok(ValidatedProsecutorAssignment {
        case,
        prosecutor,
        expected_case_version: case_record.version(),
        expected_prosecutor_version: prosecutor_record.version(),
    })
}

fn validate_prosecutor_assignment_dependencies(
    state: &AppState,
    case: ProsecutionCaseId,
    prosecutor: CharacterId,
) -> Result<(), ProsecutionStaffingError> {
    let case_record = state
        .legal
        .get_prosecution_case(case)
        .ok_or(ProsecutionStaffingError::MissingCase(case))?;
    if case_record.status() != ProsecutionCaseStatus::Reviewing {
        return Err(ProsecutionStaffingError::CaseNotReviewing(case));
    }
    if let Some(assigned) = case_record.assigned_prosecutor() {
        return Err(ProsecutionStaffingError::CaseAlreadyStaffed {
            case,
            prosecutor: assigned,
        });
    }
    if prosecutor == case_record.defendant() {
        return Err(ProsecutionStaffingError::ProsecutorIsDefendant { case, prosecutor });
    }
    if state
        .legal
        .case_witness_for(case_record.source_investigation(), prosecutor)
        .is_some()
    {
        return Err(ProsecutionStaffingError::ProsecutorIsCaseWitness { case, prosecutor });
    }
    if state
        .legal
        .get_investigation(case_record.source_investigation())
        .is_some_and(|investigation| {
            investigation
                .subjects()
                .contains(&EntityRef::Character(prosecutor))
        })
    {
        return Err(ProsecutionStaffingError::ProsecutorIsCaseSubject { case, prosecutor });
    }
    let record = state
        .world
        .get_character(prosecutor)
        .ok_or(ProsecutionStaffingError::InvalidProsecutor(prosecutor))?;
    if record.organization() != Some(case_record.prosecutor_office())
        || record.capability(CapabilityKind::LegalKnowledge).is_none()
    {
        return Err(ProsecutionStaffingError::InvalidProsecutor(prosecutor));
    }
    if state
        .legal
        .active_arrest_for_character(prosecutor)
        .is_some()
    {
        return Err(ProsecutionStaffingError::DetainedProsecutor(prosecutor));
    }
    Ok(())
}

pub(crate) fn apply_autonomous_prosecution_staffing(
    state: &mut AppState,
) -> Result<Vec<(ProsecutionCaseId, CharacterId)>, ProsecutionStaffingError> {
    let cases: Vec<_> = state
        .legal
        .reviewing_prosecution_cases_without_prosecutor()
        .collect();
    let mut staffed = Vec::new();
    for case in cases {
        let case_record = state
            .legal
            .get_prosecution_case(case)
            .ok_or(ProsecutionStaffingError::MissingCase(case))?;
        let prosecutor = find_autonomous_prosecutor(state, case_record);
        let Some(prosecutor) = prosecutor else {
            continue;
        };
        validate_assign_prosecutor(state, case, prosecutor)?.commit(state)?;
        staffed.push((case, prosecutor));
    }
    Ok(staffed)
}

/// Deterministic office staffing prefers the least-loaded eligible prosecutor, then legal
/// capability, then stable character identity. This keeps one highly skilled attorney from
/// absorbing every reviewing case while equally available colleagues remain idle, without
/// inventing an unsupported hard caseload cap.
fn find_autonomous_prosecutor(
    state: &AppState,
    case: &ProsecutionCaseRecord,
) -> Option<CharacterId> {
    state
        .world
        .characters_in_organization(case.prosecutor_office())
        .filter(|record| record.id() != case.defendant())
        .filter(|record| {
            state
                .legal
                .case_witness_for(case.source_investigation(), record.id())
                .is_none()
        })
        .filter(|record| {
            state
                .legal
                .get_investigation(case.source_investigation())
                .is_some_and(|investigation| {
                    !investigation
                        .subjects()
                        .contains(&EntityRef::Character(record.id()))
                })
        })
        .filter(|record| {
            state
                .legal
                .active_arrest_for_character(record.id())
                .is_none()
        })
        .filter_map(|record| {
            record
                .capability(CapabilityKind::LegalKnowledge)
                .map(|rating| {
                    let workload = state
                        .legal
                        .reviewing_prosecution_cases_for_prosecutor(record.id())
                        .count();
                    (record.id(), rating.value(), workload)
                })
        })
        .min_by_key(|(prosecutor, capability, workload)| {
            (*workload, Reverse(*capability), *prosecutor)
        })
        .map(|(prosecutor, _, _)| prosecutor)
}

#[derive(Clone, Copy, Debug)]
struct ReferralDependencies {
    defendant: CharacterId,
    source_investigation: InvestigationId,
    source_authority: OrganizationId,
    prosecutor_office: OrganizationId,
    prosecutor: CharacterId,
    arrest_version: u32,
    investigation_version: u32,
    prosecutor_version: u32,
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

        let information = self
            .information
            .commit(state)
            .expect("prosecution-opening information ID was preflighted before mutation");
        let report = self
            .report
            .commit(state)
            .expect("prosecution-opening report ID was preflighted before mutation");
        let case_id = state
            .ids
            .next_prosecution_case()
            .expect("prosecution-case ID was preflighted before mutation");
        let referral_id = state
            .ids
            .next_prosecution_referral()
            .expect("initial prosecution-referral ID was preflighted before mutation");
        // The referral record owns the evidence set; the case holds its own copy because
        // supplemental referrals grow the two independently.
        let case_evidence = self.draft.evidence.clone();
        state.legal.insert_prosecution_case(
            ProsecutionCaseRecord {
                id: case_id,
                context: super::ProsecutionCaseContext {
                    arrest: self.draft.arrest,
                    defendant: self.dependencies.defendant,
                    source_investigation: self.dependencies.source_investigation,
                    source_authority: self.dependencies.source_authority,
                    prosecutor_office: self.draft.prosecutor_office,
                    assigned_prosecutor: Some(self.draft.prosecutor),
                },
                referrals: super::ProsecutionCaseReferrals {
                    evidence: case_evidence,
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
                    resolution_prosecutor: None,
                },
                version: 1,
            },
            ProsecutionReferralRecord {
                id: referral_id,
                prosecution_case: case_id,
                source_investigation: self.dependencies.source_investigation,
                source_authority: self.dependencies.source_authority,
                prosecutor_office: self.draft.prosecutor_office,
                prosecutor: self.draft.prosecutor,
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
            prosecutor: draft.prosecutor,
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
    // Custody status is deliberately not checked here: charges may be referred after a
    // defendant's release, and the representation/custody sweeps elsewhere key off custody,
    // not off prosecution activity. The arrest record itself is the case's factual anchor.
    // The defendant is the subject of every artifact this case produces; opening a case against
    // an inactive person would assert the office reviewed someone who no longer exists.
    let _ = state
        .world
        .get_character(arrest.character())
        .ok_or(ProsecutionError::MissingDefendant(arrest.character()))?;
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
        arrest.character(),
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
    if source_authority.kind() != OrganizationKind::LawEnforcement
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
    if office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            draft.prosecutor_office,
        ));
    }
    let prosecutor_record = validate_prosecutor(
        state,
        draft.prosecutor_office,
        draft.prosecutor,
        arrest.character(),
        arrest.investigation(),
    )?;
    Ok(ReferralDependencies {
        defendant: arrest.character(),
        source_investigation: arrest.investigation(),
        source_authority: arrest.authority(),
        prosecutor_office: draft.prosecutor_office,
        prosecutor: draft.prosecutor,
        arrest_version: arrest.version(),
        investigation_version: investigation.version(),
        prosecutor_version: prosecutor_record.version(),
    })
}

pub struct ValidatedProsecutionReferral {
    draft: ProsecutionReferralDraft,
    prosecutor: CharacterId,
    expected_case_version: u32,
    expected_investigation_version: u32,
    expected_prosecutor_version: u32,
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
        ensure_version_can_advance(case.version(), "prosecution case")?;
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
        if case.assigned_prosecutor() != Some(self.prosecutor) {
            return Err(ProsecutionError::StaleProsecutor {
                prosecutor: self.prosecutor,
                expected: self.expected_prosecutor_version,
                found: state
                    .world
                    .get_character(self.prosecutor)
                    .map_or(0, |lead| lead.version()),
            });
        }
        let lead = state
            .world
            .get_character(self.prosecutor)
            .ok_or(ProsecutionError::MissingProsecutor(self.prosecutor))?;
        if lead.version() != self.expected_prosecutor_version {
            return Err(ProsecutionError::StaleProsecutor {
                prosecutor: lead.id(),
                expected: self.expected_prosecutor_version,
                found: lead.version(),
            });
        }
        validate_supplement_dependencies(state, &self.draft)?;
        let source_investigation = case.source_investigation();
        let source_authority = case.source_authority();
        let prosecutor_office = case.prosecutor_office();
        let information = self
            .information
            .commit(state)
            .expect("supplemental-referral information ID was preflighted before mutation");
        let report = self
            .report
            .commit(state)
            .expect("supplemental-referral report ID was preflighted before mutation");
        let referral_id = state
            .ids
            .next_prosecution_referral()
            .expect("supplemental prosecution-referral ID was preflighted before mutation");
        state
            .legal
            .add_prosecution_referral(ProsecutionReferralRecord {
                id: referral_id,
                prosecution_case: self.draft.prosecution_case,
                source_investigation,
                source_authority,
                prosecutor_office,
                prosecutor: self.prosecutor,
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
    ensure_version_can_advance(case.version(), "prosecution case")?;
    let investigation = state
        .legal
        .get_investigation(case.source_investigation())
        .expect("validated source investigation must exist");
    let prosecutor = assigned_prosecutor(case)?;
    let lead = state
        .world
        .get_character(prosecutor)
        .expect("validated assigned prosecutor must exist");
    let referred_at = state.now();
    let (information, report) = validate_referral_artifacts(
        state,
        ReferralArtifactContext {
            defendant: case.defendant(),
            source_investigation: case.source_investigation(),
            source_authority: case.source_authority(),
            prosecutor_office: case.prosecutor_office(),
            prosecutor,
            evidence: &draft.evidence,
            referred_at,
            initial: false,
        },
    )?;
    Ok(ValidatedProsecutionReferral {
        draft,
        prosecutor,
        expected_case_version: case.version(),
        expected_investigation_version: investigation.version(),
        expected_prosecutor_version: lead.version(),
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
        case.defendant(),
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
    if source.kind() != OrganizationKind::LawEnforcement {
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
    if office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            case.prosecutor_office(),
        ));
    }
    let prosecutor = assigned_prosecutor(case)?;
    validate_prosecutor(
        state,
        case.prosecutor_office(),
        prosecutor,
        case.defendant(),
        case.source_investigation(),
    )?;
    Ok(())
}

fn assigned_prosecutor(case: &ProsecutionCaseRecord) -> Result<CharacterId, ProsecutionError> {
    case.assigned_prosecutor()
        .ok_or(ProsecutionError::CaseUnstaffed { case: case.id() })
}

fn validate_prosecutor(
    state: &AppState,
    office: OrganizationId,
    prosecutor: CharacterId,
    defendant: CharacterId,
    source_investigation: InvestigationId,
) -> Result<&crate::world::CharacterRecord, ProsecutionError> {
    let lead = state
        .world
        .get_character(prosecutor)
        .ok_or(ProsecutionError::MissingProsecutor(prosecutor))?;
    if lead.organization() != Some(office) {
        return Err(ProsecutionError::InvalidProsecutor { prosecutor, office });
    }
    if prosecutor == defendant {
        return Err(ProsecutionError::ProsecutorIsDefendant {
            prosecutor,
            defendant,
        });
    }
    if state
        .legal
        .case_witness_for(source_investigation, prosecutor)
        .is_some()
    {
        return Err(ProsecutionError::ProsecutorIsCaseWitness {
            prosecutor,
            investigation: source_investigation,
        });
    }
    if state
        .legal
        .get_investigation(source_investigation)
        .is_some_and(|investigation| {
            investigation
                .subjects()
                .contains(&EntityRef::Character(prosecutor))
        })
    {
        return Err(ProsecutionError::ProsecutorIsCaseSubject {
            prosecutor,
            investigation: source_investigation,
        });
    }
    if state
        .legal
        .active_arrest_for_character(prosecutor)
        .is_some()
    {
        return Err(ProsecutionError::DetainedProsecutor(prosecutor));
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
    defendant: CharacterId,
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
        if !evidence_concerns_defendant(evidence, defendant) {
            return Err(ProsecutionError::EvidenceDefendantMismatch {
                evidence: *evidence_id,
                defendant,
            });
        }
    }
    Ok(())
}

/// Prosecution referrals are defendant-specific transfers, not bulk exports of an entire police
/// file. Keep the relevance predicate here so canonical opening/supplement validation and restore
/// invariants use one exact semantic boundary.
pub(crate) fn evidence_concerns_defendant(
    evidence: &crate::legal::EvidenceRecord,
    defendant: CharacterId,
) -> bool {
    evidence.subject() == EntityRef::Character(defendant)
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
    let lead = state
        .world
        .get_character(expected.prosecutor)
        .ok_or(ProsecutionError::MissingProsecutor(expected.prosecutor))?;
    if lead.version() != expected.prosecutor_version {
        return Err(ProsecutionError::StaleProsecutor {
            prosecutor: expected.prosecutor,
            expected: expected.prosecutor_version,
            found: lead.version(),
        });
    }
    Ok(())
}

fn validate_time(state: &AppState, expected: SimTime) -> Result<(), ProsecutionError> {
    crate::core::time::ensure_time_current(state.now(), expected)
        .map_err(|(expected, found)| ProsecutionError::StaleTime { expected, found })
}

pub struct ValidatedProsecutionCaseResolution {
    case: ProsecutionCaseId,
    resolution: ProsecutionCaseResolution,
    prosecutor: CharacterId,
    expected_case_version: u32,
    expected_prosecutor_version: u32,
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
        ensure_version_can_advance(case.version(), "prosecution case")?;
        if case.assigned_prosecutor() != Some(self.prosecutor) {
            return Err(ProsecutionError::CaseUnstaffed { case: self.case });
        }
        let lead = state
            .world
            .get_character(self.prosecutor)
            .ok_or(ProsecutionError::MissingProsecutor(self.prosecutor))?;
        if lead.version() != self.expected_prosecutor_version {
            return Err(ProsecutionError::StaleProsecutor {
                prosecutor: lead.id(),
                expected: self.expected_prosecutor_version,
                found: lead.version(),
            });
        }
        validate_resolution_dependencies(state, self.case)?;

        // A terminal prosecutorial review removes the legal basis this subsystem has for
        // continuing custody. Release the originating arrest when this is the last reviewing
        // prosecution case for it. A different office's live review keeps the arrest in force,
        // and a defendant already released (or later arrested under another arrest) is untouched.
        // Validate the release before the first artifact commit so a custody failure cannot leave
        // a half-resolved prosecution case.
        let release = if !state
            .legal
            .has_other_open_prosecution_case(case.arrest(), self.case)
            && state
                .legal
                .active_arrest_for_character(case.defendant())
                .is_some_and(|arrest| arrest.id() == case.arrest())
        {
            Some(validate_release_arrest(state, case.arrest())?)
        } else {
            None
        };

        let information = self
            .information
            .commit(state)
            .expect("prosecution-resolution information ID was preflighted before mutation");
        let report = self
            .report
            .commit(state)
            .expect("prosecution-resolution report ID was preflighted before mutation");
        state.legal.apply_prosecution_resolution(
            self.case,
            self.resolution,
            self.resolved_at,
            self.prosecutor,
            information,
            report,
        );
        if let Some(release) = release {
            release
                .commit(state)
                .expect("prevalidated prosecution custody release must remain current");
        }
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
    ensure_version_can_advance(case.version(), "prosecution case")?;
    let prosecutor = assigned_prosecutor(case)?;
    let lead = state
        .world
        .get_character(prosecutor)
        .expect("validated assigned prosecutor must exist");
    let resolved_at = state.now();
    let (information, report) =
        validate_resolution_artifacts(state, case, prosecutor, resolution, resolved_at)?;
    Ok(ValidatedProsecutionCaseResolution {
        case: case_id,
        resolution,
        prosecutor,
        expected_case_version: case.version(),
        expected_prosecutor_version: lead.version(),
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
    if office.kind() != OrganizationKind::Prosecutor {
        return Err(ProsecutionError::InvalidProsecutorOffice(
            case.prosecutor_office(),
        ));
    }
    let prosecutor = assigned_prosecutor(case)?;
    validate_prosecutor(
        state,
        case.prosecutor_office(),
        prosecutor,
        case.defendant(),
        case.source_investigation(),
    )?;
    // Resolving the case emits defendant-named artifacts; an inactive defendant cannot be
    // meaningfully reviewed, so resolution must not proceed against one.
    let _ = state
        .world
        .get_character(case.defendant())
        .ok_or(ProsecutionError::MissingDefendant(case.defendant()))?;
    Ok(case)
}

/// Renders the canonical resolution summary text for a terminal prosecution case; one
/// template source shared by the commit path and the invariant pass's scratch-buffer
/// re-render. The report titles are plain literals owned by the match arms above.
pub(crate) fn write_resolution_summary(
    out: &mut impl std::fmt::Write,
    resolution: ProsecutionCaseResolution,
    office_name: &str,
    defendant_name: &str,
    lead_name: &str,
) -> std::fmt::Result {
    match resolution {
        ProsecutionCaseResolution::Declined => write!(
            out,
            "{office_name} declined prosecution of {defendant_name} after review by {lead_name}."
        ),
        ProsecutionCaseResolution::Closed => write!(
            out,
            "{office_name} closed its prosecution review of {defendant_name} after review by {lead_name}."
        ),
    }
}

fn validate_resolution_artifacts(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    prosecutor: CharacterId,
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
        .get_character(prosecutor)
        .expect("validated lead prosecutor must exist")
        .name();
    let title = match resolution {
        ProsecutionCaseResolution::Declined => "Prosecution declined",
        ProsecutionCaseResolution::Closed => "Prosecution review closed",
    };
    let mut summary_buffer = String::new();
    write_resolution_summary(
        &mut summary_buffer,
        resolution,
        office_name,
        defendant_name,
        lead_name,
    )
    .expect("String buffer writes are infallible");
    let summary = summary_buffer;
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(case.prosecutor_office()),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(prosecutor)),
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
                    EntityRef::Character(prosecutor),
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
    prosecutor: CharacterId,
    evidence: &'a BTreeSet<EvidenceId>,
    referred_at: SimTime,
    initial: bool,
}

pub(crate) fn prosecution_referral_summary(
    source_authority: &str,
    defendant: &str,
    prosecutor_office: &str,
    evidence_count: usize,
    initial: bool,
) -> String {
    if initial {
        format!(
            "{source_authority} referred the arrest matter for {defendant} to {prosecutor_office}, sharing {evidence_count} evidence record(s)."
        )
    } else {
        format!(
            "{source_authority} supplemented the prosecution matter for {defendant} with {evidence_count} additional evidence record(s)."
        )
    }
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
        prosecutor,
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
    let summary = prosecution_referral_summary(
        source_name,
        defendant_name,
        office_name,
        evidence.len(),
        initial,
    );
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
                    EntityRef::Character(prosecutor),
                    EntityRef::Investigation(source_investigation),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok((information, report))
}

#[cfg(test)]
mod tests;
