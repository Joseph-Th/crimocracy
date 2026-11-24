//! A management crisis must not require retiring a profitable racket or rewriting history.
use super::*;

fn replacement(fixture: &mut EnterpriseFixture) -> MandateAuthority {
    let manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Successor".to_owned(),
            organization: Some(fixture.organization),
            supervisor: Some(fixture.authority.manager),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(92))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    MandateAuthority {
        mandate,
        manager,
        scope,
    }
}

fn settle(
    registry: &Registry,
    fixture: &mut EnterpriseFixture,
    enterprise: EnterpriseId,
) -> EnterpriseCycleId {
    fixture.state.advance_clock(
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .cycle(),
    );
    let variance = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .notable_variance_basis_points() as i16;
    let plan = decide_enterprise_cycle(
        registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(
            variance,
            EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
        ),
    )
    .unwrap();
    validate_enterprise_cycle_plan(&fixture.state, plan)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap()
}

#[test]
fn reassignment_preserves_historical_accounts_authority_and_next_settlement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let former = fixture.authority;
    let successor = replacement(&mut fixture);
    let enterprise = establish_protection(&registry, &mut fixture);
    let first = settle(&registry, &mut fixture, enterprise);
    let first_bytes =
        bincode::serialize(fixture.state.enterprises().get_cycle(first).unwrap()).unwrap();
    let next = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap()
        .next_cycle_at();
    validate_reassign_enterprise(&fixture.state, enterprise, successor)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    // Two more changes in the settlement minute must not rewrite the completed cycle.
    validate_reassign_enterprise(&fixture.state, enterprise, former)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    validate_reassign_enterprise(&fixture.state, enterprise, successor)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap();
    assert_eq!(record.authority_for_cycle_at(fixture.state.now()), former);
    assert_eq!(record.authority(), successor);
    assert_eq!(record.next_cycle_at(), next);
    assert_eq!(record.cash_account(), fixture.cash);
    assert_eq!(record.settlement_account(), fixture.settlement);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .active_for_mandate(former.mandate)
            .count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .active_for_mandate(successor.mandate)
            .count(),
        1
    );
    assert_eq!(
        bincode::serialize(fixture.state.enterprises().get_cycle(first).unwrap()).unwrap(),
        first_bytes
    );
    validate_revoke_mandate(&fixture.state, former.mandate)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let second = settle(&registry, &mut fixture, enterprise);
    let a = fixture.state.enterprises().get_cycle(first).unwrap();
    let b = fixture.state.enterprises().get_cycle(second).unwrap();
    assert_ne!(a.gross_revenue(), b.gross_revenue());
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap();
    assert_eq!(record.authority_for_cycle_at(a.occurred_at()), former);
    assert_eq!(record.authority_for_cycle_at(b.occurred_at()), successor);
    validate_state_against_registry(&registry, &fixture.state).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(
        bincode::serialize(&restored).unwrap(),
        bincode::serialize(&fixture.state).unwrap()
    );
    validate_invariants(&restored);
}

#[test]
fn suspended_enterprise_can_change_manager_but_stays_suspended_until_resumed() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let successor = replacement(&mut fixture);
    let enterprise = establish_protection(&registry, &mut fixture);
    let stale = validate_reassign_enterprise(&fixture.state, enterprise, successor).unwrap();
    validate_suspend_enterprise(&fixture.state, enterprise)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        stale.commit(&mut fixture.state),
        Err(EnterpriseError::StaleEnterprise { .. })
    ));
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    validate_reassign_enterprise(&fixture.state, enterprise, successor)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap();
    assert_eq!(record.status(), EnterpriseStatus::Suspended);
    assert_eq!(record.next_cycle_at(), None);
    assert_eq!(record.manager(), successor.manager);
    assert_eq!(
        fixture
            .state
            .enterprises()
            .active_for_mandate(successor.mandate)
            .count(),
        0
    );
    validate_resume_enterprise(&registry, &fixture.state, enterprise)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert_eq!(
        fixture
            .state
            .enterprises()
            .active_for_mandate(successor.mandate)
            .count(),
        1
    );
    validate_invariants(&fixture.state);
}

#[test]
fn unpaid_captain_leaves_active_racket_recoverable_after_restore() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let former = fixture.authority;
    let boss = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Executive successor".into(),
            organization: Some(fixture.organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(92))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    validate_reassign_character(
        &fixture.state,
        former.manager,
        Some(fixture.organization),
        Some(boss),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    crate::world::world_system::designate_player_organization(
        &mut fixture.state,
        fixture.organization,
    )
    .unwrap();
    for _ in 0..4 {
        fixture.state.advance_clock(SimDuration::from_minutes(1439));
        let tick = run_tick(&registry, &mut fixture.state);
        assert!(tick.payroll_departures.is_empty());
    }
    fixture.state.advance_clock(SimDuration::from_minutes(1439));
    let enterprise = establish_protection(&registry, &mut fixture);
    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    for state in [&mut fixture.state, &mut restored] {
        let tick = run_tick(&registry, state);
        assert_eq!(tick.payroll_departures, vec![former.manager]);
        assert_eq!(
            state
                .world()
                .get_character(former.manager)
                .unwrap()
                .organization(),
            None
        );
        assert!(
            state
                .delegation()
                .active_for_manager(former.manager)
                .is_none()
        );
        let racket = state.enterprises().get_enterprise(enterprise).unwrap();
        assert_eq!(racket.status(), EnterpriseStatus::Suspended);
        assert_eq!(racket.next_cycle_at(), None);
        assert_eq!(racket.cash_account(), fixture.cash);
        assert_eq!(racket.settlement_account(), fixture.settlement);
        assert_eq!(racket.authority(), former);
        let scope = former.scope;
        let mandate = validate_assign_mandate(
            state,
            MandateDraft {
                organization: fixture.organization,
                manager: boss,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .unwrap()
        .commit(state)
        .unwrap();
        let successor = MandateAuthority {
            mandate,
            manager: boss,
            scope,
        };
        validate_reassign_enterprise(state, enterprise, successor)
            .unwrap()
            .commit(state)
            .unwrap();
        validate_resume_enterprise(&registry, state, enterprise)
            .unwrap()
            .commit(state)
            .unwrap();
        assert_eq!(
            state
                .enterprises()
                .get_enterprise(enterprise)
                .unwrap()
                .status(),
            EnterpriseStatus::Active
        );
        assert!(
            state
                .history()
                .latest_payroll_departure(former.manager, fixture.organization)
                .is_some()
        );
        validate_state_against_registry(&registry, state).unwrap();
    }
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
}

#[test]
fn reassignment_does_not_retroactively_transfer_due_cycle_or_accept_ineligible_authority() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let successor = replacement(&mut fixture);
    let enterprise = establish_protection(&registry, &mut fixture);
    let invalid = MandateAuthority {
        manager: fixture.authority.manager,
        ..successor
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        validate_reassign_enterprise(&fixture.state, enterprise, invalid),
        Err(EnterpriseError::Delegation(_))
    ));
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);
    fixture.state.advance_clock(
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .cycle(),
    );
    assert_eq!(
        validate_reassign_enterprise(&fixture.state, enterprise, successor).unwrap_err(),
        EnterpriseError::EnterpriseReassignmentCycleDue(enterprise)
    );
}
