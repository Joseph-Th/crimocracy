//! Operation-objective shape, liveness, and practical-actionability validation.

use super::OperationError;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OperationId, OrganizationId};
use crate::core::state::AppState;
use crate::enterprises::EnterpriseStatus;
use crate::intelligence::KnowledgeHolder;
use crate::operations::operation_objective::{
    has_active_foreign_witness_case, has_pressureable_witness_case,
};
use crate::operations::{
    ACTIVE_ASSIGNMENT_STATUSES, OperationBusinessTargetOwnership, OperationKind,
    OperationObjective, OperationObjectiveKind,
};
use crate::registry::Registry;
use crate::world::BusinessOwner;

pub(crate) fn is_information_subject_relevant(
    state: &AppState,
    objective: &OperationObjective,
    subject: EntityRef,
) -> bool {
    let referenced = objective.referenced_entities();
    if referenced.contains(&subject) {
        return true;
    }
    let EntityRef::Neighborhood(subject_neighborhood) = subject else {
        return false;
    };
    referenced.into_iter().any(|entity| match entity {
        EntityRef::Business(business) => state
            .world
            .get_business(business)
            .is_some_and(|record| record.neighborhood() == subject_neighborhood),
        EntityRef::Neighborhood(neighborhood) => neighborhood == subject_neighborhood,
        EntityRef::Organization(_)
        | EntityRef::Character(_)
        | EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_)
        | EntityRef::Enterprise(_) => false,
    })
}

/// Whether the objective's target has a shape the objective could ever act on, independent of
/// operation kind. Used only to choose the authorization error variant.
fn is_plausible_objective_target(objective: &OperationObjective) -> bool {
    match objective {
        OperationObjective::AcquireProperty { target }
        | OperationObjective::ObtainCash { target }
        | OperationObjective::DisruptBusiness { target } => {
            matches!(target, EntityRef::Business(_))
        }
        OperationObjective::Frighten { target } => matches!(target, EntityRef::Character(_)),
        // Surveillance targets and extraction detainees are validated by their own gates.
        OperationObjective::GatherInformation { .. } | OperationObjective::FreeDetainee { .. } => {
            true
        }
    }
}

pub(crate) fn is_valid_operation_objective(
    kind: OperationKind,
    objective: &OperationObjective,
) -> bool {
    if kind.objective_kind() != objective.kind() {
        return false;
    }
    match objective {
        OperationObjective::AcquireProperty { target } => matches!(target, EntityRef::Business(_)),
        OperationObjective::GatherInformation { target } => {
            crate::operations::surveillance_integration::is_supported_surveillance_target(*target)
        }
        OperationObjective::ObtainCash { target }
        | OperationObjective::DisruptBusiness { target } => {
            matches!(target, EntityRef::Business(_))
        }
        OperationObjective::Frighten { target } => matches!(target, EntityRef::Character(_)),
        OperationObjective::FreeDetainee { .. } => true,
    }
}

/// Opportunity discovery uses the exact current operation target contract without needing crew,
/// approach, scheduling, or intelligence details that do not exist until an operation is planned.
/// Contextual opportunity targets may fail this predicate; the opportunity owner requires only one
/// currently actionable target in the discovered set.
pub(crate) fn is_actionable_opportunity_target(
    registry: &Registry,
    state: &AppState,
    responsible_organization: OrganizationId,
    kind: OperationKind,
    target: EntityRef,
) -> bool {
    let Some(objective) = kind.objective_for_target(target) else {
        return false;
    };
    validate_operation_objective(registry, state, kind, responsible_organization, &objective)
        .is_ok()
}

// Field actions may reference concrete world subjects and locations, never control-plane
// records such as operations, investigations, evidence, accounts, decisions, or mandates.
// Historical intelligence and after-action records may refer to inactive entities, so the
// general `is_entity_present` check remains existence-only. Action objectives have a stricter
// contract: their concrete world subjects must still be actionable when the operation is
// authorized.
fn validate_active_field_objective_targets(
    registry: &Registry,
    state: &AppState,
    responsible_organization: OrganizationId,
    kind: OperationKind,
    objective: &OperationObjective,
) -> Result<(), OperationError> {
    match objective {
        OperationObjective::ObtainCash { target } => {
            validate_active_field_objective_target(state, *target)?;
            validate_business_target_requirement(
                registry,
                state,
                responsible_organization,
                kind,
                *target,
            )
        }
        // Witness pressure is only meaningful against a character who is actually a named
        // witness on an active case run by another authority; anything else would resolve
        // with nothing to coerce.
        OperationObjective::Frighten { target } => {
            validate_active_field_objective_target(state, *target)?;
            let EntityRef::Character(character) = *target else {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: OperationObjectiveKind::Frighten,
                    target: *target,
                });
            };
            if !has_active_foreign_witness_case(state, responsible_organization, character) {
                return Err(OperationError::TargetNotCaseWitness(character));
            }
            if !has_pressureable_witness_case(state, responsible_organization, character) {
                return Err(OperationError::TargetNotPressureableWitness(character));
            }
            Ok(())
        }
        // Property and sabotage targets are businesses whose premises the crew acts on.
        // Taking value out of, or disrupting, the organization's own premises would enrich
        // or disrupt itself, so authorization rejects self-targeting up front.
        OperationObjective::AcquireProperty { target } => {
            let EntityRef::Business(_) = *target else {
                return Ok(());
            };
            validate_active_field_objective_target(state, *target)?;
            validate_business_target_requirement(
                registry,
                state,
                responsible_organization,
                kind,
                *target,
            )
        }
        OperationObjective::GatherInformation { target } => {
            validate_surveillance_target_knowledge(state, responsible_organization, *target)
        }
        // Sabotage targets a business whose premises the crew must physically reach, and one
        // that actually operates. Disrupting a shuttered or economy-less storefront would have
        // no modeled effect, so authorization rejects it up front.
        OperationObjective::DisruptBusiness { target } => {
            validate_active_field_objective_target(state, *target)?;
            validate_business_target_requirement(
                registry,
                state,
                responsible_organization,
                kind,
                *target,
            )?;
            let EntityRef::Business(business) = *target else {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: objective.kind(),
                    target: *target,
                });
            };
            let economy = state
                .economy
                .get_business_economy(business)
                .ok_or(OperationError::TargetWithoutOperatingEconomy(business))?;
            if economy.status() != crate::economy::BusinessOperatingStatus::Active {
                return Err(OperationError::TargetWithoutOperatingEconomy(business));
            }
            Ok(())
        }
        // Extraction targets a person who may legitimately be in custody, so the usual
        // active-field-target check does not apply; custody state is validated separately.
        OperationObjective::FreeDetainee { target } => {
            let _ = state
                .world
                .get_character(*target)
                .ok_or(OperationError::MissingEntity(EntityRef::Character(*target)))?;
            if state.legal.active_arrest_for_character(*target).is_none() {
                return Err(OperationError::TargetNotDetained(*target));
            }
            // One detainee, one live extraction plan. A second non-terminal extraction against
            // the same custody could only arrive after the first freed the target.
            if let Some(operation) = find_non_terminal_extraction_targeting(state, *target) {
                return Err(OperationError::DetaineeAlreadyTargeted {
                    character: *target,
                    operation,
                });
            }
            Ok(())
        }
    }
}

/// Administrative runtime records are not ambient world knowledge. A crew may directly surveil
/// its own operation or enterprise, and an authority may surveil its own case; otherwise the
/// responsible organization must already hold some information naming that exact record. Treat an
/// unknown-but-real administrative target as missing so authorization cannot be used as an
/// existence oracle for hidden state.
fn validate_surveillance_target_knowledge(
    state: &AppState,
    organization: OrganizationId,
    target: EntityRef,
) -> Result<(), OperationError> {
    let intrinsically_known = match target {
        EntityRef::Operation(operation) => state
            .operations
            .get_operation(operation)
            .is_some_and(|record| record.responsible_organization() == organization),
        EntityRef::Investigation(investigation) => state
            .legal
            .get_investigation(investigation)
            .is_some_and(|record| record.owner() == organization),
        EntityRef::Enterprise(enterprise) => state
            .enterprises
            .get_enterprise(enterprise)
            .is_some_and(|record| record.organization() == organization),
        EntityRef::Organization(_)
        | EntityRef::Character(_)
        | EntityRef::Neighborhood(_)
        | EntityRef::Business(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => return Ok(()),
    };
    if intrinsically_known
        || state
            .intelligence
            .information_for_holder_subject(KnowledgeHolder::Organization(organization), target)
            .next()
            .is_some()
    {
        Ok(())
    } else {
        Err(OperationError::MissingEntity(target))
    }
}

/// Smallest non-terminal operation whose objective extracts the given detainee, if any.
/// Status buckets are scanned in fixed order and ties break on operation id.
fn find_non_terminal_extraction_targeting(
    state: &AppState,
    target_character: CharacterId,
) -> Option<OperationId> {
    ACTIVE_ASSIGNMENT_STATUSES
        .iter()
        .flat_map(|status| state.operations.operations_with_status(*status))
        .filter(|operation| {
            matches!(
                operation.objective(),
                OperationObjective::FreeDetainee { target } if *target == target_character
            )
        })
        .map(|operation| operation.id())
        .min()
}

/// Applies the operation definition's one authoritative business-target contract. Ownership and
/// venue capability are checked together so authorization cannot satisfy one half of the target
/// semantics while bypassing the other.
fn validate_business_target_requirement(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    kind: OperationKind,
    target: EntityRef,
) -> Result<(), OperationError> {
    let EntityRef::Business(business) = target else {
        return Ok(());
    };
    let record = state
        .world
        .get_business(business)
        .ok_or(OperationError::MissingEntity(target))?;
    let requirement = registry
        .get_operation(kind)
        .execution()
        .business_target()
        .expect("business-target operation kind must retain its validated target definition");
    let sponsor = BusinessOwner::Organization(organization);
    match kind
        .business_target_ownership()
        .expect("business-target operation kind must define intrinsic ownership semantics")
    {
        OperationBusinessTargetOwnership::Foreign if record.owner() == sponsor => {
            return Err(OperationError::SelfTargetedBusiness { business });
        }
        OperationBusinessTargetOwnership::SponsorOwned if record.owner() != sponsor => {
            return Err(OperationError::TargetBusinessNotSponsorOwned { business });
        }
        OperationBusinessTargetOwnership::Foreign
        | OperationBusinessTargetOwnership::SponsorOwned => {}
    }
    if let Some(function) = requirement
        .required_functions()
        .iter()
        .find(|function| !record.has_function(**function))
    {
        return Err(OperationError::TargetBusinessMissingFunction {
            business,
            function: *function,
        });
    }
    Ok(())
}

fn validate_active_field_objective_target(
    state: &AppState,
    target: EntityRef,
) -> Result<(), OperationError> {
    match target {
        EntityRef::Organization(id) => {
            state
                .world
                .get_organization(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Character(id) => {
            state
                .world
                .get_character(id)
                .ok_or(OperationError::MissingEntity(target))?;
            // Ordinary field objectives assume the target is physically available in the
            // world. Extraction deliberately bypasses this helper because custody is the
            // precondition for that objective rather than an availability failure.
            if state.legal.active_arrest_for_character(id).is_some() {
                return Err(OperationError::InactiveObjectiveTarget(target));
            }
        }
        EntityRef::Neighborhood(id) => {
            state
                .world
                .get_neighborhood(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Business(id) => {
            state
                .world
                .get_business(id)
                .ok_or(OperationError::MissingEntity(target))?;
        }
        EntityRef::Enterprise(id) => {
            let active = state
                .enterprises
                .get_enterprise(id)
                .ok_or(OperationError::MissingEntity(target))?
                .status()
                == EnterpriseStatus::Active;
            if !active {
                return Err(OperationError::InactiveObjectiveTarget(target));
            }
        }
        // Control-plane records never reach this match: `is_valid_operation_objective` rejects
        // them before activation checks run, so reaching this arm means a caller skipped that gate.
        EntityRef::Operation(_)
        | EntityRef::Investigation(_)
        | EntityRef::Evidence(_)
        | EntityRef::FinancialAccount(_)
        | EntityRef::DecisionRequest(_)
        | EntityRef::Mandate(_) => {
            unreachable!(
                "administrative objective targets are rejected by is_valid_operation_objective"
            )
        }
    }
    Ok(())
}

pub(super) fn validate_operation_objective(
    registry: &Registry,
    state: &AppState,
    kind: OperationKind,
    responsible_organization: OrganizationId,
    objective: &OperationObjective,
) -> Result<(), OperationError> {
    if let OperationObjective::AcquireProperty { target } = objective {
        if !kind.can_acquire_property() {
            return Err(OperationError::InvalidObjectiveForKind {
                kind,
                objective: OperationObjectiveKind::AcquireProperty,
            });
        }
        let EntityRef::Business(business) = target else {
            return Err(OperationError::InvalidPropertyTarget(*target));
        };
        let business_record = state
            .world
            .get_business(*business)
            .ok_or(OperationError::MissingEntity(*target))?;
        state
            .world
            .get_neighborhood(business_record.neighborhood())
            .ok_or(OperationError::MissingEntity(EntityRef::Neighborhood(
                business_record.neighborhood(),
            )))?;
    }
    if !is_valid_operation_objective(kind, objective) {
        // A rejection here is either a kind/objective mismatch or a target shape the objective
        // can never act on. Only an implausible target shape reports the target.
        if !is_plausible_objective_target(objective) {
            let invalid_target = match objective {
                OperationObjective::ObtainCash { target }
                | OperationObjective::Frighten { target }
                | OperationObjective::DisruptBusiness { target } => Some(*target),
                OperationObjective::AcquireProperty { .. }
                | OperationObjective::GatherInformation { .. }
                | OperationObjective::FreeDetainee { .. } => None,
            };
            if let Some(target) = invalid_target {
                return Err(OperationError::InvalidObjectiveTarget {
                    objective: objective.kind(),
                    target,
                });
            }
        }
        return Err(OperationError::InvalidObjectiveForKind {
            kind,
            objective: objective.kind(),
        });
    }
    validate_active_field_objective_targets(
        registry,
        state,
        responsible_organization,
        kind,
        objective,
    )
}
