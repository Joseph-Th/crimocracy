//! Prosecutor assignment, detention release, and deterministic autonomous staffing.

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId, ProsecutionCaseId};
use crate::core::state::AppState;
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
use crate::legal::{ProsecutionCaseRecord, ProsecutionCaseStatus};
use crate::world::CapabilityKind;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use thiserror::Error;

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
        ensure_version_can_advance_by(case.version(), 2, "prosecution case")?;
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
    ensure_version_can_advance_by(case_record.version(), 2, "prosecution case")?;
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
    let source_investigation = state
        .legal
        .get_investigation(case_record.source_investigation())
        .expect("validated prosecution case must retain its source investigation");
    if source_investigation
        .subjects()
        .contains(&EntityRef::Character(prosecutor))
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
    let mut office_rosters: BTreeMap<OrganizationId, Vec<(CharacterId, u8)>> = BTreeMap::new();
    let mut workloads: BTreeMap<CharacterId, usize> = BTreeMap::new();
    let mut planned = Vec::new();
    for case in cases {
        let case_record = state
            .legal
            .get_prosecution_case(case)
            .ok_or(ProsecutionStaffingError::MissingCase(case))?;
        // Reviewing cases can legitimately reach the finite version rail while unstaffed.
        // Direct assignment remains fail-closed with VersionCapacity, but autonomous maintenance
        // must skip a permanently unstaffable record so it cannot fail every later tick.
        if case_record.version() > u32::MAX - 2 {
            continue;
        }
        let office = case_record.prosecutor_office();
        let roster = office_rosters.entry(office).or_insert_with(|| {
            state
                .world
                .characters_in_organization(office)
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
                            workloads.insert(record.id(), workload);
                            (record.id(), rating.value())
                        })
                })
                .collect()
        });
        let prosecutor = roster
            .iter()
            .copied()
            .filter(|(prosecutor, _)| {
                !prosecutor_conflicts_with_case(state, case_record, *prosecutor)
            })
            .min_by_key(|(prosecutor, capability)| {
                (
                    workloads
                        .get(prosecutor)
                        .copied()
                        .expect("cached prosecution roster must carry a workload"),
                    Reverse(*capability),
                    *prosecutor,
                )
            })
            .map(|(prosecutor, _)| prosecutor);
        let Some(prosecutor) = prosecutor else {
            continue;
        };
        let assignment = validate_assign_prosecutor(state, case, prosecutor)?;
        *workloads
            .get_mut(&prosecutor)
            .expect("assigned cached prosecutor must carry a workload") += 1;
        planned.push((case, prosecutor, assignment));
    }

    // Case selection and workload balancing are fully modeled in the local planning maps above.
    // Validate every ranked assignment before the first case mutates so one exhausted/stale case
    // cannot leave only the earlier case IDs staffed behind an error return.
    let mut staffed = Vec::with_capacity(planned.len());
    for (case, prosecutor, assignment) in planned {
        assignment
            .commit(state)
            .expect("prevalidated prosecution staffing plan must remain current within one pass");
        staffed.push((case, prosecutor));
    }
    Ok(staffed)
}
fn prosecutor_conflicts_with_case(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    prosecutor: CharacterId,
) -> bool {
    let source_investigation = state
        .legal
        .get_investigation(case.source_investigation())
        .expect("reviewing prosecution case must retain its source investigation");
    prosecutor == case.defendant()
        || state
            .legal
            .case_witness_for(case.source_investigation(), prosecutor)
            .is_some()
        || source_investigation
            .subjects()
            .contains(&EntityRef::Character(prosecutor))
}
