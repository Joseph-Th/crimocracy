//! Casework validation: investigations, scheduled detective work, witnesses and statements,
//! and the evidence graph they produce.

//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::EvidenceId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::legal::investigation_work_execution::is_reviewable_evidence_kind;
use crate::legal::witness_system::{resolve_witness_reliability, resolve_witness_strength};
use crate::legal::{
    Admissibility, EvidenceKind, InvestigationStatus, InvestigationWorkFocus,
    InvestigationWorkKind, InvestigationWorkOutcome, InvestigationWorkStatus, WitnessCooperation,
};
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

pub(super) fn validate_investigations(state: &AppState) -> Result<(), StateValidationError> {
    for investigation in state.legal.investigations() {
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
        if investigation.opened_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "investigation",
            });
        }
        if investigation.last_activity_at() > state.now()
            || investigation.last_activity_at() < investigation.opened_at()
        {
            return Err(StateValidationError::InvalidInvestigationActivity {
                investigation: investigation.id(),
            });
        }
        let origin = investigation.origin();
        match origin {
            Some(origin_entity) => {
                // The originating entity's organization must have been surfaced the case-open
                // knowledge: an organization whose own activity opened a case always knows a
                // case exists, even while the evidence graph stays hidden.
                let responsible_organization = match origin_entity {
                    EntityRef::Operation(operation) => state
                        .operations
                        .get_operation(operation)
                        .ok_or(StateValidationError::InvalidInvestigationActivity {
                            investigation: investigation.id(),
                        })?
                        .responsible_organization(),
                    EntityRef::Enterprise(enterprise) => state
                        .enterprises
                        .get_enterprise(enterprise)
                        .ok_or(StateValidationError::InvalidInvestigationActivity {
                            investigation: investigation.id(),
                        })?
                        .organization(),
                    EntityRef::Organization(_)
                    | EntityRef::Character(_)
                    | EntityRef::Neighborhood(_)
                    | EntityRef::Business(_)
                    | EntityRef::Investigation(_)
                    | EntityRef::Evidence(_)
                    | EntityRef::FinancialAccount(_)
                    | EntityRef::DecisionRequest(_)
                    | EntityRef::Mandate(_) => {
                        return Err(StateValidationError::InvalidInvestigationActivity {
                            investigation: investigation.id(),
                        })
                    }
                };
                if investigation.notified_organizations().is_empty()
                    || !investigation
                        .notified_organizations()
                        .contains(&responsible_organization)
                {
                    return Err(StateValidationError::InvalidInvestigationActivity {
                        investigation: investigation.id(),
                    });
                }
            }
            None if !investigation.notified_organizations().is_empty() => {
                return Err(StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                });
            }
            None => {}
        }
        for notified in investigation.notified_organizations() {
            let organization = state.world.get_organization(*notified).ok_or(
                StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                },
            )?;
            if !matches!(
                organization.kind(),
                OrganizationKind::Criminal
                    | OrganizationKind::Political
                    | OrganizationKind::Press
                    | OrganizationKind::Labor
                    | OrganizationKind::Civic
                    | OrganizationKind::Commercial
            ) {
                return Err(StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                });
            }
        }
        // Exhaustiveness canary: a new InvestigationStatus must update the status checks above.
        match investigation.status() {
            InvestigationStatus::Active
            | InvestigationStatus::Suspended
            | InvestigationStatus::Closed => {}
        }
        if investigation.version() == 0
            || investigation
                .lead_investigator()
                .is_some_and(|lead| !investigation.assigned_investigators().contains(&lead))
        {
            return Err(StateValidationError::InvalidInvestigationStaffing {
                investigation: investigation.id(),
            });
        }
        for investigator in investigation.assigned_investigators() {
            let character = state.world.get_character(*investigator).ok_or(
                StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                },
            )?;
            if investigation.status() == InvestigationStatus::Active
                && (character.organization() != Some(investigation.owner())
                    || character
                        .capability(CapabilityKind::Investigation)
                        .is_none())
            {
                return Err(StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                });
            }
        }
        for subject in investigation.subjects() {
            if !is_entity_present(state, *subject) {
                return Err(StateValidationError::MissingEntity {
                    context: "investigation subject",
                    entity: *subject,
                });
            }
        }
    }

    Ok(())
}

pub(super) fn validate_investigation_work_records(
    state: &AppState,
    derived_evidence_from_work: &mut BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    for work in state.legal.investigation_work() {
        let investigation = state
            .legal
            .get_investigation(work.investigation())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let investigator = state
            .world
            .get_character(work.investigator())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let focus_is_valid = match (work.kind(), work.focus()) {
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
            | (InvestigationWorkKind::WitnessInterview, InvestigationWorkFocus::Evidence(_)) => {
                false
            }
        };
        if !focus_is_valid
            || work.scheduled_at() > state.now()
            || work.due_at() <= work.scheduled_at()
            || work.source_evidence().iter().any(|source| {
                state.legal.get_evidence(*source).is_none_or(|evidence| {
                    evidence.investigation() != work.investigation()
                        || evidence.discovered_at() > work.scheduled_at()
                })
            })
        {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        match work.status() {
            InvestigationWorkStatus::Scheduled => {
                if work.version() != 1
                    || work.resolution().is_some()
                    || investigation.status() != InvestigationStatus::Active
                    || !investigation
                        .assigned_investigators()
                        .contains(&work.investigator())
                    || investigator.organization() != Some(investigation.owner())
                    || investigator
                        .capability(CapabilityKind::Investigation)
                        .is_none()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkStatus::Completed => {
                let resolution = work
                    .resolution()
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                if work.version() != 2
                    || resolution.resolved_at() < work.due_at()
                    || resolution.resolved_at() > state.now()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                match resolution.outcome() {
                    InvestigationWorkOutcome::Connected => {
                        let derived_id = resolution.derived_evidence().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if !derived_evidence_from_work.insert(derived_id) {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        match work.kind() {
                            InvestigationWorkKind::WitnessInterview => {
                                if !work.source_evidence().is_empty() {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                                let Some(case_witness) = work.focus().witness_id() else {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                };
                                let derived = state.legal.get_evidence(derived_id).ok_or(
                                    StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    },
                                )?;
                                // The interview's derived evidence is the recorded
                                // testimony; the named statement must exist on the case
                                // witness and point back at the same evidence. The
                                // by-evidence statement index is the direct lookup.
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
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                            }
                            InvestigationWorkKind::EvidenceReview => {
                                return Err(StateValidationError::InvalidInvestigationWork {
                                    work: work.id(),
                                });
                            }
                        }
                    }
                    InvestigationWorkOutcome::Developed => {
                        if work.kind() != InvestigationWorkKind::EvidenceReview {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        validate_developed_review_evidence(
                            state,
                            work,
                            derived_evidence_from_work,
                        )?;
                    }
                    InvestigationWorkOutcome::Inconclusive => {
                        if resolution.derived_evidence().is_some() {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(())
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

pub(super) fn validate_case_witnesses(state: &AppState) -> Result<(), StateValidationError> {
    for witness in state.legal.case_witnesses() {
        let investigation = state
            .legal
            .get_investigation(witness.investigation())
            .ok_or(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            })?;
        if state.world.get_character(witness.witness()).is_none()
            || witness.registered_at() < investigation.opened_at()
            || witness.registered_at() > state.now()
            || witness.version() == 0
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
            || evidence.strength()
                != resolve_witness_strength(statement.confidence(), statement.cooperation())
            || evidence.reliability()
                != resolve_witness_reliability(statement.confidence(), statement.cooperation())
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

pub(super) fn validate_evidence_records(
    state: &AppState,
    derived_evidence_from_work: &BTreeSet<EvidenceId>,
    named_witness_evidence: &BTreeSet<EvidenceId>,
    informant_evidence: &BTreeSet<EvidenceId>,
) -> Result<(), StateValidationError> {
    for evidence in state.legal.all_evidence() {
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
        if let Some(origin) = evidence.origin() {
            if !is_entity_present(state, origin) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence origin",
                    entity: origin,
                });
            }
        }
        if let Some(source) = evidence.source() {
            if !is_entity_present(state, source) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence source",
                    entity: source,
                });
            }
            let valid_source = matches!(source, EntityRef::Character(_))
                && match evidence.kind() {
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
                };
            if !valid_source {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        } else if named_witness_evidence.contains(&evidence.id())
            || informant_evidence.contains(&evidence.id())
        {
            return Err(StateValidationError::InvalidEvidenceProvenance {
                evidence: evidence.id(),
            });
        }
        if evidence.discovered_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "evidence",
            });
        }
        match evidence.kind() {
            EvidenceKind::ForensicAnalysis => {
                if evidence.source().is_some()
                    || evidence.derived_from().len() != 1
                    || !derived_evidence_from_work.contains(&evidence.id())
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::InformantStatement => {
                if !informant_evidence.contains(&evidence.id())
                    || evidence.source().is_none()
                    || !evidence.derived_from().is_empty()
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
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
            | EvidenceKind::Ballistics => {
                if !evidence.derived_from().is_empty() {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
        }
        for source_id in evidence.derived_from() {
            let source = state.legal.get_evidence(*source_id).ok_or(
                StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                },
            )?;
            if *source_id >= evidence.id()
                || source.investigation() != evidence.investigation()
                || source.discovered_at() > evidence.discovered_at()
            {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        }
    }

    Ok(())
}
