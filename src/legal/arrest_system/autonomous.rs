//! Autonomous evidence-to-custody policy over the canonical arrest transaction.

use super::{
    ArrestError, CustodyCorroborationSource, CustodyEvidenceAssessment,
    evidence_qualifies_for_custody, find_fresh_independent_custody_evidence,
    repeat_custody_without_new_evidence, validate_arrest, validate_custody_evidence_references,
};
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EnterpriseId, EvidenceId, InvestigationId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime};
use crate::legal::{ArrestDraft, ArrestRecord, EvidenceRecord, InvestigationStatus};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct AutonomousArrestCandidate {
    investigation: InvestigationId,
    authority: OrganizationId,
    character: CharacterId,
    evidence: BTreeSet<EvidenceId>,
    independent_sources: usize,
    strong_sources: usize,
}

/// The strongest and newest qualifying records serve different purposes: the former
/// supports corroboration and prosecution, while the latter can establish a fresh basis
/// for renewed custody. Retain both in the case's single evidence pass.
struct SourceCitations<'a> {
    strongest: &'a EvidenceRecord,
    newest: &'a EvidenceRecord,
}

impl<'a> SourceCitations<'a> {
    fn observe(&mut self, cited: &'a EvidenceRecord, evidence: &'a EvidenceRecord) {
        if Self::quality_rank(cited) > Self::quality_rank(self.strongest) {
            self.strongest = cited;
        }
        if (evidence.discovered_at(), Self::quality_rank(evidence))
            > (self.newest.discovered_at(), Self::quality_rank(self.newest))
        {
            self.newest = evidence;
        }
    }

    fn quality_rank(
        evidence: &EvidenceRecord,
    ) -> (
        crate::legal::EvidenceStrength,
        crate::legal::EvidenceReliability,
        std::cmp::Reverse<EvidenceId>,
    ) {
        (
            evidence.strength(),
            evidence.reliability(),
            std::cmp::Reverse(evidence.id()),
        )
    }
}

/// Routine vice casework may continue after release, but the same already-cited
/// racket must not create another arrest merely by being observed each day.
/// New racket evidence remains actionable after the routine cooldown, and completed
/// post-release criminal operations can bypass that cooldown.
/// This policy is reconstructed from persisted arrests and never alters direct arrest validity.
const ROUTINE_REARREST_INTERVAL_DAYS: u64 = 60;

fn case_racket(state: &AppState, case: InvestigationId) -> Option<EnterpriseId> {
    let EntityRef::Enterprise(id) = state.legal().get_investigation(case)?.origin()? else {
        return None;
    };
    Some(id)
}

/// Date a cited observation to the underlying operating event, not a later
/// forensic analysis of it.
fn observed_racket(state: &AppState, id: EvidenceId) -> Option<(EnterpriseId, SimTime)> {
    let record = state.legal().get_evidence(id)?;
    let root = match record.derived_from().len() {
        0 => record,
        1 => state
            .legal()
            .get_evidence(*record.derived_from().first()?)?,
        _ => return None,
    };
    let racket = crate::legal::investigation_system::enterprise_observation_origin(
        root.kind(),
        root.subject(),
        root.origin(),
    )?;
    Some((racket, root.discovered_at()))
}

#[derive(Default)]
struct ReleaseHistory {
    /// The general cooldown follows the most recent release by this authority.
    latest: Option<SimTime>,
    /// Vice history survives an unrelated arrest and recognizes cited incident roots.
    rackets: BTreeMap<EnterpriseId, SimTime>,
}

impl ReleaseHistory {
    fn record(&mut self, state: &AppState, arrest: &ArrestRecord) {
        let Some(released_at) = arrest.released_at() else {
            return;
        };
        self.latest = self.latest.max(Some(released_at));
        // A consolidated inquiry can cite multiple rackets. Preserve every
        // cited racket as well as the originating vice file across unrelated
        // later arrests.
        for racket in case_racket(state, arrest.investigation())
            .into_iter()
            .chain(
                arrest
                    .evidence()
                    .iter()
                    .filter_map(|&id| observed_racket(state, id).map(|(racket, _)| racket)),
            )
        {
            let last = self.rackets.entry(racket).or_insert(released_at);
            *last = (*last).max(released_at);
        }
    }

    fn blocks(&self, state: &AppState, candidate: &AutonomousArrestCandidate) -> bool {
        let Some(released_at) = self.latest else {
            return false;
        };
        if has_distinct_post_release_operation(state, released_at, &candidate.evidence) {
            return false;
        }
        if state
            .now()
            .as_minutes()
            .saturating_sub(released_at.as_minutes())
            < ROUTINE_REARREST_INTERVAL_DAYS * DAY_MINUTES
        {
            return true;
        }
        let Some(racket) = case_racket(state, candidate.investigation) else {
            return false;
        };
        let Some(&racket_release) = self.rackets.get(&racket) else {
            return false;
        };
        !candidate.evidence.iter().any(|&id| {
            observed_racket(state, id).is_some_and(|(observed, at)| {
                !self.rackets.contains_key(&observed) && at > racket_release
            })
        })
    }
}

fn has_distinct_post_release_operation(
    state: &AppState,
    released_at: SimTime,
    evidence: &BTreeSet<EvidenceId>,
) -> bool {
    evidence.iter().any(|id| {
        state.legal().get_evidence(*id).is_some_and(|record| {
            let Some(EntityRef::Operation(operation)) = record.origin() else {
                return false;
            };
            record.discovered_at() > released_at
                && state
                    .operations()
                    .get_operation(operation)
                    .and_then(|operation| operation.resolution())
                    .is_some_and(|outcome| outcome.resolved_at() > released_at)
        })
    })
}

fn release_history_by_subject_and_authority(
    state: &AppState,
    candidates: &[AutonomousArrestCandidate],
) -> BTreeMap<(CharacterId, OrganizationId), ReleaseHistory> {
    let mut history: BTreeMap<_, ReleaseHistory> = candidates
        .iter()
        .map(|candidate| {
            (
                (candidate.character, candidate.authority),
                ReleaseHistory::default(),
            )
        })
        .collect();
    let subjects: BTreeSet<_> = candidates
        .iter()
        .map(|candidate| candidate.character)
        .collect();
    for subject in subjects {
        for arrest in state.legal().arrests_for_character(subject) {
            if let Some(prior) = history.get_mut(&(subject, arrest.authority())) {
                prior.record(state, arrest);
            }
        }
    }
    history
}

/// A single case-level evidence pass supplies all character subjects. A multi-subject case
/// must not rescan its complete evidence history for every suspect on every canonical minute.
/// Corroboration and citation ranking are still tracked separately for each character.
fn resolve_autonomous_arrest_candidates(
    registry: &Registry,
    state: &AppState,
    investigation_id: InvestigationId,
) -> Result<Vec<AutonomousArrestCandidate>, ArrestError> {
    let investigation = state
        .legal
        .get_investigation(investigation_id)
        .ok_or(ArrestError::MissingInvestigation(investigation_id))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(ArrestError::InactiveInvestigation(investigation_id));
    }
    let owner = investigation.owner();
    let mut by_character: BTreeMap<
        CharacterId,
        (
            CustodyEvidenceAssessment,
            BTreeMap<CustodyCorroborationSource, SourceCitations<'_>>,
        ),
    > = investigation
        .subjects()
        .iter()
        .filter_map(|subject| subject.as_character())
        .filter(|character| {
            state
                .legal
                .active_arrest_for_character(*character)
                .is_none()
        })
        .map(|character| {
            (
                character,
                (CustodyEvidenceAssessment::default(), BTreeMap::new()),
            )
        })
        .collect();
    if by_character.is_empty() {
        return Ok(Vec::new());
    }
    for evidence_id in investigation.evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
        validate_custody_evidence_references(state, evidence)?;
        let Some(character) = evidence.subject().as_character() else {
            continue;
        };
        let Some((assessment, citations)) = by_character.get_mut(&character) else {
            continue;
        };
        if evidence.custodian() != owner {
            continue;
        }
        let Some(source) = CustodyEvidenceAssessment::corroboration_source(state, evidence) else {
            continue;
        };
        assessment.observe(state, evidence);
        // Choose the strongest qualifying record per independent source. A forensic review
        // can improve reliability without creating a second independent fact. Prefer the
        // improved analysis when its quality is higher, but use the primary on a quality tie.
        let cited = match evidence.derived_from().iter().next().copied() {
            Some(source_id) => {
                let source_evidence = state
                    .legal
                    .get_evidence(source_id)
                    .expect("custody evidence lineage was validated before citation selection");
                if evidence_qualifies_for_custody(source_evidence)
                    && (source_evidence.strength(), source_evidence.reliability())
                        >= (evidence.strength(), evidence.reliability())
                {
                    source_evidence
                } else {
                    evidence
                }
            }
            None => evidence,
        };
        // Rank the selected record itself, while keeping the actual observation minute
        // separately for renewed custody. Stable ID breaks only equal-quality ties.
        citations
            .entry(source)
            .and_modify(|current| current.observe(cited, evidence))
            .or_insert(SourceCitations {
                strongest: cited,
                newest: evidence,
            });
    }
    Ok(by_character
        .into_iter()
        .filter_map(|(character, (assessment, citations))| {
            if !assessment.meets(registry.legal().minimum_arrest_qualifying_evidence()) {
                return None;
            }
            let mut evidence: BTreeSet<_> = citations
                .values()
                .map(|citation| citation.strongest.id())
                .collect();
            if let Some((prior_id, released_at)) =
                repeat_custody_without_new_evidence(state, investigation_id, character, &evidence)
            {
                if state.now() <= released_at {
                    return None;
                }
                let prior = state
                    .legal()
                    .get_arrest(prior_id)
                    .expect("repeat-custody guard returned a persisted arrest");
                // A higher-quality older record can obscure a fresh statement from the
                // same previously uncited source. Include the fresh record as an additional
                // citation only when required; it never counts as a second independent source.
                let fresh = find_fresh_independent_custody_evidence(
                    state,
                    prior,
                    citations.values().map(|citation| citation.newest.id()),
                    released_at,
                )?;
                evidence.insert(fresh);
            }
            Some(AutonomousArrestCandidate {
                investigation: investigation_id,
                authority: owner,
                character,
                evidence,
                independent_sources: assessment.independent_qualifying(),
                strong_sources: assessment.strong_independent(),
            })
        })
        .collect())
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
    // Single scan over active cases. Each case's evidence is assessed just once for every
    // currently undetained subject, instead of once per subject on every tick.
    let mut candidates = Vec::new();
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
        candidates.extend(resolve_autonomous_arrest_candidates(
            registry,
            state,
            investigation.id(),
        )?);
    }
    // Most minutes have no custody-grade candidates. Re-read lifetime release history only
    // when there is actual qualifying evidence, not on every canonical simulation minute.
    if !candidates.is_empty() {
        let history = release_history_by_subject_and_authority(state, &candidates);
        candidates.retain(|candidate| {
            !history
                .get(&(candidate.character, candidate.authority))
                .is_some_and(|prior| prior.blocks(state, candidate))
        });
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
