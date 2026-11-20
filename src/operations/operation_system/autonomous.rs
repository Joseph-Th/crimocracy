//! Rival commercial pressure through ordinary operation authorization.
//!
//! A rival considers local competing storefronts from the same public directory the player
//! uses, not hidden foreign holdings, treasury balances, or live investigations. It first
//! scouts a venue, then follows up on its own fresh observation. Its pressure ends when
//! trained personnel, operational authority, or a viable target are no longer available.
//! There is no separate mission, damage, or randomness path.

use super::{
    ValidatedOperation, is_actionable_opportunity_target, participant_available_for_plan,
    planning_information, validate_authorize_operation,
};
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessId, CharacterId, IdKind, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimDuration, SimTime, is_day_boundary};
use crate::delegation::delegation_system::resolve_mandate_authority;
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::intelligence::{InformationSourceKind, InformationTopic, KnowledgeHolder};
use crate::operations::operation_intelligence::resolve_information_score;
use crate::operations::{
    ACTIVE_ASSIGNMENT_STATUSES, OperationApproach, OperationDraft, OperationKind,
    OperationObjective, OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::reputation::{AudienceKind, reputation_system::resolve_score};
use crate::world::{AutonomyLevel, BusinessKind, OrganizationKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// A failed watch or strike is not retried immediately. Successful strikes can be followed
/// up on the next day while the same observations are useful, so a concerted campaign can
/// actually close a vulnerable storefront rather than merely perturb one settlement.
const UNSUCCESSFUL_RETRY_MINUTES: u64 = 3 * DAY_MINUTES;

fn eligible_managers(state: &AppState) -> BTreeMap<OrganizationId, Vec<CharacterId>> {
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Operations);
    let mut by_org = BTreeMap::<_, Vec<_>>::new();
    for mandate in state.delegation().active_for_scope(scope) {
        let organization = mandate.organization();
        if state.player_organization() == Some(organization)
            || state
                .world()
                .get_organization(organization)
                .is_none_or(|org| org.kind() != OrganizationKind::Criminal)
        {
            continue;
        }
        let manager = mandate.manager();
        if state.world().get_character(manager).is_none_or(|member| {
            !matches!(
                member.autonomy(),
                AutonomyLevel::Delegated | AutonomyLevel::Broad
            )
        }) || resolve_mandate_authority(
            state,
            MandateAuthority {
                mandate: mandate.id(),
                manager,
                scope,
            },
        )
        .is_err()
        {
            continue;
        }
        by_org.entry(organization).or_default().push(manager);
    }
    for managers in by_org.values_mut() {
        managers.sort_unstable();
        managers.dedup();
    }
    by_org
}

/// Pick a trained roster, allowing the responsible manager to coordinate the strike and the
/// scout to lead the watch. Use the same booking predicate and final validator as player plans.
/// The two supported kinds have disjoint required skill mappings.
fn crew(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
    manager: CharacterId,
    kind: OperationKind,
    scheduled_for: SimTime,
) -> Option<(CharacterId, BTreeMap<RoleKind, CharacterId>)> {
    let definition = registry.get_operation(kind);
    let execution = definition.execution();
    if !participant_available_for_plan(registry, state, manager, kind, scheduled_for, &[]) {
        return None;
    }
    let mut used = BTreeSet::new();
    let mut roles = BTreeMap::new();
    for role in definition.required_roles() {
        let capability = execution.capability_for_role(*role)?;
        let participant = state
            .world()
            .characters_in_organization(organization)
            .filter(|p| !used.contains(&p.id()))
            .filter(|p| *role != RoleKind::Coordinator || p.id() == manager)
            .filter(|p| {
                *role == RoleKind::Coordinator
                    || kind == OperationKind::Surveillance
                    || p.id() != manager
            })
            .filter(|p| {
                participant_available_for_plan(registry, state, p.id(), kind, scheduled_for, &[])
            })
            .filter_map(|p| {
                p.capability(capability)
                    .filter(|rating| rating.value() > 0)
                    .map(|rating| (rating.value(), Reverse(p.id())))
            })
            .max()?
            .1
            .0;
        used.insert(participant);
        roles.insert(*role, participant);
    }
    let leader = if kind == OperationKind::Surveillance {
        *roles.get(&RoleKind::Surveillance)?
    } else {
        manager
    };
    Some((leader, roles))
}

fn choose_plan<'r>(
    registry: &'r Registry,
    state: &AppState,
    organization: OrganizationId,
    managers: &[CharacterId],
    scheduled_for: SimTime,
) -> Option<ValidatedOperation<'r>> {
    if resolve_score(
        registry,
        state.reputation(),
        organization,
        AudienceKind::Police,
    ) >= registry.reputation().expansion_police_fear_ceiling()
    {
        return None;
    }
    // This is self-knowledge plus public directory information. A business need not be
    // known to belong to a criminal competitor to compete for the same local customers.
    let mut sectors = BTreeMap::<(_, BusinessKind), usize>::new();
    let mut owned = BTreeSet::new();
    for business in state.world().businesses_owned_by_organization(organization) {
        owned.insert(business.id());
        if state
            .economy()
            .get_business_economy(business.id())
            .is_some_and(|economy| {
                economy.status() == crate::economy::BusinessOperatingStatus::Active
            })
        {
            *sectors
                .entry((business.neighborhood(), business.kind()))
                .or_default() += 1;
        }
    }
    if sectors.is_empty() {
        return None;
    }

    let mut last = BTreeMap::<(BusinessId, OperationKind), _>::new();
    for operation in state.operations().operations_for_organization(organization) {
        if ACTIVE_ASSIGNMENT_STATUSES.contains(&operation.status()) {
            return None; // one strategic operation at a time per outfit
        }
        let Some(target) = operation.objective().business_target() else {
            continue;
        };
        if !matches!(
            operation.kind(),
            OperationKind::Surveillance | OperationKind::Sabotage
        ) {
            continue;
        }
        let success = operation.status() == OperationStatus::Completed
            && operation.resolution().is_some_and(|resolution| {
                resolution.objective_outcome()
                    != crate::operations::OperationObjectiveOutcome::Failed
            });
        let ended_at = operation
            .resolution()
            .map(|resolution| resolution.resolved_at())
            .or_else(|| operation.abort_record().map(|abort| abort.aborted_at()))
            .expect("terminal operation must record its end instant");
        let entry = last
            .entry((target, operation.kind()))
            .or_insert((ended_at, success));
        if ended_at > entry.0 {
            *entry = (ended_at, success);
        }
    }

    let age_limit = u64::from(
        registry
            .get_operation(OperationKind::Surveillance)
            .execution()
            .max_intelligence_age()
            .as_minutes(),
    );
    let mut candidates = Vec::new();
    for (id, name, kind, district) in state.world().public_business_directory() {
        let Some(stake) = sectors.get(&(district, kind)) else {
            continue;
        };
        if owned.contains(&id)
            || !is_actionable_opportunity_target(
                registry,
                state,
                organization,
                OperationKind::Sabotage,
                EntityRef::Business(id),
            )
        {
            continue;
        }
        let observed = state
            .intelligence()
            .information_for_holder_subject(
                KnowledgeHolder::Organization(organization),
                EntityRef::Business(id),
            )
            .any(|info| {
                info.topic() == InformationTopic::MarketAccess
                    && info.source_kind() == InformationSourceKind::Surveillance
                    && info.observed_at() <= scheduled_for
                    && resolve_information_score(
                        registry.information_quality(),
                        info,
                        scheduled_for,
                        age_limit,
                    ) > 0
            });
        let kind = if observed {
            OperationKind::Sabotage
        } else {
            OperationKind::Surveillance
        };
        let attempt = last.get(&(id, kind));
        if attempt.is_some_and(|(time, success)| {
            !success
                && state.now().as_minutes().saturating_sub(time.as_minutes())
                    < UNSUCCESSFUL_RETRY_MINUTES
        }) {
            continue;
        }
        // A scouted target is the tactical priority. Prefer economic stakes, then a
        // previously successful strike to maintain pressure, then stable public identity.
        let follow_up = last
            .get(&(id, OperationKind::Sabotage))
            .is_some_and(|(time, success)| {
                *success && state.now().as_minutes().saturating_sub(time.as_minutes()) <= age_limit
            });
        candidates.push((!observed, Reverse(*stake), !follow_up, id, name, kind));
    }
    candidates.sort_unstable_by_key(|item| (item.0, item.1, item.2, item.3));
    for (_, _, _, id, name, kind) in candidates {
        let target = EntityRef::Business(id);
        let objective = if kind == OperationKind::Surveillance {
            OperationObjective::GatherInformation { target }
        } else {
            debug_assert_eq!(kind, OperationKind::Sabotage);
            OperationObjective::DisruptBusiness { target }
        };
        for manager in managers {
            let Some((leader, roles)) =
                crew(registry, state, organization, *manager, kind, scheduled_for)
            else {
                continue;
            };
            let intelligence = planning_information(
                registry,
                state,
                organization,
                kind,
                &objective,
                scheduled_for,
            )
            .into_values()
            .map(|(id, _)| id)
            .collect();
            let draft = OperationDraft {
                title: format!("{}: {name}", registry.get_operation(kind).display_name()),
                kind,
                responsible_organization: organization,
                leader,
                objective: objective.clone(),
                approach: OperationApproach::Covert,
                roles,
                intelligence,
                constraints: Vec::new(),
                // NPCs must not create unresolved player-style leadership decisions.
                // Normal exposure, police arrivals and criminal evidence still apply.
                contingencies: Vec::new(),
                scheduled_for,
            };
            if let Ok(plan) = validate_authorize_operation(registry, state, draft) {
                return Some(plan);
            }
        }
    }
    None
}

/// Admit a complete same-boundary cohort or none if operation IDs are exhausted. Each outfit
/// has distinct personnel, and authorizing does not mutate business state or other plans.
pub(crate) fn apply_due_rival_operations(
    registry: &Registry,
    state: &mut AppState,
) -> Vec<crate::core::id::OperationId> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let Some(scheduled_for) = state.now().checked_add(SimDuration::ONE_MINUTE) else {
        return Vec::new();
    };
    let plans: Vec<_> = eligible_managers(state)
        .into_iter()
        .filter_map(|(organization, managers)| {
            choose_plan(registry, state, organization, &managers, scheduled_for)
        })
        .collect();
    let count = u32::try_from(plans.len()).expect("organization count fits ID capacity");
    if state
        .ids
        .reserve_many(&[(IdKind::Operation, count)])
        .is_err()
    {
        return Vec::new();
    }
    plans
        .into_iter()
        .map(|plan| {
            plan.commit(state)
                .expect("independent rival plans remain valid within the preflighted cohort")
        })
        .collect()
}

#[cfg(test)]
mod tests;
