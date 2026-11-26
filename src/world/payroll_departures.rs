//! Autonomous voluntary departures following actual payroll shortfalls.
//!
//! Resentment alone is not a departure: a fresh missed wage and weak remaining attachment
//! are required. Departures use the membership owner and unwind live responsibilities before
//! changing membership. No random draws or extra persistent counters are needed.

use crate::contacts::contact_system::validate_terminate_contact;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::state::AppState;
use crate::delegation::delegation_system::validate_revoke_mandate;
use crate::enterprises::EnterpriseStatus;
use crate::enterprises::enterprise_execution::validate_suspend_enterprise;
use crate::legal::LegalRepresentationEndReason;
use crate::legal::legal_representation_system::validate_end_legal_representation;
use crate::operations::OperationStatus;
use crate::operations::operation_system::{OperationTransition, apply_transition};
use crate::registry::Registry;
use crate::reports::report_system::validate_record_report;
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::TraitKind;
use crate::world::payroll_execution::PayrollOutcome;
use crate::world::world_system::validate_reassign_character;
use std::collections::BTreeSet;

/// A person may tolerate a bad week for a trusted supervisor, but even the most
/// attached member is not permanently bound to a crew that never pays them.
fn departure_threshold(
    registry: &Registry,
    state: &AppState,
    member: CharacterId,
    supervisor: CharacterId,
) -> Option<u8> {
    let person = state.world().get_character(member)?;
    let dimensions = state
        .social()
        .get_relationship(member, supervisor)?
        .dimensions();
    let attachment = (u16::from(dimensions.trust.value())
        + u16::from(dimensions.respect.value())
        + u16::from(dimensions.affection.value())
        + u16::from(dimensions.dependence.value())
        + u16::from(dimensions.fear.value()))
        / 5;
    let loyal = if person.has_trait(TraitKind::LoyalToFamily) {
        10
    } else {
        0
    };
    let threshold =
        u16::from(registry.upkeep().departure_resentment_floor()) + attachment / 3 + loyal;
    Some(threshold.min(95) as u8)
}

/// Choose from the exact people who were actually underpaid this day, not all
/// characters with a bad relationship. Process deeper subordinates before their
/// managers so departures can naturally propagate through the hierarchy.
pub(crate) fn apply_payroll_departures(
    registry: &Registry,
    state: &mut AppState,
    payrolls: &[PayrollOutcome],
) -> Vec<CharacterId> {
    let mut candidates = Vec::new();
    for payroll in payrolls {
        for &member in payroll.underpaid_members() {
            let Some(person) = state.world().get_character(member) else {
                continue;
            };
            let Some(supervisor) = person.supervisor() else {
                continue;
            };
            // A completed term belongs to the expiry owner even if custody or
            // another binding obligation temporarily prevents normal release.
            if state
                .world()
                .active_freelance_contract(member)
                .is_some_and(|contract| contract.expires_at() <= state.now())
            {
                continue;
            }
            let Some(threshold) = departure_threshold(registry, state, member, supervisor) else {
                continue;
            };
            if state
                .social()
                .get_relationship(member, supervisor)
                .is_none_or(|edge| edge.dimensions().resentment.value() < threshold)
            {
                continue;
            }
            if state.legal().active_arrest_for_character(member).is_some() {
                continue;
            }
            let mut depth = 0usize;
            let mut cursor = Some(supervisor);
            while let Some(id) = cursor {
                depth += 1;
                cursor = state
                    .world()
                    .get_character(id)
                    .and_then(|record| record.supervisor());
            }
            candidates.push((std::cmp::Reverse(depth), member, payroll.organization()));
        }
    }
    candidates.sort_unstable();
    let mut departed = Vec::new();
    for (_, member, organization) in candidates {
        // A departure is an exceptional, cross-owner transaction. Stage it only for a
        // willing candidate; any blocked dependency or exhausted artifact rail leaves all
        // the original responsibilities and membership intact. The canonical validators
        // remain authoritative; a blocked candidate can reconsider on a later shortfall.
        let mut projected = state.clone();
        if stage_departure(registry, &mut projected, member, organization).is_some() {
            *state = projected;
            departed.push(member);
        }
    }
    // Once even the last leader has no crew, assets, or currently viable means to
    // rebuild, a fresh unpaid wage can end the organization itself.
    for payroll in payrolls {
        let organization = payroll.organization();
        if !state
            .history()
            .organization_has_payroll_departures(organization)
        {
            continue;
        }
        let mut members = state.world().characters_in_organization(organization);
        let Some(leader) = members.next().map(|person| person.id()) else {
            continue;
        };
        if members.next().is_some() || !payroll.underpaid_members().contains(&leader) {
            continue;
        }
        drop(members);
        if state.legal().active_arrest_for_character(leader).is_some()
            || state
                .world()
                .businesses_owned_by_organization(organization)
                .next()
                .is_some()
            || state
                .finance()
                .accounts_for(crate::finance::FinancialOwner::Organization(organization))
                .any(|account| account.available_payment_balance() > crate::finance::Money::ZERO)
            || state
                .enterprises()
                .enterprises_for_organization(organization)
                .any(|enterprise| enterprise.status() != EnterpriseStatus::Retired)
            || state
                .operations()
                .operations_for_organization(organization)
                .any(|operation| {
                    !matches!(
                        operation.status(),
                        OperationStatus::Completed | OperationStatus::Aborted
                    ) || operation.resolution().is_some_and(|resolution| {
                        (resolution.cash_proceeds().is_some()
                            && operation.cash_disposition().is_none())
                            || (resolution.property_proceeds().is_some()
                                && operation.property_disposition().is_none())
                    })
                })
            || state
                .opportunities()
                .opportunities_for_organization(organization)
                .any(|opportunity| {
                    opportunity.status() == crate::opportunities::OpportunityStatus::Open
                })
            || state
                .decisions()
                .pending_for_recipient(organization)
                .next()
                .is_some()
            || !crate::recruitment::recruitment_system::find_recruitment_candidates(
                registry,
                state,
                organization,
                leader,
            )
            .is_ok_and(|candidates| candidates.is_empty())
        {
            continue;
        }
        let mut projected = state.clone();
        if stage_departure(registry, &mut projected, leader, organization).is_some() {
            *state = projected;
            departed.push(leader);
        }
    }
    departed
}

fn stage_departure(
    registry: &Registry,
    state: &mut AppState,
    member: CharacterId,
    organization: OrganizationId,
) -> Option<()> {
    let person = state.world().get_character(member)?;
    if person.organization() != Some(organization) {
        return None;
    }
    let supervisor = person.supervisor();
    let name = person.name().to_owned();
    let direct_reports: Vec<_> = state
        .world()
        .direct_reports(member)
        .map(|record| record.id())
        .collect();
    // Remaining loyal reports stay with the organization and report to the next
    // supervisor. If a live responsibility forbids reassignment, defer this departure
    // rather than leave an invalid command chain.
    for subordinate in direct_reports {
        validate_reassign_character(state, subordinate, Some(organization), supervisor)
            .ok()?
            .commit(state)
            .ok()?;
    }
    // Do not manufacture a leadership response to an unresolved police decision.
    // Such a decision must finish normally, then the member can leave.
    while let Some(operation) = state.operations().find_active_operation_booking(member) {
        let record = state.operations().get_operation(operation)?;
        if record.status() == OperationStatus::AwaitingDecision {
            return None;
        }
        apply_transition(registry, state, operation, OperationTransition::Abort).ok()?;
    }
    let mut suspended = 0;
    if let Some(mandate) = state
        .delegation()
        .active_for_manager(member)
        .map(|record| record.id())
    {
        // Live work must stop before its authority can be revoked. A captain's
        // departure does not destroy the organization's sites or historical earnings;
        // suspended rackets can be reassigned through the ordinary succession path.
        let enterprises: Vec<_> = state
            .enterprises()
            .active_for_mandate(mandate)
            .map(|record| record.id())
            .collect();
        for enterprise in enterprises {
            validate_suspend_enterprise(state, enterprise)
                .ok()?
                .commit(state)
                .ok()?;
            suspended += 1;
        }
        validate_revoke_mandate(state, mandate)
            .ok()?
            .commit(state)
            .ok()?;
    }
    let contacts: BTreeSet<_> = state
        .contacts()
        .active_contacts_for_handler(member)
        .chain(state.contacts().active_contacts_for_character(member))
        .map(|record| record.id())
        .collect();
    let channel_count = contacts.len();
    for contact in contacts {
        let representations: Vec<_> = state
            .legal()
            .active_representations_for_contact(contact)
            .map(|record| record.id())
            .collect();
        for representation in representations {
            validate_end_legal_representation(
                state,
                representation,
                LegalRepresentationEndReason::SponsorWithdrawn,
            )
            .ok()?
            .commit(state)
            .ok()?;
        }
        validate_terminate_contact(state, contact)
            .ok()?
            .commit(state)
            .ok()?;
    }
    let reassignment = validate_reassign_character(state, member, None, None).ok()?;
    let history = crate::history::history_system::validate_payroll_departure_event(
        state,
        member,
        organization,
        registry.upkeep().per_member_daily(),
    )
    .ok()?;
    reassignment.commit(state).ok()?;
    history.commit(state).ok()?;

    if state.player_organization() == Some(organization) {
        let remaining = state
            .world()
            .characters_in_organization(organization)
            .count();
        let no_crew = !state
            .world()
            .characters_in_organization(organization)
            .any(|record| record.supervisor().is_some());
        let summary = if remaining == 0 {
            format!(
                "{name} abandoned the organization after its crew and viable resources were lost. Campaign over: the organization dissolved."
            )
        } else {
            format!(
                "{name} walked away after wage nonpayment and accumulated resentment. {remaining} member(s) remain; {suspended} active managed racket(s) suspended for reassignment and {channel_count} institutional channel(s) lost.{}",
                if no_crew {
                    " The supervised crew has disintegrated; any remaining leadership must rebuild its organization."
                } else {
                    ""
                }
            )
        };
        let mut entities = BTreeSet::from([
            EntityRef::Character(member),
            EntityRef::Organization(organization),
        ]);
        if let Some(supervisor) = supervisor {
            entities.insert(EntityRef::Character(supervisor));
        }
        validate_record_report(
            state,
            ReportDraft {
                recipient: organization,
                kind: ReportKind::Financial,
                title: if remaining == 0 {
                    "Organization dissolved".to_owned()
                } else if no_crew {
                    "Crew disintegrated".to_owned()
                } else {
                    "Member walked away".to_owned()
                },
                entries: vec![ReportEntry {
                    attention: if no_crew {
                        AttentionClass::Crisis
                    } else {
                        AttentionClass::Exception
                    },
                    summary,
                    sources: Vec::new(),
                    entities,
                    decision: None,
                }],
            },
        )
        .ok()?
        .commit(state)
        .ok()?;
    }
    Some(())
}
