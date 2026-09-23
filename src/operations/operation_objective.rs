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

/// A person who is the direct subject of the operation. These objectives require the crew and
/// target to be distinct people: surveillance, coercion, and extraction cannot meaningfully use
/// the subject as one of the actors carrying out the same operation.
pub(crate) const fn character_objective_target(
    objective: &OperationObjective,
) -> Option<CharacterId> {
    match objective {
        OperationObjective::GatherInformation {
            target: EntityRef::Character(character),
        }
        | OperationObjective::Frighten {
            target: EntityRef::Character(character),
        } => Some(*character),
        OperationObjective::FreeDetainee { target } => Some(*target),
        OperationObjective::AcquireProperty { .. }
        | OperationObjective::GatherInformation { .. }
        | OperationObjective::ObtainCash { .. }
        | OperationObjective::Frighten { .. }
        | OperationObjective::DisruptBusiness { .. } => None,
    }
}

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
    if operation.kind().business_target_ownership().is_some()
        && let Some(business) = operation.objective().business_target()
        && business_target_ownership_mismatch(state, operation, business)
    {
        return Some(OperationObjectiveBlocker::TargetBusinessOwnershipMismatch);
    }
    match operation.objective() {
        OperationObjective::AcquireProperty {
            target: EntityRef::Business(_),
        }
        | OperationObjective::GatherInformation {
            target: EntityRef::Business(_),
        } => None,
        OperationObjective::ObtainCash {
            target: EntityRef::Business(business),
        } => state
            .economy
            .get_business_economy(*business)
            .is_some_and(|economy| {
                economy.status() != crate::economy::BusinessOperatingStatus::Active
            })
            .then_some(OperationObjectiveBlocker::TargetEconomyInactive),
        OperationObjective::DisruptBusiness {
            target: EntityRef::Business(business),
        } => {
            if !state
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
        } => pressureable_witness_targets_for_cases(
            state,
            operation.responsible_organization(),
            *character,
            operation.witness_pressure_cases(),
        )
        .is_empty()
        .then_some(OperationObjectiveBlocker::NoPressureableWitnessCase),
        OperationObjective::FreeDetainee { target } => {
            let arrest_id = operation
                .extraction_arrest()
                .expect("validated extraction operation must retain its pinned arrest");
            let arrest = state
                .legal
                .get_arrest(arrest_id)
                .expect("validated extraction operation must reference a persisted arrest");
            (arrest.character() != *target
                || arrest.status() != crate::legal::ArrestStatus::Detained)
                .then_some(OperationObjectiveBlocker::ExtractionCustodyEnded)
        }
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
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch
            if kind == OperationKind::DocumentTheft =>
        {
            "The target business came under the sponsoring organization's ownership before the crew reached the records, so stealing its own files would no longer acquire outside information."
        }
        OperationObjectiveBlocker::TargetBusinessOwnershipMismatch => {
            "The target came under the sponsoring organization's ownership before the crew reached the objective, so taking or damaging it would have meant hitting its own assets."
        }
        OperationObjectiveBlocker::TargetEconomyInactive if operation_cash_kind(kind) => {
            "The target was no longer operating when the crew reached the objective, so there was no active cash-generating business to collect from."
        }
        OperationObjectiveBlocker::TargetEconomyInactive => {
            "The target was no longer operating when the crew reached the objective, so there was no active business to disrupt."
        }
        OperationObjectiveBlocker::NoPressureableWitnessCase => {
            "By the time the crew reached the witness, there was no witness cooperation the crew could still affect."
        }
        OperationObjectiveBlocker::ExtractionCustodyEnded => {
            "The target was no longer detained when the crew reached the objective, so there was no custody left to break."
        }
    }
}

/// Exact witness registrations whose future cooperation can still be reduced by a field
/// intimidation. Existing testimony stores the cooperation snapshot used when it was recorded,
/// and police custody makes the character physically unavailable, so neither state has a
/// remaining modeled pressure effect.
pub(crate) fn pressureable_witness_targets_for_cases(
    state: &AppState,
    responsible_organization: OrganizationId,
    character: CharacterId,
    cases: &std::collections::BTreeSet<CaseWitnessId>,
) -> Vec<(CaseWitnessId, WitnessCooperation)> {
    // A detained witness is not physically available to a field intimidation operation.
    // If custody begins after authorization, this same predicate feeds the existing
    // NoPressureableWitnessCase execution blocker, so the operation fails practically rather
    // than mutating cooperation through police custody.
    if state.legal.active_arrest_for_character(character).is_some() {
        return Vec::new();
    }
    state
        .legal
        .case_witnesses_for_character(character)
        .filter(|witness| cases.contains(&witness.id()))
        .filter(|witness| witness.witness() == character)
        .filter(|witness| {
            active_foreign_case(state, responsible_organization, witness.investigation())
                && witness.statements().is_empty()
                && witness.cooperation() != WitnessCooperation::Hostile
                && !crate::legal::witness_system::case_witness_is_case_subject(state, witness)
        })
        .map(|witness| (witness.id(), witness.cooperation()))
        .collect()
}

fn active_foreign_case(
    state: &AppState,
    responsible_organization: OrganizationId,
    investigation: crate::core::id::InvestigationId,
) -> bool {
    let record = state
        .legal
        .get_investigation(investigation)
        .expect("persisted case witness must reference a persisted investigation");
    record.status() == InvestigationStatus::Active && record.owner() != responsible_organization
}

fn business_target_ownership_mismatch(
    state: &AppState,
    operation: &OperationRecord,
    business: crate::core::id::BusinessId,
) -> bool {
    let record = state
        .world
        .get_business(business)
        .expect("validated business objective must reference a persisted business");
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
                && operation.objective().business_target().is_some()
        }
        OperationObjectiveBlocker::TargetEconomyInactive => matches!(
            operation.objective(),
            OperationObjective::ObtainCash {
                target: EntityRef::Business(_)
            } | OperationObjective::DisruptBusiness {
                target: EntityRef::Business(_)
            }
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

const fn operation_cash_kind(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::Robbery
            | OperationKind::Smuggling
            | OperationKind::Intimidation
            | OperationKind::GamblingEvent
    )
}
