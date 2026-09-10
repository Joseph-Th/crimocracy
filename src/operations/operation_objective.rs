//! Runtime viability of authored operation objectives.
//!
//! Authorization proves that an objective is actionable when leadership approves it. This module
//! owns the smaller, mutable question at execution time: whether the world still contains a
//! practical effect for a tactically successful crew to achieve.

use crate::core::entity::EntityRef;
use crate::core::id::{CaseWitnessId, CharacterId, OrganizationId};
use crate::core::state::AppState;
use crate::legal::{InvestigationStatus, WitnessCooperation};
use crate::operations::{
    OperationBusinessTargetOwnership, OperationKind, OperationObjective, OperationObjectiveBlocker,
    OperationObjectiveOutcome, OperationRecord,
};
use crate::world::BusinessOwner;

/// Effective objective result after applying a practical blocker. A blocker is only authored for
/// a base result that would otherwise achieve something; tactical failure remains tactical
/// failure and therefore carries no redundant practical-failure explanation.
pub(crate) const fn effective_objective_outcome(
    base: OperationObjectiveOutcome,
    blocker: Option<OperationObjectiveBlocker>,
) -> OperationObjectiveOutcome {
    if blocker.is_some() {
        OperationObjectiveOutcome::Failed
    } else {
        base
    }
}

/// Mutable execution-time condition that prevents a non-failed objective from producing its
/// authored effect. The operation record is assumed to have passed structural/authorship
/// validation, so malformed objective shapes are not silently normalized here.
pub(crate) fn resolve_objective_blocker(
    state: &AppState,
    operation: &OperationRecord,
) -> Option<OperationObjectiveBlocker> {
    match operation.objective() {
        OperationObjective::AcquireProperty {
            target: EntityRef::Business(business),
        } => business_target_ownership_mismatch(state, operation, *business)
            .then_some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch),
        OperationObjective::ObtainCash {
            target: EntityRef::Business(business),
        } => business_target_ownership_mismatch(state, operation, *business)
            .then_some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch),
        OperationObjective::DisruptBusiness {
            target: EntityRef::Business(business),
        } => {
            if business_target_ownership_mismatch(state, operation, *business) {
                Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch)
            } else if !state
                .economy
                .get_business_economy(*business)
                .is_some_and(|economy| {
                    economy.status() == crate::economy::BusinessOperatingStatus::Active
                })
            {
                Some(OperationObjectiveBlocker::TargetEconomyInactive)
            } else {
                None
            }
        }
        OperationObjective::Frighten {
            target: EntityRef::Character(character),
        } => pressureable_witness_targets(state, operation.responsible_organization(), *character)
            .is_empty()
            .then_some(OperationObjectiveBlocker::NoPressureableWitnessCase),
        OperationObjective::FreeDetainee { target } => operation
            .extraction_arrest()
            .and_then(|arrest| state.legal.get_arrest(arrest))
            .is_none_or(|arrest| {
                arrest.character() != *target
                    || arrest.status() != crate::legal::ArrestStatus::Detained
            })
            .then_some(OperationObjectiveBlocker::ExtractionCustodyEnded),
        OperationObjective::GatherInformation { .. } => None,
        // These shapes are rejected by operation authorship validation. Keeping the fallback
        // explicit prevents a malformed save from being converted into an invented blocker.
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::DisruptBusiness { .. } => None,
    }
}

pub(crate) fn blocker_clause(
    kind: OperationKind,
    blocker: OperationObjectiveBlocker,
) -> &'static str {
    match blocker {
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch
            if kind == OperationKind::GamblingEvent =>
        {
            "The gambling venue left the sponsoring organization's control before the event could pay out, so there was no authorized house operation left to run."
        }
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
            "The target came under the sponsoring organization's ownership before the crew reached the objective, so taking or damaging it would have meant hitting its own assets."
        }
        OperationObjectiveBlocker::TargetEconomyInactive => {
            "The target was no longer operating when the crew reached the objective, so there was no active business to disrupt."
        }
        OperationObjectiveBlocker::NoPressureableWitnessCase => {
            "By the time the crew reached the witness, no active case still depended on cooperation that intimidation could reduce."
        }
        OperationObjectiveBlocker::ExtractionCustodyEnded => {
            "The target was no longer detained when the crew reached the objective, so there was no custody left to break."
        }
    }
}

pub(crate) fn has_active_foreign_witness_case(
    state: &AppState,
    responsible_organization: OrganizationId,
    character: CharacterId,
) -> bool {
    state
        .legal
        .case_witnesses_for_character(character)
        .any(|witness| {
            active_foreign_case(state, responsible_organization, witness.investigation())
        })
}

pub(crate) fn has_pressureable_witness_case(
    state: &AppState,
    responsible_organization: OrganizationId,
    character: CharacterId,
) -> bool {
    !pressureable_witness_targets(state, responsible_organization, character).is_empty()
}

/// Exact witness registrations whose future cooperation can still be reduced. Existing testimony
/// stores the cooperation snapshot used when it was recorded, so intimidating a statemented
/// witness cannot retroactively weaken that evidence and has no remaining modeled effect.
pub(crate) fn pressureable_witness_targets(
    state: &AppState,
    responsible_organization: OrganizationId,
    character: CharacterId,
) -> Vec<(CaseWitnessId, WitnessCooperation)> {
    state
        .legal
        .case_witnesses_for_character(character)
        .filter(|witness| witness.witness() == character)
        .filter(|witness| {
            active_foreign_case(state, responsible_organization, witness.investigation())
                && witness.statements().is_empty()
                && witness.cooperation() != WitnessCooperation::Hostile
        })
        .map(|witness| (witness.id(), witness.cooperation()))
        .collect()
}

fn active_foreign_case(
    state: &AppState,
    responsible_organization: OrganizationId,
    investigation: crate::core::id::InvestigationId,
) -> bool {
    state
        .legal
        .get_investigation(investigation)
        .is_some_and(|record| {
            record.status() == InvestigationStatus::Active
                && record.owner() != responsible_organization
        })
}

fn business_target_ownership_mismatch(
    state: &AppState,
    operation: &OperationRecord,
    business: crate::core::id::BusinessId,
) -> bool {
    let Some(record) = state.world.get_business(business) else {
        return false;
    };
    let sponsor = BusinessOwner::Organization(operation.responsible_organization());
    match operation
        .kind()
        .business_target_ownership()
        .expect("business-target operation must define intrinsic ownership semantics")
    {
        OperationBusinessTargetOwnership::Foreign => record.owner() == sponsor,
        OperationBusinessTargetOwnership::SponsorOwned => record.owner() != sponsor,
    }
}

/// Exhaustiveness canary used by registry/state validation when checking persisted blockers.
pub(crate) fn blocker_matches_objective(
    operation: &OperationRecord,
    blocker: OperationObjectiveBlocker,
) -> bool {
    match blocker {
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
            operation.kind().business_target_ownership().is_some()
                && matches!(
                    operation.objective(),
                    OperationObjective::AcquireProperty {
                        target: EntityRef::Business(_)
                    } | OperationObjective::ObtainCash {
                        target: EntityRef::Business(_)
                    } | OperationObjective::DisruptBusiness {
                        target: EntityRef::Business(_)
                    }
                )
        }
        OperationObjectiveBlocker::TargetEconomyInactive => matches!(
            (operation.kind(), operation.objective()),
            (
                OperationKind::Sabotage | OperationKind::Arson,
                OperationObjective::DisruptBusiness {
                    target: EntityRef::Business(_)
                }
            )
        ),
        OperationObjectiveBlocker::NoPressureableWitnessCase => matches!(
            (operation.kind(), operation.objective()),
            (
                OperationKind::WitnessPressure,
                OperationObjective::Frighten {
                    target: EntityRef::Character(_)
                }
            )
        ),
        OperationObjectiveBlocker::ExtractionCustodyEnded => matches!(
            (operation.kind(), operation.objective()),
            (
                OperationKind::Extraction,
                OperationObjective::FreeDetainee { .. }
            )
        ),
    }
}
