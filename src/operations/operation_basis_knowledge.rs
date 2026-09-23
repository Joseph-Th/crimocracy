//! Knowledge proof for operation openings whose practical basis is not ambient world truth.
//!
//! Most field operations can target visible people, places, and businesses directly. Witness
//! pressure and extraction are different: the actionable fact is institutional legal status.
//! This module makes that status a learned, typed fact rather than an oracle query into legal
//! state. Opportunity discovery and direct operation authorization share this exact proof.

use crate::core::entity::EntityRef;
use crate::core::id::{ArrestId, CaseWitnessId, CharacterId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::{
    InformationRecord, InformationSignal, KnowledgeHolder, LegalPersonStatusSignal,
};
use crate::operations::OperationKind;
use crate::operations::operation_intelligence::resolve_information_score;
use crate::registry::Registry;
use std::collections::BTreeSet;

pub(crate) fn source_information_is_usable_for_operation_basis(
    registry: &Registry,
    operation_kind: OperationKind,
    information: &InformationRecord,
    at: SimTime,
) -> bool {
    if information.observed_at() > at {
        return false;
    }
    let max_age = u64::from(
        registry
            .get_operation(operation_kind)
            .execution()
            .max_intelligence_age()
            .as_minutes(),
    );
    resolve_information_score(registry.information_quality(), information, at, max_age) > 0
}

pub(crate) fn source_information_proves_operation_basis(
    registry: &Registry,
    state: &AppState,
    operation_kind: OperationKind,
    target: EntityRef,
    information: &InformationRecord,
    at: SimTime,
) -> bool {
    if information.subject() != target
        || !source_information_is_usable_for_operation_basis(
            registry,
            operation_kind,
            information,
            at,
        )
    {
        return false;
    }

    match operation_kind {
        OperationKind::WitnessPressure => {
            let EntityRef::Character(character) = target else {
                return false;
            };
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
                investigation,
            })) = information.signal()
            else {
                return false;
            };
            state
                .legal
                .case_witness_for(*investigation, character)
                .is_some_and(|witness| witness.registered_at() <= information.observed_at())
        }
        OperationKind::Extraction => {
            let EntityRef::Character(character) = target else {
                return false;
            };
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
                arrest,
            })) = information.signal()
            else {
                return false;
            };
            state.legal.get_arrest(*arrest).is_some_and(|arrest| {
                arrest.character() == character
                    && arrest.arrested_at() <= information.observed_at()
                    && arrest
                        .released_at()
                        .is_none_or(|released_at| information.observed_at() <= released_at)
            })
        }
        OperationKind::Burglary
        | OperationKind::Robbery
        | OperationKind::Hijacking
        | OperationKind::Smuggling
        | OperationKind::Intimidation
        | OperationKind::Surveillance
        | OperationKind::DocumentTheft
        | OperationKind::GamblingEvent
        | OperationKind::Sabotage
        | OperationKind::Arson => true,
    }
}

pub(crate) fn known_detention_arrest(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    character: CharacterId,
) -> Option<ArrestId> {
    let target = EntityRef::Character(character);
    state
        .intelligence
        .information_for_holder_subject(KnowledgeHolder::Organization(organization), target)
        .filter(|information| {
            source_information_proves_operation_basis(
                registry,
                state,
                OperationKind::Extraction,
                target,
                information,
                state.now(),
            )
        })
        .filter_map(|information| {
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
                arrest,
            })) = information.signal()
            else {
                return None;
            };
            Some((
                information.observed_at(),
                information.recorded_at(),
                information.id(),
                *arrest,
            ))
        })
        .max()
        .map(|(_, _, _, arrest)| arrest)
}

pub(crate) fn known_witness_cases(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    character: CharacterId,
) -> BTreeSet<CaseWitnessId> {
    let target = EntityRef::Character(character);
    state
        .intelligence
        .information_for_holder_subject(KnowledgeHolder::Organization(organization), target)
        .filter(|information| {
            source_information_proves_operation_basis(
                registry,
                state,
                OperationKind::WitnessPressure,
                target,
                information,
                state.now(),
            )
        })
        .filter_map(|information| {
            let Some(InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::CaseWitness {
                investigation,
            })) = information.signal()
            else {
                return None;
            };
            state
                .legal
                .case_witness_for(*investigation, character)
                .map(|witness| witness.id())
        })
        .collect()
}

pub(crate) fn organization_knew_witness_case_at(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    character: CharacterId,
    case_witness: CaseWitnessId,
    at: SimTime,
) -> bool {
    let Some(witness) = state.legal.get_case_witness(case_witness) else {
        return false;
    };
    if witness.witness() != character || witness.registered_at() > at {
        return false;
    }
    let target = EntityRef::Character(character);
    state
        .intelligence
        .information_for_holder_subject(KnowledgeHolder::Organization(organization), target)
        .any(|information| {
            information.recorded_at() <= at
                && matches!(
                    information.signal(),
                    Some(InformationSignal::LegalPersonStatus(
                        LegalPersonStatusSignal::CaseWitness { investigation }
                    )) if *investigation == witness.investigation()
                )
                && source_information_proves_operation_basis(
                    registry,
                    state,
                    OperationKind::WitnessPressure,
                    target,
                    information,
                    at,
                )
        })
}

pub(crate) fn organization_knew_detention_at(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    character: CharacterId,
    arrest: ArrestId,
    at: SimTime,
) -> bool {
    let target = EntityRef::Character(character);
    let Some(arrest_record) = state.legal.get_arrest(arrest) else {
        return false;
    };
    if arrest_record.character() != character
        || arrest_record.arrested_at() > at
        || arrest_record
            .released_at()
            .is_some_and(|released_at| released_at < at)
    {
        return false;
    }
    state
        .intelligence
        .information_for_holder_subject(KnowledgeHolder::Organization(organization), target)
        .any(|information| {
            information.recorded_at() <= at
                && information.observed_at() <= at
                && arrest_record.arrested_at() <= information.observed_at()
                && arrest_record
                    .released_at()
                    .is_none_or(|released_at| information.observed_at() <= released_at)
                && matches!(
                    information.signal(),
                    Some(InformationSignal::LegalPersonStatus(
                        LegalPersonStatusSignal::Detained {
                            arrest: learned_arrest
                        }
                    )) if *learned_arrest == arrest
                )
                && source_information_is_usable_for_operation_basis(
                    registry,
                    OperationKind::Extraction,
                    information,
                    at,
                )
        })
}
