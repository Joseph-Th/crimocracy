//! Detective staffing policy and canonical lead-assignment transaction.

use super::InvestigationError;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, IdExhaustionError, IdKind, InvestigationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::ensure_version_can_advance_by;
use crate::legal::InvestigationStatus;
use crate::world::CapabilityKind;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct ValidatedInvestigatorAssignment {
    investigation: InvestigationId,
    investigator: CharacterId,
    expected_investigation_version: u32,
    expected_investigator_version: u32,
}

struct PreparedInvestigatorAssignment {
    assignment: ValidatedInvestigatorAssignment,
    activity_knowledge: Option<crate::intelligence::intelligence_system::ValidatedInformation>,
    witness_knowledge: Vec<crate::intelligence::intelligence_system::ValidatedInformation>,
}

impl ValidatedInvestigatorAssignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), InvestigationError> {
        let prepared = self.prepare_current(state)?;
        state.ids.reserve(
            IdKind::Information,
            u32::try_from(prepared.information_count())
                .expect("persisted case-witness count must fit the information ID space"),
        )?;
        prepared.commit_preflighted(state);
        Ok(())
    }

    fn prepare_current(
        self,
        state: &AppState,
    ) -> Result<PreparedInvestigatorAssignment, InvestigationError> {
        let investigation = state
            .legal
            .get_investigation(self.investigation)
            .ok_or(InvestigationError::MissingInvestigation(self.investigation))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationError::StaleInvestigation {
                investigation: self.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        ensure_version_can_advance_by(investigation.version(), 2, "investigation")?;
        let investigator = state
            .world
            .get_character(self.investigator)
            .ok_or(InvestigationError::MissingCharacter(self.investigator))?;
        if investigator.version() != self.expected_investigator_version {
            return Err(InvestigationError::StaleInvestigator {
                investigator: self.investigator,
                expected: self.expected_investigator_version,
                found: investigator.version(),
            });
        }
        validate_investigator_assignment_dependencies(
            state,
            self.investigation,
            self.investigator,
        )?;
        // Taking the lead seat is a material case-activity fact: the new lead personally
        // knows the case is active. Prepared before mutation; committed after the role write.
        let activity_knowledge = crate::legal::case_knowledge::prepare_case_activity_knowledge(
            state,
            self.investigation,
            crate::intelligence::CaseActivitySignal::Active,
            self.investigator,
        )?;
        let witness_knowledge: Vec<_> = state
            .legal
            .case_witnesses_for_investigation(self.investigation)
            .map(|witness| {
                crate::legal::case_knowledge::prepare_case_witness_knowledge(
                    state,
                    self.investigation,
                    witness.witness(),
                    self.investigator,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(PreparedInvestigatorAssignment {
            assignment: self,
            activity_knowledge,
            witness_knowledge,
        })
    }
}

impl PreparedInvestigatorAssignment {
    fn information_count(&self) -> usize {
        usize::from(self.activity_knowledge.is_some()) + self.witness_knowledge.len()
    }

    fn commit_preflighted(self, state: &mut AppState) {
        state
            .legal
            .set_lead_investigator(self.assignment.investigation, self.assignment.investigator);
        if let Some(knowledge) = self.activity_knowledge {
            knowledge
                .commit(state)
                .expect("case-activity information ID was preflighted before staffing mutation");
        }
        for knowledge in self.witness_knowledge {
            knowledge
                .commit(state)
                .expect("case-witness information IDs were preflighted before staffing mutation");
        }
    }
}

/// Assigns an investigator as the lead of an active case. Staffing is single-seat: every
/// canonical producer promotes one lead, and support-investigator bookkeeping does not exist.
pub fn validate_assign_investigator(
    state: &AppState,
    investigation: InvestigationId,
    investigator: CharacterId,
) -> Result<ValidatedInvestigatorAssignment, InvestigationError> {
    validate_investigator_assignment_dependencies(state, investigation, investigator)?;
    let investigation_record = state
        .legal
        .get_investigation(investigation)
        .expect("validated investigation must still exist");
    ensure_version_can_advance_by(investigation_record.version(), 2, "investigation")?;
    let investigator_record = state
        .world
        .get_character(investigator)
        .expect("validated investigator must still exist");
    Ok(ValidatedInvestigatorAssignment {
        investigation,
        investigator,
        expected_investigation_version: investigation_record.version(),
        expected_investigator_version: investigator_record.version(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InvestigationStaffingPriority {
    actionable_evidence: Reverse<usize>,
    best_strength: Reverse<crate::legal::EvidenceStrength>,
    best_reliability: Reverse<crate::legal::EvidenceReliability>,
    evidence_count: Reverse<usize>,
    last_activity_at: Reverse<SimTime>,
    investigation: InvestigationId,
}

fn investigation_staffing_priority(
    state: &AppState,
    investigation_id: InvestigationId,
) -> InvestigationStaffingPriority {
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .expect("unstaffed-investigation index must reference an investigation");
    let evidence_summary = state
        .legal
        .investigation_evidence_staffing_summary(investigation_id);
    InvestigationStaffingPriority {
        actionable_evidence: Reverse(evidence_summary.actionable_count()),
        best_strength: Reverse(evidence_summary.best_strength()),
        best_reliability: Reverse(evidence_summary.best_reliability()),
        evidence_count: Reverse(investigation.evidence().len()),
        last_activity_at: Reverse(investigation.last_activity_at()),
        investigation: investigation_id,
    }
}

pub(crate) fn apply_autonomous_investigator_staffing(
    state: &mut AppState,
) -> Result<Vec<(InvestigationId, CharacterId)>, InvestigationError> {
    let mut investigations: Vec<_> = state.legal.active_investigations_without_lead().collect();
    // Scarce detective capacity is allocated by case substance rather than record creation
    // order. Prefer cases with more actionable evidence, then the strongest such evidence,
    // broader evidence, and more recent institutional activity. The case id is only the final
    // deterministic tie-break when the institution has no modeled reason to prefer either file.
    investigations.sort_unstable_by_key(|investigation| {
        investigation_staffing_priority(state, *investigation)
    });

    // Availability and Investigation capability are authority-wide facts for this staffing pass.
    // Build each authority's ranked pool once instead of rescanning every member for every
    // unstaffed case. Case-specific subject/witness conflicts remain checked below, and the
    // per-pass assignment set preserves one-active-case capacity as assignments commit.
    let owners: BTreeSet<_> = investigations
        .iter()
        .map(|investigation| {
            state
                .legal
                .get_investigation(*investigation)
                .expect("unstaffed-investigation index must reference an investigation")
                .owner()
        })
        .collect();
    let mut candidates_by_owner = BTreeMap::new();
    for owner in owners {
        let mut candidates: Vec<_> = state
            .world
            .characters_in_organization(owner)
            .filter(|record| {
                state
                    .legal
                    .active_arrest_for_character(record.id())
                    .is_none()
                    && state
                        .legal
                        .active_investigation_for_investigator(record.id())
                        .is_none()
            })
            .filter_map(|record| {
                record
                    .capability(CapabilityKind::Investigation)
                    .map(|capability| (record.id(), capability.value()))
            })
            .collect();
        candidates.sort_unstable_by_key(|(investigator, capability)| {
            (Reverse(*capability), *investigator)
        });
        candidates_by_owner.insert(
            owner,
            candidates
                .into_iter()
                .map(|(investigator, _)| investigator)
                .collect::<Vec<_>>(),
        );
    }

    let mut planned = Vec::new();
    let mut assigned_this_pass = BTreeSet::new();
    for investigation_id in investigations {
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .ok_or(InvestigationError::MissingInvestigation(investigation_id))?;
        // A max-version active case is a valid finite terminal rail: direct assignment must
        // still report VersionCapacity, but autonomous maintenance cannot ever make this case
        // representably staffable. Leave it unstaffed and continue allocating detectives to
        // cases that can still advance instead of failing every future simulation minute.
        if investigation.version() > u32::MAX - 2 {
            continue;
        }
        let owner = investigation.owner();
        let investigator = candidates_by_owner
            .get(&owner)
            .into_iter()
            .flatten()
            .copied()
            .find(|investigator| {
                !assigned_this_pass.contains(investigator)
                    && !investigation
                        .subjects()
                        .contains(&EntityRef::Character(*investigator))
                    && state
                        .legal
                        .case_witness_for(investigation_id, *investigator)
                        .is_none()
            });
        let Some(investigator) = investigator else {
            continue;
        };

        // Selection used the same current authoritative indexes and predicates as the canonical
        // validator. A rejection now is not a modeled "no investigator available" outcome; it
        // means state or allocator capacity is broken and must surface instead of disappearing.
        let prepared = validate_assign_investigator(state, investigation_id, investigator)?
            .prepare_current(state)?;
        assigned_this_pass.insert(investigator);
        planned.push((investigation_id, investigator, prepared));
    }

    // Autonomous staffing is one authority-wide allocation pass. Reserve the complete knowledge
    // budget before the first lead/index mutation so allocator exhaustion cannot staff a prefix
    // of the ranked case queue merely because those case IDs sorted first.
    let information_count = planned
        .iter()
        .try_fold(0_u32, |total, (_, _, prepared)| {
            let count = u32::try_from(prepared.information_count())
                .expect("persisted case-witness count must fit the information ID space");
            total.checked_add(count)
        })
        .ok_or_else(|| IdExhaustionError::Exhausted {
            kind: IdKind::Information.label(),
            next: state.ids.next_raw(IdKind::Information),
        })?;
    if state
        .ids
        .reserve(IdKind::Information, information_count)
        .is_err()
    {
        return Ok(Vec::new());
    }

    let mut staffed = Vec::with_capacity(planned.len());
    for (investigation_id, investigator, prepared) in planned {
        prepared.commit_preflighted(state);
        // The assignment commit records the new lead's personal case-activity knowledge, so
        // contact channels can disclose it without any case-graph read.
        staffed.push((investigation_id, investigator));
    }
    Ok(staffed)
}

fn validate_investigator_assignment_dependencies(
    state: &AppState,
    investigation_id: InvestigationId,
    investigator_id: CharacterId,
) -> Result<(), InvestigationError> {
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(InvestigationError::MissingInvestigation(investigation_id))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationError::InactiveInvestigation);
    }
    let investigator = state
        .world
        .get_character(investigator_id)
        .ok_or(InvestigationError::MissingCharacter(investigator_id))?;
    if let Some(arrest) = state.legal.active_arrest_for_character(investigator_id) {
        return Err(InvestigationError::DetainedInvestigator {
            investigator: investigator_id,
            arrest: arrest.id(),
        });
    }
    if investigation
        .subjects()
        .contains(&EntityRef::Character(investigator_id))
    {
        return Err(InvestigationError::InvestigatorIsCaseSubject {
            investigation: investigation_id,
            investigator: investigator_id,
        });
    }
    if let Some(witness) = state
        .legal
        .case_witness_for(investigation_id, investigator_id)
    {
        return Err(InvestigationError::InvestigatorIsCaseWitness {
            investigation: investigation_id,
            investigator: investigator_id,
            witness: witness.id(),
        });
    }
    if investigator.organization() != Some(investigation.owner()) {
        return Err(InvestigationError::InvestigatorOwnerMismatch {
            investigator: investigator_id,
            owner: investigation.owner(),
        });
    }
    // One active case per investigator: a detective already leading another active case cannot
    // take a second active case.
    if state
        .legal
        .active_investigation_for_investigator(investigator_id)
        .is_some()
    {
        return Err(InvestigationError::InvestigatorAtCaseCapacity {
            investigator: investigator_id,
        });
    }
    if investigator
        .capability(CapabilityKind::Investigation)
        .is_none()
    {
        return Err(InvestigationError::MissingInvestigationCapability(
            investigator_id,
        ));
    }
    // Single-seat staffing: a canonical producer fills an empty lead seat; replacing a lead
    // is not an assignment operation.
    if let Some(current) = investigation.lead_investigator() {
        return Err(InvestigationError::LeadSeatFilled {
            investigation: investigation_id,
            lead: current,
        });
    }
    Ok(())
}
