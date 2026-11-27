//! Commercial agreements and objective ownership can change during live field work.

use super::*;
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{MandateAuthority, MandateDraft, ResponsibilityScope};
use crate::enterprises::enterprise_execution::validate_establish_enterprise;
use crate::enterprises::{EnterpriseDraft, EnterpriseKind, EnterpriseLocation};
use crate::world::territory_influence::validate_negotiate_standstill;

fn establish_local_presence(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    manager: CharacterId,
    district: crate::core::id::NeighborhoodId,
) {
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
            name: format!("{organization} gambling venue"),
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
fn acquired_target_under_new_standstill_keeps_campaign_saveable() {
    let (registry, mut state, _, district, operation) =
        make_exposed_operation_fixture(OperationKind::Burglary, false, Vec::new());
    let record = state.operations().get_operation(operation).unwrap();
    let sponsor = record.responsible_organization();
    let manager = record.leader();
    let business = record.objective().business_target().unwrap();
    let rival = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Treaty Counterparty".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let rival_manager = insert_character(
        &mut state,
        CharacterDraft {
            name: "Rival Executive".to_owned(),
            organization: Some(rival),
            supervisor: None,
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
    establish_local_presence(&registry, &mut state, sponsor, manager, district);
    establish_local_presence(&registry, &mut state, rival, rival_manager, district);
    assert_eq!(
        run_tick(&registry, &mut state).started_operations,
        vec![operation]
    );
    let due_at = state
        .operations()
        .get_operation(operation)
        .unwrap()
        .resolution_due_at()
        .unwrap();
    let active = build_save(&registry, &state).unwrap();
    let variance = i8::try_from(
        registry
            .get_operation(OperationKind::Burglary)
            .execution()
            .variance_limit(),
    )
    .unwrap();
    for (acquire_target, expected, explanation) in [
        (
            true,
            OperationObjectiveBlocker::TargetBusinessOwnershipMismatch,
            "hitting its own assets",
        ),
        (
            false,
            OperationObjectiveBlocker::TerritorialStandstill,
            "governed the district when the crew reached its objective",
        ),
    ] {
        let mut original = restore_save(&registry, active.clone()).unwrap();
        if acquire_target {
            validate_transfer_business_ownership(
                &original,
                business,
                BusinessOwner::Organization(sponsor),
            )
            .unwrap()
            .commit(&mut original)
            .unwrap();
        }
        validate_negotiate_standstill(&original, sponsor, rival, district)
            .unwrap()
            .commit(&mut original)
            .unwrap();
        original.advance_clock(SimDuration::from_minutes(
            u32::try_from(due_at.as_minutes() - original.now().as_minutes()).unwrap(),
        ));
        let mut replay =
            restore_save(&registry, build_save(&registry, &original).unwrap()).unwrap();
        for state in [&mut original, &mut replay] {
            let plan = decide_operation_resolution(
                &registry,
                state,
                operation,
                OperationResolutionRandomness::new(variance, 0),
            )
            .unwrap();
            assert_eq!(
                plan.outcome.objective_outcome,
                OperationObjectiveOutcome::Failed
            );
            assert!(plan.outcome.property_proceeds_plan.proceeds.is_none());
            validate_operation_resolution_plan(&registry, state, plan)
                .unwrap()
                .commit(state)
                .unwrap();
            // All input facts came from canonical operations; settlement must never make this campaign unsaveable.
            let saved = build_save(&registry, state)
                .expect("overlapping practical blockers must not invalidate the campaign");
            *state = restore_save(&registry, saved).unwrap();
            let resolution = state
                .operations()
                .get_operation(operation)
                .unwrap()
                .resolution()
                .unwrap();
            assert_eq!(resolution.objective_blocker(), Some(expected));
            assert!(resolution.property_proceeds().is_none());
            assert!(
                state
                    .intelligence()
                    .get_information(resolution.after_action_information())
                    .unwrap()
                    .summary()
                    .contains(explanation)
            );
        }
        assert_eq!(
            bincode::serialize(&original).unwrap(),
            bincode::serialize(&replay).unwrap()
        );
    }
}
