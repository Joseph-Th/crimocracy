//! Protection-offer cancellation and territorial agreement composition.

use super::*;
use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::persistence::{LoadError, SaveEnvelope};
use crate::decisions::decision_system::validate_resolve_decision;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::operations::OperationKind;
use crate::operations::operation_system::is_actionable_opportunity_target;
use crate::world::OrganizationDraft;
use crate::world::territory_influence::{
    TerritoryNegotiationError, validate_negotiate_standstill,
    validate_negotiate_standstill_with_tribute,
};
use crate::world::world_system::insert_organization;

fn establish_treaty_presence(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    executive: CharacterId,
    district: NeighborhoodId,
) {
    let manager = insert_character(
        state,
        CharacterDraft {
            name: "Treaty ward lieutenant".to_owned(),
            organization: Some(organization),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Management,
                Rating::try_new(80).unwrap(),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Neighborhood(district);
    let mandate = validate_assign_mandate(
        state,
        MandateDraft {
            organization,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
    let venue = insert_business(
        registry,
        state,
        BusinessDraft {
            name: "Treaty Gambling Hall".to_owned(),
            kind: BusinessKind::Hospitality,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::MeetingSpace,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood: district,
            owner: BusinessOwner::Organization(organization),
        },
    )
    .unwrap();
    let cash_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::StreetCash,
        },
    )
    .unwrap();
    let settlement_account = insert_account(
        state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(organization),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    validate_establish_enterprise(
        registry,
        state,
        EnterpriseDraft {
            kind: EnterpriseKind::Gambling,
            organization,
            authority: MandateAuthority {
                mandate,
                manager,
                scope,
            },
            location: EnterpriseLocation::Business(venue),
            supporting_businesses: BTreeSet::new(),
            cash_account,
            settlement_account,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
}

#[test]
fn signed_standstill_retires_superseded_protection_offer() {
    let (registry, mut city, district) = exposed_city();
    let (decision, terms) = first_demand(&registry, &mut city);
    assert_eq!(terms.district(), district);
    let payer = fund_protection_payment(&mut city, terms.amount().cents());
    let held = validate_resolve_decision(
        &registry,
        &city.state,
        decision,
        city.player,
        DecisionResponse::Approve,
    )
    .unwrap();
    establish_treaty_presence(&registry, &mut city.state, city.player, city.boss, district);
    let mut replay = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    let before_balance = city.state.finance().get_account(payer).unwrap().balance();
    for state in [&mut city.state, &mut replay] {
        let treaty = validate_negotiate_standstill(state, city.player, terms.attacker(), district)
            .unwrap()
            .commit(state)
            .unwrap();
        let cancelled = state.decisions().get_decision(decision).unwrap();
        assert_eq!(
            cancelled.status(),
            DecisionStatus::Cancelled,
            "the new thirty-day treaty outlasts the remaining offer and removes its threat"
        );
        assert_eq!(
            cancelled.cancellation().unwrap().reason(),
            DecisionCancellationReason::ProtectionSupersededByStandstill(treaty)
        );
        assert_eq!(
            cancelled.cancellation().unwrap().cancelled_at(),
            state.now()
        );
        assert_eq!(state.decisions().pending_protection_demand(district), None);
        assert!(cancelled.resolution().is_none());
        assert!(!recently_refused_protection(
            state,
            terms.attacker(),
            district
        ));
        assert!(
            !rival_pressure_blocked(state, terms.attacker(), district),
            "the superseded demand must not extend its reconnaissance restriction"
        );
        assert!(is_actionable_opportunity_target(
            &registry,
            state,
            terms.attacker(),
            OperationKind::Surveillance,
            EntityRef::Business(terms.business())
        ));
        assert!(!is_actionable_opportunity_target(
            &registry,
            state,
            terms.attacker(),
            OperationKind::Sabotage,
            EntityRef::Business(terms.business())
        ));
        assert_eq!(
            state.finance().get_account(payer).unwrap().balance(),
            before_balance
        );
        *state = restore_save(&registry, build_save(&registry, state).unwrap()).unwrap();
    }
    let before = bincode::serialize(&city.state).unwrap();
    assert_eq!(
        held.commit(&mut city.state).err(),
        Some(DecisionError::StaleDecision {
            decision,
            expected: 1,
            found: 2
        })
    );
    assert_eq!(
        validate_resolve_decision(
            &registry,
            &city.state,
            decision,
            city.player,
            DecisionResponse::Approve
        )
        .err(),
        Some(DecisionError::DecisionNotPending(decision))
    );
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);
    assert_eq!(bincode::serialize(&replay).unwrap(), before);

    // A cancellation cannot cite an invented signed agreement.
    let record = city.state.decisions().get_decision(decision).unwrap();
    let original_reason = record.cancellation().unwrap().reason();
    let record_bytes = bincode::serialize(record).unwrap();
    let reason_bytes = bincode::serialize(&original_reason).unwrap();
    let mut save_bytes = bincode::serialize(&build_save(&registry, &city.state).unwrap()).unwrap();
    let positions: Vec<_> = save_bytes
        .windows(record_bytes.len())
        .enumerate()
        .filter_map(|(index, bytes)| (bytes == record_bytes).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1);
    // The reason is the final serialized field of a cancelled decision's lifecycle.
    let offsets: Vec<_> = record_bytes
        .windows(reason_bytes.len())
        .enumerate()
        .filter_map(|(index, bytes)| (bytes == reason_bytes).then_some(index))
        .collect();
    let offset = *offsets.last().unwrap();
    assert_eq!(
        offset + reason_bytes.len() + bincode::serialize(&record.version()).unwrap().len(),
        record_bytes.len()
    );
    let forged = bincode::serialize(
        &DecisionCancellationReason::ProtectionSupersededByStandstill(HistoryEventId::from_raw(
            u32::MAX,
        )),
    )
    .unwrap();
    assert_eq!(forged.len(), reason_bytes.len());
    let start = positions[0] + offset;
    save_bytes[start..start + forged.len()].copy_from_slice(&forged);
    let envelope: SaveEnvelope = bincode::deserialize(&save_bytes).unwrap();
    assert_eq!(
        restore_save(&registry, envelope).err(),
        Some(LoadError::InvalidState(
            StateValidationError::InvalidDecisionContext { decision }
        ))
    );
}

#[test]
fn held_standstill_rejects_changed_protection_decision_before_tribute() {
    let (registry, mut city, district) = exposed_city();
    let (decision, terms) = first_demand(&registry, &mut city);
    establish_treaty_presence(&registry, &mut city.state, city.player, city.boss, district);
    let treaty = validate_negotiate_standstill_with_tribute(
        &city.state,
        city.player,
        terms.attacker(),
        district,
        Money::from_cents(1_000),
    )
    .unwrap();
    validate_resolve_decision(
        &registry,
        &city.state,
        decision,
        city.player,
        DecisionResponse::Reject,
    )
    .unwrap()
    .commit(&mut city.state)
    .unwrap();
    let before = bincode::serialize(&city.state).unwrap();
    assert_eq!(
        treaty.commit(&mut city.state),
        Err(TerritoryNegotiationError::ProtectionDemandChanged(district))
    );
    assert_eq!(bincode::serialize(&city.state).unwrap(), before);
    assert!(active_standstill(&city.state, city.player, district).is_none());
    assert!(recently_refused_protection(
        &city.state,
        terms.attacker(),
        district
    ));
    restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
}

#[test]
fn standstill_preserves_paid_protection_and_unrelated_pending_demands() {
    let (registry, mut city, district) = exposed_city();
    let (decision, terms) = first_demand(&registry, &mut city);
    fund_protection_payment(&mut city, terms.amount().cents());
    let mut paid = restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
    validate_resolve_decision(
        &registry,
        &paid,
        decision,
        city.player,
        DecisionResponse::Approve,
    )
    .unwrap()
    .commit(&mut paid)
    .unwrap();
    establish_treaty_presence(&registry, &mut paid, city.player, city.boss, district);
    validate_negotiate_standstill(&paid, city.player, terms.attacker(), district)
        .unwrap()
        .commit(&mut paid)
        .unwrap();
    let record = paid.decisions().get_decision(decision).unwrap();
    assert_eq!(
        record.resolution().unwrap().response(),
        DecisionResponse::Approve
    );
    assert!(record.cancellation().is_none());
    assert!(rival_pressure_blocked(&paid, terms.attacker(), district));
    restore_save(&registry, build_save(&registry, &paid).unwrap()).unwrap();

    let third = insert_organization(
        &registry,
        &mut city.state,
        OrganizationDraft {
            name: "Unrelated Treaty Party".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let executive = insert_character(
        &mut city.state,
        CharacterDraft {
            name: "Third Executive".to_owned(),
            organization: Some(third),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    establish_treaty_presence(&registry, &mut city.state, third, executive, district);
    validate_negotiate_standstill(&city.state, terms.attacker(), third, district)
        .unwrap()
        .commit(&mut city.state)
        .unwrap();
    assert_eq!(
        city.state.decisions().pending_protection_demand(district),
        Some(decision)
    );
    assert_eq!(
        city.state
            .decisions()
            .get_decision(decision)
            .unwrap()
            .status(),
        DecisionStatus::Pending
    );
    assert!(rival_pressure_blocked(
        &city.state,
        terms.attacker(),
        district
    ));
    restore_save(&registry, build_save(&registry, &city.state).unwrap()).unwrap();
}
