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
use crate::decisions::decision_system::{recently_refused_protection, rival_pressure_blocked};
use crate::delegation::delegation_system::resolve_mandate_authority;
use crate::delegation::{MandateAuthority, ResponsibilityFunction, ResponsibilityScope};
use crate::economy::business_economy_system::{
    forecast_competitor_disruption_gain, resolve_business_gross_potential,
};
use crate::intelligence::{InformationSourceKind, InformationTopic, KnowledgeHolder};
use crate::operations::operation_intelligence::resolve_information_score;
use crate::operations::{
    ACTIVE_ASSIGNMENT_STATUSES, OperationApproach, OperationDraft, OperationKind,
    OperationObjective, OperationStatus, RoleKind,
};
use crate::registry::Registry;
use crate::reputation::{AudienceKind, reputation_system::resolve_score};
use crate::world::organization_pressure::public_prominence_tier;
use crate::world::territory_conflict::{DEFENSIVE_ADVANTAGE, district_of_racket};
use crate::world::{AutonomyLevel, BusinessKind, BusinessOwner, OrganizationKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// A failed watch or strike is not retried immediately. Two successful strikes
/// against a small owner (four against a prominent one) constitute a concerted
/// campaign; reassess other opportunities before repeatedly committing the
/// same scarce crew to the same storefront.
const UNSUCCESSFUL_RETRY_MINUTES: u64 = 3 * DAY_MINUTES;
const CAMPAIGN_REVIEW_MINUTES: u64 = 7 * DAY_MINUTES;

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

/// Criminal storefront owners cannot shed every local racket and retain its protection for
/// free. A rival with an operating racket or a substantial paid named crew can send
/// personnel to pressure an under-defended shop, even without a competing local business.
/// A local foothold carries full strength; projecting from elsewhere costs one associate.
/// The attacker estimates the defender's visible baseline crews, never its secret extra hires
/// or private books. Actual sabotage still requires scouting, an authorized operation, and
/// successful execution under ordinary police and business-economy rules.
fn predatory_stake(
    registry: &Registry,
    state: &AppState,
    attacker: OrganizationId,
    target: BusinessId,
    owner: BusinessOwner,
    force_by_district: &BTreeMap<crate::core::id::NeighborhoodId, u32>,
    strongest_force: u32,
) -> i128 {
    let BusinessOwner::Organization(defender) = owner else {
        return 0;
    };
    if defender == attacker
        || state
            .world()
            .get_organization(defender)
            .is_none_or(|organization| organization.kind() != OrganizationKind::Criminal)
    {
        return 0;
    }
    let Some(shop) = state.world().get_business(target) else {
        return 0;
    };
    let district = shop.neighborhood();
    let local_force = force_by_district.get(&district).copied().unwrap_or(0);
    let projected_force = local_force.max(strongest_force.saturating_sub(1));
    let visible_defense: u32 = state
        .enterprises()
        .active_for_organization(defender)
        .filter(|racket| district_of_racket(state, racket.location()) == Some(district))
        .map(|racket| u32::from(racket.kind().baseline_associates()))
        .sum();
    if projected_force <= visible_defense.saturating_add(DEFENSIVE_ADVANTAGE) {
        return 0;
    }
    // A public venue's normal gross, not its confidential ledgers, measures how much
    // hostile disruption can cost a competitor. Local enforcers prefer close targets.
    let gross = resolve_business_gross_potential(registry, state, target)
        .map(|value| value.cents().max(0))
        .unwrap_or(0);
    i128::from(gross) * if local_force > 0 { 2 } else { 1 } / 2
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
    let mut sectors = BTreeMap::<_, Vec<BusinessId>>::new();
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
            sectors
                .entry(business.neighborhood())
                .or_default()
                .push(business.id());
        }
    }
    let mut force_by_district = BTreeMap::new();
    for racket in state.enterprises().active_for_organization(organization) {
        if let Some(district) = district_of_racket(state, racket.location()) {
            *force_by_district.entry(district).or_insert(0_u32) += u32::from(racket.associates());
        }
    }
    // A gang whose rackets are temporarily closed can still send its *named* paid
    // organization against a competitor. Do not require a profitable racket before
    // a rival is capable of hostile operations; detained members contribute nothing.
    // Actual scouting and sabotage still require available trained characters.
    let named_force = state
        .world()
        .characters_in_organization(organization)
        .filter(|member| {
            state
                .legal()
                .active_arrest_for_character(member.id())
                .is_none()
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let strongest_force = force_by_district
        .values()
        .copied()
        .max()
        .unwrap_or(0)
        .max(named_force);

    let mut last = BTreeMap::<(BusinessId, OperationKind), _>::new();
    let mut recent_strikes = BTreeMap::<BusinessId, usize>::new();
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
        if success
            && operation.kind() == OperationKind::Sabotage
            && state
                .now()
                .as_minutes()
                .saturating_sub(ended_at.as_minutes())
                < CAMPAIGN_REVIEW_MINUTES
        {
            *recent_strikes.entry(target).or_default() += 1;
        }
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
    let mut gains = BTreeMap::<(_, BusinessKind), i128>::new();
    let mut prominence_by_owner = BTreeMap::<OrganizationId, u8>::new();
    for (id, name, kind, district) in state.world().public_business_directory() {
        let target_owner = state
            .world()
            .get_business(id)
            .map(|business| business.owner());
        // Publicly conspicuous competitors provoke longer campaigns. Small owners still
        // receive the old two-strike respite and an attacker still needs a real local stake.
        let prominence = match target_owner {
            Some(BusinessOwner::Organization(owner)) => *prominence_by_owner
                .entry(owner)
                .or_insert_with(|| public_prominence_tier(state, owner)),
            _ => 0,
        };
        let strike_limit = if prominence >= 3 { 4 } else { 2 };
        if owned.contains(&id) || recent_strikes.get(&id).copied().unwrap_or(0) >= strike_limit {
            continue;
        }
        if state
            .player_organization()
            .is_some_and(|player| target_owner == Some(BusinessOwner::Organization(player)))
            && rival_pressure_blocked(state, organization, district)
        {
            continue;
        }
        let gain = sectors.get(&district).map_or(0, |stakes| {
            *gains.entry((district, kind)).or_insert_with(|| {
                forecast_competitor_disruption_gain(registry, state, id, stakes).unwrap_or(0)
            })
        });
        let predatory_gain = target_owner.map_or(0, |owner| {
            predatory_stake(
                registry,
                state,
                organization,
                id,
                owner,
                &force_by_district,
                strongest_force,
            )
        });
        // Rival businesses also have a political reason to oppose a local
        // challenger: displacing an established civic venue can interrupt the
        // commercial constituency behind a public campaign. This requires an
        // actual player-owned active shop and a visible 500-point local political
        // machine. It never licenses blind strikes on independent owners or
        // against a ward where the attacker has no operating business.
        let local_machine = if state
            .player_organization()
            .is_some_and(|player| target_owner == Some(BusinessOwner::Organization(player)))
        {
            state
                .player_organization()
                .and_then(|player| state.politics().organization(player))
                .and_then(|campaign| campaign.district(district))
                .map_or(0, |office| office.influence())
        } else {
            0
        };
        let political_stake = if local_machine >= 500
            && state
                .economy()
                .get_business_economy(id)
                .is_some_and(|economy| {
                    economy.status() == crate::economy::BusinessOperatingStatus::Active
                }) {
            i128::from(local_machine - 300) * 18
        } else {
            0
        };
        if gain <= 0 && political_stake == 0 && predatory_gain == 0 {
            continue;
        }
        if !is_actionable_opportunity_target(
            registry,
            state,
            organization,
            OperationKind::Sabotage,
            EntityRef::Business(id),
        ) {
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
        // Compare the actual forecast benefit to our own businesses, not the count of
        // properties or the target's ID. Finish a viable watch/strike sequence on equal
        // stakes; after a bounded concerted campaign, other targets get their turn. The oldest
        // attempt wins an otherwise equal choice (untried properties precede old ones).
        let last_attempt = [OperationKind::Surveillance, OperationKind::Sabotage]
            .into_iter()
            .filter_map(|kind| last.get(&(id, kind)).map(|(at, _)| *at))
            .max();
        // Rank by the genuine expected customer gain, but treat the public reach of
        // the competing owner as a strategic threat worth contesting. Never infer
        // concealed rackets or treasury balances when selecting a rival target.
        let strategic_gain = (gain.saturating_mul(100 + i128::from(prominence) * 35) / 100)
            .saturating_add(political_stake)
            .saturating_add(predatory_gain)
            .saturating_add(
                if recently_refused_protection(state, organization, district) {
                    predatory_gain.saturating_mul(4)
                } else {
                    0
                },
            );
        candidates.push((
            Reverse(strategic_gain),
            !observed,
            last_attempt,
            id,
            name,
            kind,
        ));
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
