//! Casework validation: investigations, scheduled detective work, witnesses and statements,
//! and the evidence graph they produce.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{CaseWitnessId, EvidenceId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::legal::investigation_work_execution::is_reviewable_evidence_kind;
use crate::legal::witness_system::{
    resolve_witness_reliability, resolve_witness_strength, witness_subject_is_case_relevant,
};
use crate::legal::{
    Admissibility, EvidenceKind, EvidenceRecord, InvestigationRecord, InvestigationStatus,
    InvestigationWorkFocus, InvestigationWorkKind, InvestigationWorkOutcome,
    InvestigationWorkRecord, InvestigationWorkStatus, WitnessCooperation,
};
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_investigations(state: &AppState) -> Result<(), StateValidationError> {
    let mut active_leads = BTreeSet::new();
    for investigation in state.legal.investigations() {
        validate_investigation(state, investigation)?;
        if investigation.status() == InvestigationStatus::Active
            && let Some(investigator) = investigation.lead_investigator()
            && !active_leads.insert(investigator)
        {
            return Err(invalid_investigation_staffing(investigation));
        }
    }

    Ok(())
}

fn validate_investigation(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    validate_investigation_definition(investigation)?;
    validate_investigation_owner(state, investigation)?;
    validate_investigation_chronology(state, investigation)?;
    validate_investigation_origin(state, investigation)?;
    validate_investigation_staffing(state, investigation)?;
    validate_investigation_subjects(state, investigation)
}

fn validate_investigation_definition(
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    if investigation.title().trim().is_empty()
        || investigation.declared_subjects().is_empty()
        || investigation.subjects().is_empty()
    {
        return Err(StateValidationError::InvalidInvestigationDefinition {
            investigation: investigation.id(),
        });
    }
    Ok(())
}

fn validate_investigation_owner(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    let owner = state.world.get_organization(investigation.owner()).ok_or(
        StateValidationError::MissingEntity {
            context: "investigation owner",
            entity: EntityRef::Organization(investigation.owner()),
        },
    )?;
    if !matches!(
        owner.kind(),
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    ) {
        return Err(StateValidationError::MissingEntity {
            context: "investigation owner",
            entity: EntityRef::Organization(investigation.owner()),
        });
    }
    Ok(())
}

fn validate_investigation_chronology(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    if investigation.opened_at() > state.now() {
        return Err(StateValidationError::FutureTimestamp {
            context: "investigation",
        });
    }
    if investigation.last_activity_at() > state.now()
        || investigation.last_activity_at() < investigation.opened_at()
    {
        return Err(invalid_investigation_activity(investigation));
    }
    Ok(())
}

fn validate_investigation_origin(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    let Some(origin) = investigation.origin() else {
        return Ok(());
    };
    if crate::legal::investigation_system::case_origin_responsible_organization(state, origin)
        .is_none()
    {
        return Err(invalid_investigation_activity(investigation));
    }
    Ok(())
}

fn validate_investigation_staffing(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    match investigation.status() {
        InvestigationStatus::Active
        | InvestigationStatus::Suspended
        | InvestigationStatus::Closed => {}
    }
    if investigation.version() == 0 {
        return Err(invalid_investigation_staffing(investigation));
    }
    let Some(investigator) = investigation.lead_investigator() else {
        return Ok(());
    };
    if investigation.status() != InvestigationStatus::Active {
        return Err(invalid_investigation_staffing(investigation));
    }
    let character = state
        .world
        .get_character(investigator)
        .ok_or_else(|| invalid_investigation_staffing(investigation))?;
    if character.organization() != Some(investigation.owner())
        || character
            .capability(CapabilityKind::Investigation)
            .is_none()
        || state
            .legal
            .active_arrest_for_character(investigator)
            .is_some()
        || investigation
            .subjects()
            .contains(&EntityRef::Character(investigator))
    {
        return Err(invalid_investigation_staffing(investigation));
    }
    Ok(())
}

fn validate_investigation_subjects(
    state: &AppState,
    investigation: &InvestigationRecord,
) -> Result<(), StateValidationError> {
    let mut expected_subjects = investigation.declared_subjects().clone();
    for subject in investigation.declared_subjects() {
        if !is_entity_present(state, *subject) {
            return Err(StateValidationError::MissingEntity {
                context: "investigation declared subject",
                entity: *subject,
            });
        }
    }
    for evidence_id in investigation.evidence() {
        let evidence = state.legal.get_evidence(*evidence_id).ok_or(
            StateValidationError::InvalidInvestigationDefinition {
                investigation: investigation.id(),
            },
        )?;
        if crate::legal::investigation_system::evidence_is_actionable_case_lead(evidence) {
            expected_subjects.insert(evidence.subject());
        }
    }
    if expected_subjects != *investigation.subjects() {
        return Err(StateValidationError::InvalidInvestigationDefinition {
            investigation: investigation.id(),
        });
    }
    for subject in investigation.subjects() {
        if !is_entity_present(state, *subject) {
            return Err(StateValidationError::MissingEntity {
                context: "investigation subject",
                entity: *subject,
            });
        }
    }
    Ok(())
}

fn invalid_investigation_activity(investigation: &InvestigationRecord) -> StateValidationError {
    StateValidationError::InvalidInvestigationActivity {
        investigation: investigation.id(),
    }
}

fn invalid_investigation_staffing(investigation: &InvestigationRecord) -> StateValidationError {
    StateValidationError::InvalidInvestigationStaffing {
        investigation: investigation.id(),
    }
}

pub(super) fn validate_investigation_work_records(
    state: &AppState,
    derived_evidence_from_work: &mut BTreeSet<EvidenceId>,
    completed_interviews_by_witness: &mut BTreeMap<CaseWitnessId, u32>,
) -> Result<(), StateValidationError> {
    let mut scheduled_investigators = BTreeSet::new();
    for work in state.legal.investigation_work() {
        validate_investigation_work_record(
            state,
            work,
            &mut scheduled_investigators,
            derived_evidence_from_work,
            completed_interviews_by_witness,
        )?;
    }

    Ok(())
}

fn validate_investigation_work_record(
    state: &AppState,
    work: &InvestigationWorkRecord,
    scheduled_investigators: &mut BTreeSet<crate::core::id::CharacterId>,
    derived_evidence_from_work: &mut BTreeSet<EvidenceId>,
    completed_interviews_by_witness: &mut BTreeMap<CaseWitnessId, u32>,
) -> Result<(), StateValidationError> {
    let investigation = state
        .legal
        .get_investigation(work.investigation())
        .ok_or_else(|| invalid_work(work))?;
    let investigator = state
        .world
        .get_character(work.investigator())
        .ok_or_else(|| invalid_work(work))?;
    validate_work_focus(state, work)?;
    validate_work_schedule_shape(state, work)?;
    match work.status() {
        InvestigationWorkStatus::Scheduled => validate_scheduled_work(
            state,
            work,
            investigation,
            investigator,
            scheduled_investigators,
        ),
        InvestigationWorkStatus::Completed => validate_completed_work(
            state,
            work,
            investigation,
            derived_evidence_from_work,
            completed_interviews_by_witness,
        ),
        InvestigationWorkStatus::Cancelled => validate_cancelled_work(state, work),
    }
}

fn validate_work_focus(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<(), StateValidationError> {
    let valid = match (work.kind(), work.focus()) {
        (InvestigationWorkKind::EvidenceReview, InvestigationWorkFocus::Evidence(source)) => {
            work.source_evidence().len() == 1
                && work.source_evidence().contains(&source)
                && state.legal.get_evidence(source).is_some_and(|evidence| {
                    evidence.investigation() == work.investigation()
                        && evidence.discovered_at() <= work.scheduled_at()
                        && is_reviewable_evidence_kind(evidence.kind())
                })
        }
        (
            InvestigationWorkKind::WitnessInterview,
            InvestigationWorkFocus::Witness(case_witness),
        ) => {
            work.source_evidence().is_empty()
                && state
                    .legal
                    .get_case_witness(case_witness)
                    .is_some_and(|witness| {
                        witness.investigation() == work.investigation()
                            && witness.registered_at() <= work.scheduled_at()
                    })
        }
        (InvestigationWorkKind::EvidenceReview, InvestigationWorkFocus::Witness(_))
        | (InvestigationWorkKind::WitnessInterview, InvestigationWorkFocus::Evidence(_)) => false,
    };
    if !valid {
        return Err(invalid_work(work));
    }
    Ok(())
}

fn validate_work_schedule_shape(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<(), StateValidationError> {
    if work.scheduled_at() > state.now()
        || work.due_at() <= work.scheduled_at()
        || work.source_evidence().iter().any(|source| {
            state.legal.get_evidence(*source).is_none_or(|evidence| {
                evidence.investigation() != work.investigation()
                    || evidence.discovered_at() > work.scheduled_at()
            })
        })
    {
        return Err(invalid_work(work));
    }
    Ok(())
}

fn validate_scheduled_work(
    state: &AppState,
    work: &InvestigationWorkRecord,
    investigation: &InvestigationRecord,
    investigator: &crate::world::CharacterRecord,
    scheduled_investigators: &mut BTreeSet<crate::core::id::CharacterId>,
) -> Result<(), StateValidationError> {
    let invalid_witness_focus = work
        .focus()
        .witness_id()
        .and_then(|case_witness| state.legal.get_case_witness(case_witness))
        .is_some_and(|witness| {
            !witness.statements().is_empty()
                || crate::legal::witness_system::case_witness_is_case_subject(state, witness)
        });
    if work.version() != 1
        || work.resolution().is_some()
        || work.cancellation().is_some()
        || invalid_witness_focus
        || !scheduled_investigators.insert(work.investigator())
        || investigation.status() != InvestigationStatus::Active
        || investigation.lead_investigator() != Some(work.investigator())
        || investigator.organization() != Some(investigation.owner())
        || investigator
            .capability(CapabilityKind::Investigation)
            .is_none()
    {
        return Err(invalid_work(work));
    }
    Ok(())
}

fn validate_completed_work(
    state: &AppState,
    work: &InvestigationWorkRecord,
    investigation: &InvestigationRecord,
    derived_evidence_from_work: &mut BTreeSet<EvidenceId>,
    completed_interviews_by_witness: &mut BTreeMap<CaseWitnessId, u32>,
) -> Result<(), StateValidationError> {
    let resolution = work.resolution().ok_or_else(|| invalid_work(work))?;
    if work.version() != 2
        || work.cancellation().is_some()
        || resolution.resolved_at() < work.due_at()
        || resolution.resolved_at() > state.now()
    {
        return Err(invalid_work(work));
    }
    record_completed_interview(work, completed_interviews_by_witness)?;
    match resolution.outcome() {
        InvestigationWorkOutcome::Connected => {
            validate_connected_work(state, work, investigation, derived_evidence_from_work)
        }
        InvestigationWorkOutcome::Developed => {
            if work.kind() != InvestigationWorkKind::EvidenceReview {
                return Err(invalid_work(work));
            }
            validate_developed_review_evidence(state, work, derived_evidence_from_work)
        }
        InvestigationWorkOutcome::Inconclusive => {
            if resolution.derived_evidence().is_some() {
                return Err(invalid_work(work));
            }
            Ok(())
        }
    }
}

fn record_completed_interview(
    work: &InvestigationWorkRecord,
    completed_interviews_by_witness: &mut BTreeMap<CaseWitnessId, u32>,
) -> Result<(), StateValidationError> {
    if work.kind() != InvestigationWorkKind::WitnessInterview {
        return Ok(());
    }
    let case_witness = work
        .focus()
        .witness_id()
        .ok_or_else(|| invalid_work(work))?;
    let attempts = completed_interviews_by_witness
        .entry(case_witness)
        .or_insert(0);
    *attempts = attempts
        .checked_add(1)
        .ok_or(StateValidationError::InvalidCaseWitness {
            witness: case_witness,
        })?;
    Ok(())
}

fn validate_connected_work(
    state: &AppState,
    work: &InvestigationWorkRecord,
    investigation: &InvestigationRecord,
    derived_evidence_from_work: &mut BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    let resolution = work.resolution().ok_or_else(|| invalid_work(work))?;
    let derived_id = resolution
        .derived_evidence()
        .ok_or_else(|| invalid_work(work))?;
    if !derived_evidence_from_work.insert(derived_id) {
        return Err(invalid_work(work));
    }
    if work.kind() != InvestigationWorkKind::WitnessInterview || !work.source_evidence().is_empty()
    {
        return Err(invalid_work(work));
    }
    let case_witness = work
        .focus()
        .witness_id()
        .ok_or_else(|| invalid_work(work))?;
    let derived = state
        .legal
        .get_evidence(derived_id)
        .ok_or_else(|| invalid_work(work))?;
    let statement_ok = state
        .legal
        .witness_statement_for_evidence(derived_id)
        .is_some_and(|statement| {
            statement.case_witness() == case_witness
                && statement.recorded_at() == resolution.resolved_at()
        });
    if derived.investigation() != work.investigation()
        || derived.custodian() != investigation.owner()
        || derived.kind() != EvidenceKind::WitnessTestimony
        || derived.discovered_at() != resolution.resolved_at()
        || !statement_ok
    {
        return Err(invalid_work(work));
    }
    Ok(())
}

fn validate_cancelled_work(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<(), StateValidationError> {
    let cancellation = work.cancellation().ok_or_else(|| invalid_work(work))?;
    let reason_valid = match cancellation.reason() {
        crate::legal::InvestigationWorkCancellationReason::InvestigatorDetained(arrest) => {
            state.legal.get_arrest(arrest).is_some_and(|arrest| {
                arrest.character() == work.investigator()
                    && arrest.arrested_at() == cancellation.cancelled_at()
            })
        }
        crate::legal::InvestigationWorkCancellationReason::WitnessBecameCaseSubject(evidence_id) => {
            work.kind() == InvestigationWorkKind::WitnessInterview
                && work
                    .focus()
                    .witness_id()
                    .and_then(|case_witness| state.legal.get_case_witness(case_witness))
                    .is_some_and(|witness| {
                        let evidence = state.legal.get_evidence(evidence_id);
                        witness.investigation() == work.investigation()
                            && evidence.is_some_and(|evidence| {
                                evidence.investigation() == work.investigation()
                                    && evidence.subject()
                                        == EntityRef::Character(witness.witness())
                                    && crate::legal::investigation_system::evidence_is_actionable_case_lead(
                                        evidence,
                                    )
                                    && evidence.discovered_at() <= cancellation.cancelled_at()
                            })
                    })
        }
        crate::legal::InvestigationWorkCancellationReason::WitnessStatementRecorded(statement) => {
            work.kind() == InvestigationWorkKind::WitnessInterview
                && work.focus().witness_id().is_some_and(|case_witness| {
                    state
                        .legal
                        .get_witness_statement(statement)
                        .is_some_and(|statement| {
                            statement.case_witness() == case_witness
                                && statement.recorded_at() == cancellation.cancelled_at()
                        })
                })
        }
    };
    if work.version() != 2
        || work.resolution().is_some()
        || cancellation.cancelled_at() < work.scheduled_at()
        || cancellation.cancelled_at() > state.now()
        || !reason_valid
    {
        return Err(invalid_work(work));
    }
    Ok(())
}

fn invalid_work(work: &InvestigationWorkRecord) -> StateValidationError {
    StateValidationError::InvalidInvestigationWork { work: work.id() }
}

/// Shared evidence contract for a completed evidence review: the derived forensic record must
/// re-derive exactly from its source through the canonical improvement rule. Used by both
/// release-safe validators so the contract has one owner and cannot drift between them.
pub(crate) fn validate_developed_review_evidence(
    state: &AppState,
    work: &crate::legal::InvestigationWorkRecord,
    derived_evidence_from_work: &mut BTreeSet<crate::core::id::EvidenceId>,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidInvestigationWork { work: work.id() };
    let Some(resolution) = work.resolution() else {
        return Err(invalid());
    };
    let investigation = state
        .legal
        .get_investigation(work.investigation())
        .ok_or_else(invalid)?;
    let source_id = work.focus().evidence_id().ok_or_else(invalid)?;
    let source = state.legal.get_evidence(source_id).ok_or_else(invalid)?;
    let derived_id = resolution.derived_evidence().ok_or_else(invalid)?;
    if !derived_evidence_from_work.insert(derived_id) {
        return Err(invalid());
    }
    let derived = state.legal.get_evidence(derived_id).ok_or_else(invalid)?;
    if derived.investigation() != work.investigation()
        || derived.custodian() != investigation.owner()
        || derived.kind() != EvidenceKind::ForensicAnalysis
        || derived.subject() != source.subject()
        || derived.origin() != source.origin()
        || derived.strength() != source.strength()
        || derived.reliability()
            != crate::legal::investigation_work_execution::resolve_improved_evidence_reliability(
                source.reliability(),
            )
        || derived.admissibility() != source.admissibility()
        || derived.discovered_at() != resolution.resolved_at()
        || derived.derived_from().len() != 1
        || !derived.derived_from().contains(&source_id)
        || source_id >= derived_id
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn validate_case_witnesses(
    state: &AppState,
    completed_interviews_by_witness: &BTreeMap<CaseWitnessId, u32>,
) -> Result<(), StateValidationError> {
    for witness in state.legal.case_witnesses() {
        let investigation = state
            .legal
            .get_investigation(witness.investigation())
            .ok_or(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            })?;
        let completed_interviews = completed_interviews_by_witness
            .get(&witness.id())
            .copied()
            .unwrap_or(0);
        let minimum_version = 1_u32
            .checked_add(completed_interviews)
            .and_then(|version| {
                u32::try_from(witness.statements().len())
                    .ok()
                    .and_then(|statements| version.checked_add(statements))
            })
            .ok_or(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            })?;
        if state.world.get_character(witness.witness()).is_none()
            || !is_entity_present(state, witness.subject())
            || !case_witness_subject_was_relevant_at_registration(state, investigation, witness)
            || witness.registered_at() < investigation.opened_at()
            || witness.registered_at() > state.now()
            || witness.statements().len() > 1
            || u32::from(witness.interview_attempts()) != completed_interviews
            // Registration starts at version 1. Every completed interview and every persisted
            // statement advances the witness exactly once; cooperation changes may add further
            // unhistoried increments, so this is a derivable lower bound rather than equality.
            || witness.version() < minimum_version
        {
            return Err(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            });
        }
        // Exhaustiveness canary: a new WitnessCooperation variant must be classified in
        // `discount_band` before persisted statements remain validatable.
        match witness.cooperation() {
            WitnessCooperation::Hostile
            | WitnessCooperation::Reluctant
            | WitnessCooperation::Cooperative => {}
        }
    }

    Ok(())
}

fn case_witness_subject_was_relevant_at_registration(
    state: &AppState,
    investigation: &InvestigationRecord,
    witness: &crate::legal::CaseWitnessRecord,
) -> bool {
    investigation.origin() == Some(witness.subject())
        || investigation
            .declared_subjects()
            .contains(&witness.subject())
        || investigation.evidence().iter().copied().any(|evidence_id| {
            state
                .legal
                .get_evidence(evidence_id)
                .is_some_and(|evidence| {
                    evidence.subject() == witness.subject()
                        && evidence.discovered_at() <= witness.registered_at()
                        && !(evidence.kind() == EvidenceKind::WitnessTestimony
                            && evidence.source() == Some(EntityRef::Character(witness.witness())))
                })
        })
}

pub(super) fn validate_witness_statements(
    state: &AppState,
) -> Result<BTreeSet<EvidenceId>, StateValidationError> {
    let mut named_witness_evidence = BTreeSet::new();
    for statement in state.legal.witness_statements() {
        let case_witness = state
            .legal
            .get_case_witness(statement.case_witness())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        let investigation = state
            .legal
            .get_investigation(case_witness.investigation())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        if statement.summary().trim().is_empty()
            || statement.recorded_at() < case_witness.registered_at()
            || statement.recorded_at() > state.now()
            || !is_entity_present(state, statement.subject())
            || statement.subject() != case_witness.subject()
            || !witness_subject_is_case_relevant(
                state,
                investigation,
                statement.subject(),
                Some(statement.evidence()),
            )
            || statement
                .origin()
                .is_some_and(|origin| !is_entity_present(state, origin))
            || !named_witness_evidence.insert(statement.evidence())
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
        let evidence = state.legal.get_evidence(statement.evidence()).ok_or(
            StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            },
        )?;
        if evidence.investigation() != case_witness.investigation()
            || evidence.custodian() != investigation.owner()
            || evidence.subject() != statement.subject()
            || evidence.origin() != statement.origin()
            || evidence.source() != Some(EntityRef::Character(case_witness.witness()))
            || evidence.kind() != EvidenceKind::WitnessTestimony
            || evidence.admissibility() != Admissibility::Unknown
            || evidence.discovered_at() != statement.recorded_at()
            || !evidence.derived_from().is_empty()
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
    }

    Ok(named_witness_evidence)
}

pub(crate) fn validate_witness_statements_against_registry(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let testimony = registry.legal().witness_testimony();
    for statement in state.legal.witness_statements() {
        let evidence = state.legal.get_evidence(statement.evidence()).ok_or(
            StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            },
        )?;
        if evidence.strength()
            != resolve_witness_strength(testimony, statement.confidence(), statement.cooperation())
            || evidence.reliability()
                != resolve_witness_reliability(
                    testimony,
                    statement.confidence(),
                    statement.cooperation(),
                )
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_evidence_records(
    state: &AppState,
    derived_evidence_from_work: &BTreeSet<EvidenceId>,
    named_witness_evidence: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    for evidence in state.legal.all_evidence() {
        validate_evidence_record(
            state,
            evidence,
            derived_evidence_from_work,
            named_witness_evidence,
            informant_evidence,
        )?;
    }

    Ok(())
}

fn validate_evidence_record(
    state: &AppState,
    evidence: &EvidenceRecord,
    derived_evidence_from_work: &BTreeSet<EvidenceId>,
    named_witness_evidence: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    validate_evidence_references(state, evidence)?;
    validate_evidence_named_source(state, evidence, named_witness_evidence, informant_evidence)?;
    if evidence.discovered_at() > state.now() {
        return Err(StateValidationError::FutureTimestamp {
            context: "evidence",
        });
    }
    validate_evidence_kind_provenance(evidence, derived_evidence_from_work, informant_evidence)?;
    validate_evidence_lineage(state, evidence)
}

fn validate_evidence_references(
    state: &AppState,
    evidence: &EvidenceRecord,
) -> Result<(), StateValidationError> {
    let investigation = state
        .legal
        .get_investigation(evidence.investigation())
        .ok_or(StateValidationError::MissingEntity {
            context: "evidence investigation",
            entity: EntityRef::Investigation(evidence.investigation()),
        })?;
    if state.world.get_organization(evidence.custodian()).is_none()
        || evidence.custodian() != investigation.owner()
    {
        return Err(StateValidationError::MissingEntity {
            context: "evidence custodian",
            entity: EntityRef::Organization(evidence.custodian()),
        });
    }
    if !is_entity_present(state, evidence.subject()) {
        return Err(StateValidationError::MissingEntity {
            context: "evidence subject",
            entity: evidence.subject(),
        });
    }
    if let Some(origin) = evidence.origin()
        && !is_entity_present(state, origin)
    {
        return Err(StateValidationError::MissingEntity {
            context: "evidence origin",
            entity: origin,
        });
    }
    Ok(())
}

fn validate_evidence_named_source(
    state: &AppState,
    evidence: &EvidenceRecord,
    named_witness_evidence: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    let Some(source) = evidence.source() else {
        if named_witness_evidence.contains(&evidence.id())
            || informant_evidence.contains(&evidence.id())
        {
            return Err(invalid_evidence_provenance(evidence));
        }
        return Ok(());
    };
    if !is_entity_present(state, source) {
        return Err(StateValidationError::MissingEntity {
            context: "evidence source",
            entity: source,
        });
    }
    let valid_source = matches!(source, EntityRef::Character(_))
        && evidence_source_kind_matches(evidence, named_witness_evidence, informant_evidence);
    if !valid_source {
        return Err(invalid_evidence_provenance(evidence));
    }
    Ok(())
}

fn evidence_source_kind_matches(
    evidence: &EvidenceRecord,
    named_witness_evidence: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> bool {
    match evidence.kind() {
        EvidenceKind::WitnessTestimony => {
            named_witness_evidence.contains(&evidence.id())
                && !informant_evidence.contains(&evidence.id())
        }
        EvidenceKind::InformantStatement => {
            informant_evidence.contains(&evidence.id())
                && !named_witness_evidence.contains(&evidence.id())
        }
        EvidenceKind::VehicleDescription
        | EvidenceKind::Fingerprint
        | EvidenceKind::RecoveredProperty
        | EvidenceKind::FinancialRecord
        | EvidenceKind::Surveillance
        | EvidenceKind::CommunicationRecord
        | EvidenceKind::KnownAssociation
        | EvidenceKind::Document
        | EvidenceKind::Ballistics
        | EvidenceKind::ForensicAnalysis => false,
    }
}

fn validate_evidence_kind_provenance(
    evidence: &EvidenceRecord,
    derived_evidence_from_work: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    let valid = match evidence.kind() {
        EvidenceKind::ForensicAnalysis => {
            evidence.source().is_none()
                && evidence.derived_from().len() == 1
                && derived_evidence_from_work.contains(&evidence.id())
        }
        EvidenceKind::InformantStatement => {
            informant_evidence.contains(&evidence.id())
                && evidence.source().is_some()
                && evidence.derived_from().is_empty()
        }
        EvidenceKind::WitnessTestimony
        | EvidenceKind::VehicleDescription
        | EvidenceKind::Fingerprint
        | EvidenceKind::RecoveredProperty
        | EvidenceKind::FinancialRecord
        | EvidenceKind::Surveillance
        | EvidenceKind::CommunicationRecord
        | EvidenceKind::KnownAssociation
        | EvidenceKind::Document
        | EvidenceKind::Ballistics => evidence.derived_from().is_empty(),
    };
    if !valid {
        return Err(invalid_evidence_provenance(evidence));
    }
    Ok(())
}

fn validate_evidence_lineage(
    state: &AppState,
    evidence: &EvidenceRecord,
) -> Result<(), StateValidationError> {
    for source_id in evidence.derived_from() {
        let source = state
            .legal
            .get_evidence(*source_id)
            .ok_or_else(|| invalid_evidence_provenance(evidence))?;
        if *source_id >= evidence.id()
            || source.investigation() != evidence.investigation()
            || source.discovered_at() > evidence.discovered_at()
        {
            return Err(invalid_evidence_provenance(evidence));
        }
    }
    Ok(())
}

fn invalid_evidence_provenance(evidence: &EvidenceRecord) -> StateValidationError {
    StateValidationError::InvalidEvidenceProvenance {
        evidence: evidence.id(),
    }
}
