use super::super::staffing::organization_racket_overextension;
use super::*;

#[test]
fn operating_empire_requires_live_named_crew_and_suspends_excess_after_departures() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let owner = fixture.organization;
    let executive = fixture.authority.manager;
    let neighborhood = match fixture.location {
        EnterpriseLocation::Neighborhood(id) => id,
        _ => unreachable!(),
    };
    let profile = fixture
        .state
        .world()
        .get_neighborhood(neighborhood)
        .unwrap()
        .profile();
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Enterprise);
    validate_revise_mandate(
        &fixture.state,
        fixture.authority.mandate,
        MandateRevisionDraft {
            scopes: BTreeSet::from([fixture.authority.scope, scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let first_authority = MandateAuthority {
        scope,
        ..fixture.authority
    };
    let second_manager = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "District Captain".to_owned(),
            organization: Some(owner),
            supervisor: Some(executive),
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let second_mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: owner,
            manager: second_manager,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::new(),
            budget: None,
        },
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let second_authority = MandateAuthority {
        mandate: second_mandate,
        manager: second_manager,
        scope,
    };
    let mut districts = vec![neighborhood];
    for index in 1..7 {
        districts.push(
            insert_neighborhood(
                &mut fixture.state,
                NeighborhoodDraft {
                    name: format!("Expansion District {index}"),
                    profile,
                },
            )
            .unwrap(),
        );
    }
    let mut rackets = Vec::new();
    for (index, &district) in districts.iter().enumerate().take(6) {
        let settlement = if index == 0 {
            fixture.settlement
        } else {
            insert_account(
                &mut fixture.state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Organization(owner),
                    kind: AccountKind::Settlement,
                },
            )
            .unwrap()
        };
        let token = validate_establish_enterprise(
            &registry,
            &fixture.state,
            EnterpriseDraft {
                kind: EnterpriseKind::Protection,
                organization: owner,
                authority: first_authority,
                location: EnterpriseLocation::Neighborhood(district),
                supporting_businesses: BTreeSet::new(),
                cash_account: fixture.cash,
                settlement_account: settlement,
            },
        )
        .unwrap();
        rackets.push(token.commit(&mut fixture.state).unwrap());
    }
    assert_eq!(
        organization_operating_capacity(&fixture.state, owner),
        (6, 24)
    );
    let seventh_settlement = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(owner),
            kind: AccountKind::Settlement,
        },
    )
    .unwrap();
    let seventh = EnterpriseDraft {
        kind: EnterpriseKind::Protection,
        organization: owner,
        authority: second_authority,
        location: EnterpriseLocation::Neighborhood(districts[6]),
        supporting_businesses: BTreeSet::new(),
        cash_account: fixture.cash,
        settlement_account: seventh_settlement,
    };
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(matches!(
        validate_establish_enterprise(&registry, &fixture.state, seventh.clone()),
        Err(EnterpriseError::OrganizationCapacityExceeded {
            rackets: 7,
            associates: 28,
            max_rackets: 6,
            max_associates: 24,
            ..
        })
    ));
    assert_eq!(before, bincode::serialize(&fixture.state).unwrap());
    let mut extra = Vec::new();
    for index in 0..2 {
        extra.push(
            insert_character(
                &mut fixture.state,
                CharacterDraft {
                    name: format!("Additional Executive {index}"),
                    organization: Some(owner),
                    supervisor: Some(executive),
                    autonomy: AutonomyLevel::Guided,
                    capabilities: BTreeMap::new(),
                    traits: BTreeSet::new(),
                    drives: BTreeMap::new(),
                },
            )
            .unwrap(),
        );
    }
    assert_eq!(
        organization_operating_capacity(&fixture.state, owner),
        (8, 40)
    );
    let seventh_id = validate_establish_enterprise(&registry, &fixture.state, seventh)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    for member in extra {
        validate_reassign_character(&fixture.state, member, None, None)
            .unwrap()
            .commit(&mut fixture.state)
            .unwrap();
    }
    assert_eq!(
        organization_operating_capacity(&fixture.state, owner),
        (6, 24)
    );
    assert!(organization_racket_overextension(&fixture.state, owner).contains(&seventh_id));
    assert!(
        apply_due_understaffing_suspensions(&mut fixture.state).is_empty(),
        "staffing maintenance only changes live rackets at the day boundary"
    );
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    assert_eq!(
        apply_due_understaffing_suspensions(&mut fixture.state),
        vec![seventh_id]
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(seventh_id)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );
    assert!(rackets.iter().all(|id| {
        fixture
            .state
            .enterprises()
            .get_enterprise(*id)
            .unwrap()
            .status()
            == EnterpriseStatus::Active
    }));
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert!(organization_racket_overextension(&restored, owner).is_empty());
    validate_state_against_registry(&registry, &restored).unwrap();
}

#[test]
fn rank_and_file_staffing_changes_operating_books_and_preserves_historical_payroll() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let original = fixture.state.clone();
    let min = EnterpriseKind::Protection.baseline_associates();
    assert_eq!(min, 4);
    assert_eq!(EnterpriseKind::Protection.max_associates(), 12);
    let before = bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap();
    for count in [0, 3, 13, u16::MAX] {
        assert_eq!(
            validate_staff_enterprise(&fixture.state, enterprise, count).err(),
            Some(EnterpriseError::InvalidAssociateCount {
                enterprise,
                minimum: 4,
                maximum: 12,
            })
        );
    }
    assert_eq!(
        validate_staff_enterprise(&fixture.state, enterprise, min).err(),
        Some(EnterpriseError::EnterpriseStaffingUnchanged(enterprise))
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap()
    );
    let old_token = validate_staff_enterprise(&fixture.state, enterprise, 7).unwrap();
    validate_staff_enterprise(&fixture.state, enterprise, 9)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(matches!(
        old_token.commit(&mut fixture.state),
        Err(EnterpriseError::StaleEnterprise { enterprise: found, .. }) if found == enterprise
    ));
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .unwrap()
            .associates(),
        9
    );
    fixture.state =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();

    let mut baseline = original;
    let day = SimDuration::from_minutes(1_440);
    baseline.advance_clock(day);
    fixture.state.advance_clock(day);
    let randomness = EnterpriseCycleRandomness::new(
        0,
        EnterpriseCycleRandomness::MAX_ENFORCEMENT_ATTENTION_ROLL,
    );
    let baseline_plan =
        decide_enterprise_cycle(&registry, &baseline, enterprise, randomness).unwrap();
    let staffed_plan =
        decide_enterprise_cycle(&registry, &fixture.state, enterprise, randomness).unwrap();
    let profile = resolve_location_profile(&fixture.state, fixture.location).unwrap();
    let (additional_revenue, additional_wages) =
        economics::additional_associate_economics(EnterpriseKind::Protection, profile, 9).unwrap();
    assert_eq!(additional_wages, Money::from_cents(5 * 1_500));
    assert_eq!(
        staffed_plan.economics.gross_revenue,
        baseline_plan
            .economics
            .gross_revenue
            .checked_add(additional_revenue)
            .unwrap()
    );
    assert_eq!(
        staffed_plan.economics.operating_cost,
        baseline_plan
            .economics
            .operating_cost
            .checked_add(additional_wages)
            .unwrap()
    );
    assert_eq!(
        staffed_plan.economics.net_cash,
        baseline_plan
            .economics
            .net_cash
            .checked_add(additional_revenue)
            .unwrap()
            .checked_sub(additional_wages)
            .unwrap()
    );
    validate_enterprise_cycle_plan(&fixture.state, staffed_plan)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    // Change staffing after the settlement and validate historical re-derivation.
    validate_staff_enterprise(&fixture.state, enterprise, 4)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let record = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap();
    assert_eq!(record.associates(), 4);
    assert_eq!(record.associates_at(fixture.state.now()), 9);
    validate_state_against_registry(&registry, &fixture.state).unwrap();
    let restored = restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert_eq!(
        restored
            .enterprises()
            .get_enterprise(enterprise)
            .unwrap()
            .associates_at(restored.now()),
        9
    );
    validate_invariants(&restored);
}

#[test]
fn staffing_change_cannot_steal_an_already_due_settlement() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let held = validate_staff_enterprise(&fixture.state, enterprise, 8).unwrap();
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(1_440));
    let before = bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap();
    assert!(matches!(
        held.commit(&mut fixture.state),
        Err(EnterpriseError::StaleCycleTime { .. })
    ));
    assert_eq!(
        validate_staff_enterprise(&fixture.state, enterprise, 8).err(),
        Some(EnterpriseError::EnterpriseStaffingCycleDue(enterprise))
    );
    assert_eq!(
        before,
        bincode::serialize(&build_save(&registry, &fixture.state).unwrap()).unwrap()
    );
}

#[test]
fn named_manager_cannot_supervise_unbounded_field_workers() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let manager = fixture.authority.manager;
    let baseline = fixture.state.clone();
    assert_eq!(MAX_RACKETS_PER_MANAGER, 6);
    assert_eq!(MAX_ASSOCIATES_PER_MANAGER, 24);
    assert!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            24,
            Some(enterprise)
        )
        .is_ok()
    );
    assert!(matches!(
        ensure_manager_capacity(
            &fixture.state,
            fixture.organization,
            manager,
            25,
            Some(enterprise)
        ),
        Err(EnterpriseError::ManagerCapacityExceeded {
            max_associates: 24,
            ..
        })
    ));
    validate_staff_enterprise(&fixture.state, enterprise, 12)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    assert!(matches!(
        ensure_manager_capacity(&fixture.state, fixture.organization, manager, 13, None),
        Err(EnterpriseError::ManagerCapacityExceeded { associates: 25, .. })
    ));
    assert!(
        ensure_manager_capacity(&fixture.state, fixture.organization, manager, 12, None).is_ok()
    );
    assert_eq!(
        baseline
            .enterprises()
            .get_enterprise(enterprise)
            .unwrap()
            .associates(),
        4
    );
    validate_state_against_registry(&registry, &fixture.state).unwrap();
}

#[test]
fn restore_rejects_forged_racket_workforce_and_tampered_staffing_history() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    validate_staff_enterprise(&fixture.state, enterprise, 9)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    let original = fixture
        .state
        .enterprises()
        .get_enterprise(enterprise)
        .unwrap();
    let save = build_save(&registry, &fixture.state).unwrap();

    let mut forged = enterprise_wire(original);
    forged.assignment.associates = 0;
    assert_eq!(
        restore_save(
            &registry,
            replace_serialized_enterprise(save.clone(), original, &forged)
        )
        .err(),
        Some(LoadError::InvalidState(
            StateValidationError::InvalidEnterpriseRuntime { enterprise }
        ))
    );

    let mut forged_history = enterprise_wire(original);
    *forged_history
        .assignment
        .previous_associates
        .values_mut()
        .next()
        .unwrap() = u16::MAX;
    assert_eq!(
        restore_save(
            &registry,
            replace_serialized_enterprise(save, original, &forged_history)
        )
        .err(),
        Some(LoadError::InvalidState(
            StateValidationError::InvalidEnterpriseRuntime { enterprise }
        ))
    );
}
