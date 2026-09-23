//! Autonomous evidence-to-custody policy over the canonical arrest transaction.

use super::{
    ArrestError, CustodyCorroborationSource, CustodyEvidenceAssessment,
    evidence_qualifies_for_custody, repeat_custody_without_new_evidence, validate_arrest,
    validate_custody_evidence_references,
};
use crate::core::entity::EntityRef;
use crate::core::id::{ArrestId, CharacterId, EvidenceId, InvestigationId, OperationId};
use crate::core::state::AppState;
use crate::legal::{ArrestDraft, InvestigationStatus};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct AutonomousArrestCandidate {
    investigation: InvestigationId,
    character: CharacterId,
    evidence: BTreeSet<EvidenceId>,
    independent_sources: usize,
    strong_sources: usize,
}

fn resolve_autonomous_arrest_candidate(
    registry: &Registry,
    state: &AppState,
    investigation_id: InvestigationId,
    character: CharacterId,
) -> Result<Option<AutonomousArrestCandidate>, ArrestError> {
    if state.legal.active_arrest_for_character(character).is_some() {
        return Ok(None);
    }
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(ArrestError::MissingInvestigation(investigation_id))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(ArrestError::InactiveInvestigation(investigation_id));
    }
    let owner = investigation.owner();
    let mut assessment = CustodyEvidenceAssessment::default();
    let mut citations: BTreeMap<CustodyCorroborationSource, EvidenceId> = BTreeMap::new();
    for evidence_id in investigation.evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
        validate_custody_evidence_references(state, evidence)?;
        if evidence.subject() != EntityRef::Character(character) || evidence.custodian() != owner {
            continue;
        }
        let Some(source) = CustodyEvidenceAssessment::corroboration_source(state, evidence) else {
            continue;
        };
        assessment.observe(state, evidence);
        // Cite one qualifying record per independent source. Prefer the primary source when it
        // is itself custody-grade; otherwise a developed derivative must stand in for the weak
        // or questionable original that it legitimately improved. Evidence review preserves
        // source strength, so a custody-grade primary cannot discard a Strong/Direct property
        // that exists only on its derivative.
        let citation = match evidence.derived_from().iter().next().copied() {
            Some(source_id) => {
                let source_evidence = state
                    .legal
                    .get_evidence(source_id)
                    .expect("custody evidence lineage was validated before citation selection");
                if evidence_qualifies_for_custody(source_evidence) {
                    source_evidence.id()
                } else {
                    evidence.id()
                }
            }
            None => evidence.id(),
        };
        // Among several qualifying records from one source, cite the strongest account, not
        // merely the oldest: a Corroborating record must not stand in for a Strong one from
        // the same source. Evidence IDs break only exact strength/reliability ties.
        let rank = (
            evidence.strength(),
            evidence.reliability(),
            std::cmp::Reverse(evidence.id()),
        );
        citations
            .entry(source)
            .and_modify(|current| {
                let current_record = state
                    .legal
                    .get_evidence(*current)
                    .expect("autonomous citation must reference persisted evidence");
                let current_rank = (
                    current_record.strength(),
                    current_record.reliability(),
                    std::cmp::Reverse(current_record.id()),
                );
                if rank > current_rank {
                    *current = citation;
                }
            })
            .or_insert(citation);
    }
    if !assessment.meets(registry.legal().minimum_arrest_qualifying_evidence()) {
        return Ok(None);
    }
    let evidence: BTreeSet<_> = citations.into_values().collect();
    if repeat_custody_without_new_evidence(state, investigation_id, character, &evidence).is_some()
    {
        return Ok(None);
    }
    Ok(Some(AutonomousArrestCandidate {
        investigation: investigation_id,
        character,
        evidence,
        independent_sources: assessment.independent_qualifying(),
        strong_sources: assessment.strong_independent(),
    }))
}

/// Evidence bar for the autonomous conversion step: the authored number of independent
/// qualifying sources, with at least one Strong or Direct. Qualifying evidence targets the
/// subject directly, is held by the case's own authority, is not known inadmissible, and meets
/// the same minimum strength/reliability floor as the canonical arrest path. This is a
/// deliberately conservative institutional gate: it consumes case evidence that already exists;
/// it never generates new leads.
///
/// Runs the police institution's evidence-to-custody conversion across active cases. Custody
/// preempts conflicting operation bookings and scheduled detective work through their explicit
/// abort or cancellation lifecycles, so internal commitments cannot make an arrestable subject
/// immune.
pub fn apply_autonomous_evidence_arrests(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<ArrestId>, ArrestError> {
    // Single scan over active cases. Case provenance controls lifecycle and information flow,
    // not whether equally strong evidence can produce custody; subject pairs append directly so
    // a tick with no candidates allocates nothing beyond the one candidate buffer.
    let mut case_subjects: Vec<(InvestigationId, CharacterId)> = Vec::new();
    for investigation in state.legal().active_investigations() {
        // Legal authorities can own investigative files, but only law-enforcement institutions
        // have custody authority. Skip non-police case owners here instead of turning a valid
        // legal-authority case with strong evidence into a tick-failing InvalidAuthority error.
        let owner = state
            .world()
            .get_organization(investigation.owner())
            .ok_or(ArrestError::InvalidAuthority(investigation.owner()))?;
        if owner.kind() != OrganizationKind::LawEnforcement {
            continue;
        }
        let investigation_id = investigation.id();
        case_subjects.extend(investigation.subjects().iter().filter_map(|subject| {
            subject
                .as_character()
                .map(|character| (investigation_id, character))
        }));
    }
    if case_subjects.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for (investigation, character) in case_subjects {
        if let Some(candidate) =
            resolve_autonomous_arrest_candidate(registry, state, investigation, character)?
        {
            candidates.push(candidate);
        }
    }
    // One character can be arrestable in several police files at once, but only one active
    // detention can own them. Prefer the case with more independent qualifying sources, then
    // more Strong/Direct independent sources. Stable case id breaks only a true evidentiary tie
    // instead of silently rewarding whichever investigation happened to be opened first.
    candidates.sort_unstable_by_key(|candidate| {
        (
            candidate.character,
            std::cmp::Reverse(candidate.independent_sources),
            std::cmp::Reverse(candidate.strong_sources),
            candidate.investigation,
        )
    });
    candidates.dedup_by_key(|candidate| candidate.character);

    // Prevalidate every distinct arrestee and reserve the complete artifact budget before custody
    // becomes authoritative for anyone. Different arrestees may share one active operation; the
    // first detention aborts it, so count that abort's artifacts once.
    let mut id_budget = Vec::new();
    let mut seen_preempted_operations = BTreeSet::<OperationId>::new();
    let mut actionable = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let validated = match validate_arrest(
            registry,
            state,
            ArrestDraft {
                character: candidate.character,
                investigation: candidate.investigation,
                evidence: candidate.evidence.clone(),
            },
        ) {
            Ok(validated) => validated,
            // Direct custody must remain fail-closed when one of the character's live
            // responsibilities cannot be detached at its finite version rail. Autonomous
            // evidence conversion has no useful mutation available for that character, though,
            // and retrying the same permanent condition on every tick must not block unrelated
            // arrestable subjects in the cohort.
            Err(error) if autonomous_custody_is_terminally_blocked(&error) => continue,
            Err(error) => return Err(error),
        };
        id_budget.extend(
            validated.id_budget_excluding_duplicate_operation_preemptions(
                &mut seen_preempted_operations,
            ),
        );
        actionable.push(candidate);
    }
    if state.ids.reserve_many(&id_budget).is_err() {
        // The autonomous custody pass has not mutated anyone yet. At the finite persistence rail,
        // detain none of the cohort rather than returning an error that the canonical tick treats
        // as impossible or letting stable character/case order decide who is arrested first.
        return Ok(Vec::new());
    }

    let mut arrests = Vec::new();
    for candidate in actionable {
        // Revalidate against the post-earlier-arrest state so a shared operation already aborted
        // by another participant simply disappears from this character's preemption set. The
        // complete allocator budget and every candidate's independent legal dependencies were
        // already proven before the first custody mutation.
        let arrest = validate_arrest(
            registry,
            state,
            ArrestDraft {
                character: candidate.character,
                investigation: candidate.investigation,
                evidence: candidate.evidence,
            },
        )?
        .commit(state)?;
        arrests.push(arrest);
    }
    Ok(arrests)
}

fn autonomous_custody_is_terminally_blocked(error: &ArrestError) -> bool {
    matches!(
        error,
        ArrestError::SimulationTimeOverflow
            | ArrestError::VersionCapacity(_)
            | ArrestError::Investigation(
                crate::legal::investigation_system::InvestigationError::VersionCapacity(_),
            )
            | ArrestError::InvestigationWork(
                crate::legal::investigation_work_execution::InvestigationWorkError::VersionCapacity(_),
            )
            | ArrestError::ProsecutionStaffing(
                crate::legal::prosecution_system::ProsecutionStaffingError::VersionCapacity(_),
            )
            | ArrestError::LegalRepresentation(
                crate::legal::legal_representation_system::LegalRepresentationError::VersionCapacity(_),
            )
            | ArrestError::Operation(
                crate::operations::operation_system::OperationError::VersionCapacity(_),
            )
            | ArrestError::Decision(
                crate::decisions::decision_system::DecisionError::VersionCapacity(_),
            )
            | ArrestError::Decision(crate::decisions::decision_system::DecisionError::Operation(
                crate::operations::operation_system::OperationError::VersionCapacity(_),
            ))
    )
}
